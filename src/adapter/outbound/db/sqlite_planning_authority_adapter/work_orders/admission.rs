//! The task, lineage membership and old-grant fence share one transaction.
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskAuthorityCommit;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkOrderAdmissionSnapshot, PlanningWorkOrderRegistration,
    PlanningWorkOrderSupplementalAdmission, PlanningWorkOrderSupplementalAdmissionResult,
    validate_admission_key, validate_work_order_id, validate_work_order_task_id,
};
use crate::domain::planning::{OriginSessionKind, PriorityQueueService, TaskStatus};

use super::super::{
    SqlitePlanningAuthorityAdapter, bump_planning_revision, load_direction_catalog_from_connection,
    load_task_authority_snapshot_from_connection, open_authority_connection, read_metadata_i64,
    runtime_projection,
};
use super::{
    control, ensure_work_order_tables, execution, insert_task_membership, load_task_membership,
    load_work_order_record, permits, validate_new_task_candidate,
};

pub(super) fn read_snapshot(
    connection: &Connection,
    work_order_id: &str,
) -> Result<Option<PlanningWorkOrderAdmissionSnapshot>> {
    load_work_order_record(connection, work_order_id)?
        .map(|work_order| {
            Ok(PlanningWorkOrderAdmissionSnapshot {
                work_order,
                control: control::load_control(connection, work_order_id)?,
                permit_basis: permits::load_basis(connection, work_order_id)?,
            })
        })
        .transpose()
}

pub(super) fn load_snapshot(
    workspace: &str,
    work_order_id: &str,
) -> Result<Option<PlanningWorkOrderAdmissionSnapshot>> {
    validate_work_order_id(work_order_id)?;
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    let tx = connection.transaction()?;
    read_snapshot(&tx, work_order_id)
}

pub(super) fn load_admitted_task(
    workspace: &str,
    work_order_id: &str,
    admission_key: &str,
) -> Result<Option<String>> {
    validate_work_order_id(work_order_id)?;
    validate_admission_key(admission_key)?;
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    read_admitted_task(&connection, work_order_id, admission_key)
}

fn read_admitted_task(
    connection: &Connection,
    work_order_id: &str,
    key: &str,
) -> Result<Option<String>> {
    let stored: Option<(String, String)> = connection.query_row(
        "SELECT work_order_id, task_id FROM planning_work_order_task_admissions WHERE admission_key=?1",
        [key], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    stored
        .map(|(owner, task_id)| {
            if owner != work_order_id {
                bail!("supplemental admission key belongs to another work order");
            }
            if load_task_membership(connection, &task_id)?.as_deref() != Some(work_order_id) {
                bail!("supplemental admission lost its immutable lineage");
            }
            Ok(task_id)
        })
        .transpose()
}

pub(super) fn admit(
    workspace: &str,
    request: PlanningWorkOrderSupplementalAdmission<'_>,
) -> Result<PlanningWorkOrderSupplementalAdmissionResult> {
    let work_order_id = &request.expected.work_order.work_order_id;
    validate_work_order_id(work_order_id)?;
    validate_admission_key(request.admission_key)?;
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = read_snapshot(&tx, work_order_id)?
        .context("the supplemental task work order is missing")?;
    // A replay is a read, even after task removal, cancellation or authority reset.
    if let Some(task_id) = read_admitted_task(&tx, work_order_id, request.admission_key)? {
        return Ok(PlanningWorkOrderSupplementalAdmissionResult::Existing {
            snapshot: current,
            task_id,
        });
    }
    if current != *request.expected {
        return Ok(PlanningWorkOrderSupplementalAdmissionResult::WorkOrderConflict { current });
    }
    if !current.control.allows_automation() {
        bail!("supplemental admission requires an automatically owned active work order");
    }
    runtime_projection::ensure_no_admin_authority_mutation_guard(
        &tx,
        "work-order supplemental admission",
        None,
    )?;
    let revision = read_metadata_i64(&tx, "planning_revision")?.unwrap_or(0);
    if revision != request.observed_planning_revision {
        return Ok(
            PlanningWorkOrderSupplementalAdmissionResult::RevisionConflict {
                observed_planning_revision: request.observed_planning_revision,
                current_planning_revision: revision,
            },
        );
    }
    validate_work_order_task_id(request.task_id)?;
    let existing = load_task_authority_snapshot_from_connection(&tx)?
        .context("planning task authority is unavailable")?;
    let member = load_task_membership(&tx, request.task_id)?;
    let prior_task = existing
        .task_authority
        .tasks
        .iter()
        .find(|task| task.id == request.task_id);
    let created = prior_task.is_none();
    let already_linked = member.as_deref() == Some(work_order_id.as_str());
    if member.as_ref().is_some_and(|owner| owner != work_order_id) || (member.is_some() && created)
    {
        return Ok(
            PlanningWorkOrderSupplementalAdmissionResult::TaskIdConflict {
                task_id: request.task_id.to_owned(),
            },
        );
    }
    if let Some(task) = prior_task {
        if task.provenance.origin_session_kind != Some(OriginSessionKind::System)
            || task.provenance.turn_id.as_deref() != Some(request.admission_key)
            || request.task_authority != &existing.task_authority
        {
            return Ok(
                PlanningWorkOrderSupplementalAdmissionResult::TaskIdConflict {
                    task_id: request.task_id.to_owned(),
                },
            );
        }
        if !already_linked {
            super::validation_links::ensure_unowned_validation_source_can_be_linked(
                &tx,
                request.task_id,
            )?;
            if !matches!(
                task.status,
                TaskStatus::Ready | TaskStatus::Proposed | TaskStatus::Blocked
            ) {
                bail!("a historical supplemental task must be idle before work-order adoption");
            }
            let active: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM runtime_slot_leases WHERE json_extract(content,'$.task_id')=?1)
                    OR EXISTS(SELECT 1 FROM runtime_session_details WHERE json_extract(content,'$.task_id')=?1)
                    OR EXISTS(SELECT 1 FROM runtime_distributor_queue WHERE json_extract(content,'$.task_id')=?1)",
                [request.task_id], |row| row.get(0),
            )?;
            if active {
                bail!("a historical supplemental task already has execution ownership");
            }
        }
    }
    if !execution::task_attempt_allowed(&tx, work_order_id, request.task_id)? {
        bail!("work-order execution attempt limit reached");
    }
    let candidate_task = request
        .task_authority
        .tasks
        .iter()
        .find(|task| task.id == request.task_id)
        .context("the supplemental task candidate is missing")?;
    if candidate_task.provenance.origin_session_kind != Some(OriginSessionKind::System)
        || candidate_task.provenance.turn_id.as_deref() != Some(request.admission_key)
        || &candidate_task.provenance != request.audit.provenance
        || request.audit.legacy_source_turn_id.is_some()
    {
        bail!("the supplemental candidate and audit must identify their admission key");
    }
    if created {
        validate_new_task_candidate(
            &existing,
            PlanningWorkOrderRegistration {
                work_order_id,
                origin: &current.work_order.origin,
                initial_task_id: request.task_id,
                observed_planning_revision: request.observed_planning_revision,
                task_authority: request.task_authority,
                queue_projection: request.queue_projection,
                audit: request.audit,
            },
        )?;
    }
    if request.audit.task_ids.len() != 1 || request.audit.task_ids[0] != request.task_id {
        bail!("supplemental admission audit must identify exactly its task");
    }
    let mut candidate = request.task_authority.clone();
    let task = candidate
        .tasks
        .iter_mut()
        .find(|task| task.id == request.task_id)
        .context("the supplemental task candidate is missing")?;
    if !already_linked {
        control::append_pending_instructions(&tx, work_order_id, task)?;
    }
    let projection = if candidate == *request.task_authority {
        request.queue_projection.clone()
    } else {
        let directions = load_direction_catalog_from_connection(&tx)?
            .context("planning directions are unavailable")?;
        PriorityQueueService::new().build_projection(&directions, &candidate)?
    };
    let planning_revision = if candidate != existing.task_authority {
        SqlitePlanningAuthorityAdapter::write_task_authority_change_in_transaction(
            &tx,
            &location,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: Some(request.observed_planning_revision),
                task_authority: &candidate,
                queue_projection: &projection,
            },
            Some(&existing.task_authority),
            Some(request.audit),
        )?
    } else {
        bump_planning_revision(&tx)?
    };
    if !already_linked {
        insert_task_membership(
            &tx,
            work_order_id,
            request.task_id,
            i64::try_from(current.work_order.task_ids.len())
                .context("work-order lineage is too large")?,
        )?;
        permits::fence_new_lineage(&tx, work_order_id)?;
    }
    tx.execute(
        "INSERT INTO planning_work_order_task_admissions(admission_key,work_order_id,task_id) VALUES (?1,?2,?3)",
        params![request.admission_key,work_order_id,request.task_id],
    )?;
    runtime_projection::append_runtime_event(
        &tx, "work_order_supplemental_task_admitted", "work_order", work_order_id,
        "supplemental task and work-order permit fence committed",
        &serde_json::json!({"work_order_id":work_order_id,"task_id":request.task_id,"admission_key":request.admission_key}).to_string(),
    )?;
    let snapshot =
        read_snapshot(&tx, work_order_id)?.context("work order disappeared during admission")?;
    tx.commit()?;
    Ok(PlanningWorkOrderSupplementalAdmissionResult::Admitted {
        snapshot,
        task_id: request.task_id.to_owned(),
        planning_revision,
        created,
    })
}

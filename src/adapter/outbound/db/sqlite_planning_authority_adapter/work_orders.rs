use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthoritySnapshot,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkLinkReference, PlanningWorkOrderError, PlanningWorkOrderLinkResult,
    PlanningWorkOrderRecord, PlanningWorkOrderRegistration, PlanningWorkOrderRegistrationResult,
    PlanningWorkOrderRepositoryPort, PlanningWorkOrderValidationLink, validate_work_order_id,
    validate_work_order_identity, validate_work_order_task_id,
};

use crate::domain::planning::task_authority_format_version_supported;

use super::{
    SqlitePlanningAuthorityAdapter, bump_planning_revision,
    load_task_authority_snapshot_from_connection, open_authority_connection, read_metadata_i64,
    runtime_projection, table_exists,
};

mod action;
mod admission;
mod control;
pub(super) use action::{
    identity as worker_action_identity, validate_launch as validate_worker_launch,
};
mod execution;
mod validation_links;
mod validation_recovery;
pub(super) use control::{claim_work_order_launch, validate_cancelled_lineage};
mod permits;
pub(super) use permits::{check_work_order_permit_in_transaction, load_basis as load_permit_basis};

// Keep schema version 19: older binaries ignore these additive tables. There is deliberately no
// foreign key to planning_tasks, whose rows can be removed or replaced independently of lineage.
pub(super) fn ensure_work_order_tables(connection: &Connection) -> Result<()> {
    if table_exists(connection, "planning_work_orders")?
        && table_exists(connection, "planning_work_order_tasks")?
        && table_exists(connection, "planning_work_order_links")?
        && table_exists(connection, "planning_work_order_permit_basis")?
        && table_exists(connection, "planning_work_order_permit_revocations")?
        && table_exists(connection, "planning_work_order_controls")?
        && table_exists(connection, "planning_work_order_control_commands")?
        && table_exists(connection, "planning_work_order_instructions")?
        && table_exists(connection, "planning_work_order_launch_seals")?
        && table_exists(connection, "planning_work_order_validation_links")?
        && table_exists(connection, "planning_work_order_execution_limits")?
        && table_exists(connection, "planning_work_order_started_tasks")?
        && table_exists(connection, "planning_work_order_task_admissions")?
    {
        return Ok(());
    }
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS planning_work_orders (
                work_order_id TEXT PRIMARY KEY NOT NULL,
                origin TEXT NOT NULL,
                permits_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_links (
                work_order_id TEXT PRIMARY KEY NOT NULL,
                reference_json TEXT NOT NULL,
                FOREIGN KEY (work_order_id) REFERENCES planning_work_orders(work_order_id)
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_tasks (
                task_id TEXT PRIMARY KEY NOT NULL,
                work_order_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
                UNIQUE (work_order_id, ordinal),
                FOREIGN KEY (work_order_id) REFERENCES planning_work_orders(work_order_id)
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_permit_basis (
                work_order_id TEXT PRIMARY KEY NOT NULL,
                basis_json TEXT NOT NULL,
                FOREIGN KEY (work_order_id) REFERENCES planning_work_orders(work_order_id)
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_permit_revocations (
                work_order_id TEXT NOT NULL,
                stage TEXT NOT NULL,
                gate_revision TEXT NOT NULL,
                PRIMARY KEY (work_order_id, stage),
                FOREIGN KEY (work_order_id) REFERENCES planning_work_orders(work_order_id)
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_controls (
                work_order_id TEXT PRIMARY KEY NOT NULL,
                control_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_control_commands (
                command_id TEXT PRIMARY KEY NOT NULL,
                work_order_id TEXT NOT NULL,
                action_json TEXT NOT NULL,
                result_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_instructions (
                work_order_id TEXT NOT NULL,
                command_id TEXT NOT NULL,
                message TEXT NOT NULL,
                ordinal INTEGER NOT NULL,
                PRIMARY KEY (work_order_id, command_id),
                UNIQUE (work_order_id, ordinal)
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_launch_seals (
                work_order_id TEXT NOT NULL,
                slot_id TEXT NOT NULL,
                session_key TEXT NOT NULL,
                PRIMARY KEY (work_order_id, slot_id)
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_validation_links (
                validation_record_key TEXT PRIMARY KEY NOT NULL,
                work_order_id TEXT,
                source_task_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
                UNIQUE (work_order_id, ordinal),
                FOREIGN KEY (work_order_id) REFERENCES planning_work_orders(work_order_id)
            );
            CREATE INDEX IF NOT EXISTS planning_work_order_validation_links_owner
                ON planning_work_order_validation_links(work_order_id, validation_record_key);
            CREATE TABLE IF NOT EXISTS planning_work_order_execution_limits (
                work_order_id TEXT PRIMARY KEY NOT NULL,
                max_attempts INTEGER NOT NULL CHECK (max_attempts >= 0)
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_started_tasks (
                work_order_id TEXT NOT NULL,
                task_id TEXT NOT NULL,
                first_session_key TEXT NOT NULL,
                PRIMARY KEY (work_order_id, task_id)
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_task_admissions (
                admission_key TEXT PRIMARY KEY NOT NULL,
                work_order_id TEXT NOT NULL,
                task_id TEXT NOT NULL UNIQUE,
                FOREIGN KEY (work_order_id) REFERENCES planning_work_orders(work_order_id)
            );",
        )
        .context("failed to ensure planning work-order tables")
}

pub(super) fn resolve_validation_successor_key(
    workspace: &str,
    key: &crate::domain::parallel_mode::PrValidationRecordKey,
) -> Result<crate::domain::parallel_mode::PrValidationRecordKey> {
    validation_recovery::resolve_successor_key(workspace, key)
}

impl PlanningWorkOrderRepositoryPort for SqlitePlanningAuthorityAdapter {
    fn recover_work_order_validation(
        &self,
        workspace: &str,
        request: crate::application::port::outbound::planning_work_order_repository_port::PlanningValidationRecoveryRequest<'_>,
    ) -> Result<crate::application::port::outbound::planning_work_order_repository_port::PlanningValidationRecoveryReceipt>{
        validation_recovery::recover(workspace, request)
    }

    fn load_work_order_validation_recovery_origin(
        &self,
        workspace: &str,
        validation_record_key: &str,
    ) -> Result<Option<String>> {
        validation_recovery::load_origin(workspace, validation_record_key)
    }

    fn load_work_order_admission_snapshot(
        &self, workspace: &str, work_order_id: &str,
    ) -> Result<Option<crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderAdmissionSnapshot>>{
        admission::load_snapshot(workspace, work_order_id)
    }

    fn load_work_order_supplemental_task(
        &self,
        workspace: &str,
        work_order_id: &str,
        admission_key: &str,
    ) -> Result<Option<String>> {
        admission::load_admitted_task(workspace, work_order_id, admission_key)
    }

    fn admit_supplemental_work_order_task(
        &self, workspace: &str,
        request: crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderSupplementalAdmission<'_>,
    ) -> Result<crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderSupplementalAdmissionResult>{
        admission::admit(workspace, request)
    }

    fn link_work_order_validation_record(
        &self,
        workspace_dir: &str,
        validation_record_key: &str,
        source_task_id: &str,
    ) -> Result<Option<PlanningWorkOrderValidationLink>> {
        validation_links::link(workspace_dir, validation_record_key, source_task_id)
    }

    fn load_work_order_validation_source(
        &self, workspace: &str, validation_record_key: &str,
    ) -> Result<Option<crate::application::port::outbound::planning_work_order_repository_port::PlanningValidationSource>>{
        validation_links::load_source(workspace, validation_record_key)
    }

    fn load_work_order_validation_link(
        &self,
        workspace_dir: &str,
        validation_record_key: &str,
    ) -> Result<Option<PlanningWorkOrderValidationLink>> {
        validation_links::load(workspace_dir, validation_record_key)
    }

    fn work_order_validation_record_keys(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
    ) -> Result<Vec<String>> {
        validation_links::keys(workspace_dir, work_order_id)
    }

    fn replay_work_order(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
        origin: &str,
        link_reference: Option<&PlanningWorkLinkReference>,
    ) -> Result<Option<PlanningWorkOrderRecord>> {
        validate_work_order_identity(work_order_id, origin)?;
        if let Some(reference) = link_reference {
            reference.validate()?;
        }
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open work-order replay transaction")?;
        let work_order =
            replay_existing_work_order(&transaction, work_order_id, origin, link_reference)?;
        transaction
            .commit()
            .context("failed to commit work-order replay")?;
        Ok(work_order)
    }

    fn load_work_order(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
    ) -> Result<Option<PlanningWorkOrderRecord>> {
        validate_work_order_id(work_order_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction()
            .context("failed to open work-order read transaction")?;
        load_work_order_record(&transaction, work_order_id)
    }

    fn load_work_order_for_task(
        &self,
        workspace_dir: &str,
        task_id: &str,
    ) -> Result<Option<PlanningWorkOrderRecord>> {
        validate_work_order_task_id(task_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction()
            .context("failed to open work-order task read transaction")?;
        let Some(work_order_id) = load_task_membership(&transaction, task_id)? else {
            return Ok(None);
        };
        let record = load_work_order_record(&transaction, &work_order_id)?
            .context("planning task membership has no work-order record")?;
        Ok(Some(record))
    }

    fn register_work_order(
        &self,
        workspace_dir: &str,
        registration: PlanningWorkOrderRegistration<'_>,
    ) -> Result<PlanningWorkOrderRegistrationResult> {
        validate_work_order_identity(registration.work_order_id, registration.origin)?;
        if let Some(reference) = registration.link_reference {
            reference.validate()?;
        }
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open work-order registration transaction")?;

        // The durable identity wins even after task removal, receipt pruning, or authority reset.
        // No candidate task, guard, or stale revision can turn a replay into another creation.
        if let Some(work_order) = replay_existing_work_order(
            &transaction,
            registration.work_order_id,
            registration.origin,
            registration.link_reference,
        )? {
            transaction
                .commit()
                .context("failed to commit existing work-order replay")?;
            return Ok(PlanningWorkOrderRegistrationResult::Existing { work_order });
        }

        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "work-order registration",
            None,
        )?;
        let current_planning_revision =
            read_metadata_i64(&transaction, "planning_revision")?.unwrap_or(0);
        if registration.observed_planning_revision != current_planning_revision {
            return Ok(PlanningWorkOrderRegistrationResult::RevisionConflict {
                observed_planning_revision: registration.observed_planning_revision,
                current_planning_revision,
            });
        }
        validate_work_order_task_id(registration.initial_task_id)?;
        let existing = load_task_authority_snapshot_from_connection(&transaction)?
            .context("planning task authority is unavailable")?;
        if load_task_membership(&transaction, registration.initial_task_id)?.is_some()
            || existing
                .task_authority
                .tasks
                .iter()
                .any(|task| task.id.trim() == registration.initial_task_id)
        {
            return Ok(PlanningWorkOrderRegistrationResult::TaskIdConflict {
                task_id: registration.initial_task_id.to_string(),
            });
        }
        validate_new_task_candidate(&existing, registration)?;

        let mut candidate = registration.task_authority.clone();
        let task = candidate
            .tasks
            .iter_mut()
            .find(|task| task.id == registration.initial_task_id)
            .context("the initial work-order task is missing")?;
        control::append_pending_instructions(&transaction, registration.work_order_id, task)?;
        if control::load_control(&transaction, registration.work_order_id)?.cancelled {
            task.status = crate::domain::planning::TaskStatus::Cancelled;
        }
        let projection = if candidate == *registration.task_authority {
            registration.queue_projection.clone()
        } else {
            let directions = super::load_direction_catalog_from_connection(&transaction)?
                .context("planning directions are unavailable")?;
            crate::domain::planning::PriorityQueueService::new()
                .build_projection(&directions, &candidate)?
        };
        let planning_revision = Self::write_task_authority_change_in_transaction(
            &transaction,
            &location,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: Some(registration.observed_planning_revision),
                task_authority: &candidate,
                queue_projection: &projection,
            },
            Some(&existing.task_authority),
            Some(registration.audit),
        )?;
        transaction
            .execute(
                "INSERT INTO planning_work_orders (work_order_id, origin, permits_json)
                 VALUES (?1, ?2, '[]')",
                params![registration.work_order_id, registration.origin],
            )
            .context("failed to register planning work order")?;
        if let Some(reference) = registration.link_reference {
            transaction.execute(
                "INSERT INTO planning_work_order_links (work_order_id, reference_json) VALUES (?1, ?2)",
                params![registration.work_order_id, serde_json::to_string(reference)?],
            ).context("failed to store work-order publication reference")?;
        }
        insert_task_membership(
            &transaction,
            registration.work_order_id,
            registration.initial_task_id,
            0,
        )?;
        let work_order = PlanningWorkOrderRecord {
            work_order_id: registration.work_order_id.to_string(),
            origin: registration.origin.to_string(),
            link_reference: registration.link_reference.cloned(),
            task_ids: vec![registration.initial_task_id.to_string()],
            permits: Vec::new(),
        };
        transaction
            .commit()
            .context("failed to commit planning work order and task")?;
        Ok(PlanningWorkOrderRegistrationResult::Registered {
            work_order,
            planning_revision,
        })
    }

    fn link_supplemental_task(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
        task_id: &str,
    ) -> Result<PlanningWorkOrderLinkResult> {
        validate_work_order_id(work_order_id)?;
        validate_work_order_task_id(task_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open supplemental task transaction")?;
        let mut work_order =
            load_work_order_record(&transaction, work_order_id)?.ok_or_else(|| {
                PlanningWorkOrderError::WorkOrderNotFound {
                    work_order_id: work_order_id.to_string(),
                }
            })?;
        if let Some(owner) = load_task_membership(&transaction, task_id)? {
            if owner != work_order_id {
                return Err(PlanningWorkOrderError::TaskAlreadyLinked {
                    task_id: task_id.to_string(),
                    work_order_id: owner,
                }
                .into());
            }
            return Ok(PlanningWorkOrderLinkResult {
                work_order,
                linked: false,
                planning_revision: read_metadata_i64(&transaction, "planning_revision")?
                    .unwrap_or(0),
            });
        }
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "work-order supplemental task link",
            None,
        )?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM planning_tasks WHERE task_id = ?1)",
                params![task_id],
                |row| row.get(0),
            )
            .context("failed to inspect supplemental planning task")?;
        if !exists {
            return Err(PlanningWorkOrderError::TaskNotFound {
                task_id: task_id.to_string(),
            }
            .into());
        }
        let ordinal = i64::try_from(work_order.task_ids.len())
            .context("planning work-order lineage is too large")?;
        validation_links::ensure_unowned_validation_source_can_be_linked(&transaction, task_id)?;
        insert_task_membership(&transaction, work_order_id, task_id, ordinal)?;
        work_order.task_ids.push(task_id.to_string());
        permits::fence_new_lineage(&transaction, work_order_id)?;
        work_order.permits.clear();
        if control::load_control(&transaction, work_order_id)?.cancelled {
            control::cancel_lineage(&transaction, &location, &work_order.task_ids)?;
        }
        let planning_revision = bump_planning_revision(&transaction)?;
        transaction
            .commit()
            .context("failed to commit supplemental task link")?;
        Ok(PlanningWorkOrderLinkResult {
            work_order,
            linked: true,
            planning_revision,
        })
    }
}

fn replay_existing_work_order(
    transaction: &Transaction<'_>,
    work_order_id: &str,
    origin: &str,
    link_reference: Option<&PlanningWorkLinkReference>,
) -> Result<Option<PlanningWorkOrderRecord>> {
    // Loading validates contiguous task lineage and every stored permit's identity/shape.
    // Do not check a permit against the current basis: replay may precede its refresh.
    let Some(mut work_order) = load_work_order_record(transaction, work_order_id)? else {
        return Ok(None);
    };
    work_order.ensure_origin(origin)?;
    if work_order.link_reference.as_ref() == link_reference {
        return Ok(Some(work_order));
    }
    let Some(reference) = link_reference.filter(|reference| !reference.close_on_merge) else {
        bail!("work order publication reference changed");
    };
    if work_order.link_reference.is_some() {
        bail!("work order publication reference changed");
    }
    // IMMEDIATE serializes the missing-row observation and INSERT with all other writers.
    // Recovery only records a non-closing reference; it cannot alter execution authority.
    transaction
        .execute(
            "INSERT INTO planning_work_order_links (work_order_id, reference_json) VALUES (?1, ?2)",
            params![work_order_id, serde_json::to_string(reference)?],
        )
        .context("failed to recover legacy work-order publication reference")?;
    work_order.link_reference = Some(reference.clone());
    Ok(Some(work_order))
}

fn validate_new_task_candidate(
    existing: &PlanningTaskAuthoritySnapshot,
    registration: PlanningWorkOrderRegistration<'_>,
) -> Result<()> {
    let next = registration.task_authority;
    let previous = &existing.task_authority;
    if !task_authority_format_version_supported(previous.version)
        || !task_authority_format_version_supported(next.version)
        || next.version < previous.version
    {
        bail!(
            "work-order registration requires a supported task authority version without downgrade"
        );
    }
    if next.tasks.len() != previous.tasks.len() + 1 {
        bail!("work-order registration must add exactly one task");
    }
    if next
        .tasks
        .iter()
        .filter(|task| task.id == registration.initial_task_id)
        .count()
        != 1
    {
        bail!("work-order registration must contain its initial task exactly once");
    }
    let retained = next
        .tasks
        .iter()
        .filter(|task| task.id != registration.initial_task_id);
    if !retained.eq(previous.tasks.iter()) {
        bail!("work-order registration must preserve existing tasks");
    }
    if registration.audit.task_ids.len() != 1
        || registration.audit.task_ids[0] != registration.initial_task_id
    {
        bail!("work-order registration audit must identify exactly its initial task");
    }
    Ok(())
}

pub(super) fn load_task_membership(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT work_order_id FROM planning_work_order_tasks WHERE task_id = ?1",
            params![task_id],
            |row| row.get(0),
        )
        .optional()
        .context("failed to read planning task membership")
}

pub(super) fn load_work_order_record(
    connection: &Connection,
    work_order_id: &str,
) -> Result<Option<PlanningWorkOrderRecord>> {
    let Some((origin, permits_json)) = connection
        .query_row(
            "SELECT origin, permits_json FROM planning_work_orders WHERE work_order_id = ?1",
            params![work_order_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .context("failed to read planning work-order record")?
    else {
        return Ok(None);
    };
    let mut statement = connection
        .prepare(
            "SELECT task_id, ordinal FROM planning_work_order_tasks
             WHERE work_order_id = ?1 ORDER BY ordinal",
        )
        .context("failed to prepare planning work-order lineage")?;
    let rows = statement
        .query_map(params![work_order_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .context("failed to read planning work-order lineage")?;
    let mut task_ids = Vec::new();
    for row in rows {
        let (task_id, ordinal) = row.context("failed to decode planning work-order lineage")?;
        if ordinal != i64::try_from(task_ids.len()).context("work-order lineage is too large")? {
            bail!("planning work-order lineage is not contiguous");
        }
        task_ids.push(task_id);
    }
    let link_json = connection
        .query_row(
            "SELECT reference_json FROM planning_work_order_links WHERE work_order_id=?1",
            params![work_order_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("failed to read work-order publication reference")?;
    let link_reference = link_json
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .context("failed to decode work-order publication reference")?;
    let record = PlanningWorkOrderRecord {
        work_order_id: work_order_id.to_string(),
        origin,
        link_reference,
        task_ids,
        permits: serde_json::from_str(&permits_json)
            .context("failed to decode planning work-order permits")?,
    };
    record.validate()?;
    Ok(Some(record))
}

fn insert_task_membership(
    connection: &Connection,
    work_order_id: &str,
    task_id: &str,
    ordinal: i64,
) -> Result<()> {
    connection
        .execute(
            "INSERT INTO planning_work_order_tasks (task_id, work_order_id, ordinal)
             VALUES (?1, ?2, ?3)",
            params![task_id, work_order_id, ordinal],
        )
        .context("failed to insert planning work-order task membership")?;
    Ok(())
}

#[cfg(test)]
mod tests;

pub(super) fn load_work_order_control_in_transaction(
    connection: &Connection,
    work_order_id: &str,
) -> Result<
    crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkControl,
> {
    ensure_work_order_tables(connection)?;
    control::load_control(connection, work_order_id)
}

pub(super) fn link_validation_source_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    key: &str,
    task_id: &str,
) -> Result<()> {
    validation_links::link_in_transaction(transaction, key, task_id)?;
    Ok(())
}

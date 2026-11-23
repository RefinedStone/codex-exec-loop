use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkOrderError, PlanningWorkPermit, PlanningWorkPermitBasis,
    PlanningWorkPermitDecision, PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
    evaluate_work_order_permit, validate_work_order_id, validate_work_order_task_id,
};

use super::super::{open_authority_connection, runtime_projection};
use super::{
    SqlitePlanningAuthorityAdapter, ensure_work_order_tables, load_task_membership,
    load_work_order_record,
};

pub(super) fn load_basis(
    connection: &Connection,
    work_order_id: &str,
) -> Result<Option<PlanningWorkPermitBasis>> {
    let json: Option<String> = connection
        .query_row(
            "SELECT basis_json FROM planning_work_order_permit_basis WHERE work_order_id = ?1",
            params![work_order_id],
            |row| row.get(0),
        )
        .optional()
        .context("failed to read planning work-order permit basis")?;
    json.map(|json| {
        let basis: PlanningWorkPermitBasis =
            serde_json::from_str(&json).context("failed to decode work-order permit basis")?;
        basis.validate()?;
        Ok(basis)
    })
    .transpose()
}

fn require_work_order(
    connection: &Connection,
    work_order_id: &str,
) -> Result<crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderRecord>
{
    load_work_order_record(connection, work_order_id)?.ok_or_else(|| {
        PlanningWorkOrderError::WorkOrderNotFound {
            work_order_id: work_order_id.to_string(),
        }
        .into()
    })
}

pub(super) fn write_permits(
    transaction: &Transaction<'_>,
    work_order_id: &str,
    permits: &[PlanningWorkPermit],
) -> Result<()> {
    transaction
        .execute(
            "UPDATE planning_work_orders SET permits_json = ?2 WHERE work_order_id = ?1",
            params![
                work_order_id,
                serde_json::to_string(permits).context("failed to encode work-order permits")?
            ],
        )
        .context("failed to write planning work-order permits")?;
    Ok(())
}

fn last_revoked_gate(
    connection: &Connection,
    work_order_id: &str,
    stage: PlanningWorkPermitStage,
) -> Result<Option<u64>> {
    let revision: Option<String> = connection
        .query_row(
            "SELECT gate_revision FROM planning_work_order_permit_revocations
             WHERE work_order_id = ?1 AND stage = ?2",
            params![work_order_id, stage.label()],
            |row| row.get(0),
        )
        .optional()
        .context("failed to read work-order permit revocation fence")?;
    revision
        .map(|revision| {
            revision
                .parse()
                .context("invalid permit revocation gate revision")
        })
        .transpose()
}

impl PlanningWorkPermitRepositoryPort for SqlitePlanningAuthorityAdapter {
    fn load_work_order_permit_basis(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
    ) -> Result<Option<PlanningWorkPermitBasis>> {
        validate_work_order_id(work_order_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        load_basis(&connection, work_order_id)
    }

    fn set_work_order_permit_basis(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
        basis: &PlanningWorkPermitBasis,
    ) -> Result<()> {
        validate_work_order_id(work_order_id)?;
        basis.validate()?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open work-order permit basis transaction")?;
        require_work_order(&transaction, work_order_id)?;
        super::control::load_control(&transaction, work_order_id)?
            .require_epoch(basis.authorization_epoch)?;
        if let Some(current) = load_basis(&transaction, work_order_id)? {
            if current == *basis {
                return Ok(());
            }
            if basis.gate_revision <= current.gate_revision {
                bail!("permit_stale: changed work-order basis requires a newer gate revision");
            }
        }
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "work-order permit basis update",
            None,
        )?;
        let json = serde_json::to_string(basis).context("failed to encode permit basis")?;
        transaction
            .execute(
                "INSERT INTO planning_work_order_permit_basis (work_order_id, basis_json)
                 VALUES (?1, ?2)
                 ON CONFLICT(work_order_id) DO UPDATE SET basis_json = excluded.basis_json",
                params![work_order_id, json],
            )
            .context("failed to save work-order permit basis")?;
        runtime_projection::append_runtime_event(
            &transaction,
            "work_order_permit_basis_updated",
            "work_order_permit",
            work_order_id,
            "work-order permit evidence updated",
            &json,
        )?;
        transaction
            .commit()
            .context("failed to commit work-order permit basis")
    }

    fn grant_work_order_permit(
        &self,
        workspace_dir: &str,
        permit: &PlanningWorkPermit,
    ) -> Result<()> {
        permit.validate()?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open work-order permit grant transaction")?;
        let mut work_order = require_work_order(&transaction, &permit.work_order_id)?;
        super::control::load_control(&transaction, &permit.work_order_id)?
            .require_epoch(permit.basis.authorization_epoch)?;
        if work_order.origin == "issue_automation"
            && permit.stage != PlanningWorkPermitStage::Dispatch
            && permit.basis.delivery_review.is_none()
        {
            bail!(
                "permit_missing: issue automation delivery requires reviewed frozen diff evidence"
            );
        }
        if load_basis(&transaction, &permit.work_order_id)?.as_ref() != Some(&permit.basis) {
            bail!("permit_stale: grant does not match current work-order evidence");
        }
        if last_revoked_gate(&transaction, &permit.work_order_id, permit.stage)?
            .is_some_and(|revision| permit.basis.gate_revision <= revision)
        {
            bail!("permit_stale: revoked permission requires a newer gate revision");
        }
        if let Some(existing) = work_order
            .permits
            .iter()
            .find(|value| value.stage == permit.stage)
        {
            if existing == permit {
                return Ok(());
            }
            if existing.basis.gate_revision == permit.basis.gate_revision {
                bail!("permit_stale: a stage grant cannot change within one gate revision");
            }
        }
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "work-order permit grant",
            None,
        )?;
        work_order
            .permits
            .retain(|value| value.stage != permit.stage);
        work_order.permits.push(permit.clone());
        work_order.permits.sort_by_key(|value| value.stage);
        write_permits(&transaction, &permit.work_order_id, &work_order.permits)?;
        runtime_projection::append_runtime_event(
            &transaction,
            "work_order_permit_granted",
            "work_order_permit",
            &permit.work_order_id,
            &format!("work-order {} permission granted", permit.stage.label()),
            &serde_json::to_string(permit).context("failed to encode permit grant event")?,
        )?;
        transaction
            .commit()
            .context("failed to commit work-order permit grant")
    }

    fn revoke_work_order_permits(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
        stages: &[PlanningWorkPermitStage],
        reason: &str,
    ) -> Result<usize> {
        validate_work_order_id(work_order_id)?;
        if reason.trim().is_empty() || reason.len() > 2048 || reason.chars().any(char::is_control) {
            bail!("permit revocation reason must be bounded nonempty text");
        }
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open work-order permit revocation transaction")?;
        let mut work_order = require_work_order(&transaction, work_order_id)?;
        let stages = stages.iter().copied().collect::<BTreeSet<_>>();
        if stages.is_empty() {
            return Ok(0);
        }
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "work-order permit revocation",
            None,
        )?;
        let revision = load_basis(&transaction, work_order_id)?
            .map(|basis| basis.gate_revision)
            .unwrap_or(0);
        let previous_count = work_order.permits.len();
        work_order
            .permits
            .retain(|permit| !stages.contains(&permit.stage));
        let removed = previous_count - work_order.permits.len();
        let mut changed_fence = false;
        for stage in &stages {
            if last_revoked_gate(&transaction, work_order_id, *stage)?
                .is_some_and(|current| current >= revision)
            {
                continue;
            }
            transaction
                .execute(
                    "INSERT INTO planning_work_order_permit_revocations
                     (work_order_id, stage, gate_revision) VALUES (?1, ?2, ?3)
                     ON CONFLICT(work_order_id, stage) DO UPDATE
                     SET gate_revision = excluded.gate_revision",
                    params![work_order_id, stage.label(), revision.to_string()],
                )
                .context("failed to fence revoked work-order permit")?;
            changed_fence = true;
        }
        if removed == 0 && !changed_fence {
            return Ok(0);
        }
        write_permits(&transaction, work_order_id, &work_order.permits)?;
        runtime_projection::append_runtime_event(
            &transaction,
            "work_order_permit_revoked",
            "work_order_permit",
            work_order_id,
            reason,
            &serde_json::json!({
                "work_order_id": work_order_id,
                "stages": stages,
                "gate_revision": revision,
                "reason": reason,
                "removed": removed,
            })
            .to_string(),
        )?;
        transaction
            .commit()
            .context("failed to commit work-order permit revocation")?;
        Ok(removed)
    }

    fn check_work_order_permit_for_task(
        &self,
        workspace_dir: &str,
        task_id: &str,
        stage: PlanningWorkPermitStage,
        source_sha: Option<&str>,
    ) -> Result<PlanningWorkPermitDecision> {
        validate_work_order_task_id(task_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction()
            .context("failed to open work-order permit check transaction")?;
        check_work_order_permit_in_transaction(&transaction, task_id, stage, source_sha)
    }
}

/// Use this inside the same immediate transaction that installs the slot claim.
pub(in crate::adapter::outbound::db::sqlite_planning_authority_adapter) fn check_work_order_permit_in_transaction(
    transaction: &Transaction<'_>,
    task_id: &str,
    stage: PlanningWorkPermitStage,
    source_sha: Option<&str>,
) -> Result<PlanningWorkPermitDecision> {
    ensure_work_order_tables(transaction)?;
    let Some(work_order_id) = load_task_membership(transaction, task_id)? else {
        return Ok(PlanningWorkPermitDecision::Legacy);
    };
    let work_order = require_work_order(transaction, &work_order_id)?;
    let basis = load_basis(transaction, &work_order_id)?;
    let control = super::control::load_control(transaction, &work_order_id)?;
    if !control.allows_automation()
        || basis
            .as_ref()
            .is_some_and(|basis| basis.authorization_epoch != control.authorization_epoch)
    {
        return Ok(PlanningWorkPermitDecision::Denied(
            crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkPermitBlockReason::Stale,
        ));
    }
    if stage == PlanningWorkPermitStage::Dispatch
        && !super::execution::task_attempt_allowed(transaction, &work_order_id, task_id)?
    {
        return Ok(PlanningWorkPermitDecision::Denied(
            crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkPermitBlockReason::ExecutionAttemptLimit,
        ));
    }
    Ok(evaluate_work_order_permit(
        &work_order,
        basis.as_ref(),
        stage,
        source_sha,
    ))
}

/// A new member cannot inherit any old grant. The evidence generation advances while
/// the operator's authorization epoch and control flags remain unchanged.
pub(super) fn fence_new_lineage(transaction: &Transaction<'_>, work_order_id: &str) -> Result<()> {
    let work_order = require_work_order(transaction, work_order_id)?;
    let mut basis = load_basis(transaction, work_order_id)?;
    let mut revoked_through = basis.as_ref().map(|value| value.gate_revision).unwrap_or(0);
    for permit in &work_order.permits {
        revoked_through = revoked_through.max(permit.basis.gate_revision);
    }
    for stage in PlanningWorkPermitStage::ALL {
        revoked_through =
            revoked_through.max(last_revoked_gate(transaction, work_order_id, stage)?.unwrap_or(0));
    }
    if let Some(current) = basis.as_mut() {
        current.gate_revision = revoked_through
            .checked_add(1)
            .context("work-order permit gate revision exhausted")?;
        current.delivery_review = None;
        transaction.execute(
            "UPDATE planning_work_order_permit_basis SET basis_json=?2 WHERE work_order_id=?1",
            params![work_order_id, serde_json::to_string(current)?],
        )?;
    }
    for stage in PlanningWorkPermitStage::ALL {
        transaction.execute(
            "INSERT INTO planning_work_order_permit_revocations(work_order_id,stage,gate_revision)
             VALUES (?1,?2,?3) ON CONFLICT(work_order_id,stage) DO UPDATE SET gate_revision=excluded.gate_revision",
            params![work_order_id,stage.label(),revoked_through.to_string()],
        )?;
    }
    write_permits(transaction, work_order_id, &[])?;
    runtime_projection::append_runtime_event(
        transaction,
        "work_order_lineage_fenced",
        "work_order",
        work_order_id,
        "new work-order lineage requires current execution evidence",
        &serde_json::json!({
            "work_order_id":work_order_id,
            "revoked_through":revoked_through,
            "gate_revision":basis.as_ref().map(|value|value.gate_revision),
        })
        .to_string(),
    )?;
    Ok(())
}

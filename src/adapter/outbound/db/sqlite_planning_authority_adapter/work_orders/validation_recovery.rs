//! Fresh validation recovery is one authority transaction; the original terminal is never edited.
use anyhow::{Context, Result, bail};
use chrono::Utc;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use super::super::{
    SqlitePlanningAuthorityAdapter, load_task_authority_snapshot_from_connection,
    open_authority_connection, read_metadata_i64, runtime_projection, upsert_authority_metadata,
};
use super::{control, ensure_work_order_tables, load_work_order_record, permits, validation_links};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningValidationRecoveryKind, PlanningValidationRecoveryOutcome,
    PlanningValidationRecoveryReceipt, PlanningValidationRecoveryRequest, validate_work_order_id,
};
use crate::domain::parallel_mode::{
    PrValidationEvent, PrValidationRecord, PrValidationRecordKey, PrValidationTerminalReason,
};
use crate::domain::planning::{TaskAuthorityDocument, TaskStatus};

mod storage;
pub(super) use storage::{load_origin, resolve_successor_key};

fn blocked(
    request: &PlanningValidationRecoveryRequest<'_>,
    reason: &str,
) -> PlanningValidationRecoveryReceipt {
    PlanningValidationRecoveryReceipt {
        command_id: request.command_id.to_owned(),
        terminal_record_id: request.terminal_record_id.to_owned(),
        outcome: PlanningValidationRecoveryOutcome::Blocked {
            reason: reason.to_owned(),
        },
    }
}

fn validate(request: &PlanningValidationRecoveryRequest<'_>) -> Result<()> {
    for value in [
        request.command_id,
        request.work_order_id,
        request.terminal_record_id,
        request.policy_version,
    ] {
        validate_work_order_id(value)?;
    }
    request.observed_work_order.validate()?;
    if request.observed_work_order.work_order_id != request.work_order_id
        || request.observed_planning_revision < 0
        || request.original.observation_revision() > i64::MAX as u64
    {
        bail!("invalid validation recovery comparison basis");
    }
    let reason = match request.kind {
        PlanningValidationRecoveryKind::ObserveAgain => {
            PrValidationTerminalReason::ObservationFailed
        }
        PlanningValidationRecoveryKind::QueueRecovery => {
            PrValidationTerminalReason::RemediationAdmissionFailed
        }
    };
    if request.original.terminal_reason() != Some(&reason) {
        bail!("validation recovery kind does not match its original terminal");
    }
    Ok(())
}

fn binding(request: &PlanningValidationRecoveryRequest<'_>) -> Result<String> {
    Ok(serde_json::to_string(&serde_json::json!({
        "command_id":request.command_id,
        "work_order_id":request.work_order_id,
        "terminal_record_id":request.terminal_record_id,
        "kind":request.kind,
        "authorization_epoch":request.authorization_epoch,
        "gate_revision":request.gate_revision,
        "policy_version":request.policy_version,
        "original":request.original,
    }))?)
}

pub(super) fn recover(
    workspace: &str,
    request: PlanningValidationRecoveryRequest<'_>,
) -> Result<PlanningValidationRecoveryReceipt> {
    validate(&request)?;
    let binding = binding(&request)?;
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // A completed replay is a fact, even after cancellation or a later planning revision.
    if let Some(receipt) = storage::existing_receipt(&tx, &request, &binding)? {
        return Ok(receipt);
    }
    let Some(order) = load_work_order_record(&tx, request.work_order_id)? else {
        return Ok(blocked(&request, "work_order_missing"));
    };
    if order != *request.observed_work_order || !request.is_current_terminal {
        return Ok(blocked(&request, "execution_changed"));
    }
    let control = control::load_control(&tx, request.work_order_id)?;
    if !control.allows_automation() {
        return Ok(blocked(&request, "work_control_blocked"));
    }
    if control.authorization_epoch != request.authorization_epoch {
        return Ok(blocked(&request, "authorization_epoch_changed"));
    }
    let Some(basis) = permits::load_basis(&tx, request.work_order_id)? else {
        return Ok(blocked(&request, "approval_missing"));
    };
    if basis.authorization_epoch != request.authorization_epoch
        || basis.gate_revision != request.gate_revision
        || basis.policy_version != request.policy_version
        || storage::revoked_at_or_after(&tx, request.work_order_id, request.gate_revision)?
    {
        return Ok(blocked(&request, "approval_changed"));
    }
    if read_metadata_i64(&tx, "planning_revision")?.unwrap_or(0)
        != request.observed_planning_revision
    {
        return Ok(blocked(&request, "planning_revision_changed"));
    }
    validation_links::backfill_owned_queue_sources(&tx, request.work_order_id)?;
    let Some(source) = validation_links::read_source(&tx, request.original.key().as_str())? else {
        return Ok(blocked(&request, "validation_source_missing"));
    };
    if source.work_order_id.as_deref() != Some(request.work_order_id)
        || !order.task_ids.contains(&source.source_task_id)
    {
        return Ok(blocked(&request, "validation_source_changed"));
    }
    // Registration of another validation between observation and this transaction invalidates
    // this terminal. No historical record may start a second recovery chain.
    let latest: Option<String> = tx
        .query_row(
            "SELECT validation_record_key FROM planning_work_order_validation_links
             WHERE work_order_id=?1 ORDER BY ordinal DESC LIMIT 1",
            [request.work_order_id],
            |row| row.get(0),
        )
        .optional()?;
    if latest.as_deref() != Some(request.original.key().as_str()) {
        return Ok(blocked(&request, "validation_changed"));
    }
    let Some(original) = storage::original(&tx, request.original.key().as_str())? else {
        return Ok(blocked(&request, "validation_missing"));
    };
    let current: PrValidationRecord = serde_json::from_str(&original.content)
        .context("invalid original validation recovery record")?;
    if current != *request.original
        || current.observation_revision() != request.original.observation_revision()
    {
        return Ok(blocked(&request, "validation_revision_changed"));
    }
    if current.evidence_sha().is_some() {
        if request.kind != PlanningValidationRecoveryKind::ObserveAgain {
            return Ok(blocked(&request, "integrated_queue_recovery_blocked"));
        }
        let execution_limit: Option<i64> = tx.query_row(
            "SELECT max_attempts FROM planning_work_order_execution_limits WHERE work_order_id=?1",
            [request.work_order_id], |row| row.get(0),
        ).optional()?;
        if execution_limit != Some(0) {
            return Ok(blocked(&request, "integrated_execution_limit_changed"));
        }
    }
    if original.operator_paused {
        return Ok(blocked(&request, "operator_paused"));
    }
    if original.poll_in_flight(Utc::now())? {
        return Ok(blocked(&request, "validation_poll_in_flight"));
    }
    runtime_projection::ensure_no_admin_authority_mutation_guard(
        &tx,
        "work-order validation recovery",
        None,
    )?;
    let successor_key = PrValidationRecordKey::new(format!(
        "issue-validation-successor-{:x}",
        Sha256::digest(request.command_id.as_bytes()),
    ))
    .map_err(anyhow::Error::msg)?;
    let successor = match current.fresh_recovery_successor(successor_key.clone()) {
        Ok(record) => record,
        Err(_) => return Ok(blocked(&request, "terminal_not_recoverable")),
    };
    let tasks = load_task_authority_snapshot_from_connection(&tx)?
        .context("planning task authority is unavailable for validation recovery")?;
    let successor = restore_pending_remediation(successor, &tasks.task_authority, &order.task_ids)?;
    let origin = storage::origin_in_connection(&tx, current.key().as_str())?
        .unwrap_or_else(|| current.key().as_str().to_owned());
    let receipt = PlanningValidationRecoveryReceipt {
        command_id: request.command_id.to_owned(),
        terminal_record_id: request.terminal_record_id.to_owned(),
        outcome: PlanningValidationRecoveryOutcome::Registered {
            validation_record_key: successor_key.as_str().to_owned(),
        },
    };
    storage::insert_successor(&tx, current.key().as_str(), &original.content, &successor)?;
    validation_links::link_in_transaction(&tx, successor_key.as_str(), &source.source_task_id)?;
    tx.execute(
        "INSERT INTO planning_work_order_validation_recoveries(
            command_id,work_order_id,terminal_record_id,original_record_key,original_revision,
            remediation_origin_record_key,successor_record_key,request_json,receipt_json
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            request.command_id,
            request.work_order_id,
            request.terminal_record_id,
            current.key().as_str(),
            current.observation_revision() as i64,
            origin,
            successor_key.as_str(),
            binding,
            serde_json::to_string(&receipt)?,
        ],
    )?;
    upsert_authority_metadata(&tx, &location, "last_runtime_projection_at")?;
    runtime_projection::append_runtime_event(
        &tx,
        "pr_validation_recovery_registered",
        "validation_record",
        successor_key.as_str(),
        "existing work-order validation recovery registered",
        &serde_json::json!({
            "work_order_id":request.work_order_id,
            "original_record_key":current.key().as_str(),
            "original_revision":current.observation_revision(),
            "successor_record_key":successor_key.as_str(),
            "command_id":request.command_id,
            "kind":request.kind,
        })
        .to_string(),
    )?;
    tx.commit()?;
    Ok(receipt)
}

/// Existing task state restores only its already recorded correlation. This never admits work.
fn restore_pending_remediation(
    mut successor: PrValidationRecord,
    tasks: &TaskAuthorityDocument,
    owned: &[String],
) -> Result<PrValidationRecord> {
    if successor.evidence_sha().is_some() {
        // A post-integration successor only observes. Existing task correlations stay as facts.
        return Ok(successor);
    }
    let mut pending = None;
    for key in successor.finding_keys() {
        let Some(correlation) = successor.remediation_for(&key) else {
            continue;
        };
        let task_id = correlation.remediation_key().as_str();
        if !owned.iter().any(|id| id == task_id) {
            bail!("validation recovery remediation belongs to another work order");
        }
        let task = tasks
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .context("validation recovery remediation task is missing")?;
        if matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled) {
            continue;
        }
        if pending.is_some() {
            bail!("validation recovery has multiple pending remediation tasks");
        }
        pending = Some((
            correlation.clone(),
            matches!(
                task.status,
                TaskStatus::InProgress | TaskStatus::AwaitingUser
            ),
        ));
    }
    if let Some((correlation, running)) = pending {
        successor = successor
            .transition(PrValidationEvent::BeginPreMergeObservation)
            .map_err(|_| anyhow::anyhow!("cannot begin existing remediation observation"))?;
        successor = successor
            .transition(PrValidationEvent::RemediationQueued(correlation.clone()))
            .map_err(|_| anyhow::anyhow!("cannot restore existing remediation correlation"))?;
        if running {
            successor = successor
                .transition(PrValidationEvent::RemediationStarted {
                    finding_key: correlation.finding_key().clone(),
                })
                .map_err(|_| anyhow::anyhow!("cannot restore existing running remediation"))?;
        }
    }
    Ok(successor)
}

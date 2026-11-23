//! Durable, one-shot isolated-thread continuation. Unknown is never automatically retried.
use anyhow::{Context, Result, bail};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::application::port::outbound::planning_authority_port::{
    PlanningWorkerContinuationClaim, PlanningWorkerContinuationClaimOutcome,
    PlanningWorkerContinuationDelivery, PlanningWorkerContinuationReceipt,
    PlanningWorkerContinuationRejection, PlanningWorkerContinuationRequest,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityMutationAudit,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeSlotLeaseOwnerProcess, ParallelModeSlotLeaseSnapshot,
};
use crate::domain::planning::{
    PlanningAuthorityLocation, PriorityQueueService, TaskActor, TaskMutationProvenance, TaskStatus,
};

use super::{
    SqlitePlanningAuthorityAdapter, load_direction_catalog_from_connection,
    load_task_authority_snapshot_from_connection, open_authority_connection, runtime_projection,
    upsert_authority_metadata, work_orders,
};

mod validation;
use validation::{load_lease, validate_question, validate_request, validate_start};

#[derive(Clone, Serialize, Deserialize)]
struct StoredContinuation {
    request: PlanningWorkerContinuationRequest,
    receipt: PlanningWorkerContinuationReceipt,
    authorization_epoch: u64,
    previous_lease: Option<ParallelModeSlotLeaseSnapshot>,
    lease: Option<ParallelModeSlotLeaseSnapshot>,
}

impl StoredContinuation {
    fn state(&self) -> &'static str {
        match &self.receipt.delivery {
            PlanningWorkerContinuationDelivery::Queued => "queued",
            PlanningWorkerContinuationDelivery::Unknown => "unknown",
            PlanningWorkerContinuationDelivery::Accepted { .. } => "accepted",
            PlanningWorkerContinuationDelivery::RejectedBeforeStart { .. } => "rejected",
        }
    }
}

fn load(connection: &Connection, command_id: &str) -> Result<Option<StoredContinuation>> {
    connection
        .query_row(
            "SELECT content FROM runtime_worker_continuations WHERE command_id=?1",
            params![command_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|json| serde_json::from_str(&json).context("invalid worker continuation record"))
        .transpose()
}

fn save(connection: &Connection, row: &StoredContinuation) -> Result<()> {
    connection.execute(
        "INSERT INTO runtime_worker_continuations
         (command_id, question_id, task_id, work_order_id, worktree_path, delivery_state, updated_at, content)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(command_id) DO UPDATE SET delivery_state=excluded.delivery_state,
             updated_at=excluded.updated_at,content=excluded.content",
        params![row.request.command_id, row.request.question.question_id, row.request.question.task_id,
            row.request.work_order_id, row.request.question.worktree_path, row.state(),
            Utc::now().to_rfc3339(), serde_json::to_string(row)?])?;
    Ok(())
}

pub(super) fn enqueue(
    workspace: &str,
    request: &PlanningWorkerContinuationRequest,
) -> Result<PlanningWorkerContinuationReceipt> {
    validate_request(request)?;
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    work_orders::ensure_work_order_tables(&connection)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let receipt = enqueue_in_transaction(&tx, request)?;
    tx.commit()?;
    Ok(receipt)
}

fn enqueue_in_transaction(
    connection: &Transaction<'_>,
    request: &PlanningWorkerContinuationRequest,
) -> Result<PlanningWorkerContinuationReceipt> {
    if let Some(stored) = load(connection, &request.command_id)? {
        if stored.request != *request {
            bail!("continuation command payload cannot change");
        }
        return Ok(stored.receipt);
    }
    let duplicate_question: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtime_worker_continuations WHERE work_order_id=?1 AND question_id=?2 AND delivery_state IN ('queued','unknown','accepted'))",
        params![request.work_order_id, request.question.question_id],
        |row| row.get(0),
    )?;
    if duplicate_question {
        bail!("this worker question already has an answer command");
    }
    // Only a settled pre-start rejection can admit another human approval. It must belong to
    // a later authorization epoch; an old command always keeps its original immutable receipt.
    let mut rejected = connection.prepare(
        "SELECT content FROM runtime_worker_continuations WHERE work_order_id=?1 AND question_id=?2 AND delivery_state='rejected'",
    )?;
    let rejected_rows = rejected
        .query_map(
            params![request.work_order_id, request.question.question_id],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for json in rejected_rows {
        let previous: StoredContinuation = serde_json::from_str(&json)?;
        if request.authorization_epoch <= previous.request.authorization_epoch {
            bail!("a rejected worker answer requires a newer authorization epoch");
        }
    }
    let pending: bool=connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtime_worker_continuations WHERE task_id=?1 AND delivery_state IN ('queued','unknown'))",
        params![request.question.task_id], |row| row.get(0))?;
    let validation = validate_question(connection, request, Some(request.authorization_epoch))?;
    let (epoch, delivery) = if pending {
        (
            0,
            PlanningWorkerContinuationDelivery::RejectedBeforeStart {
                reason: PlanningWorkerContinuationRejection::ContinuationPending,
            },
        )
    } else {
        match validation {
            Ok(current) => (
                current.authorization_epoch,
                PlanningWorkerContinuationDelivery::Queued,
            ),
            Err(reason) => (
                0,
                PlanningWorkerContinuationDelivery::RejectedBeforeStart { reason },
            ),
        }
    };
    let row = StoredContinuation {
        request: request.clone(),
        receipt: PlanningWorkerContinuationReceipt {
            command_id: request.command_id.clone(),
            question_id: request.question.question_id.clone(),
            delivery,
        },
        authorization_epoch: epoch,
        previous_lease: None,
        lease: None,
    };
    save(connection, &row)?;
    Ok(row.receipt)
}

pub(super) fn receipt(
    workspace: &str,
    work_order_id: &str,
    command_id: &str,
) -> Result<Option<PlanningWorkerContinuationReceipt>> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let connection = open_authority_connection(&location)?;
    let Some(row) = load(&connection, command_id)? else {
        return Ok(None);
    };
    if row.request.work_order_id != work_order_id {
        bail!("continuation belongs to another work order");
    }
    Ok(Some(row.receipt))
}

pub(super) fn queued(workspace: &str) -> Result<Vec<PlanningWorkerContinuationRequest>> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let connection = open_authority_connection(&location)?;
    let mut statement=connection.prepare(
        "SELECT content FROM runtime_worker_continuations WHERE delivery_state='queued' ORDER BY updated_at,command_id LIMIT 32")?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|json| Ok(serde_json::from_str::<StoredContinuation>(&json)?.request))
        .collect()
}

pub(super) fn recheck_queued(
    workspace: &str,
    command_id: &str,
) -> Result<PlanningWorkerContinuationReceipt> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    work_orders::ensure_work_order_tables(&connection)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let receipt = recheck_queued_in_transaction(&tx, command_id)?;
    tx.commit()?;
    Ok(receipt)
}

fn recheck_queued_in_transaction(
    connection: &Transaction<'_>,
    command_id: &str,
) -> Result<PlanningWorkerContinuationReceipt> {
    let mut row = load(connection, command_id)?.context("worker continuation disappeared")?;
    if row.receipt.delivery == PlanningWorkerContinuationDelivery::Queued
        && let Err(reason) =
            validate_question(connection, &row.request, Some(row.authorization_epoch))?
    {
        row.receipt.delivery = PlanningWorkerContinuationDelivery::RejectedBeforeStart { reason };
        save(connection, &row)?;
    }
    Ok(row.receipt)
}

pub(super) fn claim(
    workspace: &str,
    command_id: &str,
    new_generation: &str,
    owner: Option<ParallelModeSlotLeaseOwnerProcess>,
) -> Result<PlanningWorkerContinuationClaimOutcome> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    work_orders::ensure_work_order_tables(&connection)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let outcome = claim_in_transaction(&tx, command_id, new_generation, owner)?;
    upsert_authority_metadata(&tx, &location, "last_runtime_projection_commit_at")?;
    tx.commit()?;
    Ok(outcome)
}

fn claim_in_transaction(
    connection: &Transaction<'_>,
    command_id: &str,
    new_generation: &str,
    owner: Option<ParallelModeSlotLeaseOwnerProcess>,
) -> Result<PlanningWorkerContinuationClaimOutcome> {
    let mut row = load(connection, command_id)?.context("worker continuation disappeared")?;
    if row.receipt.delivery != PlanningWorkerContinuationDelivery::Queued {
        return Ok(PlanningWorkerContinuationClaimOutcome::Settled(row.receipt));
    }
    let current = match validate_question(connection, &row.request, Some(row.authorization_epoch))?
    {
        Ok(current) => current,
        Err(reason) => {
            row.receipt.delivery =
                PlanningWorkerContinuationDelivery::RejectedBeforeStart { reason };
            save(connection, &row)?;
            return Ok(PlanningWorkerContinuationClaimOutcome::Settled(row.receipt));
        }
    };
    if !current.ready {
        return Ok(PlanningWorkerContinuationClaimOutcome::Waiting);
    }
    let mut lease = current.lease.clone();
    lease.lease_generation = Some(new_generation.to_string());
    if new_generation.is_empty()
        || !lease.has_valid_lease_generation()
        || current.lease.lease_generation.as_deref() == Some(new_generation)
    {
        bail!("continuation requires a fresh valid lease generation");
    }
    lease.leased_at = Utc::now().to_rfc3339();
    lease.running_started_at = None;
    lease.owner_process = owner;
    // Keep Running: startup must not release this possibly-started continuation as an unstarted slot.
    let mut detail = current.detail;
    detail.session_key = lease.session_key();
    detail.lease_started_at = lease.leased_at.clone();
    detail.state_label = "continuation_unknown".into();
    detail.completion_state_label = "continuation_unknown".into();
    detail.latest_summary = "답변 전달을 시작했습니다. 확인 전에는 다시 보내지 않습니다.".into();
    detail.updated_at = lease.leased_at.clone();
    detail
        .history
        .push(ParallelModeAgentSessionHistoryEntry::new(
            "continuation_unknown",
            &detail.updated_at,
            &detail.latest_summary,
        ));
    write_detail(connection, &detail)?;
    let changed = connection.execute(
        "UPDATE runtime_slot_leases SET updated_at=?2,content=?3 WHERE slot_id=?1",
        params![
            lease.slot_id,
            lease.leased_at,
            serde_json::to_string(&lease)?
        ],
    )?;
    if changed != 1 {
        bail!("waiting lease changed before continuation claim");
    }
    row.receipt.delivery = PlanningWorkerContinuationDelivery::Unknown;
    row.previous_lease = Some(current.lease.clone());
    row.lease = Some(lease.clone());
    save(connection, &row)?;
    Ok(PlanningWorkerContinuationClaimOutcome::Claimed(Box::new(
        PlanningWorkerContinuationClaim {
            request: row.request,
            task: current.task,
            previous_lease: current.lease,
            lease,
        },
    )))
}

pub(super) fn validate_before_start(
    workspace: &str,
    claim: &PlanningWorkerContinuationClaim,
) -> Result<()> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    let transaction = connection.transaction()?;
    let row =
        load(&transaction, &claim.request.command_id)?.context("continuation claim disappeared")?;
    validate_start(&transaction, &row, claim)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn acknowledge(
    workspace: &str,
    claim: &PlanningWorkerContinuationClaim,
    thread_id: &str,
    turn_id: &str,
) -> Result<PlanningWorkerContinuationReceipt> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let receipt = acknowledge_in_transaction(&tx, &location, claim, thread_id, turn_id)?;
    tx.commit()?;
    Ok(receipt)
}

fn acknowledge_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    location: &PlanningAuthorityLocation,
    claim: &PlanningWorkerContinuationClaim,
    thread_id: &str,
    turn_id: &str,
) -> Result<PlanningWorkerContinuationReceipt> {
    if thread_id != claim.request.question.thread_id
        || turn_id.trim().is_empty()
        || turn_id.len() > 1024
        || turn_id.contains('\0')
    {
        bail!("worker continuation ACK identity did not match");
    }
    let mut row = load(tx, &claim.request.command_id)?.context("continuation claim disappeared")?;
    let delivery = PlanningWorkerContinuationDelivery::Accepted {
        thread_id: thread_id.into(),
        turn_id: turn_id.into(),
        lease_generation: claim
            .lease
            .lease_generation
            .clone()
            .context("continuation generation is missing")?,
    };
    if row.request != claim.request
        || row.lease.as_ref() != Some(&claim.lease)
        || row.previous_lease.as_ref() != Some(&claim.previous_lease)
    {
        bail!("worker continuation ACK belongs to another claim");
    }
    if row.receipt.delivery == delivery {
        return Ok(row.receipt);
    }
    // ACK recording is evidence, not a new authorization. A concurrent control change must not
    // erase the fact that turn/start was accepted. Only the exact lease and waiting revision
    // may advance the task; newer user/worker edits are retained without erasing the ACK.
    if row.receipt.delivery != PlanningWorkerContinuationDelivery::Unknown {
        bail!("worker continuation ACK cannot replace a settled receipt");
    }
    let current =
        load_lease(tx, &claim.lease.slot_id)?.context("continuation lease disappeared")?;
    if !current.same_generation_as(&claim.lease) {
        bail!("worker continuation ACK lease changed");
    }
    resume_task_after_ack(tx, location, &claim.request)?;
    let json: String = tx.query_row(
        "SELECT content FROM runtime_session_details WHERE session_key=?1",
        params![claim.lease.session_key()],
        |row| row.get(0),
    )?;
    let mut detail: ParallelModeAgentSessionDetailSnapshot = serde_json::from_str(&json)?;
    if detail.thread_id.as_deref() != Some(thread_id) {
        bail!("continuation thread detail changed");
    }
    detail.state_label = "running".into();
    detail.completion_state_label = "in_progress".into();
    detail.latest_summary = "같은 작업 스레드가 답변을 받았습니다.".into();
    detail.updated_at = Utc::now().to_rfc3339();
    detail
        .history
        .push(ParallelModeAgentSessionHistoryEntry::new(
            "running",
            &detail.updated_at,
            &detail.latest_summary,
        ));
    write_detail(tx, &detail)?;
    row.receipt.delivery = delivery;
    save(tx, &row)?;
    Ok(row.receipt)
}

fn resume_task_after_ack(
    tx: &rusqlite::Transaction<'_>,
    location: &PlanningAuthorityLocation,
    request: &PlanningWorkerContinuationRequest,
) -> Result<()> {
    let Some(snapshot) = load_task_authority_snapshot_from_connection(tx)? else {
        return Ok(());
    };
    let mut candidate = snapshot.task_authority.clone();
    let Some(task) = candidate
        .tasks
        .iter_mut()
        .find(|task| task.id == request.question.task_id)
    else {
        return Ok(());
    };
    if task.status != TaskStatus::AwaitingUser
        || task.updated_at != request.question.awaiting_task_updated_at
    {
        // The ACK is a delivery fact. Preserve any newer user/worker task mutation.
        return Ok(());
    }
    task.status = TaskStatus::InProgress;
    task.last_updated_by = TaskActor::User;
    task.updated_at = Utc::now().to_rfc3339();
    runtime_projection::ensure_no_admin_authority_mutation_guard(
        tx,
        "worker continuation ACK",
        None,
    )?;
    let directions = load_direction_catalog_from_connection(tx)?
        .context("planning directions are unavailable")?;
    let projection = PriorityQueueService::new().build_projection(&directions, &candidate)?;
    SqlitePlanningAuthorityAdapter::write_task_authority_change_in_transaction(
        tx,
        location,
        PlanningTaskAuthorityCommit {
            observed_planning_revision: Some(snapshot.planning_revision),
            task_authority: &candidate,
            queue_projection: &projection,
        },
        Some(&snapshot.task_authority),
        Some(PlanningTaskAuthorityMutationAudit {
            task_ids: std::slice::from_ref(&request.question.task_id),
            legacy_source_turn_id: None,
            provenance: &TaskMutationProvenance::default(),
        }),
    )?;
    Ok(())
}

pub(super) fn write_detail(
    connection: &Connection,
    detail: &ParallelModeAgentSessionDetailSnapshot,
) -> Result<()> {
    connection.execute("INSERT INTO runtime_session_details(session_key,slot_id,updated_at,content)
        VALUES(?1,?2,?3,?4) ON CONFLICT(session_key) DO UPDATE SET updated_at=excluded.updated_at,content=excluded.content",
        params![detail.session_key,detail.slot_id,detail.updated_at,serde_json::to_string(detail)?])?;
    Ok(())
}

pub(super) fn mark_awaiting_user_detail(
    detail: &mut ParallelModeAgentSessionDetailSnapshot,
    at: &str,
) {
    detail.state_label = "awaiting_user".into();
    detail.completion_state_label = "awaiting_user".into();
    detail.latest_summary =
        "사용자 답변을 기다리며 기존 작업 디렉터리와 스레드를 보존합니다.".into();
    detail.updated_at = at.to_owned();
    detail
        .history
        .push(ParallelModeAgentSessionHistoryEntry::new(
            "awaiting_user",
            &detail.updated_at,
            &detail.latest_summary,
        ));
}

pub(super) fn preserve_waiting(
    workspace: &str,
    expected: &ParallelModeSlotLeaseSnapshot,
) -> Result<bool> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(current) = load_lease(&tx, &expected.slot_id)? else {
        return Ok(false);
    };
    if !current.same_generation_as(expected) {
        return Ok(false);
    }
    if unknown_for_task(&tx, &expected.task_id)? {
        return Ok(true);
    }
    if !task_awaiting(&tx, &expected.task_id)? {
        return Ok(false);
    }
    let json: Option<String> = tx
        .query_row(
            "SELECT content FROM runtime_session_details WHERE session_key=?1",
            params![expected.session_key()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(json) = json else {
        return Ok(true);
    };
    let mut detail: ParallelModeAgentSessionDetailSnapshot = serde_json::from_str(&json)?;
    if detail.task_id != expected.task_id
        || detail.worktree_path != expected.worktree_path
        || detail.branch_name != expected.branch_name
        || detail.slot_id != expected.slot_id
        || detail.thread_id.as_ref().is_none_or(|id| id.is_empty())
    {
        bail!("waiting worker detail identity changed");
    }
    if detail.completion_state_label != "awaiting_user" {
        mark_awaiting_user_detail(&mut detail, &Utc::now().to_rfc3339());
        write_detail(&tx, &detail)?;
    }
    tx.commit()?;
    Ok(true)
}

fn task_awaiting(connection: &Connection, task_id: &str) -> Result<bool> {
    let json: Option<String> = connection
        .query_row(
            "SELECT content_json FROM planning_tasks WHERE task_id=?1",
            params![task_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(json
        .map(|json| serde_json::from_str::<crate::domain::planning::TaskDefinition>(&json))
        .transpose()?
        .is_some_and(|task| task.status == TaskStatus::AwaitingUser))
}

fn unknown_for_task(connection: &Connection, task_id: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtime_worker_continuations
        WHERE task_id=?1 AND delivery_state='unknown')",
        params![task_id],
        |row| row.get(0),
    )?)
}

pub(super) fn task_preserved(workspace: &str, task_id: &str) -> Result<bool> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let connection = open_authority_connection(&location)?;
    task_preserved_in_connection(&connection, task_id)
}

pub(super) fn task_preserved_in_connection(connection: &Connection, task_id: &str) -> Result<bool> {
    let pending: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtime_worker_continuations
        WHERE task_id=?1 AND delivery_state IN ('queued','unknown'))",
        params![task_id],
        |row| row.get(0),
    )?;
    Ok(pending || task_awaiting(connection, task_id)?)
}

pub(super) fn worktree_preserved(workspace: &str, worktree_path: &str) -> Result<bool> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let connection = open_authority_connection(&location)?;
    let pending: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtime_worker_continuations
        WHERE worktree_path=?1 AND delivery_state IN ('queued','unknown'))",
        params![worktree_path],
        |row| row.get(0),
    )?;
    if pending {
        return Ok(true);
    }
    let mut statement = connection.prepare("SELECT content FROM runtime_session_details")?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for json in rows {
        let detail: ParallelModeAgentSessionDetailSnapshot = serde_json::from_str(&json)?;
        if detail.worktree_path == worktree_path && task_awaiting(&connection, &detail.task_id)? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests;

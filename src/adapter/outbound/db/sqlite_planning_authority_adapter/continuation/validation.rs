use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};

use super::{StoredContinuation, load_task_authority_snapshot_from_connection, work_orders};
use crate::application::port::outbound::planning_authority_port::{
    PlanningWorkerContinuationClaim, PlanningWorkerContinuationDelivery,
    PlanningWorkerContinuationRejection as Rejection, PlanningWorkerContinuationRequest,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkPermitDecision, PlanningWorkPermitStage,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState,
};
use crate::domain::planning::TaskStatus;

pub(super) struct CurrentQuestion {
    pub task: crate::domain::planning::TaskDefinition,
    pub lease: ParallelModeSlotLeaseSnapshot,
    pub detail: ParallelModeAgentSessionDetailSnapshot,
    pub authorization_epoch: u64,
    pub ready: bool,
}

pub(super) fn validate_request(request: &PlanningWorkerContinuationRequest) -> Result<()> {
    let question = &request.question;
    for value in [
        &request.command_id,
        &request.work_order_id,
        &question.question_id,
        &question.task_id,
        &question.session_key,
        &question.thread_id,
        &question.slot_id,
        &question.branch_name,
        &question.lease_generation,
        &question.awaiting_task_updated_at,
    ] {
        if value.trim().is_empty() || value.len() > 1024 || value.contains('\0') {
            bail!("invalid worker continuation identity");
        }
    }
    if question.worktree_path.trim().is_empty()
        || question.worktree_path.len() > 32768
        || question.worktree_path.contains('\0')
        || request.message.trim().is_empty()
        || request.message.len() > 65536
        || request.message.contains('\0')
    {
        bail!("invalid worker continuation payload");
    }
    Ok(())
}

pub(super) fn load_lease(
    connection: &Connection,
    slot_id: &str,
) -> Result<Option<ParallelModeSlotLeaseSnapshot>> {
    connection
        .query_row(
            "SELECT content FROM runtime_slot_leases WHERE slot_id=?1",
            params![slot_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|json| serde_json::from_str(&json).context("invalid continuation lease record"))
        .transpose()
}

pub(super) fn validate_question(
    connection: &Connection,
    request: &PlanningWorkerContinuationRequest,
    expected_epoch: Option<u64>,
) -> Result<std::result::Result<CurrentQuestion, Rejection>> {
    let q = &request.question;
    if work_orders::load_task_membership(connection, &q.task_id)?.as_deref()
        != Some(&request.work_order_id)
    {
        return Ok(Err(Rejection::StaleQuestion));
    }
    let Some(order) = work_orders::load_work_order_record(connection, &request.work_order_id)?
    else {
        return Ok(Err(Rejection::StaleQuestion));
    };
    if !order.task_ids.contains(&q.task_id) {
        return Ok(Err(Rejection::StaleQuestion));
    }
    let control =
        work_orders::load_work_order_control_in_transaction(connection, &request.work_order_id)?;
    if !control.allows_automation()
        || control.authorization_epoch != request.authorization_epoch
        || expected_epoch.is_some_and(|epoch| epoch != control.authorization_epoch)
    {
        return Ok(Err(Rejection::WorkControlled));
    }
    let Some(snapshot) = load_task_authority_snapshot_from_connection(connection)? else {
        return Ok(Err(Rejection::StaleQuestion));
    };
    let Some(task) = snapshot
        .task_authority
        .tasks
        .iter()
        .find(|task| task.id == q.task_id)
    else {
        return Ok(Err(Rejection::StaleQuestion));
    };
    if task.status != TaskStatus::AwaitingUser || task.updated_at != q.awaiting_task_updated_at {
        return Ok(Err(Rejection::StaleQuestion));
    }
    let Some(lease) = load_lease(connection, &q.slot_id)? else {
        return Ok(Err(Rejection::InvalidLease));
    };
    if lease.task_id != q.task_id
        || lease.slot_id != q.slot_id
        || lease.worktree_path != q.worktree_path
        || lease.branch_name != q.branch_name
        || lease.lease_generation.as_deref() != Some(&q.lease_generation)
        || !lease.has_valid_lease_generation()
        || lease.session_key() != q.session_key
        || lease.state != ParallelModeSlotLeaseState::Running
        || lease.delivery_target.is_none()
    {
        return Ok(Err(Rejection::InvalidLease));
    }
    let json: Option<String> = connection
        .query_row(
            "SELECT content FROM runtime_session_details WHERE session_key=?1",
            params![q.session_key],
            |row| row.get(0),
        )
        .optional()?;
    let Some(json) = json else {
        return Ok(Err(Rejection::InvalidLease));
    };
    let detail: ParallelModeAgentSessionDetailSnapshot = serde_json::from_str(&json)?;
    if detail.session_key != q.session_key
        || detail.task_id != q.task_id
        || detail.slot_id != q.slot_id
        || detail.thread_id.as_deref() != Some(&q.thread_id)
        || detail.worktree_path != q.worktree_path
        || detail.branch_name != q.branch_name
        || detail.lease_started_at != lease.leased_at
    {
        return Ok(Err(Rejection::InvalidLease));
    }
    let permit = work_orders::check_work_order_permit_in_transaction(
        connection,
        &q.task_id,
        PlanningWorkPermitStage::Dispatch,
        lease
            .delivery_target
            .as_ref()
            .map(|target| target.integration_base_commit_sha.as_str()),
    )?;
    if !matches!(permit, PlanningWorkPermitDecision::Granted { .. }) {
        return Ok(Err(Rejection::PermitUnavailable));
    }
    let ready = detail.completion_state_label == "awaiting_user"
        || (detail.state_label == "awaiting_user"
            && detail.completion_state_label == "in_progress");
    Ok(Ok(CurrentQuestion {
        task: task.clone(),
        lease,
        detail,
        authorization_epoch: control.authorization_epoch,
        ready,
    }))
}

pub(super) fn validate_start(
    connection: &Connection,
    row: &StoredContinuation,
    claim: &PlanningWorkerContinuationClaim,
) -> Result<()> {
    if row.request != claim.request
        || row.previous_lease.as_ref() != Some(&claim.previous_lease)
        || row.lease.as_ref() != Some(&claim.lease)
        || row.receipt.delivery != PlanningWorkerContinuationDelivery::Unknown
    {
        bail!("worker continuation is no longer the claimed action");
    }
    let current =
        load_lease(connection, &claim.lease.slot_id)?.context("continuation lease disappeared")?;
    if !current.same_generation_as(&claim.lease)
        || current.state != ParallelModeSlotLeaseState::Running
    {
        bail!("worker continuation lease changed");
    }
    if work_orders::load_task_membership(connection, &claim.lease.task_id)?.as_deref()
        != Some(row.request.work_order_id.as_str())
    {
        bail!("worker continuation lineage changed");
    }
    let control = work_orders::load_work_order_control_in_transaction(
        connection,
        &row.request.work_order_id,
    )?;
    control.require_epoch(row.authorization_epoch)?;
    let snapshot = load_task_authority_snapshot_from_connection(connection)?
        .context("continuation task disappeared")?;
    if !snapshot.task_authority.tasks.iter().any(|task| {
        task.id == row.request.question.task_id
            && task.status == TaskStatus::AwaitingUser
            && task.updated_at == row.request.question.awaiting_task_updated_at
    }) {
        bail!("waiting task changed before continuation launch");
    }
    let permit = work_orders::check_work_order_permit_in_transaction(
        connection,
        &current.task_id,
        PlanningWorkPermitStage::Dispatch,
        current
            .delivery_target
            .as_ref()
            .map(|target| target.integration_base_commit_sha.as_str()),
    )?;
    if !matches!(permit, PlanningWorkPermitDecision::Granted { .. }) {
        bail!("worker continuation permit changed");
    }
    Ok(())
}

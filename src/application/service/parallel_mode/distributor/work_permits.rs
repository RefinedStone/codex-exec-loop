use super::*;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkPermitDecision, PlanningWorkPermitStage,
};

pub(super) fn work_order_permit_decision(
    authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    record: &ParallelModeDistributorQueueRecord,
    stage: PlanningWorkPermitStage,
) -> Result<PlanningWorkPermitDecision, String> {
    authority
        .check_work_order_permit_for_task(
            repo_root,
            &record.task_id,
            stage,
            Some(&record.effective_source_commit_sha()),
        )
        .map_err(|error| format!("permit_unavailable: {} / {error}", stage.label()))
}

/// The process policy is a ceiling; a task grant can only narrow it.
pub(super) fn work_order_delivery_gate(
    authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    record: &ParallelModeDistributorQueueRecord,
    stage: PlanningWorkPermitStage,
) -> Result<bool, String> {
    let decision = work_order_permit_decision(authority, repo_root, record, stage)?;
    if let Some(reason) = decision.blocked_reason() {
        return Err(format!(
            "{reason}: {} / task {}",
            stage.label(),
            record.task_id
        ));
    }
    if stage == PlanningWorkPermitStage::Integrate
        && decision.requires_human_review()
        && record.pull_request_number.is_none()
    {
        return Err(
            "work-order integration requires human review through a pull request".to_string(),
        );
    }
    Ok(decision.requires_human_review())
}

pub(super) fn block_if_work_order_permit_denied(
    authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    stage: PlanningWorkPermitStage,
) -> Result<Option<String>, String> {
    match work_order_delivery_gate(authority, &resolution.context.repo_root, record, stage) {
        Ok(_) => Ok(None),
        Err(detail) => block_distributor_queue_record(
            authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            detail,
        )
        .map(Some),
    }
}

pub(super) fn is_work_order_permit_wait(record: &ParallelModeDistributorQueueRecord) -> bool {
    record.queue_state == ParallelModeQueueItemState::Blocked
        && blocked_stage(&record.integration_note).is_some()
}

fn blocked_stage(detail: &str) -> Option<PlanningWorkPermitStage> {
    let detail = detail
        .strip_prefix("permit_missing: ")
        .or_else(|| detail.strip_prefix("permit_stale: "))?;
    match detail.split(" / ").next()? {
        "push" => Some(PlanningWorkPermitStage::Push),
        "pull_request" => Some(PlanningWorkPermitStage::PullRequest),
        "integrate" => Some(PlanningWorkPermitStage::Integrate),
        _ => None,
    }
}

pub(super) enum WorkOrderPermitWaitRecheck {
    NotWaiting,
    Waiting,
    Retry(String),
}

/// Only a work-order permit wait or typed evidence wait may yield its place in the queue.
/// Existing operator, conflict, and cleanup blocks retain their previous ordering.
pub(super) fn select_permitted_queue_head(
    authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    evidence_gate: &dyn TaskDeliveryEvidenceGatePort,
    context: &mut PoolRuntimeContext,
    allow_cleanup_probe: bool,
) -> Result<Option<usize>, String> {
    for index in 0..context.distributor_queue_records.len() {
        if context.distributor_queue_records[index].queue_state == ParallelModeQueueItemState::Done
        {
            continue;
        }
        let mut record = context.distributor_queue_records[index].clone();
        if allow_cleanup_probe && super::evidence::can_probe_evidence_wait_cleanup(&record) {
            return Ok(Some(index));
        }
        if super::evidence::is_legacy_integration_evidence_wait(&record) {
            context.distributor_queue_records[index] = record;
            return Ok(Some(index));
        }
        match recheck_evidence_wait(
            runtime,
            evidence_gate,
            &context.repo_root,
            &context.pool_root,
            &mut record,
        )? {
            WorkOrderPermitWaitRecheck::NotWaiting => {}
            WorkOrderPermitWaitRecheck::Waiting => {
                context.distributor_queue_records[index] = record;
                continue;
            }
            WorkOrderPermitWaitRecheck::Retry(detail) => return Err(detail),
        }
        match recheck_work_order_permit_wait(
            authority,
            runtime,
            &context.repo_root,
            &context.pool_root,
            &mut record,
        )? {
            WorkOrderPermitWaitRecheck::NotWaiting => {}
            WorkOrderPermitWaitRecheck::Waiting => continue,
            WorkOrderPermitWaitRecheck::Retry(detail) => return Err(detail),
        }
        context.distributor_queue_records[index] = record;
        return Ok(Some(index));
    }
    Ok(None)
}

/// A new grant re-arms only this task. A lost CAS invalidates the caller's queue snapshot.
pub(super) fn recheck_work_order_permit_wait(
    authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    pool_root: &Path,
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<WorkOrderPermitWaitRecheck, String> {
    if record.queue_state != ParallelModeQueueItemState::Blocked {
        return Ok(WorkOrderPermitWaitRecheck::NotWaiting);
    }
    let Some(stage) = blocked_stage(&record.integration_note) else {
        return Ok(WorkOrderPermitWaitRecheck::NotWaiting);
    };
    let decision = work_order_permit_decision(authority, repo_root, record, stage)?;
    if decision.blocked_reason().is_some() {
        return Ok(WorkOrderPermitWaitRecheck::Waiting);
    }
    let expected = record.clone();
    record.queue_state = ParallelModeQueueItemState::Queued;
    record.recovery_note = Some("work-order execution permit was refreshed".to_string());
    record.integration_note =
        "work-order execution permit is current; queued for revalidation".to_string();
    record.updated_at = current_timestamp();
    record.retry_not_before = None;
    super::run_before_work_order_permit_rearm_hook();
    if !super::store::compare_and_swap_distributor_queue_record(
        authority, runtime, repo_root, pool_root, &expected, record,
    )? {
        // The newer row may be runnable or owned by another distributor. Neither queue
        // selection nor recovery may skip it or write anything else from this snapshot.
        *record = expected;
        return Ok(WorkOrderPermitWaitRecheck::Retry(format!(
            "distributor queue changed during work-order permit refresh / item {}; retry with a fresh authority snapshot",
            record.queue_item_id
        )));
    }
    Ok(WorkOrderPermitWaitRecheck::NotWaiting)
}

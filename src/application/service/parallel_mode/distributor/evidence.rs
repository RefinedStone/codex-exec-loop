//! Evidence is an additional delivery condition, including for legacy work-order permits.
//! Only the repository's typed evidence wait may be rearmed by a fresh observation.
use super::*;
use crate::application::port::inbound::task_delivery_evidence_port::TaskDeliveryEvidenceIntegration;
use crate::application::port::outbound::task_delivery_evidence_repository_port::TaskDeliveryEvidenceOperation;
use crate::domain::planning::delivery_evidence::TaskEvidenceDeliveryStage;

#[allow(clippy::too_many_arguments)]
pub(super) fn block_if_delivery_gates_denied(
    authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    gate: &dyn TaskDeliveryEvidenceGatePort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    stage: TaskEvidenceDeliveryStage,
    integration: Option<&TaskDeliveryEvidenceIntegration>,
    automation: Option<&super::super::ParallelModeAutomationPermit>,
    claim: &DistributorQueueHeadClaimPermit,
) -> Result<Option<String>, String> {
    let permit_stage = match stage {
        TaskEvidenceDeliveryStage::Push => PlanningWorkPermitStage::Push,
        TaskEvidenceDeliveryStage::PullRequest => PlanningWorkPermitStage::PullRequest,
        TaskEvidenceDeliveryStage::Integrate => PlanningWorkPermitStage::Integrate,
    };
    if let Some(notice) =
        block_if_work_order_permit_denied(authority, runtime, resolution, record, permit_stage)?
    {
        return Ok(Some(notice));
    }
    claim.renew("delivery evidence observation")?;
    let decision = gate.evaluate(
        &resolution.context.repo_root,
        record,
        stage,
        integration,
        &TaskDeliveryEvidenceOperation::Effect {
            owner_token: claim.owner_token.clone(),
        },
    )?;
    *record = decision.updated_queue;
    if !decision.allowed {
        super::store::mirror_distributor_queue_record(
            runtime,
            &resolution.context.pool_root,
            record,
        )?;
        if record.queue_state != ParallelModeQueueItemState::Blocked {
            return Err(
                "delivery evidence did not authorize the effect or persist its wait".into(),
            );
        }
        return Ok(Some(format!(
            "task delivery evidence is waiting / task: {} / stage: {} / {}",
            record.task_id,
            stage.label(),
            decision
                .reason
                .map(|reason| reason.label())
                .unwrap_or("evidence_unavailable")
        )));
    }

    // The gate's final DB transaction rechecks the claim, permit, control and evidence after
    // observation. Keep only the in-memory epoch fence here: a successful mirror write or
    // another DB read would create a fresh mutation window before the external effect.
    if let Some(notice) = super::delivery::block_if_automation_epoch_closed(
        authority,
        runtime,
        resolution,
        record,
        automation,
        stage.label(),
    )? {
        return Ok(Some(notice));
    }
    // The caller's ordinary durable state write mirrors the returned audit after the effect.
    Ok(None)
}

pub(super) fn is_evidence_wait(record: &ParallelModeDistributorQueueRecord) -> bool {
    record.queue_state == ParallelModeQueueItemState::Blocked && record.evidence_wait.is_some()
}

/// A persisted integration result may already have been published before a crash. This only
/// selects a read-only remote proof probe; it never authorizes another integration push.
pub(super) fn can_probe_evidence_wait_cleanup(record: &ParallelModeDistributorQueueRecord) -> bool {
    is_evidence_wait(record)
        && record.evidence_wait == Some(TaskEvidenceDeliveryStage::Integrate)
        && record.integration_base_commit_sha.is_some()
        && record.integration_commit_sha.is_some()
}

pub(super) fn recheck_evidence_wait(
    runtime: &dyn ParallelModeRuntimePort,
    gate: &dyn TaskDeliveryEvidenceGatePort,
    repo_root: &str,
    pool_root: &Path,
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<WorkOrderPermitWaitRecheck, String> {
    if !is_evidence_wait(record) {
        return Ok(WorkOrderPermitWaitRecheck::NotWaiting);
    }
    let stage = record.evidence_wait.expect("typed wait was checked");
    let integration = (stage == TaskEvidenceDeliveryStage::Integrate)
        .then(|| {
            Some(TaskDeliveryEvidenceIntegration {
                base_commit_sha: record.integration_base_commit_sha.clone()?,
                result_commit_sha: record.integration_commit_sha.clone()?,
                preflight_requires_human_review: None,
            })
        })
        .flatten();
    let decision = gate.evaluate(
        repo_root,
        record,
        stage,
        integration.as_ref(),
        &TaskDeliveryEvidenceOperation::Recheck,
    )?;
    // Recheck's allowed is always false. Only the durable typed-wait CAS can rearm a row.
    *record = decision.updated_queue;
    super::store::mirror_distributor_queue_record(runtime, pool_root, record)?;
    if is_evidence_wait(record) {
        return Ok(WorkOrderPermitWaitRecheck::Waiting);
    }
    if record.queue_state == ParallelModeQueueItemState::Queued && record.evidence_wait.is_none() {
        return Ok(WorkOrderPermitWaitRecheck::NotWaiting);
    }
    Ok(WorkOrderPermitWaitRecheck::Retry(
        "distributor queue changed during evidence recheck; reload the authority snapshot".into(),
    ))
}

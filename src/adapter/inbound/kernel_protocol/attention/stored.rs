use super::{AttentionCause, AttentionItem, AttentionSubjectKind, item};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use crate::domain::kernel_event::{KernelEvent, RecordedKernelEvent, TurnEndOutcome};
use crate::domain::planning::delivery_evidence::TaskDeliveryEvidenceReason;

pub(crate) fn terminal_turn_item(record: RecordedKernelEvent) -> Option<AttentionItem> {
    let KernelEvent::TurnEnded {
        thread_id,
        turn_id,
        outcome,
        ..
    } = record.event
    else {
        return None;
    };
    let (cause, summary) = match outcome? {
        TurnEndOutcome::Failed => (AttentionCause::TurnFailed, "The kernel turn failed."),
        TurnEndOutcome::Interrupted => (
            AttentionCause::TurnInterrupted,
            "The kernel turn was interrupted.",
        ),
        TurnEndOutcome::Lost => (
            AttentionCause::TurnLost,
            "The kernel lost the turn before completion.",
        ),
        TurnEndOutcome::Completed | TurnEndOutcome::Unknown => return None,
    };
    Some(item(
        cause,
        AttentionSubjectKind::Thread,
        thread_id,
        format!("turn:{turn_id}:{}", record.sequence),
        summary.to_owned(),
        Some(record.recorded_at),
    ))
}

pub(crate) fn delivery_wait_items(
    row: PlanningAuthorityDistributorQueueRecord,
) -> Vec<AttentionItem> {
    let Some(observation) = row
        .evidence_observations
        .iter()
        .find(|observation| Some(observation.stage) == row.evidence_wait)
    else {
        return Vec::new();
    };
    observation
        .reasons
        .iter()
        .filter_map(|reason| {
            let (cause, summary) = match reason {
                TaskDeliveryEvidenceReason::CriteriaNotMet => (
                    AttentionCause::CriteriaNotMet,
                    "Delivery is waiting for unmet criteria.",
                ),
                TaskDeliveryEvidenceReason::EvidenceStale => (
                    AttentionCause::EvidenceStale,
                    "Delivery is waiting for current evidence.",
                ),
                TaskDeliveryEvidenceReason::CriteriaMissing
                | TaskDeliveryEvidenceReason::EvidenceMissing
                | TaskDeliveryEvidenceReason::EvidenceUnavailable
                | TaskDeliveryEvidenceReason::IntegrationSourceMismatch
                | TaskDeliveryEvidenceReason::AutoRequiresEnforce
                | TaskDeliveryEvidenceReason::AutoRequiresMachineCriterion => return None,
            };
            let revision = format!(
                "delivery:{}:{}:{}:{}:{}",
                row.queue_item_id,
                observation.stage.label(),
                observation.evidence_revision,
                observation.observed_at,
                reason.label()
            );
            Some(item(
                cause,
                AttentionSubjectKind::Task,
                row.task_id.clone(),
                revision,
                summary.to_owned(),
                Some(observation.observed_at.clone()),
            ))
        })
        .collect()
}

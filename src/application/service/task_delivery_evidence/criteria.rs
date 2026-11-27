use super::*;
use crate::application::port::outbound::task_evidence_repository_port::TaskEvidenceTaskSnapshot;
use crate::application::port::outbound::task_verification_repository_port::StoredTaskVerification;
use crate::domain::parallel_mode::ParallelModeSlotLeaseState;
use crate::domain::planning::evidence::{TaskEvidenceStatus, criteria_sha256};
use crate::domain::planning::verification::{TaskVerificationPhase, TaskVerificationResult};
use crate::domain::planning::{TaskAcceptanceChannel, TaskAcceptanceCriterion};

pub(super) fn evaluate(
    dependencies: &TaskDeliveryEvidenceDependencies,
    snapshot: &StoredTaskDeliveryEvidence,
    observation: &mut TaskDeliveryEvidenceObservation,
    environments: &mut environment::Environments,
) {
    let Some(task) = &snapshot.task else {
        add_reason(observation, TaskDeliveryEvidenceReason::CriteriaMissing);
        return;
    };
    if task.scope.validate().is_err()
        || task.task.id != snapshot.queue.task_id
        || task.scope.task_id != task.task.id
        || task.scope.acceptance_revision != task.task.acceptance_revision
        || criteria_sha256(&task.task.acceptance).ok().as_ref() != Some(&task.scope.criteria_sha256)
    {
        observation.scope = None;
        add_reason(observation, TaskDeliveryEvidenceReason::EvidenceUnavailable);
        return;
    }
    if snapshot
        .queue
        .evidence_binding
        .as_ref()
        .is_none_or(|binding| {
            binding.validate().is_err()
                || binding.scope != task.scope
                || task.leases.iter().any(|lease| {
                    lease.session_key() != snapshot.queue.session_key
                        || lease.lease_generation.as_ref() != Some(&binding.lease_generation)
                        || lease.slot_id != snapshot.queue.slot_id
                })
        })
    {
        add_reason(observation, TaskDeliveryEvidenceReason::EvidenceStale);
    }
    for criterion in task
        .task
        .acceptance
        .iter()
        .filter(|criterion| criterion.essential)
    {
        let outcome = criterion_outcome(
            dependencies,
            snapshot,
            task,
            criterion,
            observation.source.as_ref(),
            environments,
        );
        let row = match outcome {
            Ok((state, event_id)) => TaskDeliveryCriterionObservation {
                criterion_id: criterion.id.clone(),
                state,
                reason: None,
                receipt_event_id: Some(event_id),
            },
            Err(reason) => {
                add_reason(observation, reason);
                TaskDeliveryCriterionObservation {
                    criterion_id: criterion.id.clone(),
                    state: TaskDeliveryCriterionState::Blocked,
                    reason: Some(reason),
                    receipt_event_id: None,
                }
            }
        };
        observation.criteria.push(row);
    }
    if observation.criteria.is_empty() {
        add_reason(observation, TaskDeliveryEvidenceReason::CriteriaMissing);
    }
}

fn criterion_outcome(
    dependencies: &TaskDeliveryEvidenceDependencies,
    snapshot: &StoredTaskDeliveryEvidence,
    task: &TaskEvidenceTaskSnapshot,
    criterion: &TaskAcceptanceCriterion,
    source: Option<&TaskEvidenceDeliverySource>,
    environments: &mut environment::Environments,
) -> Result<(TaskDeliveryCriterionState, String), TaskDeliveryEvidenceReason> {
    let invalid = TaskDeliveryEvidenceReason::EvidenceUnavailable;
    if let Some(receipt) = task
        .waivers
        .iter()
        .find(|value| value.waiver.criterion_id == criterion.id)
    {
        receipt.validate().map_err(|_| invalid)?;
        if receipt.evidence_revision > snapshot.evidence_revision {
            return Err(invalid);
        }
        if receipt.waiver.scope == task.scope {
            return Ok((TaskDeliveryCriterionState::Waived, receipt.event_id.clone()));
        }
    }
    let source = source.ok_or(invalid)?;
    let binding = snapshot
        .queue
        .evidence_binding
        .as_ref()
        .ok_or(TaskDeliveryEvidenceReason::EvidenceStale)?;
    if let Some(receipt) = task
        .evidence
        .iter()
        .find(|value| value.evidence.attempt.criterion_id == criterion.id)
    {
        receipt.validate().map_err(|_| invalid)?;
        if receipt.evidence_revision > snapshot.evidence_revision {
            return Err(invalid);
        }
        let report = &receipt.evidence;
        if report.attempt.scope == task.scope
            && report.attempt.session_key == snapshot.queue.session_key
            && report.attempt.lease_generation == binding.lease_generation
            && report.recorded_tree.tree_sha == source.source_tree_sha
            && matches!(
                report.status,
                TaskEvidenceStatus::Fail | TaskEvidenceStatus::Blocked
            )
        {
            dependencies
                .artifacts
                .verify_sealed(&dependencies.workspace, &report.attempt, &report.artifact)
                .map_err(|_| invalid)?;
            return Err(TaskDeliveryEvidenceReason::CriteriaNotMet);
        }
    }
    if criterion.channel != TaskAcceptanceChannel::Command
        || criterion.verification_recipe.is_none()
    {
        if criterion.channel != TaskAcceptanceChannel::Command {
            let reviewed = task
                .evidence
                .iter()
                .find(|e| e.evidence.attempt.criterion_id == criterion.id)
                .and_then(|e| task.reviews.iter().find(|r| r.matches(e)).map(|r| (e, r)));
            if let Some((e, r)) = reviewed {
                if r.scope != task.scope
                    || e.evidence.attempt.session_key != snapshot.queue.session_key
                    || e.evidence.attempt.lease_generation != binding.lease_generation
                    || r.tree_sha != source.source_tree_sha
                {
                    return Err(TaskDeliveryEvidenceReason::EvidenceStale);
                }
                dependencies
                    .artifacts
                    .verify_sealed(
                        &dependencies.workspace,
                        &e.evidence.attempt,
                        &e.evidence.artifact,
                    )
                    .map_err(|_| invalid)?;
                return match r.decision {
                    crate::domain::planning::evidence::HumanEvidenceDecision::Pass => {
                        Ok((TaskDeliveryCriterionState::HumanPassed, r.review_id.clone()))
                    }
                    crate::domain::planning::evidence::HumanEvidenceDecision::Fail => {
                        Err(TaskDeliveryEvidenceReason::CriteriaNotMet)
                    }
                };
            }
        }
        return Err(TaskDeliveryEvidenceReason::EvidenceMissing);
    }
    let stored = task
        .host_verifications
        .iter()
        .find(|value| value.attempt.criterion_id == criterion.id)
        .ok_or(TaskDeliveryEvidenceReason::EvidenceMissing)?;
    validate_host(snapshot, task, criterion, stored, source)?;
    if stored.phase != TaskVerificationPhase::Finished {
        return Err(TaskDeliveryEvidenceReason::EvidenceMissing);
    }
    let receipt = stored.receipt.as_ref().ok_or(invalid)?;
    if !matches!(receipt.result, TaskVerificationResult::Executed { .. }) {
        return Err(TaskDeliveryEvidenceReason::CriteriaNotMet);
    }
    let manifest = stored.manifest.as_ref().ok_or(invalid)?;
    if environments.observe(dependencies.runner.as_ref(), manifest, &criterion.id)?
        != manifest.environment_sha256
    {
        return Err(TaskDeliveryEvidenceReason::EvidenceStale);
    }
    let artifact = receipt.artifact.as_ref().ok_or(invalid)?;
    dependencies
        .artifacts
        .verify_host_capture(
            &dependencies.workspace,
            &stored.attempt.attempt_id,
            artifact,
        )
        .map_err(|_| invalid)?;
    if !receipt.passed() {
        return Err(TaskDeliveryEvidenceReason::CriteriaNotMet);
    }
    Ok((
        TaskDeliveryCriterionState::HostPassed,
        receipt.event_id.clone(),
    ))
}

fn validate_host(
    snapshot: &StoredTaskDeliveryEvidence,
    task: &TaskEvidenceTaskSnapshot,
    criterion: &TaskAcceptanceCriterion,
    stored: &StoredTaskVerification,
    source: &TaskEvidenceDeliverySource,
) -> Result<(), TaskDeliveryEvidenceReason> {
    let invalid = TaskDeliveryEvidenceReason::EvidenceUnavailable;
    let stale = TaskDeliveryEvidenceReason::EvidenceStale;
    let attempt = &stored.attempt;
    let lease = &stored.lease;
    attempt.validate().map_err(|_| invalid)?;
    if lease.state != ParallelModeSlotLeaseState::Running
        || lease.task_id != attempt.scope.task_id
        || lease.slot_id != attempt.slot_id
        || lease.lease_generation.as_deref() != Some(attempt.lease_generation.as_str())
        || lease.session_key() != attempt.session_key
    {
        return Err(invalid);
    }
    let binding = snapshot.queue.evidence_binding.as_ref().ok_or(stale)?;
    if attempt.scope != task.scope
        || criterion.verification_recipe != Some(attempt.recipe)
        || attempt.session_key != snapshot.queue.session_key
        || attempt.slot_id != snapshot.queue.slot_id
        || attempt.lease_generation != binding.lease_generation
        || lease
            .delivery_target
            .as_ref()
            .is_none_or(|target| target.integration_base_commit_sha != source.base_commit_sha)
    {
        return Err(stale);
    }
    if let Some(manifest) = &stored.manifest {
        manifest.validate().map_err(|_| invalid)?;
        if manifest.recipe != attempt.recipe
            || manifest.base_commit_sha != source.base_commit_sha
            || manifest.base_tree_sha != source.base_tree_sha
            || manifest.source.tree_sha != source.source_tree_sha
        {
            return Err(stale);
        }
        // A host commit may change HEAD while retaining the verified full tree.
    }
    if let Some(receipt) = &stored.receipt {
        receipt.validate().map_err(|_| invalid)?;
        if receipt.attempt != *attempt
            || receipt.manifest != stored.manifest
            || receipt.evidence_revision > snapshot.evidence_revision
        {
            return Err(invalid);
        }
    }
    if (stored.phase == TaskVerificationPhase::Pending && stored.manifest.is_some())
        || (stored.phase == TaskVerificationPhase::Running && stored.manifest.is_none())
        || (stored.phase == TaskVerificationPhase::Finished) != stored.receipt.is_some()
    {
        return Err(invalid);
    }
    Ok(())
}

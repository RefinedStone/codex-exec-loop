use super::*;

pub(super) fn validate_observation(
    current: &StoredTaskDeliveryEvidence,
    observation: &TaskDeliveryEvidenceObservation,
) -> Result<()> {
    observation.validate()?;
    if observation.evidence_revision != current.evidence_revision
        || observation.scope.as_ref() != current.task.as_ref().map(|task| &task.scope)
    {
        return Err(TaskDeliveryEvidenceError::Conflict.into());
    }
    if let Some(source) = &observation.source {
        if source.base_commit_sha != current.queue.source_base_commit_sha
            || source.source_commit_sha != current.queue.source_commit_sha
        {
            return Err(TaskDeliveryEvidenceError::Conflict.into());
        }
        if let Some(integration) = &source.integration
            && (current.queue.integration_base_commit_sha.as_deref()
                != Some(integration.base_commit_sha.as_str())
                || current.queue.integration_commit_sha.as_deref()
                    != Some(integration.result_commit_sha.as_str()))
        {
            return Err(TaskDeliveryEvidenceError::Conflict.into());
        }
    }
    if !observation.satisfied() {
        return Ok(());
    }
    let task = current
        .task
        .as_ref()
        .ok_or(TaskDeliveryEvidenceError::Invalid)?;
    let binding = current
        .queue
        .evidence_binding
        .as_ref()
        .ok_or(TaskDeliveryEvidenceError::Invalid)?;
    let source = observation
        .source
        .as_ref()
        .ok_or(TaskDeliveryEvidenceError::Invalid)?;
    if binding.scope != task.scope
        || task.leases.iter().any(|lease| {
            lease.session_key() != current.queue.session_key
                || lease.lease_generation.as_ref() != Some(&binding.lease_generation)
                || lease.slot_id != current.queue.slot_id
        })
    {
        return Err(TaskDeliveryEvidenceError::Invalid.into());
    }
    let essential: Vec<_> = task
        .task
        .acceptance
        .iter()
        .filter(|criterion| criterion.essential)
        .collect();
    if essential.len() != observation.criteria.len() {
        return Err(TaskDeliveryEvidenceError::Invalid.into());
    }
    for criterion in essential {
        let row = observation
            .criteria
            .iter()
            .find(|row| row.criterion_id == criterion.id)
            .ok_or(TaskDeliveryEvidenceError::Invalid)?;
        let event_id = row
            .receipt_event_id
            .as_ref()
            .ok_or(TaskDeliveryEvidenceError::Invalid)?;
        match row.state {
            TaskDeliveryCriterionState::Waived => {
                if !task.waivers.iter().any(|receipt| {
                    receipt.event_id == *event_id
                        && receipt.waiver.scope == task.scope
                        && receipt.waiver.criterion_id == criterion.id
                }) {
                    return Err(TaskDeliveryEvidenceError::Invalid.into());
                }
            }
            TaskDeliveryCriterionState::HostPassed => {
                if task.evidence.iter().any(|receipt| {
                    let evidence = &receipt.evidence;
                    evidence.attempt.scope == task.scope
                        && evidence.attempt.criterion_id == criterion.id
                        && evidence.attempt.session_key == current.queue.session_key
                        && evidence.recorded_tree.tree_sha == source.source_tree_sha
                        && matches!(
                            evidence.status,
                            TaskEvidenceStatus::Fail | TaskEvidenceStatus::Blocked
                        )
                }) {
                    return Err(TaskDeliveryEvidenceError::Invalid.into());
                }
                let stored = task
                    .host_verifications
                    .iter()
                    .find(|stored| stored.attempt.criterion_id == criterion.id)
                    .ok_or(TaskDeliveryEvidenceError::Invalid)?;
                let receipt = stored
                    .receipt
                    .as_ref()
                    .ok_or(TaskDeliveryEvidenceError::Invalid)?;
                let manifest = stored
                    .manifest
                    .as_ref()
                    .ok_or(TaskDeliveryEvidenceError::Invalid)?;
                if !receipt.passed()
                    || receipt.event_id != *event_id
                    || receipt.attempt != stored.attempt
                    || receipt.manifest.as_ref() != Some(manifest)
                    || stored.attempt.scope != task.scope
                    || stored.attempt.session_key != current.queue.session_key
                    || stored.attempt.lease_generation != binding.lease_generation
                    || criterion.verification_recipe != Some(stored.attempt.recipe)
                    || manifest.base_commit_sha != source.base_commit_sha
                    || manifest.base_tree_sha != source.base_tree_sha
                    || manifest.source.tree_sha != source.source_tree_sha
                {
                    return Err(TaskDeliveryEvidenceError::Invalid.into());
                }
            }
            TaskDeliveryCriterionState::Blocked => {
                return Err(TaskDeliveryEvidenceError::Invalid.into());
            }
        }
    }
    Ok(())
}

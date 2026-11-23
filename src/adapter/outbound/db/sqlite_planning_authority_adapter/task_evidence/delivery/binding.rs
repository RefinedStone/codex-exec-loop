use super::*;

fn initial_binding(
    transaction: &Transaction<'_>,
    queue: &PlanningAuthorityDistributorQueueRecord,
) -> Result<Option<TaskEvidenceDeliveryBinding>> {
    let Some(scope) = read_scope(transaction, &queue.task_id)? else {
        return Ok(None);
    };
    let task = current_task(transaction, &queue.task_id)?;
    if task.acceptance_revision != scope.acceptance_revision
        || criteria_sha256(&task.acceptance)? != scope.criteria_sha256
    {
        return Ok(None);
    }
    let lease: Option<String> = transaction
        .query_row(
            "SELECT content FROM runtime_slot_leases WHERE slot_id=?1",
            [&queue.slot_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(json) = lease else { return Ok(None) };
    let lease: ParallelModeSlotLeaseSnapshot = serde_json::from_str(&json)?;
    let Some(generation) = lease.lease_generation.as_ref() else {
        return Ok(None);
    };
    if lease.state != ParallelModeSlotLeaseState::Running
        || lease.task_id != queue.task_id
        || lease.slot_id != queue.slot_id
        || lease.session_key() != queue.session_key
        || lease.branch_name != queue.effective_source_branch()
        || lease.worktree_path != queue.worktree_path
        || lease
            .delivery_target
            .as_ref()
            .is_none_or(|target| target.integration_base_commit_sha != queue.source_base_commit_sha)
    {
        return Ok(None);
    }
    let binding = TaskEvidenceDeliveryBinding {
        scope,
        lease_generation: generation.clone(),
    };
    binding.validate()?;
    Ok(Some(binding))
}

/// General queue writes may advance delivery state, but cannot manufacture evidence authority.
pub(in crate::adapter::outbound::db::sqlite_planning_authority_adapter) fn protect_queue_evidence(
    transaction: &Transaction<'_>,
    incoming: &PlanningAuthorityDistributorQueueRecord,
) -> Result<PlanningAuthorityDistributorQueueRecord> {
    let mut next = incoming.clone();
    if let Some(previous) = read_queue(transaction, &incoming.queue_item_id)? {
        if incoming.evidence_binding.is_some()
            && incoming.evidence_binding != previous.evidence_binding
        {
            return Err(TaskDeliveryEvidenceError::Conflict.into());
        }
        if previous.evidence_binding.is_some()
            && (previous.task_id != incoming.task_id
                || previous.session_key != incoming.session_key
                || previous.slot_id != incoming.slot_id
                || previous.source_base_commit_sha != incoming.source_base_commit_sha
                || previous.source_commit_sha != incoming.source_commit_sha
                || previous.source_branch != incoming.source_branch)
        {
            return Err(TaskDeliveryEvidenceError::Conflict.into());
        }
        next.evidence_binding = previous.evidence_binding;
        next.evidence_observations = previous.evidence_observations;
        next.evidence_wait = if incoming.queue_state == ParallelModeQueueItemState::Blocked
            && previous.queue_state == ParallelModeQueueItemState::Blocked
            && incoming.integration_note == previous.integration_note
        {
            previous.evidence_wait
        } else {
            None
        };
    } else {
        if !incoming.evidence_observations.is_empty() || incoming.evidence_wait.is_some() {
            return Err(TaskDeliveryEvidenceError::Invalid.into());
        }
        let binding = initial_binding(transaction, incoming)?;
        if incoming.evidence_binding.is_some() && incoming.evidence_binding != binding {
            return Err(TaskDeliveryEvidenceError::Conflict.into());
        }
        next.evidence_binding = binding;
        next.evidence_observations.clear();
        next.evidence_wait = None;
    }
    Ok(next)
}

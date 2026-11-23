use super::*;

pub(super) fn snapshot(
    transaction: &Transaction<'_>,
    expected: &PlanningAuthorityDistributorQueueRecord,
    stage: TaskEvidenceDeliveryStage,
) -> Result<StoredTaskDeliveryEvidence> {
    let queue = read_queue(transaction, &expected.queue_item_id)?
        .ok_or(TaskDeliveryEvidenceError::Missing)?;
    if queue != *expected {
        return Err(TaskDeliveryEvidenceError::Conflict.into());
    }
    let planning_revision =
        read_metadata_i64_connection(transaction, "planning_revision")?.unwrap_or(0);
    let evidence_revision: u64 = transaction.query_row(
        "SELECT revision FROM task_evidence_state WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if evidence_revision > MAX_TASK_EVIDENCE_REVISION {
        return Err(TaskDeliveryEvidenceError::Unavailable.into());
    }
    let has_task: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM planning_tasks WHERE task_id=?1)",
        [&queue.task_id],
        |row| row.get(0),
    )?;
    // Old unsupported scopes remain a visible absence of evidence in Observe mode.
    let task = if has_task && read_scope(transaction, &queue.task_id)?.is_some() {
        Some(super::super::queries::task_snapshot(
            transaction,
            &queue.task_id,
        )?)
    } else {
        None
    };
    let permit = work_orders::check_work_order_permit_in_transaction(
        transaction,
        &queue.task_id,
        permit_stage(stage),
        Some(&queue.effective_source_commit_sha()),
    )?;
    let requires_preservation =
        continuation::task_preserved_in_connection(transaction, &queue.task_id)?;
    Ok(StoredTaskDeliveryEvidence {
        planning_revision,
        evidence_revision,
        queue,
        task,
        permit,
        requires_preservation,
    })
}

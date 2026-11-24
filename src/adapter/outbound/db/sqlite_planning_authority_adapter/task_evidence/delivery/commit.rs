use super::*;

fn require_operation(
    transaction: &Transaction<'_>,
    queue: &PlanningAuthorityDistributorQueueRecord,
    stage: TaskEvidenceDeliveryStage,
    operation: &TaskDeliveryEvidenceOperation,
) -> Result<()> {
    let owner: Option<(String, String)> = transaction.query_row(
        "SELECT owner_token,claimed_at FROM runtime_claims WHERE claim_kind=?1 AND scope_key=?2",
        params![DISTRIBUTOR_QUEUE_CLAIM_KIND, queue.queue_item_id],
        |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    match operation {
        TaskDeliveryEvidenceOperation::Effect { owner_token } => {
            if owner_token.trim().is_empty()
                || owner.as_ref().is_none_or(|(owner, at)| {
                    owner != owner_token || runtime_projection::claim_is_stale(at)
                })
                || !matches!(
                    queue.queue_state,
                    ParallelModeQueueItemState::Queued
                        | ParallelModeQueueItemState::Pushing
                        | ParallelModeQueueItemState::PrPending
                        | ParallelModeQueueItemState::MergePending
                        | ParallelModeQueueItemState::Integrating
                )
            {
                return Err(TaskDeliveryEvidenceError::ClaimLost.into());
            }
        }
        TaskDeliveryEvidenceOperation::Recheck => {
            if queue.queue_state != ParallelModeQueueItemState::Blocked
                || queue.evidence_wait != Some(stage)
            {
                return Err(TaskDeliveryEvidenceError::Invalid.into());
            }
            if owner
                .as_ref()
                .is_some_and(|(_, at)| !runtime_projection::claim_is_stale(at))
            {
                return Err(TaskDeliveryEvidenceError::ClaimLost.into());
            }
            runtime_projection::clear_stale_runtime_claim(
                transaction,
                DISTRIBUTOR_QUEUE_CLAIM_KIND,
                &queue.queue_item_id,
            )?;
        }
    }
    Ok(())
}

fn require_control(
    transaction: &Transaction<'_>,
    current: &StoredTaskDeliveryEvidence,
    stage: TaskEvidenceDeliveryStage,
) -> Result<()> {
    runtime_projection::ensure_task_not_retired(
        transaction,
        &current.queue.task_id,
        "task evidence delivery",
    )?;
    let has_task: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM planning_tasks WHERE task_id=?1)",
        [&current.queue.task_id],
        |row| row.get(0),
    )?;
    if current.requires_preservation
        || matches!(current.permit, PlanningWorkPermitDecision::Denied(_))
        || (stage == TaskEvidenceDeliveryStage::Integrate
            && current.permit.requires_human_review()
            && current.queue.pull_request_number.is_none())
        || (has_task
            && !matches!(
                current_task(transaction, &current.queue.task_id)?.status,
                TaskStatus::Ready | TaskStatus::InProgress | TaskStatus::Done
            ))
    {
        return Err(TaskDeliveryEvidenceError::ControlDenied.into());
    }
    Ok(())
}

pub(super) fn commit(
    workspace: &str,
    value: &TaskDeliveryEvidenceObservationCommit,
) -> Result<TaskDeliveryEvidenceObservationCommitResult> {
    value.observation.validate()?;
    let mut connection = connection(workspace)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = read::snapshot(&transaction, &value.expected.queue, value.observation.stage)?;
    if current != value.expected {
        return Err(TaskDeliveryEvidenceError::Conflict.into());
    }
    require_operation(
        &transaction,
        &current.queue,
        value.observation.stage,
        &value.operation,
    )?;
    require_control(&transaction, &current, value.observation.stage)?;
    proof::validate_observation(&current, &value.observation)?;

    let mut queue = current.queue.clone();
    let previous = queue
        .evidence_observations
        .iter()
        .position(|observation| observation.stage == value.observation.stage);
    let duplicate = previous.is_some_and(|index| {
        queue.evidence_observations[index].same_assessment_as(&value.observation)
    });
    let observation = if duplicate {
        queue.evidence_observations[previous.expect("duplicate stage")].clone()
    } else {
        if let Some(index) = previous {
            queue.evidence_observations[index] = value.observation.clone();
        } else {
            queue.evidence_observations.push(value.observation.clone());
        }
        value.observation.clone()
    };
    let integration_source_mismatch_is_operator_block = observation.mode
        == TaskEvidenceDeliveryMode::Enforce
        && observation.stage == TaskEvidenceDeliveryStage::Integrate
        && observation
            .reasons
            .contains(&TaskDeliveryEvidenceReason::IntegrationSourceMismatch);
    if integration_source_mismatch_is_operator_block {
        // The frozen source was integrated against a different base/tree. Re-reading the same
        // immutable queue record cannot make that proof valid; require an operator to rebase and
        // obtain fresh host evidence instead of creating an endlessly rechecked typed wait.
        queue.queue_state = ParallelModeQueueItemState::Blocked;
        queue.integration_state = "blocked".into();
        queue.evidence_wait = None;
        queue.retry_not_before = None;
        queue.recovery_note = Some(
            "operator must rebase the integration source and obtain fresh host evidence".into(),
        );
        queue.integration_note =
            "operator required: integration source changed after dispatch; rebase and revalidate before delivery".into();
    } else {
        match value.operation {
            TaskDeliveryEvidenceOperation::Effect { .. } if !observation.allows_delivery() => {
                queue.queue_state = ParallelModeQueueItemState::Blocked;
                queue.integration_state = "blocked".into();
                queue.evidence_wait = Some(observation.stage);
                queue.integration_note = format!(
                    "task evidence waiting: {} / {}",
                    observation.stage.label(),
                    observation
                        .reasons
                        .first()
                        .copied()
                        .unwrap_or(TaskDeliveryEvidenceReason::CriteriaNotMet)
                        .label()
                );
            }
            TaskDeliveryEvidenceOperation::Recheck if observation.allows_delivery() => {
                queue.queue_state = ParallelModeQueueItemState::Queued;
                queue.integration_state = "queued".into();
                queue.evidence_wait = None;
                queue.retry_not_before = None;
                queue.recovery_note =
                    Some("task evidence wait rearmed for current delivery checks".into());
                queue.integration_note =
                    "task evidence wait cleared; queued for current delivery checks".into();
            }
            _ => {}
        }
    }
    let changed = queue != current.queue;
    if changed {
        queue.updated_at = value.observation.observed_at.clone();
        let original_json: String = transaction.query_row(
            "SELECT content FROM runtime_distributor_queue WHERE queue_item_id=?1",
            [&queue.queue_item_id],
            |row| row.get(0),
        )?;
        let json = serde_json::to_string(&queue)?;
        let rows = transaction.execute(
            "UPDATE runtime_distributor_queue SET queue_state=?2,updated_at=?3,content=?4
             WHERE queue_item_id=?1 AND content=?5",
            params![
                queue.queue_item_id,
                queue.queue_state.label(),
                queue.updated_at,
                json,
                original_json
            ],
        )?;
        if rows != 1 {
            return Err(TaskDeliveryEvidenceError::Conflict.into());
        }
        runtime_projection::append_runtime_event(
            &transaction,
            if duplicate {
                "task_delivery_evidence_wait_changed"
            } else {
                "task_delivery_evidence_observed"
            },
            "task_delivery_evidence",
            &format!("{}:{}", queue.queue_item_id, observation.stage.label()),
            "task delivery evidence checked against the frozen source",
            &serde_json::to_string(&observation)?,
        )?;
    }
    transaction.commit()?;
    Ok(TaskDeliveryEvidenceObservationCommitResult {
        effect_allowed: matches!(
            value.operation,
            TaskDeliveryEvidenceOperation::Effect { .. }
        ) && observation.allows_delivery(),
        updated_queue: queue,
        observation,
        changed,
    })
}

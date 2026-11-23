use super::*;

pub(super) fn reserve_entry(transaction: &Transaction<'_>, artifact: bool) -> Result<()> {
    let (entries, bytes, reserved): (u64,u64,u64) = transaction.query_row(
        "SELECT entry_count,artifact_bytes,reserved_bytes FROM task_evidence_state WHERE singleton=1", [],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
    let entries = entries
        .checked_add(1)
        .filter(|value| *value <= MAX_TASK_EVIDENCE_RECEIPTS)
        .ok_or(TaskEvidenceError::Limit)?;
    let reserved = reserved
        .checked_add(if artifact {
            MAX_TASK_EVIDENCE_ARTIFACT_BYTES
        } else {
            0
        })
        .ok_or(TaskEvidenceError::Limit)?;
    if bytes
        .checked_add(reserved)
        .is_none_or(|total| total > MAX_TASK_EVIDENCE_TOTAL_BYTES)
    {
        return Err(TaskEvidenceError::Limit.into());
    }
    transaction.execute(
        "UPDATE task_evidence_state SET entry_count=?1,reserved_bytes=?2 WHERE singleton=1",
        params![entries as i64, reserved as i64],
    )?;
    Ok(())
}

pub(super) fn settle_artifact(
    transaction: &Transaction<'_>,
    artifact: &TaskEvidenceArtifact,
) -> Result<()> {
    artifact.validate()?;
    let (bytes, reserved): (u64, u64) = transaction.query_row(
        "SELECT artifact_bytes,reserved_bytes FROM task_evidence_state WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let reserved = reserved
        .checked_sub(MAX_TASK_EVIDENCE_ARTIFACT_BYTES)
        .ok_or(TaskEvidenceError::Unavailable)?;
    let bytes = bytes
        .checked_add(artifact.size_bytes)
        .ok_or(TaskEvidenceError::Limit)?;
    if bytes
        .checked_add(reserved)
        .is_none_or(|total| total > MAX_TASK_EVIDENCE_TOTAL_BYTES)
    {
        return Err(TaskEvidenceError::Limit.into());
    }
    transaction.execute(
        "UPDATE task_evidence_state SET artifact_bytes=?1,reserved_bytes=?2 WHERE singleton=1",
        params![bytes as i64, reserved as i64],
    )?;
    Ok(())
}

pub(super) fn release_artifact_reservation(transaction: &Transaction<'_>) -> Result<()> {
    let reserved: u64 = transaction.query_row(
        "SELECT reserved_bytes FROM task_evidence_state WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    let reserved = reserved
        .checked_sub(MAX_TASK_EVIDENCE_ARTIFACT_BYTES)
        .ok_or(TaskEvidenceError::Unavailable)?;
    transaction.execute(
        "UPDATE task_evidence_state SET reserved_bytes=?1 WHERE singleton=1",
        [reserved as i64],
    )?;
    Ok(())
}

pub(super) fn prepare(
    workspace: &str,
    commit: &TaskEvidencePrepareCommit,
) -> Result<TaskEvidenceWriteResult<TaskEvidenceAttempt>> {
    validate_evidence_request_id(&commit.request_id)?;
    commit.scope.validate()?;
    validate_evidence_identifier(&commit.criterion_id)?;
    commit.prepared_tree.validate()?;
    let fingerprint = digest(&(&commit.scope, &commit.criterion_id))?;
    let mut connection = connection(workspace)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let task = require_current_scope(&transaction, &commit.scope, &commit.criterion_id)?;
    require_running_lease(&transaction, &task, &commit.lease)?;
    let prior: Option<(String, String)> = transaction
        .query_row(
            "SELECT attempt_id,prepare_fingerprint FROM task_evidence_attempts WHERE request_id=?1",
            [&commit.request_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((id, old_fingerprint)) = prior {
        if old_fingerprint != fingerprint {
            return Err(TaskEvidenceError::Conflict.into());
        }
        let old = read_attempt(&transaction, &id)?.ok_or(TaskEvidenceError::Unavailable)?;
        if !old.lease.same_generation_as(&commit.lease) {
            return Err(TaskEvidenceError::LeaseChanged.into());
        }
        if old.attempt.prepared_tree != commit.prepared_tree {
            return Err(TaskEvidenceError::Stale.into());
        }
        transaction.commit()?;
        return Ok(TaskEvidenceWriteResult {
            value: old.attempt,
            changed: false,
        });
    }
    let ordinal =
        verification::next_ordinal(&transaction, &commit.scope.scope_id, &commit.criterion_id)?;
    let attempt = TaskEvidenceAttempt {
        attempt_id: random_token()?,
        request_id: commit.request_id.clone(),
        scope: commit.scope.clone(),
        criterion_id: commit.criterion_id.clone(),
        attempt: ordinal,
        slot_id: commit.lease.slot_id.clone(),
        lease_generation: commit
            .lease
            .lease_generation
            .clone()
            .ok_or(TaskEvidenceError::LeaseChanged)?,
        session_key: commit.lease.session_key(),
        prepared_tree: commit.prepared_tree.clone(),
        prepared_at: commit.at.clone(),
    };
    attempt.validate()?;
    reserve_entry(&transaction, true)?;
    transaction.execute(
        "INSERT INTO task_evidence_attempts(request_id,attempt_id,task_id,scope_id,criterion_id,ordinal,prepare_fingerprint,lease_json,attempt_json)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![attempt.request_id,attempt.attempt_id,attempt.scope.task_id,attempt.scope.scope_id,attempt.criterion_id,
            attempt.attempt,fingerprint,serde_json::to_string(&commit.lease)?,serde_json::to_string(&attempt)?])?;
    bump_revision(&transaction)?;
    transaction.commit()?;
    Ok(TaskEvidenceWriteResult {
        value: attempt,
        changed: true,
    })
}

pub(super) fn record(
    workspace: &str,
    commit: &TaskEvidenceRecordCommit,
) -> Result<TaskEvidenceWriteResult<TaskEvidenceReceipt>> {
    commit.attempt.validate()?;
    commit.artifact.validate()?;
    commit.recorded_tree.validate()?;
    let note = normalize_evidence_note(commit.note.as_deref())?;
    let fingerprint = digest(&(
        commit.status,
        &commit.artifact,
        &commit.recorded_tree,
        &note,
    ))?;
    let mut connection = connection(workspace)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored = read_attempt(&transaction, &commit.attempt.attempt_id)?
        .ok_or(TaskEvidenceError::Missing)?;
    if stored.attempt != commit.attempt || !stored.lease.same_generation_as(&commit.lease) {
        return Err(TaskEvidenceError::Conflict.into());
    }
    if let Some(receipt) = stored.receipt {
        let old_fingerprint: String = transaction.query_row(
            "SELECT record_fingerprint FROM task_evidence_attempts WHERE attempt_id=?1",
            [&commit.attempt.attempt_id],
            |row| row.get(0),
        )?;
        if old_fingerprint != fingerprint {
            return Err(TaskEvidenceError::Conflict.into());
        }
        // Re-delivering an immutable past receipt does not apply it to a new scope or source.
        transaction.commit()?;
        return Ok(TaskEvidenceWriteResult {
            value: receipt,
            changed: false,
        });
    }
    let task = require_current_scope(
        &transaction,
        &commit.attempt.scope,
        &commit.attempt.criterion_id,
    )?;
    require_running_lease(&transaction, &task, &commit.lease)?;
    if commit.recorded_tree != stored.attempt.prepared_tree {
        return Err(TaskEvidenceError::Stale.into());
    }
    settle_artifact(&transaction, &commit.artifact)?;
    let receipt = TaskEvidenceReceipt {
        event_id: format!("task-evidence-recorded:{}", commit.attempt.attempt_id),
        evidence_revision: bump_revision(&transaction)?,
        evidence: TaskCriterionEvidence {
            attempt: stored.attempt,
            status: commit.status,
            artifact: commit.artifact.clone(),
            recorded_tree: commit.recorded_tree.clone(),
            note,
            by: commit.by.clone(),
            recorded_at: commit.at.clone(),
        },
    };
    receipt.validate()?;
    let changed = transaction.execute(
        "UPDATE task_evidence_attempts SET record_fingerprint=?1,record_revision=?2,receipt_json=?3 WHERE attempt_id=?4 AND receipt_json IS NULL",
        params![fingerprint,receipt.evidence_revision as i64,serde_json::to_string(&receipt)?,commit.attempt.attempt_id])?;
    if changed != 1 {
        return Err(TaskEvidenceError::Conflict.into());
    }
    insert_event(
        &transaction,
        &TaskEvidenceEvent::Recorded {
            receipt: Box::new(receipt.clone()),
        },
    )?;
    transaction.commit()?;
    Ok(TaskEvidenceWriteResult {
        value: receipt,
        changed: true,
    })
}

pub(super) fn waive(
    workspace: &str,
    commit: &TaskCriterionWaiverCommit,
) -> Result<TaskEvidenceWriteResult<TaskCriterionWaiverReceipt>> {
    validate_evidence_request_id(&commit.request_id)?;
    commit.scope.validate()?;
    validate_evidence_identifier(&commit.criterion_id)?;
    validate_evidence_request_id(&commit.confirmation_ref)?;
    let (reason, reason_digest) = normalize_waiver_reason(&commit.reason)?;
    let fingerprint = digest(&(&commit.scope, &commit.criterion_id, &reason))?;
    let mut connection = connection(workspace)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let prior: Option<(String, String)> = transaction
        .query_row(
            "SELECT fingerprint,receipt_json FROM task_criterion_waivers WHERE request_id=?1",
            [&commit.request_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((old_fingerprint, json)) = prior {
        if old_fingerprint != fingerprint {
            return Err(TaskEvidenceError::Conflict.into());
        }
        let receipt: TaskCriterionWaiverReceipt = serde_json::from_str(&json)?;
        receipt.validate()?;
        if receipt.waiver.request_id != commit.request_id {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        transaction.commit()?;
        return Ok(TaskEvidenceWriteResult {
            value: receipt,
            changed: false,
        });
    }
    require_current_scope(&transaction, &commit.scope, &commit.criterion_id)?;
    reserve_entry(&transaction, false)?;
    let receipt = TaskCriterionWaiverReceipt {
        event_id: format!("task-criterion-waived:{}", digest(&commit.request_id)?),
        evidence_revision: bump_revision(&transaction)?,
        waiver: TaskCriterionWaiver {
            request_id: commit.request_id.clone(),
            scope: commit.scope.clone(),
            criterion_id: commit.criterion_id.clone(),
            reason,
            reason_digest,
            confirmation_ref: commit.confirmation_ref.clone(),
            recorded_at: commit.at.clone(),
        },
    };
    receipt.validate()?;
    transaction.execute(
        "INSERT INTO task_criterion_waivers(request_id,task_id,scope_id,criterion_id,fingerprint,evidence_revision,receipt_json)
        VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![commit.request_id,commit.scope.task_id,commit.scope.scope_id,commit.criterion_id,fingerprint,
            receipt.evidence_revision as i64,serde_json::to_string(&receipt)?])?;
    insert_event(
        &transaction,
        &TaskEvidenceEvent::Waived {
            receipt: Box::new(receipt.clone()),
        },
    )?;
    transaction.commit()?;
    Ok(TaskEvidenceWriteResult {
        value: receipt,
        changed: true,
    })
}

impl TaskEvidenceRepositoryPort for SqlitePlanningAuthorityAdapter {
    fn load_current_snapshot(
        &self,
        workspace: &str,
        query: &TaskEvidenceSnapshotQuery,
    ) -> Result<TaskEvidenceStoredSnapshot> {
        queries::snapshot(workspace, query)
    }

    fn load_attempt(
        &self,
        workspace: &str,
        attempt_id: &str,
    ) -> Result<Option<StoredTaskEvidenceAttempt>> {
        validate_evidence_token(attempt_id)?;
        read_attempt(&connection(workspace)?, attempt_id)
    }

    fn prepare_attempt(
        &self,
        workspace: &str,
        commit: &TaskEvidencePrepareCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceAttempt>> {
        prepare(workspace, commit)
    }

    fn record_evidence(
        &self,
        workspace: &str,
        commit: &TaskEvidenceRecordCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceReceipt>> {
        record(workspace, commit)
    }

    fn waive_criterion(
        &self,
        workspace: &str,
        commit: &TaskCriterionWaiverCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskCriterionWaiverReceipt>> {
        waive(workspace, commit)
    }

    fn pending_events(&self, workspace: &str, limit: usize) -> Result<Vec<TaskEvidenceEvent>> {
        queries::pending(workspace, limit)
    }

    fn acknowledge_event(&self, workspace: &str, event_id: &str) -> Result<()> {
        validate_evidence_request_id(event_id)?;
        let mut connection = connection(workspace)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        queries::require_event(&transaction, event_id)?;
        transaction.execute(
            "UPDATE task_evidence_events SET acknowledged=1 WHERE event_id=?1",
            [event_id],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

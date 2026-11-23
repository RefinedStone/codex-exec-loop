use super::*;

fn finish_in_transaction(
    transaction: &Transaction<'_>,
    run: &Run,
    result: &TaskVerificationResult,
    artifact: Option<&TaskEvidenceArtifact>,
    at: &str,
) -> Result<TaskVerificationReceipt> {
    let receipt = TaskVerificationReceipt {
        event_id: format!(
            "task-verification-finished:{}",
            run.stored.attempt.attempt_id
        ),
        evidence_revision: bump_revision(transaction)?,
        attempt: run.stored.attempt.clone(),
        manifest: run.stored.manifest.clone(),
        result: result.clone(),
        artifact: artifact.cloned(),
        finished_at: at.to_owned(),
    };
    receipt.validate()?;
    if let Some(artifact) = artifact {
        writes::settle_artifact(transaction, artifact)?;
    } else if matches!(result, TaskVerificationResult::NotRun { .. }) {
        // NotRun is a definitive pre-execution outcome: no capture artifact can
        // have been created, so return the reservation while retaining the receipt.
        writes::release_artifact_reservation(transaction)?;
    }
    // Executed without a sealed artifact and interrupted recovery are uncertain:
    // retain their reservation because a partial capture may still occupy disk.
    let fingerprint = digest(&(result, artifact))?;
    let changed = transaction.execute(
        "UPDATE task_verification_runs SET phase='finished',finish_fingerprint=?2,receipt_json=?3,finished_revision=?4
         WHERE attempt_id=?1 AND receipt_json IS NULL",
        params![run.stored.attempt.attempt_id,fingerprint,serde_json::to_string(&receipt)?,receipt.evidence_revision as i64],
    )?;
    if changed != 1 {
        return Err(TaskEvidenceError::Conflict.into());
    }
    insert_event(
        transaction,
        &TaskEvidenceEvent::VerificationFinished {
            receipt: Box::new(receipt.clone()),
        },
    )?;
    Ok(receipt)
}

pub(super) fn finish(
    workspace: &str,
    commit: &TaskVerificationFinishCommit,
) -> Result<TaskEvidenceWriteResult<TaskVerificationReceipt>> {
    validate_time(&commit.at)?;
    let mut connection = connection(workspace)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let run =
        read_run(&transaction, &commit.execution.attempt_id)?.ok_or(TaskEvidenceError::Missing)?;
    require_execution(&run, &commit.execution)?;
    let fingerprint = digest(&(&commit.result, commit.artifact.as_ref()))?;
    if let Some(receipt) = &run.stored.receipt {
        if run.finish_fingerprint.as_deref() != Some(fingerprint.as_str()) {
            return Err(TaskEvidenceError::Conflict.into());
        }
        transaction.commit()?;
        return Ok(TaskEvidenceWriteResult {
            value: receipt.clone(),
            changed: false,
        });
    }
    require_owner(&transaction, &commit.execution.owner_token)?;
    // Current scope/lease/control may have changed after execution. Persist the terminal fact
    // without re-authorizing it; current query/dispatch fences determine its applicability.
    if matches!(commit.result, TaskVerificationResult::Executed { .. })
        && run.stored.phase != TaskVerificationPhase::Running
    {
        return Err(TaskEvidenceError::Conflict.into());
    }
    let value = finish_in_transaction(
        &transaction,
        &run,
        &commit.result,
        commit.artifact.as_ref(),
        &commit.at,
    )?;
    transaction.commit()?;
    Ok(TaskEvidenceWriteResult {
        value,
        changed: true,
    })
}

pub(super) fn recover(
    workspace: &str,
    owner_token: &str,
    at: &str,
) -> Result<Vec<TaskVerificationReceipt>> {
    validate_evidence_token(owner_token)?;
    validate_time(at)?;
    let mut connection = connection(workspace)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ids = {
        let mut statement = transaction.prepare(
            "SELECT attempt_id FROM task_verification_runs WHERE phase!='finished' AND owner_token!=?1 ORDER BY rowid LIMIT 2",
        )?;
        statement
            .query_map([owner_token], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if ids.len() > 1 {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    let mut receipts = Vec::new();
    for id in ids {
        let run = read_run(&transaction, &id)?.ok_or(TaskEvidenceError::Unavailable)?;
        receipts.push(finish_in_transaction(
            &transaction,
            &run,
            &TaskVerificationResult::Interrupted {},
            None,
            at,
        )?);
    }
    transaction.execute(
        "UPDATE task_verification_state SET owner_token=?1 WHERE singleton=1",
        [owner_token],
    )?;
    transaction.commit()?;
    Ok(receipts)
}

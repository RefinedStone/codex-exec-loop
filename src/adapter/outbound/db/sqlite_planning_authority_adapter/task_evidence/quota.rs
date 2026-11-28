use super::*;

pub(super) fn reserve_entry(transaction: &Transaction<'_>, artifact: bool) -> Result<()> {
    let addition = if artifact {
        MAX_TASK_EVIDENCE_ARTIFACT_BYTES
    } else {
        0
    };
    let (count, occupied): (u64, u64) = transaction.query_row(
        "SELECT entry_count, artifact_bytes+reserved_bytes FROM task_evidence_state WHERE singleton=1",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let pressure = occupied
        .checked_add(addition)
        .ok_or(TaskEvidenceError::Limit)?
        > MAX_TASK_EVIDENCE_TOTAL_BYTES;
    if (pressure || count >= MAX_TASK_EVIDENCE_RECEIPTS)
        && !super::retention::refund(transaction, pressure)?
    {
        return Err(TaskEvidenceError::Limit.into());
    }
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

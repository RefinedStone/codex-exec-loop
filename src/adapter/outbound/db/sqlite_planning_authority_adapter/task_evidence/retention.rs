//! Pressure-only retirement of unused host-verification quota. No artifact
//! path is opened or deleted, and no receipt, attempt or event is rewritten.
use super::*;

/// Called within the same IMMEDIATE transaction that will reserve the new
/// entry. Rollback restores both the old charge and its audit marker.
pub(super) fn refund(tx: &Transaction<'_>, needs_bytes: bool) -> Result<bool> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS task_evidence_retired_charges (
        attempt_id TEXT PRIMARY KEY NOT NULL,
        receipt_event_id TEXT NOT NULL UNIQUE,
        retired_at TEXT NOT NULL,
        reserved_bytes INTEGER NOT NULL CHECK(reserved_bytes >= 0)
    ) STRICT",
    )?;
    // Pending/prepared worker evidence, uncertain captures after Running,
    // real sealed bytes, and anything with a live lease/queue are ineligible.
    // A Pending interruption has no manifest: the host never bound a run and
    // could not have sent any output to its capture service.
    let candidate: Option<(String, String)> = tx.query_row(
        "SELECT r.attempt_id, r.receipt_json
         FROM task_verification_runs r JOIN planning_tasks t ON t.task_id=r.task_id
         JOIN task_evidence_events e ON e.evidence_revision=r.finished_revision
         WHERE r.phase='finished' AND r.receipt_json IS NOT NULL AND e.acknowledged=1
           AND e.event_id='task-verification-finished:'||r.attempt_id
           AND json_extract(t.content_json,'$.status') IN ('done','cancelled')
           AND julianday(json_extract(t.content_json,'$.updated_at')) <= julianday('now','-30 days')
           AND julianday(json_extract(r.receipt_json,'$.finished_at')) <= julianday('now','-30 days')
           AND json_extract(r.receipt_json,'$.artifact') IS NULL
           AND ((r.manifest_json IS NULL AND json_extract(r.receipt_json,'$.result.kind')='interrupted')
              OR (?1=0 AND json_extract(r.receipt_json,'$.result.kind')='not_run'))
           AND NOT EXISTS (SELECT 1 FROM runtime_slot_leases l WHERE json_extract(l.content,'$.task_id')=r.task_id)
           AND NOT EXISTS (SELECT 1 FROM runtime_distributor_queue q WHERE json_extract(q.content,'$.task_id')=r.task_id)
           AND NOT EXISTS (SELECT 1 FROM task_evidence_retired_charges a WHERE a.attempt_id=r.attempt_id)
         ORDER BY r.finished_revision LIMIT 1", [needs_bytes], |row| Ok((row.get(0)?, row.get(1)?))
    ).optional()?;
    let Some((id, json)) = candidate else {
        return Ok(false);
    };
    let receipt: crate::domain::planning::verification::TaskVerificationReceipt =
        serde_json::from_str(&json)?;
    receipt.validate()?;
    if receipt.attempt.attempt_id != id || receipt.artifact.is_some() {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    let reserved = match receipt.result {
        crate::domain::planning::verification::TaskVerificationResult::Interrupted {}
            if receipt.manifest.is_none() =>
        {
            MAX_TASK_EVIDENCE_ARTIFACT_BYTES
        }
        crate::domain::planning::verification::TaskVerificationResult::NotRun { .. }
            if !needs_bytes =>
        {
            0
        }
        _ => return Err(TaskEvidenceError::Unavailable.into()),
    };
    tx.execute(
        "INSERT INTO task_evidence_retired_charges VALUES (?1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?3)",
        params![id, receipt.event_id, reserved],
    )?;
    if tx.execute(
        "UPDATE task_evidence_state SET entry_count=entry_count-1, reserved_bytes=reserved_bytes-?1
         WHERE singleton=1 AND entry_count>=1 AND reserved_bytes>=?1",
        [reserved],
    )? != 1
    {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    Ok(true)
}

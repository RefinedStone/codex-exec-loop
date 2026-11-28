use super::*;

const TABLE: &str = "task_human_reviews";

pub(super) fn latest(db: &Connection, task: &TaskDefinition) -> Result<Vec<HumanEvidenceReview>> {
    if !table_exists(db, TABLE)? {
        return Ok(Vec::new());
    }
    let mut query = db.prepare(
        "SELECT receipt FROM task_human_reviews WHERE task_id=?1 AND criterion_id=?2 ORDER BY revision DESC LIMIT 1")?;
    let mut reviews = Vec::new();
    for criterion in &task.acceptance {
        if let Some(json) = query
            .query_row(params![task.id, criterion.id], |r| r.get::<_, String>(0))
            .optional()?
        {
            let review: HumanEvidenceReview = serde_json::from_str(&json)?;
            validate_evidence_token(&review.review_id)?;
            validate_evidence_token(&review.confirmation_sha256)?;
            if review.scope.task_id != task.id || review.criterion_id != criterion.id {
                return Err(TaskEvidenceError::Unavailable.into());
            }
            reviews.push(review);
        }
    }
    Ok(reviews)
}

pub(super) fn record(workspace: &str, review: &HumanEvidenceReview) -> Result<()> {
    validate_evidence_token(&review.review_id)?;
    validate_evidence_token(&review.confirmation_sha256)?;
    let mut db = connection(workspace)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_current_scope(&tx, &review.scope, &review.criterion_id)?;
    let task = queries::task_snapshot(&tx, &review.scope.task_id)?;
    if !task.task.acceptance.iter().any(|c| {
        c.id == review.criterion_id
            && c.channel != crate::domain::planning::TaskAcceptanceChannel::Command
    }) || !task.evidence.iter().any(|e| review.matches(e))
    {
        return Err(TaskEvidenceError::Stale.into());
    }
    let lease = task.leases.first().ok_or(TaskEvidenceError::LeaseChanged)?;
    require_running_lease(&tx, &task.task, lease)?;
    let record = task
        .evidence
        .iter()
        .find(|e| review.matches(e))
        .ok_or(TaskEvidenceError::Stale)?;
    if record.evidence.attempt.session_key != lease.session_key()
        || Some(&record.evidence.attempt.lease_generation) != lease.lease_generation.as_ref()
    {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    tx.execute_batch("CREATE TABLE IF NOT EXISTS task_human_reviews (
        review_id TEXT PRIMARY KEY, task_id TEXT NOT NULL, criterion_id TEXT NOT NULL,
        revision INTEGER UNIQUE NOT NULL, recorded_at TEXT NOT NULL, receipt TEXT NOT NULL) STRICT;
        CREATE INDEX IF NOT EXISTS task_human_reviews_latest ON task_human_reviews(task_id,criterion_id,revision DESC)")?;
    let revision = bump_revision(&tx)?;
    tx.execute("INSERT INTO task_human_reviews VALUES (?1,?2,?3,?4,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?5)",
        params![review.review_id, task.task.id, review.criterion_id, revision, serde_json::to_string(review)?])?;
    tx.commit()?;
    Ok(())
}

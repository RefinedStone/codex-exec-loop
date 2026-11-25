//! 사람 답변의 최초 전달권과 후속 ACK를 분리해 영속 기록한다.
use super::{
    SqliteIssueAutomationStoreAdapter, next_revision, sql_revision, transaction,
    validate_expected_revision, validate_reference_scope,
};
use crate::domain::issue_automation::IssueAutomationState;
use crate::domain::issue_automation::agent_question::{
    agent_question_reply, agent_question_targets,
};
use crate::domain::issue_automation::decision::{
    IssueDecisionAnswerBy, IssueDecisionBasis, IssueDecisionKind, IssueDecisionState,
};
use crate::domain::issue_automation::execution::IssueExecutionState;
use crate::domain::issue_automation::execution_answer::{
    IssueExecutionAnswerClaim, IssueExecutionAnswerIntent, IssueExecutionAnswerRecord,
    IssueExecutionAnswerStatus, answer_hash,
};
use crate::domain::issue_automation::policy::IssueAutomationGate;
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, Row, TransactionBehavior, params};

pub(super) const TABLE: &str = "issue_execution_answers";
pub(super) const SCHEMA: &str = r#"
CREATE TABLE issue_execution_answers (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    command_id TEXT COLLATE BINARY NOT NULL,
    work_order_id TEXT COLLATE BINARY NOT NULL,
    decision_id TEXT COLLATE BINARY NOT NULL,
    question_id TEXT COLLATE BINARY NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('unknown', 'queued', 'accepted', 'rejected_before_start')),
    record TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    PRIMARY KEY (workspace_dir, command_id),
    UNIQUE (workspace_dir, decision_id),
    FOREIGN KEY (workspace_dir, work_order_id)
        REFERENCES issue_work_checkpoints (workspace_dir, work_order_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (workspace_dir, decision_id)
        REFERENCES issue_decision_requests (workspace_dir, decision_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;

const COLUMNS: &str =
    "workspace_dir, command_id, work_order_id, decision_id, question_id, status, record, revision";
const MAX_RECORD_BYTES: usize = 64 * 1024;

pub(super) const INDEXES: [(&str, &str); 2] = [
    (
        "issue_execution_answers_live_question",
        "CREATE UNIQUE INDEX issue_execution_answers_live_question
         ON issue_execution_answers (workspace_dir, work_order_id, question_id)
         WHERE status IN ('unknown', 'queued', 'accepted')",
    ),
    (
        "issue_execution_answers_question_history",
        "CREATE INDEX issue_execution_answers_question_history
         ON issue_execution_answers (workspace_dir, work_order_id, question_id)",
    ),
];

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(SCHEMA)?;
    for (_, schema) in INDEXES {
        connection.execute_batch(schema)?;
    }
    Ok(())
}

fn status_name(status: &IssueExecutionAnswerStatus) -> &'static str {
    match status {
        IssueExecutionAnswerStatus::Unknown => "unknown",
        IssueExecutionAnswerStatus::Queued => "queued",
        IssueExecutionAnswerStatus::Accepted { .. } => "accepted",
        IssueExecutionAnswerStatus::RejectedBeforeStart { .. } => "rejected_before_start",
    }
}

fn encode(record: &IssueExecutionAnswerRecord) -> Result<String> {
    record.validate().map_err(|reason| anyhow!(reason))?;
    let encoded = serde_json::to_string(record)?;
    if encoded.len() > MAX_RECORD_BYTES {
        bail!("the issue answer receipt exceeds its storage bound");
    }
    Ok(encoded)
}

fn hydrate(row: &Row<'_>) -> Result<IssueExecutionAnswerRecord> {
    let encoded: String = row.get(6)?;
    if encoded.len() > MAX_RECORD_BYTES {
        bail!("the stored issue answer receipt exceeds its storage bound");
    }
    let record: IssueExecutionAnswerRecord = serde_json::from_str(&encoded)
        .map_err(|_| anyhow!("the stored issue answer receipt is malformed"))?;
    record.validate().map_err(|reason| anyhow!(reason))?;
    if row.get::<_, String>(0)? != record.intent.issue.project_scope.workspace_dir
        || row.get::<_, String>(1)? != record.intent.command_id
        || row.get::<_, String>(2)? != record.intent.work_order_id
        || row.get::<_, String>(3)? != record.intent.decision_id.as_str()
        || row.get::<_, String>(4)? != record.intent.question.question_id
        || row.get::<_, String>(5)? != status_name(&record.status)
        || super::read_revision(row, 7)? != record.revision
    {
        bail!("the stored issue answer identity does not match its record");
    }
    Ok(record)
}

fn read(
    connection: &Connection,
    workspace: &str,
    command_id: &str,
) -> Result<Option<IssueExecutionAnswerRecord>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM issue_execution_answers WHERE workspace_dir=?1 AND command_id=?2"
    ))?;
    let mut rows = statement.query(params![workspace, command_id])?;
    let record = rows.next()?.map(hydrate).transpose()?;
    if let Some(record) = &record {
        validate_reference_scope(workspace, &record.intent.issue)?;
        if record.intent.command_id != command_id {
            bail!("the issue answer command identity does not match");
        }
    }
    Ok(record)
}

pub(super) fn claim(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    intent: &IssueExecutionAnswerIntent,
) -> Result<IssueExecutionAnswerClaim> {
    intent.validate().map_err(|reason| anyhow!(reason))?;
    validate_reference_scope(workspace, &intent.issue)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            if let Some(record) = read(tx, workspace, &intent.command_id)? {
                if record.intent != *intent {
                    bail!("an issue answer command was reused for a different intent");
                }
                return Ok(IssueExecutionAnswerClaim {
                    record,
                    created: false,
                });
            }
            validate_question_history(tx, workspace, intent)?;
            validate_claim(tx, workspace, intent)?;
            let record = IssueExecutionAnswerRecord {
                intent: intent.clone(),
                status: IssueExecutionAnswerStatus::Unknown,
                revision: 1,
            };
            tx.execute(
                "INSERT INTO issue_execution_answers
                 (workspace_dir,command_id,work_order_id,decision_id,question_id,status,record,revision)
                 VALUES (?1,?2,?3,?4,?5,'unknown',?6,1)",
                params![
                    workspace,
                    intent.command_id,
                    intent.work_order_id,
                    intent.decision_id.as_str(),
                    intent.question.question_id,
                    encode(&record)?,
                ],
            )?;
            Ok(IssueExecutionAnswerClaim {
                record,
                created: true,
            })
        })
    })
}

fn validate_question_history(
    connection: &Connection,
    workspace: &str,
    intent: &IssueExecutionAnswerIntent,
) -> Result<()> {
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM issue_execution_answers
         WHERE workspace_dir=?1 AND work_order_id=?2 AND question_id=?3"
    ))?;
    let mut rows = statement.query(params![
        workspace,
        intent.work_order_id,
        intent.question.question_id
    ])?;
    while let Some(row) = rows.next()? {
        let previous = hydrate(row)?;
        validate_reference_scope(workspace, &previous.intent.issue)?;
        if previous.intent.issue != intent.issue
            || previous.intent.work_order_id != intent.work_order_id
            || previous.intent.question != intent.question
        {
            bail!("the worker answer question identity changed");
        }
        if !matches!(
            previous.status,
            IssueExecutionAnswerStatus::RejectedBeforeStart { .. }
        ) {
            bail!("the worker question already has an unresolved or accepted answer");
        }
        if intent.authorization_epoch <= previous.intent.authorization_epoch {
            bail!("a rejected worker answer requires a newer authorization epoch");
        }
    }
    Ok(())
}

fn validate_claim(
    connection: &Connection,
    workspace: &str,
    intent: &IssueExecutionAnswerIntent,
) -> Result<()> {
    let work = super::coordinator::read(connection, workspace, &intent.issue)?
        .ok_or_else(|| anyhow!("the issue answer work is missing"))?;
    let tracked = super::read_issue(connection, workspace, &intent.issue)?
        .ok_or_else(|| anyhow!("the issue answer subject is missing"))?;
    let flags = tracked.flags();
    if work.order.work_order_id != intent.work_order_id
        || !work.task_ids.contains(&intent.question.task_id)
        || work.current_authorization_basis().authorization_epoch != intent.authorization_epoch
        || super::intervention::authorization_epoch(connection, workspace, &intent.issue)?
            != intent.authorization_epoch
        || super::intervention::has_pending(connection, workspace, &intent.issue)?
        || !super::intervention::is_tracked(connection, workspace, &intent.issue)?
        || super::autonomy::read(connection, workspace)?
            .pending
            .is_some()
        || flags.held
        || flags.stale
        || flags.unknown
        || matches!(
            tracked.state(),
            IssueAutomationState::Cancelled
                | IssueAutomationState::Done
                | IssueAutomationState::Failed
        )
    {
        bail!("the issue answer authorization is no longer current");
    }
    let execution = super::execution::read(connection, workspace, &intent.work_order_id)?
        .ok_or_else(|| anyhow!("the issue answer execution is missing"))?;
    if execution.progress.requires_human_recovery()
        || execution.question.as_ref() != Some(&intent.question)
        || !matches!(
            execution.progress.state,
            IssueExecutionState::Running | IssueExecutionState::InReview
        )
    {
        bail!("the issue answer does not match the current execution question");
    }
    let stored = super::decision::read(connection, workspace, &intent.decision_id)?
        .ok_or_else(|| anyhow!("the issue answer decision is missing"))?;
    let request = &stored.request;
    let definition = request.definition();
    let answer = request
        .answer()
        .ok_or_else(|| anyhow!("the issue answer has no recorded human response"))?;
    if request.issue() != &intent.issue
        || request.state() != IssueDecisionState::Answered
        || definition.kind != IssueDecisionKind::AgentQuestion
        || definition.gate != IssueAutomationGate::Spec
        || definition.basis.work_order_id.as_deref() != Some(intent.work_order_id.as_str())
        || definition.basis.authorization_epoch != intent.authorization_epoch
        || definition.basis.field_hash.as_ref() != Some(&work.order.field_hash)
        || !agent_question_targets(definition, &intent.question)
        || answer.by != IssueDecisionAnswerBy::Human
        || answer.confirmation.is_none()
        || agent_question_reply(definition, answer).map(|message| answer_hash(&message))
            != Some(intent.answer_sha256.clone())
        || !super::decision::work_context_current(connection, workspace, request)?
    {
        bail!("the issue answer does not have the exact confirmed human decision");
    }
    let item = super::collection::queries::item(connection, workspace, &intent.issue)?
        .ok_or_else(|| anyhow!("the issue answer source is missing"))?;
    let triage = item
        .triage
        .as_ref()
        .ok_or_else(|| anyhow!("the issue answer assessment is missing"))?;
    triage
        .assessment
        .validate_current(&intent.issue, item.current.as_ref(), flags.held)?;
    if !super::autonomy::permits_assessment(
        connection,
        workspace,
        triage.assessment.basis().runtime_selection_revision,
    )? {
        bail!("the issue answer runtime policy is no longer current");
    }
    let mut basis = IssueDecisionBasis::from_assessment(
        &triage.assessment,
        IssueAutomationGate::Spec,
        Some(intent.work_order_id.clone()),
    )?;
    basis.authorization_epoch = intent.authorization_epoch;
    if definition.basis != basis {
        bail!("the issue answer decision basis is no longer current");
    }
    Ok(())
}

pub(super) fn pending(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    work_order_id: &str,
) -> Result<Vec<IssueExecutionAnswerRecord>> {
    adapter.with_connection(workspace, |connection, guard| {
        let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        guard.verify_identity()?;
        let mut statement = tx.prepare(&format!(
            "SELECT {COLUMNS} FROM issue_execution_answers
             WHERE workspace_dir=?1 AND work_order_id=?2 AND status IN ('unknown','queued')
             ORDER BY command_id"
        ))?;
        let mut rows = statement.query(params![workspace, work_order_id])?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            let record = hydrate(row)?;
            validate_reference_scope(workspace, &record.intent.issue)?;
            if record.intent.work_order_id != work_order_id || !record.status.pending() {
                bail!("the pending issue answer query returned an inconsistent record");
            }
            records.push(record);
        }
        drop(rows);
        drop(statement);
        guard.verify_identity()?;
        tx.commit()?;
        Ok(records)
    })
}

pub(super) fn save(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    command_id: &str,
    expected_revision: u64,
    status: &IssueExecutionAnswerStatus,
) -> Result<Option<IssueExecutionAnswerRecord>> {
    validate_expected_revision(expected_revision)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            let Some(mut record) = read(tx, workspace, command_id)? else {
                return Ok(None);
            };
            status
                .validate_for(&record.intent)
                .map_err(|reason| anyhow!(reason))?;
            if &record.status == status {
                return Ok(Some(record));
            }
            if record.revision != expected_revision {
                return Ok(None);
            }
            if !record.status.pending() {
                bail!("a completed issue answer receipt cannot change");
            }
            record.status = status.clone();
            record.revision = next_revision(record.revision)?;
            let changed = tx.execute(
                "UPDATE issue_execution_answers SET status=?1,record=?2,revision=?3
                 WHERE workspace_dir=?4 AND command_id=?5 AND revision=?6",
                params![
                    status_name(status),
                    encode(&record)?,
                    sql_revision(record.revision)?,
                    workspace,
                    command_id,
                    sql_revision(expected_revision)?,
                ],
            )?;
            if changed != 1 {
                bail!("the issue answer receipt changed during its transaction");
            }
            Ok(Some(record))
        })
    })
}

#[cfg(test)]
mod tests;

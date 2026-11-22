use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, Row, params};
use sha2::{Digest, Sha256};

use super::super::{
    SqliteIssueAutomationStoreAdapter, enum_name, sql_revision, transaction, validate_workspace,
};
use super::{decode, encode, intents, validate_issue};
use crate::domain::issue_automation::feedback::{
    IssueFeedbackEvent, IssueFeedbackIntentRecord, IssueFeedbackIntentState, IssueFeedbackReceipt,
    validate_feedback_id,
};

const COLUMNS: &str = "event_id,provider,server_id,repository_id,issue_id,owner_workspace,intent_id,event,sequence,acknowledged";

fn event_id(event: &IssueFeedbackEvent) -> Result<String> {
    let input = serde_json::to_vec(&(
        &event.issue,
        &event.intent_id,
        event.action,
        &event.receipt,
        event.revision,
    ))?;
    Ok(format!("issue-feedback:{:x}", Sha256::digest(input)))
}

pub(super) fn append(
    connection: &Connection,
    record: &IssueFeedbackIntentRecord,
    receipt: &IssueFeedbackReceipt,
    revision: u64,
) -> Result<()> {
    record.validate().map_err(|reason| anyhow!(reason))?;
    if record.state != IssueFeedbackIntentState::Confirmed
        || record.receipt.as_ref() != Some(receipt)
    {
        bail!("only confirmed feedback can publish an event");
    }
    let mut event = IssueFeedbackEvent {
        event_id: String::new(),
        issue: record.intent.issue.clone(),
        intent_id: record.intent.intent_id.clone(),
        action: record.intent.operation.action(),
        receipt: receipt.clone(),
        revision,
    };
    event.event_id = event_id(&event)?;
    let last: i64 = connection.query_row(
        "SELECT COALESCE(MAX(sequence),0) FROM issue_feedback_outbox",
        [],
        |row| row.get(0),
    )?;
    let sequence = last
        .checked_add(1)
        .ok_or_else(|| anyhow!("the feedback outbox sequence is exhausted"))?;
    let issue = &event.issue;
    connection.execute(
        "INSERT INTO issue_feedback_outbox
         (event_id,provider,server_id,repository_id,issue_id,owner_workspace,intent_id,event,sequence,acknowledged)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0)",
        params![event.event_id, enum_name(&issue.provider)?, issue.server_id, issue.repository_id,
            issue.issue_id, issue.project_scope.workspace_dir, event.intent_id, encode(&event)?, sequence],
    )?;
    Ok(())
}

fn hydrate(connection: &Connection, row: &Row<'_>) -> Result<IssueFeedbackEvent> {
    let event: IssueFeedbackEvent = decode(row.get(7)?)?;
    validate_issue(&event.issue)?;
    validate_feedback_id(&event.intent_id).map_err(|reason| anyhow!(reason))?;
    sql_revision(event.revision)?;
    if event.revision == 0
        || row.get::<_, i64>(8)? < 1
        || !matches!(row.get::<_, i64>(9)?, 0 | 1)
        || event.event_id != row.get::<_, String>(0)?
        || event.event_id != event_id(&event)?
        || enum_name(&event.issue.provider)? != row.get::<_, String>(1)?
        || event.issue.server_id != row.get::<_, String>(2)?
        || event.issue.repository_id != row.get::<_, String>(3)?
        || event.issue.issue_id != row.get::<_, String>(4)?
        || event.issue.project_scope.workspace_dir != row.get::<_, String>(5)?
        || event.intent_id != row.get::<_, String>(6)?
    {
        bail!("the feedback event identity does not match");
    }
    let record = intents::read(connection, &event.intent_id)?
        .ok_or_else(|| anyhow!("the feedback event has no confirmed intent"))?;
    if record.state != IssueFeedbackIntentState::Confirmed
        || record.receipt.as_ref() != Some(&event.receipt)
        || record.intent.issue != event.issue
        || record.intent.operation.action() != event.action
    {
        bail!("the feedback event does not match its confirmed receipt");
    }
    event
        .receipt
        .validate_for(&record.intent)
        .map_err(|reason| anyhow!(reason))?;
    Ok(event)
}

pub(super) fn pending(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
) -> Result<Vec<IssueFeedbackEvent>> {
    validate_workspace(workspace)?;
    adapter.with_connection(workspace, |connection, guard| {
        let tx = connection.transaction()?;
        guard.verify_identity()?;
        let mut statement = tx.prepare(&format!(
            "SELECT {COLUMNS} FROM issue_feedback_outbox
             WHERE acknowledged=0 ORDER BY sequence,event_id LIMIT 100"
        ))?;
        let mut rows = statement.query([])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            let event = hydrate(&tx, row)?;
            events.push(event);
        }
        drop(rows);
        drop(statement);
        guard.verify_identity()?;
        tx.commit()?;
        Ok(events)
    })
}

pub(super) fn acknowledge(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    event_id: &str,
) -> Result<()> {
    validate_workspace(workspace)?;
    validate_feedback_id(event_id).map_err(|reason| anyhow!(reason))?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            let mut statement = tx.prepare(&format!(
                "SELECT {COLUMNS} FROM issue_feedback_outbox WHERE event_id=?1"
            ))?;
            let mut rows = statement.query([event_id])?;
            let Some(row) = rows.next()? else {
                return Ok(());
            };
            hydrate(tx, row)?;
            drop(rows);
            drop(statement);
            tx.execute(
                "UPDATE issue_feedback_outbox SET acknowledged=1
                 WHERE event_id=?1 AND acknowledged=0",
                [event_id],
            )?;
            Ok(())
        })
    })
}

//! One durable schedule, receipt and pending notification per decision.
use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use super::{SqliteIssueAutomationStoreAdapter, decision, transaction, triage};
use crate::application::port::outbound::issue_automation_store_port::IssueDecisionFence;
use crate::domain::issue_automation::decision::{
    IssueDecisionDeadline, IssueDecisionId, IssueDecisionReminder, IssueDecisionRequest,
    IssueDecisionState,
};
use crate::domain::issue_automation::triage::IssueTriageAssessment;

pub(super) const TABLE: &str = "issue_decision_reminders";
pub(super) const SCHEMA: &str = r#"
CREATE TABLE issue_decision_reminders (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    decision_id TEXT COLLATE BINARY NOT NULL,
    due_at TEXT NOT NULL,
    remind_at TEXT NOT NULL,
    event_id TEXT COLLATE BINARY NOT NULL,
    event TEXT,
    acknowledged INTEGER NOT NULL CHECK (acknowledged IN (0, 1)),
    PRIMARY KEY (workspace_dir, decision_id),
    UNIQUE (workspace_dir, event_id),
    CHECK (event IS NOT NULL OR acknowledged = 0),
    FOREIGN KEY (workspace_dir, decision_id)
        REFERENCES issue_decision_requests (workspace_dir, decision_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;
const PENDING_INDEX: &str = "issue_decision_reminders_pending";
const PENDING_SCHEMA: &str = "CREATE INDEX issue_decision_reminders_pending
    ON issue_decision_reminders (workspace_dir, acknowledged, event_id)
    WHERE event IS NOT NULL";
const EVENT_PREFIX: &str = "issue-decision-reminder:";
const MAX_PAGE_SIZE: usize = 256;

pub(super) fn schema_objects() -> [(&'static str, &'static str); 2] {
    [(TABLE, SCHEMA), (PENDING_INDEX, PENDING_SCHEMA)]
}

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    for (_, schema) in schema_objects() {
        connection.execute_batch(schema)?;
    }
    Ok(())
}

/// The caller inserts the new immutable decision in this same transaction.
pub(super) fn schedule(
    connection: &Connection,
    workspace: &str,
    request: &IssueDecisionRequest,
    remind_before_seconds: u64,
) -> Result<()> {
    let Some(due_at) = request.definition().due_at.as_deref() else {
        return Ok(());
    };
    if remind_before_seconds == 0 {
        return Ok(());
    }
    super::validate_reference_scope(workspace, request.issue())?;
    if request.state() != IssueDecisionState::Open {
        bail!("only a new open decision can schedule a reminder");
    }
    let lead = TimeDelta::try_seconds(i64::try_from(remind_before_seconds)?)
        .ok_or_else(|| anyhow!("the decision reminder interval is out of range"))?;
    let remind_at = parse_time(due_at)?
        .checked_sub_signed(lead)
        .ok_or_else(|| anyhow!("the decision reminder time is out of range"))?;
    connection.execute(
        "INSERT INTO issue_decision_reminders
         (workspace_dir,decision_id,due_at,remind_at,event_id,event,acknowledged)
         VALUES (?1,?2,?3,?4,?5,NULL,0)",
        params![
            workspace,
            request.id().as_str(),
            due_at,
            format_time(remind_at),
            event_id(workspace, request.id(), due_at)?
        ],
    )?;
    Ok(())
}

pub(super) fn batch(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    after_id: Option<&IssueDecisionId>,
    limit: usize,
) -> Result<Vec<IssueDecisionDeadline>> {
    let limit = limit.min(MAX_PAGE_SIZE);
    if limit == 0 {
        return Ok(Vec::new());
    }
    adapter.with_connection(workspace, |connection, guard| {
        let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        guard.verify_identity()?;
        let ids = {
            let mut statement = tx.prepare(
                "SELECT decision_id FROM issue_decision_requests
                 WHERE workspace_dir=?1 AND state='open'
                   AND (?2 IS NULL OR decision_id>?2)
                   AND json_extract(request,'$.definition.dueAt') IS NOT NULL
                 ORDER BY decision_id LIMIT ?3",
            )?;
            statement
                .query_map(
                    params![
                        workspace,
                        after_id.map(IssueDecisionId::as_str),
                        limit as i64
                    ],
                    |row| row.get::<_, String>(0),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut deadlines = Vec::with_capacity(ids.len());
        for id in ids {
            let id = IssueDecisionId::new(id)?;
            let record = decision::read(&tx, workspace, &id)?
                .ok_or_else(|| anyhow!("the selected decision is missing"))?;
            let schedule = read_schedule(&tx, workspace, &record.request)?;
            deadlines.push(IssueDecisionDeadline {
                record,
                remind_at: schedule.as_ref().map(|schedule| schedule.remind_at.clone()),
                reminded: schedule.is_some_and(|schedule| schedule.event.is_some()),
            });
        }
        guard.verify_identity()?;
        tx.commit()?;
        Ok(deadlines)
    })
}

pub(super) fn remind(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected: IssueDecisionFence,
    current: &IssueTriageAssessment,
    id: &IssueDecisionId,
    now_unix_ms: u64,
) -> Result<Option<IssueDecisionReminder>> {
    super::validate_expected_revision(expected.decision_revision)?;
    let now = from_unix_ms(now_unix_ms)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            let Some(record) = decision::read(tx, workspace, id)? else {
                return Ok(None);
            };
            if record.request.state() != IssueDecisionState::Open
                || record.revision != expected.decision_revision
                || decision::current_item(tx, workspace, expected.triage, current)?.is_none()
                || record.request.definition().basis
                    != decision::current_basis(tx, workspace, &record.request, current)?
                || !decision::work_context_current(tx, workspace, &record.request)?
            {
                return Ok(None);
            }
            let Some(schedule) = read_schedule(tx, workspace, &record.request)? else {
                return Ok(None);
            };
            if schedule.event.is_some()
                || now < parse_time(&schedule.remind_at)?
                || now >= parse_time(&schedule.due_at)?
            {
                return Ok(None);
            }
            let revision = triage::bump_catalog(tx, workspace)?;
            let event = IssueDecisionReminder {
                event_id: schedule.event_id,
                fact: record.request.fact(),
                due_at: schedule.due_at,
                reminded_at: format_time(now),
                revision,
            };
            let changed = tx.execute(
                "UPDATE issue_decision_reminders SET event=?1
                 WHERE workspace_dir=?2 AND decision_id=?3 AND event IS NULL",
                params![serde_json::to_string(&event)?, workspace, id.as_str()],
            )?;
            if changed != 1 {
                bail!("the decision reminder changed during its transaction");
            }
            Ok(Some(event))
        })
    })
}

pub(super) fn pending(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    limit: usize,
) -> Result<Vec<IssueDecisionReminder>> {
    let limit = limit.min(MAX_PAGE_SIZE);
    if limit == 0 {
        return Ok(Vec::new());
    }
    adapter.with_connection(workspace, |connection, guard| {
        let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        guard.verify_identity()?;
        let ids = {
            let mut statement = tx.prepare(
                "SELECT decision_id FROM issue_decision_reminders
                 WHERE workspace_dir=?1 AND acknowledged=0 AND event IS NOT NULL
                 ORDER BY event_id LIMIT ?2",
            )?;
            statement
                .query_map(params![workspace, limit as i64], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut events = Vec::with_capacity(ids.len());
        for id in ids {
            let id = IssueDecisionId::new(id)?;
            let record = decision::read(&tx, workspace, &id)?
                .ok_or_else(|| anyhow!("the reminder decision is missing"))?;
            let event = read_schedule(&tx, workspace, &record.request)?
                .and_then(|schedule| schedule.event)
                .ok_or_else(|| anyhow!("the pending decision reminder is missing"))?;
            events.push(event);
        }
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
    let Some(hash) = event_id.strip_prefix(EVENT_PREFIX) else {
        bail!("invalid decision reminder event ID");
    };
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("invalid decision reminder event ID");
    }
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            tx.execute(
                "UPDATE issue_decision_reminders SET acknowledged=1
                 WHERE workspace_dir=?1 AND event_id=?2 AND event IS NOT NULL",
                params![workspace, event_id],
            )?;
            Ok(())
        })
    })
}

struct ReminderSchedule {
    due_at: String,
    remind_at: String,
    event_id: String,
    event: Option<IssueDecisionReminder>,
}

fn read_schedule(
    connection: &Connection,
    workspace: &str,
    request: &IssueDecisionRequest,
) -> Result<Option<ReminderSchedule>> {
    let row: Option<(String, String, String, Option<String>, i64)> = connection
        .query_row(
            "SELECT due_at,remind_at,event_id,event,acknowledged FROM issue_decision_reminders
             WHERE workspace_dir=?1 AND decision_id=?2",
            params![workspace, request.id().as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let Some((due_at, remind_at, id, encoded, acknowledged)) = row else {
        return Ok(None);
    };
    if request.definition().due_at.as_deref() != Some(due_at.as_str())
        || parse_time(&remind_at)? >= parse_time(&due_at)?
        || id != event_id(workspace, request.id(), &due_at)?
        || !matches!(acknowledged, 0 | 1)
        || (encoded.is_none() && acknowledged != 0)
    {
        bail!("the decision reminder does not match its immutable request");
    }
    let event = encoded
        .map(|encoded| -> Result<IssueDecisionReminder> {
            if encoded.len() > 64 * 1024 {
                bail!("the decision reminder exceeds its bound");
            }
            let event: IssueDecisionReminder = serde_json::from_str(&encoded)
                .map_err(|_| anyhow!("invalid stored decision reminder"))?;
            let mut fact = request.fact();
            fact.state = IssueDecisionState::Open;
            fact.answer_by = None;
            fact.option = None;
            super::validate_expected_revision(event.revision)?;
            if event.event_id != id
                || event.fact != fact
                || event.due_at != due_at
                || parse_time(&event.reminded_at)? < parse_time(&remind_at)?
                || parse_time(&event.reminded_at)? >= parse_time(&due_at)?
            {
                bail!("the decision reminder event does not match its receipt");
            }
            Ok(event)
        })
        .transpose()?;
    Ok(Some(ReminderSchedule {
        due_at,
        remind_at,
        event_id: id,
        event,
    }))
}

fn event_id(workspace: &str, id: &IssueDecisionId, due_at: &str) -> Result<String> {
    let input = serde_json::to_vec(&(workspace, id, due_at))?;
    Ok(format!("{EVENT_PREFIX}{:x}", Sha256::digest(input)))
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    if value.len() > 64 {
        bail!("invalid decision reminder timestamp");
    }
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|_| anyhow!("invalid decision reminder timestamp"))
}

fn from_unix_ms(value: u64) -> Result<DateTime<Utc>> {
    DateTime::from_timestamp_millis(i64::try_from(value)?)
        .ok_or_else(|| anyhow!("the decision reminder clock is out of range"))
}

fn format_time(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

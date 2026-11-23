use super::*;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const ATTEMPTS: &str = "issue_arbiter_attempts";
const ATTEMPTS_SCHEMA: &str = r#"
CREATE TABLE issue_arbiter_attempts (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    attempt_id TEXT COLLATE BINARY NOT NULL,
    decision_id TEXT COLLATE BINARY NOT NULL,
    decision_revision INTEGER NOT NULL CHECK (decision_revision >= 1),
    utc_day INTEGER NOT NULL CHECK (utc_day >= 0),
    phase TEXT NOT NULL CHECK (phase IN ('pending', 'finished')),
    attempt TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    PRIMARY KEY (workspace_dir, attempt_id),
    UNIQUE (workspace_dir, decision_id),
    FOREIGN KEY (workspace_dir, decision_id)
        REFERENCES issue_decision_requests (workspace_dir, decision_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;
const EVENTS: &str = "issue_arbiter_finished_events";
const EVENTS_SCHEMA: &str = r#"
CREATE TABLE issue_arbiter_finished_events (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    decision_id TEXT COLLATE BINARY NOT NULL,
    decision_revision INTEGER NOT NULL CHECK (decision_revision >= 1),
    event_id TEXT COLLATE BINARY NOT NULL,
    result TEXT NOT NULL,
    acknowledged INTEGER NOT NULL CHECK (acknowledged IN (0, 1)),
    PRIMARY KEY (workspace_dir, decision_id),
    UNIQUE (workspace_dir, event_id),
    FOREIGN KEY (workspace_dir, decision_id)
        REFERENCES issue_decision_requests (workspace_dir, decision_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;
const CLOCK: &str = "issue_arbiter_budget_clock";
const CLOCK_SCHEMA: &str = r#"
CREATE TABLE issue_arbiter_budget_clock (
    workspace_dir TEXT COLLATE BINARY NOT NULL PRIMARY KEY,
    utc_day INTEGER NOT NULL CHECK (utc_day >= 0)
) STRICT, WITHOUT ROWID
"#;
const PENDING_ATTEMPTS: &str = "issue_arbiter_attempts_pending";
const PENDING_ATTEMPTS_SCHEMA: &str = "CREATE INDEX issue_arbiter_attempts_pending
    ON issue_arbiter_attempts (workspace_dir, phase, attempt_id)";
const PENDING_EVENTS: &str = "issue_arbiter_events_pending";
const PENDING_EVENTS_SCHEMA: &str = "CREATE INDEX issue_arbiter_events_pending
    ON issue_arbiter_finished_events (workspace_dir, acknowledged, event_id)";

pub(in super::super) fn schema_objects() -> [(&'static str, &'static str); 5] {
    [
        (ATTEMPTS, ATTEMPTS_SCHEMA),
        (EVENTS, EVENTS_SCHEMA),
        (CLOCK, CLOCK_SCHEMA),
        (PENDING_ATTEMPTS, PENDING_ATTEMPTS_SCHEMA),
        (PENDING_EVENTS, PENDING_EVENTS_SCHEMA),
    ]
}

pub(in super::super) fn create_schema(connection: &Connection) -> Result<()> {
    for (_, schema) in schema_objects() {
        connection.execute_batch(schema)?;
    }
    Ok(())
}

fn digest(prefix: &str, workspace: &str, id: &IssueDecisionId, revision: u64) -> Result<String> {
    let bytes = serde_json::to_vec(&(workspace, id, revision))?;
    Ok(format!("{prefix}{:x}", Sha256::digest(bytes)))
}

pub(super) fn attempt_id(workspace: &str, id: &IssueDecisionId, revision: u64) -> Result<String> {
    digest("issue-arbiter-call:", workspace, id, revision)
}

pub(super) fn event_id(workspace: &str, id: &IssueDecisionId, revision: u64) -> Result<String> {
    digest("issue-arbiter-finished:", workspace, id, revision)
}

pub(super) fn insert_attempt(
    connection: &Connection,
    workspace: &str,
    attempt: &IssueArbiterAttempt,
) -> Result<()> {
    attempt.validate().map_err(|reason| anyhow!(reason))?;
    if attempt.revision != 1 || attempt.finished_event_id.is_some() {
        bail!("a new arbiter attempt must be pending at its first revision");
    }
    connection.execute(
        "INSERT INTO issue_arbiter_attempts
         (workspace_dir,attempt_id,decision_id,decision_revision,utc_day,phase,attempt,revision)
         VALUES (?1,?2,?3,?4,?5,'pending',?6,1)",
        params![
            workspace,
            attempt.attempt_id,
            attempt.decision_id.as_str(),
            sql_revision(attempt.input_basis.decision_revision)?,
            i64::try_from(attempt.utc_day)?,
            serde_json::to_string(attempt)?
        ],
    )?;
    Ok(())
}

type AttemptRow = (String, String, i64, i64, String, String, i64);

pub(super) fn read_attempt(
    connection: &Connection,
    workspace: &str,
    attempt_id: &str,
) -> Result<Option<IssueArbiterAttempt>> {
    let row: Option<AttemptRow> = connection
        .query_row(
            "SELECT attempt_id,decision_id,decision_revision,utc_day,phase,attempt,revision
         FROM issue_arbiter_attempts WHERE workspace_dir=?1 AND attempt_id=?2",
            params![workspace, attempt_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(id, decision, decision_revision, utc_day, phase, encoded, revision)| {
            if encoded.len() > 16 * 1024 {
                bail!("the stored arbiter attempt exceeds its bound");
            }
            let attempt: IssueArbiterAttempt = serde_json::from_str(&encoded)?;
            attempt.validate().map_err(|reason| anyhow!(reason))?;
            if attempt.attempt_id != id
                || attempt.decision_id.as_str() != decision
                || attempt.input_basis.decision_revision != u64::try_from(decision_revision)?
                || attempt.utc_day != u64::try_from(utc_day)?
                || attempt.revision != u64::try_from(revision)?
                || (phase == "finished") != attempt.finished_event_id.is_some()
                || !matches!(phase.as_str(), "pending" | "finished")
                || attempt.attempt_id
                    != self::attempt_id(
                        workspace,
                        &attempt.decision_id,
                        attempt.input_basis.decision_revision,
                    )?
            {
                bail!("the stored arbiter attempt identity is inconsistent");
            }
            Ok(attempt)
        },
    )
    .transpose()
}

pub(super) fn attempt_for_decision(
    connection: &Connection,
    workspace: &str,
    id: &IssueDecisionId,
) -> Result<Option<IssueArbiterAttempt>> {
    let id: Option<String> = connection.query_row(
        "SELECT attempt_id FROM issue_arbiter_attempts WHERE workspace_dir=?1 AND decision_id=?2",
        params![workspace, id.as_str()], |row| row.get(0),
    ).optional()?;
    id.map(|id| {
        read_attempt(connection, workspace, &id)?
            .ok_or_else(|| anyhow!("the selected arbiter attempt is missing"))
    })
    .transpose()
}

pub(super) fn finish_attempt(
    connection: &Connection,
    workspace: &str,
    previous: &IssueArbiterAttempt,
    event_id: &str,
) -> Result<()> {
    let mut attempt = previous.clone();
    attempt.revision = next_revision(attempt.revision)?;
    attempt.finished_event_id = Some(event_id.to_owned());
    attempt.validate().map_err(|reason| anyhow!(reason))?;
    let changed = connection.execute(
        "UPDATE issue_arbiter_attempts SET phase='finished',attempt=?1,revision=?2
         WHERE workspace_dir=?3 AND attempt_id=?4 AND revision=?5 AND phase='pending'",
        params![
            serde_json::to_string(&attempt)?,
            sql_revision(attempt.revision)?,
            workspace,
            attempt.attempt_id,
            sql_revision(previous.revision)?
        ],
    )?;
    if changed != 1 {
        bail!("the arbiter attempt changed during its finish transaction");
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredEvent {
    source_decision_revision: u64,
    request: PersistedIssueDecisionRequest,
    decision_revision: u64,
    event: IssueArbiterFinishedEvent,
}

pub(super) fn insert_event(
    connection: &Connection,
    workspace: &str,
    source_decision_revision: u64,
    record: &StoredIssueDecision,
    event: &IssueArbiterFinishedEvent,
) -> Result<()> {
    record.validate()?;
    event.validate().map_err(|reason| anyhow!(reason))?;
    if record.request.fact() != event.fact
        || event.event_id
            != self::event_id(workspace, record.request.id(), source_decision_revision)?
    {
        bail!("the arbiter result must preserve its transaction decision snapshot");
    }
    let stored = StoredEvent {
        source_decision_revision,
        request: record.request.to_record(),
        decision_revision: record.revision,
        event: event.clone(),
    };
    connection.execute(
        "INSERT INTO issue_arbiter_finished_events
         (workspace_dir,decision_id,decision_revision,event_id,result,acknowledged)
         VALUES (?1,?2,?3,?4,?5,0)",
        params![
            workspace,
            record.request.id().as_str(),
            sql_revision(source_decision_revision)?,
            event.event_id,
            serde_json::to_string(&stored)?
        ],
    )?;
    Ok(())
}

pub(super) fn read_event(
    connection: &Connection,
    workspace: &str,
    id: &IssueDecisionId,
) -> Result<Option<(u64, IssueArbiterFinishCommit)>> {
    let row: Option<(String, i64, String)> = connection
        .query_row(
            "SELECT event_id,decision_revision,result FROM issue_arbiter_finished_events
         WHERE workspace_dir=?1 AND decision_id=?2",
            params![workspace, id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    row.map(|(event_id, source_revision, encoded)| {
        if encoded.len() > 512 * 1024 {
            bail!("the stored arbiter event exceeds its bound");
        }
        let stored: StoredEvent = serde_json::from_str(&encoded)?;
        let record = StoredIssueDecision {
            request: IssueDecisionRequest::hydrate(stored.request)?,
            revision: stored.decision_revision,
        };
        record.validate()?;
        stored.event.validate().map_err(|reason| anyhow!(reason))?;
        if record.request.id() != id
            || record.request.issue().project_scope.workspace_dir != workspace
            || stored.source_decision_revision != u64::try_from(source_revision)?
            || stored.source_decision_revision == 0
            || stored.event.event_id != event_id
            || stored.event.event_id
                != self::event_id(workspace, id, stored.source_decision_revision)?
            || stored.event.fact != record.request.fact()
        {
            bail!("the stored arbiter event identity is inconsistent");
        }
        Ok((
            stored.source_decision_revision,
            IssueArbiterFinishCommit {
                record,
                event: stored.event,
                changed: false,
            },
        ))
    })
    .transpose()
}

pub(super) fn reserve_day(
    connection: &Connection,
    workspace: &str,
    now_unix_ms: u64,
) -> Result<u64> {
    let day = i64::try_from(now_unix_ms / 86_400_000)?;
    connection.execute(
        "INSERT INTO issue_arbiter_budget_clock (workspace_dir,utc_day) VALUES (?1,?2)
         ON CONFLICT(workspace_dir) DO UPDATE SET utc_day=max(utc_day,excluded.utc_day)",
        params![workspace, day],
    )?;
    let reserved: i64 = connection.query_row(
        "SELECT utc_day FROM issue_arbiter_budget_clock WHERE workspace_dir=?1",
        [workspace],
        |row| row.get(0),
    )?;
    Ok(u64::try_from(reserved)?)
}

pub(super) fn budget_available(
    connection: &Connection,
    workspace: &str,
    settings: &IssueArbiterSettings,
    day: u64,
) -> Result<bool> {
    let running: i64 = connection.query_row(
        "SELECT count(*) FROM issue_arbiter_attempts WHERE workspace_dir=?1 AND phase='pending'",
        [workspace],
        |row| row.get(0),
    )?;
    let started: i64 = connection.query_row(
        "SELECT count(*) FROM issue_arbiter_attempts WHERE workspace_dir=?1 AND utc_day=?2",
        params![workspace, i64::try_from(day)?],
        |row| row.get(0),
    )?;
    Ok(u64::try_from(running)? < u64::from(settings.max_running)
        && u64::try_from(started)? < u64::from(settings.max_started_per_day))
}

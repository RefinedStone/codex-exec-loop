//! Exact schema objects for durable issue feedback. Network effects never hold this transaction.
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, params};

use super::super::{coordinator, decode_enum, enum_name, validate_reference_scope};
use crate::domain::issue_work::{IssueProjectScope, IssueReference};

pub(super) const STATE_TABLE: &str = "issue_feedback_state";
pub(super) const STATE_SCHEMA: &str = r#"
CREATE TABLE issue_feedback_state (
    provider TEXT COLLATE BINARY NOT NULL CHECK (provider IN ('github', 'jira')),
    server_id TEXT COLLATE BINARY NOT NULL,
    repository_id TEXT COLLATE BINARY NOT NULL,
    issue_id TEXT COLLATE BINARY NOT NULL,
    owner_workspace TEXT COLLATE BINARY NOT NULL,
    record TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    desired_revision INTEGER NOT NULL CHECK (desired_revision >= 0),
    PRIMARY KEY (provider, server_id, repository_id, issue_id)
) STRICT, WITHOUT ROWID
"#;

pub(super) const INTENT_TABLE: &str = "issue_feedback_intents";
pub(super) const INTENT_SCHEMA: &str = r#"
CREATE TABLE issue_feedback_intents (
    intent_id TEXT COLLATE BINARY NOT NULL,
    provider TEXT COLLATE BINARY NOT NULL CHECK (provider IN ('github', 'jira')),
    server_id TEXT COLLATE BINARY NOT NULL,
    repository_id TEXT COLLATE BINARY NOT NULL,
    issue_id TEXT COLLATE BINARY NOT NULL,
    owner_workspace TEXT COLLATE BINARY NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('recorded', 'sent', 'confirmed', 'needs_confirmation', 'abandoned')),
    record TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    recorded_fence TEXT NOT NULL,
    PRIMARY KEY (intent_id),
    FOREIGN KEY (provider, server_id, repository_id, issue_id)
        REFERENCES issue_feedback_state (provider, server_id, repository_id, issue_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;

pub(super) const OUTBOX_TABLE: &str = "issue_feedback_outbox";
pub(super) const OUTBOX_SCHEMA: &str = r#"
CREATE TABLE issue_feedback_outbox (
    event_id TEXT COLLATE BINARY NOT NULL,
    provider TEXT COLLATE BINARY NOT NULL CHECK (provider IN ('github', 'jira')),
    server_id TEXT COLLATE BINARY NOT NULL,
    repository_id TEXT COLLATE BINARY NOT NULL,
    issue_id TEXT COLLATE BINARY NOT NULL,
    owner_workspace TEXT COLLATE BINARY NOT NULL,
    intent_id TEXT COLLATE BINARY NOT NULL,
    event TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence >= 1),
    acknowledged INTEGER NOT NULL CHECK (acknowledged IN (0, 1)),
    PRIMARY KEY (event_id),
    UNIQUE (sequence),
    FOREIGN KEY (intent_id) REFERENCES issue_feedback_intents (intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (provider, server_id, repository_id, issue_id)
        REFERENCES issue_feedback_state (provider, server_id, repository_id, issue_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;

// A conservative remote fact, not a canonical-owner or desired-state claim.
pub(super) const GRANT_TABLE: &str = "issue_feedback_grant_entries";
pub(super) const GRANT_SCHEMA: &str = r#"
CREATE TABLE issue_feedback_grant_entries (
    provider TEXT COLLATE BINARY NOT NULL CHECK (provider IN ('github', 'jira')),
    server_id TEXT COLLATE BINARY NOT NULL,
    repository_id TEXT COLLATE BINARY NOT NULL,
    issue_id TEXT COLLATE BINARY NOT NULL,
    PRIMARY KEY (provider, server_id, repository_id, issue_id)
) STRICT, WITHOUT ROWID
"#;

pub(super) const INDEXES: [(&str, &str); 4] = [
    (
        "issue_feedback_publication_history",
        "CREATE INDEX issue_feedback_publication_history
         ON issue_feedback_intents (provider, server_id, repository_id, issue_id, status)",
    ),
    (
        "issue_feedback_one_unresolved",
        "CREATE UNIQUE INDEX issue_feedback_one_unresolved
         ON issue_feedback_intents (provider, server_id, repository_id, issue_id)
         WHERE status IN ('recorded', 'sent', 'needs_confirmation')",
    ),
    (
        "issue_feedback_pending",
        "CREATE INDEX issue_feedback_pending
         ON issue_feedback_intents (status, intent_id)
         WHERE status IN ('recorded', 'sent', 'needs_confirmation')",
    ),
    (
        "issue_feedback_outbox_pending",
        "CREATE INDEX issue_feedback_outbox_pending
         ON issue_feedback_outbox (acknowledged, sequence)",
    ),
];

pub(super) const TABLES: [(&str, &str); 4] = [
    (STATE_TABLE, STATE_SCHEMA),
    (INTENT_TABLE, INTENT_SCHEMA),
    (OUTBOX_TABLE, OUTBOX_SCHEMA),
    (GRANT_TABLE, GRANT_SCHEMA),
];

pub(super) fn create(connection: &Connection) -> Result<()> {
    for (_, schema) in TABLES {
        connection.execute_batch(schema)?;
    }
    for (_, schema) in INDEXES {
        connection.execute_batch(schema)?;
    }
    Ok(())
}

/// Older checkpoints cannot prove that no permit was ever issued. Import only
/// the remote grant-entry fact; leave all existing rows and revisions untouched.
pub(super) fn seed_existing_grants(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT workspace_dir, provider, server_id, repository_id, issue_id
         FROM issue_work_checkpoints
         ORDER BY workspace_dir, provider, server_id, repository_id, issue_id",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let workspace: String = row.get(0)?;
        let issue = IssueReference {
            provider: decode_enum(row.get(1)?)?,
            server_id: row.get(2)?,
            repository_id: row.get(3)?,
            issue_id: row.get(4)?,
            project_scope: IssueProjectScope {
                workspace_dir: workspace.clone(),
            },
        };
        validate_reference_scope(&workspace, &issue)?;
        if [&issue.server_id, &issue.repository_id, &issue.issue_id]
            .iter()
            .any(|value| value.is_empty())
        {
            bail!("the legacy issue work identity is empty");
        }
        coordinator::read(connection, &workspace, &issue)?
            .ok_or_else(|| anyhow!("the legacy issue work disappeared during migration"))?;
        connection.execute(
            "INSERT INTO issue_feedback_grant_entries
             (provider, server_id, repository_id, issue_id) VALUES (?1,?2,?3,?4)
             ON CONFLICT (provider, server_id, repository_id, issue_id) DO NOTHING",
            params![
                enum_name(&issue.provider)?,
                issue.server_id,
                issue.repository_id,
                issue.issue_id,
            ],
        )?;
    }
    Ok(())
}

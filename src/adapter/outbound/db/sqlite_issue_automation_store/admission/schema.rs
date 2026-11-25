use anyhow::Result;
use rusqlite::Connection;

const TABLE: &str = "issue_work_admissions";
const SCHEMA: &str = r#"
CREATE TABLE issue_work_admissions (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    work_order_id TEXT COLLATE BINARY NOT NULL,
    utc_day INTEGER NOT NULL CHECK (utc_day >= 0),
    admitted_at_unix_ms INTEGER NOT NULL CHECK (admitted_at_unix_ms >= 0),
    PRIMARY KEY (workspace_dir, work_order_id),
    FOREIGN KEY (workspace_dir, work_order_id)
        REFERENCES issue_work_checkpoints (workspace_dir, work_order_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;
const DAY_INDEX: &str = "issue_work_admissions_day";
const DAY_INDEX_SCHEMA: &str =
    "CREATE INDEX issue_work_admissions_day ON issue_work_admissions (workspace_dir, utc_day)";
const WAIT_TABLE: &str = "issue_start_budget_waits";
const WAIT_SCHEMA: &str = r#"
CREATE TABLE issue_start_budget_waits (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    provider TEXT COLLATE BINARY NOT NULL CHECK (provider IN ('github', 'jira', 'linear')),
    server_id TEXT COLLATE BINARY NOT NULL,
    repository_id TEXT COLLATE BINARY NOT NULL,
    issue_id TEXT COLLATE BINARY NOT NULL,
    input_fingerprint TEXT NOT NULL CHECK (length(input_fingerprint) = 64),
    authorization_epoch INTEGER NOT NULL CHECK (authorization_epoch >= 0),
    collection_epoch INTEGER NOT NULL CHECK (collection_epoch >= 0),
    waiting TEXT NOT NULL,
    PRIMARY KEY (workspace_dir, provider, server_id, repository_id, issue_id),
    FOREIGN KEY (provider, server_id, repository_id, issue_id, workspace_dir)
        REFERENCES tracked_issues (provider, server_id, repository_id, issue_id, workspace_dir)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;

pub(in super::super) fn schema_objects() -> [(&'static str, &'static str); 3] {
    [
        (TABLE, SCHEMA),
        (DAY_INDEX, DAY_INDEX_SCHEMA),
        (WAIT_TABLE, WAIT_SCHEMA),
    ]
}

pub(in super::super) fn create_schema(connection: &Connection) -> Result<()> {
    for (_, sql) in schema_objects() {
        connection.execute_batch(sql)?;
    }
    // v9 checkpoints have no trustworthy original start time. Do not invent historical
    // daily receipts. Their nonterminal reservations still count towards max_running.
    Ok(())
}

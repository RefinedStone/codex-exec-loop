//! Profile-wide stable Jira ownership. Claims are retained across cancellation and restart.
use crate::application::port::outbound::issue_source_claim_port::IssueSourceClaimPort;
use crate::domain::issue_automation::IssueAutomationBindingId;
use crate::domain::issue_work::{IssueProvider, IssueReference};
use crate::private_sqlite::{DatabaseFiles, Synchronous};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::{Path, PathBuf};

const FILES: DatabaseFiles = DatabaseFiles::new(
    "source-claims.db",
    "source-claims.db-journal",
    "source-claims.db-wal",
    "source-claims.db-shm",
);
const APPLICATION_ID: i64 = 0x414b_4943;
const SCHEMA: &str = "CREATE TABLE issue_source_claims (
    cloud_id TEXT COLLATE BINARY NOT NULL,
    project_id TEXT COLLATE BINARY NOT NULL,
    issue_id TEXT COLLATE BINARY NOT NULL,
    workspace_root TEXT COLLATE BINARY NOT NULL,
    binding_id TEXT COLLATE BINARY NOT NULL,
    PRIMARY KEY (cloud_id, issue_id),
    CHECK (length(cloud_id) BETWEEN 1 AND 64),
    CHECK (length(project_id) BETWEEN 1 AND 64),
    CHECK (length(issue_id) BETWEEN 1 AND 64),
    CHECK (length(workspace_root) BETWEEN 1 AND 16384),
    CHECK (length(binding_id) BETWEEN 1 AND 512)
) WITHOUT ROWID";

pub struct SqliteIssueSourceClaimsAdapter {
    resolve_runtime: Box<dyn Fn() -> Result<PathBuf> + Send + Sync>,
}

impl SqliteIssueSourceClaimsAdapter {
    pub fn new(resolve_runtime: impl Fn() -> Result<PathBuf> + Send + Sync + 'static) -> Self {
        Self {
            resolve_runtime: Box::new(resolve_runtime),
        }
    }
}

impl IssueSourceClaimPort for SqliteIssueSourceClaimsAdapter {
    fn claim(&self, issue: &IssueReference, binding: &IssueAutomationBindingId) -> Result<bool> {
        validate(issue, binding)?;
        let runtime = (self.resolve_runtime)()?;
        let mut opened =
            crate::private_sqlite::open_or_create_strict(&runtime, FILES, Synchronous::Extra)?;
        let (connection, guard) = opened.parts();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        guard.verify_identity()?;
        if !validate_schema(&transaction)? {
            transaction.execute_batch(SCHEMA)?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", 1)?;
            validate_schema(&transaction)?;
        }
        transaction.execute(
            "INSERT INTO issue_source_claims(cloud_id,project_id,issue_id,workspace_root,binding_id)
             VALUES(?1,?2,?3,?4,?5) ON CONFLICT(cloud_id,issue_id) DO NOTHING",
            params![issue.server_id, issue.repository_id, issue.issue_id, issue.project_scope.workspace_dir, binding.as_str()],
        )?;
        let owned = read_owner(&transaction, issue, binding)?;
        guard.verify_identity()?;
        transaction.commit()?;
        guard.verify_identity()?;
        Ok(owned)
    }

    fn owns(&self, issue: &IssueReference, binding: &IssueAutomationBindingId) -> Result<bool> {
        validate(issue, binding)?;
        let runtime = (self.resolve_runtime)()?;
        let Some(mut opened) =
            crate::private_sqlite::open_existing(&runtime, FILES, Synchronous::Extra)?
        else {
            return Ok(false);
        };
        let (connection, guard) = opened.parts();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        guard.verify_identity()?;
        let owned = validate_schema(&transaction)? && read_owner(&transaction, issue, binding)?;
        guard.verify_identity()?;
        transaction.commit()?;
        guard.verify_identity()?;
        Ok(owned)
    }
}

fn read_owner(
    connection: &Connection,
    issue: &IssueReference,
    binding: &IssueAutomationBindingId,
) -> Result<bool> {
    let owner: Option<(String,String,String)> = connection.query_row(
        "SELECT project_id,workspace_root,binding_id FROM issue_source_claims WHERE cloud_id=?1 AND issue_id=?2",
        params![issue.server_id, issue.issue_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?;
    Ok(owner.is_some_and(|(project, workspace, id)| {
        project == issue.repository_id
            && workspace == issue.project_scope.workspace_dir
            && id == binding.as_str()
    }))
}

fn validate(issue: &IssueReference, binding: &IssueAutomationBindingId) -> Result<()> {
    ensure!(
        issue.provider == IssueProvider::Jira,
        "only Jira uses profile source claims"
    );
    ensure!(
        !issue.server_id.is_empty()
            && issue.server_id.len() <= 64
            && issue
                .server_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
        "invalid Jira cloud identity"
    );
    for value in [&issue.repository_id, &issue.issue_id] {
        ensure!(
            !value.is_empty() && value.len() <= 64 && value.bytes().all(|c| c.is_ascii_digit()),
            "invalid Jira stable identity"
        );
    }
    let workspace = &issue.project_scope.workspace_dir;
    ensure!(
        !workspace.is_empty() && workspace.len() <= 16384 && Path::new(workspace).is_absolute(),
        "invalid source claim workspace"
    );
    let canonical =
        std::fs::canonicalize(workspace).context("source claim workspace unavailable")?;
    ensure!(
        canonical.to_str() == Some(workspace),
        "source claim requires the canonical primary checkout"
    );
    ensure!(
        binding.as_str().len() <= 512,
        "source claim binding is too long"
    );
    Ok(())
}

fn validate_schema(connection: &Connection) -> Result<bool> {
    let id: i64 = connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let mut statement = connection.prepare(
        "SELECT name,sql FROM sqlite_master WHERE name NOT GLOB 'sqlite_*' ORDER BY name",
    )?;
    let mut rows = statement.query([])?;
    let first = rows.next()?;
    if id == 0 && version == 0 && first.is_none() {
        return Ok(false);
    }
    ensure!(
        id == APPLICATION_ID && version == 1,
        "unsupported source claim database"
    );
    let row = first.context("source claim schema is missing")?;
    let name: String = row.get(0)?;
    let sql: String = row.get(1)?;
    let normalized = |sql: &str| {
        sql.trim()
            .trim_end_matches(';')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    ensure!(
        name == "issue_source_claims"
            && normalized(&sql) == normalized(SCHEMA)
            && rows.next()?.is_none(),
        "invalid source claim schema"
    );
    Ok(true)
}

#[cfg(test)]
mod tests;

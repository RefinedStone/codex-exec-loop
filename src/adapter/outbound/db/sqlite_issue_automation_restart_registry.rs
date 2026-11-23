//! Profile-scoped restart candidates; current authority stays in each repository ledger.
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, Transaction, TransactionBehavior, params, types::ValueRef};

use crate::application::port::outbound::issue_automation_restart_registry_port::{
    IssueAutomationRestartEntry, IssueAutomationRestartRegistryPort,
};
use crate::private_sqlite::{DatabaseFiles, PrivateStoreGuard, Synchronous};

type RuntimeResolver = dyn Fn() -> Result<PathBuf> + Send + Sync;

const DATABASE_NAME: &str = "restart-registry.db";
const FILES: DatabaseFiles = DatabaseFiles::new(
    DATABASE_NAME,
    "restart-registry.db-journal",
    "restart-registry.db-wal",
    "restart-registry.db-shm",
);
const APPLICATION_ID: i64 = 0x414b_4952;
const SCHEMA_VERSION: i64 = 1;
const MAX_ENTRIES: usize = 256;
const MAX_KEY_BYTES: usize = 256;
const MAX_WORKSPACE_BYTES: usize = 16 * 1024;
const SCHEMA: &str = r#"
CREATE TABLE restart_candidates (
    repository_key TEXT COLLATE BINARY NOT NULL PRIMARY KEY
        CHECK (typeof(repository_key) = 'text' AND length(CAST(repository_key AS BLOB)) BETWEEN 1 AND 256),
    workspace_root TEXT COLLATE BINARY NOT NULL
        CHECK (typeof(workspace_root) = 'text' AND length(CAST(workspace_root AS BLOB)) BETWEEN 1 AND 16384)
) WITHOUT ROWID
"#;

/// Composition supplies the authority profile's runtime lazily; construction does no I/O.
pub struct SqliteIssueAutomationRestartRegistryAdapter {
    resolve_runtime: Box<RuntimeResolver>,
}

impl SqliteIssueAutomationRestartRegistryAdapter {
    pub fn new(resolve_runtime: impl Fn() -> Result<PathBuf> + Send + Sync + 'static) -> Self {
        Self {
            resolve_runtime: Box::new(resolve_runtime),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_runtime(runtime: PathBuf) -> Self {
        Self::new(move || Ok(runtime.clone()))
    }
}

impl IssueAutomationRestartRegistryPort for SqliteIssueAutomationRestartRegistryAdapter {
    fn register(&self, entry: &IssueAutomationRestartEntry) -> Result<()> {
        validate_entry(entry)?;
        let runtime = (self.resolve_runtime)()?;
        let mut opened =
            match crate::private_sqlite::open_existing(&runtime, FILES, Synchronous::Extra)? {
                Some(opened) => opened,
                None => crate::private_sqlite::open_or_create_strict(
                    &runtime,
                    FILES,
                    Synchronous::Extra,
                )?,
            };
        let (connection, guard) = opened.parts();
        transaction(
            connection,
            guard,
            TransactionBehavior::Immediate,
            |transaction| {
                if !validate_schema(transaction)? {
                    transaction.execute_batch(SCHEMA)?;
                    transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
                    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                    validate_schema(transaction)?;
                }
                let current = read_entries(transaction)?;
                if current.len() >= MAX_ENTRIES
                    && !current
                        .iter()
                        .any(|saved| saved.repository_key == entry.repository_key)
                {
                    bail!("issue automation restart registry has too many repositories");
                }
                transaction.execute(
                    "INSERT INTO restart_candidates(repository_key,workspace_root) VALUES(?1,?2)
                 ON CONFLICT(repository_key) DO UPDATE SET workspace_root=excluded.workspace_root
                 WHERE restart_candidates.workspace_root<>excluded.workspace_root",
                    params![entry.repository_key, entry.workspace_root],
                )?;
                Ok(())
            },
        )
    }

    fn unregister(&self, repository_key: &str) -> Result<()> {
        validate_key(repository_key)?;
        let runtime = (self.resolve_runtime)()?;
        let Some(mut opened) =
            crate::private_sqlite::open_existing(&runtime, FILES, Synchronous::Extra)?
        else {
            return Ok(());
        };
        let (connection, guard) = opened.parts();
        // Committing BEGIN IMMEDIATE can materialize a header in an empty SQLite file,
        // even without a DELETE. Inspect it in a read transaction before taking a writer.
        if !transaction(
            connection,
            guard,
            TransactionBehavior::Deferred,
            |transaction| validate_schema(transaction),
        )? {
            return Ok(());
        }
        transaction(
            connection,
            guard,
            TransactionBehavior::Immediate,
            |transaction| {
                if validate_schema(transaction)? {
                    // A mutation must not silently hide malformed or over-limit retained candidates.
                    read_entries(transaction)?;
                    transaction.execute(
                        "DELETE FROM restart_candidates WHERE repository_key=?1",
                        params![repository_key],
                    )?;
                }
                Ok(())
            },
        )
    }

    fn list(&self) -> Result<Vec<IssueAutomationRestartEntry>> {
        let runtime = (self.resolve_runtime)()?;
        let Some(mut opened) =
            crate::private_sqlite::open_existing(&runtime, FILES, Synchronous::Extra)?
        else {
            return Ok(Vec::new());
        };
        let (connection, guard) = opened.parts();
        transaction(
            connection,
            guard,
            TransactionBehavior::Deferred,
            |transaction| {
                if validate_schema(transaction)? {
                    read_entries(transaction)
                } else {
                    // A process may have stopped after creating its private file but before schema commit.
                    // Reading that empty database must not initialize a schema or create an entry.
                    Ok(Vec::new())
                }
            },
        )
    }
}

fn transaction<T>(
    connection: &mut Connection,
    guard: &PrivateStoreGuard,
    behavior: TransactionBehavior,
    work: impl FnOnce(&Transaction<'_>) -> Result<T>,
) -> Result<T> {
    let transaction = connection.transaction_with_behavior(behavior)?;
    guard.verify_identity()?;
    let result = work(&transaction)?;
    guard.verify_identity()?;
    transaction.commit()?;
    guard.verify_identity()?;
    Ok(result)
}

/// Return false only for an empty, uninitialized database; never repair another schema.
fn validate_schema(connection: &Connection) -> Result<bool> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let mut statement = connection.prepare(
        "SELECT name,sql FROM sqlite_master WHERE name NOT GLOB 'sqlite_*' ORDER BY name",
    )?;
    let mut rows = statement.query([])?;
    let first = rows.next()?;
    if application_id == 0 && version == 0 && first.is_none() {
        return Ok(false);
    }
    ensure!(
        application_id == APPLICATION_ID && version == SCHEMA_VERSION,
        "issue automation restart registry identity or schema version is unsupported"
    );
    let row = first.context("issue automation restart registry schema is missing")?;
    let name: String = row.get(0)?;
    let sql: String = row.get(1)?;
    ensure!(
        name == "restart_candidates" && normalize_sql(&sql) == normalize_sql(SCHEMA),
        "issue automation restart registry schema is invalid"
    );
    ensure!(
        rows.next()?.is_none(),
        "issue automation restart registry contains an unexpected schema object"
    );
    Ok(true)
}

fn normalize_sql(sql: &str) -> String {
    sql.trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn read_entries(connection: &Connection) -> Result<Vec<IssueAutomationRestartEntry>> {
    let mut statement = connection.prepare(
        "SELECT repository_key,workspace_root FROM restart_candidates
         ORDER BY repository_key LIMIT 257",
    )?;
    let mut rows = statement.query([])?;
    let mut entries = Vec::new();
    while let Some(row) = rows.next()? {
        ensure!(
            entries.len() < MAX_ENTRIES,
            "issue automation restart registry has too many repositories"
        );
        let entry = IssueAutomationRestartEntry {
            repository_key: bounded_text(row.get_ref(0)?, MAX_KEY_BYTES)?,
            workspace_root: bounded_text(row.get_ref(1)?, MAX_WORKSPACE_BYTES)?,
        };
        validate_entry(&entry)?;
        entries.push(entry);
    }
    Ok(entries)
}

fn bounded_text(value: ValueRef<'_>, limit: usize) -> Result<String> {
    let ValueRef::Text(bytes) = value else {
        bail!("issue automation restart registry value is not text");
    };
    ensure!(
        !bytes.is_empty() && bytes.len() <= limit,
        "issue automation restart registry value is empty or too long"
    );
    Ok(std::str::from_utf8(bytes)
        .context("issue automation restart registry value is not Unicode")?
        .to_owned())
}

fn validate_key(key: &str) -> Result<()> {
    ensure!(
        !key.is_empty()
            && key.len() <= MAX_KEY_BYTES
            && key.trim() == key
            && !key.chars().any(char::is_control),
        "issue automation restart repository key is invalid"
    );
    Ok(())
}

fn validate_entry(entry: &IssueAutomationRestartEntry) -> Result<()> {
    validate_key(&entry.repository_key)?;
    let path = Path::new(&entry.workspace_root);
    ensure!(
        !entry.workspace_root.is_empty()
            && entry.workspace_root.len() <= MAX_WORKSPACE_BYTES
            && !entry.workspace_root.chars().any(char::is_control)
            && path.is_absolute()
            && !path
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir)),
        "issue automation restart checkout must be an absolute canonical scope"
    );
    Ok(())
}

#[cfg(test)]
mod tests;

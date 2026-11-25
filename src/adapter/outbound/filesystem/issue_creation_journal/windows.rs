//! Windows creation intent journal. The reservation DB is distinct from the
//! payload DB so Sent commits durably before a remote creation, while the
//! creation lease stays held and provenance can read a committed snapshot.
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, TransactionBehavior, params};

use super::records::{MAX_JOURNAL_BYTES, decode_records, encode_records, upsert};
use crate::application::port::outbound::issue_creation_journal_port::IssueCreationJournalTransaction;
use crate::domain::issue_creation::IssueCreationIntentRecord;
use crate::private_sqlite::{DatabaseFiles, PrivateConnection, Synchronous};

const JOURNAL_NAME: &str = "issue-create-intents.db";
const JOURNAL_FILES: DatabaseFiles = DatabaseFiles::new(
    JOURNAL_NAME,
    "issue-create-intents.db-journal",
    "issue-create-intents.db-wal",
    "issue-create-intents.db-shm",
);
const LEASE_FILES: DatabaseFiles = DatabaseFiles::new(
    "issue-creation-lease.db",
    "issue-creation-lease.db-journal",
    "issue-creation-lease.db-wal",
    "issue-creation-lease.db-shm",
);
const APPLICATION_ID: i64 = 0x414b4943;
const SCHEMA_VERSION: i64 = 1;
const CREATE_TABLE: &str = "CREATE TABLE issue_creation_journal (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    payload BLOB NOT NULL
)";

pub(super) struct Transaction {
    database: PrivateConnection,
    reservation: PrivateConnection,
    runtime: PathBuf,
    workspace: String,
}

impl Transaction {
    pub(super) fn open(runtime: PathBuf, workspace: String, timeout: Duration) -> Result<Self> {
        let started = Instant::now();
        reject_legacy(&runtime)?;
        let mut reservation = crate::private_sqlite::open_with_busy_timeout(
            &runtime,
            LEASE_FILES,
            Synchronous::Extra,
            timeout.saturating_sub(started.elapsed()),
        )?;
        let (connection, guard) = reservation.parts();
        connection.busy_timeout(timeout.saturating_sub(started.elapsed()))?;
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .context("another issue creation transaction is still running")?;
        let (version, application, objects) = schema(connection)?;
        if version != 0 || application != 0 || !objects.is_empty() {
            bail!("the issue creation reservation has an unsupported schema");
        }
        guard.verify_identity()?;
        reject_legacy(&runtime)?;

        let database = crate::private_sqlite::open(&runtime, JOURNAL_FILES, Synchronous::Extra)?;
        let mut transaction = Self {
            database,
            reservation,
            runtime,
            workspace,
        };
        transaction.initialize()?;
        Ok(transaction)
    }

    fn verify_identity(&self) -> Result<()> {
        self.reservation.verify_identity()?;
        self.database.verify_identity()?;
        reject_legacy(&self.runtime)
    }

    fn initialize(&mut self) -> Result<()> {
        self.verify_identity()?;
        let (connection, guard) = self.database.parts();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_schema(&tx, true)?;
        let records = read_records(&tx)?;
        verify_workspace(&records, &self.workspace)?;
        guard.verify_identity()?;
        tx.commit()?;
        self.verify_identity()
    }
}

impl IssueCreationJournalTransaction for Transaction {
    fn records(&mut self) -> Result<Vec<IssueCreationIntentRecord>> {
        self.verify_identity()?;
        let (connection, guard) = self.database.parts();
        let tx = connection.transaction()?;
        ensure_schema(&tx, false)?;
        let records = read_records(&tx)?;
        verify_workspace(&records, &self.workspace)?;
        guard.verify_identity()?;
        tx.commit()?;
        self.verify_identity()?;
        Ok(records)
    }

    fn save(&mut self, record: &IssueCreationIntentRecord) -> Result<()> {
        self.verify_identity()?;
        let (connection, guard) = self.database.parts();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_schema(&tx, false)?;
        let mut records = read_records(&tx)?;
        verify_workspace(&records, &self.workspace)?;
        upsert(&mut records, &self.workspace, record)?;
        let body = encode_records(&records)?;
        if tx.execute(
            "UPDATE issue_creation_journal SET payload = ?1 WHERE id = 1",
            params![body],
        )? != 1
        {
            bail!("the issue creation journal lost its payload row");
        }
        guard.verify_identity()?;
        self.reservation.verify_identity()?;
        reject_legacy(&self.runtime)?;
        // Sent must reach this durable commit before the service may POST.
        tx.commit()?;
        self.verify_identity()
    }
}

/// The collector reads a committed snapshot without acquiring the writer
/// reservation or creating absent journal files. Unknown schema/legacy files
/// fail closed rather than reporting an empty set of creation provenance.
pub(super) fn read_validated_records(runtime: &Path) -> Result<Vec<IssueCreationIntentRecord>> {
    reject_legacy(runtime)?;
    let Some(mut database) =
        crate::private_sqlite::open_existing(runtime, JOURNAL_FILES, Synchronous::Extra)?
    else {
        reject_legacy(runtime)?;
        return Ok(Vec::new());
    };
    let (connection, guard) = database.parts();
    let tx = connection.transaction()?;
    ensure_schema(&tx, false)?;
    let records = read_records(&tx)?;
    guard.verify_identity()?;
    reject_legacy(runtime)?;
    tx.commit()?;
    guard.verify_identity()?;
    reject_legacy(runtime)?;
    Ok(records)
}

fn reject_legacy(runtime: &Path) -> Result<()> {
    for name in ["issue-create-intents.jsonl", "issue-create-intents.lock"] {
        match fs::symlink_metadata(runtime.join(name)) {
            Ok(_) => bail!("a legacy issue creation file prevents Windows SQLite creation"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .context("could not establish absence of legacy issue creation files");
            }
        }
    }
    Ok(())
}

type SchemaObjects = Vec<(String, String, String)>;

fn schema(connection: &Connection) -> Result<(i64, i64, SchemaObjects)> {
    let version = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let application = connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    let objects = connection
        .prepare("SELECT type, name, COALESCE(sql, '') FROM sqlite_schema ORDER BY name")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok((version, application, objects))
}

fn ensure_schema(connection: &Connection, initialize: bool) -> Result<()> {
    let (version, application, objects) = schema(connection)?;
    if initialize && version == 0 && application == 0 && objects.is_empty() {
        connection.execute_batch(CREATE_TABLE)?;
        connection.execute(
            "INSERT INTO issue_creation_journal (id, payload) VALUES (1, X'')",
            [],
        )?;
        connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        connection.pragma_update(None, "application_id", APPLICATION_ID)?;
        return Ok(());
    }
    if version != SCHEMA_VERSION
        || application != APPLICATION_ID
        || objects
            != [(
                "table".to_string(),
                "issue_creation_journal".to_string(),
                CREATE_TABLE.to_string(),
            )]
    {
        bail!("the issue creation database has an unsupported schema");
    }
    Ok(())
}

fn read_records(connection: &Connection) -> Result<Vec<IssueCreationIntentRecord>> {
    let count: i64 =
        connection.query_row("SELECT COUNT(*) FROM issue_creation_journal", [], |row| {
            row.get(0)
        })?;
    if count != 1 {
        bail!("the issue creation journal has an invalid payload count");
    }
    let (kind, bytes): (String, i64) = connection.query_row(
        "SELECT typeof(payload), length(payload) FROM issue_creation_journal WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if kind != "blob" || bytes < 0 || bytes as u64 > MAX_JOURNAL_BYTES as u64 {
        bail!("the issue creation journal has an invalid payload or exceeds its byte limit");
    }
    let payload: Vec<u8> = connection.query_row(
        "SELECT payload FROM issue_creation_journal WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    decode_records(&payload)
}

fn verify_workspace(records: &[IssueCreationIntentRecord], workspace: &str) -> Result<()> {
    if records
        .iter()
        .any(|record| record.repository.project_scope.workspace_dir != workspace)
    {
        bail!("the issue creation journal belongs to a different workspace");
    }
    Ok(())
}

#[cfg(test)]
mod tests;

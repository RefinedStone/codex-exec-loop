//! Windows-only publication journal and repository-scoped SQLite reservation.
//! The reservation database is deliberately separate from the journal: Sent must commit
//! while the caller still owns the publication reservation, including across processes.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, TransactionBehavior, params};

use super::{JOURNAL_FILE_NAME, PUBLICATION_LOCK_NAME};
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublicationLease;
use crate::domain::issue_publish_intent::IssuePublishIntentRecord;
use crate::private_sqlite::{DatabaseFiles, PrivateConnection, Synchronous};

pub(super) const DATABASE_NAME: &str = "issue-publish-intents.db";
const JOURNAL_FILES: DatabaseFiles = DatabaseFiles::new(
    DATABASE_NAME,
    "issue-publish-intents.db-journal",
    "issue-publish-intents.db-wal",
    "issue-publish-intents.db-shm",
);
const LEASE_FILES: DatabaseFiles = DatabaseFiles::new(
    "issue-publication-lease.db",
    "issue-publication-lease.db-journal",
    "issue-publication-lease.db-wal",
    "issue-publication-lease.db-shm",
);
const SCHEMA_VERSION: i64 = 1;
const APPLICATION_ID: i64 = 0x414b4950;
const CREATE_TABLE: &str = "CREATE TABLE issue_publish_intents (
    position INTEGER PRIMARY KEY,
    intent_id TEXT NOT NULL UNIQUE,
    record TEXT NOT NULL
)";
const MAX_BYTES: usize = super::super::secure_fs::MAX_FILE_BYTES;

/// An open transaction owns the reservation until drop; there is no PID/TTL eviction.
pub(super) struct PublicationLease {
    database: PrivateConnection,
    runtime: PathBuf,
}

impl PublicationLease {
    pub(super) fn acquire(runtime: PathBuf, timeout: Duration) -> Result<Self> {
        let started = Instant::now();
        reject_legacy(&runtime)?;
        let mut database = crate::private_sqlite::open_with_busy_timeout(
            &runtime,
            LEASE_FILES,
            Synchronous::Extra,
            timeout.saturating_sub(started.elapsed()),
        )?;
        let (connection, guard) = database.parts();
        connection.busy_timeout(timeout.saturating_sub(started.elapsed()))?;
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .context("another issue publication transaction is still running")?;
        // The lease has no application records. Reject a database repurposed by another
        // schema instead of treating it as a newly empty coordination file.
        let objects: i64 =
            connection.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| row.get(0))?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let application: i64 =
            connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
        if objects != 0 || version != 0 || application != 0 {
            bail!("the issue publication lease database has an unsupported schema")
        }
        guard.verify_identity()?;
        reject_legacy(&runtime)?;
        let lease = Self { database, runtime };
        lease.verify_identity()?;
        Ok(lease)
    }
}

impl IssuePublicationLease for PublicationLease {
    fn verify_identity(&self) -> Result<()> {
        self.database.verify_identity()?;
        reject_legacy(&self.runtime)
    }
}

fn reject_legacy(runtime: &Path) -> Result<()> {
    for name in [JOURNAL_FILE_NAME, PUBLICATION_LOCK_NAME] {
        match fs::symlink_metadata(runtime.join(name)) {
            Ok(_) => {
                bail!("a legacy issue publication file prevents Windows SQLite publication: {name}")
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .context("could not establish absence of legacy issue publication files");
            }
        }
    }
    Ok(())
}

fn open_journal(runtime: &Path) -> Result<PrivateConnection> {
    reject_legacy(runtime)?;
    let opened = crate::private_sqlite::open(runtime, JOURNAL_FILES, Synchronous::Extra)?;
    reject_legacy(runtime)?;
    Ok(opened)
}

fn ensure_schema(connection: &Connection) -> Result<()> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let application: i64 = connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    let objects: Vec<(String, String, String)> = connection
        .prepare("SELECT type, name, COALESCE(sql, '') FROM sqlite_schema ORDER BY name")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if version == 0 && application == 0 && objects.is_empty() {
        connection.execute_batch(CREATE_TABLE)?;
        connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        connection.pragma_update(None, "application_id", APPLICATION_ID)?;
        return Ok(());
    }
    if version != SCHEMA_VERSION
        || application != APPLICATION_ID
        || objects
            != vec![
                (
                    "table".to_string(),
                    "issue_publish_intents".to_string(),
                    CREATE_TABLE.to_string(),
                ),
                (
                    "index".to_string(),
                    "sqlite_autoindex_issue_publish_intents_1".to_string(),
                    String::new(),
                ),
            ]
    {
        bail!("the issue publish intent database has an unsupported schema")
    }
    Ok(())
}

fn read_rows(connection: &Connection) -> Result<Vec<IssuePublishIntentRecord>> {
    let bytes: i64 = connection.query_row(
        "SELECT COALESCE(SUM(length(CAST(record AS BLOB)) + 1), 0) FROM issue_publish_intents",
        [],
        |row| row.get(0),
    )?;
    if bytes < 0 || bytes as u64 > MAX_BYTES as u64 {
        bail!("the issue publish intent journal exceeds its serialized byte limit")
    }
    let mut statement = connection.prepare(
        "SELECT position, intent_id, record FROM issue_publish_intents ORDER BY position",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut records = Vec::new();
    for row in rows {
        let (position, id, body) = row?;
        let record: IssuePublishIntentRecord = serde_json::from_str(&body)
            .context("the issue publish intent journal has a broken record")?;
        if position <= 0 || record.intent_id != id {
            bail!("the issue publish intent journal has an inconsistent record identity")
        }
        records.push(record);
    }
    serialized_size(&records)?;
    Ok(records)
}

fn serialized_size(records: &[IssuePublishIntentRecord]) -> Result<usize> {
    let mut bytes = 0usize;
    for record in records {
        bytes = bytes
            .checked_add(serde_json::to_vec(record)?.len() + 1)
            .context("the issue publish intent journal size overflowed")?;
        if bytes > MAX_BYTES {
            bail!("the issue publish intent journal exceeds its serialized byte limit")
        }
    }
    Ok(bytes)
}

pub(super) fn read_records(runtime: &Path) -> Result<Vec<IssuePublishIntentRecord>> {
    let mut database = open_journal(runtime)?;
    let (connection, guard) = database.parts();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_schema(&transaction)?;
    let records = read_rows(&transaction)?;
    guard.verify_identity()?;
    reject_legacy(runtime)?;
    transaction.commit()?;
    guard.verify_identity()?;
    Ok(records)
}

pub(super) fn save_record(runtime: &Path, record: &IssuePublishIntentRecord) -> Result<()> {
    let mut database = open_journal(runtime)?;
    let (connection, guard) = database.parts();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_schema(&transaction)?;
    let mut records = read_rows(&transaction)?;
    match records
        .iter_mut()
        .find(|old| old.intent_id == record.intent_id)
    {
        Some(old) => *old = record.clone(),
        None => records.push(record.clone()),
    }
    let removed = super::retention::trim(&mut records);
    serialized_size(&records)?;
    transaction.execute(
        "INSERT INTO issue_publish_intents (intent_id, record) VALUES (?1, ?2)
         ON CONFLICT (intent_id) DO UPDATE SET record = excluded.record",
        params![record.intent_id, serde_json::to_string(record)?],
    )?;
    for id in removed {
        transaction.execute(
            "DELETE FROM issue_publish_intents WHERE intent_id = ?1",
            [id],
        )?;
    }
    guard.verify_identity()?;
    reject_legacy(runtime)?;
    // No publication can start until this durable Sent transaction has returned success.
    transaction.commit()?;
    guard.verify_identity()?;
    Ok(())
}

#[cfg(test)]
mod tests;

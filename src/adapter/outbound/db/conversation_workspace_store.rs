//! 준비 기록과 별도 예약 DB를 사용한다. git 부작용 전의 Preparing commit을 잠금과 함께 유지한다.
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::application::port::outbound::conversation_workspace_store_port::{
    ConversationWorkspaceStoreLocation, ConversationWorkspaceStorePort,
    ConversationWorkspaceTransaction,
};
use crate::domain::conversation_workspace::{
    ConversationWorkspaceRecord, WORKSPACE_INVENTORY_LIMIT, request_key, validate_request_id,
};
use crate::private_sqlite::{
    DatabaseFiles, PinnedPrivateDirectory, PrivateConnection, Synchronous,
};

const RECORDS: DatabaseFiles = DatabaseFiles::new(
    "conversation-workspaces.db",
    "conversation-workspaces.db-journal",
    "conversation-workspaces.db-wal",
    "conversation-workspaces.db-shm",
);
const LEASE: DatabaseFiles = DatabaseFiles::new(
    "conversation-workspaces-lease.db",
    "conversation-workspaces-lease.db-journal",
    "conversation-workspaces-lease.db-wal",
    "conversation-workspaces-lease.db-shm",
);
const APPLICATION_ID: i64 = 0x414b4357;
const CREATE_TABLE: &str = "CREATE TABLE conversation_workspaces (request_id TEXT PRIMARY KEY NOT NULL, payload TEXT NOT NULL)";
type LocationResolver =
    dyn Fn(&str, bool) -> Result<Option<ConversationWorkspaceStoreLocation>> + Send + Sync;

pub(crate) struct ConversationWorkspaceStoreAdapter {
    resolve: Box<LocationResolver>,
}

impl ConversationWorkspaceStoreAdapter {
    /// create=false는 없는 저장소 runtime이나 기록을 만들지 않아야 한다.
    pub(crate) fn new(
        resolve: impl Fn(&str, bool) -> Result<Option<ConversationWorkspaceStoreLocation>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            resolve: Box::new(resolve),
        }
    }
}

struct WorkspaceTransaction {
    location: ConversationWorkspaceStoreLocation,
    workspaces: PinnedPrivateDirectory,
    records: PrivateConnection,
    reservation: PrivateConnection,
}

impl WorkspaceTransaction {
    fn verify(&mut self) -> Result<()> {
        self.workspaces.verify()?;
        self.records.parts().1.verify_identity()?;
        self.reservation.parts().1.verify_identity()
    }
}

impl ConversationWorkspaceStorePort for ConversationWorkspaceStoreAdapter {
    fn begin(&self, cwd: &str) -> Result<Box<dyn ConversationWorkspaceTransaction + '_>> {
        let location = (self.resolve)(cwd, true)?
            .context("conversation workspace repository is unavailable")?;
        let mut reservation = crate::private_sqlite::open_or_create_strict_with_busy_timeout(
            &location.runtime_dir,
            LEASE,
            Synchronous::Extra,
            Duration::from_secs(30),
        )?;
        let (connection, guard) = reservation.parts();
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .context("another workspace preparation is running")?;
        let objects: i64 =
            connection.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))?;
        let app: i64 = connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        ensure!(
            objects == 0 && app == 0 && version == 0,
            "unknown workspace reservation schema"
        );
        guard.verify_identity()?;
        let mut records = crate::private_sqlite::open_or_create_strict(
            &location.runtime_dir,
            RECORDS,
            Synchronous::Extra,
        )?;
        let (connection, guard) = records.parts();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_schema(&tx, true)?;
        guard.verify_identity()?;
        tx.commit()?;
        let runtime = PinnedPrivateDirectory::open(&location.runtime_dir, false)?;
        let workspaces = runtime.child("conversation-workspaces", true)?;
        let mut transaction = WorkspaceTransaction {
            location,
            workspaces,
            records,
            reservation,
        };
        transaction.verify()?;
        Ok(Box::new(transaction))
    }

    fn inventory(
        &self,
        cwd: &str,
        after: Option<&str>,
    ) -> Result<Vec<ConversationWorkspaceRecord>> {
        let Some(location) = (self.resolve)(cwd, false)? else {
            return Ok(Vec::new());
        };
        let Some(mut database) = crate::private_sqlite::open_existing(
            &location.runtime_dir,
            RECORDS,
            Synchronous::Extra,
        )?
        else {
            return Ok(Vec::new());
        };
        let (connection, guard) = database.parts();
        let tx = connection.transaction()?;
        ensure_schema(&tx, false)?;
        let ids = tx.prepare("SELECT request_id FROM conversation_workspaces WHERE request_id > ?1 ORDER BY request_id LIMIT ?2")?
            .query_map(params![after.unwrap_or(""), (WORKSPACE_INVENTORY_LIMIT + 1) as i64], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            records.push(
                read_record(&tx, &location, &id)?
                    .context("workspace record missing from inventory")?,
            );
        }
        guard.verify_identity()?;
        tx.commit()?;
        Ok(records)
    }

    fn get(&self, cwd: &str, request_id: &str) -> Result<Option<ConversationWorkspaceRecord>> {
        let Some(location) = (self.resolve)(cwd, false)? else {
            return Ok(None);
        };
        let Some(mut database) = crate::private_sqlite::open_existing(
            &location.runtime_dir,
            RECORDS,
            Synchronous::Extra,
        )?
        else {
            return Ok(None);
        };
        let (connection, guard) = database.parts();
        let tx = connection.transaction()?;
        ensure_schema(&tx, false)?;
        let record = read_record(&tx, &location, request_id)?;
        guard.verify_identity()?;
        tx.commit()?;
        Ok(record)
    }
}

impl ConversationWorkspaceTransaction for WorkspaceTransaction {
    fn repository_dir(&self) -> &str {
        &self.location.repository_dir
    }
    fn workspaces_dir(&self) -> &Path {
        self.workspaces.path()
    }

    fn find(&mut self, request_id: &str) -> Result<Option<ConversationWorkspaceRecord>> {
        self.verify()?;
        let (connection, guard) = self.records.parts();
        ensure_schema(connection, false)?;
        let record = read_record(connection, &self.location, request_id)?;
        guard.verify_identity()?;
        self.verify()?;
        Ok(record)
    }

    fn save(&mut self, record: &ConversationWorkspaceRecord) -> Result<()> {
        self.verify()?;
        validate_record(&self.location, record)?;
        let payload = serde_json::to_string(record)?;
        ensure!(payload.len() <= 65_536, "workspace record is too large");
        let (connection, guard) = self.records.parts();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_schema(&tx, false)?;
        if let Some(existing) = read_record(&tx, &self.location, &record.request_id)? {
            ensure!(
                existing.repository_dir == record.repository_dir
                    && existing.workspace_dir == record.workspace_dir
                    && existing.branch == record.branch
                    && existing.base_commit == record.base_commit,
                "workspace ownership identity cannot be replaced"
            );
        }
        tx.execute(
            "INSERT INTO conversation_workspaces(request_id,payload) VALUES(?1,?2) ON CONFLICT(request_id) DO UPDATE SET payload=excluded.payload",
            params![record.request_id, payload],
        )?;
        guard.verify_identity()?;
        self.reservation.parts().1.verify_identity()?;
        self.workspaces.verify()?;
        tx.commit()?;
        self.verify()
    }
}

fn ensure_schema(connection: &Connection, create: bool) -> Result<()> {
    let app: i64 = connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let objects: Vec<String> = connection
        .prepare("SELECT sql FROM sqlite_master WHERE name NOT GLOB 'sqlite_*' ORDER BY name")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if app == 0 && version == 0 && objects.is_empty() && create {
        connection.execute_batch(CREATE_TABLE)?;
        connection.pragma_update(None, "application_id", APPLICATION_ID)?;
        connection.pragma_update(None, "user_version", 1_i64)?;
    } else {
        ensure!(
            app == APPLICATION_ID && version == 1 && objects == [CREATE_TABLE],
            "unsupported conversation workspace schema"
        );
    }
    Ok(())
}

fn read_record(
    connection: &Connection,
    location: &ConversationWorkspaceStoreLocation,
    request_id: &str,
) -> Result<Option<ConversationWorkspaceRecord>> {
    let payload: Option<String> = connection
        .query_row(
            "SELECT payload FROM conversation_workspaces WHERE request_id=?1",
            [request_id],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| {
            ensure!(payload.len() <= 65_536, "workspace record is too large");
            let record: ConversationWorkspaceRecord =
                serde_json::from_str(&payload).context("invalid workspace ownership record")?;
            ensure!(
                record.request_id == request_id,
                "workspace request identity differs"
            );
            validate_record(location, &record)?;
            Ok(record)
        })
        .transpose()
}

fn validate_record(
    location: &ConversationWorkspaceStoreLocation,
    record: &ConversationWorkspaceRecord,
) -> Result<()> {
    validate_request_id(&record.request_id)?;
    use crate::domain::conversation_workspace::ConversationWorkspaceState;
    ensure!(
        match record.state {
            ConversationWorkspaceState::Preparing | ConversationWorkspaceState::Prepared =>
                record.reason.is_none(),
            ConversationWorkspaceState::Preserved =>
                record.reason.as_deref().is_some_and(|reason| matches!(
                    reason,
                    "worktree_creation_incomplete"
                        | "workspace_identity_unconfirmed"
                        | "created_workspace_identity_unconfirmed"
                )),
        },
        "workspace record state and reason differ"
    );
    let key = request_key(&record.request_id);
    let expected = location
        .runtime_dir
        .join("conversation-workspaces")
        .join(&key);
    ensure!(
        record.repository_dir == location.repository_dir
            && Path::new(&record.workspace_dir) == expected
            && record.branch == format!("akra-conversation/{key}")
            && crate::domain::git_graph::is_git_commit_id(&record.base_commit),
        "workspace ownership record is outside its repository allocation"
    );
    Ok(())
}

#[cfg(test)]
mod tests;

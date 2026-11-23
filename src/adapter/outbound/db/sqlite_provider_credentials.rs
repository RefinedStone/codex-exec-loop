//! Profile-bound, secret-free credential metadata. Key-store I/O never runs in these transactions.
use std::path::PathBuf;

use rusqlite::{Connection, Transaction, TransactionBehavior, params, types::ValueRef};

use crate::application::port::outbound::provider_credentials_port::ProviderCredentialLedgerPort;
use crate::domain::provider_credentials::{
    CredentialLedger, CredentialResult, Provider, ProviderCredentialError, valid_profile_id,
};
use crate::private_sqlite::{DatabaseFiles, PrivateStoreGuard, Synchronous};

type RuntimeResolver = dyn Fn() -> anyhow::Result<PathBuf> + Send + Sync;
const DATABASE_NAME: &str = "provider-credentials.db";
const FILES: DatabaseFiles = DatabaseFiles::new(
    DATABASE_NAME,
    "provider-credentials.db-journal",
    "provider-credentials.db-wal",
    "provider-credentials.db-shm",
);
const APPLICATION_ID: i64 = 0x414b_5043;
const SCHEMA_VERSION: i64 = 1;
const MAX_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;
const SCHEMA: &str = r#"
CREATE TABLE provider_credentials (
    provider TEXT COLLATE BINARY NOT NULL PRIMARY KEY
        CHECK (provider IN ('jira', 'linear')),
    row_version INTEGER NOT NULL CHECK (typeof(row_version) = 'integer' AND row_version > 0),
    document TEXT NOT NULL
        CHECK (typeof(document) = 'text' AND length(CAST(document AS BLOB)) BETWEEN 1 AND 2097152)
) WITHOUT ROWID
"#;

pub struct SqliteProviderCredentialLedgerAdapter {
    profile_id: String,
    resolve_runtime: Box<RuntimeResolver>,
}

impl SqliteProviderCredentialLedgerAdapter {
    /// The caller supplies the validated authority profile and its runtime; construction does no I/O.
    pub fn new(
        profile_id: String,
        resolve_runtime: impl Fn() -> anyhow::Result<PathBuf> + Send + Sync + 'static,
    ) -> CredentialResult<Self> {
        if !valid_profile_id(&profile_id) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        Ok(Self {
            profile_id,
            resolve_runtime: Box::new(resolve_runtime),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_runtime(profile_id: String, runtime: PathBuf) -> Self {
        Self::new(profile_id, move || Ok(runtime.clone())).expect("valid test profile")
    }
}

impl ProviderCredentialLedgerPort for SqliteProviderCredentialLedgerAdapter {
    fn load(&self, provider: Provider) -> CredentialResult<CredentialLedger> {
        let runtime = (self.resolve_runtime)().map_err(|_| ProviderCredentialError::Unavailable)?;
        let Some(mut opened) =
            crate::private_sqlite::open_existing(&runtime, FILES, Synchronous::Extra)
                .map_err(|_| ProviderCredentialError::Unavailable)?
        else {
            return Ok(CredentialLedger::empty(self.profile_id.clone(), provider));
        };
        let (connection, guard) = opened.parts();
        transaction(
            connection,
            guard,
            TransactionBehavior::Deferred,
            &|| true,
            |transaction| {
                if !validate_schema(transaction)? {
                    return Ok(CredentialLedger::empty(self.profile_id.clone(), provider));
                }
                let documents = read_documents(transaction, &self.profile_id)?;
                Ok(documents
                    .into_iter()
                    .find(|document| document.provider == provider)
                    .unwrap_or_else(|| CredentialLedger::empty(self.profile_id.clone(), provider)))
            },
        )
    }

    fn compare_exchange_guarded(
        &self,
        expected: &CredentialLedger,
        next: &CredentialLedger,
        publish_allowed: &dyn Fn() -> bool,
    ) -> CredentialResult<bool> {
        expected.validate()?;
        next.validate()?;
        if expected.profile_id != self.profile_id
            || next.profile_id != self.profile_id
            || expected.provider != next.provider
            || next.row_version
                != expected
                    .row_version
                    .checked_add(1)
                    .ok_or(ProviderCredentialError::CapacityExceeded)?
            || next.revision < expected.revision
            || (expected.tombstone && !next.tombstone)
        {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let serialized =
            serde_json::to_string(next).map_err(|_| ProviderCredentialError::Corrupt)?;
        if serialized.len() > MAX_DOCUMENT_BYTES {
            return Err(ProviderCredentialError::CapacityExceeded);
        }
        let runtime = (self.resolve_runtime)().map_err(|_| ProviderCredentialError::Unavailable)?;
        let existing = crate::private_sqlite::open_existing(&runtime, FILES, Synchronous::Extra)
            .map_err(|_| ProviderCredentialError::Unavailable)?;
        let mut opened = match existing {
            Some(opened) => opened,
            None if expected.row_version == 0 => {
                crate::private_sqlite::open_or_create_strict(&runtime, FILES, Synchronous::Extra)
                    .map_err(|_| ProviderCredentialError::Unavailable)?
            }
            None => return Err(ProviderCredentialError::Corrupt),
        };
        let (connection, guard) = opened.parts();
        transaction(
            connection,
            guard,
            TransactionBehavior::Immediate,
            publish_allowed,
            |transaction| {
                if !publish_allowed() {
                    return Err(ProviderCredentialError::Cancelled);
                }
                if !validate_schema(transaction)? {
                    if expected.row_version != 0 {
                        return Err(ProviderCredentialError::Corrupt);
                    }
                    transaction.execute_batch(SCHEMA).map_err(database_error)?;
                    transaction
                        .pragma_update(None, "application_id", APPLICATION_ID)
                        .map_err(database_error)?;
                    transaction
                        .pragma_update(None, "user_version", SCHEMA_VERSION)
                        .map_err(database_error)?;
                    validate_schema(transaction)?;
                }
                let documents = read_documents(transaction, &self.profile_id)?;
                let current = documents
                    .iter()
                    .find(|document| document.provider == expected.provider)
                    .cloned()
                    .unwrap_or_else(|| {
                        CredentialLedger::empty(self.profile_id.clone(), expected.provider)
                    });
                if &current != expected {
                    return Ok(false);
                }
                transaction.execute(
                "INSERT INTO provider_credentials(provider,row_version,document) VALUES(?1,?2,?3)
                 ON CONFLICT(provider) DO UPDATE SET row_version=excluded.row_version,document=excluded.document",
                params![next.provider.as_str(), next.row_version, serialized],
            ).map_err(database_error)?;
                // Triggers are excluded by schema validation; also verify the committed candidate.
                let saved = read_documents(transaction, &self.profile_id)?;
                if saved
                    .iter()
                    .find(|document| document.provider == next.provider)
                    != Some(next)
                {
                    return Err(ProviderCredentialError::Corrupt);
                }
                Ok(true)
            },
        )
    }
}

fn transaction<T>(
    connection: &mut Connection,
    guard: &PrivateStoreGuard,
    behavior: TransactionBehavior,
    publish_allowed: &dyn Fn() -> bool,
    work: impl FnOnce(&Transaction<'_>) -> CredentialResult<T>,
) -> CredentialResult<T> {
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(database_error)?;
    guard
        .verify_identity()
        .map_err(|_| ProviderCredentialError::Unavailable)?;
    let result = work(&transaction)?;
    guard
        .verify_identity()
        .map_err(|_| ProviderCredentialError::Unavailable)?;
    if !publish_allowed() {
        return Err(ProviderCredentialError::Cancelled);
    }
    transaction.commit().map_err(database_error)?;
    guard
        .verify_identity()
        .map_err(|_| ProviderCredentialError::Unavailable)?;
    Ok(result)
}

fn validate_schema(connection: &Connection) -> CredentialResult<bool> {
    let application_id: i64 = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(database_error)?;
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(database_error)?;
    let mut statement = connection
        .prepare("SELECT name,sql FROM sqlite_master WHERE name NOT GLOB 'sqlite_*' ORDER BY name")
        .map_err(database_error)?;
    let mut rows = statement.query([]).map_err(database_error)?;
    let first = rows.next().map_err(database_error)?;
    if application_id == 0 && version == 0 && first.is_none() {
        return Ok(false);
    }
    if application_id != APPLICATION_ID || version != SCHEMA_VERSION {
        return Err(ProviderCredentialError::Corrupt);
    }
    let row = first.ok_or(ProviderCredentialError::Corrupt)?;
    let name: String = row.get(0).map_err(database_error)?;
    let sql: String = row.get(1).map_err(database_error)?;
    if name != "provider_credentials"
        || normalize_sql(&sql) != normalize_sql(SCHEMA)
        || rows.next().map_err(database_error)?.is_some()
    {
        return Err(ProviderCredentialError::Corrupt);
    }
    Ok(true)
}

fn normalize_sql(sql: &str) -> String {
    sql.trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn read_documents(
    connection: &Connection,
    profile_id: &str,
) -> CredentialResult<Vec<CredentialLedger>> {
    let mut statement = connection.prepare(
        "SELECT provider,row_version,document FROM provider_credentials ORDER BY provider LIMIT 3",
    ).map_err(database_error)?;
    let mut rows = statement.query([]).map_err(database_error)?;
    let mut documents = Vec::new();
    while let Some(row) = rows.next().map_err(database_error)? {
        if documents.len() >= 2 {
            return Err(ProviderCredentialError::Corrupt);
        }
        let provider: String = row.get(0).map_err(database_error)?;
        let row_version: u64 = row.get(1).map_err(database_error)?;
        let ValueRef::Text(bytes) = row.get_ref(2).map_err(database_error)? else {
            return Err(ProviderCredentialError::Corrupt);
        };
        if bytes.is_empty() || bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(ProviderCredentialError::Corrupt);
        }
        let document: CredentialLedger =
            serde_json::from_slice(bytes).map_err(|_| ProviderCredentialError::Corrupt)?;
        document.validate()?;
        if document.profile_id != profile_id
            || document.provider.as_str() != provider
            || document.row_version != row_version
            || row_version == 0
        {
            return Err(ProviderCredentialError::Corrupt);
        }
        documents.push(document);
    }
    Ok(documents)
}

fn database_error(_: rusqlite::Error) -> ProviderCredentialError {
    // SQLite messages may contain supplied metadata. Do not propagate them through RPC or logs.
    ProviderCredentialError::Corrupt
}

#[cfg(test)]
mod tests;

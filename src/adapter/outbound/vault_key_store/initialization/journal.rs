use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::application::port::outbound::vault_key_store_port::VaultKeyStoreError;
use crate::private_sqlite::{DatabaseFiles, PrivateConnection, PrivateStoreGuard, Synchronous};

use super::super::unavailable;

const APPLICATION_ID: i64 = 0x414B_564B;
const SCHEMA_VERSION: i64 = 1;
const FILES: DatabaseFiles = DatabaseFiles::new(
    "key-initialization.db",
    "key-initialization.db-journal",
    "key-initialization.db-wal",
    "key-initialization.db-shm",
);
const TABLE_SQL: &str = "CREATE TABLE vault_key_initialization (
    scope TEXT PRIMARY KEY NOT NULL CHECK(length(scope) = 64),
    generation INTEGER NOT NULL CHECK(generation > 0),
    phase TEXT NOT NULL CHECK(phase IN ('pending', 'settled')),
    fingerprint BLOB NOT NULL CHECK(length(fingerprint) = 32)
) STRICT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Pending,
    Settled,
}

/// 키 재료는 없다. 마지막 generation을 지우지 않아 오래된 None 조회의 ABA를 막는다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct State {
    pub(super) generation: i64,
    pub(super) phase: Phase,
    pub(super) fingerprint: [u8; 32],
}

pub(super) struct Journal {
    opened: PrivateConnection,
    scope: String,
}

impl Journal {
    pub(super) fn open(
        runtime: &std::path::Path,
        scope: String,
    ) -> Result<Self, VaultKeyStoreError> {
        let mut opened = crate::private_sqlite::open_or_create(runtime, FILES, Synchronous::Extra)
            .map_err(|error| failed("open", error))?;
        let (connection, guard) = opened.parts();
        ensure_schema(connection, guard)?;
        Ok(Self { opened, scope })
    }

    pub(super) fn verify_identity(&mut self) -> Result<(), VaultKeyStoreError> {
        let (_, guard) = self.opened.parts();
        verify(guard, "identity")
    }

    pub(super) fn snapshot(&mut self) -> Result<Option<State>, VaultKeyStoreError> {
        let (connection, guard) = self.opened.parts();
        verify(guard, "snapshot")?;
        let state = read_state(connection, &self.scope)?;
        verify(guard, "snapshot")?;
        Ok(state)
    }

    pub(super) fn begin(&mut self) -> Result<JournalTransaction<'_>, VaultKeyStoreError> {
        let (connection, guard) = self.opened.parts();
        verify(guard, "begin")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| failed("begin", error))?;
        Ok(JournalTransaction {
            transaction,
            guard,
            scope: &self.scope,
        })
    }
}

pub(super) struct JournalTransaction<'a> {
    transaction: Transaction<'a>,
    guard: &'a PrivateStoreGuard,
    scope: &'a str,
}

impl JournalTransaction<'_> {
    pub(super) fn snapshot(&self) -> Result<Option<State>, VaultKeyStoreError> {
        verify(self.guard, "snapshot")?;
        read_state(&self.transaction, self.scope)
    }

    pub(super) fn pending(
        &self,
        previous: Option<&State>,
        fingerprint: [u8; 32],
    ) -> Result<State, VaultKeyStoreError> {
        if previous.is_some_and(|state| state.phase == Phase::Pending) {
            return Err(unavailable());
        }
        let generation = previous
            .map_or(Some(1), |state| state.generation.checked_add(1))
            .ok_or_else(unavailable)?;
        verify(self.guard, "pending write")?;
        self.transaction
            .execute(
                "INSERT INTO vault_key_initialization(scope, generation, phase, fingerprint)
             VALUES (?1, ?2, 'pending', ?3)
             ON CONFLICT(scope) DO UPDATE SET generation = excluded.generation,
                 phase = excluded.phase, fingerprint = excluded.fingerprint",
                params![self.scope, generation, fingerprint.as_slice()],
            )
            .map_err(|error| failed("pending write", error))?;
        Ok(State {
            generation,
            phase: Phase::Pending,
            fingerprint,
        })
    }

    pub(super) fn settle(&self, expected: &State) -> Result<(), VaultKeyStoreError> {
        verify(self.guard, "settle write")?;
        let changed = self
            .transaction
            .execute(
                "UPDATE vault_key_initialization SET phase = 'settled'
             WHERE scope = ?1 AND generation = ?2 AND phase = 'pending' AND fingerprint = ?3",
                params![
                    self.scope,
                    expected.generation,
                    expected.fingerprint.as_slice()
                ],
            )
            .map_err(|error| failed("settle write", error))?;
        if changed != 1 {
            return Err(unavailable());
        }
        Ok(())
    }

    pub(super) fn commit(self) -> Result<(), VaultKeyStoreError> {
        verify(self.guard, "commit")?;
        self.transaction
            .commit()
            .map_err(|error| failed("commit", error))?;
        verify(self.guard, "commit")
    }
}

fn verify(guard: &PrivateStoreGuard, stage: &'static str) -> Result<(), VaultKeyStoreError> {
    guard
        .verify_identity()
        .map_err(|error| failed(stage, error))
}

/// 제품 오류는 고정 문구만 쓴다. 시험 프로세스에서만 실패 경계와 하위 오류를 stderr에 남긴다.
/// journal에는 키 재료가 없고 SQL 인자는 오류 문구에 들어가지 않는다.
fn failed(stage: &'static str, error: impl std::fmt::Debug) -> VaultKeyStoreError {
    #[cfg(test)]
    eprintln!("vault key initialization journal {stage} failed: {error:?}");
    #[cfg(not(test))]
    let _ = (stage, error);
    unavailable()
}

fn ensure_schema(
    connection: &mut Connection,
    guard: &PrivateStoreGuard,
) -> Result<(), VaultKeyStoreError> {
    verify(guard, "schema")?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| failed("schema", error))?;
    let application_id: i64 = transaction
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(|error| failed("schema", error))?;
    let version: i64 = transaction
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| failed("schema", error))?;
    if application_id == 0 && version == 0 {
        let objects: i64 = transaction
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| failed("schema", error))?;
        if objects != 0 {
            return Err(failed("schema", "unexpected objects in a new journal"));
        }
        transaction
            .execute_batch(TABLE_SQL)
            .map_err(|error| failed("schema", error))?;
        transaction
            .pragma_update(None, "application_id", APPLICATION_ID)
            .map_err(|error| failed("schema", error))?;
        transaction
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|error| failed("schema", error))?;
    } else if application_id != APPLICATION_ID || version != SCHEMA_VERSION {
        return Err(failed("schema", "unexpected application id or version"));
    }
    validate_schema(&transaction)?;
    verify(guard, "schema")?;
    transaction
        .commit()
        .map_err(|error| failed("schema", error))?;
    verify(guard, "schema")
}

fn validate_schema(connection: &Connection) -> Result<(), VaultKeyStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT type, name, sql FROM sqlite_schema
         WHERE name NOT GLOB 'sqlite_*' ORDER BY name",
        )
        .map_err(|error| failed("schema validation", error))?;
    let objects = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| failed("schema validation", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| failed("schema validation", error))?;
    if objects.len() != 1
        || objects[0].0 != "table"
        || objects[0].1 != "vault_key_initialization"
        || objects[0].2 != TABLE_SQL
    {
        return Err(failed("schema validation", "unexpected schema objects"));
    }
    Ok(())
}

fn read_state(connection: &Connection, scope: &str) -> Result<Option<State>, VaultKeyStoreError> {
    let raw = connection
        .query_row(
            "SELECT generation, phase, fingerprint FROM vault_key_initialization WHERE scope = ?1",
            [scope],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| failed("state read", error))?;
    let Some((generation, phase, fingerprint)) = raw else {
        return Ok(None);
    };
    if generation <= 0 {
        return Err(unavailable());
    }
    let phase = match phase.as_str() {
        "pending" => Phase::Pending,
        "settled" => Phase::Settled,
        _ => return Err(unavailable()),
    };
    let fingerprint = fingerprint.try_into().map_err(|_| unavailable())?;
    Ok(Some(State {
        generation,
        phase,
        fingerprint,
    }))
}

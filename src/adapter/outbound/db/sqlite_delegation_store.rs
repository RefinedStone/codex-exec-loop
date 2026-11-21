//! The repository's private, independent text delegation ledger.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::application::port::outbound::delegation_store_port::{DelegationStorePort, Job};
use crate::domain::delegation::{Delegation, Delivery, Phase, State};

const FILE_NAME: &str = "delegations.sqlite3";

#[derive(Clone)]
pub(crate) struct DelegationStore(PathBuf);

impl DelegationStore {
    pub(crate) fn new(runtime: PathBuf) -> Self {
        Self(runtime.join(FILE_NAME))
    }

    pub(crate) fn exists(&self) -> bool {
        self.0.exists()
    }

    fn open(&self) -> Result<Connection> {
        let db = Connection::open_with_flags(
            &self.0,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        db.busy_timeout(Duration::from_secs(2))?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS delegation (
                id TEXT PRIMARY KEY NOT NULL,
                request_key TEXT UNIQUE NOT NULL,
                instruction TEXT NOT NULL,
                view TEXT NOT NULL,
                active INTEGER NOT NULL,
                stop_pending INTEGER NOT NULL DEFAULT 0,
                result TEXT
            );",
        )?;
        Ok(db)
    }
}

impl DelegationStorePort for DelegationStore {
    fn submit(&self, key: &str, instruction: &str, caller: &str) -> Result<Delegation> {
        if key.trim().is_empty() || key.len() > 128 {
            bail!("idempotency_key must contain 1 to 128 bytes");
        }
        if instruction.trim().is_empty() || instruction.len() > 65_536 {
            bail!("instruction must contain 1 to 65536 bytes");
        }
        let mut db = self.open()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = tx
            .query_row(
                "SELECT instruction, view FROM delegation WHERE request_key = ?1",
                [key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((original, view)) = existing {
            if original != instruction {
                bail!("idempotency_key already belongs to another instruction");
            }
            return Ok(serde_json::from_str(&view)?);
        }
        let id: String = tx.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        let now = chrono::Utc::now().timestamp_millis();
        let view = Delegation {
            id,
            caller: caller.to_owned(),
            state: State::Queued,
            delivery: Delivery::NotSent,
            phase: Phase::Unknown,
            reason: None,
            conversation_url: None,
            outcome_unknown: false,
            created_at: now,
            updated_at: now,
        };
        tx.execute(
            "INSERT INTO delegation(id, request_key, instruction, view, active)
             VALUES (?1, ?2, ?3, ?4, 1)",
            params![view.id, key, instruction, serde_json::to_string(&view)?],
        )?;
        tx.commit()?;
        Ok(view)
    }

    fn find(&self, id: &str) -> Result<Option<Delegation>> {
        let view: Option<String> = self
            .open()?
            .query_row("SELECT view FROM delegation WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        view.map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    fn result(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .open()?
            .query_row("SELECT result FROM delegation WHERE id = ?1", [id], |row| {
                row.get::<_, Option<String>>(0)
            })
            .optional()?
            .flatten())
    }

    fn list(&self) -> Result<Vec<Delegation>> {
        let db = self.open()?;
        let mut query = db.prepare("SELECT view FROM delegation ORDER BY rowid DESC LIMIT 100")?;
        query
            .query_map([], |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect()
    }

    fn active(&self) -> Result<Vec<Job>> {
        let db = self.open()?;
        let mut query = db.prepare(
            "SELECT view, instruction, stop_pending FROM delegation
             WHERE active = 1 OR stop_pending = 1 ORDER BY stop_pending DESC, rowid",
        )?;
        query
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?))
            })?
            .map(|row| {
                let (view, instruction, stop_pending) = row?;
                Ok(Job {
                    view: serde_json::from_str(&view)?,
                    instruction,
                    stop_pending,
                })
            })
            .collect()
    }

    /// Persist a state transition before its external effect. A stale observation cannot
    /// overwrite cancellation, even when a cancellation and an RPC response race.
    fn update(
        &self,
        old: &Delegation,
        next: &mut Delegation,
        result: Option<&str>,
        stop_pending: bool,
    ) -> Result<bool> {
        if old == next && result.is_none() && !stop_pending {
            return Ok(true);
        }
        next.updated_at = chrono::Utc::now()
            .timestamp_millis()
            .max(old.updated_at.saturating_add(1));
        let rows = self.open()?.execute(
            "UPDATE delegation SET view = ?2, active = ?3, result = ?4, stop_pending = ?5
             WHERE id = ?1 AND view = ?6",
            params![
                old.id,
                serde_json::to_string(next)?,
                !next.state.terminal(),
                result,
                stop_pending,
                serde_json::to_string(old)?,
            ],
        )?;
        Ok(rows == 1)
    }

    fn cancel(&self, id: &str) -> Result<Option<Delegation>> {
        loop {
            let Some(old) = self.find(id)? else {
                return Ok(None);
            };
            if old.state.terminal() {
                return Ok(Some(old));
            }
            let mut next = old.clone();
            next.state = State::Cancelled;
            next.reason = None;
            if self.update(&old, &mut next, None, old.state != State::Queued)? {
                return Ok(Some(next));
            }
        }
    }

    fn stopped(&self, id: &str) -> Result<()> {
        self.open()?
            .execute("UPDATE delegation SET stop_pending = 0 WHERE id = ?1", [id])?;
        Ok(())
    }
}

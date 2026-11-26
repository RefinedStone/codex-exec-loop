//! The repository's private, independent text delegation ledger.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail, ensure};
use rand::{RngCore, rngs::OsRng};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use crate::application::port::outbound::delegation_store_port::{
    DelegationStorePort, DelegationUsageProjection, Job, SubmissionContext,
};
use crate::domain::delegation::policy::{DEFAULT_DEADLINE_MS, Route};
use crate::domain::delegation::{
    ChatSession, Delegation, Delivery, EndReason, Evidence, EvidenceSource, MyTurn, Phase, State,
    can_fall_back,
};

const DAY_MS: i64 = 86_400_000;

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
            );
            CREATE TABLE IF NOT EXISTS delegation_route (
                id TEXT PRIMARY KEY NOT NULL,
                route TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS delegation_account (
                id TEXT PRIMARY KEY NOT NULL,
                digest BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS delegation_circuit (
                backend TEXT PRIMARY KEY NOT NULL,
                until_ms INTEGER,
                failures INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS delegation_callback (
                id TEXT PRIMARY KEY NOT NULL,
                digest BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS delegation_retry (
                request_key TEXT PRIMARY KEY NOT NULL,
                original_id TEXT NOT NULL,
                new_id TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS delegation_event (
                sequence INTEGER PRIMARY KEY,
                id TEXT NOT NULL,
                event TEXT NOT NULL,
                at INTEGER NOT NULL,
                detail TEXT NOT NULL
            );",
        )?;
        Ok(db)
    }

    fn callback(
        &self,
        id: &str,
        token: &str,
        body: &str,
        is_result: bool,
    ) -> Result<Option<Delegation>> {
        ensure!(
            !body.trim().is_empty() && body.len() <= if is_result { 1_048_576 } else { 65_536 },
            "invalid callback body length"
        );
        let mut db = self.open()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<(String, Vec<u8>)> = tx
            .query_row(
                "SELECT d.view, c.digest FROM delegation d JOIN delegation_callback c ON c.id = d.id
                 WHERE d.id = ?1 AND d.active = 1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((view, expected)) = current else {
            return Ok(None);
        };
        let actual = Sha256::digest(token.as_bytes());
        if expected.len() != actual.len()
            || actual
                .iter()
                .zip(expected)
                .fold(0, |delta, (a, b)| delta | (a ^ b))
                != 0
        {
            return Ok(None);
        }
        let now = now_ms();
        let mut view: Delegation = serde_json::from_str(&view)?;
        if now >= view.effective_deadline() || view.state == State::Queued {
            return Ok(None);
        }
        let phase = if is_result {
            Phase::Complete
        } else {
            Phase::Working
        };
        view.apply_observation(Delivery::Observed, phase);
        view.last_progress_at = Some(now);
        view.updated_at = now.max(view.updated_at.saturating_add(1));
        if let Some(attempt) = view.attempts.last_mut() {
            attempt.record(Evidence {
                source: EvidenceSource::Callback,
                at: now,
                delivery: Delivery::Observed,
                phase,
            });
        }
        if is_result {
            view.state = State::Succeeded;
            view.reason = None;
            tx.execute("DELETE FROM delegation_callback WHERE id = ?1", [id])?;
        }
        tx.execute(
            "UPDATE delegation SET view = ?2, active = ?3, result = ?4 WHERE id = ?1",
            params![
                id,
                serde_json::to_string(&view)?,
                !is_result,
                is_result.then_some(body)
            ],
        )?;
        record(
            &tx,
            id,
            if is_result {
                "delegation_succeeded"
            } else {
                "delegation_checkpointed"
            },
            now,
            fingerprint(body),
        )?;
        tx.commit()?;
        Ok(Some(view))
    }
}

impl DelegationStorePort for DelegationStore {
    fn submit_task_session(
        &self,
        key: &str,
        instruction: &str,
        caller: &str,
        route: Result<Route>,
        session: ChatSession,
        context: SubmissionContext<'_>,
    ) -> Result<Delegation> {
        let SubmissionContext {
            account_digest,
            task_id,
        } = context;
        if let Some(task_id) = task_id {
            crate::domain::planning::evidence::validate_evidence_identifier(task_id)
                .map_err(anyhow::Error::msg)?;
        }
        if key.trim().is_empty() || key.len() > 128 {
            bail!("idempotency_key must contain 1 to 128 bytes");
        }
        if instruction.trim().is_empty() || instruction.len() > 262_144 {
            bail!("invalid delegation instruction length");
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
            let existing: Delegation = serde_json::from_str(&view)?;
            if original != instruction
                || existing.session != session
                || existing.task_id.as_deref() != task_id
            {
                bail!("idempotency_key already belongs to another delegation request");
            }
            return Ok(existing);
        }
        let mut route = route?;
        ensure!(session.valid(), "invalid ChatGPT conversation URL");
        if session.selected_url().is_some() {
            ensure!(account_digest.is_some(), "chat_account_unknown");
            route
                .backends
                .retain(|backend| backend.kind == crate::domain::delegation::policy::CHAT);
            ensure!(!route.backends.is_empty(), "chat_route_unavailable");
        }
        ensure!(
            route.requires_defensive_isolation || instruction.len() <= 65_536,
            "instruction must contain 1 to 65536 bytes"
        );
        let view = insert(
            &tx,
            key,
            instruction,
            caller,
            &route,
            session,
            SubmissionContext {
                account_digest,
                task_id,
            },
        )?;
        tx.commit()?;
        Ok(view)
    }

    fn task_usage(&self, task_id: &str) -> Result<DelegationUsageProjection> {
        if !self.exists() {
            return Ok(DelegationUsageProjection {
                delegations: Vec::new(),
                unlinked: 0,
                truncated: false,
            });
        }
        let db = Connection::open_with_flags(
            &self.0,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        db.busy_timeout(Duration::from_secs(2))?;
        let unlinked: u64 = db.query_row(
            "SELECT count(*) FROM delegation WHERE json_extract(view,'$.task_id') IS NULL",
            [],
            |row| row.get(0),
        )?;
        let mut statement = db.prepare(
            "SELECT view FROM delegation
            WHERE json_extract(view,'$.task_id')=?1 ORDER BY rowid LIMIT 4097",
        )?;
        let mut delegations = statement
            .query_map([task_id], |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str::<Delegation>(&row?)?))
            .collect::<Result<Vec<_>>>()?;
        let truncated = delegations.len() > 4096;
        delegations.truncate(4096);
        Ok(DelegationUsageProjection {
            delegations,
            unlinked,
            truncated,
        })
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
        let mut query =
            db.prepare("SELECT view, stop_pending FROM delegation ORDER BY rowid DESC LIMIT 100")?;
        query
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?)))?
            .map(|row| {
                let (view, stop_pending) = row?;
                project(&db, &view, stop_pending)
            })
            .collect()
    }

    fn detail(&self, id: &str) -> Result<Option<Delegation>> {
        let db = self.open()?;
        let row: Option<(String, bool)> = db
            .query_row(
                "SELECT view, stop_pending FROM delegation WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(view, stop_pending)| project(&db, &view, stop_pending))
            .transpose()
    }

    fn my_turn(&self) -> Result<Vec<MyTurn>> {
        let db = self.open()?;
        let mut query = db.prepare(
            "SELECT id, coalesce(json_extract(view, '$.reason'), 'outcome_unknown')
             FROM delegation WHERE active = 1 AND json_extract(view, '$.state') = 'awaiting_human'
             ORDER BY rowid DESC LIMIT 100",
        )?;
        Ok(query
            .query_map([], |row| {
                Ok(MyTurn {
                    id: row.get(0)?,
                    reason: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    fn active(&self) -> Result<Vec<Job>> {
        let db = self.open()?;
        let mut query = db.prepare(
            "SELECT d.view, d.instruction, d.stop_pending, r.route, a.digest
             FROM delegation d LEFT JOIN delegation_route r ON d.id = r.id
             LEFT JOIN delegation_account a ON d.id = a.id
             WHERE d.active = 1 OR d.stop_pending = 1 ORDER BY d.stop_pending DESC, d.rowid",
        )?;
        query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get(4)?,
                ))
            })?
            .map(|row| {
                let (view, instruction, stop_pending, route, account_digest) = row?;
                Ok(Job {
                    view: serde_json::from_str(&view)?,
                    instruction,
                    stop_pending,
                    account_digest,
                    route: route
                        .as_deref()
                        .map(serde_json::from_str)
                        .transpose()?
                        .unwrap_or_default(),
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
        let mut db = self.open()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(
            next.task_id == old.task_id,
            "delegation task identity cannot change"
        );
        let stored_view: Option<String> = tx
            .query_row(
                "SELECT view FROM delegation WHERE id = ?1",
                [&old.id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(stored_view) = stored_view else {
            return Ok(false);
        };
        // Decode legacy rows with current defaults, but use their original bytes
        // in the write predicate. Re-encoding `old` can add fields absent on disk.
        if serde_json::from_str::<Delegation>(&stored_view)? != *old {
            return Ok(false);
        }
        if old == next && result.is_none() && !stop_pending {
            return Ok(true);
        }
        next.updated_at = now_ms().max(old.updated_at.saturating_add(1));
        if old.state == State::Queued
            && next.state == State::Dispatching
            && let Some(attempt) = next.attempts.last()
        {
            if now_ms() >= old.effective_deadline() {
                return Ok(false);
            }
            let route = load_route(&tx, &old.id)?;
            let limit = route
                .backends
                .iter()
                .find(|backend| backend.name == attempt.backend)
                .map_or(0, |backend| backend.max_concurrent);
            let count: u32 = tx.query_row(
                "SELECT count(*) FROM delegation
                 WHERE (active = 1 OR stop_pending = 1) AND json_extract(view, '$.state') != 'queued'
                 AND (json_extract(view, '$.attempts[#-1].backend') = ?1
                   OR (?2 = 'chatgpt_chat' AND coalesce(json_extract(view, '$.attempts[#-1].backend_kind'), 'chatgpt_chat') = ?2))",
                params![attempt.backend, attempt.backend_kind], |row| row.get(0),
            )?;
            if count >= limit
                || circuit_until(&tx, &attempt.backend)?.is_some_and(|until| now_ms() < until)
            {
                return Ok(false);
            }
        }
        let rows = tx.execute(
            "UPDATE delegation SET view = ?2, active = ?3, result = ?4, stop_pending = ?5
             WHERE id = ?1 AND view = ?6",
            params![
                old.id,
                serde_json::to_string(next)?,
                !next.state.terminal(),
                result,
                stop_pending,
                stored_view,
            ],
        )?;
        if rows == 1 {
            if next.state != old.state {
                record(
                    &tx,
                    &old.id,
                    match next.state {
                        State::Queued => "delegation_queued",
                        State::Dispatching => "delegation_dispatching",
                        State::Running => "delegation_running",
                        State::Interrupted => "delegation_interrupted",
                        State::AwaitingHuman => "delegation_awaiting_human",
                        State::Succeeded => "delegation_succeeded",
                        State::Failed => "delegation_failed",
                        State::Cancelled => "delegation_cancelled",
                        State::Expired => "delegation_expired",
                    },
                    next.updated_at,
                    result.map_or(serde_json::Value::Null, fingerprint),
                )?;
            }
            for fallback in next.fallbacks.iter().skip(old.fallbacks.len()) {
                record(
                    &tx,
                    &old.id,
                    "delegation_fell_back",
                    fallback.at,
                    serde_json::to_value(fallback)?,
                )?;
            }
            if next.state.terminal() || next.state == State::Queued {
                tx.execute("DELETE FROM delegation_callback WHERE id = ?1", [&old.id])?;
            }
            if let Some(attempt) = next.attempts.last()
                && !old
                    .attempts
                    .last()
                    .is_some_and(|last| last.id == attempt.id && last.reason == attempt.reason)
            {
                match attempt.reason {
                    Some(EndReason::Quota) => {
                        let until = attempt.retry_at.unwrap_or_else(|| now_ms() + 60_000);
                        tx.execute(
                            "INSERT INTO delegation_circuit VALUES (?1, ?2, 0)
                             ON CONFLICT(backend) DO UPDATE SET until_ms = max(coalesce(until_ms, 0), excluded.until_ms), failures = 0",
                            params![attempt.backend, until],
                        )?;
                    }
                    Some(EndReason::Transient) => {
                        tx.execute(
                            "INSERT INTO delegation_circuit VALUES (?1, NULL, 1)
                             ON CONFLICT(backend) DO UPDATE SET failures = failures + 1,
                             until_ms = CASE WHEN failures >= 2 THEN max(coalesce(until_ms, 0), ?2) ELSE until_ms END",
                            params![attempt.backend, now_ms() + 300_000],
                        )?;
                    }
                    _ => {}
                }
            }
            if next.state == State::Succeeded
                && let Some(attempt) = next.attempts.last()
            {
                tx.execute(
                    "UPDATE delegation_circuit SET failures = 0 WHERE backend = ?1",
                    [&attempt.backend],
                )?;
            }
        }
        tx.commit()?;
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

    fn take_stop(&self, id: &str) -> Result<bool> {
        Ok(self.open()?.execute(
            "UPDATE delegation SET stop_pending = 0 WHERE id = ?1 AND stop_pending = 1",
            [id],
        )? == 1)
    }

    fn circuit_until(&self, backend: &str) -> Result<Option<i64>> {
        circuit_until(&self.open()?, backend)
    }

    fn issue_callback_capability(&self, id: &str) -> Result<Option<String>> {
        let mut random = [0_u8; 32];
        OsRng
            .try_fill_bytes(&mut random)
            .map_err(|_| anyhow::anyhow!("callback entropy unavailable"))?;
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let digest = Sha256::digest(token.as_bytes());
        let inserted = self.open()?.execute(
            "INSERT OR IGNORE INTO delegation_callback SELECT id, ?2 FROM delegation
             WHERE id = ?1 AND active = 1 AND json_extract(view, '$.state') = 'dispatching'
             AND COALESCE(json_extract(view, '$.deadline'), json_extract(view, '$.created_at') + ?4) > ?3",
            params![id, digest.as_slice(), now_ms(), crate::domain::delegation::policy::DEFAULT_DEADLINE_MS],
        )?;
        Ok((inserted == 1).then_some(token))
    }

    fn checkpoint(&self, id: &str, token: &str, message: &str) -> Result<Option<Delegation>> {
        self.callback(id, token, message, false)
    }

    fn report(&self, id: &str, token: &str, result: &str) -> Result<Option<Delegation>> {
        self.callback(id, token, result, true)
    }

    fn retry(&self, id: &str, key: &str) -> Result<Option<Delegation>> {
        ensure!(
            !key.trim().is_empty() && key.len() <= 128,
            "invalid retry idempotency key"
        );
        let mut db = self.open()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, String)> = tx.query_row(
            "SELECT r.original_id, d.view FROM delegation_retry r JOIN delegation d ON r.new_id = d.id
             WHERE r.request_key = ?1", [key], |row| Ok((row.get(0)?, row.get(1)?))
        ).optional()?;
        if let Some((original, view)) = existing {
            ensure!(
                original == id,
                "retry key already belongs to another delegation"
            );
            return Ok(Some(serde_json::from_str(&view)?));
        }
        let original: Option<(String, String, bool)> = tx
            .query_row(
                "SELECT view, instruction, stop_pending FROM delegation WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((view, instruction, stop_pending)) = original else {
            return Ok(None);
        };
        let view: Delegation = serde_json::from_str(&view)?;
        let route = load_route(&tx, id)?;
        ensure!(
            can_retry(&view, &route, stop_pending),
            "delegation cannot be retried safely"
        );
        let account_digest: Option<Vec<u8>> = tx
            .query_row(
                "SELECT digest FROM delegation_account WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        let internal_key: String =
            tx.query_row("SELECT 'retry:' || lower(hex(randomblob(32)))", [], |row| {
                row.get(0)
            })?;
        let next = insert(
            &tx,
            &internal_key,
            &instruction,
            &view.caller,
            &route,
            view.session,
            SubmissionContext {
                account_digest: account_digest.as_deref(),
                task_id: view.task_id.as_deref(),
            },
        )?;
        tx.execute(
            "INSERT INTO delegation_retry VALUES (?1, ?2, ?3)",
            params![key, id, next.id],
        )?;
        tx.commit()?;
        Ok(Some(next))
    }
}

fn can_retry(view: &Delegation, route: &Route, stop_pending: bool) -> bool {
    let effect = if view.session.selected_url().is_some() {
        crate::domain::delegation::Effect::SideEffecting
    } else {
        route.effect
    };
    !route.requires_defensive_isolation
        && matches!(
            view.state,
            State::Failed | State::Expired | State::Cancelled
        )
        && !stop_pending
        && !view.outcome_unknown
        && view
            .attempts
            .iter()
            .all(|a| can_fall_back(effect, a.delivery, true, false))
        && can_fall_back(effect, view.delivery, true, false)
}

fn project(db: &Connection, value: &str, stop_pending: bool) -> Result<Delegation> {
    let mut view: Delegation = serde_json::from_str(value)?;
    view.can_cancel = !view.state.terminal();
    view.current_backend = view.attempts.last().map(|attempt| attempt.backend.clone());
    if view.state.terminal() {
        view.can_retry = can_retry(&view, &load_route(db, &view.id)?, stop_pending);
    }
    Ok(view)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn fingerprint(text: &str) -> serde_json::Value {
    serde_json::json!({ "sha256": format!("{:x}", Sha256::digest(text.as_bytes())), "bytes": text.len() })
}

fn record(
    db: &Connection,
    id: &str,
    event: &str,
    at: i64,
    detail: serde_json::Value,
) -> Result<()> {
    db.execute(
        "INSERT INTO delegation_event (id, event, at, detail) VALUES (?1, ?2, ?3, ?4)",
        params![id, event, at, serde_json::to_string(&detail)?],
    )?;
    Ok(())
}

fn load_route(db: &Connection, id: &str) -> Result<Route> {
    let value: Option<String> = db
        .query_row(
            "SELECT route FROM delegation_route WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default())
}

fn circuit_until(db: &Connection, backend: &str) -> Result<Option<i64>> {
    Ok(db
        .query_row(
            "SELECT until_ms FROM delegation_circuit WHERE backend = ?1",
            [backend],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten())
}

fn insert(
    db: &Connection,
    key: &str,
    instruction: &str,
    caller: &str,
    route: &Route,
    session: ChatSession,
    context: SubmissionContext<'_>,
) -> Result<Delegation> {
    let SubmissionContext {
        account_digest,
        task_id,
    } = context;
    let now = now_ms();
    let (running, started): (u32, u32) = db.query_row(
        "SELECT coalesce(sum(active = 1 OR stop_pending = 1), 0),
                coalesce(sum(json_extract(view, '$.created_at') >= ?1), 0) FROM delegation",
        [now.div_euclid(DAY_MS) * DAY_MS],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    ensure!(
        running < route.max_running && started < route.max_started_per_day,
        "budget_exhausted"
    );
    let id: String = db.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
    let view = Delegation {
        id,
        caller: caller.to_owned(),
        task_id: task_id.map(str::to_owned),
        state: State::Queued,
        delivery: Delivery::NotSent,
        phase: Phase::Unknown,
        reason: None,
        conversation_url: None,
        outcome_unknown: false,
        created_at: now,
        updated_at: now,
        deadline: Some(now.saturating_add(if route.deadline_ms > 0 {
            route.deadline_ms
        } else {
            DEFAULT_DEADLINE_MS
        })),
        policy_revision: Some(route.revision.clone()),
        attempts: Vec::new(),
        fallbacks: Vec::new(),
        last_progress_at: None,
        session,
        current_backend: None,
        can_cancel: false,
        can_retry: false,
    };
    db.execute(
        "INSERT INTO delegation(id, request_key, instruction, view, active) VALUES (?1, ?2, ?3, ?4, 1)",
        params![view.id, key, instruction, serde_json::to_string(&view)?],
    )?;
    db.execute(
        "INSERT INTO delegation_route VALUES (?1, ?2)",
        params![view.id, serde_json::to_string(route)?],
    )?;
    if let Some(digest) = account_digest {
        db.execute(
            "INSERT INTO delegation_account VALUES (?1, ?2)",
            params![view.id, digest],
        )?;
    }
    record(
        db,
        &view.id,
        "delegation_queued",
        now,
        fingerprint(instruction),
    )?;
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ledger() -> (TestDirectory, DelegationStore) {
        let directory = crate::test_utils::platform_safe_temp_dir()
            .join(format!("delegation-ledger-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&directory).unwrap();
        let store = DelegationStore::new(directory.clone());
        (TestDirectory(directory), store)
    }

    #[test]
    fn task_usage_delegation_scope_is_idempotent_and_retry_keeps_it() {
        let (_directory, store) = ledger();
        let submit = |task_id| {
            store.submit_task_session(
                "task-key",
                "explain",
                "Desktop",
                Ok(Route::default()),
                ChatSession::New,
                SubmissionContext {
                    account_digest: None,
                    task_id,
                },
            )
        };
        let original = submit(Some("task-a")).unwrap();
        assert_eq!(submit(Some("task-a")).unwrap().id, original.id);
        assert!(submit(Some("task-b")).is_err());
        assert!(submit(None).is_err());
        assert_eq!(store.task_usage("task-a").unwrap().delegations.len(), 1);
        let mut rebound = original.clone();
        rebound.task_id = Some("task-b".into());
        assert!(store.update(&original, &mut rebound, None, false).is_err());
        store.cancel(&original.id).unwrap();
        let retried = store.retry(&original.id, "retry-task").unwrap().unwrap();
        assert_eq!(retried.task_id.as_deref(), Some("task-a"));
        store.submit("old", "unlinked", "Desktop").unwrap();
        let projection = store.task_usage("task-a").unwrap();
        assert_eq!(projection.delegations.len(), 2);
        assert_eq!(projection.unlinked, 1);
        assert!(store.task_usage("task-b").unwrap().delegations.is_empty());
    }

    fn insert_prerelease_active_row(store: &DelegationStore) -> Delegation {
        // The prerelease writer serialized only these fields, in this order.
        let view = concat!(
            r#"{"id":"legacy","caller":"test","state":"running","delivery":"accepted","#,
            r#""phase":"working","reason":null,"conversation_url":null,"outcome_unknown":false,"#,
            r#""created_at":1000,"updated_at":1001}"#,
        );
        store
            .open()
            .unwrap()
            .execute(
                "INSERT INTO delegation(id, request_key, instruction, view, active)
                 VALUES ('legacy', 'legacy-key', 'text', ?1, 1)",
                [view],
            )
            .unwrap();
        let old = store.active().unwrap().pop().unwrap().view;
        assert_ne!(serde_json::to_string(&old).unwrap(), view);
        old
    }

    #[test]
    fn delegation_prerelease_callback_uses_creation_time_when_deadline_is_missing() {
        let (_directory, store) = ledger();
        insert_prerelease_active_row(&store);
        let db = store.open().unwrap();
        db.execute(
            "UPDATE delegation SET view=json_set(view,'$.state','dispatching') WHERE id='legacy'",
            [],
        )
        .unwrap();
        assert!(store.issue_callback_capability("legacy").unwrap().is_none());
        db.execute(
            "UPDATE delegation SET view=json_set(view,'$.created_at',?1) WHERE id='legacy'",
            [now_ms()],
        )
        .unwrap();
        assert!(store.issue_callback_capability("legacy").unwrap().is_some());
    }

    #[test]
    fn delegation_prerelease_row_can_expire() {
        use crate::domain::delegation::{NextAction, next_action};

        let (directory, store) = ledger();
        let old = insert_prerelease_active_row(&store);
        assert_eq!(next_action(&old, false, now_ms()), NextAction::Expire);
        let mut expired = old.clone();
        expired.state = State::Expired;
        expired.reason = Some("deadline_exceeded".into());
        assert!(store.update(&old, &mut expired, None, true).unwrap());
        let restarted = DelegationStore::new(directory.0.clone());
        assert_eq!(restarted.find(&old.id).unwrap(), Some(expired));
        assert!(restarted.take_stop(&old.id).unwrap());
        assert!(restarted.active().unwrap().is_empty());
    }

    #[test]
    fn delegation_prerelease_row_can_cancel_and_reject_stale_completion() {
        let (directory, store) = ledger();
        let old = insert_prerelease_active_row(&store);
        let other_writer = DelegationStore::new(directory.0.clone());
        let cancelled = other_writer.cancel(&old.id).unwrap().unwrap();
        assert_eq!(cancelled.state, State::Cancelled);
        assert!(cancelled.updated_at > old.updated_at);

        let mut completed = old.clone();
        completed.state = State::Succeeded;
        assert!(
            !store
                .update(&old, &mut completed, Some("stale result"), false)
                .unwrap()
        );
        assert_eq!(store.find(&old.id).unwrap(), Some(cancelled));
        assert!(store.result(&old.id).unwrap().is_none());
        assert!(store.take_stop(&old.id).unwrap());
        assert!(store.active().unwrap().is_empty());
    }

    #[test]
    fn delegation_capability_is_job_scoped_durable_and_revoked_after_report() {
        let (directory, store) = ledger();
        let a = store.submit("a", "private instruction", "test").unwrap();
        let b = store.submit("b", "private instruction", "test").unwrap();
        let mut claimed = a.clone();
        claimed.state = State::Dispatching;
        claimed.delivery = Delivery::Uncertain;
        claimed.outcome_unknown = true;
        assert!(store.update(&a, &mut claimed, None, false).unwrap());
        let token = store.issue_callback_capability(&a.id).unwrap().unwrap();
        let restarted = DelegationStore::new(directory.0.clone());
        assert!(
            restarted
                .issue_callback_capability(&a.id)
                .unwrap()
                .is_none()
        );
        assert!(restarted.report(&b.id, &token, "wrong").unwrap().is_none());
        assert!(restarted.report(&a.id, "wrong", "wrong").unwrap().is_none());
        let working = restarted
            .checkpoint(&a.id, &token, "working")
            .unwrap()
            .unwrap();
        assert_eq!(working.state, State::Running);
        assert!(!working.outcome_unknown);
        let done = restarted.report(&a.id, &token, "result").unwrap().unwrap();
        assert_eq!(done.state, State::Succeeded);
        assert_eq!(restarted.result(&a.id).unwrap().as_deref(), Some("result"));
        assert!(restarted.report(&a.id, &token, "late").unwrap().is_none());
        assert!(
            !std::fs::read(&restarted.0)
                .unwrap()
                .windows(token.len())
                .any(|bytes| bytes == token.as_bytes())
        );
    }

    #[test]
    fn delegation_budget_and_retry_are_transactional_and_never_replay_unknown() {
        let (_directory, store) = ledger();
        let route = Route {
            max_running: 1,
            max_started_per_day: 2,
            ..Route::default()
        };
        let a = store
            .submit_with_route("a", "work", "tester", Ok(route.clone()))
            .unwrap();
        assert_eq!(
            store
                .submit_with_route(
                    "a",
                    "work",
                    "tester",
                    Err(anyhow::anyhow!("policy_blocked"))
                )
                .unwrap()
                .id,
            a.id
        );
        assert!(
            store
                .submit_with_route("b", "next", "tester", Ok(route.clone()))
                .is_err()
        );
        store.cancel(&a.id).unwrap();
        let retry = store.retry(&a.id, "retry-key").unwrap().unwrap();
        assert_ne!(retry.id, a.id);
        assert_eq!(
            store.retry(&a.id, "retry-key").unwrap().unwrap().id,
            retry.id
        );
        let mut uncertain = retry.clone();
        uncertain.state = State::Expired;
        uncertain.delivery = Delivery::Uncertain;
        uncertain.outcome_unknown = true;
        assert!(store.update(&retry, &mut uncertain, None, false).unwrap());
        assert!(store.retry(&retry.id, "unsafe").is_err());
        assert!(
            store
                .submit_with_route("c", "third", "tester", Ok(route))
                .is_err()
        );
    }
}

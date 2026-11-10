use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, TransactionBehavior, params};

use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::domain::kernel_event::KernelEvent;

use super::{SqlitePlanningAuthorityAdapter, open_authority_connection};

/*
 * 사건 기록 표는 `ensure_schema` 가 아니라 여기서 만든다 (plan 19 Q3-1). schema version 을 올리면 이전 binary(병렬
 * slot 의 akra 등)가 이 DB 를 열지 못한다. 표가 이미 있으면 아무것도 쓰지 않는다.
 */
fn ensure_kernel_event_table(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS kernel_events (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                thread_id TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                recorded_at TEXT NOT NULL
            );",
        )
        .context("failed to ensure the kernel event table")
}

impl KernelEventLogPort for SqlitePlanningAuthorityAdapter {
    fn append_kernel_events(&self, workspace_dir: &str, events: &[KernelEvent]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_kernel_event_table(&connection)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open the kernel event append transaction")?;
        let recorded_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
        for event in events {
            let payload =
                serde_json::to_string(event).context("failed to serialize a kernel event")?;
            transaction
                .execute(
                    "INSERT INTO kernel_events (kind, thread_id, payload_json, recorded_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![event.kind(), event.thread_id(), payload, recorded_at],
                )
                .context("failed to append a kernel event")?;
        }
        transaction
            .commit()
            .context("failed to commit kernel events")
    }

    fn load_kernel_events(&self, workspace_dir: &str, kinds: &[&str]) -> Result<Vec<KernelEvent>> {
        if kinds.is_empty() {
            return Ok(Vec::new());
        }
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        ensure_kernel_event_table(&connection)?;
        let placeholders = vec!["?"; kinds.len()].join(", ");
        let mut statement = connection
            .prepare(&format!(
                "SELECT payload_json FROM kernel_events WHERE kind IN ({placeholders})
                 ORDER BY sequence"
            ))
            .context("failed to prepare the kernel event read")?;
        let payloads = statement
            .query_map(rusqlite::params_from_iter(kinds), |row| {
                row.get::<_, String>(0)
            })
            .context("failed to read kernel events")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read a kernel event row")?;
        // 더 새 binary 가 쓴 사건은 이 binary 가 모른다. 건너뛰어도 아는 사건의 짝은 맞는다.
        Ok(payloads
            .iter()
            .filter_map(|payload| serde_json::from_str(payload).ok())
            .collect())
    }
}

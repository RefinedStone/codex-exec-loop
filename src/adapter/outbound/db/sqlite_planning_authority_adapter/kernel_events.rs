use anyhow::{Context, Result};
use rusqlite::{Connection, TransactionBehavior, params};

use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::domain::kernel_event::{
    KernelEvent, KernelEventPage, MAX_KERNEL_EVENT_PAGE, REPOSITORY_KINDS, RETAINED_KERNEL_EVENTS,
    RecordedKernelEvent, THREAD_PROMPT_QUEUE_KINDS,
};
use crate::domain::kernel_spec::KernelRecordKind;

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
    fn append_kernel_events(
        &self,
        workspace_dir: &str,
        records: &[RecordedKernelEvent],
    ) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_kernel_event_table(&connection)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open the kernel event append transaction")?;
        for record in records {
            let event = &record.event;
            let payload =
                serde_json::to_string(event).context("failed to serialize a kernel event")?;
            transaction
                .execute(
                    "INSERT INTO kernel_events (sequence, kind, thread_id, payload_json, recorded_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        record.sequence,
                        event.kind(),
                        event.thread_id(),
                        payload,
                        record.recorded_at
                    ],
                )
                .context("failed to append a kernel event")?;
        }
        // 남김 종류는 복구에 필요하므로 지우지 않는다. 다른 사건은 최근 것만 남긴다 (plan 19 Q3-3).
        // 마지막 git 상태도 남긴다. 다음 커널이 그 상태와 견준다 (plan 20 §4.1). 그 앞의 git 사건은 지운다.
        // 마지막 자동 후속 설정도 남긴다. 잠깐 붙는 클라이언트가 떠난 뒤의 다음 커널이 그 설정을 되돌린다 (plan 18 K4-8).
        transaction
            .execute(
                "DELETE FROM kernel_events
                 WHERE kind NOT IN (?1, ?2)
                   AND sequence <= (SELECT MAX(sequence) FROM kernel_events) - ?3
                   AND sequence <> COALESCE(
                       (SELECT MAX(sequence) FROM kernel_events WHERE kind = ?4), 0)
                   AND sequence <> COALESCE(
                       (SELECT MAX(sequence) FROM kernel_events WHERE kind = ?5), 0)",
                params![
                    THREAD_PROMPT_QUEUE_KINDS[0],
                    THREAD_PROMPT_QUEUE_KINDS[1],
                    RETAINED_KERNEL_EVENTS,
                    REPOSITORY_KINDS[0],
                    KernelRecordKind::AutoFollowConfigured.name()
                ],
            )
            .context("failed to trim old kernel events")?;
        transaction
            .commit()
            .context("failed to commit kernel events")
    }

    fn last_kernel_event_sequence(&self, workspace_dir: &str) -> Result<i64> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        ensure_kernel_event_table(&connection)?;
        connection
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM kernel_events",
                [],
                |row| row.get(0),
            )
            .context("failed to read the last kernel event sequence")
    }

    fn load_kernel_events_after(
        &self,
        workspace_dir: &str,
        after: i64,
        limit: usize,
    ) -> Result<KernelEventPage> {
        let limit = limit.clamp(1, MAX_KERNEL_EVENT_PAGE);
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        ensure_kernel_event_table(&connection)?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, recorded_at, payload_json FROM kernel_events
                 WHERE sequence > ?1 ORDER BY sequence LIMIT ?2",
            )
            .context("failed to prepare the kernel event page read")?;
        let rows = statement
            .query_map(params![after, limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .context("failed to read the kernel event page")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read a kernel event row")?;
        let next_after = rows.last().map_or(after, |(sequence, _, _)| *sequence);
        // 이 binary 가 모르는 사건은 건너뛴다. cursor 는 그 사건 뒤로 간다.
        let records = rows
            .into_iter()
            .filter_map(|(sequence, recorded_at, payload)| {
                let event = serde_json::from_str(&payload).ok()?;
                Some(RecordedKernelEvent {
                    sequence,
                    recorded_at,
                    event,
                })
            })
            .collect();
        Ok(KernelEventPage {
            records,
            next_after,
        })
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

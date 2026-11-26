//! Latest counters share the kernel-log transaction and survive history retention.
use super::super::SqlitePlanningAuthorityAdapter;
use crate::domain::kernel_event::{KernelEvent, RecordedKernelEvent};
use crate::domain::task_usage::{TaskUsageLink, UsageProjection, UsageTurn};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};

const LIMIT: usize = 4096;

pub(super) fn ensure_tables(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS kernel_task_usage_links (
        thread_id TEXT NOT NULL, op_id INTEGER NOT NULL, task_id TEXT NOT NULL,
        PRIMARY KEY(thread_id, op_id));
        CREATE TABLE IF NOT EXISTS kernel_turn_usage (
        thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, op_id INTEGER, payload TEXT NOT NULL,
        PRIMARY KEY(thread_id, turn_id));",
    )?;
    Ok(())
}

pub(super) fn record(db: &Connection, record: &RecordedKernelEvent) -> Result<()> {
    if let KernelEvent::TaskExecutionLinked {
        task_id,
        thread_id,
        op_id,
    } = &record.event
    {
        let prior: Option<String> = db
            .query_row(
                "SELECT task_id FROM kernel_task_usage_links WHERE thread_id=?1 AND op_id=?2",
                params![thread_id, op_id],
                |row| row.get(0),
            )
            .optional()?;
        ensure!(
            prior.as_deref().is_none_or(|id| id == task_id),
            "usage operation task link changed"
        );
        db.execute(
            "INSERT OR IGNORE INTO kernel_task_usage_links VALUES (?1,?2,?3)",
            params![thread_id, op_id, task_id],
        )?;
        return Ok(());
    }
    let (thread_id, turn_id, op_id, usage) = match &record.event {
        KernelEvent::TurnStarted {
            thread_id,
            turn_id,
            op_id,
        } => (thread_id, turn_id, *op_id, None),
        KernelEvent::TurnUsageObserved {
            thread_id,
            turn_id,
            op_id,
            last,
            total,
        } => (thread_id, turn_id, *op_id, Some((*last, *total))),
        _ => return Ok(()),
    };
    let old: Option<String> = db
        .query_row(
            "SELECT payload FROM kernel_turn_usage WHERE thread_id=?1 AND turn_id=?2",
            params![thread_id, turn_id],
            |row| row.get(0),
        )
        .optional()?;
    let mut turn = match old {
        Some(json) => serde_json::from_str::<UsageTurn>(&json)?,
        None => UsageTurn::new(thread_id.clone(), turn_id.clone(), op_id, record.sequence),
    };
    ensure!(turn.op_id == op_id, "usage turn operation identity changed");
    if let Some((last, total)) = usage {
        turn.observe(record.sequence, last, total);
    }
    db.execute(
        "INSERT INTO kernel_turn_usage VALUES (?1,?2,?3,?4)
        ON CONFLICT(thread_id,turn_id) DO UPDATE SET payload=excluded.payload",
        params![thread_id, turn_id, op_id, serde_json::to_string(&turn)?],
    )?;
    Ok(())
}

pub(super) fn load(workspace: &str, task_id: &str) -> Result<Option<UsageProjection>> {
    let Some(location) =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_for_read(workspace)?
    else {
        return Ok(None);
    };
    let Some(store) = super::issue_timeline::read_only::open(&location)? else {
        return Ok(Some(UsageProjection::default()));
    };
    load_connection(&store.connection, task_id).map(Some)
}

fn load_connection(db: &Connection, task_id: &str) -> Result<UsageProjection> {
    let exists: i64 = db.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table'
        AND name IN ('kernel_task_usage_links','kernel_turn_usage')",
        [],
        |row| row.get(0),
    )?;
    if exists != 2 {
        return Ok(UsageProjection::default());
    }
    let mut query = db.prepare(
        "SELECT u.payload FROM kernel_turn_usage u
        JOIN kernel_task_usage_links l ON u.thread_id=l.thread_id AND u.op_id=l.op_id
        WHERE l.task_id=?1
        ORDER BY u.thread_id,u.turn_id LIMIT ?2",
    )?;
    let mut turns = query
        .query_map(params![task_id, LIMIT + 1], |row| row.get::<_, String>(0))?
        .map(|row| Ok(serde_json::from_str::<UsageTurn>(&row?)?))
        .collect::<Result<Vec<_>>>()?;
    let mut query = db.prepare(
        "SELECT thread_id,op_id FROM kernel_task_usage_links
        WHERE task_id=?1 ORDER BY thread_id,op_id LIMIT ?2",
    )?;
    let mut links = query
        .query_map(params![task_id, LIMIT + 1], |row| {
            Ok(TaskUsageLink {
                thread_id: row.get(0)?,
                op_id: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let truncated = turns.len() > LIMIT || links.len() > LIMIT;
    turns.truncate(LIMIT);
    links.truncate(LIMIT);
    Ok(UsageProjection {
        turns,
        links,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation_progressive_activity::ConversationProgressiveTokenUsageBreakdown as Tokens;
    fn tokens(n: u64) -> Tokens {
        Tokens {
            input_tokens: n,
            total_tokens: n,
            ..Tokens::default()
        }
    }
    fn append(db: &Connection, sequence: i64, event: KernelEvent) {
        record(
            db,
            &RecordedKernelEvent {
                sequence,
                recorded_at: "2026-11-26T00:00:00Z".into(),
                event,
            },
        )
        .unwrap();
    }
    #[test]
    fn task_usage_projection_survives_log_retention_and_separates_tasks_on_one_thread() {
        let db = Connection::open_in_memory().unwrap();
        ensure_tables(&db).unwrap();
        for (op_id, task_id, turn_id, offset) in
            [(7, "task-a", "turn-a", 0), (8, "task-b", "turn-b", 3)]
        {
            append(
                &db,
                1 + offset,
                KernelEvent::TaskExecutionLinked {
                    task_id: task_id.into(),
                    thread_id: "thread".into(),
                    op_id,
                },
            );
            append(
                &db,
                2 + offset,
                KernelEvent::TurnStarted {
                    thread_id: "thread".into(),
                    turn_id: turn_id.into(),
                    op_id: Some(op_id),
                },
            );
            append(
                &db,
                3 + offset,
                KernelEvent::TurnUsageObserved {
                    thread_id: "thread".into(),
                    turn_id: turn_id.into(),
                    op_id: Some(op_id),
                    last: tokens(10),
                    total: tokens(100 + offset as u64),
                },
            );
        }
        // A shared thread cannot pull task-b observations into task-a.
        let a = load_connection(&db, "task-a").unwrap();
        assert_eq!(a.turns.len(), 1);
        assert_eq!(a.turns[0].observed, Some(tokens(10)));
        assert_eq!(a.turns[0].turn_id, "turn-a");
        assert_eq!(load_connection(&db, "task-b").unwrap().turns.len(), 1);
        assert!(load_connection(&db, "unknown").unwrap().turns.is_empty());
        // No history table exists: the projection is independent from history retention.
        let replay = KernelEvent::TurnUsageObserved {
            thread_id: "thread".into(),
            turn_id: "turn-a".into(),
            op_id: Some(7),
            last: tokens(10),
            total: tokens(100),
        };
        append(&db, 3, replay.clone());
        append(&db, 9, replay);
        assert_eq!(
            load_connection(&db, "task-a").unwrap().turns[0].observed,
            Some(tokens(10))
        );
    }
}

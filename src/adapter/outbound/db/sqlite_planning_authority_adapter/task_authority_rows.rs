use std::collections::BTreeSet;

use anyhow::{Context, Result};
use rusqlite::params;

use crate::domain::planning::{
    PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskAuthorityDocument,
};

use super::{TASK_LEDGER_VERSION_METADATA_KEY, upsert_metadata};

pub(super) fn replace_task_authority_tables(
    transaction: &rusqlite::Transaction<'_>,
    task_authority: &TaskAuthorityDocument,
    queue_projection: &PriorityQueueProjection,
) -> Result<()> {
    clear_queue_projection_rows(transaction)?;
    sync_task_authority_rows(transaction, task_authority)?;
    insert_queue_projection_tasks(transaction, "active", &queue_projection.active_tasks)?;
    insert_queue_projection_tasks(transaction, "proposed", &queue_projection.proposed_tasks)?;
    insert_queue_projection_skipped(transaction, &queue_projection.skipped_tasks)?;
    upsert_metadata(
        transaction,
        TASK_LEDGER_VERSION_METADATA_KEY,
        &task_authority.version.to_string(),
    )?;
    Ok(())
}

fn sync_task_authority_rows(
    transaction: &rusqlite::Transaction<'_>,
    task_authority: &TaskAuthorityDocument,
) -> Result<()> {
    let desired_task_ids = task_authority
        .tasks
        .iter()
        .map(|task| task.id.trim().to_string())
        .collect::<BTreeSet<_>>();
    delete_stale_task_rows(transaction, &desired_task_ids)?;
    for (index, task) in task_authority.tasks.iter().enumerate() {
        upsert_task_row(transaction, index, task)?;
        let task_id = task.id.trim();
        transaction
            .execute(
                "DELETE FROM planning_task_edges WHERE task_id = ?1",
                params![task_id],
            )
            .with_context(|| format!("failed to clear planning task edges for `{task_id}`"))?;
        insert_task_edges(transaction, task_id, "depends_on", &task.depends_on)?;
        insert_task_edges(transaction, task_id, "blocked_by", &task.blocked_by)?;
    }
    Ok(())
}

fn delete_stale_task_rows(
    transaction: &rusqlite::Transaction<'_>,
    desired_task_ids: &BTreeSet<String>,
) -> Result<()> {
    let mut statement = transaction
        .prepare("SELECT task_id FROM planning_tasks")
        .context("failed to read existing planning task ids")?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .context("failed to iterate existing planning task ids")?;
    let mut stale_task_ids = Vec::new();
    for row in rows {
        let task_id = row.context("failed to decode existing planning task id")?;
        if !desired_task_ids.contains(task_id.trim()) {
            stale_task_ids.push(task_id);
        }
    }
    drop(statement);

    for task_id in stale_task_ids {
        transaction
            .execute(
                "DELETE FROM planning_task_edges WHERE task_id = ?1",
                params![task_id.trim()],
            )
            .with_context(|| format!("failed to clear stale planning task edges `{task_id}`"))?;
        transaction
            .execute(
                "DELETE FROM planning_tasks WHERE task_id = ?1",
                params![task_id.trim()],
            )
            .with_context(|| format!("failed to delete stale planning task `{task_id}`"))?;
    }
    Ok(())
}

fn upsert_task_row(
    transaction: &rusqlite::Transaction<'_>,
    index: usize,
    task: &crate::domain::planning::TaskDefinition,
) -> Result<()> {
    let task_id = task.id.trim();
    transaction
        .execute(
            "INSERT INTO planning_tasks
             (task_id, task_order, direction_id, title, status, base_priority,
              dynamic_priority_delta, combined_priority, updated_at, source_turn_id, content_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(task_id) DO UPDATE SET
                 task_order = excluded.task_order,
                 direction_id = excluded.direction_id,
                 title = excluded.title,
                 status = excluded.status,
                 base_priority = excluded.base_priority,
                 dynamic_priority_delta = excluded.dynamic_priority_delta,
                 combined_priority = excluded.combined_priority,
                 updated_at = excluded.updated_at,
                 source_turn_id = excluded.source_turn_id,
                 content_json = excluded.content_json",
            params![
                task_id,
                index as i64,
                task.direction_id.trim(),
                task.title.trim(),
                task.status.label(),
                task.base_priority,
                task.dynamic_priority_delta,
                task.combined_priority(),
                task.updated_at.as_str(),
                task.source_turn_id.as_deref(),
                serde_json::to_string(task).context("failed to serialize planning task row")?,
            ],
        )
        .with_context(|| format!("failed to persist planning task `{task_id}`"))?;
    Ok(())
}

fn insert_task_edges(
    transaction: &rusqlite::Transaction<'_>,
    task_id: &str,
    edge_kind: &str,
    target_task_ids: &[String],
) -> Result<()> {
    for (index, target_task_id) in target_task_ids.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO planning_task_edges (task_id, edge_kind, target_task_id, edge_order)
                 VALUES (?1, ?2, ?3, ?4)",
                params![task_id, edge_kind, target_task_id.trim(), index as i64],
            )
            .with_context(|| {
                format!("failed to persist planning task edge `{task_id}:{edge_kind}`")
            })?;
    }
    Ok(())
}

fn insert_queue_projection_tasks(
    transaction: &rusqlite::Transaction<'_>,
    bucket: &str,
    tasks: &[PriorityQueueTask],
) -> Result<()> {
    for task in tasks {
        transaction
            .execute(
                "INSERT INTO planning_queue_projection
                 (bucket, rank, task_id, item_kind, content_json)
                 VALUES (?1, ?2, ?3, 'task', ?4)",
                params![
                    bucket,
                    task.rank as i64,
                    task.task_id.trim(),
                    serde_json::to_string(task)
                        .context("failed to serialize planning queue task projection")?,
                ],
            )
            .with_context(|| {
                format!(
                    "failed to persist planning queue projection `{bucket}:{}`",
                    task.task_id
                )
            })?;
    }
    Ok(())
}

fn insert_queue_projection_skipped(
    transaction: &rusqlite::Transaction<'_>,
    skipped_tasks: &[PriorityQueueSkippedTask],
) -> Result<()> {
    for (index, task) in skipped_tasks.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO planning_queue_projection
                 (bucket, rank, task_id, item_kind, content_json)
                 VALUES ('skipped', ?1, ?2, 'skipped', ?3)",
                params![
                    index as i64 + 1,
                    task.task_id.trim(),
                    serde_json::to_string(task)
                        .context("failed to serialize skipped planning queue projection")?,
                ],
            )
            .with_context(|| {
                format!(
                    "failed to persist skipped planning queue projection `{}`",
                    task.task_id
                )
            })?;
    }
    Ok(())
}

pub(super) fn clear_task_authority_tables(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    clear_task_authority_rows(transaction)?;
    transaction
        .execute(
            "DELETE FROM authority_metadata WHERE key = ?1",
            params![TASK_LEDGER_VERSION_METADATA_KEY],
        )
        .context("failed to clear planning task authority metadata")?;
    Ok(())
}

fn clear_task_authority_rows(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    clear_queue_projection_rows(transaction)?;
    transaction
        .execute("DELETE FROM planning_task_edges", [])
        .context("failed to clear planning task edges")?;
    transaction
        .execute("DELETE FROM planning_tasks", [])
        .context("failed to clear planning task rows")?;
    Ok(())
}

fn clear_queue_projection_rows(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction
        .execute("DELETE FROM planning_queue_projection", [])
        .context("failed to clear planning queue projection")?;
    Ok(())
}

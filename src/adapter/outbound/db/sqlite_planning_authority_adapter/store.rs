use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskAuthoritySnapshot;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::application::service::planning::{DIRECTIONS_FILE_PATH, RESULT_OUTPUT_FILE_PATH};
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningAuthorityLocation,
    PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskAuthorityDocument,
};

use super::{
    AUTHORITY_STORE_MODE, AUTHORITY_STORE_SCHEMA_VERSION, TASK_LEDGER_VERSION_METADATA_KEY,
    read_metadata_i64_connection, read_metadata_string_connection, replace_task_authority_tables,
    table_exists,
};

pub(super) fn ensure_schema(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS authority_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS shadow_documents (
                relative_path TEXT PRIMARY KEY,
                content TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS staged_drafts (
                draft_name TEXT PRIMARY KEY,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS staged_draft_files (
                draft_name TEXT NOT NULL,
                active_path TEXT NOT NULL,
                content TEXT NOT NULL,
                PRIMARY KEY (draft_name, active_path),
                FOREIGN KEY (draft_name) REFERENCES staged_drafts(draft_name) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS active_documents (
                relative_path TEXT PRIMARY KEY,
                content TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS planning_tasks (
                task_id TEXT PRIMARY KEY,
                task_order INTEGER NOT NULL,
                direction_id TEXT NOT NULL,
                title TEXT NOT NULL,
                status TEXT NOT NULL,
                base_priority INTEGER NOT NULL,
                dynamic_priority_delta INTEGER NOT NULL,
                combined_priority INTEGER NOT NULL,
                updated_at TEXT NOT NULL,
                source_turn_id TEXT,
                content_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS planning_task_edges (
                task_id TEXT NOT NULL,
                edge_kind TEXT NOT NULL,
                target_task_id TEXT NOT NULL,
                edge_order INTEGER NOT NULL,
                PRIMARY KEY (task_id, edge_kind, edge_order),
                FOREIGN KEY (task_id) REFERENCES planning_tasks(task_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS planning_queue_projection (
                bucket TEXT NOT NULL,
                rank INTEGER NOT NULL,
                task_id TEXT NOT NULL,
                item_kind TEXT NOT NULL,
                content_json TEXT NOT NULL,
                PRIMARY KEY (bucket, rank, task_id),
                FOREIGN KEY (task_id) REFERENCES planning_tasks(task_id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_planning_tasks_status_priority_updated
                ON planning_tasks(status, combined_priority, updated_at);
            CREATE INDEX IF NOT EXISTS idx_planning_tasks_direction
                ON planning_tasks(direction_id);
            CREATE INDEX IF NOT EXISTS idx_planning_task_edges_lookup
                ON planning_task_edges(target_task_id, edge_kind);
            CREATE INDEX IF NOT EXISTS idx_planning_queue_projection_bucket_rank
                ON planning_queue_projection(bucket, rank);

            CREATE TABLE IF NOT EXISTS runtime_claims (
                claim_kind TEXT NOT NULL,
                scope_key TEXT NOT NULL,
                owner_token TEXT NOT NULL,
                claim_value TEXT NOT NULL,
                claimed_at TEXT NOT NULL,
                PRIMARY KEY (claim_kind, scope_key)
            );

            CREATE TABLE IF NOT EXISTS runtime_slot_leases (
                slot_id TEXT PRIMARY KEY,
                updated_at TEXT NOT NULL,
                content TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS runtime_invalid_slot_leases (
                slot_id TEXT PRIMARY KEY,
                detected_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS runtime_session_details (
                session_key TEXT PRIMARY KEY,
                slot_id TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                content TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS runtime_distributor_queue (
                queue_item_id TEXT PRIMARY KEY,
                session_key TEXT NOT NULL,
                queue_state TEXT NOT NULL,
                enqueued_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                content TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS runtime_events (
                sequence INTEGER PRIMARY KEY,
                event_kind TEXT NOT NULL,
                projection_kind TEXT NOT NULL,
                projection_key TEXT NOT NULL,
                observed_planning_revision INTEGER NOT NULL,
                summary TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                recorded_at TEXT NOT NULL
            );
            "#,
        )
        .context("failed to initialize authority-store schema")?;
    Ok(())
}

pub(super) fn store_shadow_documents(
    connection: &mut Connection,
    location: &PlanningAuthorityLocation,
    documents: &BTreeMap<String, String>,
) -> Result<()> {
    let transaction = connection
        .transaction()
        .context("failed to open shadow-store transaction")?;
    transaction
        .execute("DELETE FROM shadow_documents", [])
        .context("failed to clear shadow documents")?;
    for (relative_path, content) in documents {
        transaction
            .execute(
                "INSERT INTO shadow_documents (relative_path, content) VALUES (?1, ?2)",
                params![relative_path, content],
            )
            .with_context(|| format!("failed to mirror `{relative_path}` into the shadow store"))?;
    }

    upsert_authority_metadata(&transaction, location, "last_synced_at")?;

    transaction
        .commit()
        .context("failed to commit shadow-store transaction")?;
    Ok(())
}

pub(super) fn upsert_authority_metadata(
    transaction: &rusqlite::Transaction<'_>,
    location: &PlanningAuthorityLocation,
    timestamp_key: &str,
) -> Result<()> {
    upsert_metadata(
        transaction,
        "schema_version",
        &AUTHORITY_STORE_SCHEMA_VERSION.to_string(),
    )?;
    upsert_metadata(transaction, "mode", AUTHORITY_STORE_MODE)?;
    upsert_metadata(
        transaction,
        "canonical_repo_root",
        &location.canonical_repo_root,
    )?;
    upsert_metadata(transaction, "workspace_root", &location.workspace_root)?;
    upsert_metadata(transaction, timestamp_key, &Utc::now().to_rfc3339())?;
    Ok(())
}

pub(super) fn upsert_metadata(
    transaction: &rusqlite::Transaction<'_>,
    key: &str,
    value: &str,
) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO authority_metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .with_context(|| format!("failed to update authority metadata `{key}`"))?;
    Ok(())
}

pub(super) fn load_shadow_documents(connection: &Connection) -> Result<BTreeMap<String, String>> {
    if !table_exists(connection, "shadow_documents")? {
        return Ok(BTreeMap::new());
    }

    let mut statement = connection
        .prepare("SELECT relative_path, content FROM shadow_documents ORDER BY relative_path")
        .context("failed to read shadow-store documents")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate shadow-store documents")?;
    let mut documents = BTreeMap::new();
    for row in rows {
        let (relative_path, content) = row.context("failed to decode shadow-store row")?;
        documents.insert(relative_path, content);
    }

    Ok(documents)
}

pub(super) fn load_active_documents(connection: &Connection) -> Result<BTreeMap<String, String>> {
    let mut statement = connection
        .prepare("SELECT relative_path, content FROM active_documents ORDER BY relative_path")
        .context("failed to read active authority documents")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate active authority documents")?;
    let mut documents = BTreeMap::new();
    for row in rows {
        let (relative_path, content) = row.context("failed to decode active authority row")?;
        documents.insert(relative_path, content);
    }

    Ok(documents)
}

pub(super) fn load_active_authority_documents(
    connection: &Connection,
) -> Result<BTreeMap<String, String>> {
    load_active_documents(connection)
}

pub(super) fn load_active_workspace_record(
    connection: &Connection,
) -> Result<PlanningWorkspaceLoadRecord> {
    let documents = load_active_documents(connection)?;
    Ok(PlanningWorkspaceLoadRecord {
        directions_toml: documents.get(DIRECTIONS_FILE_PATH).cloned(),
        result_output_markdown: documents.get(RESULT_OUTPUT_FILE_PATH).cloned(),
    })
}

pub(super) fn load_active_document(
    connection: &Connection,
    relative_path: &str,
) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT content FROM active_documents WHERE relative_path = ?1",
            params![relative_path],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("failed to read active authority document `{relative_path}`"))
}

pub(super) fn load_task_authority_snapshot_from_connection(
    connection: &Connection,
) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
    let Some(task_authority) = load_task_authority_from_connection(connection)? else {
        return Ok(None);
    };
    let queue_projection =
        load_queue_projection_from_connection(connection)?.unwrap_or_else(empty_queue_projection);
    let planning_revision =
        read_metadata_i64_connection(connection, "planning_revision")?.unwrap_or(0);
    Ok(Some(PlanningTaskAuthoritySnapshot {
        planning_revision,
        task_authority,
        queue_projection,
    }))
}

pub(super) fn load_task_authority_from_connection(
    connection: &Connection,
) -> Result<Option<TaskAuthorityDocument>> {
    if !task_authority_exists(connection)? {
        return Ok(None);
    }

    let version = read_metadata_i64_connection(connection, TASK_LEDGER_VERSION_METADATA_KEY)?
        .unwrap_or(i64::from(PLANNING_FORMAT_VERSION)) as u32;
    let mut statement = connection
        .prepare(
            "SELECT task_id, content_json
             FROM planning_tasks
             ORDER BY task_order ASC, task_id ASC",
        )
        .context("failed to read planning task rows")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate planning task rows")?;
    let mut tasks = Vec::new();
    for row in rows {
        let (task_id, content_json) = row.context("failed to decode planning task row")?;
        tasks.push(
            serde_json::from_str(&content_json)
                .with_context(|| format!("failed to deserialize planning task row `{task_id}`"))?,
        );
    }

    Ok(Some(TaskAuthorityDocument { version, tasks }))
}

pub(super) fn load_queue_projection_from_connection(
    connection: &Connection,
) -> Result<Option<PriorityQueueProjection>> {
    if !task_authority_exists(connection)? {
        return Ok(None);
    }

    let mut active_tasks = Vec::new();
    let mut proposed_tasks = Vec::new();
    let mut skipped_tasks = Vec::new();
    let mut statement = connection
        .prepare(
            "SELECT bucket, task_id, content_json
             FROM planning_queue_projection
             ORDER BY bucket ASC, rank ASC, task_id ASC",
        )
        .context("failed to read planning queue projection")?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .context("failed to iterate planning queue projection")?;
    for row in rows {
        let (bucket, task_id, content_json) =
            row.context("failed to decode planning queue projection row")?;
        match bucket.as_str() {
            "active" => {
                active_tasks.push(
                    serde_json::from_str::<PriorityQueueTask>(&content_json).with_context(
                        || format!("failed to deserialize active queue projection `{task_id}`"),
                    )?,
                );
            }
            "proposed" => {
                proposed_tasks.push(
                    serde_json::from_str::<PriorityQueueTask>(&content_json).with_context(
                        || format!("failed to deserialize proposed queue projection `{task_id}`"),
                    )?,
                );
            }
            "skipped" => {
                skipped_tasks.push(
                    serde_json::from_str::<PriorityQueueSkippedTask>(&content_json).with_context(
                        || format!("failed to deserialize skipped queue projection `{task_id}`"),
                    )?,
                );
            }
            _ => {}
        }
    }

    Ok(Some(PriorityQueueProjection {
        next_task: active_tasks.first().cloned(),
        active_tasks,
        proposed_tasks,
        skipped_tasks,
    }))
}

pub(super) fn task_authority_exists(connection: &Connection) -> Result<bool> {
    if read_metadata_string_connection(connection, TASK_LEDGER_VERSION_METADATA_KEY)?.is_some() {
        return Ok(true);
    }
    connection
        .query_row("SELECT 1 FROM planning_tasks LIMIT 1", [], |_| Ok(()))
        .optional()
        .context("failed to inspect planning task authority")
        .map(|value| value.is_some())
}

pub(super) fn empty_queue_projection() -> PriorityQueueProjection {
    PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    }
}

pub(super) fn reconcile_task_authority_with_directions(
    transaction: &rusqlite::Transaction<'_>,
    directions_toml: Option<&str>,
) -> Result<()> {
    let Some(mut task_authority) = load_task_authority_from_connection(transaction)? else {
        return Ok(());
    };
    let direction_ids = match directions_toml {
        Some(directions_toml) => parse_direction_ids(directions_toml)
            .with_context(|| format!("failed to parse `{DIRECTIONS_FILE_PATH}`"))?,
        None => BTreeSet::new(),
    };
    if !prune_task_authority_to_direction_ids(&mut task_authority, &direction_ids) {
        return Ok(());
    }
    replace_task_authority_tables(transaction, &task_authority, &empty_queue_projection())?;
    Ok(())
}

pub(super) fn parse_direction_ids(directions_toml: &str) -> Result<BTreeSet<String>> {
    Ok(toml::from_str::<DirectionCatalogDocument>(directions_toml)?
        .directions
        .into_iter()
        .map(|direction| direction.id.trim().to_string())
        .collect())
}

pub(super) fn prune_task_authority_to_direction_ids(
    task_authority: &mut TaskAuthorityDocument,
    direction_ids: &BTreeSet<String>,
) -> bool {
    let mut removed_task_ids = BTreeSet::new();
    task_authority.tasks.retain(|task| {
        let keep = direction_ids.contains(task.direction_id.trim());
        if !keep {
            removed_task_ids.insert(task.id.trim().to_string());
        }
        keep
    });
    if removed_task_ids.is_empty() {
        return false;
    }
    for task in &mut task_authority.tasks {
        task.depends_on
            .retain(|task_id| !removed_task_ids.contains(task_id.trim()));
        task.blocked_by
            .retain(|task_id| !removed_task_ids.contains(task_id.trim()));
    }
    true
}

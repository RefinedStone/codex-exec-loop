use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;

pub(super) fn apply_active_workspace_record(
    transaction: &rusqlite::Transaction<'_>,
    record: &PlanningWorkspaceLoadRecord,
) -> Result<bool> {
    let mut changed = false;
    changed |= set_active_document(
        transaction,
        RESULT_OUTPUT_FILE_PATH,
        record.result_output_markdown.as_deref(),
    )?;
    Ok(changed)
}

pub(super) fn set_active_document(
    transaction: &rusqlite::Transaction<'_>,
    relative_path: &str,
    body: Option<&str>,
) -> Result<bool> {
    let existing = transaction
        .query_row(
            "SELECT content FROM active_documents WHERE relative_path = ?1",
            params![relative_path],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("failed to read active document `{relative_path}`"))?;
    if existing.as_deref() == body {
        return Ok(false);
    }

    match body {
        Some(body) => {
            transaction
                .execute(
                    "INSERT INTO active_documents (relative_path, content) VALUES (?1, ?2)
                     ON CONFLICT(relative_path) DO UPDATE SET content = excluded.content",
                    params![relative_path, body],
                )
                .with_context(|| format!("failed to store active document `{relative_path}`"))?;
        }
        None => {
            transaction
                .execute(
                    "DELETE FROM active_documents WHERE relative_path = ?1",
                    params![relative_path],
                )
                .with_context(|| format!("failed to delete active document `{relative_path}`"))?;
        }
    }

    Ok(true)
}

pub(super) fn remove_active_documents(
    transaction: &rusqlite::Transaction<'_>,
    relative_path: &str,
) -> Result<bool> {
    let deleted_rows = transaction
        .execute(
            "DELETE FROM active_documents
             WHERE relative_path = ?1 OR relative_path LIKE ?2",
            params![relative_path, format!("{relative_path}/%")],
        )
        .with_context(|| format!("failed to remove active authority entry `{relative_path}`"))?;
    Ok(deleted_rows > 0)
}

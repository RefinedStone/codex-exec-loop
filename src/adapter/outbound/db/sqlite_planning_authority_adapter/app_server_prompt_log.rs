use anyhow::{Context, Result};
use rusqlite::params;

use crate::application::port::outbound::app_server_prompt_log_port::{
    AppServerPromptInteractionRecord, AppServerPromptInteractionSnapshot, AppServerPromptLogPort,
};

use super::store::upsert_authority_metadata;
use super::{SqlitePlanningAuthorityAdapter, open_authority_connection};

const RETAINED_PROMPT_INTERACTION_COUNT: i64 = 200;

impl SqlitePlanningAuthorityAdapter {
    pub(crate) fn append_app_server_prompt_interaction_record(
        workspace_dir: &str,
        record: AppServerPromptInteractionRecord,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open app-server prompt log transaction")?;
        let content_json = serde_json::to_string(&record)
            .context("failed to serialize app-server prompt log record")?;

        upsert_authority_metadata(&transaction, &location, "last_app_server_prompt_log_at")?;
        transaction
            .execute(
                "INSERT INTO app_server_prompt_interactions
                 (interaction_id, session_kind, operation, service_name, thread_id, turn_id, status, started_at, completed_at, content_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    record.interaction_id,
                    record.session_kind,
                    record.operation,
                    record.service_name,
                    record.thread_id,
                    record.turn_id,
                    record.status,
                    record.started_at,
                    record.completed_at,
                    content_json,
                ],
            )
            .context("failed to insert app-server prompt log record")?;
        transaction
            .execute(
                "DELETE FROM app_server_prompt_interactions
                 WHERE sequence NOT IN (
                     SELECT sequence
                     FROM app_server_prompt_interactions
                     ORDER BY sequence DESC
                     LIMIT ?1
                 )",
                params![RETAINED_PROMPT_INTERACTION_COUNT],
            )
            .context("failed to trim old app-server prompt log records")?;
        transaction
            .commit()
            .context("failed to commit app-server prompt log transaction")?;
        Ok(())
    }

    pub(crate) fn load_recent_app_server_prompt_interaction_records(
        workspace_dir: &str,
        limit: usize,
    ) -> Result<AppServerPromptInteractionSnapshot> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        let bounded_limit = i64::try_from(limit.clamp(1, 200)).unwrap_or(200);
        let mut statement = connection
            .prepare(
                "SELECT sequence, content_json
                 FROM app_server_prompt_interactions
                 ORDER BY sequence DESC
                 LIMIT ?1",
            )
            .context("failed to prepare app-server prompt log query")?;
        let rows = statement
            .query_map(params![bounded_limit], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .context("failed to query app-server prompt log records")?;

        let mut records = Vec::new();
        for row in rows {
            let (sequence, content_json) =
                row.context("failed to read app-server prompt log row")?;
            let mut record: AppServerPromptInteractionRecord = serde_json::from_str(&content_json)
                .with_context(|| {
                    format!("failed to decode app-server prompt log record {sequence}")
                })?;
            record.sequence = sequence;
            records.push(record);
        }

        Ok(AppServerPromptInteractionSnapshot { records })
    }
}

impl AppServerPromptLogPort for SqlitePlanningAuthorityAdapter {
    fn append_app_server_prompt_interaction(
        &self,
        workspace_dir: &str,
        record: AppServerPromptInteractionRecord,
    ) -> Result<()> {
        Self::append_app_server_prompt_interaction_record(workspace_dir, record)
    }

    fn load_recent_app_server_prompt_interactions(
        &self,
        workspace_dir: &str,
        limit: usize,
    ) -> Result<AppServerPromptInteractionSnapshot> {
        Self::load_recent_app_server_prompt_interaction_records(workspace_dir, limit)
    }
}

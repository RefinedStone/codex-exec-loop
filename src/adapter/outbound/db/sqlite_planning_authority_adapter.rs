use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningDirectionAuthoritySnapshot,
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskAuthoritySnapshot,
    PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
};
mod active_documents;
mod draft_files;
mod repo_scoped_workspace;
mod runtime_projection;
mod store;
mod task_authority_rows;
mod workspace_paths;

use self::active_documents::{
    apply_active_workspace_record, remove_active_documents, set_active_document,
};
use self::store::*;
use self::task_authority_rows::{clear_task_authority_tables, replace_task_authority_tables};
use crate::domain::planning::{
    PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection,
    PlanningAuthorityShadowStoreSyncState,
};

const AUTHORITY_STORE_SCHEMA_VERSION: i64 = 5;
const AUTHORITY_STORE_MODE: &str = "authority-store";
const OFFICIAL_REFRESH_SCOPE_KEY: &str = "official-refresh";
const DISTRIBUTOR_QUEUE_CLAIM_KIND: &str = "distributor-queue-head";
const CLAIM_STALE_AFTER_SECS: i64 = 300;
const TASK_LEDGER_VERSION_METADATA_KEY: &str = "task_authority_version";
#[derive(Default)]
pub struct SqlitePlanningAuthorityAdapter;

impl SqlitePlanningAuthorityAdapter {
    pub fn new() -> Self {
        Self
    }

    pub(crate) fn commit_active_workspace_files(
        workspace_dir: &str,
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open authority-store active commit transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_active_commit_at")?;
        let changed = apply_active_workspace_record(&transaction, record)?;
        if changed {
            bump_planning_revision(&transaction)?;
        }
        transaction
            .commit()
            .context("failed to commit authority-store active commit transaction")?;

        Ok(())
    }

    pub(crate) fn load_active_workspace_files(
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_active_workspace_record(&connection)
    }

    pub(crate) fn load_active_planning_file(
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_active_document(&connection, relative_path)
    }

    pub(crate) fn load_task_authority_snapshot(
        workspace_dir: &str,
    ) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_task_authority_snapshot_from_connection(&connection)
    }

    pub(crate) fn load_direction_authority_snapshot(
        workspace_dir: &str,
    ) -> Result<Option<PlanningDirectionAuthoritySnapshot>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_direction_authority_snapshot_from_connection(&connection)
    }

    pub(crate) fn commit_direction_authority_snapshot(
        workspace_dir: &str,
        commit: PlanningDirectionAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open direction authority commit transaction")?;
        let current_revision = read_metadata_i64(&transaction, "planning_revision")?.unwrap_or(0);
        if let Some(observed_revision) = commit.observed_planning_revision
            && observed_revision != current_revision
        {
            return Ok(PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: observed_revision,
                current_planning_revision: current_revision,
            });
        }
        if let Some(existing_snapshot) =
            load_direction_authority_snapshot_from_connection(&transaction)?
            && existing_snapshot.directions == *commit.directions
        {
            return Ok(PlanningTaskAuthorityCommitResult::Committed {
                planning_revision: current_revision,
            });
        }

        upsert_authority_metadata(
            &transaction,
            &location,
            "last_direction_authority_commit_at",
        )?;
        replace_direction_authority_tables(&transaction, commit.directions)?;
        reconcile_task_authority_with_directions(&transaction, Some(commit.directions))?;
        let planning_revision = bump_planning_revision(&transaction)?;
        transaction
            .commit()
            .context("failed to commit direction authority transaction")?;

        Ok(PlanningTaskAuthorityCommitResult::Committed { planning_revision })
    }

    pub(crate) fn clear_direction_authority_snapshot(workspace_dir: &str) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open direction authority clear transaction")?;
        upsert_authority_metadata(
            &transaction,
            &location,
            "last_direction_authority_commit_at",
        )?;
        clear_direction_authority_tables(&transaction)?;
        reconcile_task_authority_with_directions(&transaction, None)?;
        bump_planning_revision(&transaction)?;
        transaction
            .commit()
            .context("failed to clear direction authority transaction")?;

        Ok(())
    }

    pub(crate) fn commit_task_authority_snapshot(
        workspace_dir: &str,
        commit: PlanningTaskAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open task authority commit transaction")?;
        let current_revision = read_metadata_i64(&transaction, "planning_revision")?.unwrap_or(0);
        if let Some(observed_revision) = commit.observed_planning_revision
            && observed_revision != current_revision
        {
            return Ok(PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: observed_revision,
                current_planning_revision: current_revision,
            });
        }
        if let Some(existing_snapshot) = load_task_authority_snapshot_from_connection(&transaction)?
            && existing_snapshot.task_authority == *commit.task_authority
            && existing_snapshot.queue_projection == *commit.queue_projection
        {
            return Ok(PlanningTaskAuthorityCommitResult::Committed {
                planning_revision: current_revision,
            });
        }

        upsert_authority_metadata(&transaction, &location, "last_task_authority_commit_at")?;
        replace_task_authority_tables(
            &transaction,
            commit.task_authority,
            commit.queue_projection,
        )?;
        let planning_revision = bump_planning_revision(&transaction)?;
        transaction
            .commit()
            .context("failed to commit task authority transaction")?;

        Ok(PlanningTaskAuthorityCommitResult::Committed { planning_revision })
    }

    pub(crate) fn clear_task_authority_snapshot(workspace_dir: &str) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open task authority clear transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_task_authority_commit_at")?;
        clear_task_authority_tables(&transaction)?;
        bump_planning_revision(&transaction)?;
        transaction
            .commit()
            .context("failed to clear task authority transaction")?;

        Ok(())
    }

    pub(crate) fn replace_active_planning_file(
        workspace_dir: &str,
        relative_path: &str,
        body: Option<&str>,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open authority-store active file transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_active_commit_at")?;
        let changed = set_active_document(&transaction, relative_path, body)?;
        if changed {
            bump_planning_revision(&transaction)?;
        }
        transaction
            .commit()
            .context("failed to commit authority-store active file transaction")?;

        Ok(())
    }

    pub(crate) fn remove_active_planning_entry(
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open authority-store active removal transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_active_commit_at")?;
        let changed = remove_active_documents(&transaction, relative_path)?;
        if changed {
            bump_planning_revision(&transaction)?;
        }
        transaction
            .commit()
            .context("failed to commit authority-store active removal transaction")?;

        Ok(())
    }

    fn inspect_shadow_store_impl(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection> {
        let location = self.resolve_authority_location(workspace_dir)?;
        let authority_store_path = PathBuf::from(&location.authority_store_path);
        let had_store = authority_store_path.is_file();
        let mut connection = open_authority_connection(&location)?;
        let previous_documents = load_shadow_documents(&connection)?;
        let source_documents = load_active_authority_documents(&connection)?;
        let shadow_parity_issues = compare_shadow_documents(&source_documents, &previous_documents);
        store_shadow_documents(&mut connection, &location, &source_documents)?;

        let mirrored_documents = load_shadow_documents(&connection)?;
        let post_sync_issues = compare_shadow_documents(&source_documents, &mirrored_documents);
        if !post_sync_issues.is_empty() {
            let summary = post_sync_issues.join(", ");
            return Err(anyhow!(
                "shadow store parity check failed after sync: {summary}"
            ));
        }

        let sync_state = if !had_store || previous_documents.is_empty() {
            PlanningAuthorityShadowStoreSyncState::Bootstrapped
        } else if shadow_parity_issues.is_empty() {
            PlanningAuthorityShadowStoreSyncState::InSync
        } else {
            PlanningAuthorityShadowStoreSyncState::Resynced
        };
        let parity_issue_examples = shadow_parity_issues
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>();

        Ok(PlanningAuthorityShadowStoreInspection {
            location,
            sync_state,
            mirrored_document_count: source_documents.len(),
            parity_issue_count: shadow_parity_issues.len(),
            parity_issue_examples,
        })
    }
}

fn compare_shadow_documents(
    source_documents: &BTreeMap<String, String>,
    mirrored_documents: &BTreeMap<String, String>,
) -> Vec<String> {
    let document_paths = source_documents
        .keys()
        .chain(mirrored_documents.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut issues = Vec::new();
    for relative_path in document_paths {
        match (
            source_documents.get(&relative_path),
            mirrored_documents.get(&relative_path),
        ) {
            (Some(_), None) => issues.push(format!("{relative_path}: missing from shadow store")),
            (None, Some(_)) => issues.push(format!(
                "{relative_path}: shadow store contains stale content"
            )),
            (Some(source), Some(mirrored)) if source != mirrored => {
                issues.push(format!("{relative_path}: content mismatch"));
            }
            _ => {}
        }
    }

    issues
}

impl PlanningAuthorityPort for SqlitePlanningAuthorityAdapter {
    fn resolve_authority_location(&self, workspace_dir: &str) -> Result<PlanningAuthorityLocation> {
        Self::resolve_authority_location_from_workspace(workspace_dir)
    }

    fn inspect_shadow_store(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection> {
        self.inspect_shadow_store_impl(workspace_dir)
    }

    fn reserve_next_official_refresh_order(&self, workspace_dir: &str) -> Result<u64> {
        Self::reserve_next_official_refresh_order(workspace_dir)
    }

    fn acquire_official_refresh_claim(
        &self,
        workspace_dir: &str,
        refresh_order: u64,
        owner_token: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshClaimStatus> {
        Self::acquire_official_refresh_claim(workspace_dir, refresh_order, owner_token)
    }

    fn release_official_refresh_claim(
        &self,
        workspace_dir: &str,
        refresh_order: u64,
        owner_token: &str,
    ) -> Result<()> {
        Self::release_official_refresh_claim(workspace_dir, refresh_order, owner_token)
    }

    fn try_acquire_distributor_queue_claim(
        &self,
        workspace_dir: &str,
        queue_item_id: &str,
        owner_token: &str,
    ) -> Result<bool> {
        Self::try_acquire_distributor_queue_claim(workspace_dir, queue_item_id, owner_token)
    }

    fn release_distributor_queue_claim(
        &self,
        workspace_dir: &str,
        queue_item_id: &str,
        owner_token: &str,
    ) -> Result<()> {
        Self::release_distributor_queue_claim(workspace_dir, queue_item_id, owner_token)
    }

    fn load_runtime_projections(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        Self::load_runtime_projections(workspace_dir)
    }

    fn upsert_runtime_slot_lease(
        &self,
        workspace_dir: &str,
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()> {
        Self::upsert_runtime_slot_lease(workspace_dir, lease)
    }

    fn remove_runtime_slot_lease(&self, workspace_dir: &str, slot_id: &str) -> Result<()> {
        Self::remove_runtime_slot_lease(workspace_dir, slot_id)
    }

    fn upsert_runtime_session_detail(
        &self,
        workspace_dir: &str,
        detail: &ParallelModeAgentSessionDetailSnapshot,
    ) -> Result<()> {
        Self::upsert_runtime_session_detail(workspace_dir, detail)
    }

    fn upsert_runtime_distributor_queue_record(
        &self,
        workspace_dir: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
    ) -> Result<()> {
        Self::upsert_runtime_distributor_queue_record(workspace_dir, record)
    }
}

impl PlanningTaskRepositoryPort for SqlitePlanningAuthorityAdapter {
    fn load_direction_authority_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<PlanningDirectionAuthoritySnapshot>> {
        Self::load_direction_authority_snapshot(workspace_dir)
    }

    fn commit_direction_authority_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningDirectionAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        Self::commit_direction_authority_snapshot(workspace_dir, commit)
    }

    fn clear_direction_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        Self::clear_direction_authority_snapshot(workspace_dir)
    }

    fn load_task_authority_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
        Self::load_task_authority_snapshot(workspace_dir)
    }

    fn commit_task_authority_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningTaskAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        Self::commit_task_authority_snapshot(workspace_dir, commit)
    }

    fn clear_task_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        Self::clear_task_authority_snapshot(workspace_dir)
    }
}

fn open_authority_connection(location: &PlanningAuthorityLocation) -> Result<Connection> {
    let authority_store_path = Path::new(&location.authority_store_path);
    migrate_legacy_authority_store_if_needed(location)?;
    if let Some(parent) = authority_store_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let connection = Connection::open(authority_store_path)
        .with_context(|| format!("failed to open {}", authority_store_path.display()))?;
    validate_authority_store_schema(&connection)?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .context("failed to enable authority-store foreign keys")?;
    ensure_schema(&connection)?;
    Ok(connection)
}

fn migrate_legacy_authority_store_if_needed(location: &PlanningAuthorityLocation) -> Result<()> {
    let authority_store_path = Path::new(&location.authority_store_path);
    if authority_store_path.exists() {
        return Ok(());
    }

    let legacy_store_path = Path::new(&location.canonical_repo_root)
        .join(".codex-exec-loop/runtime/planning-authority.db");
    if !legacy_store_path.is_file() {
        return Ok(());
    }

    if let Some(parent) = authority_store_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(&legacy_store_path, authority_store_path).with_context(|| {
        format!(
            "failed to migrate legacy authority store from {} to {}",
            legacy_store_path.display(),
            authority_store_path.display()
        )
    })?;
    Ok(())
}

fn load_schema_version(connection: &Connection) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("failed to read authority-store schema version")
}

fn validate_authority_store_schema(connection: &Connection) -> Result<()> {
    let metadata_exists = table_exists(connection, "authority_metadata")?;
    if !metadata_exists {
        return Ok(());
    }

    if let Some(schema_version) = load_schema_version(connection)?
        && !matches!(
            schema_version.parse::<i64>().ok(),
            Some(4) | Some(AUTHORITY_STORE_SCHEMA_VERSION)
        )
    {
        return Err(anyhow!(
            "unsupported authority-store schema version: {schema_version}"
        ));
    }

    Ok(())
}

fn read_metadata_string_connection(connection: &Connection, key: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("failed to read authority metadata `{key}`"))
}

fn read_metadata_i64_connection(connection: &Connection, key: &str) -> Result<Option<i64>> {
    read_metadata_string_connection(connection, key)
        .map(|value| value.and_then(|value| value.parse::<i64>().ok()))
}

fn bump_planning_revision(transaction: &rusqlite::Transaction<'_>) -> Result<i64> {
    let next_revision = read_metadata_i64(transaction, "planning_revision")?.unwrap_or(0) + 1;
    upsert_metadata(transaction, "planning_revision", &next_revision.to_string())?;
    Ok(next_revision)
}

fn read_metadata_i64(transaction: &rusqlite::Transaction<'_>, key: &str) -> Result<Option<i64>> {
    transaction
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("failed to read authority metadata `{key}`"))
        .map(|value| value.and_then(|value| value.parse::<i64>().ok()))
}

fn table_exists(connection: &Connection, table_name: &str) -> Result<bool> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table_name],
            |_| Ok(()),
        )
        .optional()
        .with_context(|| format!("failed to inspect sqlite table `{table_name}`"))
        .map(|value| value.is_some())
}

#[cfg(test)]
mod tests;

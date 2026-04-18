use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityOfficialRefreshClaimStatus, PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadFileRecord, PlanningDraftLoadRecord,
    PlanningDraftStageRecord, PlanningStagedFileRecord, PlanningWorkspaceLoadRecord,
};
use crate::application::service::planning_contract::{
    ACTIVE_PLANNING_FILE_PATHS, DIRECTIONS_FILE_PATH, PLAN_OFF_FILE_PATH,
    PLANNING_DIRECTION_DOCS_DIRECTORY, PLANNING_PROMPTS_DIRECTORY, QUEUE_SNAPSHOT_FILE_PATH,
    RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
};
use crate::domain::planning::{
    PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection,
    PlanningAuthorityShadowStoreSyncState,
};

const RUNTIME_DIRECTORY: &str = ".codex-exec-loop/runtime";
const AUTHORITY_STORE_FILE_NAME: &str = "planning-authority.db";
const LEGACY_SHADOW_STORE_SCHEMA_VERSION: i64 = 1;
const ACTIVE_DRAFT_SCHEMA_VERSION: i64 = 2;
const AUTHORITY_STORE_SCHEMA_VERSION: i64 = 3;
const AUTHORITY_STORE_MODE: &str = "authority-store";
const OFFICIAL_REFRESH_SCOPE_KEY: &str = "official-refresh";
const DISTRIBUTOR_QUEUE_CLAIM_KIND: &str = "distributor-queue-head";

#[derive(Default)]
pub struct SqlitePlanningAuthorityAdapter;

impl SqlitePlanningAuthorityAdapter {
    pub fn new() -> Self {
        Self
    }

    pub(crate) fn is_git_backed_workspace(workspace_dir: &str) -> bool {
        resolve_canonical_repo_root(workspace_dir).is_some()
    }

    pub(crate) fn resolve_active_workspace_root(workspace_dir: &str) -> PathBuf {
        Self::resolve_authority_location_from_workspace(workspace_dir)
            .map(|location| PathBuf::from(location.canonical_repo_root))
            .unwrap_or_else(|_| canonicalize_best_effort(Path::new(workspace_dir)))
    }

    pub(crate) fn stage_repo_scoped_draft_files(
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open authority-store draft stage transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_draft_updated_at")?;
        upsert_draft_entry(&transaction, draft_name)?;
        transaction
            .execute(
                "DELETE FROM staged_draft_files WHERE draft_name = ?1",
                params![draft_name],
            )
            .with_context(|| format!("failed to clear staged draft `{draft_name}`"))?;

        let mut staged_files = Vec::with_capacity(files.len());
        for file in files {
            transaction
                .execute(
                    "INSERT INTO staged_draft_files (draft_name, active_path, content)
                     VALUES (?1, ?2, ?3)",
                    params![draft_name, &file.active_path, &file.body],
                )
                .with_context(|| {
                    format!(
                        "failed to persist staged draft file `{}` for `{draft_name}`",
                        file.active_path
                    )
                })?;
            staged_files.push(PlanningStagedFileRecord {
                active_path: file.active_path.clone(),
                staged_path: draft_display_path(&location, draft_name, &file.active_path),
            });
        }

        transaction
            .commit()
            .context("failed to commit authority-store draft stage transaction")?;

        Ok(PlanningDraftStageRecord {
            draft_name: draft_name.to_string(),
            draft_directory: draft_directory_display_path(&location, draft_name),
            staged_files,
        })
    }

    pub(crate) fn load_repo_scoped_draft_files(
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        let draft_exists = connection
            .query_row(
                "SELECT 1 FROM staged_drafts WHERE draft_name = ?1",
                params![draft_name],
                |_| Ok(()),
            )
            .optional()
            .with_context(|| format!("failed to inspect staged draft `{draft_name}`"))?
            .is_some();
        if !draft_exists {
            return Err(anyhow!("staged draft `{draft_name}` does not exist"));
        }

        let mut statement = connection
            .prepare(
                "SELECT active_path, content
                 FROM staged_draft_files
                 WHERE draft_name = ?1
                 ORDER BY active_path",
            )
            .with_context(|| format!("failed to read staged draft `{draft_name}`"))?;
        let rows = statement
            .query_map(params![draft_name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .with_context(|| format!("failed to iterate staged draft `{draft_name}`"))?;

        let mut staged_files = Vec::new();
        for row in rows {
            let (active_path, body) = row.context("failed to decode staged draft row")?;
            staged_files.push(PlanningDraftLoadFileRecord {
                staged_path: draft_display_path(&location, draft_name, &active_path),
                body,
                active_path,
            });
        }

        Ok(PlanningDraftLoadRecord {
            draft_name: draft_name.to_string(),
            draft_directory: draft_directory_display_path(&location, draft_name),
            staged_files,
        })
    }

    pub(crate) fn replace_repo_scoped_draft_file(
        workspace_dir: &str,
        draft_name: &str,
        active_path: &str,
        body: &str,
    ) -> Result<String> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open authority-store draft replace transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_draft_updated_at")?;
        upsert_draft_entry(&transaction, draft_name)?;
        transaction
            .execute(
                "INSERT INTO staged_draft_files (draft_name, active_path, content)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(draft_name, active_path) DO UPDATE
                 SET content = excluded.content",
                params![draft_name, active_path, body],
            )
            .with_context(|| {
                format!("failed to update staged draft file `{active_path}` for `{draft_name}`")
            })?;
        transaction
            .commit()
            .context("failed to commit authority-store draft replace transaction")?;
        Ok(draft_display_path(&location, draft_name, active_path))
    }

    pub(crate) fn commit_active_workspace_files(
        workspace_dir: &str,
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        bootstrap_active_documents(&mut connection, &location)?;

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

        export_active_workspace_record(&location, record)?;
        Ok(())
    }

    pub(crate) fn replace_active_planning_file(
        workspace_dir: &str,
        relative_path: &str,
        body: Option<&str>,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        bootstrap_active_documents(&mut connection, &location)?;

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

        export_active_document(&location, relative_path, body)?;
        Ok(())
    }

    pub(crate) fn remove_active_planning_entry(
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        bootstrap_active_documents(&mut connection, &location)?;
        let canonical_repo_root = PathBuf::from(&location.canonical_repo_root);

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

        let export_path = canonical_repo_root.join(relative_path);
        if export_path.exists() {
            if export_path.is_dir() {
                fs::remove_dir_all(&export_path)
                    .with_context(|| format!("failed to remove {}", export_path.display()))?;
            } else {
                fs::remove_file(&export_path)
                    .with_context(|| format!("failed to remove {}", export_path.display()))?;
            }
        }
        Ok(())
    }

    pub(crate) fn reserve_next_official_refresh_order(workspace_dir: &str) -> Result<u64> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open official refresh order transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        let next_refresh_order =
            read_metadata_i64(&transaction, "next_official_refresh_order")?.unwrap_or(1);
        upsert_metadata(
            &transaction,
            "next_official_refresh_order",
            &(next_refresh_order + 1).to_string(),
        )?;
        transaction
            .commit()
            .context("failed to commit official refresh order transaction")?;
        Ok(next_refresh_order as u64)
    }

    pub(crate) fn acquire_official_refresh_claim(
        workspace_dir: &str,
        refresh_order: u64,
        owner_token: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshClaimStatus> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open official refresh claim transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        let next_executable =
            read_metadata_i64(&transaction, "next_executable_refresh_order")?.unwrap_or(1);
        if (refresh_order as i64) < next_executable {
            transaction
                .rollback()
                .context("failed to roll back completed official refresh claim transaction")?;
            return Ok(PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted);
        }
        if (refresh_order as i64) > next_executable {
            transaction
                .rollback()
                .context("failed to roll back waiting official refresh claim transaction")?;
            return Ok(PlanningAuthorityOfficialRefreshClaimStatus::Waiting);
        }

        let existing_owner = transaction
            .query_row(
                "SELECT owner_token FROM runtime_claims
                 WHERE claim_kind = 'official-refresh' AND scope_key = ?1",
                params![OFFICIAL_REFRESH_SCOPE_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("failed to read official refresh claim")?;
        if let Some(existing_owner) = existing_owner {
            if existing_owner == owner_token {
                transaction
                    .commit()
                    .context("failed to commit existing official refresh claim transaction")?;
                return Ok(PlanningAuthorityOfficialRefreshClaimStatus::Acquired);
            }
            transaction
                .rollback()
                .context("failed to roll back contended official refresh claim transaction")?;
            return Ok(PlanningAuthorityOfficialRefreshClaimStatus::Waiting);
        }

        transaction
            .execute(
                "INSERT INTO runtime_claims (claim_kind, scope_key, owner_token, claim_value, claimed_at)
                 VALUES ('official-refresh', ?1, ?2, ?3, ?4)",
                params![
                    OFFICIAL_REFRESH_SCOPE_KEY,
                    owner_token,
                    refresh_order.to_string(),
                    Utc::now().to_rfc3339()
                ],
            )
            .context("failed to acquire official refresh claim")?;
        transaction
            .commit()
            .context("failed to commit official refresh claim transaction")?;
        Ok(PlanningAuthorityOfficialRefreshClaimStatus::Acquired)
    }

    pub(crate) fn release_official_refresh_claim(
        workspace_dir: &str,
        refresh_order: u64,
        owner_token: &str,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open official refresh release transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        let deleted_rows = transaction
            .execute(
                "DELETE FROM runtime_claims
                 WHERE claim_kind = 'official-refresh' AND scope_key = ?1 AND owner_token = ?2 AND claim_value = ?3",
                params![OFFICIAL_REFRESH_SCOPE_KEY, owner_token, refresh_order.to_string()],
            )
            .context("failed to release official refresh claim")?;
        if deleted_rows > 0 {
            let next_executable =
                read_metadata_i64(&transaction, "next_executable_refresh_order")?.unwrap_or(1);
            if next_executable <= refresh_order as i64 {
                upsert_metadata(
                    &transaction,
                    "next_executable_refresh_order",
                    &(refresh_order + 1).to_string(),
                )?;
            }
        }
        transaction
            .commit()
            .context("failed to commit official refresh release transaction")?;
        Ok(())
    }

    pub(crate) fn try_acquire_distributor_queue_claim(
        workspace_dir: &str,
        queue_item_id: &str,
        owner_token: &str,
    ) -> Result<bool> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open distributor queue claim transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        let inserted_rows = transaction
            .execute(
                "INSERT OR IGNORE INTO runtime_claims
                 (claim_kind, scope_key, owner_token, claim_value, claimed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    DISTRIBUTOR_QUEUE_CLAIM_KIND,
                    queue_item_id,
                    owner_token,
                    queue_item_id,
                    Utc::now().to_rfc3339()
                ],
            )
            .context("failed to acquire distributor queue claim")?;
        transaction
            .commit()
            .context("failed to commit distributor queue claim transaction")?;
        Ok(inserted_rows > 0)
    }

    pub(crate) fn release_distributor_queue_claim(
        workspace_dir: &str,
        queue_item_id: &str,
        owner_token: &str,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open distributor queue release transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        transaction
            .execute(
                "DELETE FROM runtime_claims
                 WHERE claim_kind = ?1 AND scope_key = ?2 AND owner_token = ?3",
                params![DISTRIBUTOR_QUEUE_CLAIM_KIND, queue_item_id, owner_token],
            )
            .context("failed to release distributor queue claim")?;
        transaction
            .commit()
            .context("failed to commit distributor queue release transaction")?;
        Ok(())
    }

    fn resolve_authority_location_from_workspace(
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityLocation> {
        let workspace_root = canonicalize_best_effort(Path::new(workspace_dir));
        let canonical_repo_root =
            resolve_canonical_repo_root(workspace_dir).unwrap_or_else(|| workspace_root.clone());
        let runtime_dir = canonical_repo_root.join(RUNTIME_DIRECTORY);
        let authority_store_path = runtime_dir.join(AUTHORITY_STORE_FILE_NAME);

        Ok(PlanningAuthorityLocation {
            workspace_root: workspace_root.display().to_string(),
            canonical_repo_root: canonical_repo_root.display().to_string(),
            runtime_dir: runtime_dir.display().to_string(),
            authority_store_path: authority_store_path.display().to_string(),
        })
    }

    fn collect_authority_documents(canonical_repo_root: &Path) -> Result<BTreeMap<String, String>> {
        let mut documents = BTreeMap::new();
        for relative_path in ACTIVE_PLANNING_FILE_PATHS
            .iter()
            .copied()
            .chain(std::iter::once(PLAN_OFF_FILE_PATH))
        {
            let path = canonical_repo_root.join(relative_path);
            if !path.is_file() {
                continue;
            }
            let body = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            documents.insert(relative_path.to_string(), body);
        }

        collect_directory_documents(
            canonical_repo_root,
            PLANNING_DIRECTION_DOCS_DIRECTORY,
            &mut documents,
        )?;
        collect_directory_documents(
            canonical_repo_root,
            PLANNING_PROMPTS_DIRECTORY,
            &mut documents,
        )?;

        Ok(documents)
    }

    fn inspect_shadow_store_impl(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection> {
        let location = self.resolve_authority_location(workspace_dir)?;
        let canonical_repo_root = PathBuf::from(&location.canonical_repo_root);
        let authority_store_path = PathBuf::from(&location.authority_store_path);
        let source_documents = Self::collect_authority_documents(&canonical_repo_root)?;
        let had_store = authority_store_path.is_file();
        let previous_documents = load_shadow_documents(&authority_store_path)?;
        let parity_issues = compare_documents(&source_documents, &previous_documents);

        if let Some(parent) = authority_store_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let mut connection = Connection::open(&authority_store_path)
            .with_context(|| format!("failed to open {}", authority_store_path.display()))?;
        ensure_schema(&connection)?;
        store_shadow_documents(&mut connection, &location, &source_documents)?;

        let mirrored_documents = load_shadow_documents(&authority_store_path)?;
        let post_sync_issues = compare_documents(&source_documents, &mirrored_documents);
        if !post_sync_issues.is_empty() {
            let summary = post_sync_issues.join(", ");
            return Err(anyhow!(
                "shadow store parity check failed after sync: {summary}"
            ));
        }

        let sync_state = if !had_store {
            PlanningAuthorityShadowStoreSyncState::Bootstrapped
        } else if parity_issues.is_empty() {
            PlanningAuthorityShadowStoreSyncState::InSync
        } else {
            PlanningAuthorityShadowStoreSyncState::Resynced
        };

        Ok(PlanningAuthorityShadowStoreInspection {
            location,
            sync_state,
            mirrored_document_count: source_documents.len(),
            parity_issue_count: parity_issues.len(),
            parity_issue_examples: parity_issues.into_iter().take(3).collect(),
        })
    }
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
}

fn ensure_schema(connection: &Connection) -> Result<()> {
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

            CREATE TABLE IF NOT EXISTS runtime_claims (
                claim_kind TEXT NOT NULL,
                scope_key TEXT NOT NULL,
                owner_token TEXT NOT NULL,
                claim_value TEXT NOT NULL,
                claimed_at TEXT NOT NULL,
                PRIMARY KEY (claim_kind, scope_key)
            );
            "#,
        )
        .context("failed to initialize authority-store schema")?;
    Ok(())
}

fn store_shadow_documents(
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

fn upsert_authority_metadata(
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

fn upsert_metadata(transaction: &rusqlite::Transaction<'_>, key: &str, value: &str) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO authority_metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .with_context(|| format!("failed to update authority metadata `{key}`"))?;
    Ok(())
}

fn load_shadow_documents(authority_store_path: &Path) -> Result<BTreeMap<String, String>> {
    if !authority_store_path.is_file() {
        return Ok(BTreeMap::new());
    }

    let connection = Connection::open(authority_store_path)
        .with_context(|| format!("failed to open {}", authority_store_path.display()))?;
    if let Some(schema_version) = load_schema_version(&connection)? {
        let parsed = schema_version.parse::<i64>().ok();
        if parsed != Some(LEGACY_SHADOW_STORE_SCHEMA_VERSION)
            && parsed != Some(ACTIVE_DRAFT_SCHEMA_VERSION)
            && parsed != Some(AUTHORITY_STORE_SCHEMA_VERSION)
        {
            return Err(anyhow!(
                "unsupported authority-store schema version: {schema_version}"
            ));
        }
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

fn open_authority_connection(location: &PlanningAuthorityLocation) -> Result<Connection> {
    let authority_store_path = Path::new(&location.authority_store_path);
    if let Some(parent) = authority_store_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let connection = Connection::open(authority_store_path)
        .with_context(|| format!("failed to open {}", authority_store_path.display()))?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .context("failed to enable authority-store foreign keys")?;
    ensure_schema(&connection)?;
    Ok(connection)
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

fn upsert_draft_entry(transaction: &rusqlite::Transaction<'_>, draft_name: &str) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO staged_drafts (draft_name, updated_at) VALUES (?1, ?2)
             ON CONFLICT(draft_name) DO UPDATE SET updated_at = excluded.updated_at",
            params![draft_name, Utc::now().to_rfc3339()],
        )
        .with_context(|| format!("failed to update staged draft `{draft_name}`"))?;
    Ok(())
}

fn draft_directory_display_path(location: &PlanningAuthorityLocation, draft_name: &str) -> String {
    format!("{}#drafts/{draft_name}", location.authority_store_path)
}

fn draft_display_path(
    location: &PlanningAuthorityLocation,
    draft_name: &str,
    active_path: &str,
) -> String {
    let draft_relative_path = active_path
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches(".codex-exec-loop/planning/")
        .to_string();
    format!(
        "{}#drafts/{draft_name}/{draft_relative_path}",
        location.authority_store_path
    )
}

fn bootstrap_active_documents(
    connection: &mut Connection,
    location: &PlanningAuthorityLocation,
) -> Result<()> {
    let active_document_count = connection
        .query_row("SELECT COUNT(*) FROM active_documents", [], |row| {
            row.get::<_, i64>(0)
        })
        .context("failed to count active authority documents")?;
    if active_document_count > 0 {
        return Ok(());
    }

    let documents = SqlitePlanningAuthorityAdapter::collect_authority_documents(Path::new(
        &location.canonical_repo_root,
    ))?;
    let transaction = connection
        .transaction()
        .context("failed to open active document bootstrap transaction")?;
    upsert_authority_metadata(&transaction, location, "last_active_commit_at")?;
    for (relative_path, content) in &documents {
        transaction
            .execute(
                "INSERT INTO active_documents (relative_path, content) VALUES (?1, ?2)",
                params![relative_path, content],
            )
            .with_context(|| format!("failed to bootstrap active document `{relative_path}`"))?;
    }
    upsert_metadata(
        &transaction,
        "planning_revision",
        if documents.is_empty() { "0" } else { "1" },
    )?;
    transaction
        .commit()
        .context("failed to commit active document bootstrap transaction")?;
    Ok(())
}

fn apply_active_workspace_record(
    transaction: &rusqlite::Transaction<'_>,
    record: &PlanningWorkspaceLoadRecord,
) -> Result<bool> {
    let mut changed = false;
    changed |= set_active_document(
        transaction,
        DIRECTIONS_FILE_PATH,
        record.directions_toml.as_deref(),
    )?;
    changed |= set_active_document(
        transaction,
        TASK_LEDGER_FILE_PATH,
        record.task_ledger_json.as_deref(),
    )?;
    changed |= set_active_document(
        transaction,
        TASK_LEDGER_SCHEMA_FILE_PATH,
        record.task_ledger_schema_json.as_deref(),
    )?;
    changed |= set_active_document(
        transaction,
        QUEUE_SNAPSHOT_FILE_PATH,
        record.queue_snapshot_json.as_deref(),
    )?;
    changed |= set_active_document(
        transaction,
        RESULT_OUTPUT_FILE_PATH,
        record.result_output_markdown.as_deref(),
    )?;
    Ok(changed)
}

fn set_active_document(
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

fn remove_active_documents(
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

fn bump_planning_revision(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    let next_revision = read_metadata_i64(transaction, "planning_revision")?.unwrap_or(0) + 1;
    upsert_metadata(transaction, "planning_revision", &next_revision.to_string())?;
    Ok(())
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

fn export_active_workspace_record(
    location: &PlanningAuthorityLocation,
    record: &PlanningWorkspaceLoadRecord,
) -> Result<()> {
    export_active_document(
        location,
        DIRECTIONS_FILE_PATH,
        record.directions_toml.as_deref(),
    )?;
    export_active_document(
        location,
        TASK_LEDGER_FILE_PATH,
        record.task_ledger_json.as_deref(),
    )?;
    export_active_document(
        location,
        TASK_LEDGER_SCHEMA_FILE_PATH,
        record.task_ledger_schema_json.as_deref(),
    )?;
    export_active_document(
        location,
        QUEUE_SNAPSHOT_FILE_PATH,
        record.queue_snapshot_json.as_deref(),
    )?;
    export_active_document(
        location,
        RESULT_OUTPUT_FILE_PATH,
        record.result_output_markdown.as_deref(),
    )?;
    Ok(())
}

fn export_active_document(
    location: &PlanningAuthorityLocation,
    relative_path: &str,
    body: Option<&str>,
) -> Result<()> {
    let path = Path::new(&location.canonical_repo_root).join(relative_path);
    match body {
        Some(body) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::write(&path, body)
                .with_context(|| format!("failed to write {}", path.display()))?;
        }
        None => {
            if path.exists() {
                fs::remove_file(&path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
            }
        }
    }
    Ok(())
}

fn collect_directory_documents(
    workspace_root: &Path,
    relative_directory: &str,
    documents: &mut BTreeMap<String, String>,
) -> Result<()> {
    let directory = workspace_root.join(relative_directory);
    if !directory.is_dir() {
        return Ok(());
    }

    let mut stack = vec![directory];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current)
            .with_context(|| format!("failed to read {}", current.display()))?
        {
            let entry =
                entry.with_context(|| format!("failed to inspect {}", current.display()))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            let relative_path = path
                .strip_prefix(workspace_root)
                .with_context(|| format!("failed to strip {}", workspace_root.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            let body = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            documents.insert(relative_path, body);
        }
    }

    Ok(())
}

fn compare_documents(
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
            (Some(_), None) => {
                issues.push(format!("{relative_path}: missing from shadow store"));
            }
            (None, Some(_)) => {
                issues.push(format!(
                    "{relative_path}: shadow store contains stale content"
                ));
            }
            (Some(source), Some(mirrored)) if source != mirrored => {
                issues.push(format!("{relative_path}: content mismatch"));
            }
            _ => {}
        }
    }

    issues
}

fn resolve_canonical_repo_root(workspace_dir: &str) -> Option<PathBuf> {
    let common_dir = git_stdout(workspace_dir, &["rev-parse", "--git-common-dir"])?;
    let common_dir_path = absolutize_path(Path::new(workspace_dir), Path::new(&common_dir));
    let canonical_common_dir = canonicalize_best_effort(&common_dir_path);
    canonical_common_dir.parent().map(Path::to_path_buf)
}

fn git_stdout(workspace_dir: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(workspace_dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

fn absolutize_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    base.join(path)
}

fn canonicalize_best_effort(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;

    use super::SqlitePlanningAuthorityAdapter;
    use crate::application::port::outbound::planning_authority_port::{
        PlanningAuthorityOfficialRefreshClaimStatus, PlanningAuthorityPort,
    };
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
    use crate::application::service::planning_contract::{
        DIRECTIONS_FILE_PATH, TASK_LEDGER_FILE_PATH,
    };
    use crate::domain::planning::PlanningAuthorityShadowStoreSyncState;

    struct TempGitRepo {
        root: PathBuf,
        repo_root: PathBuf,
        worktree_root: PathBuf,
    }

    impl TempGitRepo {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("{label}-{unique}"));
            let repo_root = root.join("repo");
            let worktree_root = root.join("worktrees").join("linked");
            fs::create_dir_all(&repo_root).expect("temp repo root should exist");
            run_git(&repo_root, &["init", "-q"]);
            run_git(&repo_root, &["config", "user.name", "RefinedStone"]);
            run_git(
                &repo_root,
                &["config", "user.email", "chem.en.9273@gmail.com"],
            );
            fs::write(repo_root.join("README.md"), "seed\n").expect("seed file should write");
            run_git(&repo_root, &["add", "README.md"]);
            run_git(&repo_root, &["commit", "-qm", "init"]);
            fs::create_dir_all(
                worktree_root
                    .parent()
                    .expect("worktree parent should exist"),
            )
            .expect("worktree parent should exist");
            run_git(
                &repo_root,
                &[
                    "worktree",
                    "add",
                    "-b",
                    "feature/worktree",
                    worktree_root.to_str().expect("valid worktree path"),
                ],
            );

            Self {
                root,
                repo_root,
                worktree_root,
            }
        }

        fn write_repo_file(&self, relative_path: &str, body: &str) {
            let path = self.repo_root.join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("parent directory should exist");
            }
            fs::write(path, body).expect("repo file should write");
        }
    }

    impl Drop for TempGitRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn run_git(repo_root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(repo_root)
            .args(args)
            .status()
            .expect("git command should spawn");
        assert!(
            status.success(),
            "git command should succeed: git {}",
            args.join(" ")
        );
    }

    #[test]
    fn resolve_authority_location_uses_canonical_repo_root_for_linked_worktree() {
        let repo = TempGitRepo::new("authority-location");
        let adapter = SqlitePlanningAuthorityAdapter::new();

        let location = adapter
            .resolve_authority_location(repo.worktree_root.to_str().expect("valid path"))
            .expect("authority location should resolve");

        assert_eq!(
            location.canonical_repo_root,
            fs::canonicalize(&repo.repo_root)
                .expect("repo root should canonicalize")
                .display()
                .to_string()
        );
        assert_eq!(
            location.workspace_root,
            fs::canonicalize(&repo.worktree_root)
                .expect("worktree root should canonicalize")
                .display()
                .to_string()
        );
        assert!(location.runtime_dir.ends_with(".codex-exec-loop/runtime"));
        assert!(
            location
                .authority_store_path
                .ends_with(".codex-exec-loop/runtime/planning-authority.db")
        );
    }

    #[test]
    fn inspect_shadow_store_bootstraps_from_canonical_repo_root() {
        let repo = TempGitRepo::new("shadow-bootstrap");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        repo.write_repo_file(".codex-exec-loop/planning/directions.toml", "version = 1\n");
        repo.write_repo_file(
            ".codex-exec-loop/planning/task-ledger.json",
            "{\"version\":1,\"tasks\":[]}\n",
        );
        repo.write_repo_file(
            ".codex-exec-loop/planning/prompts/queue-idle-review.md",
            "# review\n",
        );

        let inspection = adapter
            .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
            .expect("shadow store should inspect");

        assert_eq!(
            inspection.sync_state,
            PlanningAuthorityShadowStoreSyncState::Bootstrapped
        );
        assert_eq!(inspection.mirrored_document_count, 3);
        let connection = Connection::open(&inspection.location.authority_store_path)
            .expect("shadow store should open");
        let content = connection
            .query_row(
                "SELECT content FROM shadow_documents WHERE relative_path = ?1",
                [".codex-exec-loop/planning/directions.toml"],
                |row| row.get::<_, String>(0),
            )
            .expect("directions content should exist");
        assert_eq!(content, "version = 1\n");
    }

    #[test]
    fn inspect_shadow_store_reports_resynced_when_source_changes() {
        let repo = TempGitRepo::new("shadow-resync");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        repo.write_repo_file(".codex-exec-loop/planning/directions.toml", "version = 1\n");

        adapter
            .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
            .expect("initial shadow store sync should succeed");

        repo.write_repo_file(".codex-exec-loop/planning/directions.toml", "version = 2\n");

        let inspection = adapter
            .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
            .expect("shadow store resync should succeed");

        assert_eq!(
            inspection.sync_state,
            PlanningAuthorityShadowStoreSyncState::Resynced
        );
        assert_eq!(inspection.parity_issue_count, 1);
        assert!(
            inspection
                .parity_issue_examples
                .iter()
                .any(|issue| issue.contains("directions.toml"))
        );
    }

    #[test]
    fn inspect_shadow_store_upgrades_legacy_schema_version_one_store() {
        let repo = TempGitRepo::new("shadow-upgrade-v1");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        repo.write_repo_file(".codex-exec-loop/planning/directions.toml", "version = 1\n");
        let runtime_dir = repo.repo_root.join(".codex-exec-loop/runtime");
        fs::create_dir_all(&runtime_dir).expect("runtime directory should exist");
        let authority_store_path = runtime_dir.join("planning-authority.db");
        let connection =
            Connection::open(&authority_store_path).expect("legacy authority store should open");
        connection
            .execute_batch(
                r#"
                CREATE TABLE authority_metadata (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                CREATE TABLE shadow_documents (
                    relative_path TEXT PRIMARY KEY,
                    content TEXT NOT NULL
                );
                "#,
            )
            .expect("legacy shadow-store schema should initialize");
        connection
            .execute(
                "INSERT INTO authority_metadata (key, value) VALUES ('schema_version', '1')",
                [],
            )
            .expect("legacy schema version should insert");
        connection
            .execute(
                "INSERT INTO shadow_documents (relative_path, content) VALUES (?1, ?2)",
                [".codex-exec-loop/planning/directions.toml", "version = 0\n"],
            )
            .expect("legacy shadow document should insert");

        let inspection = adapter
            .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
            .expect("legacy store should upgrade during inspection");

        assert_eq!(
            inspection.sync_state,
            PlanningAuthorityShadowStoreSyncState::Resynced
        );
        let connection =
            Connection::open(&authority_store_path).expect("upgraded authority store should open");
        let schema_version = connection
            .query_row(
                "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("schema version should load");
        assert_eq!(
            schema_version,
            super::AUTHORITY_STORE_SCHEMA_VERSION.to_string()
        );
        let draft_table = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'staged_drafts'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("staged_drafts table should exist after upgrade");
        assert_eq!(draft_table, "staged_drafts");
    }

    #[test]
    fn active_commit_updates_repo_scoped_documents_for_linked_worktree() {
        let repo = TempGitRepo::new("authority-active-commit");

        SqlitePlanningAuthorityAdapter::commit_active_workspace_files(
            repo.worktree_root.to_str().expect("valid worktree path"),
            &PlanningWorkspaceLoadRecord {
                directions_toml: Some("version = 4\n".to_string()),
                task_ledger_json: Some("{\"version\":1,\"tasks\":[]}\n".to_string()),
                task_ledger_schema_json: Some("{\"type\":\"object\"}\n".to_string()),
                queue_snapshot_json: Some("{\"next_task\":null}\n".to_string()),
                result_output_markdown: Some("# result\n".to_string()),
            },
        )
        .expect("active commit should succeed");

        assert_eq!(
            fs::read_to_string(repo.repo_root.join(DIRECTIONS_FILE_PATH))
                .expect("repo directions should exist"),
            "version = 4\n"
        );
        let location = SqlitePlanningAuthorityAdapter::new()
            .resolve_authority_location(repo.worktree_root.to_str().expect("valid worktree path"))
            .expect("authority location should resolve");
        let connection =
            Connection::open(&location.authority_store_path).expect("authority store should open");
        let stored_task_ledger = connection
            .query_row(
                "SELECT content FROM active_documents WHERE relative_path = ?1",
                [TASK_LEDGER_FILE_PATH],
                |row| row.get::<_, String>(0),
            )
            .expect("active task ledger should be stored");
        assert_eq!(stored_task_ledger, "{\"version\":1,\"tasks\":[]}\n");
    }

    #[test]
    fn official_refresh_claims_enforce_reserved_execution_order() {
        let repo = TempGitRepo::new("authority-official-claims");
        let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");

        let first =
            SqlitePlanningAuthorityAdapter::reserve_next_official_refresh_order(workspace_dir)
                .expect("first order should reserve");
        let second =
            SqlitePlanningAuthorityAdapter::reserve_next_official_refresh_order(workspace_dir)
                .expect("second order should reserve");

        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(
            SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
                workspace_dir,
                second,
                "owner-2",
            )
            .expect("later order claim should inspect"),
            PlanningAuthorityOfficialRefreshClaimStatus::Waiting
        );
        assert_eq!(
            SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
                workspace_dir,
                first,
                "owner-1",
            )
            .expect("first order claim should acquire"),
            PlanningAuthorityOfficialRefreshClaimStatus::Acquired
        );
        assert_eq!(
            SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
                workspace_dir,
                first,
                "other-owner",
            )
            .expect("contended first order claim should inspect"),
            PlanningAuthorityOfficialRefreshClaimStatus::Waiting
        );

        SqlitePlanningAuthorityAdapter::release_official_refresh_claim(
            workspace_dir,
            first,
            "owner-1",
        )
        .expect("first order claim should release");

        assert_eq!(
            SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
                workspace_dir,
                first,
                "owner-1",
            )
            .expect("completed first order claim should inspect"),
            PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted
        );
        assert_eq!(
            SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
                workspace_dir,
                second,
                "owner-2",
            )
            .expect("second order claim should acquire after release"),
            PlanningAuthorityOfficialRefreshClaimStatus::Acquired
        );
    }

    #[test]
    fn distributor_queue_claims_are_unique_until_release() {
        let repo = TempGitRepo::new("authority-distributor-claims");
        let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");

        assert!(
            SqlitePlanningAuthorityAdapter::try_acquire_distributor_queue_claim(
                workspace_dir,
                "queue-item-1",
                "owner-1",
            )
            .expect("first queue claim should succeed")
        );
        assert!(
            !SqlitePlanningAuthorityAdapter::try_acquire_distributor_queue_claim(
                workspace_dir,
                "queue-item-1",
                "owner-2",
            )
            .expect("duplicate queue claim should be rejected")
        );

        SqlitePlanningAuthorityAdapter::release_distributor_queue_claim(
            workspace_dir,
            "queue-item-1",
            "owner-1",
        )
        .expect("queue claim should release");

        assert!(
            SqlitePlanningAuthorityAdapter::try_acquire_distributor_queue_claim(
                workspace_dir,
                "queue-item-1",
                "owner-2",
            )
            .expect("released queue claim should be reacquired")
        );
    }
}

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
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
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
};
use crate::domain::planning::{
    PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection,
    PlanningAuthorityShadowStoreSyncState,
};

const RUNTIME_DIRECTORY: &str = ".codex-exec-loop/runtime";
const AUTHORITY_STORE_FILE_NAME: &str = "planning-authority.db";
const LEGACY_SHADOW_STORE_SCHEMA_VERSION: i64 = 1;
const ACTIVE_DRAFT_SCHEMA_VERSION: i64 = 2;
const ACTIVE_MUTATION_SCHEMA_VERSION: i64 = 3;
const AUTHORITY_STORE_SCHEMA_VERSION: i64 = 4;
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

    pub(crate) fn load_active_workspace_files(
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        bootstrap_active_documents(&mut connection, &location)?;
        load_active_workspace_record(&connection)
    }

    pub(crate) fn load_active_planning_file(
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        bootstrap_active_documents(&mut connection, &location)?;
        load_active_document(&connection, relative_path)
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

    pub(crate) fn load_runtime_projections(
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        bootstrap_runtime_projections(&mut connection, &location)?;
        load_runtime_projection_snapshot(&connection)
    }

    pub(crate) fn upsert_runtime_slot_lease(
        workspace_dir: &str,
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        bootstrap_runtime_projections(&mut connection, &location)?;

        let payload_json = serde_json::to_string(lease)
            .context("failed to serialize runtime slot lease projection")?;
        let transaction = connection
            .transaction()
            .context("failed to open runtime slot lease transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        transaction
            .execute(
                "INSERT INTO runtime_slot_leases (slot_id, updated_at, content)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(slot_id) DO UPDATE
                 SET updated_at = excluded.updated_at,
                     content = excluded.content",
                params![lease.slot_id, Utc::now().to_rfc3339(), payload_json],
            )
            .with_context(|| format!("failed to persist runtime slot lease `{}`", lease.slot_id))?;
        transaction
            .execute(
                "DELETE FROM runtime_invalid_slot_leases WHERE slot_id = ?1",
                params![lease.slot_id],
            )
            .with_context(|| {
                format!(
                    "failed to clear invalid runtime slot lease `{}`",
                    lease.slot_id
                )
            })?;
        append_runtime_event(
            &transaction,
            "slot_lease_upsert",
            "slot_lease",
            &lease.slot_id,
            &format!(
                "runtime slot lease stored / slot: {} / state: {}",
                lease.slot_id,
                lease.state.label()
            ),
            &serde_json::to_string(lease)
                .context("failed to serialize runtime slot lease event payload")?,
        )?;
        transaction
            .commit()
            .context("failed to commit runtime slot lease transaction")?;

        mirror_runtime_slot_lease(&location, lease)?;
        Ok(())
    }

    pub(crate) fn remove_runtime_slot_lease(workspace_dir: &str, slot_id: &str) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        bootstrap_runtime_projections(&mut connection, &location)?;

        let transaction = connection
            .transaction()
            .context("failed to open runtime slot lease removal transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        let deleted_rows = transaction
            .execute(
                "DELETE FROM runtime_slot_leases WHERE slot_id = ?1",
                params![slot_id],
            )
            .with_context(|| format!("failed to delete runtime slot lease `{slot_id}`"))?;
        transaction
            .execute(
                "DELETE FROM runtime_invalid_slot_leases WHERE slot_id = ?1",
                params![slot_id],
            )
            .with_context(|| format!("failed to clear invalid runtime slot lease `{slot_id}`"))?;
        if deleted_rows > 0 {
            append_runtime_event(
                &transaction,
                "slot_lease_removed",
                "slot_lease",
                slot_id,
                &format!("runtime slot lease removed / slot: {slot_id}"),
                "{}",
            )?;
        }
        transaction
            .commit()
            .context("failed to commit runtime slot lease removal transaction")?;

        remove_runtime_slot_lease_mirror(&location, slot_id)?;
        Ok(())
    }

    pub(crate) fn upsert_runtime_session_detail(
        workspace_dir: &str,
        detail: &ParallelModeAgentSessionDetailSnapshot,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        bootstrap_runtime_projections(&mut connection, &location)?;

        let payload_json = serde_json::to_string(detail)
            .context("failed to serialize runtime session detail projection")?;
        let transaction = connection
            .transaction()
            .context("failed to open runtime session detail transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        transaction
            .execute(
                "INSERT INTO runtime_session_details (session_key, slot_id, updated_at, content)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(session_key) DO UPDATE
                 SET slot_id = excluded.slot_id,
                     updated_at = excluded.updated_at,
                     content = excluded.content",
                params![
                    detail.session_key,
                    detail.slot_id,
                    detail.updated_at,
                    payload_json
                ],
            )
            .with_context(|| {
                format!(
                    "failed to persist runtime session detail `{}`",
                    detail.session_key
                )
            })?;
        append_runtime_event(
            &transaction,
            "session_detail_upsert",
            "session_detail",
            &detail.session_key,
            &format!(
                "runtime session detail stored / session: {} / state: {}",
                detail.session_key, detail.state_label
            ),
            &serde_json::to_string(detail)
                .context("failed to serialize runtime session detail event payload")?,
        )?;
        transaction
            .commit()
            .context("failed to commit runtime session detail transaction")?;

        mirror_runtime_session_detail(&location, detail)?;
        Ok(())
    }

    pub(crate) fn upsert_runtime_distributor_queue_record(
        workspace_dir: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        bootstrap_runtime_projections(&mut connection, &location)?;

        let payload_json = serde_json::to_string(record)
            .context("failed to serialize runtime distributor queue projection")?;
        let transaction = connection
            .transaction()
            .context("failed to open runtime distributor queue transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        transaction
            .execute(
                "INSERT INTO runtime_distributor_queue
                 (queue_item_id, session_key, queue_state, enqueued_at, updated_at, content)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(queue_item_id) DO UPDATE
                 SET session_key = excluded.session_key,
                     queue_state = excluded.queue_state,
                     enqueued_at = excluded.enqueued_at,
                     updated_at = excluded.updated_at,
                     content = excluded.content",
                params![
                    record.queue_item_id,
                    record.session_key,
                    record.queue_state.label(),
                    record.enqueued_at,
                    record.updated_at,
                    payload_json
                ],
            )
            .with_context(|| {
                format!(
                    "failed to persist runtime distributor queue record `{}`",
                    record.queue_item_id
                )
            })?;
        append_runtime_event(
            &transaction,
            "distributor_queue_upsert",
            "distributor_queue",
            &record.queue_item_id,
            &format!(
                "runtime distributor queue stored / item: {} / state: {}",
                record.queue_item_id,
                record.queue_state.label()
            ),
            &serde_json::to_string(record)
                .context("failed to serialize runtime distributor queue event payload")?,
        )?;
        transaction
            .commit()
            .context("failed to commit runtime distributor queue transaction")?;

        mirror_runtime_distributor_queue_record(&location, record)?;
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
        let had_store = authority_store_path.is_file();
        let previous_documents = load_shadow_documents(&authority_store_path)?;
        let mut connection = open_authority_connection(&location)?;
        bootstrap_active_documents(&mut connection, &location)?;
        let source_documents = load_active_documents(&connection)?;
        let shadow_parity_issues = compare_shadow_documents(&source_documents, &previous_documents);
        let exported_documents = Self::collect_authority_documents(&canonical_repo_root)?;
        let export_parity_issues =
            compare_exported_documents(&source_documents, &exported_documents);
        if !export_parity_issues.is_empty() {
            sync_exported_authority_documents(&location, &source_documents, &exported_documents)?;
        }
        store_shadow_documents(&mut connection, &location, &source_documents)?;

        let mirrored_documents = load_shadow_documents(&authority_store_path)?;
        let post_sync_issues = compare_shadow_documents(&source_documents, &mirrored_documents);
        if !post_sync_issues.is_empty() {
            let summary = post_sync_issues.join(", ");
            return Err(anyhow!(
                "shadow store parity check failed after sync: {summary}"
            ));
        }

        let sync_state = if !had_store {
            PlanningAuthorityShadowStoreSyncState::Bootstrapped
        } else if shadow_parity_issues.is_empty() && export_parity_issues.is_empty() {
            PlanningAuthorityShadowStoreSyncState::InSync
        } else {
            PlanningAuthorityShadowStoreSyncState::Resynced
        };
        let parity_issue_examples = shadow_parity_issues
            .iter()
            .chain(export_parity_issues.iter())
            .take(3)
            .cloned()
            .collect::<Vec<_>>();

        Ok(PlanningAuthorityShadowStoreInspection {
            location,
            sync_state,
            mirrored_document_count: source_documents.len(),
            parity_issue_count: shadow_parity_issues.len() + export_parity_issues.len(),
            parity_issue_examples,
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
            && parsed != Some(ACTIVE_MUTATION_SCHEMA_VERSION)
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

fn load_active_documents(connection: &Connection) -> Result<BTreeMap<String, String>> {
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

fn load_active_workspace_record(connection: &Connection) -> Result<PlanningWorkspaceLoadRecord> {
    let documents = load_active_documents(connection)?;
    Ok(PlanningWorkspaceLoadRecord {
        directions_toml: documents.get(DIRECTIONS_FILE_PATH).cloned(),
        task_ledger_json: documents.get(TASK_LEDGER_FILE_PATH).cloned(),
        task_ledger_schema_json: documents.get(TASK_LEDGER_SCHEMA_FILE_PATH).cloned(),
        queue_snapshot_json: documents.get(QUEUE_SNAPSHOT_FILE_PATH).cloned(),
        result_output_markdown: documents.get(RESULT_OUTPUT_FILE_PATH).cloned(),
    })
}

fn load_active_document(connection: &Connection, relative_path: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT content FROM active_documents WHERE relative_path = ?1",
            params![relative_path],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("failed to read active authority document `{relative_path}`"))
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

fn bootstrap_runtime_projections(
    connection: &mut Connection,
    location: &PlanningAuthorityLocation,
) -> Result<()> {
    let runtime_projection_count = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM runtime_slot_leases) +
                (SELECT COUNT(*) FROM runtime_invalid_slot_leases) +
                (SELECT COUNT(*) FROM runtime_session_details) +
                (SELECT COUNT(*) FROM runtime_distributor_queue)",
            [],
            |row| row.get::<_, i64>(0),
        )
        .context("failed to count runtime authority projections")?;
    if runtime_projection_count > 0 {
        return Ok(());
    }

    let pool_root = legacy_runtime_pool_root(location);
    let (slot_leases, invalid_slot_leases) = read_legacy_slot_leases(&pool_root);
    let session_details = read_legacy_session_details(&pool_root);
    let distributor_queue_records = read_legacy_distributor_queue_records(&pool_root);
    if slot_leases.is_empty()
        && invalid_slot_leases.is_empty()
        && session_details.is_empty()
        && distributor_queue_records.is_empty()
    {
        return Ok(());
    }

    let transaction = connection
        .transaction()
        .context("failed to open runtime projection bootstrap transaction")?;
    upsert_authority_metadata(&transaction, location, "last_runtime_projection_at")?;
    for lease in slot_leases.values() {
        transaction
            .execute(
                "INSERT INTO runtime_slot_leases (slot_id, updated_at, content)
                 VALUES (?1, ?2, ?3)",
                params![
                    lease.slot_id,
                    lease
                        .running_started_at
                        .as_deref()
                        .unwrap_or(lease.leased_at.as_str()),
                    serde_json::to_string(lease)
                        .context("failed to serialize bootstrapped slot lease")?,
                ],
            )
            .with_context(|| {
                format!("failed to bootstrap runtime slot lease `{}`", lease.slot_id)
            })?;
    }
    for slot_id in &invalid_slot_leases {
        transaction
            .execute(
                "INSERT INTO runtime_invalid_slot_leases (slot_id, detected_at)
                 VALUES (?1, ?2)",
                params![slot_id, Utc::now().to_rfc3339()],
            )
            .with_context(|| {
                format!("failed to bootstrap invalid runtime slot lease `{slot_id}`")
            })?;
    }
    for detail in &session_details {
        transaction
            .execute(
                "INSERT INTO runtime_session_details (session_key, slot_id, updated_at, content)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    detail.session_key,
                    detail.slot_id,
                    detail.updated_at,
                    serde_json::to_string(detail)
                        .context("failed to serialize bootstrapped session detail")?,
                ],
            )
            .with_context(|| {
                format!(
                    "failed to bootstrap runtime session detail `{}`",
                    detail.session_key
                )
            })?;
    }
    for record in &distributor_queue_records {
        transaction
            .execute(
                "INSERT INTO runtime_distributor_queue
                 (queue_item_id, session_key, queue_state, enqueued_at, updated_at, content)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    record.queue_item_id,
                    record.session_key,
                    record.queue_state.label(),
                    record.enqueued_at,
                    record.updated_at,
                    serde_json::to_string(record)
                        .context("failed to serialize bootstrapped distributor queue record")?,
                ],
            )
            .with_context(|| {
                format!(
                    "failed to bootstrap runtime distributor queue record `{}`",
                    record.queue_item_id
                )
            })?;
    }
    upsert_metadata(&transaction, "runtime_event_sequence", "0")?;
    transaction
        .commit()
        .context("failed to commit runtime projection bootstrap transaction")?;
    Ok(())
}

fn load_runtime_projection_snapshot(
    connection: &Connection,
) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
    let mut slot_leases = BTreeMap::new();
    let mut invalid_slot_leases = BTreeSet::new();
    let mut session_details = Vec::new();
    let mut distributor_queue_records = Vec::new();

    let mut slot_statement = connection
        .prepare("SELECT slot_id, content FROM runtime_slot_leases ORDER BY slot_id")
        .context("failed to read runtime slot leases")?;
    let slot_rows = slot_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate runtime slot leases")?;
    for row in slot_rows {
        let (slot_id, content) = row.context("failed to decode runtime slot lease row")?;
        let lease = serde_json::from_str::<ParallelModeSlotLeaseSnapshot>(&content)
            .with_context(|| format!("failed to deserialize runtime slot lease `{slot_id}`"))?;
        slot_leases.insert(slot_id, lease);
    }

    let mut invalid_slot_statement = connection
        .prepare("SELECT slot_id FROM runtime_invalid_slot_leases ORDER BY slot_id")
        .context("failed to read invalid runtime slot leases")?;
    let invalid_slot_rows = invalid_slot_statement
        .query_map([], |row| row.get::<_, String>(0))
        .context("failed to iterate invalid runtime slot leases")?;
    for row in invalid_slot_rows {
        invalid_slot_leases.insert(row.context("failed to decode invalid runtime slot row")?);
    }

    let mut session_statement = connection
        .prepare(
            "SELECT session_key, content
             FROM runtime_session_details
             ORDER BY updated_at DESC, session_key ASC",
        )
        .context("failed to read runtime session details")?;
    let session_rows = session_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate runtime session details")?;
    for row in session_rows {
        let (session_key, content) = row.context("failed to decode runtime session detail row")?;
        session_details.push(
            serde_json::from_str::<ParallelModeAgentSessionDetailSnapshot>(&content).with_context(
                || format!("failed to deserialize runtime session detail `{session_key}`"),
            )?,
        );
    }

    let mut queue_statement = connection
        .prepare(
            "SELECT queue_item_id, content
             FROM runtime_distributor_queue
             ORDER BY enqueued_at ASC, queue_item_id ASC",
        )
        .context("failed to read runtime distributor queue records")?;
    let queue_rows = queue_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate runtime distributor queue records")?;
    for row in queue_rows {
        let (queue_item_id, content) =
            row.context("failed to decode runtime distributor queue row")?;
        distributor_queue_records.push(
            serde_json::from_str::<PlanningAuthorityDistributorQueueRecord>(&content)
                .with_context(|| {
                    format!(
                        "failed to deserialize runtime distributor queue record `{queue_item_id}`"
                    )
                })?,
        );
    }

    Ok(PlanningAuthorityRuntimeProjectionSnapshot {
        slot_leases,
        invalid_slot_leases,
        session_details,
        distributor_queue_records,
    })
}

fn append_runtime_event(
    transaction: &rusqlite::Transaction<'_>,
    event_kind: &str,
    projection_kind: &str,
    projection_key: &str,
    summary: &str,
    payload_json: &str,
) -> Result<()> {
    let sequence = read_metadata_i64(transaction, "runtime_event_sequence")?.unwrap_or(0) + 1;
    let observed_planning_revision =
        read_metadata_i64(transaction, "planning_revision")?.unwrap_or(0);
    upsert_metadata(transaction, "runtime_event_sequence", &sequence.to_string())?;
    transaction
        .execute(
            "INSERT INTO runtime_events
             (sequence, event_kind, projection_kind, projection_key, observed_planning_revision, summary, payload_json, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                sequence,
                event_kind,
                projection_kind,
                projection_key,
                observed_planning_revision,
                summary,
                payload_json,
                Utc::now().to_rfc3339()
            ],
        )
        .with_context(|| {
            format!(
                "failed to append runtime event `{event_kind}` for `{projection_kind}:{projection_key}`"
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

fn legacy_runtime_pool_root(location: &PlanningAuthorityLocation) -> PathBuf {
    let canonical_repo_root = Path::new(&location.canonical_repo_root);
    let repo_name = canonical_repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    let parent_dir = canonical_repo_root.parent().unwrap_or(canonical_repo_root);
    parent_dir
        .join(format!("{repo_name}-worktrees"))
        .join(stable_short_hash(&canonical_repo_root.to_string_lossy()))
        .join("akra-pool")
}

fn stable_short_hash(value: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..12].to_string()
}

fn runtime_slot_leases_root(location: &PlanningAuthorityLocation) -> PathBuf {
    legacy_runtime_pool_root(location).join(".leases")
}

fn runtime_slot_lease_path(location: &PlanningAuthorityLocation, slot_id: &str) -> PathBuf {
    runtime_slot_leases_root(location).join(format!("{slot_id}.json"))
}

fn runtime_session_history_dir(location: &PlanningAuthorityLocation) -> PathBuf {
    legacy_runtime_pool_root(location).join(".agent-sessions")
}

fn runtime_session_detail_path(location: &PlanningAuthorityLocation, session_key: &str) -> PathBuf {
    let filename = session_key
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    runtime_session_history_dir(location).join(format!("{filename}.json"))
}

fn runtime_distributor_queue_root(location: &PlanningAuthorityLocation) -> PathBuf {
    legacy_runtime_pool_root(location).join(".distributor-queue")
}

fn runtime_distributor_queue_path(
    location: &PlanningAuthorityLocation,
    queue_item_id: &str,
) -> PathBuf {
    runtime_distributor_queue_root(location).join(format!("{queue_item_id}.json"))
}

fn mirror_runtime_slot_lease(
    location: &PlanningAuthorityLocation,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<()> {
    let leases_root = runtime_slot_leases_root(location);
    fs::create_dir_all(&leases_root)
        .with_context(|| format!("failed to create {}", leases_root.display()))?;
    fs::write(
        runtime_slot_lease_path(location, &lease.slot_id),
        serde_json::to_string_pretty(lease)
            .context("failed to serialize mirrored runtime slot lease")?,
    )
    .with_context(|| format!("failed to mirror runtime slot lease `{}`", lease.slot_id))?;
    Ok(())
}

fn remove_runtime_slot_lease_mirror(
    location: &PlanningAuthorityLocation,
    slot_id: &str,
) -> Result<()> {
    let path = runtime_slot_lease_path(location, slot_id);
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

fn mirror_runtime_session_detail(
    location: &PlanningAuthorityLocation,
    detail: &ParallelModeAgentSessionDetailSnapshot,
) -> Result<()> {
    let history_dir = runtime_session_history_dir(location);
    fs::create_dir_all(&history_dir)
        .with_context(|| format!("failed to create {}", history_dir.display()))?;
    fs::write(
        runtime_session_detail_path(location, &detail.session_key),
        serde_json::to_string_pretty(detail)
            .context("failed to serialize mirrored runtime session detail")?,
    )
    .with_context(|| {
        format!(
            "failed to mirror runtime session detail `{}`",
            detail.session_key
        )
    })?;
    Ok(())
}

fn mirror_runtime_distributor_queue_record(
    location: &PlanningAuthorityLocation,
    record: &PlanningAuthorityDistributorQueueRecord,
) -> Result<()> {
    let queue_root = runtime_distributor_queue_root(location);
    fs::create_dir_all(&queue_root)
        .with_context(|| format!("failed to create {}", queue_root.display()))?;
    fs::write(
        runtime_distributor_queue_path(location, &record.queue_item_id),
        serde_json::to_string_pretty(record)
            .context("failed to serialize mirrored distributor queue record")?,
    )
    .with_context(|| {
        format!(
            "failed to mirror runtime distributor queue record `{}`",
            record.queue_item_id
        )
    })?;
    Ok(())
}

fn read_legacy_slot_leases(
    pool_root: &Path,
) -> (
    BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
    BTreeSet<String>,
) {
    let leases_root = pool_root.join(".leases");
    let Ok(entries) = fs::read_dir(&leases_root) else {
        return (BTreeMap::new(), BTreeSet::new());
    };

    let mut slot_leases = BTreeMap::new();
    let mut invalid_slot_leases = BTreeSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let slot_id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_string)
            .unwrap_or_default();
        if slot_id.is_empty() {
            continue;
        }

        let Ok(contents) = fs::read_to_string(&path) else {
            invalid_slot_leases.insert(slot_id);
            continue;
        };
        let Ok(lease) = serde_json::from_str::<ParallelModeSlotLeaseSnapshot>(&contents) else {
            invalid_slot_leases.insert(slot_id);
            continue;
        };
        if lease.slot_id != slot_id {
            invalid_slot_leases.insert(slot_id);
            continue;
        }
        slot_leases.insert(slot_id, lease);
    }

    (slot_leases, invalid_slot_leases)
}

fn read_legacy_session_details(pool_root: &Path) -> Vec<ParallelModeAgentSessionDetailSnapshot> {
    let history_dir = pool_root.join(".agent-sessions");
    let Ok(entries) = fs::read_dir(&history_dir) else {
        return Vec::new();
    };

    let mut records = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("json"))
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|content| {
            serde_json::from_str::<ParallelModeAgentSessionDetailSnapshot>(&content).ok()
        })
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.session_key.cmp(&right.session_key))
    });
    records
}

fn read_legacy_distributor_queue_records(
    pool_root: &Path,
) -> Vec<PlanningAuthorityDistributorQueueRecord> {
    let queue_root = pool_root.join(".distributor-queue");
    let Ok(entries) = fs::read_dir(&queue_root) else {
        return Vec::new();
    };

    let mut records = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|content| {
            serde_json::from_str::<PlanningAuthorityDistributorQueueRecord>(&content).ok()
        })
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        left.enqueued_at
            .cmp(&right.enqueued_at)
            .then_with(|| left.queue_item_id.cmp(&right.queue_item_id))
    });
    records
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

fn sync_exported_authority_documents(
    location: &PlanningAuthorityLocation,
    source_documents: &BTreeMap<String, String>,
    exported_documents: &BTreeMap<String, String>,
) -> Result<()> {
    for (relative_path, content) in source_documents {
        export_active_document(location, relative_path, Some(content))?;
    }
    for relative_path in exported_documents.keys() {
        if !source_documents.contains_key(relative_path) {
            export_active_document(location, relative_path, None)?;
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

fn compare_exported_documents(
    source_documents: &BTreeMap<String, String>,
    exported_documents: &BTreeMap<String, String>,
) -> Vec<String> {
    let document_paths = source_documents
        .keys()
        .chain(exported_documents.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut issues = Vec::new();
    for relative_path in document_paths {
        match (
            source_documents.get(&relative_path),
            exported_documents.get(&relative_path),
        ) {
            (Some(_), None) => {
                issues.push(format!("{relative_path}: missing tracked export"));
            }
            (None, Some(_)) => {
                issues.push(format!(
                    "{relative_path}: tracked export contains stale content"
                ));
            }
            (Some(source), Some(exported)) if source != exported => {
                issues.push(format!("{relative_path}: tracked export mismatch"));
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
    fn inspect_shadow_store_restores_diverged_tracked_exports_from_active_store() {
        let repo = TempGitRepo::new("shadow-resync");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        SqlitePlanningAuthorityAdapter::commit_active_workspace_files(
            repo.worktree_root.to_str().expect("valid path"),
            &PlanningWorkspaceLoadRecord {
                directions_toml: Some("version = 1\n".to_string()),
                task_ledger_json: None,
                task_ledger_schema_json: None,
                queue_snapshot_json: None,
                result_output_markdown: None,
            },
        )
        .expect("active planning should seed the authority store");
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
                .any(|issue| issue.contains("tracked export"))
        );
        assert_eq!(
            fs::read_to_string(repo.repo_root.join(DIRECTIONS_FILE_PATH))
                .expect("tracked export should be restored"),
            "version = 1\n"
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
    fn active_workspace_load_reads_store_when_tracked_export_is_missing() {
        let repo = TempGitRepo::new("authority-active-load");

        SqlitePlanningAuthorityAdapter::commit_active_workspace_files(
            repo.worktree_root.to_str().expect("valid worktree path"),
            &PlanningWorkspaceLoadRecord {
                directions_toml: Some("version = 4\n".to_string()),
                task_ledger_json: None,
                task_ledger_schema_json: None,
                queue_snapshot_json: None,
                result_output_markdown: None,
            },
        )
        .expect("active commit should succeed");
        fs::remove_file(repo.repo_root.join(DIRECTIONS_FILE_PATH))
            .expect("tracked export should be removable");

        let loaded = SqlitePlanningAuthorityAdapter::load_active_workspace_files(
            repo.worktree_root.to_str().expect("valid worktree path"),
        )
        .expect("active workspace should load from store");
        let directions = SqlitePlanningAuthorityAdapter::load_active_planning_file(
            repo.worktree_root.to_str().expect("valid worktree path"),
            DIRECTIONS_FILE_PATH,
        )
        .expect("active directions should load");

        assert_eq!(loaded.directions_toml.as_deref(), Some("version = 4\n"));
        assert_eq!(directions.as_deref(), Some("version = 4\n"));
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

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskAuthoritySnapshot,
    PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadFileRecord, PlanningDraftLoadRecord,
    PlanningDraftStageRecord, PlanningStagedFileRecord, PlanningWorkspaceLoadRecord,
};
use crate::application::service::planning::shared::contract::{
    DIRECTIONS_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    TASK_LEDGER_SCHEMA_FILE_PATH,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
};
use crate::domain::planning::{
    PLANNING_FORMAT_VERSION, PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection,
    PlanningAuthorityShadowStoreSyncState, PriorityQueueSkippedTask, PriorityQueueSnapshot,
    PriorityQueueTask, TaskLedgerDocument,
};

const AKRA_HOME_ENV: &str = "AKRA_HOME";
const AKRA_HOME_DIRECTORY: &str = ".akra";
const AKRA_PROJECTS_DIRECTORY: &str = "projects";
const RUNTIME_DIRECTORY: &str = "runtime";
const RUNTIME_EXPORTS_DIRECTORY: &str = ".codex-exec-loop/runtime/exports";
const AUTHORITY_STORE_FILE_NAME: &str = "planning-authority.db";
const PLANNING_SNAPSHOT_EXPORT_FILE_NAME: &str = "planning-snapshot.json";
const TASK_LEDGER_EXPORT_FILE_NAME: &str = "task-ledger.json";
const QUEUE_SNAPSHOT_EXPORT_FILE_NAME: &str = "queue.snapshot.json";
const AUTHORITY_STORE_SCHEMA_VERSION: i64 = 5;
const AUTHORITY_STORE_MODE: &str = "authority-store";
const OFFICIAL_REFRESH_SCOPE_KEY: &str = "official-refresh";
const DISTRIBUTOR_QUEUE_CLAIM_KIND: &str = "distributor-queue-head";
const CLAIM_STALE_AFTER_SECS: i64 = 300;
const TASK_LEDGER_VERSION_METADATA_KEY: &str = "task_ledger_version";

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

        refresh_runtime_exports(&location)?;
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
        let existing_snapshot = load_task_authority_snapshot_from_connection(&transaction)?;
        let has_stale_task_documents = raw_active_document(&transaction, TASK_LEDGER_FILE_PATH)?
            .is_some()
            || raw_active_document(&transaction, QUEUE_SNAPSHOT_FILE_PATH)?.is_some();
        if let Some(existing_snapshot) = existing_snapshot
            && existing_snapshot.task_ledger == *commit.task_ledger
            && existing_snapshot.queue_snapshot == *commit.queue_snapshot
            && !has_stale_task_documents
        {
            return Ok(PlanningTaskAuthorityCommitResult::Committed {
                planning_revision: current_revision,
            });
        }

        upsert_authority_metadata(&transaction, &location, "last_task_authority_commit_at")?;
        replace_task_authority_tables(&transaction, commit.task_ledger, commit.queue_snapshot)?;
        remove_active_documents(&transaction, TASK_LEDGER_FILE_PATH)?;
        remove_active_documents(&transaction, QUEUE_SNAPSHOT_FILE_PATH)?;
        let planning_revision = bump_planning_revision(&transaction)?;
        transaction
            .commit()
            .context("failed to commit task authority transaction")?;

        refresh_runtime_exports(&location)?;
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
        remove_active_documents(&transaction, TASK_LEDGER_FILE_PATH)?;
        remove_active_documents(&transaction, QUEUE_SNAPSHOT_FILE_PATH)?;
        bump_planning_revision(&transaction)?;
        transaction
            .commit()
            .context("failed to clear task authority transaction")?;

        refresh_runtime_exports(&location)?;
        Ok(())
    }

    pub(crate) fn replace_active_planning_file(
        workspace_dir: &str,
        relative_path: &str,
        body: Option<&str>,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let existing_task_ledger = if relative_path == QUEUE_SNAPSHOT_FILE_PATH {
            load_task_ledger_from_connection(&connection)?
        } else {
            None
        };
        let existing_queue_snapshot = if relative_path == TASK_LEDGER_FILE_PATH {
            load_queue_snapshot_from_connection(&connection)?
        } else {
            None
        };

        let transaction = connection
            .transaction()
            .context("failed to open authority-store active file transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_active_commit_at")?;
        let changed = if relative_path == TASK_LEDGER_FILE_PATH {
            match body {
                Some(body) => {
                    let task_ledger = serde_json::from_str::<TaskLedgerDocument>(body)
                        .with_context(|| format!("failed to parse `{TASK_LEDGER_FILE_PATH}`"))?;
                    let queue_snapshot =
                        existing_queue_snapshot.unwrap_or_else(empty_queue_snapshot);
                    replace_task_authority_tables(&transaction, &task_ledger, &queue_snapshot)?;
                    remove_active_documents(&transaction, TASK_LEDGER_FILE_PATH)?;
                    remove_active_documents(&transaction, QUEUE_SNAPSHOT_FILE_PATH)?;
                }
                None => {
                    clear_task_authority_tables(&transaction)?;
                    remove_active_documents(&transaction, TASK_LEDGER_FILE_PATH)?;
                    remove_active_documents(&transaction, QUEUE_SNAPSHOT_FILE_PATH)?;
                }
            }
            true
        } else if relative_path == QUEUE_SNAPSHOT_FILE_PATH {
            match (existing_task_ledger, body) {
                (Some(task_ledger), Some(body)) => {
                    let queue_snapshot = parse_queue_snapshot_export(body);
                    replace_task_authority_tables(&transaction, &task_ledger, &queue_snapshot)?;
                    remove_active_documents(&transaction, QUEUE_SNAPSHOT_FILE_PATH)?;
                }
                (Some(task_ledger), None) => {
                    replace_task_authority_tables(
                        &transaction,
                        &task_ledger,
                        &empty_queue_snapshot(),
                    )?;
                    remove_active_documents(&transaction, QUEUE_SNAPSHOT_FILE_PATH)?;
                }
                (None, _) => {
                    set_active_document(&transaction, relative_path, body)?;
                }
            }
            true
        } else {
            set_active_document(&transaction, relative_path, body)?
        };
        if changed {
            bump_planning_revision(&transaction)?;
        }
        transaction
            .commit()
            .context("failed to commit authority-store active file transaction")?;

        refresh_runtime_exports(&location)?;
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
        let changed = if relative_path == TASK_LEDGER_FILE_PATH {
            clear_task_authority_tables(&transaction)?;
            remove_active_documents(&transaction, TASK_LEDGER_FILE_PATH)?;
            remove_active_documents(&transaction, QUEUE_SNAPSHOT_FILE_PATH)?;
            true
        } else if relative_path == QUEUE_SNAPSHOT_FILE_PATH {
            if let Some(task_ledger) = load_task_ledger_from_connection(&transaction)? {
                replace_task_authority_tables(&transaction, &task_ledger, &empty_queue_snapshot())?;
                remove_active_documents(&transaction, QUEUE_SNAPSHOT_FILE_PATH)?;
                true
            } else {
                remove_active_documents(&transaction, relative_path)?
            }
        } else {
            remove_active_documents(&transaction, relative_path)?
        };
        if changed {
            bump_planning_revision(&transaction)?;
        }
        transaction
            .commit()
            .context("failed to commit authority-store active removal transaction")?;

        refresh_runtime_exports(&location)?;
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

        if clear_stale_runtime_claim(&transaction, "official-refresh", OFFICIAL_REFRESH_SCOPE_KEY)?
        {
            upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        }
        let existing_owner =
            load_runtime_claim(&transaction, "official-refresh", OFFICIAL_REFRESH_SCOPE_KEY)?
                .map(|claim| claim.owner_token);
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
        if clear_stale_runtime_claim(&transaction, DISTRIBUTOR_QUEUE_CLAIM_KIND, queue_item_id)? {
            upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        }
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
        let connection = open_authority_connection(&location)?;
        load_runtime_projection_snapshot(&connection)
    }

    pub(crate) fn upsert_runtime_slot_lease(
        workspace_dir: &str,
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

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
        let runtime_dir = management_project_root(&canonical_repo_root).join(RUNTIME_DIRECTORY);
        let authority_store_path = runtime_dir.join(AUTHORITY_STORE_FILE_NAME);

        Ok(PlanningAuthorityLocation {
            workspace_root: workspace_root.display().to_string(),
            canonical_repo_root: canonical_repo_root.display().to_string(),
            runtime_dir: runtime_dir.display().to_string(),
            authority_store_path: authority_store_path.display().to_string(),
        })
    }

    fn collect_runtime_export_view(
        location: &PlanningAuthorityLocation,
    ) -> Result<PlanningAuthorityExportView> {
        Ok(PlanningAuthorityExportView {
            snapshot_documents: load_planning_snapshot_export(location)?,
            task_ledger_view: read_optional_export_file(&task_ledger_export_path(location))?,
            queue_snapshot_view: read_optional_export_file(&queue_snapshot_export_path(location))?,
        })
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
        let exported_view = Self::collect_runtime_export_view(&location)?;
        if source_documents.is_empty() && exported_view.has_any_content() {
            return Err(anyhow!(
                "authority store is empty while runtime exports still exist"
            ));
        }
        let export_parity_issues = compare_exported_documents(&source_documents, &exported_view);
        if !export_parity_issues.is_empty() {
            sync_exported_authority_documents(&location, &source_documents)?;
        }
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

impl PlanningTaskRepositoryPort for SqlitePlanningAuthorityAdapter {
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

fn load_shadow_documents(connection: &Connection) -> Result<BTreeMap<String, String>> {
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

fn load_active_authority_documents(connection: &Connection) -> Result<BTreeMap<String, String>> {
    let mut documents = load_active_documents(connection)?;
    if let Some(task_ledger_json) = load_task_ledger_json_from_connection(connection)? {
        documents.insert(TASK_LEDGER_FILE_PATH.to_string(), task_ledger_json);
    }
    if let Some(queue_snapshot_json) = load_queue_snapshot_json_from_connection(connection)? {
        documents.insert(QUEUE_SNAPSHOT_FILE_PATH.to_string(), queue_snapshot_json);
    }
    Ok(documents)
}

fn load_active_workspace_record(connection: &Connection) -> Result<PlanningWorkspaceLoadRecord> {
    let documents = load_active_documents(connection)?;
    Ok(PlanningWorkspaceLoadRecord {
        directions_toml: documents.get(DIRECTIONS_FILE_PATH).cloned(),
        task_ledger_json: load_task_ledger_json_from_connection(connection)?
            .or_else(|| documents.get(TASK_LEDGER_FILE_PATH).cloned()),
        task_ledger_schema_json: documents.get(TASK_LEDGER_SCHEMA_FILE_PATH).cloned(),
        queue_snapshot_json: load_queue_snapshot_json_from_connection(connection)?
            .or_else(|| documents.get(QUEUE_SNAPSHOT_FILE_PATH).cloned()),
        result_output_markdown: documents.get(RESULT_OUTPUT_FILE_PATH).cloned(),
    })
}

fn load_active_document(connection: &Connection, relative_path: &str) -> Result<Option<String>> {
    if relative_path == TASK_LEDGER_FILE_PATH {
        return load_task_ledger_json_from_connection(connection);
    }
    if relative_path == QUEUE_SNAPSHOT_FILE_PATH {
        return load_queue_snapshot_json_from_connection(connection);
    }

    connection
        .query_row(
            "SELECT content FROM active_documents WHERE relative_path = ?1",
            params![relative_path],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("failed to read active authority document `{relative_path}`"))
}

fn raw_active_document(connection: &Connection, relative_path: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT content FROM active_documents WHERE relative_path = ?1",
            params![relative_path],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("failed to read active authority document `{relative_path}`"))
}

fn load_task_authority_snapshot_from_connection(
    connection: &Connection,
) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
    let Some(task_ledger) = load_task_ledger_from_connection(connection)? else {
        return Ok(None);
    };
    let queue_snapshot =
        load_queue_snapshot_from_connection(connection)?.unwrap_or_else(empty_queue_snapshot);
    let planning_revision =
        read_metadata_i64_connection(connection, "planning_revision")?.unwrap_or(0);
    Ok(Some(PlanningTaskAuthoritySnapshot {
        planning_revision,
        task_ledger,
        queue_snapshot,
    }))
}

fn load_task_ledger_json_from_connection(connection: &Connection) -> Result<Option<String>> {
    load_task_ledger_from_connection(connection)?
        .map(|task_ledger| serialize_pretty_json(&task_ledger))
        .transpose()
}

fn load_queue_snapshot_json_from_connection(connection: &Connection) -> Result<Option<String>> {
    load_queue_snapshot_from_connection(connection)?
        .map(|queue_snapshot| serialize_pretty_json(&queue_snapshot))
        .transpose()
}

fn load_task_ledger_from_connection(connection: &Connection) -> Result<Option<TaskLedgerDocument>> {
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

    Ok(Some(TaskLedgerDocument { version, tasks }))
}

fn load_queue_snapshot_from_connection(
    connection: &Connection,
) -> Result<Option<PriorityQueueSnapshot>> {
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

    Ok(Some(PriorityQueueSnapshot {
        next_task: active_tasks.first().cloned(),
        active_tasks,
        proposed_tasks,
        skipped_tasks,
    }))
}

fn task_authority_exists(connection: &Connection) -> Result<bool> {
    if read_metadata_string_connection(connection, TASK_LEDGER_VERSION_METADATA_KEY)?.is_some() {
        return Ok(true);
    }
    connection
        .query_row("SELECT 1 FROM planning_tasks LIMIT 1", [], |_| Ok(()))
        .optional()
        .context("failed to inspect planning task authority")
        .map(|value| value.is_some())
}

fn serialize_pretty_json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value).context("failed to serialize planning authority json")
}

fn empty_queue_snapshot() -> PriorityQueueSnapshot {
    PriorityQueueSnapshot {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    }
}

fn parse_queue_snapshot_export(body: &str) -> PriorityQueueSnapshot {
    serde_json::from_str::<PriorityQueueSnapshot>(body).unwrap_or_else(|_| empty_queue_snapshot())
}

fn open_authority_connection(location: &PlanningAuthorityLocation) -> Result<Connection> {
    let authority_store_path = Path::new(&location.authority_store_path);
    migrate_legacy_authority_store_if_needed(location)?;
    if let Some(parent) = authority_store_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let mut connection = Connection::open(authority_store_path)
        .with_context(|| format!("failed to open {}", authority_store_path.display()))?;
    validate_authority_store_schema(&connection)?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .context("failed to enable authority-store foreign keys")?;
    ensure_schema(&connection)?;
    backfill_task_authority_from_active_documents(&mut connection)?;
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

#[derive(Default)]
struct PlanningAuthorityExportView {
    snapshot_documents: BTreeMap<String, String>,
    task_ledger_view: Option<String>,
    queue_snapshot_view: Option<String>,
}

impl PlanningAuthorityExportView {
    fn has_any_content(&self) -> bool {
        !self.snapshot_documents.is_empty()
            || self.task_ledger_view.is_some()
            || self.queue_snapshot_view.is_some()
    }
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

fn parsed_schema_version(connection: &Connection) -> Result<Option<i64>> {
    Ok(load_schema_version(connection)?.and_then(|value| value.parse::<i64>().ok()))
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

fn backfill_task_authority_from_active_documents(connection: &mut Connection) -> Result<()> {
    let schema_version = parsed_schema_version(connection)?;
    if read_metadata_string_connection(connection, TASK_LEDGER_VERSION_METADATA_KEY)?.is_some() {
        record_current_schema_version_if_needed(connection, schema_version)?;
        return Ok(());
    }
    if connection
        .query_row("SELECT 1 FROM planning_tasks LIMIT 1", [], |_| Ok(()))
        .optional()
        .context("failed to inspect existing planning task rows")?
        .is_some()
    {
        record_current_schema_version_if_needed(connection, schema_version)?;
        return Ok(());
    }
    if schema_version.is_some_and(|version| version >= AUTHORITY_STORE_SCHEMA_VERSION) {
        return Ok(());
    }

    let Some(task_ledger_json) = raw_active_document(connection, TASK_LEDGER_FILE_PATH)? else {
        record_current_schema_version_if_needed(connection, schema_version)?;
        return Ok(());
    };
    let task_ledger = match serde_json::from_str::<TaskLedgerDocument>(&task_ledger_json) {
        Ok(task_ledger) => task_ledger,
        Err(_) => {
            record_current_schema_version_if_needed(connection, schema_version)?;
            return Ok(());
        }
    };
    let queue_snapshot = match raw_active_document(connection, QUEUE_SNAPSHOT_FILE_PATH)? {
        Some(queue_snapshot_json) => parse_queue_snapshot_export(&queue_snapshot_json),
        None => empty_queue_snapshot(),
    };

    let transaction = connection
        .transaction()
        .context("failed to open task authority backfill transaction")?;
    replace_task_authority_tables(&transaction, &task_ledger, &queue_snapshot)?;
    upsert_metadata(
        &transaction,
        "schema_version",
        &AUTHORITY_STORE_SCHEMA_VERSION.to_string(),
    )?;
    transaction
        .commit()
        .context("failed to commit task authority backfill transaction")?;
    Ok(())
}

fn record_current_schema_version_if_needed(
    connection: &mut Connection,
    schema_version: Option<i64>,
) -> Result<()> {
    if schema_version.is_some_and(|version| version >= AUTHORITY_STORE_SCHEMA_VERSION) {
        return Ok(());
    }

    let transaction = connection
        .transaction()
        .context("failed to open authority-store schema migration transaction")?;
    upsert_metadata(
        &transaction,
        "schema_version",
        &AUTHORITY_STORE_SCHEMA_VERSION.to_string(),
    )?;
    transaction
        .commit()
        .context("failed to commit authority-store schema migration transaction")?;
    Ok(())
}

#[derive(Debug)]
struct RuntimeClaimRecord {
    owner_token: String,
    claimed_at: String,
}

fn load_runtime_claim(
    transaction: &rusqlite::Transaction<'_>,
    claim_kind: &str,
    scope_key: &str,
) -> Result<Option<RuntimeClaimRecord>> {
    transaction
        .query_row(
            "SELECT owner_token, claimed_at
             FROM runtime_claims
             WHERE claim_kind = ?1 AND scope_key = ?2",
            params![claim_kind, scope_key],
            |row| {
                Ok(RuntimeClaimRecord {
                    owner_token: row.get::<_, String>(0)?,
                    claimed_at: row.get::<_, String>(1)?,
                })
            },
        )
        .optional()
        .with_context(|| format!("failed to read runtime claim `{claim_kind}:{scope_key}`"))
}

fn clear_stale_runtime_claim(
    transaction: &rusqlite::Transaction<'_>,
    claim_kind: &str,
    scope_key: &str,
) -> Result<bool> {
    let Some(existing_claim) = load_runtime_claim(transaction, claim_kind, scope_key)? else {
        return Ok(false);
    };
    if !claim_is_stale(&existing_claim.claimed_at) {
        return Ok(false);
    }

    transaction
        .execute(
            "DELETE FROM runtime_claims WHERE claim_kind = ?1 AND scope_key = ?2",
            params![claim_kind, scope_key],
        )
        .with_context(|| {
            format!("failed to clear stale runtime claim `{claim_kind}:{scope_key}`")
        })?;
    Ok(true)
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
        .join(format!("{repo_name}-akra-worktrees"))
        .join(stable_short_hash(&canonical_repo_root.to_string_lossy()))
        .join("akra-pool")
}

fn management_project_root(canonical_repo_root: &Path) -> PathBuf {
    let repo_name = canonical_repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    akra_home_root().join(AKRA_PROJECTS_DIRECTORY).join(format!(
        "{repo_name}-{}",
        stable_short_hash(&canonical_repo_root.to_string_lossy())
    ))
}

fn akra_home_root() -> PathBuf {
    if let Some(path) = env::var_os(AKRA_HOME_ENV).filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }

    #[cfg(test)]
    {
        env::temp_dir().join(AKRA_HOME_DIRECTORY).join("tests")
    }

    #[cfg(not(test))]
    {
        env::var_os("HOME")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(AKRA_HOME_DIRECTORY)
    }
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
    if let Some(task_ledger_json) = record.task_ledger_json.as_deref() {
        let task_ledger = serde_json::from_str::<TaskLedgerDocument>(task_ledger_json)
            .with_context(|| {
                format!("failed to parse active authority document `{TASK_LEDGER_FILE_PATH}`")
            })?;
        let queue_snapshot = match record.queue_snapshot_json.as_deref() {
            Some(queue_snapshot_json) => parse_queue_snapshot_export(queue_snapshot_json),
            None => empty_queue_snapshot(),
        };
        replace_task_authority_tables(transaction, &task_ledger, &queue_snapshot)?;
        changed |= true;
    } else {
        clear_task_authority_tables(transaction)?;
        changed |= true;
    }
    changed |= remove_active_documents(transaction, TASK_LEDGER_FILE_PATH)?;
    changed |= set_active_document(
        transaction,
        TASK_LEDGER_SCHEMA_FILE_PATH,
        record.task_ledger_schema_json.as_deref(),
    )?;
    changed |= remove_active_documents(transaction, QUEUE_SNAPSHOT_FILE_PATH)?;
    changed |= set_active_document(
        transaction,
        RESULT_OUTPUT_FILE_PATH,
        record.result_output_markdown.as_deref(),
    )?;
    Ok(changed)
}

fn replace_task_authority_tables(
    transaction: &rusqlite::Transaction<'_>,
    task_ledger: &TaskLedgerDocument,
    queue_snapshot: &PriorityQueueSnapshot,
) -> Result<()> {
    clear_task_authority_rows(transaction)?;
    for (index, task) in task_ledger.tasks.iter().enumerate() {
        let task_id = task.id.trim();
        transaction
            .execute(
                "INSERT INTO planning_tasks
                 (task_id, task_order, direction_id, title, status, base_priority,
                  dynamic_priority_delta, combined_priority, updated_at, source_turn_id, content_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                    serde_json::to_string(task)
                        .context("failed to serialize planning task row")?,
                ],
            )
            .with_context(|| format!("failed to persist planning task `{task_id}`"))?;
        insert_task_edges(transaction, task_id, "depends_on", &task.depends_on)?;
        insert_task_edges(transaction, task_id, "blocked_by", &task.blocked_by)?;
    }
    insert_queue_projection_tasks(transaction, "active", &queue_snapshot.active_tasks)?;
    insert_queue_projection_tasks(transaction, "proposed", &queue_snapshot.proposed_tasks)?;
    insert_queue_projection_skipped(transaction, &queue_snapshot.skipped_tasks)?;
    upsert_metadata(
        transaction,
        TASK_LEDGER_VERSION_METADATA_KEY,
        &task_ledger.version.to_string(),
    )?;
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

fn clear_task_authority_tables(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
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
    transaction
        .execute("DELETE FROM planning_queue_projection", [])
        .context("failed to clear planning queue projection")?;
    transaction
        .execute("DELETE FROM planning_task_edges", [])
        .context("failed to clear planning task edges")?;
    transaction
        .execute("DELETE FROM planning_tasks", [])
        .context("failed to clear planning task rows")?;
    Ok(())
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

fn claim_is_stale(claimed_at: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(claimed_at)
        .map(|timestamp| {
            Utc::now()
                .signed_duration_since(timestamp.with_timezone(&Utc))
                .num_seconds()
                >= CLAIM_STALE_AFTER_SECS
        })
        .unwrap_or(true)
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

fn refresh_runtime_exports(location: &PlanningAuthorityLocation) -> Result<()> {
    let connection = open_authority_connection(location)?;
    let source_documents = load_active_authority_documents(&connection)?;
    sync_exported_authority_documents(location, &source_documents)
}

fn runtime_exports_root(location: &PlanningAuthorityLocation) -> PathBuf {
    Path::new(&location.canonical_repo_root).join(RUNTIME_EXPORTS_DIRECTORY)
}

fn planning_snapshot_export_path(location: &PlanningAuthorityLocation) -> PathBuf {
    runtime_exports_root(location).join(PLANNING_SNAPSHOT_EXPORT_FILE_NAME)
}

fn task_ledger_export_path(location: &PlanningAuthorityLocation) -> PathBuf {
    runtime_exports_root(location).join(TASK_LEDGER_EXPORT_FILE_NAME)
}

fn queue_snapshot_export_path(location: &PlanningAuthorityLocation) -> PathBuf {
    runtime_exports_root(location).join(QUEUE_SNAPSHOT_EXPORT_FILE_NAME)
}

fn read_optional_export_file(path: &Path) -> Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }

    fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))
        .map(Some)
}

fn write_optional_export_file(path: &Path, body: Option<&str>) -> Result<()> {
    match body {
        Some(body) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))?;
        }
        None => {
            if path.exists() {
                fs::remove_file(path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
            }
        }
    }
    Ok(())
}

fn load_planning_snapshot_export(
    location: &PlanningAuthorityLocation,
) -> Result<BTreeMap<String, String>> {
    let Some(snapshot_body) = read_optional_export_file(&planning_snapshot_export_path(location))?
    else {
        return Ok(BTreeMap::new());
    };
    serde_json::from_str::<BTreeMap<String, String>>(&snapshot_body).with_context(|| {
        format!(
            "failed to parse {}",
            planning_snapshot_export_path(location).display()
        )
    })
}

fn sync_exported_authority_documents(
    location: &PlanningAuthorityLocation,
    source_documents: &BTreeMap<String, String>,
) -> Result<()> {
    let snapshot_path = planning_snapshot_export_path(location);
    let snapshot_body = if source_documents.is_empty() {
        None
    } else {
        let mut snapshot_json = serde_json::to_string_pretty(source_documents)
            .context("failed to serialize runtime export planning snapshot")?;
        snapshot_json.push('\n');
        Some(snapshot_json)
    };
    write_optional_export_file(&snapshot_path, snapshot_body.as_deref())?;
    write_optional_export_file(
        &task_ledger_export_path(location),
        source_documents
            .get(TASK_LEDGER_FILE_PATH)
            .map(String::as_str),
    )?;
    write_optional_export_file(
        &queue_snapshot_export_path(location),
        source_documents
            .get(QUEUE_SNAPSHOT_FILE_PATH)
            .map(String::as_str),
    )?;
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

fn compare_runtime_export_view(
    label: &str,
    source: Option<&str>,
    exported: Option<&str>,
    issues: &mut Vec<String>,
) {
    match (source, exported) {
        (Some(_), None) => {
            issues.push(format!("{label}: runtime export missing"));
        }
        (None, Some(_)) => {
            issues.push(format!("{label}: runtime export contains stale content"));
        }
        (Some(source), Some(exported)) if source != exported => {
            issues.push(format!("{label}: runtime export mismatch"));
        }
        _ => {}
    }
}

fn compare_exported_documents(
    source_documents: &BTreeMap<String, String>,
    exported_view: &PlanningAuthorityExportView,
) -> Vec<String> {
    let document_paths = source_documents
        .keys()
        .chain(exported_view.snapshot_documents.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut issues = Vec::new();
    for relative_path in document_paths {
        match (
            source_documents.get(&relative_path),
            exported_view.snapshot_documents.get(&relative_path),
        ) {
            (Some(_), None) => {
                issues.push(format!("{relative_path}: runtime export snapshot missing"));
            }
            (None, Some(_)) => {
                issues.push(format!(
                    "{relative_path}: runtime export snapshot contains stale content"
                ));
            }
            (Some(source), Some(exported)) if source != exported => {
                issues.push(format!("{relative_path}: runtime export snapshot mismatch"));
            }
            _ => {}
        }
    }

    compare_runtime_export_view(
        TASK_LEDGER_FILE_PATH,
        source_documents
            .get(TASK_LEDGER_FILE_PATH)
            .map(String::as_str),
        exported_view.task_ledger_view.as_deref(),
        &mut issues,
    );
    compare_runtime_export_view(
        QUEUE_SNAPSHOT_FILE_PATH,
        source_documents
            .get(QUEUE_SNAPSHOT_FILE_PATH)
            .map(String::as_str),
        exported_view.queue_snapshot_view.as_deref(),
        &mut issues,
    );

    issues
}

fn resolve_canonical_repo_root(workspace_dir: &str) -> Option<PathBuf> {
    let cache_key = canonicalize_best_effort(Path::new(workspace_dir))
        .display()
        .to_string();
    if let Some(cached_root) = canonical_repo_root_cache()
        .lock()
        .expect("canonical repo root cache mutex poisoned")
        .get(&cache_key)
        .cloned()
    {
        return Some(cached_root);
    }

    let resolved_root = resolve_canonical_repo_root_uncached(workspace_dir)?;
    canonical_repo_root_cache()
        .lock()
        .expect("canonical repo root cache mutex poisoned")
        .insert(cache_key, resolved_root.clone());
    Some(resolved_root)
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

fn resolve_canonical_repo_root_uncached(workspace_dir: &str) -> Option<PathBuf> {
    let show_toplevel = git_stdout(workspace_dir, &["rev-parse", "--show-toplevel"])?;
    let common_dir = git_stdout(workspace_dir, &["rev-parse", "--git-common-dir"])?;
    let git_dir = git_stdout(workspace_dir, &["rev-parse", "--git-dir"])?;
    let workspace_path = Path::new(workspace_dir);
    let canonical_toplevel =
        canonicalize_best_effort(&absolutize_path(workspace_path, Path::new(&show_toplevel)));
    let canonical_common_dir =
        canonicalize_best_effort(&absolutize_path(workspace_path, Path::new(&common_dir)));
    let canonical_git_dir =
        canonicalize_best_effort(&absolutize_path(workspace_path, Path::new(&git_dir)));
    let worktrees_root = canonical_common_dir.join("worktrees");
    if canonical_git_dir.starts_with(&worktrees_root) {
        return canonical_common_dir.parent().map(Path::to_path_buf);
    }
    Some(canonical_toplevel)
}

fn canonical_repo_root_cache() -> &'static Mutex<BTreeMap<String, PathBuf>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, PathBuf>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
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
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;

    use super::{AUTHORITY_STORE_SCHEMA_VERSION, SqlitePlanningAuthorityAdapter};
    use crate::application::port::outbound::planning_authority_port::{
        PlanningAuthorityOfficialRefreshClaimStatus, PlanningAuthorityPort,
    };
    use crate::application::port::outbound::planning_task_repository_port::{
        PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
    };
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
    use crate::application::service::planning::shared::contract::{
        DIRECTIONS_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    };
    use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};
    use crate::domain::planning::{
        PlanningAuthorityShadowStoreSyncState, PriorityQueueSnapshot, TaskLedgerDocument,
    };

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

    fn runtime_exports_root(repo_root: &Path) -> PathBuf {
        repo_root.join(".codex-exec-loop/runtime/exports")
    }

    fn planning_snapshot_export_path(repo_root: &Path) -> PathBuf {
        runtime_exports_root(repo_root).join("planning-snapshot.json")
    }

    fn task_ledger_export_path(repo_root: &Path) -> PathBuf {
        runtime_exports_root(repo_root).join("task-ledger.json")
    }

    fn queue_snapshot_export_path(repo_root: &Path) -> PathBuf {
        runtime_exports_root(repo_root).join("queue.snapshot.json")
    }

    fn read_planning_snapshot_export(repo_root: &Path) -> BTreeMap<String, String> {
        let snapshot_body = fs::read_to_string(planning_snapshot_export_path(repo_root))
            .expect("planning snapshot export should exist");
        serde_json::from_str::<BTreeMap<String, String>>(&snapshot_body)
            .expect("planning snapshot export should parse")
    }

    fn task_ledger_with_ready_task_json() -> String {
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "direction-1",
      "direction_relation_note": "implements direction",
      "title": "Task One",
      "description": "Do task one.",
      "status": "ready",
      "base_priority": 10,
      "dynamic_priority_delta": 2,
      "priority_reason": "important",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "system",
      "source_turn_id": "turn-1",
      "updated_at": "2026-04-20T10:00:00Z"
    }
  ]
}"#
        .to_string()
    }

    fn queue_snapshot_with_ready_task_json() -> String {
        r#"{
  "next_task": {
    "rank": 1,
    "task_id": "task-1",
    "direction_id": "direction-1",
    "direction_title": "Direction One",
    "task_title": "Task One",
    "status": "ready",
    "combined_priority": 12,
    "updated_at": "2026-04-20T10:00:00Z",
    "rank_reasons": [
      "status=ready",
      "combined_priority=12 (base 10 + delta 2)"
    ]
  },
  "active_tasks": [
    {
      "rank": 1,
      "task_id": "task-1",
      "direction_id": "direction-1",
      "direction_title": "Direction One",
      "task_title": "Task One",
      "status": "ready",
      "combined_priority": 12,
      "updated_at": "2026-04-20T10:00:00Z",
      "rank_reasons": [
        "status=ready",
        "combined_priority=12 (base 10 + delta 2)"
      ]
    }
  ],
  "proposed_tasks": [],
  "skipped_tasks": []
}"#
        .to_string()
    }

    fn parse_task_ledger(body: &str) -> TaskLedgerDocument {
        serde_json::from_str(body).expect("task ledger should parse")
    }

    fn parse_queue_snapshot(body: &str) -> PriorityQueueSnapshot {
        serde_json::from_str(body).expect("queue snapshot should parse")
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
        let normalized_runtime_dir = location.runtime_dir.replace('\\', "/");
        let normalized_store_path = location.authority_store_path.replace('\\', "/");
        assert!(normalized_runtime_dir.contains("/.akra/tests/projects/"));
        assert!(normalized_runtime_dir.ends_with("/runtime"));
        assert!(normalized_store_path.ends_with("/runtime/planning-authority.db"));
    }

    #[test]
    fn resolve_authority_location_uses_workspace_root_for_separate_git_dir_repo() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("separate-git-dir-{unique}"));
        let workspace_root = root.join("workspace");
        let external_git_dir = root.join("external-git-dir");
        fs::create_dir_all(&workspace_root).expect("workspace root should exist");
        let status = Command::new("git")
            .current_dir(&root)
            .args([
                "init",
                "--separate-git-dir",
                external_git_dir.to_str().expect("valid git dir path"),
                workspace_root.to_str().expect("valid workspace path"),
            ])
            .status()
            .expect("git init should spawn");
        assert!(
            status.success(),
            "git init with separate git dir should succeed"
        );

        let adapter = SqlitePlanningAuthorityAdapter::new();
        let location = adapter
            .resolve_authority_location(workspace_root.to_str().expect("valid path"))
            .expect("authority location should resolve");

        assert_eq!(
            location.canonical_repo_root,
            fs::canonicalize(&workspace_root)
                .expect("workspace root should canonicalize")
                .display()
                .to_string()
        );
        assert!(
            location
                .authority_store_path
                .replace('\\', "/")
                .ends_with("/runtime/planning-authority.db")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn inspect_shadow_store_bootstraps_from_active_store() {
        let repo = TempGitRepo::new("shadow-bootstrap");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        SqlitePlanningAuthorityAdapter::replace_active_planning_file(
            repo.worktree_root.to_str().expect("valid path"),
            ".codex-exec-loop/planning/directions.toml",
            Some("version = 1\n"),
        )
        .expect("directions should seed the authority store");
        SqlitePlanningAuthorityAdapter::replace_active_planning_file(
            repo.worktree_root.to_str().expect("valid path"),
            ".codex-exec-loop/planning/task-ledger.json",
            Some("{\"version\":1,\"tasks\":[]}\n"),
        )
        .expect("task ledger should seed the authority store");
        SqlitePlanningAuthorityAdapter::replace_active_planning_file(
            repo.worktree_root.to_str().expect("valid path"),
            ".codex-exec-loop/planning/prompts/queue-idle-review.md",
            Some("# review\n"),
        )
        .expect("prompt should seed the authority store");

        let inspection = adapter
            .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
            .expect("shadow store should inspect");

        assert_eq!(
            inspection.sync_state,
            PlanningAuthorityShadowStoreSyncState::Bootstrapped
        );
        assert_eq!(inspection.mirrored_document_count, 4);
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
    fn inspect_shadow_store_restores_diverged_runtime_exports_from_active_store() {
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

        fs::write(
            planning_snapshot_export_path(&repo.repo_root),
            "{\n  \".codex-exec-loop/planning/directions.toml\": \"version = 2\\n\"\n}\n",
        )
        .expect("runtime export snapshot should diverge");

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
                .any(|issue| issue.contains("runtime export"))
        );
        assert_eq!(
            read_planning_snapshot_export(&repo.repo_root)
                .get(DIRECTIONS_FILE_PATH)
                .expect("runtime export snapshot should be restored"),
            "version = 1\n"
        );
    }

    #[test]
    fn inspect_shadow_store_rejects_export_only_legacy_state() {
        let repo = TempGitRepo::new("shadow-export-only");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        fs::create_dir_all(runtime_exports_root(&repo.repo_root))
            .expect("runtime exports root should exist");
        fs::write(
            planning_snapshot_export_path(&repo.repo_root),
            "{\n  \".codex-exec-loop/planning/directions.toml\": \"version = 1\\n\"\n}\n",
        )
        .expect("runtime export snapshot should write");

        let error = adapter
            .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
            .expect_err("export-only legacy state should be rejected");

        assert!(
            error
                .to_string()
                .contains("authority store is empty while runtime exports still exist")
        );
    }

    #[test]
    fn inspect_shadow_store_rejects_legacy_schema_version_one_store() {
        let repo = TempGitRepo::new("shadow-upgrade-v1");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        repo.write_repo_file(".codex-exec-loop/planning/directions.toml", "version = 1\n");
        let location = adapter
            .resolve_authority_location(repo.worktree_root.to_str().expect("valid path"))
            .expect("authority location should resolve");
        let runtime_dir = PathBuf::from(&location.runtime_dir);
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

        let error = adapter
            .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
            .expect_err("legacy schema version should be rejected");

        assert_eq!(
            error.to_string(),
            "unsupported authority-store schema version: 1"
        );
    }

    #[test]
    fn active_commit_updates_repo_scoped_documents_for_linked_worktree() {
        let repo = TempGitRepo::new("authority-active-commit");
        let task_ledger_json = task_ledger_with_ready_task_json();
        let queue_snapshot_json = queue_snapshot_with_ready_task_json();

        SqlitePlanningAuthorityAdapter::commit_active_workspace_files(
            repo.worktree_root.to_str().expect("valid worktree path"),
            &PlanningWorkspaceLoadRecord {
                directions_toml: Some("version = 4\n".to_string()),
                task_ledger_json: Some(task_ledger_json.clone()),
                task_ledger_schema_json: Some("{\"type\":\"object\"}\n".to_string()),
                queue_snapshot_json: Some(queue_snapshot_json.clone()),
                result_output_markdown: Some("# result\n".to_string()),
            },
        )
        .expect("active commit should succeed");

        assert_eq!(
            read_planning_snapshot_export(&repo.repo_root)
                .get(DIRECTIONS_FILE_PATH)
                .expect("runtime export directions should exist"),
            "version = 4\n"
        );
        assert!(
            fs::read_to_string(task_ledger_export_path(&repo.repo_root))
                .expect("runtime export task ledger should exist")
                .contains("\"id\": \"task-1\"")
        );
        assert!(
            !fs::read_to_string(queue_snapshot_export_path(&repo.repo_root))
                .expect("runtime export queue snapshot should exist")
                .contains("\"bucket\"")
        );
        let location = SqlitePlanningAuthorityAdapter::new()
            .resolve_authority_location(repo.worktree_root.to_str().expect("valid worktree path"))
            .expect("authority location should resolve");
        let connection =
            Connection::open(&location.authority_store_path).expect("authority store should open");
        let stored_task_count = connection
            .query_row(
                "SELECT COUNT(*) FROM planning_tasks WHERE task_id = 'task-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("planning task rows should be readable");
        assert_eq!(stored_task_count, 1);
        let active_document_task_count = connection
            .query_row(
                "SELECT COUNT(*) FROM active_documents WHERE relative_path IN (?1, ?2)",
                [TASK_LEDGER_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH],
                |row| row.get::<_, i64>(0),
            )
            .expect("active document rows should be readable");
        assert_eq!(active_document_task_count, 0);
    }

    #[test]
    fn task_repository_commit_round_trips_relational_authority_projection() {
        let repo = TempGitRepo::new("authority-task-repository");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");
        let task_ledger = parse_task_ledger(&task_ledger_with_ready_task_json());
        let queue_snapshot = parse_queue_snapshot(&queue_snapshot_with_ready_task_json());

        adapter
            .commit_task_authority_snapshot(
                workspace_dir,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: None,
                    task_ledger: &task_ledger,
                    queue_snapshot: &queue_snapshot,
                },
            )
            .expect("task authority should commit");

        let snapshot = adapter
            .load_task_authority_snapshot(workspace_dir)
            .expect("task authority should load")
            .expect("task authority should exist");
        assert_eq!(snapshot.task_ledger, task_ledger);
        assert_eq!(snapshot.queue_snapshot, queue_snapshot);
        assert!(
            !fs::read_to_string(task_ledger_export_path(&repo.repo_root))
                .expect("task ledger export should exist")
                .contains("\"task_id\"")
        );
        assert!(
            fs::read_to_string(queue_snapshot_export_path(&repo.repo_root))
                .expect("queue snapshot export should exist")
                .contains("\"task_id\": \"task-1\"")
        );
    }

    #[test]
    fn task_repository_commit_rejects_stale_observed_revision() {
        let repo = TempGitRepo::new("authority-task-repository-conflict");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");
        let task_ledger = parse_task_ledger(&task_ledger_with_ready_task_json());
        let queue_snapshot = parse_queue_snapshot(&queue_snapshot_with_ready_task_json());
        let first_result = adapter
            .commit_task_authority_snapshot(
                workspace_dir,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: None,
                    task_ledger: &task_ledger,
                    queue_snapshot: &queue_snapshot,
                },
            )
            .expect("initial task authority should commit");
        let PlanningTaskAuthorityCommitResult::Committed { planning_revision } = first_result
        else {
            panic!("initial commit should not conflict");
        };
        assert_eq!(planning_revision, 1);

        let mut stale_task_ledger = task_ledger.clone();
        stale_task_ledger.tasks[0].title = "Stale writer update".to_string();
        let stale_result = adapter
            .commit_task_authority_snapshot(
                workspace_dir,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: Some(0),
                    task_ledger: &stale_task_ledger,
                    queue_snapshot: &queue_snapshot,
                },
            )
            .expect("stale task authority commit should return a conflict");

        assert_eq!(
            stale_result,
            PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: 0,
                current_planning_revision: 1,
            }
        );
    }

    #[test]
    fn legacy_active_task_ledger_blob_backfills_relational_tables() {
        let repo = TempGitRepo::new("authority-task-backfill");
        let location = SqlitePlanningAuthorityAdapter::new()
            .resolve_authority_location(repo.worktree_root.to_str().expect("valid worktree path"))
            .expect("authority location should resolve");
        let runtime_dir = Path::new(&location.runtime_dir);
        fs::create_dir_all(runtime_dir).expect("runtime dir should exist");
        let connection =
            Connection::open(&location.authority_store_path).expect("authority store should open");
        connection
            .execute_batch(
                r#"
                CREATE TABLE authority_metadata (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                CREATE TABLE active_documents (
                    relative_path TEXT PRIMARY KEY,
                    content TEXT NOT NULL
                );
                "#,
            )
            .expect("legacy authority store should initialize");
        connection
            .execute(
                "INSERT INTO authority_metadata (key, value) VALUES ('schema_version', '4')",
                [],
            )
            .expect("legacy schema version should insert");
        connection
            .execute(
                "INSERT INTO active_documents (relative_path, content) VALUES (?1, ?2)",
                [
                    TASK_LEDGER_FILE_PATH,
                    task_ledger_with_ready_task_json().as_str(),
                ],
            )
            .expect("legacy task ledger should insert");
        connection
            .execute(
                "INSERT INTO active_documents (relative_path, content) VALUES (?1, ?2)",
                [
                    QUEUE_SNAPSHOT_FILE_PATH,
                    queue_snapshot_with_ready_task_json().as_str(),
                ],
            )
            .expect("legacy queue snapshot should insert");
        drop(connection);

        let loaded = SqlitePlanningAuthorityAdapter::load_active_workspace_files(
            repo.worktree_root.to_str().expect("valid worktree path"),
        )
        .expect("active workspace should load");

        assert!(
            loaded
                .task_ledger_json
                .as_deref()
                .expect("task ledger should load")
                .contains("\"id\": \"task-1\"")
        );
        let connection =
            Connection::open(&location.authority_store_path).expect("authority store should open");
        let stored_task_count = connection
            .query_row("SELECT COUNT(*) FROM planning_tasks", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("planning task rows should be readable");
        let active_projection_count = connection
            .query_row(
                "SELECT COUNT(*) FROM planning_queue_projection WHERE bucket = 'active'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("planning queue projection rows should be readable");
        assert_eq!(stored_task_count, 1);
        assert_eq!(active_projection_count, 1);
        let schema_version = connection
            .query_row(
                "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("schema version should be readable");
        assert_eq!(schema_version, AUTHORITY_STORE_SCHEMA_VERSION.to_string());
    }

    #[test]
    fn legacy_invalid_task_ledger_blob_does_not_block_authority_open() {
        let repo = TempGitRepo::new("authority-task-backfill-invalid");
        let location = SqlitePlanningAuthorityAdapter::new()
            .resolve_authority_location(repo.worktree_root.to_str().expect("valid worktree path"))
            .expect("authority location should resolve");
        let runtime_dir = Path::new(&location.runtime_dir);
        fs::create_dir_all(runtime_dir).expect("runtime dir should exist");
        let connection =
            Connection::open(&location.authority_store_path).expect("authority store should open");
        connection
            .execute_batch(
                r#"
                CREATE TABLE authority_metadata (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                CREATE TABLE active_documents (
                    relative_path TEXT PRIMARY KEY,
                    content TEXT NOT NULL
                );
                "#,
            )
            .expect("legacy authority store should initialize");
        connection
            .execute(
                "INSERT INTO authority_metadata (key, value) VALUES ('schema_version', '4')",
                [],
            )
            .expect("legacy schema version should insert");
        connection
            .execute(
                "INSERT INTO active_documents (relative_path, content) VALUES (?1, ?2)",
                [TASK_LEDGER_FILE_PATH, "{\"version\":1,\"tasks\":["],
            )
            .expect("legacy invalid task ledger should insert");
        drop(connection);

        let loaded = SqlitePlanningAuthorityAdapter::load_active_workspace_files(
            repo.worktree_root.to_str().expect("valid worktree path"),
        )
        .expect("active workspace should still load");

        assert_eq!(
            loaded.task_ledger_json.as_deref(),
            Some("{\"version\":1,\"tasks\":[")
        );
        let connection =
            Connection::open(&location.authority_store_path).expect("authority store should open");
        let stored_task_count = connection
            .query_row("SELECT COUNT(*) FROM planning_tasks", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("planning task rows should be readable");
        let schema_version = connection
            .query_row(
                "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("schema version should be readable");
        assert_eq!(stored_task_count, 0);
        assert_eq!(schema_version, AUTHORITY_STORE_SCHEMA_VERSION.to_string());
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
        assert!(
            !repo.repo_root.join(DIRECTIONS_FILE_PATH).exists(),
            "tracked planning files should stay untouched in git-backed mode"
        );

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
    fn active_workspace_load_does_not_bootstrap_tracked_exports() {
        let repo = TempGitRepo::new("authority-active-no-bootstrap");
        repo.write_repo_file(DIRECTIONS_FILE_PATH, "version = 9\n");

        let loaded = SqlitePlanningAuthorityAdapter::load_active_workspace_files(
            repo.worktree_root.to_str().expect("valid worktree path"),
        )
        .expect("active workspace should load without bootstrap");
        let directions = SqlitePlanningAuthorityAdapter::load_active_planning_file(
            repo.worktree_root.to_str().expect("valid worktree path"),
            DIRECTIONS_FILE_PATH,
        )
        .expect("active directions should inspect without bootstrap");

        assert_eq!(loaded, PlanningWorkspaceLoadRecord::default());
        assert_eq!(directions, None);
        assert_eq!(
            fs::read_to_string(repo.repo_root.join(DIRECTIONS_FILE_PATH))
                .expect("tracked export should remain untouched"),
            "version = 9\n"
        );
    }

    #[test]
    fn runtime_projection_load_does_not_bootstrap_legacy_mirror_files() {
        let repo = TempGitRepo::new("runtime-projection-no-bootstrap");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        let location = adapter
            .resolve_authority_location(repo.worktree_root.to_str().expect("valid worktree path"))
            .expect("authority location should resolve");
        let mirrored_lease = ParallelModeSlotLeaseSnapshot::new(
            "slot-1",
            "task-1",
            "Task One",
            "agent-1",
            "akra-agent/slot-1/task-one",
            repo.worktree_root.display().to_string(),
            ParallelModeSlotLeaseState::Running,
            "2026-04-18T10:00:00Z",
            Some("2026-04-18T10:05:00Z".to_string()),
        );
        let mirrored_path = super::runtime_slot_lease_path(&location, &mirrored_lease.slot_id);
        fs::create_dir_all(
            mirrored_path
                .parent()
                .expect("runtime mirror should have a parent directory"),
        )
        .expect("runtime mirror parent should exist");
        fs::write(
            &mirrored_path,
            serde_json::to_string_pretty(&mirrored_lease).expect("mirrored lease should serialize"),
        )
        .expect("runtime mirror should write");

        let snapshot = SqlitePlanningAuthorityAdapter::load_runtime_projections(
            repo.worktree_root.to_str().expect("valid worktree path"),
        )
        .expect("runtime projections should load without bootstrap");

        assert!(snapshot.slot_leases.is_empty());
        assert!(snapshot.invalid_slot_leases.is_empty());
        assert!(snapshot.session_details.is_empty());
        assert!(snapshot.distributor_queue_records.is_empty());
    }

    #[test]
    fn authority_open_migrates_legacy_repo_local_runtime_store() {
        let repo = TempGitRepo::new("authority-legacy-runtime-migration");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        let location = adapter
            .resolve_authority_location(repo.worktree_root.to_str().expect("valid worktree path"))
            .expect("authority location should resolve");
        assert!(
            !Path::new(&location.authority_store_path).exists(),
            "new authority store should not exist before migration"
        );
        let legacy_runtime_dir = repo.repo_root.join(".codex-exec-loop/runtime");
        fs::create_dir_all(&legacy_runtime_dir).expect("legacy runtime dir should exist");
        let legacy_store_path = legacy_runtime_dir.join("planning-authority.db");
        let legacy_connection =
            Connection::open(&legacy_store_path).expect("legacy authority store should open");
        legacy_connection
            .execute_batch(
                r#"
                CREATE TABLE authority_metadata (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                CREATE TABLE runtime_slot_leases (
                    slot_id TEXT PRIMARY KEY,
                    updated_at TEXT NOT NULL,
                    content TEXT NOT NULL
                );
                "#,
            )
            .expect("legacy runtime schema should initialize");
        legacy_connection
            .execute(
                "INSERT INTO authority_metadata (key, value) VALUES ('schema_version', ?1)",
                [AUTHORITY_STORE_SCHEMA_VERSION.to_string()],
            )
            .expect("legacy schema version should insert");
        let legacy_lease = ParallelModeSlotLeaseSnapshot::new(
            "slot-1",
            "task-1",
            "Task One",
            "agent-1",
            "akra-agent/slot-1/task-one",
            repo.worktree_root.display().to_string(),
            ParallelModeSlotLeaseState::Running,
            "2026-04-18T10:00:00Z",
            Some("2026-04-18T10:05:00Z".to_string()),
        );
        legacy_connection
            .execute(
                "INSERT INTO runtime_slot_leases (slot_id, updated_at, content) VALUES (?1, ?2, ?3)",
                (
                    legacy_lease.slot_id.as_str(),
                    "2026-04-18T10:05:00Z",
                    serde_json::to_string(&legacy_lease).expect("legacy lease should serialize"),
                ),
            )
            .expect("legacy runtime lease should insert");
        drop(legacy_connection);

        let snapshot = SqlitePlanningAuthorityAdapter::load_runtime_projections(
            repo.worktree_root.to_str().expect("valid worktree path"),
        )
        .expect("runtime projections should load from migrated store");

        assert!(Path::new(&location.authority_store_path).is_file());
        assert_eq!(
            snapshot
                .slot_leases
                .get("slot-1")
                .expect("legacy slot lease should migrate")
                .branch_name,
            "akra-agent/slot-1/task-one"
        );
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

    #[test]
    fn official_refresh_claims_can_reclaim_stale_owner() {
        let repo = TempGitRepo::new("authority-official-stale-claim");
        let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");
        let refresh_order =
            SqlitePlanningAuthorityAdapter::reserve_next_official_refresh_order(workspace_dir)
                .expect("refresh order should reserve");
        assert_eq!(
            SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
                workspace_dir,
                refresh_order,
                "stale-owner",
            )
            .expect("initial claim should acquire"),
            PlanningAuthorityOfficialRefreshClaimStatus::Acquired
        );

        let location = SqlitePlanningAuthorityAdapter::new()
            .resolve_authority_location(workspace_dir)
            .expect("authority location should resolve");
        let connection =
            Connection::open(&location.authority_store_path).expect("authority store should open");
        connection
            .execute(
                "UPDATE runtime_claims
                 SET claimed_at = '2000-01-01T00:00:00Z'
                 WHERE claim_kind = 'official-refresh' AND scope_key = ?1",
                ["official-refresh"],
            )
            .expect("stale official refresh claim should update");

        assert_eq!(
            SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
                workspace_dir,
                refresh_order,
                "fresh-owner",
            )
            .expect("stale claim should be reclaimed"),
            PlanningAuthorityOfficialRefreshClaimStatus::Acquired
        );
    }

    #[test]
    fn distributor_queue_claims_can_reclaim_stale_owner() {
        let repo = TempGitRepo::new("authority-distributor-stale-claim");
        let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");
        assert!(
            SqlitePlanningAuthorityAdapter::try_acquire_distributor_queue_claim(
                workspace_dir,
                "queue-item-stale",
                "stale-owner",
            )
            .expect("initial queue claim should acquire")
        );

        let location = SqlitePlanningAuthorityAdapter::new()
            .resolve_authority_location(workspace_dir)
            .expect("authority location should resolve");
        let connection =
            Connection::open(&location.authority_store_path).expect("authority store should open");
        connection
            .execute(
                "UPDATE runtime_claims
                 SET claimed_at = '2000-01-01T00:00:00Z'
                 WHERE claim_kind = ?1 AND scope_key = ?2",
                ["distributor-queue-head", "queue-item-stale"],
            )
            .expect("stale distributor claim should update");

        assert!(
            SqlitePlanningAuthorityAdapter::try_acquire_distributor_queue_claim(
                workspace_dir,
                "queue-item-stale",
                "fresh-owner",
            )
            .expect("stale queue claim should be reclaimed")
        );
    }
}

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityPort,
};
use crate::domain::parallel_mode::{
    ParallelModeDistributorQueueItem, ParallelModeDistributorSnapshot, ParallelModeQueueItemState,
    ParallelModeReadinessSnapshot, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};

use super::{
    DEFAULT_PUSH_REMOTE_NAME, DISTRIBUTOR_INTEGRATION_BRANCH, PoolRuntimeContext,
    WorkspaceSlotLeaseResolution, branch_exists, branch_is_integrated_into, cleanup_slot,
    command_succeeds, current_branch_name, current_timestamp, ensure_directory_exists,
    inspect_slot_git_status, lease_session_key, load_pool_runtime_context, reconcile_pool_board,
    record_cleaned_session_detail, record_cleanup_pending_session_detail,
    record_distributor_failed_session_detail, record_integrating_session_detail,
    record_merge_pending_session_detail, record_merge_queued_session_detail,
    record_pr_pending_session_detail, record_pushing_session_detail, resolve_workspace_head_sha,
    resolve_workspace_slot_lease, run_command, short_sha, write_slot_lease,
};

pub(super) type ParallelModeDistributorQueueRecord = PlanningAuthorityDistributorQueueRecord;

mod delivery;
mod queue_keys;
mod snapshot;

use self::delivery::process_distributor_queue_record;
use self::queue_keys::{distributor_claim_owner_token, sanitize_runtime_record_key};
use self::snapshot::{
    build_distributor_snapshot_from_context, build_placeholder_distributor_snapshot,
};

#[derive(Clone)]
pub(super) struct ParallelModeDistributorService {
    github_automation: Arc<dyn GithubAutomationPort>,
    planning_authority: Arc<dyn PlanningAuthorityPort>,
}

struct DistributorQueueHeadClaimPermit {
    planning_authority: Arc<dyn PlanningAuthorityPort>,
    workspace_directory: String,
    queue_item_id: String,
    owner_token: String,
}

impl Drop for DistributorQueueHeadClaimPermit {
    fn drop(&mut self) {
        let _ = self.planning_authority.release_distributor_queue_claim(
            &self.workspace_directory,
            &self.queue_item_id,
            &self.owner_token,
        );
    }
}

impl ParallelModeDistributorService {
    pub(super) fn with_planning_authority(
        github_automation: Arc<dyn GithubAutomationPort>,
        planning_authority: Arc<dyn PlanningAuthorityPort>,
    ) -> Self {
        Self {
            github_automation,
            planning_authority,
        }
    }

    pub(super) fn build_snapshot(
        &self,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    ) -> ParallelModeDistributorSnapshot {
        match readiness_snapshot {
            Some(snapshot) if mode_enabled && snapshot.allows_parallel_mode() => {
                self.inspect_snapshot(workspace_dir)
            }
            Some(_) if mode_enabled => build_placeholder_distributor_snapshot(
                "paused",
                "distributor waits for readiness recovery before queue processing",
            ),
            None if mode_enabled => build_placeholder_distributor_snapshot(
                "pending",
                "rerun readiness before distributor state can be trusted",
            ),
            Some(_) => build_placeholder_distributor_snapshot(
                "inactive",
                "enable parallel mode to surface live distributor activity",
            ),
            None => build_placeholder_distributor_snapshot("inactive", "parallel mode is off"),
        }
    }

    pub(super) fn enqueue_workspace_commit_ready_result(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeDistributorQueueItem>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        let session_key = lease_session_key(&resolution.lease);
        let detail = resolution
            .context
            .session_details
            .iter()
            .find(|detail| detail.session_key == session_key)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "slot `{}` does not have a persisted session detail record",
                    resolution.lease.slot_id
                )
            })?;
        if !matches!(
            detail.state_label.as_str(),
            "commit_ready" | "merge_queued" | "integrating"
        ) {
            return Ok(None);
        }

        if let Some(existing) = find_distributor_queue_record_by_session_key(
            &resolution.context.distributor_queue_records,
            &session_key,
        ) {
            return Ok(Some(existing.display_item()));
        }

        let commit_sha =
            resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
                format!(
                    "slot `{}` workspace head could not be resolved for distributor enqueue",
                    resolution.lease.slot_id
                )
            })?;
        let github_capabilities = self
            .github_automation
            .inspect_capabilities(&resolution.context.repo_root);
        let timestamp = current_timestamp();
        let record = ParallelModeDistributorQueueRecord {
            queue_item_id: distributor_queue_item_id(&resolution.lease, &timestamp),
            queue_order_key: queue_order_key_from_timestamp(&timestamp),
            session_key,
            root_turn_id: None,
            slot_id: resolution.lease.slot_id.clone(),
            agent_id: resolution.lease.agent_id.clone(),
            task_id: resolution.lease.task_id.clone(),
            task_title: resolution.lease.task_title.clone(),
            source_branch: resolution.lease.branch_name.clone(),
            source_commit_sha: commit_sha.clone(),
            branch_name: resolution.lease.branch_name.clone(),
            worktree_path: resolution.lease.worktree_path.clone(),
            original_commit_sha: Some(commit_sha.clone()),
            commit_sha,
            planning_refresh_state: "done".to_string(),
            integration_state: "queued".to_string(),
            conflict_files: Vec::new(),
            recovery_note: None,
            validation_summary: detail.validation_summary.clone(),
            authority_refresh_outcome: detail.authority_refresh_outcome.clone(),
            github_capabilities: Some(github_capabilities),
            pull_request_number: None,
            pull_request_url: None,
            queue_state: ParallelModeQueueItemState::Queued,
            integration_note: "commit-ready result accepted into distributor queue".to_string(),
            enqueued_at: timestamp.clone(),
            updated_at: timestamp,
        };
        write_distributor_queue_record(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &record,
        )?;
        let _ = record_merge_queued_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
        );

        Ok(Some(record.display_item()))
    }

    pub(super) fn process_queue(&self, workspace_dir: &str) -> Result<Vec<String>, String> {
        let _ = reconcile_pool_board(self.planning_authority.as_ref(), workspace_dir);
        let context = self.recover_runtime_state(workspace_dir)?;
        let mut records = context.distributor_queue_records.clone();
        let Some(head_index) = records
            .iter()
            .position(|record| record.queue_state != ParallelModeQueueItemState::Done)
        else {
            return Ok(Vec::new());
        };

        let head = &mut records[head_index];
        if matches!(
            head.queue_state,
            ParallelModeQueueItemState::Blocked | ParallelModeQueueItemState::Failed
        ) {
            return Ok(vec![format!(
                "distributor queue head is blocked / agent: {} / task: {} / {}",
                head.agent_id, head.task_id, head.integration_note
            )]);
        }

        let Some(_claim_permit) =
            self.acquire_queue_head_claim(workspace_dir, &head.queue_item_id)?
        else {
            return Ok(vec![format!(
                "distributor queue head is already claimed by another process / agent: {} / task: {}",
                head.agent_id, head.task_id
            )]);
        };

        process_distributor_queue_record(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            head,
            self.github_automation.as_ref(),
        )
    }

    fn inspect_snapshot(&self, workspace_dir: &str) -> ParallelModeDistributorSnapshot {
        match load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir) {
            Ok(context) => build_distributor_snapshot_from_context(&context),
            Err((_, detail)) => build_placeholder_distributor_snapshot(
                "unavailable",
                format!("distributor snapshot unavailable / {detail}"),
            ),
        }
    }

    fn acquire_queue_head_claim(
        &self,
        workspace_dir: &str,
        queue_item_id: &str,
    ) -> Result<Option<DistributorQueueHeadClaimPermit>, String> {
        let owner_token = distributor_claim_owner_token(queue_item_id);
        let acquired = self
            .planning_authority
            .try_acquire_distributor_queue_claim(workspace_dir, queue_item_id, &owner_token)
            .map_err(|error| error.to_string())?;
        if !acquired {
            return Ok(None);
        }

        Ok(Some(DistributorQueueHeadClaimPermit {
            planning_authority: self.planning_authority.clone(),
            workspace_directory: workspace_dir.to_string(),
            queue_item_id: queue_item_id.to_string(),
            owner_token,
        }))
    }

    pub(super) fn recover_runtime_state(
        &self,
        workspace_dir: &str,
    ) -> Result<PoolRuntimeContext, String> {
        let mut context =
            load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
                .map_err(|(_, detail)| detail.to_string())?;

        for index in 0..context.distributor_queue_records.len() {
            let mut record = context.distributor_queue_records[index].clone();
            let matching_lease = matching_lease_for_queue_record(&context, &record).cloned();
            recover_mismatched_slot_worktree(
                self.planning_authority.as_ref(),
                &context.repo_root,
                &context.pool_root,
                matching_lease.as_ref(),
                &mut record,
            )?;
            recover_retryable_blocked_queue_record(
                self.planning_authority.as_ref(),
                &context.repo_root,
                &context.pool_root,
                matching_lease.as_ref(),
                &mut record,
            )?;
            context.distributor_queue_records[index] = record.clone();

            if matches!(
                record.queue_state,
                ParallelModeQueueItemState::Idle
                    | ParallelModeQueueItemState::Done
                    | ParallelModeQueueItemState::Blocked
                    | ParallelModeQueueItemState::Failed
            ) {
                continue;
            }

            if !Path::new(&record.worktree_path).exists() {
                let _ = block_distributor_queue_record(
                    self.planning_authority.as_ref(),
                    &context.repo_root,
                    &context.pool_root,
                    matching_lease.as_ref(),
                    &mut record,
                    "recovered after restart: source worktree is missing; distributor cannot continue"
                        .to_string(),
                )?;
                context.distributor_queue_records[index] = record;
                continue;
            }

            if branch_is_integrated_into(
                &context.repo_root,
                &record.branch_name,
                DISTRIBUTOR_INTEGRATION_BRANCH,
            ) {
                recover_integrated_queue_record(
                    self.planning_authority.as_ref(),
                    &context,
                    matching_lease.as_ref(),
                    &mut record,
                )?;
                context.distributor_queue_records[index] = record;
                continue;
            }

            if let Some(pr_number) = record.pull_request_number
                && let Ok(pull_request) = self
                    .github_automation
                    .inspect_pull_request(&context.repo_root, pr_number)
            {
                record.pull_request_url = Some(pull_request.url.clone());
                if !pull_request.state.eq_ignore_ascii_case("open") {
                    let _ = block_distributor_queue_record(
                        self.planning_authority.as_ref(),
                        &context.repo_root,
                        &context.pool_root,
                        matching_lease.as_ref(),
                        &mut record,
                        format!(
                            "recovered after restart: pull request #{pr_number} is `{}` before integration",
                            pull_request.state
                        ),
                    )?;
                    context.distributor_queue_records[index] = record;
                    continue;
                }
                if pull_request.is_draft {
                    let _ = block_distributor_queue_record(
                        self.planning_authority.as_ref(),
                        &context.repo_root,
                        &context.pool_root,
                        matching_lease.as_ref(),
                        &mut record,
                        format!(
                            "recovered after restart: pull request #{pr_number} is still a draft"
                        ),
                    )?;
                    context.distributor_queue_records[index] = record;
                    continue;
                }
                write_distributor_queue_record(
                    self.planning_authority.as_ref(),
                    &context.repo_root,
                    &context.pool_root,
                    &record,
                )?;
            }

            context.distributor_queue_records[index] = record;
        }

        Ok(context)
    }
}

fn recover_mismatched_slot_worktree(
    planning_authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    pool_root: &Path,
    matching_lease: Option<&ParallelModeSlotLeaseSnapshot>,
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
    let Some(lease) = matching_lease else {
        return Ok(());
    };
    if record.queue_state != ParallelModeQueueItemState::Blocked {
        return Ok(());
    }
    if record.branch_name != lease.branch_name || record.worktree_path != lease.worktree_path {
        return Ok(());
    }
    if !Path::new(&record.worktree_path).exists() {
        return Ok(());
    }
    if !branch_exists(repo_root, &lease.branch_name) {
        return Ok(());
    }
    if current_branch_name(Path::new(&record.worktree_path)).as_deref()
        == Some(lease.branch_name.as_str())
    {
        return Ok(());
    }
    let Some(slot_status) = inspect_slot_git_status(Path::new(&record.worktree_path)) else {
        return Ok(());
    };
    if !slot_status.is_clean_baseline() {
        return Ok(());
    }
    if !command_succeeds(
        "git",
        [
            "-C",
            record.worktree_path.as_str(),
            "checkout",
            lease.branch_name.as_str(),
        ],
    ) {
        return Ok(());
    }

    record.queue_state = ParallelModeQueueItemState::Queued;
    record.integration_state = "queued".to_string();
    record.recovery_note =
        Some("recovered mismatched clean slot worktree checkout before retry".to_string());
    record.integration_note =
        "recovered clean slot worktree checkout and queued distributor retry".to_string();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(planning_authority, repo_root, pool_root, record)?;
    Ok(())
}

fn recover_retryable_blocked_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    pool_root: &Path,
    matching_lease: Option<&ParallelModeSlotLeaseSnapshot>,
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
    let Some(lease) = matching_lease else {
        return Ok(());
    };
    if record.queue_state != ParallelModeQueueItemState::Blocked {
        return Ok(());
    }
    if !is_retryable_distributor_block(&record.integration_note) {
        return Ok(());
    }
    if record.branch_name != lease.branch_name || record.worktree_path != lease.worktree_path {
        return Ok(());
    }
    if current_branch_name(Path::new(&record.worktree_path)).as_deref()
        != Some(lease.branch_name.as_str())
    {
        return Ok(());
    }
    let Some(slot_status) = inspect_slot_git_status(Path::new(&record.worktree_path)) else {
        return Ok(());
    };
    if slot_status.has_pending_operation {
        return Ok(());
    }

    record.queue_state = ParallelModeQueueItemState::Queued;
    record.integration_state = "queued".to_string();
    record.recovery_note = Some("recovered retryable distributor block before retry".to_string());
    record.integration_note = "recovered retryable distributor block and queued retry".to_string();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(planning_authority, repo_root, pool_root, record)?;
    Ok(())
}

fn is_retryable_distributor_block(detail: &str) -> bool {
    detail.contains("pull request ensure failed")
        || detail.contains("could not be inspected")
        || detail.contains("could not cherry-pick")
        || detail.contains("integration worktree must be clean before cherry-pick delivery")
        || detail.contains("source branch was pushed but GitHub automation is unavailable")
}

fn matching_lease_for_queue_record<'a>(
    context: &'a PoolRuntimeContext,
    record: &ParallelModeDistributorQueueRecord,
) -> Option<&'a ParallelModeSlotLeaseSnapshot> {
    context
        .slot_leases
        .values()
        .find(|lease| lease_session_key(lease) == record.session_key)
        .or_else(|| {
            context.slot_leases.values().find(|lease| {
                lease.branch_name == record.branch_name
                    && lease.worktree_path == record.worktree_path
            })
        })
}

fn recover_integrated_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    context: &PoolRuntimeContext,
    matching_lease: Option<&ParallelModeSlotLeaseSnapshot>,
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
    if let Some(lease) = matching_lease {
        if lease.state == ParallelModeSlotLeaseState::Running {
            let mut cleanup_pending_lease = lease.clone();
            cleanup_pending_lease.state = ParallelModeSlotLeaseState::CleanupPending;
            write_slot_lease(
                planning_authority,
                &context.repo_root,
                &context.pool_root,
                &cleanup_pending_lease,
            )?;
            let _ = record_cleanup_pending_session_detail(
                planning_authority,
                &context.repo_root,
                &context.pool_root,
                &cleanup_pending_lease,
            );
        }
    } else if !branch_exists(&context.repo_root, &record.branch_name) {
        record.queue_state = ParallelModeQueueItemState::Done;
        record.integration_note = format!(
            "recovered after restart: branch is already integrated into {DISTRIBUTOR_INTEGRATION_BRANCH} and slot cleanup completed"
        );
        record.updated_at = current_timestamp();
        write_distributor_queue_record(
            planning_authority,
            &context.repo_root,
            &context.pool_root,
            record,
        )?;
        return Ok(());
    }

    record.queue_state = ParallelModeQueueItemState::Cleaning;
    record.integration_note = format!(
        "recovered after restart: branch is already integrated into {DISTRIBUTOR_INTEGRATION_BRANCH} and cleanup is pending"
    );
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        &context.repo_root,
        &context.pool_root,
        record,
    )?;
    Ok(())
}

fn distributor_queue_root(pool_root: &Path) -> PathBuf {
    pool_root.join(".distributor-queue")
}

fn distributor_queue_record_path(pool_root: &Path, queue_item_id: &str) -> PathBuf {
    distributor_queue_root(pool_root).join(format!("{queue_item_id}.json"))
}

fn distributor_queue_item_id(lease: &ParallelModeSlotLeaseSnapshot, timestamp: &str) -> String {
    sanitize_runtime_record_key(&format!(
        "{}-{}-{}",
        lease.slot_id, lease.agent_id, timestamp
    ))
}

fn queue_order_key_from_timestamp(timestamp: &str) -> u64 {
    timestamp
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .take(20)
        .collect::<String>()
        .parse::<u64>()
        .unwrap_or(0)
}

#[cfg(test)]
pub(super) fn load_distributor_queue_records(
    pool_root: &Path,
) -> Vec<ParallelModeDistributorQueueRecord> {
    let queue_root = distributor_queue_root(pool_root);
    let Ok(entries) = fs::read_dir(queue_root) else {
        return Vec::new();
    };

    let mut records = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|content| {
            serde_json::from_str::<ParallelModeDistributorQueueRecord>(&content).ok()
        })
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        left.enqueued_at
            .cmp(&right.enqueued_at)
            .then_with(|| left.queue_item_id.cmp(&right.queue_item_id))
    });
    records
}

fn find_distributor_queue_record_by_session_key(
    queue_records: &[ParallelModeDistributorQueueRecord],
    session_key: &str,
) -> Option<ParallelModeDistributorQueueRecord> {
    queue_records
        .iter()
        .find(|record| record.session_key == session_key)
        .cloned()
}

fn write_distributor_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    record: &ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
    planning_authority
        .upsert_runtime_distributor_queue_record(workspace_dir, record)
        .map_err(|error| {
            format!(
                "failed to store distributor queue record `{}`: {error}",
                record.queue_item_id
            )
        })?;

    let queue_root = distributor_queue_root(pool_root);
    ensure_directory_exists(&queue_root)
        .map_err(|error| format!("failed to create distributor queue directory: {error}"))?;

    let path = distributor_queue_record_path(pool_root, &record.queue_item_id);
    let temp_path = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(record)
        .map_err(|error| format!("failed to serialize distributor queue record: {error}"))?;
    fs::write(&temp_path, body).map_err(|error| {
        format!(
            "failed to write temporary distributor queue record `{}`: {error}",
            record.queue_item_id
        )
    })?;
    fs::rename(&temp_path, &path).map_err(|error| {
        format!(
            "failed to persist distributor queue record `{}`: {error}",
            record.queue_item_id
        )
    })
}

fn block_distributor_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: Option<&ParallelModeSlotLeaseSnapshot>,
    record: &mut ParallelModeDistributorQueueRecord,
    failure_detail: String,
) -> Result<String, String> {
    record.queue_state = ParallelModeQueueItemState::Blocked;
    record.integration_state = "blocked".to_string();
    if record.recovery_note.is_none() {
        record.recovery_note = Some(failure_detail.clone());
    }
    record.integration_note = failure_detail.clone();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(planning_authority, workspace_dir, pool_root, record)?;
    if let Some(lease) = lease {
        let _ = record_distributor_failed_session_detail(
            planning_authority,
            workspace_dir,
            pool_root,
            lease,
            &failure_detail,
        );
    }

    Ok(format!(
        "distributor queue head blocked / agent: {} / {}",
        record.agent_id, failure_detail
    ))
}

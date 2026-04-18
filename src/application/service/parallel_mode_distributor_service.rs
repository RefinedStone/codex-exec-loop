use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubAutomationPort,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeCompletionFeedEntry,
    ParallelModeDistributorQueueItem, ParallelModeDistributorSnapshot, ParallelModeQueueItemState,
    ParallelModeReadinessSnapshot, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};

use super::parallel_mode_supervisor_service::selected_runtime_session_detail;
use super::{
    AKRA_BRANCH, DEFAULT_PUSH_REMOTE_NAME, PoolRuntimeContext, WorkspaceSlotLeaseResolution,
    branch_is_integrated_into_akra, cleanup_slot, command_succeeds, current_branch_name,
    current_timestamp, ensure_directory_exists, inspect_slot_git_status, lease_session_key,
    load_agent_session_detail_records, load_pool_runtime_context, read_agent_session_detail_record,
    record_cleaned_session_detail, record_cleanup_pending_session_detail,
    record_distributor_failed_session_detail, record_integrating_session_detail,
    record_merge_pending_session_detail, record_merge_queued_session_detail,
    record_pr_pending_session_detail, record_pushing_session_detail, resolve_workspace_head_sha,
    resolve_workspace_slot_lease, short_sha, write_slot_lease,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct ParallelModeDistributorQueueRecord {
    queue_item_id: String,
    pub(super) session_key: String,
    agent_id: String,
    task_id: String,
    task_title: String,
    branch_name: String,
    worktree_path: String,
    pub(super) commit_sha: String,
    #[serde(default)]
    pub(super) original_commit_sha: Option<String>,
    validation_summary: String,
    ledger_refresh_outcome: String,
    #[serde(default)]
    github_capabilities: Option<GithubAutomationCapabilities>,
    #[serde(default)]
    pull_request_number: Option<u64>,
    #[serde(default)]
    pull_request_url: Option<String>,
    pub(super) queue_state: ParallelModeQueueItemState,
    pub(super) integration_note: String,
    enqueued_at: String,
    updated_at: String,
}

impl ParallelModeDistributorQueueRecord {
    fn display_item(&self) -> ParallelModeDistributorQueueItem {
        ParallelModeDistributorQueueItem::new(
            self.agent_id.clone(),
            self.task_title.clone(),
            self.queue_state,
            self.branch_name.clone(),
            short_sha(&self.commit_sha),
            self.integration_note.clone(),
        )
    }
}

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
        let Some(resolution) = resolve_workspace_slot_lease(workspace_dir)? else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        let session_key = lease_session_key(&resolution.lease);
        let detail = read_agent_session_detail_record(&resolution.context.pool_root, &session_key)
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
            &resolution.context.pool_root,
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
            session_key,
            agent_id: resolution.lease.agent_id.clone(),
            task_id: resolution.lease.task_id.clone(),
            task_title: resolution.lease.task_title.clone(),
            branch_name: resolution.lease.branch_name.clone(),
            worktree_path: resolution.lease.worktree_path.clone(),
            original_commit_sha: Some(commit_sha.clone()),
            commit_sha,
            validation_summary: detail.validation_summary.clone(),
            ledger_refresh_outcome: detail.ledger_refresh_outcome.clone(),
            github_capabilities: Some(github_capabilities),
            pull_request_number: None,
            pull_request_url: None,
            queue_state: ParallelModeQueueItemState::Queued,
            integration_note: "commit-ready result accepted into distributor queue".to_string(),
            enqueued_at: timestamp.clone(),
            updated_at: timestamp,
        };
        write_distributor_queue_record(&resolution.context.pool_root, &record)?;
        let _ =
            record_merge_queued_session_detail(&resolution.context.pool_root, &resolution.lease);

        Ok(Some(record.display_item()))
    }

    pub(super) fn process_queue(&self, workspace_dir: &str) -> Result<Vec<String>, String> {
        let context =
            load_pool_runtime_context(workspace_dir).map_err(|(_, detail)| detail.to_string())?;
        let mut records = load_distributor_queue_records(&context.pool_root);
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

        process_distributor_queue_record(&context.pool_root, head, self.github_automation.as_ref())
    }

    fn inspect_snapshot(&self, workspace_dir: &str) -> ParallelModeDistributorSnapshot {
        match load_pool_runtime_context(workspace_dir) {
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
}

fn distributor_claim_owner_token(queue_item_id: &str) -> String {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "distributor-queue-head-{}-{}-{unique_suffix}",
        std::process::id(),
        sanitize_runtime_record_key(queue_item_id)
    )
}

fn build_distributor_snapshot_from_context(
    context: &PoolRuntimeContext,
) -> ParallelModeDistributorSnapshot {
    let history = load_agent_session_detail_records(&context.pool_root);
    let queue_records = load_distributor_queue_records(&context.pool_root);
    let queue_items = queue_records
        .iter()
        .filter(|record| record.queue_state.is_active())
        .map(ParallelModeDistributorQueueRecord::display_item)
        .collect::<Vec<_>>();
    let completion_feed = build_distributor_completion_feed(&history);

    if let Some(queue_head) = active_distributor_queue_head(&queue_records) {
        return ParallelModeDistributorSnapshot::new(
            queue_items,
            completion_feed,
            queue_head.queue_state.label(),
            queue_head.integration_note.clone(),
        )
        .with_head_blocked_detail(blocked_head_detail(queue_head))
        .with_head_rebase_provenance(rebase_provenance_label(queue_head));
    }

    let Some(detail) = selected_runtime_session_detail(context, &history, &queue_records) else {
        return build_placeholder_distributor_snapshot(
            ParallelModeQueueItemState::Idle.label(),
            "no distributor queue items are waiting",
        );
    };

    let (head_summary, note) = match detail.state_label.as_str() {
        "reported_complete" => ("reported".to_string(), detail.latest_summary.clone()),
        "ledger_refreshing" => (
            "ledger refreshing".to_string(),
            detail.ledger_refresh_outcome.clone(),
        ),
        "commit_ready" => (
            "official".to_string(),
            detail.distributor_outcome.clone().unwrap_or_else(|| {
                "commit-ready result is waiting for distributor enqueue".to_string()
            }),
        ),
        "failed" if detail_has_history_state(&detail, "reported_complete") => {
            ("blocked".to_string(), detail.ledger_refresh_outcome.clone())
        }
        _ => (
            ParallelModeQueueItemState::Idle.label().to_string(),
            "no distributor queue items are waiting".to_string(),
        ),
    };

    ParallelModeDistributorSnapshot::new(queue_items, completion_feed, head_summary, note)
        .with_head_rebase_provenance(history_rebase_provenance(&detail))
}

fn active_distributor_queue_head(
    queue_records: &[ParallelModeDistributorQueueRecord],
) -> Option<&ParallelModeDistributorQueueRecord> {
    queue_records
        .iter()
        .find(|record| record.queue_state.is_active())
}

fn blocked_head_detail(record: &ParallelModeDistributorQueueRecord) -> Option<String> {
    (record.queue_state == ParallelModeQueueItemState::Blocked)
        .then(|| record.integration_note.clone())
}

fn rebase_provenance_label(record: &ParallelModeDistributorQueueRecord) -> Option<String> {
    let original_commit_sha = record
        .original_commit_sha
        .as_deref()
        .filter(|commit| !commit.trim().is_empty())
        .unwrap_or(record.commit_sha.as_str());
    (original_commit_sha != record.commit_sha).then(|| {
        format!(
            "rebased {} -> {} onto `{AKRA_BRANCH}`",
            short_sha(original_commit_sha),
            short_sha(&record.commit_sha)
        )
    })
}

fn history_rebase_provenance(detail: &ParallelModeAgentSessionDetailSnapshot) -> Option<String> {
    detail
        .history
        .iter()
        .rev()
        .find(|entry| entry.state_label == "integrating" && entry.summary.starts_with("rebased "))
        .map(|entry| entry.summary.clone())
}

fn detail_has_history_state(
    detail: &ParallelModeAgentSessionDetailSnapshot,
    state_label: &str,
) -> bool {
    detail
        .history
        .iter()
        .any(|entry| entry.state_label == state_label)
}

fn build_distributor_completion_feed(
    history: &[ParallelModeAgentSessionDetailSnapshot],
) -> Vec<ParallelModeCompletionFeedEntry> {
    vec![
        ParallelModeCompletionFeedEntry::new(
            "reported",
            latest_history_summary_across_records(history, &["reported_complete"])
                .unwrap_or_else(|| "no agent results reported yet".to_string()),
        ),
        ParallelModeCompletionFeedEntry::new(
            "ledger refreshing",
            latest_history_summary_across_records(history, &["ledger_refreshing"])
                .unwrap_or_else(|| "no official refresh workers are active".to_string()),
        ),
        ParallelModeCompletionFeedEntry::new(
            "official",
            latest_history_summary_across_records(history, &["commit_ready"])
                .unwrap_or_else(|| "nothing is queued for merge".to_string()),
        ),
        ParallelModeCompletionFeedEntry::new(
            "merge queued",
            latest_history_summary_across_records(
                history,
                &[
                    "merge_queued",
                    "pushing",
                    "pr_pending",
                    "merge_pending",
                    "integrating",
                ],
            )
            .unwrap_or_else(|| "no distributor queue items are waiting".to_string()),
        ),
        ParallelModeCompletionFeedEntry::new(
            "merged",
            latest_history_summary_across_records(history, &["merged", "cleaned"])
                .unwrap_or_else(|| "nothing has been integrated into akra yet".to_string()),
        ),
    ]
}

fn latest_history_summary_across_records(
    history: &[ParallelModeAgentSessionDetailSnapshot],
    state_labels: &[&str],
) -> Option<String> {
    history
        .iter()
        .flat_map(|detail| detail.history.iter())
        .filter(|entry| state_labels.contains(&entry.state_label.as_str()))
        .max_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.summary.cmp(&right.summary))
        })
        .map(|entry| entry.summary.clone())
}

fn build_placeholder_distributor_snapshot(
    head_summary: impl Into<String>,
    note: impl Into<String>,
) -> ParallelModeDistributorSnapshot {
    ParallelModeDistributorSnapshot::new(
        Vec::new(),
        vec![
            ParallelModeCompletionFeedEntry::new("reported", "no agent results reported yet"),
            ParallelModeCompletionFeedEntry::new(
                "ledger refreshing",
                "no official refresh workers are active",
            ),
            ParallelModeCompletionFeedEntry::new("official", "nothing is queued for merge"),
            ParallelModeCompletionFeedEntry::new(
                "merge queued",
                "no distributor queue items are waiting",
            ),
            ParallelModeCompletionFeedEntry::new(
                "merged",
                "nothing has been integrated into akra yet",
            ),
        ],
        head_summary,
        note,
    )
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

fn sanitize_runtime_record_key(value: &str) -> String {
    let mut key = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            key.push(ch);
        } else {
            key.push('_');
        }
    }
    key
}

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
    pool_root: &Path,
    session_key: &str,
) -> Option<ParallelModeDistributorQueueRecord> {
    load_distributor_queue_records(pool_root)
        .into_iter()
        .find(|record| record.session_key == session_key)
}

fn write_distributor_queue_record(
    pool_root: &Path,
    record: &ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
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

fn process_distributor_queue_record(
    pool_root: &Path,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
) -> Result<Vec<String>, String> {
    if !Path::new(&record.worktree_path).exists() {
        return Ok(vec![block_distributor_queue_record(
            pool_root,
            None,
            record,
            "source worktree is missing; distributor cannot continue".to_string(),
        )?]);
    }

    let resolution = match resolve_workspace_slot_lease(&record.worktree_path) {
        Ok(Some(resolution)) => resolution,
        Ok(None) => {
            return Ok(vec![block_distributor_queue_record(
                pool_root,
                None,
                record,
                "slot lease disappeared before distributor integration".to_string(),
            )?]);
        }
        Err(error) => {
            return Ok(vec![block_distributor_queue_record(
                pool_root,
                None,
                record,
                format!("slot lease could not be resolved for distributor delivery: {error}"),
            )?]);
        }
    };

    let mut notices = Vec::new();
    if matches!(
        record.queue_state,
        ParallelModeQueueItemState::Queued
            | ParallelModeQueueItemState::Pushing
            | ParallelModeQueueItemState::PrPending
            | ParallelModeQueueItemState::MergePending
            | ParallelModeQueueItemState::Integrating
    ) {
        notices.push(distributor_push_source_branch(
            &resolution,
            record,
            github_automation,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            return Ok(notices);
        }

        notices.push(distributor_ensure_pull_request(
            &resolution,
            record,
            github_automation,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            return Ok(notices);
        }

        notices.push(distributor_check_pull_request_merge_readiness(
            &resolution,
            record,
            github_automation,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            return Ok(notices);
        }

        notices.push(distributor_integrate_branch(
            &resolution,
            record,
            github_automation,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            return Ok(notices);
        }
    }

    let cleanup_notice = distributor_cleanup_integrated_slot(&resolution, record)?;
    notices.push(cleanup_notice);
    Ok(notices)
}

fn distributor_push_source_branch(
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
) -> Result<String, String> {
    let repo_root = resolution.context.repo_root.clone();
    let capabilities = github_automation.inspect_capabilities(&repo_root);
    record.github_capabilities = Some(capabilities.clone());
    if !capabilities.push_ready() {
        return block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "push capability is unavailable for distributor delivery: {}",
                capabilities.push_remote.summary()
            ),
        );
    }

    record.queue_state = ParallelModeQueueItemState::Pushing;
    record.integration_note = format!(
        "distributor is pushing `{}` to `{DEFAULT_PUSH_REMOTE_NAME}`",
        record.branch_name
    );
    record.updated_at = current_timestamp();
    write_distributor_queue_record(&resolution.context.pool_root, record)?;
    let _ = record_pushing_session_detail(
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    if let Err(error) = github_automation.push_branch(&repo_root, &record.branch_name, false) {
        return block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "source branch `{}` could not be pushed to `{DEFAULT_PUSH_REMOTE_NAME}`: {error}",
                record.branch_name
            ),
        );
    }

    record.integration_note = format!(
        "source branch pushed to `{DEFAULT_PUSH_REMOTE_NAME}` and is waiting for pull request ensure"
    );
    record.updated_at = current_timestamp();
    write_distributor_queue_record(&resolution.context.pool_root, record)?;

    Ok(format!(
        "distributor pushed source branch / agent: {} / branch: {}",
        record.agent_id, record.branch_name
    ))
}

fn distributor_ensure_pull_request(
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
) -> Result<String, String> {
    let repo_root = resolution.context.repo_root.clone();
    let capabilities = github_automation.inspect_capabilities(&repo_root);
    record.github_capabilities = Some(capabilities.clone());
    if !capabilities.github_ready() {
        let capability_summary = if capabilities.gh_binary.state
            != crate::domain::parallel_mode::ParallelModeCapabilityState::Ready
        {
            capabilities.gh_binary.summary()
        } else {
            capabilities.gh_auth.summary()
        };
        return block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "source branch was pushed but GitHub automation is unavailable: {capability_summary}"
            ),
        );
    }

    record.queue_state = ParallelModeQueueItemState::PrPending;
    record.integration_note =
        "source branch pushed and pull request ensure is in progress".to_string();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(&resolution.context.pool_root, record)?;
    let _ = record_pr_pending_session_detail(
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    let pull_request = match github_automation.ensure_pull_request(
        &repo_root,
        AKRA_BRANCH,
        &record.branch_name,
        &build_distributor_pull_request_title(record),
        &build_distributor_pull_request_body(record),
    ) {
        Ok(pull_request) => pull_request,
        Err(error) => {
            return block_distributor_queue_record(
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!(
                    "pull request ensure failed for `{}`: {error}",
                    record.branch_name
                ),
            );
        }
    };

    record.pull_request_number = Some(pull_request.number);
    record.pull_request_url = Some(pull_request.url.clone());
    record.integration_note = format!(
        "pull request #{} is open for `{}`",
        pull_request.number, record.branch_name
    );
    record.updated_at = current_timestamp();
    write_distributor_queue_record(&resolution.context.pool_root, record)?;

    Ok(format!(
        "distributor ensured pull request / agent: {} / pr: #{}",
        record.agent_id, pull_request.number
    ))
}

fn distributor_check_pull_request_merge_readiness(
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
) -> Result<String, String> {
    let Some(pr_number) = record.pull_request_number else {
        return block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            "pull request metadata is missing after PR ensure".to_string(),
        );
    };

    record.queue_state = ParallelModeQueueItemState::MergePending;
    record.integration_note =
        format!("pull request #{pr_number} is open and merge readiness is being checked");
    record.updated_at = current_timestamp();
    write_distributor_queue_record(&resolution.context.pool_root, record)?;
    let _ = record_merge_pending_session_detail(
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    let repo_root = resolution.context.repo_root.clone();
    let pull_request = match github_automation.inspect_pull_request(&repo_root, pr_number) {
        Ok(pull_request) => pull_request,
        Err(error) => {
            return block_distributor_queue_record(
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!("pull request #{pr_number} could not be inspected: {error}"),
            );
        }
    };

    record.pull_request_url = Some(pull_request.url.clone());
    if !pull_request.state.eq_ignore_ascii_case("open") {
        return block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "pull request #{} is not open (`{}`)",
                pull_request.number, pull_request.state
            ),
        );
    }
    if pull_request.is_draft {
        return block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!("pull request #{} is still a draft", pull_request.number),
        );
    }
    if pull_request.base_branch != AKRA_BRANCH {
        return block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "pull request #{} targets `{}` instead of `{AKRA_BRANCH}`",
                pull_request.number, pull_request.base_branch
            ),
        );
    }
    if pull_request.head_branch != record.branch_name {
        return block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "pull request #{} head drifted from `{}` to `{}`",
                pull_request.number, record.branch_name, pull_request.head_branch
            ),
        );
    }

    record.integration_note = format!(
        "pull request #{} is open and ready for integration into `{AKRA_BRANCH}`",
        pull_request.number
    );
    record.updated_at = current_timestamp();
    write_distributor_queue_record(&resolution.context.pool_root, record)?;

    Ok(format!(
        "distributor verified pull request readiness / agent: {} / pr: #{}",
        record.agent_id, pull_request.number
    ))
}

fn distributor_integrate_branch(
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
) -> Result<String, String> {
    let slot_status = inspect_slot_git_status(&resolution.workspace_path).ok_or_else(|| {
        format!(
            "slot `{}` git status could not be inspected for distributor delivery",
            resolution.lease.slot_id
        )
    })?;
    if slot_status.has_pending_operation {
        return block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "slot `{}` has pending merge or rebase metadata and cannot be integrated",
                resolution.lease.slot_id
            ),
        );
    }

    if current_branch_name(&resolution.workspace_path).as_deref()
        != Some(record.branch_name.as_str())
    {
        return block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "slot `{}` is no longer checked out to `{}`",
                resolution.lease.slot_id, record.branch_name
            ),
        );
    }

    let current_head = resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
        format!(
            "slot `{}` workspace head could not be resolved for distributor delivery",
            resolution.lease.slot_id
        )
    })?;
    if current_head != record.commit_sha {
        return block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "branch head drifted from expected commit `{}` to `{}`",
                short_sha(&record.commit_sha),
                short_sha(&current_head)
            ),
        );
    }

    record.queue_state = ParallelModeQueueItemState::Integrating;
    record.integration_note = match record.pull_request_number {
        Some(pr_number) => format!(
            "pull request #{pr_number} is ready and distributor is integrating the queued branch into akra"
        ),
        None => "distributor is integrating the queued branch into akra".to_string(),
    };
    record.updated_at = current_timestamp();
    write_distributor_queue_record(&resolution.context.pool_root, record)?;
    let _ = record_integrating_session_detail(
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    if !branch_is_integrated_into_akra(&resolution.context.repo_root, &record.branch_name) {
        let worktree = resolution.workspace_path.display().to_string();
        if !command_succeeds(
            "git",
            [
                "-C",
                worktree.as_str(),
                "-c",
                "rebase.autoStash=true",
                "rebase",
                AKRA_BRANCH,
            ],
        ) {
            return block_distributor_queue_record(
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!(
                    "branch `{}` could not rebase onto `{AKRA_BRANCH}` cleanly",
                    record.branch_name
                ),
            );
        }

        let previous_commit_sha = record.commit_sha.clone();
        if record
            .original_commit_sha
            .as_deref()
            .filter(|commit| !commit.trim().is_empty())
            .is_none()
        {
            record.original_commit_sha = Some(previous_commit_sha.clone());
        }
        record.commit_sha =
            resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
                format!(
                    "slot `{}` workspace head could not be resolved after distributor rebase",
                    resolution.lease.slot_id
                )
            })?;
        record.integration_note =
            format!("branch rebased onto `{AKRA_BRANCH}` and is ready for local integration");
        record.updated_at = current_timestamp();
        write_distributor_queue_record(&resolution.context.pool_root, record)?;
        let rebase_summary = rebase_provenance_label(record).unwrap_or_else(|| {
            "branch rebased onto akra and is ready for local integration".to_string()
        });
        let _ = record_integrating_session_detail(
            &resolution.context.pool_root,
            &resolution.lease,
            &rebase_summary,
        );
        if record.commit_sha != previous_commit_sha {
            let repo_root = resolution.context.repo_root.clone();
            if let Err(error) = github_automation.push_branch(&repo_root, &record.branch_name, true)
            {
                return block_distributor_queue_record(
                    &resolution.context.pool_root,
                    Some(&resolution.lease),
                    record,
                    format!(
                        "rebased branch `{}` could not be force-pushed: {error}",
                        record.branch_name
                    ),
                );
            }
            record.integration_note =
                "rebased branch force-pushed and ready for local integration".to_string();
            record.updated_at = current_timestamp();
            write_distributor_queue_record(&resolution.context.pool_root, record)?;
            let force_push_summary = rebase_provenance_label(record).map_or_else(
                || "rebased branch force-pushed and ready for local integration".to_string(),
                |provenance| format!("{provenance} / force-pushed and ready for local integration"),
            );
            let _ = record_integrating_session_detail(
                &resolution.context.pool_root,
                &resolution.lease,
                &force_push_summary,
            );
        }

        if !command_succeeds(
            "git",
            ["-C", worktree.as_str(), "branch", "-f", AKRA_BRANCH, "HEAD"],
        ) {
            return block_distributor_queue_record(
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!(
                    "local `{AKRA_BRANCH}` could not advance to `{}`",
                    short_sha(&record.commit_sha)
                ),
            );
        }
    }

    let repo_root = resolution.context.repo_root.clone();
    if let Err(error) = github_automation.push_integration_branch(&repo_root, AKRA_BRANCH) {
        return block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!("`{AKRA_BRANCH}` could not be pushed to `{DEFAULT_PUSH_REMOTE_NAME}`: {error}"),
        );
    }
    if let Some(pr_number) = record.pull_request_number {
        let pull_request = match github_automation.inspect_pull_request(&repo_root, pr_number) {
            Ok(pull_request) => pull_request,
            Err(error) => {
                return block_distributor_queue_record(
                    &resolution.context.pool_root,
                    Some(&resolution.lease),
                    record,
                    format!(
                        "pull request #{pr_number} could not be reloaded before close: {error}"
                    ),
                );
            }
        };
        record.pull_request_url = Some(pull_request.url.clone());
        if pull_request.state.eq_ignore_ascii_case("open")
            && let Err(error) = github_automation.close_pull_request(&repo_root, pr_number)
        {
            return block_distributor_queue_record(
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!("pull request #{pr_number} could not be closed: {error}"),
            );
        }
    }

    record.queue_state = ParallelModeQueueItemState::Cleaning;
    record.integration_note =
        "branch integrated into akra, pushed to origin, and the slot is entering cleanup"
            .to_string();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(&resolution.context.pool_root, record)?;

    Ok(format!(
        "distributor integrated queue head into akra / slot: {} / agent: {} / commit: {}",
        resolution.lease.slot_id,
        resolution.lease.agent_id,
        short_sha(&record.commit_sha)
    ))
}

fn distributor_cleanup_integrated_slot(
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<String, String> {
    if resolution.lease.state == ParallelModeSlotLeaseState::Running {
        let mut cleanup_pending_lease = resolution.lease.clone();
        cleanup_pending_lease.state = ParallelModeSlotLeaseState::CleanupPending;
        write_slot_lease(&resolution.context.pool_root, &cleanup_pending_lease)?;
        let _ = record_cleanup_pending_session_detail(
            &resolution.context.pool_root,
            &cleanup_pending_lease,
        );
    }

    if !cleanup_slot(
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease.slot_id,
        &resolution.workspace_path,
        &resolution.lease.branch_name,
    ) {
        return Ok(block_distributor_queue_record(
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "slot `{}` cleanup failed after distributor delivery",
                resolution.lease.slot_id
            ),
        )?);
    }

    let _ = record_cleaned_session_detail(&resolution.context.pool_root, &resolution.lease);
    record.queue_state = ParallelModeQueueItemState::Done;
    record.integration_note =
        "branch integrated into akra, GitHub delivery completed, and the slot returned to idle"
            .to_string();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(&resolution.context.pool_root, record)?;

    Ok(format!(
        "distributor returned slot to idle / slot: {} / agent: {}",
        resolution.lease.slot_id, resolution.lease.agent_id
    ))
}

fn block_distributor_queue_record(
    pool_root: &Path,
    lease: Option<&ParallelModeSlotLeaseSnapshot>,
    record: &mut ParallelModeDistributorQueueRecord,
    failure_detail: String,
) -> Result<String, String> {
    record.queue_state = ParallelModeQueueItemState::Blocked;
    record.integration_note = failure_detail.clone();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(pool_root, record)?;
    if let Some(lease) = lease {
        let _ = record_distributor_failed_session_detail(pool_root, lease, &failure_detail);
    }

    Ok(format!(
        "distributor queue head blocked / agent: {} / {}",
        record.agent_id, failure_detail
    ))
}

fn build_distributor_pull_request_title(record: &ParallelModeDistributorQueueRecord) -> String {
    format!("supersession: {}", record.task_title.trim())
}

fn build_distributor_pull_request_body(record: &ParallelModeDistributorQueueRecord) -> String {
    format!(
        "Automated distributor delivery for a supersession result.\n\n- Agent: {}\n- Task ID: {}\n- Branch: `{}`\n- Commit: `{}`\n- Validation: {}\n- Official refresh: {}",
        record.agent_id,
        record.task_id,
        record.branch_name,
        record.commit_sha,
        record.validation_summary.trim(),
        record.ledger_refresh_outcome.trim()
    )
}

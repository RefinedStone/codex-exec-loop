use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use chrono::Utc;

use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityPort,
    PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
    ParallelModeReadinessSnapshot, ParallelModeReadinessState, ParallelModeSlotLeaseRequest,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState,
};
use crate::domain::planning::{
    PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
};

pub(crate) mod distributor;
pub(crate) mod supervisor;
pub(crate) mod turn;

use self::distributor::{ParallelModeDistributorService, load_distributor_queue_records};
use self::supervisor::ParallelModeSupervisorService;

const AKRA_BRANCH: &str = "akra";
const DEFAULT_PUSH_REMOTE_NAME: &str = "origin";
const DEFAULT_POOL_SIZE: usize = 3;
const AKRA_AGENT_BRANCH_PREFIX: &str = "akra-agent";
const MAX_AGENT_BRANCH_SLUG_LEN: usize = 96;
const AGENT_BRANCH_TRUNCATION_HASH_LEN: usize = 10;
const NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL: &str =
    "agent branch is not integrated into `akra` and has no lease metadata";
const NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_NEXT_ACTION: &str =
    "inspect the slot branch, merge or discard it manually, then rerun reconcile";

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitWorktreeRecord {
    path: PathBuf,
    head_sha: String,
    branch_name: Option<String>,
    detached: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SlotGitStatus {
    has_staged: bool,
    has_unstaged: bool,
    has_untracked: bool,
    has_pending_operation: bool,
}

impl SlotGitStatus {
    fn is_clean_baseline(self) -> bool {
        !self.has_staged && !self.has_unstaged && !self.has_untracked && !self.has_pending_operation
    }

    fn detail_label(self) -> String {
        let mut details = Vec::new();
        if self.has_staged {
            details.push("staged changes");
        }
        if self.has_unstaged {
            details.push("unstaged changes");
        }
        if self.has_untracked {
            details.push("untracked files");
        }
        if self.has_pending_operation {
            details.push("merge/rebase metadata");
        }

        if details.is_empty() {
            "clean".to_string()
        } else {
            details.join(", ")
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PoolReconcileExecution {
    created_akra_branch: bool,
    created_pool_root: bool,
    provisioned_slots: usize,
    cleaned_slots: usize,
}

impl PoolReconcileExecution {
    fn has_actions(self) -> bool {
        self.created_akra_branch
            || self.created_pool_root
            || self.provisioned_slots > 0
            || self.cleaned_slots > 0
    }
}

#[derive(Debug, Clone)]
struct PoolRuntimeContext {
    repo_root: String,
    canonical_repo_root: PathBuf,
    pool_root: PathBuf,
    akra_head: String,
    worktree_records: Vec<GitWorktreeRecord>,
    slot_leases: BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
    invalid_slot_leases: BTreeSet<String>,
    session_details: Vec<ParallelModeAgentSessionDetailSnapshot>,
    distributor_queue_records: Vec<PlanningAuthorityDistributorQueueRecord>,
}

type PoolBoardWithContextResult = Result<
    (PoolRuntimeContext, ParallelModePoolBoardSnapshot),
    Box<(ParallelModePoolBoardSnapshot, String)>,
>;

#[derive(Debug, Clone)]
struct WorkspaceSlotLeaseResolution {
    context: PoolRuntimeContext,
    lease: ParallelModeSlotLeaseSnapshot,
    workspace_path: PathBuf,
}

pub type ParallelModeOfficialCompletionReport = PlanningOfficialCompletionRefreshContract;

#[derive(Clone)]
pub struct ParallelModeService {
    distributor_service: ParallelModeDistributorService,
    supervisor_service: ParallelModeSupervisorService,
    planning_authority: Arc<dyn PlanningAuthorityPort>,
}

impl std::fmt::Debug for ParallelModeService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParallelModeService")
            .finish_non_exhaustive()
    }
}

impl ParallelModeService {
    pub fn new(
        planning_authority: Arc<dyn PlanningAuthorityPort>,
        github_automation: Arc<dyn GithubAutomationPort>,
    ) -> Self {
        Self {
            distributor_service: ParallelModeDistributorService::with_planning_authority(
                github_automation,
                planning_authority.clone(),
            ),
            supervisor_service: ParallelModeSupervisorService::new(),
            planning_authority,
        }
    }

    pub fn reserve_workspace_official_completion_refresh_order(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<u64>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        self.planning_authority
            .reserve_next_official_refresh_order(&resolution.lease.worktree_path)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    pub fn inspect_readiness(
        &self,
        workspace_dir: &str,
        planning_snapshot: &PlanningRuntimeSnapshot,
    ) -> ParallelModeReadinessSnapshot {
        let repo_root = detect_git_repo_root(workspace_dir);
        let git_repository = match &repo_root {
            Some(repo_root) => ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GitRepository,
                ParallelModeCapabilityState::Ready,
                format!("git repo detected at {repo_root}"),
                None,
            ),
            None => ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GitRepository,
                ParallelModeCapabilityState::Blocked,
                "parallel mode only runs inside a git repository",
                Some("open a git-backed workspace before enabling parallel mode".to_string()),
            ),
        };

        let git_worktree = match &repo_root {
            Some(repo_root) => inspect_git_worktree(repo_root),
            None => blocked_prerequisite_capability(
                ParallelModeCapabilityKey::GitWorktree,
                "waiting for git repository detection",
                "enter a git repository first",
            ),
        };
        let akra_branch = match &repo_root {
            Some(repo_root) => inspect_akra_branch(repo_root),
            None => blocked_prerequisite_capability(
                ParallelModeCapabilityKey::AkraBranch,
                "waiting for git repository detection",
                "enter a git repository first",
            ),
        };
        let push_remote = match &repo_root {
            Some(repo_root) => inspect_push_remote(repo_root),
            None => blocked_prerequisite_capability(
                ParallelModeCapabilityKey::PushRemote,
                "waiting for git repository detection",
                "enter a git repository first",
            ),
        };
        let gh_binary = inspect_gh_binary();
        let gh_auth = inspect_gh_auth(&gh_binary, repo_root.as_deref());
        let planning = inspect_planning(planning_snapshot);
        let authority_store = inspect_authority_store(
            self.planning_authority.as_ref(),
            workspace_dir,
            &git_repository,
            &planning,
        );

        let capabilities = vec![
            git_repository,
            git_worktree,
            akra_branch,
            push_remote,
            gh_binary,
            gh_auth,
            planning,
            authority_store,
        ];
        let readiness = derive_readiness(&capabilities);
        let top_alert = capabilities
            .iter()
            .find(|capability| capability.state != ParallelModeCapabilityState::Ready)
            .map(ParallelModeCapabilitySnapshot::summary);
        let snapshot =
            ParallelModeReadinessSnapshot::new(workspace_dir, readiness, capabilities, top_alert);
        if snapshot.allows_parallel_mode() {
            let _ = self
                .distributor_service
                .recover_runtime_state(workspace_dir);
        }
        snapshot
    }

    pub fn build_supervisor_snapshot(
        &self,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    ) -> ParallelModeSupervisorSnapshot {
        self.supervisor_service.build_snapshot(
            self.planning_authority.as_ref(),
            workspace_dir,
            mode_enabled,
            readiness_snapshot,
            &self.distributor_service,
        )
    }

    pub fn reconcile_supervisor_snapshot(
        &self,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    ) -> ParallelModeSupervisorSnapshot {
        self.supervisor_service.reconcile_snapshot(
            self.planning_authority.as_ref(),
            workspace_dir,
            mode_enabled,
            readiness_snapshot,
            &self.distributor_service,
        )
    }

    pub fn acquire_slot_lease(
        &self,
        workspace_dir: &str,
        request: ParallelModeSlotLeaseRequest,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let _ = reconcile_pool_board(self.planning_authority.as_ref(), workspace_dir);
        let context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;

        if context
            .slot_leases
            .values()
            .any(|lease| lease.task_id == request.task_id)
        {
            return Err(format!(
                "task `{}` already has an active slot lease",
                request.task_id
            ));
        }
        if context
            .slot_leases
            .values()
            .any(|lease| lease.agent_id == request.agent_id)
        {
            return Err(format!(
                "agent `{}` already owns an active slot lease",
                request.agent_id
            ));
        }

        let Some(idle_slot) = build_pool_slots(&context)
            .into_iter()
            .find(|slot| slot.state == ParallelModePoolSlotState::Idle)
        else {
            return Err("no idle slot is available for lease".to_string());
        };

        let slot_path = context.pool_root.join(&idle_slot.slot_id);
        let slot_path_string = slot_path.display().to_string();
        let branch_name = allocate_agent_branch_name(
            &context.repo_root,
            &idle_slot.slot_id,
            &request.task_slug,
            &request.task_id,
            &request.task_title,
        );
        if !command_succeeds(
            "git",
            [
                "-C",
                slot_path_string.as_str(),
                "checkout",
                "-b",
                branch_name.as_str(),
                AKRA_BRANCH,
            ],
        ) {
            return Err(format!(
                "failed to create branch `{branch_name}` in slot `{}`",
                idle_slot.slot_id
            ));
        }

        let lease = ParallelModeSlotLeaseSnapshot::new(
            idle_slot.slot_id.clone(),
            request.task_id,
            request.task_title,
            request.agent_id,
            branch_name.clone(),
            slot_path_string.clone(),
            ParallelModeSlotLeaseState::Leased,
            current_timestamp(),
            None,
        );
        if let Err(error) = write_slot_lease(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        ) {
            let _ =
                discard_unstarted_slot_branch(&context.repo_root, &slot_path, branch_name.as_str());
            return Err(error);
        }
        let _ = record_assigned_session_detail(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        );

        Ok(lease)
    }

    pub fn mark_slot_running(
        &self,
        workspace_dir: &str,
        slot_id: &str,
        agent_id: &str,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;
        let mut lease = context
            .slot_leases
            .get(slot_id)
            .cloned()
            .ok_or_else(|| format!("slot `{slot_id}` does not have an active lease"))?;

        if lease.agent_id != agent_id {
            return Err(format!(
                "slot `{slot_id}` is leased by `{}` instead of `{agent_id}`",
                lease.agent_id
            ));
        }
        if lease.state == ParallelModeSlotLeaseState::CleanupPending {
            return Err(format!("slot `{slot_id}` is already waiting for cleanup",));
        }
        if current_branch_name(Path::new(&lease.worktree_path)).as_deref()
            != Some(lease.branch_name.as_str())
        {
            return Err(format!(
                "slot `{slot_id}` is no longer checked out to `{}`",
                lease.branch_name
            ));
        }

        lease.state = ParallelModeSlotLeaseState::Running;
        if lease.running_started_at.is_none() {
            lease.running_started_at = Some(current_timestamp());
        }
        write_slot_lease(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        )?;
        let _ = record_running_session_detail(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        );
        Ok(lease)
    }

    pub fn record_workspace_slot_thread_prepared(
        &self,
        workspace_dir: &str,
        thread_id: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };

        record_thread_prepared_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            thread_id,
        )
        .map(Some)
    }

    pub fn mark_slot_cleanup_pending(
        &self,
        workspace_dir: &str,
        slot_id: &str,
        agent_id: &str,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;
        let mut lease = context
            .slot_leases
            .get(slot_id)
            .cloned()
            .ok_or_else(|| format!("slot `{slot_id}` does not have an active lease"))?;

        if lease.agent_id != agent_id {
            return Err(format!(
                "slot `{slot_id}` is leased by `{}` instead of `{agent_id}`",
                lease.agent_id
            ));
        }
        if lease.state == ParallelModeSlotLeaseState::Leased {
            return Err(format!(
                "slot `{slot_id}` has not entered running state yet",
            ));
        }
        if lease.state == ParallelModeSlotLeaseState::CleanupPending {
            return Ok(lease);
        }
        if current_branch_name(Path::new(&lease.worktree_path)).as_deref()
            != Some(lease.branch_name.as_str())
        {
            return Err(format!(
                "slot `{slot_id}` is no longer checked out to `{}`",
                lease.branch_name
            ));
        }
        if !branch_is_cleanup_ready(&context.repo_root, &lease.branch_name) {
            return Err(format!(
                "slot `{slot_id}` branch `{}` is not integrated into `{AKRA_BRANCH}` yet",
                lease.branch_name
            ));
        }

        lease.state = ParallelModeSlotLeaseState::CleanupPending;
        write_slot_lease(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        )?;
        let _ = record_cleanup_pending_session_detail(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        );
        Ok(lease)
    }

    pub fn mark_workspace_slot_running(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };

        self.mark_slot_running(
            workspace_dir,
            &resolution.lease.slot_id,
            &resolution.lease.agent_id,
        )
        .map(Some)
    }

    pub fn release_workspace_slot_lease_after_failed_start(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Leased {
            return Ok(None);
        }

        let Some(slot_status) = inspect_slot_git_status(&resolution.workspace_path) else {
            return Err(format!(
                "slot `{}` could not be inspected after startup failure",
                resolution.lease.slot_id
            ));
        };
        if !slot_status.is_clean_baseline() {
            return Err(format!(
                "slot `{}` could not be released after startup failure because worktree is not clean: {}",
                resolution.lease.slot_id,
                slot_status.detail_label()
            ));
        }

        if !cleanup_slot(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease.slot_id,
            &resolution.workspace_path,
            &resolution.lease.branch_name,
        ) {
            return Err(format!(
                "slot `{}` could not be reset to `{AKRA_BRANCH}` after startup failure",
                resolution.lease.slot_id
            ));
        }
        let _ = record_failed_start_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
        );

        Ok(Some(resolution.lease))
    }

    pub fn begin_workspace_official_completion(
        &self,
        workspace_dir: &str,
        root_turn_id: &str,
        official_completion_refresh_order: Option<u64>,
        final_response_text: Option<&str>,
        validation_summary: Option<&str>,
        failure_context: Option<&str>,
    ) -> Result<Option<ParallelModeOfficialCompletionReport>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        let commit_sha =
            resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
                format!(
                    "slot `{}` workspace head could not be resolved for official completion",
                    resolution.lease.slot_id
                )
            })?;
        let completed_at = current_timestamp();
        let refresh_order = official_completion_refresh_order
            .map(Ok)
            .unwrap_or_else(|| {
                self.planning_authority
                    .reserve_next_official_refresh_order(&resolution.lease.worktree_path)
                    .map_err(|error| error.to_string())
            })?;
        let final_response_text = normalized_optional_text(final_response_text).map(str::to_string);
        let validation_summary = normalized_optional_text(validation_summary)
            .unwrap_or("validation status was not reported by runtime")
            .to_string();
        let failure_context = normalized_optional_text(failure_context).map(str::to_string);
        let final_response_summary = completion_summary_from_text(
            final_response_text.as_deref(),
            failure_context.as_deref(),
        );

        record_reported_complete_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            ReportedCompleteSessionDetailUpdate {
                completed_at: &completed_at,
                final_response_summary: &final_response_summary,
                validation_summary: &validation_summary,
                failure_context: failure_context.as_deref(),
            },
        )?;

        Ok(Some(PlanningOfficialCompletionRefreshContract::new(
            root_turn_id,
            refresh_order,
            PlanningOfficialCompletionRefreshPayload::new(
                resolution.lease.agent_id,
                resolution.lease.task_id,
                resolution.lease.task_title,
                resolution.lease.branch_name,
                resolution.lease.worktree_path,
                commit_sha,
                validation_summary,
                final_response_summary,
                final_response_text,
                failure_context,
                completed_at,
            ),
        )))
    }

    pub fn mark_workspace_official_completion_refreshing(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        record_ledger_refreshing_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
        )
        .map(Some)
    }

    pub fn mark_workspace_commit_ready(
        &self,
        workspace_dir: &str,
        ledger_refresh_outcome: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        record_commit_ready_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            ledger_refresh_outcome,
        )
        .map(Some)
    }

    pub fn enqueue_workspace_commit_ready_result(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<crate::domain::parallel_mode::ParallelModeDistributorQueueItem>, String>
    {
        self.distributor_service
            .enqueue_workspace_commit_ready_result(workspace_dir)
    }

    pub fn process_distributor_queue(&self, workspace_dir: &str) -> Result<Vec<String>, String> {
        self.distributor_service.process_queue(workspace_dir)
    }

    pub fn mark_workspace_official_completion_failed(
        &self,
        workspace_dir: &str,
        failure_detail: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        record_official_completion_failed_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            failure_detail,
        )
        .map(Some)
    }

    pub fn mark_workspace_slot_cleanup_pending_if_ready(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state == ParallelModeSlotLeaseState::CleanupPending {
            return Ok(Some(resolution.lease));
        }
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }
        if !branch_is_cleanup_ready(&resolution.context.repo_root, &resolution.lease.branch_name) {
            return Ok(None);
        }

        self.mark_slot_cleanup_pending(
            workspace_dir,
            &resolution.lease.slot_id,
            &resolution.lease.agent_id,
        )
        .map(Some)
    }

    pub fn cleanup_workspace_slot_if_pending(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::CleanupPending {
            return Ok(None);
        }

        if !cleanup_slot(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease.slot_id,
            &resolution.workspace_path,
            &resolution.lease.branch_name,
        ) {
            return Err(format!(
                "slot `{}` could not be reset to `{AKRA_BRANCH}` after successful completion",
                resolution.lease.slot_id
            ));
        }
        let _ = record_cleaned_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
        );

        Ok(Some(resolution.lease))
    }
}

fn detect_git_repo_root(workspace_dir: &str) -> Option<String> {
    run_command(
        "git",
        ["-C", workspace_dir, "rev-parse", "--show-toplevel"],
        None,
    )
    .filter(|value| !value.is_empty())
}

fn inspect_git_worktree(repo_root: &str) -> ParallelModeCapabilitySnapshot {
    match run_command(
        "git",
        ["-C", repo_root, "worktree", "list", "--porcelain"],
        None,
    ) {
        Some(_) => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GitWorktree,
            ParallelModeCapabilityState::Ready,
            "git worktree support is available",
            None,
        ),
        None => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GitWorktree,
            ParallelModeCapabilityState::Blocked,
            "git worktree commands are unavailable in this repository",
            Some("upgrade git or repair the repository worktree metadata".to_string()),
        ),
    }
}

fn inspect_akra_branch(repo_root: &str) -> ParallelModeCapabilitySnapshot {
    if command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "show-ref",
            "--verify",
            "--quiet",
            "refs/heads/akra",
        ],
    ) || command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/akra",
        ],
    ) {
        return ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::AkraBranch,
            ParallelModeCapabilityState::Ready,
            format!("{AKRA_BRANCH} is available"),
            None,
        );
    }

    if command_succeeds("git", ["-C", repo_root, "rev-parse", "--verify", "HEAD"]) {
        return ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::AkraBranch,
            ParallelModeCapabilityState::Ready,
            format!("{AKRA_BRANCH} is missing locally but can be created from HEAD"),
            None,
        );
    }

    ParallelModeCapabilitySnapshot::new(
        ParallelModeCapabilityKey::AkraBranch,
        ParallelModeCapabilityState::Blocked,
        format!("{AKRA_BRANCH} is missing and this repository has no usable HEAD yet"),
        Some("create an initial commit or restore the integration branch before enabling parallel mode".to_string()),
    )
}

fn inspect_push_remote(repo_root: &str) -> ParallelModeCapabilitySnapshot {
    let Some(push_url) = run_command(
        "git",
        [
            "-C",
            repo_root,
            "remote",
            "get-url",
            "--push",
            DEFAULT_PUSH_REMOTE_NAME,
        ],
        None,
    ) else {
        return ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::PushRemote,
            ParallelModeCapabilityState::Degraded,
            format!("push remote `{DEFAULT_PUSH_REMOTE_NAME}` is not configured"),
            Some(
                "add a push remote or keep supersession in local-only inspection mode".to_string(),
            ),
        );
    };

    let Some((host, path)) = parse_https_remote(&push_url) else {
        return ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::PushRemote,
            ParallelModeCapabilityState::Degraded,
            format!("unsupported push remote `{push_url}`"),
            Some("use an https GitHub remote to enable push capability checks".to_string()),
        );
    };

    let Some(credentials) = run_command_with_stdin(
        "git",
        ["credential", "fill"],
        format!("protocol=https\nhost={host}\npath={path}\n\n"),
    ) else {
        return ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::PushRemote,
            ParallelModeCapabilityState::Degraded,
            "git credentials are not available for the push remote",
            Some("restore push credentials before relying on distributor automation".to_string()),
        );
    };

    let username = credentials.lines().find_map(|line| {
        line.strip_prefix("username=")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });

    match username {
        Some(username) => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::PushRemote,
            ParallelModeCapabilityState::Ready,
            format!("push remote is configured and resolves credentials for {username}"),
            None,
        ),
        None => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::PushRemote,
            ParallelModeCapabilityState::Degraded,
            "push remote exists but no username was resolved",
            Some(
                "restore repository credentials before relying on distributor automation"
                    .to_string(),
            ),
        ),
    }
}

fn inspect_gh_binary() -> ParallelModeCapabilitySnapshot {
    match which::which("gh") {
        Ok(path) => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhBinary,
            ParallelModeCapabilityState::Ready,
            format!("gh found at {}", path.display()),
            None,
        ),
        Err(_) => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhBinary,
            ParallelModeCapabilityState::Degraded,
            "gh is not installed on PATH",
            Some("install GitHub CLI or plan for manual PR handling".to_string()),
        ),
    }
}

fn inspect_gh_auth(
    gh_binary: &ParallelModeCapabilitySnapshot,
    repo_root: Option<&str>,
) -> ParallelModeCapabilitySnapshot {
    if gh_binary.state != ParallelModeCapabilityState::Ready {
        return ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhAuth,
            ParallelModeCapabilityState::Degraded,
            "gh auth is unavailable until the gh binary is installed",
            Some("install gh first, then run `gh auth login`".to_string()),
        );
    }

    let mut command = Command::new("gh");
    command.args(["auth", "status"]);
    if let Some(repo_root) = repo_root {
        command.current_dir(repo_root);
    }
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command.env("GIT_TERMINAL_PROMPT", "0");

    match command.status() {
        Ok(status) if status.success() => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhAuth,
            ParallelModeCapabilityState::Ready,
            "gh auth status succeeded",
            None,
        ),
        _ => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhAuth,
            ParallelModeCapabilityState::Degraded,
            "gh is installed but not authenticated for this workspace",
            Some("run `gh auth login` before enabling GitHub automation".to_string()),
        ),
    }
}

fn inspect_planning(snapshot: &PlanningRuntimeSnapshot) -> ParallelModeCapabilitySnapshot {
    if !snapshot.workspace_present() {
        return ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::Planning,
            ParallelModeCapabilityState::Blocked,
            "planning workspace is not initialized",
            Some(
                "open `:planning` and initialize the workspace before enabling parallel mode"
                    .to_string(),
            ),
        );
    }

    match snapshot.workspace_status() {
        PlanningRuntimeWorkspaceStatus::Uninitialized => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::Planning,
            ParallelModeCapabilityState::Blocked,
            "planning workspace is not initialized",
            Some(
                "open `:planning` and initialize the workspace before enabling parallel mode"
                    .to_string(),
            ),
        ),
        PlanningRuntimeWorkspaceStatus::Invalid => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::Planning,
            ParallelModeCapabilityState::Blocked,
            snapshot
                .failure_reason()
                .unwrap_or("planning validation failed"),
            Some("repair planning state before enabling parallel mode".to_string()),
        ),
        PlanningRuntimeWorkspaceStatus::ReadyNoTask => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::Planning,
            ParallelModeCapabilityState::Ready,
            snapshot
                .queue_summary()
                .unwrap_or("planning workspace is ready with no queue head"),
            None,
        ),
        PlanningRuntimeWorkspaceStatus::ReadyWithTask => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::Planning,
            ParallelModeCapabilityState::Ready,
            snapshot
                .queue_summary()
                .unwrap_or("planning workspace is ready"),
            None,
        ),
    }
}

fn inspect_authority_store(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    git_repository: &ParallelModeCapabilitySnapshot,
    planning: &ParallelModeCapabilitySnapshot,
) -> ParallelModeCapabilitySnapshot {
    if git_repository.state != ParallelModeCapabilityState::Ready {
        return blocked_prerequisite_capability(
            ParallelModeCapabilityKey::AuthorityStore,
            "waiting for git repository detection",
            "enter a git repository first",
        );
    }

    if planning.state != ParallelModeCapabilityState::Ready {
        return blocked_prerequisite_capability(
            ParallelModeCapabilityKey::AuthorityStore,
            "waiting for planning readiness",
            "repair or initialize planning before inspecting authority parity",
        );
    }

    match planning_authority.inspect_shadow_store(workspace_dir) {
        Ok(inspection) => {
            let canonical_root = inspection.location.canonical_repo_root;
            let document_count = inspection.mirrored_document_count;
            let detail = match inspection.sync_state {
                crate::domain::planning::PlanningAuthorityShadowStoreSyncState::Bootstrapped => {
                    format!(
                        "shadow store bootstrapped from {document_count} mirrored documents / canonical root: {canonical_root}"
                    )
                }
                crate::domain::planning::PlanningAuthorityShadowStoreSyncState::InSync => {
                    format!(
                        "shadow store in sync across {document_count} mirrored documents / canonical root: {canonical_root}"
                    )
                }
                crate::domain::planning::PlanningAuthorityShadowStoreSyncState::Resynced => {
                    let sample = inspection
                        .parity_issue_examples
                        .first()
                        .map(|example| format!(" / sample: {example}"))
                        .unwrap_or_default();
                    format!(
                        "shadow store resynced {} parity issue(s) across {document_count} mirrored documents / canonical root: {canonical_root}{sample}",
                        inspection.parity_issue_count,
                    )
                }
            };
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::AuthorityStore,
                ParallelModeCapabilityState::Ready,
                detail,
                None,
            )
        }
        Err(error) => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::AuthorityStore,
            ParallelModeCapabilityState::Degraded,
            format!("shadow store inspection failed: {error}"),
            Some("inspect the repo-scoped authority store and rerun readiness".to_string()),
        ),
    }
}

fn blocked_prerequisite_capability(
    key: ParallelModeCapabilityKey,
    detail: &str,
    next_action: &str,
) -> ParallelModeCapabilitySnapshot {
    ParallelModeCapabilitySnapshot::new(
        key,
        ParallelModeCapabilityState::Blocked,
        detail,
        Some(next_action.to_string()),
    )
}

fn derive_readiness(capabilities: &[ParallelModeCapabilitySnapshot]) -> ParallelModeReadinessState {
    if capabilities
        .iter()
        .any(|capability| capability.state == ParallelModeCapabilityState::Blocked)
    {
        return ParallelModeReadinessState::Blocked;
    }

    if capabilities
        .iter()
        .any(|capability| capability.state != ParallelModeCapabilityState::Ready)
    {
        return ParallelModeReadinessState::Degraded;
    }

    ParallelModeReadinessState::Ready
}

fn derive_supervisor_state(
    mode_enabled: bool,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModeSupervisorState {
    if mode_enabled && readiness_snapshot.is_some_and(|snapshot| !snapshot.allows_parallel_mode()) {
        return ParallelModeSupervisorState::Recover;
    }

    if mode_enabled {
        return ParallelModeSupervisorState::Supervise;
    }

    ParallelModeSupervisorState::Prepare
}

fn default_supervisor_notice(
    mode_enabled: bool,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> Option<String> {
    match (mode_enabled, readiness_snapshot) {
        (true, Some(snapshot)) if snapshot.allows_parallel_mode() => {
            Some("control tower is live in read-only supervisor mode".to_string())
        }
        (true, Some(_)) => Some("repair readiness blockers before assigning agents".to_string()),
        (false, Some(_)) => Some("run `:parallel on` after reviewing the board".to_string()),
        (true, None) => Some("rerun readiness to hydrate the supervisor board".to_string()),
        (false, None) => None,
    }
}

fn build_pool_board(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModePoolBoardSnapshot {
    match readiness_snapshot {
        Some(snapshot) if snapshot.allows_parallel_mode() => {
            inspect_pool_board(planning_authority, workspace_dir)
        }
        Some(snapshot) => build_unavailable_pool_board(
            planning_authority,
            workspace_dir,
            format!(
                "reconcile blocked / readiness: {}",
                snapshot.readiness_label()
            ),
            "not leased",
            "reconcile blocked by readiness gate",
            "supervisor gate",
        ),
        None => build_unavailable_pool_board(
            planning_authority,
            workspace_dir,
            "reconcile pending / run readiness first",
            "not inspected",
            "readiness has not been checked",
            "n/a",
        ),
    }
}

fn reconcile_pool_board(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> ParallelModePoolBoardSnapshot {
    match reconcile_pool_board_and_context(planning_authority, workspace_dir) {
        Ok((_, pool)) => pool,
        Err(error) => {
            let (pool, _) = *error;
            pool
        }
    }
}

fn reconcile_pool_board_and_context(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> PoolBoardWithContextResult {
    let Some(repo_root) = detect_git_repo_root(workspace_dir) else {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile failed / git repository is unavailable",
                "repository inspection failed",
            ),
            "repository inspection failed".to_string(),
        )));
    };
    let Some(canonical_repo_root) = detect_canonical_repo_root(planning_authority, workspace_dir)
    else {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile failed / canonical repository root is unavailable",
                "canonical root inspection failed",
            ),
            "canonical root inspection failed".to_string(),
        )));
    };
    let pool_root = derive_default_pool_root(&canonical_repo_root);
    let pool_root_existed = pool_root.exists();
    if ensure_directory_exists(&pool_root).is_err() {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile failed / pool root could not be created",
                "pool root creation failed",
            ),
            "pool root creation failed".to_string(),
        )));
    }
    let created_pool_root = !pool_root_existed;
    let runtime_projection =
        load_runtime_projection_snapshot(planning_authority, &repo_root, &pool_root);
    let can_reset_akra_baseline = runtime_projection.slot_leases.is_empty()
        && runtime_projection.distributor_queue_records.is_empty();
    let Ok((_akra_head, created_akra_branch)) =
        ensure_akra_branch(&repo_root, can_reset_akra_baseline)
    else {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile blocked / `akra` baseline could not be created",
                "`akra` is unavailable during reconcile",
            ),
            "`akra` is unavailable during reconcile".to_string(),
        )));
    };
    let Some(mut worktree_records) = load_worktree_records(&repo_root) else {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile failed / git worktree inventory could not be loaded",
                "worktree list inspection failed",
            ),
            "worktree list inspection failed".to_string(),
        )));
    };
    let reset_stale_baseline_slots = if can_reset_akra_baseline {
        reset_stale_detached_baseline_slots(&repo_root, &pool_root, &worktree_records)
    } else {
        0
    };
    if reset_stale_baseline_slots > 0
        && let Some(refreshed_records) = load_worktree_records(&repo_root)
    {
        worktree_records = refreshed_records;
    }

    let provisioned_slots = provision_missing_slots(&repo_root, &pool_root, &worktree_records);
    let Some(reloaded_worktree_records) = load_worktree_records(&repo_root) else {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile failed / git worktree inventory could not be reloaded",
                "worktree list reload failed",
            ),
            "worktree list reload failed".to_string(),
        )));
    };
    let cleaned_slots = cleanup_reusable_slots(
        planning_authority,
        &repo_root,
        &pool_root,
        &reloaded_worktree_records,
    );

    let Ok(context) =
        load_pool_runtime_context_from_roots(planning_authority, &repo_root, &canonical_repo_root)
    else {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile failed / pool runtime state could not be loaded",
                "pool runtime load failed",
            ),
            "pool runtime load failed".to_string(),
        )));
    };

    let pool = build_pool_board_from_context(
        &context,
        summarize_pool_reconcile_status(
            &build_pool_slots(&context),
            &context.pool_root,
            Some(PoolReconcileExecution {
                created_akra_branch,
                created_pool_root,
                provisioned_slots,
                cleaned_slots,
            }),
        ),
    );

    Ok((context, pool))
}

fn inspect_pool_board(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> ParallelModePoolBoardSnapshot {
    match inspect_pool_board_and_context(planning_authority, workspace_dir) {
        Ok((_, pool)) => pool,
        Err(error) => {
            let (pool, _) = *error;
            pool
        }
    }
}

fn inspect_pool_board_and_context(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> PoolBoardWithContextResult {
    match load_pool_runtime_context(planning_authority, workspace_dir) {
        Ok(context) => {
            let pool = build_pool_board_from_context(
                &context,
                summarize_pool_reconcile_status(
                    &build_pool_slots(&context),
                    &context.pool_root,
                    None,
                ),
            );
            Ok((context, pool))
        }
        Err((reconcile_status, detail)) => Err(Box::new((
            build_blocked_pool_board(planning_authority, workspace_dir, reconcile_status, detail),
            detail.to_string(),
        ))),
    }
}

fn ensure_akra_branch(repo_root: &str, reset_to_current_head: bool) -> Result<(String, bool), ()> {
    if reset_to_current_head && let Some(head_sha) = resolve_branch_head(repo_root, "HEAD") {
        let existed = resolve_branch_head(repo_root, AKRA_BRANCH).is_some();
        if command_succeeds(
            "git",
            ["-C", repo_root, "branch", "-f", AKRA_BRANCH, "HEAD"],
        ) {
            return Ok((head_sha, !existed));
        }
    }

    if let Some(akra_head) = resolve_branch_head(repo_root, AKRA_BRANCH) {
        return Ok((akra_head, false));
    }

    let created = if command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/akra",
        ],
    ) {
        command_succeeds(
            "git",
            [
                "-C",
                repo_root,
                "branch",
                AKRA_BRANCH,
                "refs/remotes/origin/akra",
            ],
        )
    } else if command_succeeds("git", ["-C", repo_root, "rev-parse", "--verify", "HEAD"]) {
        command_succeeds("git", ["-C", repo_root, "branch", AKRA_BRANCH, "HEAD"])
    } else {
        false
    };

    if !created {
        return Err(());
    }

    resolve_branch_head(repo_root, AKRA_BRANCH)
        .map(|akra_head| (akra_head, true))
        .ok_or(())
}

fn ensure_directory_exists(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }

    fs::create_dir_all(path)
}

fn load_pool_runtime_context(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> Result<PoolRuntimeContext, (&'static str, &'static str)> {
    let Some(repo_root) = detect_git_repo_root(workspace_dir) else {
        return Err((
            "reconcile failed / git repository is unavailable",
            "repository inspection failed",
        ));
    };
    let Some(canonical_repo_root) = detect_canonical_repo_root(planning_authority, workspace_dir)
    else {
        return Err((
            "reconcile failed / canonical repository root is unavailable",
            "canonical root inspection failed",
        ));
    };

    load_pool_runtime_context_from_roots(planning_authority, &repo_root, &canonical_repo_root)
        .map_err(|detail| {
            (
                "reconcile failed / pool runtime state could not be loaded",
                detail,
            )
        })
}

fn resolve_workspace_slot_lease(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> Result<Option<WorkspaceSlotLeaseResolution>, String> {
    let context = load_pool_runtime_context(planning_authority, workspace_dir)
        .map_err(|(_, detail)| detail.to_string())?;
    let workspace_path = canonicalize_best_effort(Path::new(&context.repo_root));
    let Some(current_branch) = current_branch_name(&workspace_path) else {
        return Err(format!(
            "workspace `{}` does not currently resolve to a branch",
            workspace_path.display()
        ));
    };

    let mut matching_leases = context
        .slot_leases
        .values()
        .filter(|lease| worktree_paths_match(&workspace_path, Path::new(&lease.worktree_path)))
        .cloned()
        .collect::<Vec<_>>();

    if matching_leases.is_empty() {
        return Ok(None);
    }
    if matching_leases.len() > 1 {
        return Err(format!(
            "workspace `{}` matched multiple slot leases",
            workspace_path.display()
        ));
    }

    let lease = matching_leases
        .pop()
        .expect("matching lease count should be one");
    if lease.branch_name != current_branch {
        return Err(format!(
            "workspace `{}` is on `{}` but slot lease expects `{}`",
            workspace_path.display(),
            current_branch,
            lease.branch_name
        ));
    }

    Ok(Some(WorkspaceSlotLeaseResolution {
        context,
        lease,
        workspace_path,
    }))
}

fn load_pool_runtime_context_from_roots(
    planning_authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    canonical_repo_root: &Path,
) -> Result<PoolRuntimeContext, &'static str> {
    let Some(akra_head) = resolve_akra_baseline_head(repo_root) else {
        return Err("`akra` baseline is unavailable during inspection");
    };
    let Some(worktree_records) = load_worktree_records(repo_root) else {
        return Err("worktree list inspection failed");
    };
    let pool_root = derive_default_pool_root(canonical_repo_root);
    let runtime_projections =
        load_runtime_projection_snapshot(planning_authority, repo_root, &pool_root);

    Ok(PoolRuntimeContext {
        repo_root: repo_root.to_string(),
        canonical_repo_root: canonical_repo_root.to_path_buf(),
        pool_root,
        akra_head,
        worktree_records,
        slot_leases: runtime_projections.slot_leases,
        invalid_slot_leases: runtime_projections.invalid_slot_leases,
        session_details: runtime_projections.session_details,
        distributor_queue_records: runtime_projections.distributor_queue_records,
    })
}

fn load_runtime_projection_snapshot(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
) -> PlanningAuthorityRuntimeProjectionSnapshot {
    planning_authority
        .load_runtime_projections(workspace_dir)
        .unwrap_or_else(|_| {
            let (slot_leases, invalid_slot_leases) = read_slot_leases(pool_root);
            PlanningAuthorityRuntimeProjectionSnapshot {
                slot_leases,
                invalid_slot_leases,
                session_details: load_agent_session_detail_records(pool_root),
                distributor_queue_records: load_distributor_queue_records(pool_root),
            }
        })
}

fn load_worktree_records(repo_root: &str) -> Option<Vec<GitWorktreeRecord>> {
    let worktree_output = run_command(
        "git",
        ["-C", repo_root, "worktree", "list", "--porcelain"],
        None,
    )?;
    Some(parse_worktree_records(&worktree_output))
}

fn build_pool_board_from_context(
    context: &PoolRuntimeContext,
    reconcile_status: impl Into<String>,
) -> ParallelModePoolBoardSnapshot {
    let slots = build_pool_slots(context);
    let pool_root_label = display_pool_path(&context.canonical_repo_root, &context.pool_root);

    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
}

fn build_pool_slots(context: &PoolRuntimeContext) -> Vec<ParallelModePoolSlotSnapshot> {
    (1..=DEFAULT_POOL_SIZE)
        .map(|slot_number| inspect_pool_slot(context, &slot_id(slot_number)))
        .collect::<Vec<_>>()
}

fn provision_missing_slots(
    repo_root: &str,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
) -> usize {
    let mut provisioned_slots = 0;

    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_path = pool_root.join(slot_id(slot_number));
        if worktree_records
            .iter()
            .any(|record| record.path == slot_path)
            || slot_path.exists()
        {
            continue;
        }

        let Some(slot_parent) = slot_path.parent() else {
            continue;
        };
        if ensure_directory_exists(slot_parent).is_err() {
            continue;
        }

        let slot_path_string = slot_path.display().to_string();
        if command_succeeds(
            "git",
            [
                "-C",
                repo_root,
                "worktree",
                "add",
                "--detach",
                slot_path_string.as_str(),
                AKRA_BRANCH,
            ],
        ) {
            provisioned_slots += 1;
        }
    }

    provisioned_slots
}

fn reset_stale_detached_baseline_slots(
    repo_root: &str,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
) -> usize {
    let akra_head = resolve_akra_baseline_head(repo_root).unwrap_or_default();
    if akra_head.is_empty() {
        return 0;
    }

    let mut reset_slots = 0;
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_path = pool_root.join(slot_id(slot_number));
        let Some(worktree_record) = worktree_records
            .iter()
            .find(|record| record.path == slot_path)
        else {
            continue;
        };
        if !worktree_record.detached || worktree_record.head_sha == akra_head {
            continue;
        }
        if !inspect_slot_git_status(&slot_path).is_some_and(SlotGitStatus::is_clean_baseline) {
            continue;
        }
        let slot_path_string = slot_path.display().to_string();
        if command_succeeds(
            "git",
            [
                "-C",
                slot_path_string.as_str(),
                "checkout",
                "--detach",
                AKRA_BRANCH,
            ],
        ) && command_succeeds(
            "git",
            [
                "-C",
                slot_path_string.as_str(),
                "reset",
                "--hard",
                AKRA_BRANCH,
            ],
        ) && command_succeeds("git", ["-C", slot_path_string.as_str(), "clean", "-fdx"])
        {
            reset_slots += 1;
        }
    }

    reset_slots
}

fn cleanup_reusable_slots(
    planning_authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
) -> usize {
    let mut cleaned_slots = 0;
    let slot_leases =
        load_runtime_projection_snapshot(planning_authority, repo_root, pool_root).slot_leases;

    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_id = slot_id(slot_number);
        let slot_path = pool_root.join(&slot_id);
        let Some(worktree_record) = worktree_records
            .iter()
            .find(|record| record.path == slot_path)
        else {
            continue;
        };
        let Some(branch_name) = worktree_record.branch_name.as_deref() else {
            continue;
        };
        let expected_agent_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
        if !branch_name.starts_with(&expected_agent_prefix) {
            continue;
        }
        let slot_lease = slot_leases.get(&slot_id);
        let cleanup_ready = match slot_lease.map(|lease| lease.state) {
            Some(ParallelModeSlotLeaseState::CleanupPending) => {
                branch_is_cleanup_ready(repo_root, branch_name)
            }
            Some(ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running) => false,
            None => {
                inspect_slot_git_status(&slot_path).is_some_and(SlotGitStatus::is_clean_baseline)
                    && branch_is_cleanup_ready(repo_root, branch_name)
            }
        };
        if !cleanup_ready {
            continue;
        }
        if cleanup_slot(
            planning_authority,
            repo_root,
            pool_root,
            &slot_id,
            &slot_path,
            branch_name,
        ) {
            cleaned_slots += 1;
        }
    }

    cleaned_slots
}

fn branch_is_integrated_into_akra(repo_root: &str, branch_name: &str) -> bool {
    command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "merge-base",
            "--is-ancestor",
            branch_name,
            AKRA_BRANCH,
        ],
    )
}

fn branch_is_cleanup_ready(repo_root: &str, branch_name: &str) -> bool {
    branch_is_integrated_into_akra(repo_root, branch_name)
}

fn cleanup_slot(
    planning_authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    pool_root: &Path,
    slot_id: &str,
    slot_path: &Path,
    branch_name: &str,
) -> bool {
    let slot_path_string = slot_path.display().to_string();
    if !command_succeeds(
        "git",
        [
            "-C",
            slot_path_string.as_str(),
            "checkout",
            "--detach",
            AKRA_BRANCH,
        ],
    ) {
        return false;
    }
    if !command_succeeds(
        "git",
        [
            "-C",
            slot_path_string.as_str(),
            "reset",
            "--hard",
            AKRA_BRANCH,
        ],
    ) {
        return false;
    }
    if !command_succeeds("git", ["-C", slot_path_string.as_str(), "clean", "-fdx"]) {
        return false;
    }
    if !command_succeeds("git", ["-C", repo_root, "branch", "-D", branch_name]) {
        return false;
    }
    if !remove_slot_lease(planning_authority, repo_root, pool_root, slot_id) {
        return false;
    }

    inspect_slot_git_status(slot_path).is_some_and(SlotGitStatus::is_clean_baseline)
}

fn build_unavailable_pool_board(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    reconcile_status: impl Into<String>,
    branch_name: &str,
    worktree_label: &str,
    owner_label: &str,
) -> ParallelModePoolBoardSnapshot {
    let pool_root_label = derive_pool_root_label(planning_authority, workspace_dir);
    let slots = (1..=DEFAULT_POOL_SIZE)
        .map(|slot_number| {
            ParallelModePoolSlotSnapshot::new(
                slot_id(slot_number),
                ParallelModePoolSlotState::Unavailable,
                branch_name,
                worktree_label,
                owner_label,
            )
        })
        .collect::<Vec<_>>();

    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
}

fn build_blocked_pool_board(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    reconcile_status: impl Into<String>,
    detail: &str,
) -> ParallelModePoolBoardSnapshot {
    let pool_root_label = derive_pool_root_label(planning_authority, workspace_dir);
    let slots = (1..=DEFAULT_POOL_SIZE)
        .map(|slot_number| {
            ParallelModePoolSlotSnapshot::new(
                slot_id(slot_number),
                ParallelModePoolSlotState::Blocked,
                "unknown",
                detail,
                "operator recovery",
            )
        })
        .collect::<Vec<_>>();

    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
}

fn inspect_pool_slot(context: &PoolRuntimeContext, slot_id: &str) -> ParallelModePoolSlotSnapshot {
    let slot_path = context.pool_root.join(slot_id);
    let base_worktree_label = display_pool_path(&context.canonical_repo_root, &slot_path);
    let slot_lease = context.slot_leases.get(slot_id);

    if context.invalid_slot_leases.contains(slot_id) {
        return ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Blocked,
            "unknown",
            annotate_worktree_label(base_worktree_label, "invalid lease metadata"),
            "operator recovery",
        );
    }

    let Some(worktree_record) = context
        .worktree_records
        .iter()
        .find(|record| record.path == slot_path)
    else {
        if let Some(slot_lease) = slot_lease {
            return ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                slot_lease.branch_name.clone(),
                annotate_worktree_label(
                    base_worktree_label,
                    "lease exists but worktree is missing",
                ),
                slot_lease.owner_label(),
            );
        }
        if slot_path.exists() {
            return ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                "unknown",
                annotate_worktree_label(
                    base_worktree_label,
                    "directory exists outside git worktree inventory",
                ),
                "operator recovery",
            );
        }

        return ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Missing,
            AKRA_BRANCH,
            base_worktree_label,
            "reconcile pending",
        );
    };

    let Some(slot_status) = inspect_slot_git_status(&slot_path) else {
        return ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Blocked,
            slot_lease
                .map(|lease| lease.branch_name.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            annotate_worktree_label(base_worktree_label, "git status inspection failed"),
            slot_lease
                .map(ParallelModeSlotLeaseSnapshot::owner_label)
                .unwrap_or_else(|| "operator recovery".to_string()),
        );
    };

    if worktree_record.branch_name.as_deref() == Some(AKRA_BRANCH)
        || (worktree_record.detached && worktree_record.head_sha == context.akra_head)
    {
        let branch_label = if worktree_record.detached {
            format!("{AKRA_BRANCH} (detached)")
        } else {
            AKRA_BRANCH.to_string()
        };

        if let Some(slot_lease) = slot_lease {
            return ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                branch_label,
                annotate_worktree_label(base_worktree_label, "lease exists on idle baseline"),
                slot_lease.owner_label(),
            );
        }

        return if slot_status.is_clean_baseline() {
            ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Idle,
                branch_label,
                base_worktree_label,
                "idle baseline",
            )
        } else {
            ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                branch_label,
                annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                "operator recovery",
            )
        };
    }

    if let Some(branch_name) = worktree_record.branch_name.as_deref() {
        let expected_agent_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
        if branch_name.starts_with(&expected_agent_prefix) {
            if slot_status.has_pending_operation {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                    slot_lease
                        .map(ParallelModeSlotLeaseSnapshot::owner_label)
                        .unwrap_or_else(|| "operator recovery".to_string()),
                );
            }
            if slot_lease.is_none()
                && slot_status.is_clean_baseline()
                && branch_is_cleanup_ready(&context.repo_root, branch_name)
            {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::AwaitingCleanup,
                    branch_name,
                    annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                    slot_lease
                        .map(ParallelModeSlotLeaseSnapshot::owner_label)
                        .unwrap_or_else(|| "cleanup pending".to_string()),
                );
            }

            let Some(slot_lease) = slot_lease else {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(
                        base_worktree_label,
                        &orphan_agent_branch_without_lease_detail(
                            &context.repo_root,
                            branch_name,
                            slot_status,
                        ),
                    ),
                    "operator recovery",
                );
            };
            if slot_lease.branch_name != branch_name {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(
                        base_worktree_label,
                        "lease branch does not match worktree branch",
                    ),
                    slot_lease.owner_label(),
                );
            }
            if slot_lease.worktree_path != slot_path.display().to_string() {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(
                        base_worktree_label,
                        "lease worktree path does not match slot path",
                    ),
                    slot_lease.owner_label(),
                );
            }

            return ParallelModePoolSlotSnapshot::new(
                slot_id,
                match slot_lease.state {
                    ParallelModeSlotLeaseState::Leased => ParallelModePoolSlotState::Leased,
                    ParallelModeSlotLeaseState::Running => ParallelModePoolSlotState::Running,
                    ParallelModeSlotLeaseState::CleanupPending => {
                        ParallelModePoolSlotState::AwaitingCleanup
                    }
                },
                branch_name,
                annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                slot_lease.owner_label(),
            );
        }

        let detail = if branch_name.starts_with(&format!("{AKRA_AGENT_BRANCH_PREFIX}/")) {
            "agent branch belongs to a different slot"
        } else {
            "unexpected branch for pool slot"
        };

        return ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Blocked,
            branch_name,
            annotate_worktree_label(base_worktree_label, detail),
            slot_lease
                .map(ParallelModeSlotLeaseSnapshot::owner_label)
                .unwrap_or_else(|| "operator recovery".to_string()),
        );
    }

    let detached_label = format!("detached@{}", short_sha(&worktree_record.head_sha));
    ParallelModePoolSlotSnapshot::new(
        slot_id,
        ParallelModePoolSlotState::Blocked,
        detached_label,
        annotate_worktree_label(base_worktree_label, "detached away from `akra` baseline"),
        slot_lease
            .map(ParallelModeSlotLeaseSnapshot::owner_label)
            .unwrap_or_else(|| "operator recovery".to_string()),
    )
}

fn summarize_pool_reconcile_status(
    slots: &[ParallelModePoolSlotSnapshot],
    pool_root: &Path,
    execution: Option<PoolReconcileExecution>,
) -> String {
    let idle_slots = slots
        .iter()
        .filter(|slot| slot.state == ParallelModePoolSlotState::Idle)
        .count();
    let awaiting_cleanup_slots = slots
        .iter()
        .filter(|slot| slot.state == ParallelModePoolSlotState::AwaitingCleanup)
        .count();
    let blocked_slots = slots
        .iter()
        .filter(|slot| slot.state == ParallelModePoolSlotState::Blocked)
        .count();
    let missing_slots = slots
        .iter()
        .filter(|slot| slot.state == ParallelModePoolSlotState::Missing)
        .count();
    let mut prefix = String::new();
    if let Some(execution) = execution.filter(|execution| execution.has_actions()) {
        let mut action_parts = Vec::new();
        if execution.created_akra_branch {
            action_parts.push("created `akra`".to_string());
        }
        if execution.created_pool_root {
            action_parts.push("created pool root".to_string());
        }
        if execution.provisioned_slots > 0 {
            action_parts.push(format!("provisioned {}", execution.provisioned_slots));
        }
        if execution.cleaned_slots > 0 {
            action_parts.push(format!("cleaned {}", execution.cleaned_slots));
        }
        prefix = format!("actions: {} / ", action_parts.join(", "));
    }

    if blocked_slots > 0 {
        if let Some(slot) = find_non_merged_orphan_slot_branch(slots) {
            return format!(
                "{}reconcile blocked / cause: {} / blocked: {blocked_slots} / missing: {missing_slots} / cleanup: {awaiting_cleanup_slots} / root {}",
                prefix,
                non_merged_orphan_slot_branch_notice(&slot.slot_id, &slot.branch_name),
                pool_root.display()
            );
        }
        return format!(
            "{}reconcile blocked / blocked: {blocked_slots} / missing: {missing_slots} / cleanup: {awaiting_cleanup_slots} / root {}",
            prefix,
            pool_root.display()
        );
    }

    if missing_slots > 0 && awaiting_cleanup_slots > 0 {
        return format!(
            "{}reconcile pending / missing: {missing_slots} / cleanup pending: {awaiting_cleanup_slots} / root {}",
            prefix,
            pool_root.display()
        );
    }

    if missing_slots > 0 {
        return format!(
            "{}reconcile pending / create {missing_slots} missing slot(s) under {}",
            prefix,
            pool_root.display()
        );
    }

    if awaiting_cleanup_slots > 0 {
        return format!(
            "{}cleanup pending / {awaiting_cleanup_slots} slot(s) still need reset to `{AKRA_BRANCH}`",
            prefix
        );
    }

    if idle_slots == slots.len() && !slots.is_empty() {
        return format!(
            "{}reconcile complete / all slots are clean on `{AKRA_BRANCH}` baseline",
            prefix
        );
    }

    format!(
        "{}reconcile complete / pool root {}",
        prefix,
        pool_root.display()
    )
}

fn orphan_agent_branch_without_lease_detail(
    repo_root: &str,
    branch_name: &str,
    slot_status: SlotGitStatus,
) -> String {
    let mut parts = Vec::new();
    if branch_is_cleanup_ready(repo_root, branch_name) {
        parts.push("cleanup-ready agent branch has no lease metadata".to_string());
    } else {
        parts.push(NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL.to_string());
    }
    if !slot_status.is_clean_baseline() {
        parts.push(slot_status.detail_label());
    }

    parts.join(" / ")
}

fn find_non_merged_orphan_slot_branch(
    slots: &[ParallelModePoolSlotSnapshot],
) -> Option<&ParallelModePoolSlotSnapshot> {
    slots.iter().find(|slot| {
        slot.state == ParallelModePoolSlotState::Blocked
            && slot.owner_label == "operator recovery"
            && slot
                .worktree_label
                .contains(NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL)
    })
}

fn pool_operator_recovery_notice(pool: &ParallelModePoolBoardSnapshot) -> Option<String> {
    let slot = find_non_merged_orphan_slot_branch(&pool.slots)?;
    Some(format!(
        "pool: blocked / cause: {}",
        non_merged_orphan_slot_branch_notice(&slot.slot_id, &slot.branch_name)
    ))
}

fn non_merged_orphan_slot_branch_notice(slot_id: &str, branch_name: &str) -> String {
    format!(
        "{slot_id} branch `{branch_name}` is not integrated into `{AKRA_BRANCH}` and has no lease metadata / next action: {NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_NEXT_ACTION}"
    )
}

fn detect_canonical_repo_root(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> Option<PathBuf> {
    planning_authority
        .resolve_authority_location(workspace_dir)
        .ok()
        .map(|location| PathBuf::from(location.canonical_repo_root))
}

fn derive_pool_root_label(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> String {
    detect_canonical_repo_root(planning_authority, workspace_dir)
        .map(|canonical_repo_root| {
            let pool_root = derive_default_pool_root(&canonical_repo_root);
            display_pool_path(&canonical_repo_root, &pool_root)
        })
        .unwrap_or_else(|| "not available".to_string())
}

fn derive_default_pool_root(canonical_repo_root: &Path) -> PathBuf {
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

fn resolve_akra_baseline_head(repo_root: &str) -> Option<String> {
    resolve_branch_head(repo_root, AKRA_BRANCH)
        .or_else(|| resolve_branch_head(repo_root, "refs/remotes/origin/akra"))
        .or_else(|| {
            run_command(
                "git",
                ["-C", repo_root, "rev-parse", "--verify", "HEAD"],
                None,
            )
        })
}

fn resolve_branch_head(repo_root: &str, branch_name: &str) -> Option<String> {
    run_command("git", ["-C", repo_root, "rev-parse", branch_name], None)
}

fn parse_worktree_records(output: &str) -> Vec<GitWorktreeRecord> {
    #[derive(Default)]
    struct Builder {
        path: Option<PathBuf>,
        head_sha: Option<String>,
        branch_name: Option<String>,
        detached: bool,
    }

    impl Builder {
        fn build(self) -> Option<GitWorktreeRecord> {
            Some(GitWorktreeRecord {
                path: self.path?,
                head_sha: self.head_sha.unwrap_or_default(),
                branch_name: self.branch_name,
                detached: self.detached,
            })
        }
    }

    let mut records = Vec::new();
    let mut current = Builder::default();

    for line in output.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let Some(record) = std::mem::take(&mut current).build() {
                records.push(record);
            }
            continue;
        }

        if let Some(path) = line.strip_prefix("worktree ") {
            current.path = Some(PathBuf::from(path));
            continue;
        }
        if let Some(head_sha) = line.strip_prefix("HEAD ") {
            current.head_sha = Some(head_sha.to_string());
            continue;
        }
        if let Some(branch_name) = line.strip_prefix("branch refs/heads/") {
            current.branch_name = Some(branch_name.to_string());
            continue;
        }
        if line == "detached" {
            current.detached = true;
        }
    }

    records
}

fn inspect_slot_git_status(slot_path: &Path) -> Option<SlotGitStatus> {
    let slot_path_string = slot_path.display().to_string();
    let status_output = run_command(
        "git",
        [
            "-C",
            slot_path_string.as_str(),
            "status",
            "--porcelain=v1",
            "--branch",
            "--untracked-files=all",
        ],
        None,
    )?;

    let mut status = SlotGitStatus::default();
    for line in status_output.lines().skip(1) {
        if line.starts_with("??") {
            status.has_untracked = true;
            continue;
        }

        let x = line.chars().next().unwrap_or(' ');
        let y = line.chars().nth(1).unwrap_or(' ');
        if x != ' ' {
            status.has_staged = true;
        }
        if y != ' ' {
            status.has_unstaged = true;
        }
    }

    let git_dir = resolve_git_dir(slot_path)?;
    status.has_pending_operation = [
        "MERGE_HEAD",
        "REBASE_HEAD",
        "rebase-merge",
        "rebase-apply",
        "CHERRY_PICK_HEAD",
    ]
    .into_iter()
    .any(|path| git_dir.join(path).exists());

    Some(status)
}

fn resolve_git_dir(slot_path: &Path) -> Option<PathBuf> {
    let slot_path_string = slot_path.display().to_string();
    let git_dir = run_command(
        "git",
        ["-C", slot_path_string.as_str(), "rev-parse", "--git-dir"],
        None,
    )?;
    Some(absolutize_path(slot_path, Path::new(&git_dir)))
}

fn absolutize_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn display_pool_path(canonical_repo_root: &Path, path: &Path) -> String {
    let display_root = canonical_repo_root.parent().unwrap_or(canonical_repo_root);
    path.strip_prefix(display_root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn annotate_worktree_label(base_label: String, detail: &str) -> String {
    if detail.is_empty() || detail == "clean" {
        base_label
    } else {
        format!("{base_label} / {detail}")
    }
}

fn canonicalize_best_effort(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn worktree_paths_match(left: &Path, right: &Path) -> bool {
    canonicalize_best_effort(left) == canonicalize_best_effort(right)
}

fn slot_id(slot_number: usize) -> String {
    format!("slot-{slot_number}")
}

fn short_sha(commit_sha: &str) -> String {
    commit_sha.chars().take(7).collect::<String>()
}

fn slot_leases_root(pool_root: &Path) -> PathBuf {
    pool_root.join(".leases")
}

fn slot_lease_file_path(pool_root: &Path, slot_id: &str) -> PathBuf {
    slot_leases_root(pool_root).join(format!("{slot_id}.json"))
}

fn read_slot_leases(
    pool_root: &Path,
) -> (
    BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
    BTreeSet<String>,
) {
    let leases_root = slot_leases_root(pool_root);
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

fn write_slot_lease(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<(), String> {
    planning_authority
        .upsert_runtime_slot_lease(workspace_dir, lease)
        .map_err(|error| format!("failed to store slot lease `{}`: {error}", lease.slot_id))?;

    let leases_root = slot_leases_root(pool_root);
    ensure_directory_exists(&leases_root)
        .map_err(|error| format!("failed to create lease directory: {error}"))?;
    let lease_path = slot_lease_file_path(pool_root, &lease.slot_id);
    let temp_path = lease_path.with_extension("tmp");
    let lease_body = serde_json::to_string_pretty(lease)
        .map_err(|error| format!("failed to serialize slot lease: {error}"))?;
    fs::write(&temp_path, lease_body).map_err(|error| {
        format!(
            "failed to write temporary slot lease `{}`: {error}",
            lease.slot_id
        )
    })?;
    fs::rename(&temp_path, &lease_path)
        .map_err(|error| format!("failed to persist slot lease `{}`: {error}", lease.slot_id))
}

fn remove_slot_lease(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    slot_id: &str,
) -> bool {
    if planning_authority
        .remove_runtime_slot_lease(workspace_dir, slot_id)
        .is_err()
    {
        return false;
    }
    let lease_path = slot_lease_file_path(pool_root, slot_id);
    !lease_path.exists() || fs::remove_file(lease_path).is_ok()
}

fn resolve_workspace_head_sha(workspace_path: &Path) -> Option<String> {
    let workspace = workspace_path.display().to_string();
    run_command("git", ["-C", workspace.as_str(), "rev-parse", "HEAD"], None)
}

fn current_timestamp() -> String {
    Utc::now().to_rfc3339()
}

fn normalized_optional_text(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|value| !value.is_empty())
}

fn completion_summary_from_text(
    final_response_text: Option<&str>,
    failure_context: Option<&str>,
) -> String {
    if let Some(summary) = final_response_text
        .and_then(first_non_empty_line)
        .filter(|summary| !summary.is_empty())
    {
        return summary.to_string();
    }
    if let Some(context) = failure_context {
        return format!("agent session finished with follow-up context: {context}");
    }

    "agent session reported completion without a structured final summary".to_string()
}

fn first_non_empty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn allocate_agent_branch_name(
    repo_root: &str,
    slot_id: &str,
    task_slug: &str,
    task_id: &str,
    task_title: &str,
) -> String {
    let sanitized_slug = sanitize_task_slug(task_slug)
        .or_else(|| sanitize_task_slug(task_id))
        .or_else(|| sanitize_task_slug(task_title))
        .unwrap_or_else(|| "task".to_string());
    let mut collision_index = 1usize;
    loop {
        let candidate = build_agent_branch_name(slot_id, &sanitized_slug, collision_index);
        if !branch_exists(repo_root, &candidate) {
            return candidate;
        }
        collision_index += 1;
    }
}

fn build_agent_branch_name(slot_id: &str, sanitized_slug: &str, collision_index: usize) -> String {
    let collision_suffix = if collision_index > 1 {
        format!("-{collision_index}")
    } else {
        String::new()
    };
    let bounded_slug = bounded_agent_branch_slug(
        sanitized_slug,
        MAX_AGENT_BRANCH_SLUG_LEN.saturating_sub(collision_suffix.len()),
    );
    format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/{bounded_slug}{collision_suffix}")
}

fn bounded_agent_branch_slug(slug: &str, max_len: usize) -> String {
    if slug.len() <= max_len {
        return slug.to_string();
    }

    let hash = short_branch_slug_hash(slug);
    if max_len <= hash.len() {
        return hash[..max_len].to_string();
    }

    let prefix_len = max_len.saturating_sub(hash.len() + 1);
    let prefix = truncate_to_char_boundary(slug, prefix_len).trim_end_matches('-');
    if prefix.is_empty() {
        return hash[..max_len.min(hash.len())].to_string();
    }

    format!("{prefix}-{hash}")
}

fn short_branch_slug_hash(input: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..AGENT_BRANCH_TRUNCATION_HASH_LEN].to_string()
}

fn truncate_to_char_boundary(value: &str, max_len: usize) -> &str {
    if value.len() <= max_len {
        return value;
    }

    let mut boundary = 0usize;
    for (index, character) in value.char_indices() {
        let next_boundary = index + character.len_utf8();
        if next_boundary > max_len {
            break;
        }
        boundary = next_boundary;
    }

    &value[..boundary]
}

fn sanitize_task_slug(input: &str) -> Option<String> {
    let mut slug = String::new();
    let mut previous_was_dash = false;

    for ch in input.chars() {
        let normalized = ch.to_ascii_lowercase();
        if normalized.is_ascii_alphanumeric() {
            slug.push(normalized);
            previous_was_dash = false;
            continue;
        }
        if !previous_was_dash && !slug.is_empty() {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    (!slug.is_empty()).then_some(slug)
}

fn branch_exists(repo_root: &str, branch_name: &str) -> bool {
    command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch_name}"),
        ],
    )
}

fn current_branch_name(worktree_path: &Path) -> Option<String> {
    let worktree_path_string = worktree_path.display().to_string();
    run_command(
        "git",
        [
            "-C",
            worktree_path_string.as_str(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ],
        None,
    )
}

fn discard_unstarted_slot_branch(repo_root: &str, slot_path: &Path, branch_name: &str) -> bool {
    let slot_path_string = slot_path.display().to_string();
    command_succeeds(
        "git",
        [
            "-C",
            slot_path_string.as_str(),
            "checkout",
            "--detach",
            AKRA_BRANCH,
        ],
    ) && command_succeeds(
        "git",
        [
            "-C",
            slot_path_string.as_str(),
            "reset",
            "--hard",
            AKRA_BRANCH,
        ],
    ) && command_succeeds("git", ["-C", slot_path_string.as_str(), "clean", "-fdx"])
        && command_succeeds("git", ["-C", repo_root, "branch", "-D", branch_name])
}

fn default_validation_summary() -> &'static str {
    "validation summary is not recorded in runtime yet"
}

fn default_ledger_refresh_outcome() -> &'static str {
    "no official completion has been reported yet"
}

fn lease_session_key(lease: &ParallelModeSlotLeaseSnapshot) -> String {
    format!("{}@{}", lease.slot_id, lease.leased_at)
}

fn build_assigned_session_detail(
    lease: &ParallelModeSlotLeaseSnapshot,
) -> ParallelModeAgentSessionDetailSnapshot {
    ParallelModeAgentSessionDetailSnapshot::new(
        lease_session_key(lease),
        lease.agent_id.clone(),
        lease.task_id.clone(),
        lease.task_title.clone(),
        lease.slot_id.clone(),
        None,
        lease.worktree_path.clone(),
        lease.branch_name.clone(),
        lease.leased_at.clone(),
        "assigned",
        "in_progress",
        "slot lease acquired and branch reserved for launch",
        default_validation_summary(),
        default_ledger_refresh_outcome(),
        None,
        vec![ParallelModeAgentSessionHistoryEntry::new(
            "assigned",
            lease.leased_at.clone(),
            "slot lease acquired and branch reserved for launch",
        )],
        lease.leased_at.clone(),
    )
}

fn record_assigned_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    let detail = build_assigned_session_detail(lease);
    write_agent_session_detail_record(planning_authority, workspace_dir, pool_root, &detail)?;
    Ok(detail)
}

fn record_thread_prepared_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    thread_id: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.thread_id = Some(thread_id.to_string());
            detail.state_label = "starting".to_string();
            detail.completion_state_label = "in_progress".to_string();
            let summary = format!("thread prepared for the leased session / thread: {thread_id}");
            detail.latest_summary = summary.clone();
            detail.updated_at = timestamp.clone();
            push_session_history(&mut detail, "starting", timestamp, summary);
            detail
        },
    )
}

fn record_running_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = lease
                .running_started_at
                .clone()
                .unwrap_or_else(current_timestamp);
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "running".to_string();
            detail.completion_state_label = "in_progress".to_string();
            detail.latest_summary = "agent session entered the running state".to_string();
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "running",
                timestamp,
                "agent session entered the running state".to_string(),
            );
            detail
        },
    )
}

struct ReportedCompleteSessionDetailUpdate<'a> {
    completed_at: &'a str,
    final_response_summary: &'a str,
    validation_summary: &'a str,
    failure_context: Option<&'a str>,
}

fn record_reported_complete_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    update: ReportedCompleteSessionDetailUpdate<'_>,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "reported_complete".to_string();
            detail.completion_state_label = "reported_complete".to_string();
            detail.latest_summary = update.final_response_summary.to_string();
            detail.validation_summary = update.validation_summary.to_string();
            detail.ledger_refresh_outcome =
                "completion reported; official ledger refresh is pending".to_string();
            detail.distributor_outcome = None;
            detail.updated_at = update.completed_at.to_string();

            let history_summary = update.failure_context.map_or_else(
                || update.final_response_summary.to_string(),
                |context| format!("{} / context: {context}", update.final_response_summary),
            );
            push_session_history(
                &mut detail,
                "reported_complete",
                update.completed_at.to_string(),
                history_summary,
            );
            detail
        },
    )
}

fn record_ledger_refreshing_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "ledger_refreshing".to_string();
            detail.completion_state_label = "ledger_refreshing".to_string();
            detail.latest_summary =
                "completion reported and hidden planning worker is refreshing the ledger"
                    .to_string();
            detail.ledger_refresh_outcome =
                "hidden planning worker is refreshing the official task ledger".to_string();
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "ledger_refreshing",
                timestamp,
                "hidden planning worker is refreshing the official task ledger".to_string(),
            );
            detail
        },
    )
}

fn record_commit_ready_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    ledger_refresh_outcome: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "commit_ready".to_string();
            detail.completion_state_label = "commit_ready".to_string();
            detail.latest_summary =
                "official ledger refresh accepted the completion report".to_string();
            detail.ledger_refresh_outcome = ledger_refresh_outcome.trim().to_string();
            detail.distributor_outcome =
                Some("commit-ready result is waiting for distributor integration".to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "commit_ready",
                timestamp,
                "official ledger refresh accepted the completion report".to_string(),
            );
            detail
        },
    )
}

fn record_merge_queued_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "merge_queued".to_string();
            detail.completion_state_label = "merge_queued".to_string();
            detail.latest_summary =
                "commit-ready result accepted into the distributor queue".to_string();
            detail.distributor_outcome = Some(
                "distributor accepted the result and queued it for GitHub delivery".to_string(),
            );
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "merge_queued",
                timestamp,
                "distributor accepted the result and queued it for GitHub delivery".to_string(),
            );
            detail
        },
    )
}

fn record_pushing_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    summary: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "pushing".to_string();
            detail.completion_state_label = "merge_queued".to_string();
            detail.latest_summary = summary.trim().to_string();
            detail.distributor_outcome = Some(summary.trim().to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "pushing",
                timestamp,
                summary.trim().to_string(),
            );
            detail
        },
    )
}

fn record_pr_pending_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    summary: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "pr_pending".to_string();
            detail.completion_state_label = "merge_queued".to_string();
            detail.latest_summary = summary.trim().to_string();
            detail.distributor_outcome = Some(summary.trim().to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "pr_pending",
                timestamp,
                summary.trim().to_string(),
            );
            detail
        },
    )
}

fn record_merge_pending_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    summary: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "merge_pending".to_string();
            detail.completion_state_label = "merge_queued".to_string();
            detail.latest_summary = summary.trim().to_string();
            detail.distributor_outcome = Some(summary.trim().to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "merge_pending",
                timestamp,
                summary.trim().to_string(),
            );
            detail
        },
    )
}

fn record_integrating_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    summary: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "integrating".to_string();
            detail.completion_state_label = "merge_queued".to_string();
            detail.latest_summary = summary.trim().to_string();
            detail.distributor_outcome = Some(summary.trim().to_string());
            detail.updated_at = timestamp.clone();
            if let Some(last_entry) = detail.history.last_mut()
                && last_entry.state_label == "integrating"
            {
                last_entry.timestamp = timestamp;
                last_entry.summary = summary.trim().to_string();
            } else {
                push_session_history(
                    &mut detail,
                    "integrating",
                    timestamp,
                    summary.trim().to_string(),
                );
            }
            detail
        },
    )
}

fn record_distributor_failed_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    failure_detail: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "failed".to_string();
            detail.completion_state_label = "failed".to_string();
            detail.latest_summary = "distributor delivery failed".to_string();
            detail.distributor_outcome = Some(failure_detail.trim().to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "failed",
                timestamp,
                failure_detail.trim().to_string(),
            );
            detail
        },
    )
}

fn record_official_completion_failed_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    failure_detail: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "failed".to_string();
            detail.completion_state_label = "failed".to_string();
            detail.latest_summary = "official completion refresh failed".to_string();
            detail.ledger_refresh_outcome = failure_detail.trim().to_string();
            detail.distributor_outcome = Some(
                "not queued for distributor integration because official refresh failed"
                    .to_string(),
            );
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "failed",
                timestamp,
                failure_detail.trim().to_string(),
            );
            detail
        },
    )
}

fn record_cleanup_pending_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "cleanup_pending".to_string();
            detail.completion_state_label = "merged".to_string();
            detail.latest_summary =
                "agent branch is merged into akra and awaiting slot cleanup".to_string();
            detail.distributor_outcome =
                Some("branch is merged into akra and the slot is awaiting cleanup".to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "merged",
                timestamp.clone(),
                "branch is integrated into akra".to_string(),
            );
            push_session_history(
                &mut detail,
                "cleanup_pending",
                timestamp,
                "slot is waiting for cleanup before it can be reused".to_string(),
            );
            detail
        },
    )
}

fn record_cleaned_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "cleaned".to_string();
            detail.completion_state_label = "cleaned".to_string();
            detail.latest_summary =
                "merged session cleaned up and the slot returned to the idle pool".to_string();
            detail.distributor_outcome =
                Some("branch merged into akra and the slot returned to idle".to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "cleaned",
                timestamp,
                "slot cleaned and returned to the idle pool".to_string(),
            );
            detail
        },
    )
}

fn record_failed_start_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "failed".to_string();
            detail.completion_state_label = "aborted".to_string();
            detail.latest_summary =
                "launch failed before the session reached the running state".to_string();
            detail.distributor_outcome =
                Some("not queued for distributor work; slot returned to idle".to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "failed",
                timestamp.clone(),
                "launch failed before the session reached the running state".to_string(),
            );
            push_session_history(
                &mut detail,
                "cleaned",
                timestamp,
                "slot cleaned and returned to the idle pool after launch failure".to_string(),
            );
            detail
        },
    )
}

fn push_session_history(
    detail: &mut ParallelModeAgentSessionDetailSnapshot,
    state_label: &str,
    timestamp: String,
    summary: String,
) {
    if detail
        .history
        .last()
        .is_some_and(|entry| entry.state_label == state_label && entry.summary == summary)
    {
        return;
    }

    detail
        .history
        .push(ParallelModeAgentSessionHistoryEntry::new(
            state_label,
            timestamp,
            summary,
        ));
}

fn update_agent_session_detail_record<F>(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    mutate: F,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String>
where
    F: FnOnce(
        Option<ParallelModeAgentSessionDetailSnapshot>,
    ) -> ParallelModeAgentSessionDetailSnapshot,
{
    let session_key = lease_session_key(lease);
    let current = read_agent_session_detail_record(pool_root, &session_key);
    let detail = mutate(current);
    write_agent_session_detail_record(planning_authority, workspace_dir, pool_root, &detail)?;
    Ok(detail)
}

fn load_agent_session_detail_records(
    pool_root: &Path,
) -> Vec<ParallelModeAgentSessionDetailSnapshot> {
    let history_dir = agent_session_history_dir(pool_root);
    let Ok(entries) = fs::read_dir(history_dir) else {
        return Vec::new();
    };

    let mut records = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
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

fn read_agent_session_detail_record(
    pool_root: &Path,
    session_key: &str,
) -> Option<ParallelModeAgentSessionDetailSnapshot> {
    let path = agent_session_detail_record_path(pool_root, session_key);
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn write_agent_session_detail_record(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    detail: &ParallelModeAgentSessionDetailSnapshot,
) -> Result<(), String> {
    planning_authority
        .upsert_runtime_session_detail(workspace_dir, detail)
        .map_err(|error| {
            format!(
                "failed to store agent session detail `{}`: {error}",
                detail.session_key
            )
        })?;

    let history_dir = agent_session_history_dir(pool_root);
    ensure_directory_exists(&history_dir)
        .map_err(|error| format!("failed to create agent session history directory: {error}"))?;

    let path = agent_session_detail_record_path(pool_root, &detail.session_key);
    let temp_path = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(detail)
        .map_err(|error| format!("failed to serialize agent session detail: {error}"))?;
    fs::write(&temp_path, body).map_err(|error| {
        format!(
            "failed to write temporary agent session detail `{}`: {error}",
            detail.session_key
        )
    })?;
    fs::rename(&temp_path, &path).map_err(|error| {
        format!(
            "failed to persist agent session detail `{}`: {error}",
            detail.session_key
        )
    })
}

fn agent_session_history_dir(pool_root: &Path) -> PathBuf {
    pool_root.join(".agent-sessions")
}

fn agent_session_detail_record_path(pool_root: &Path, session_key: &str) -> PathBuf {
    let mut filename = String::new();
    for ch in session_key.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            filename.push(ch);
        } else {
            filename.push('_');
        }
    }

    agent_session_history_dir(pool_root).join(format!("{filename}.json"))
}

fn format_elapsed_label_from_timestamp(timestamp: &str) -> Option<String> {
    let started_at = chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()?
        .with_timezone(&Utc);
    let elapsed_seconds = Utc::now()
        .signed_duration_since(started_at)
        .num_seconds()
        .max(0);

    Some(format_elapsed_seconds(elapsed_seconds))
}

fn format_elapsed_seconds(elapsed_seconds: i64) -> String {
    if elapsed_seconds < 60 {
        return format!("{elapsed_seconds}s");
    }
    if elapsed_seconds < 60 * 60 {
        return format!("{}m", elapsed_seconds / 60);
    }
    if elapsed_seconds < 60 * 60 * 24 {
        let hours = elapsed_seconds / (60 * 60);
        let minutes = (elapsed_seconds % (60 * 60)) / 60;
        return format!("{hours}h {minutes}m");
    }

    let days = elapsed_seconds / (60 * 60 * 24);
    let hours = (elapsed_seconds % (60 * 60 * 24)) / (60 * 60);
    format!("{days}d {hours}h")
}

fn parse_https_remote(push_url: &str) -> Option<(String, String)> {
    let stripped = push_url.trim().strip_prefix("https://")?;
    let mut parts = stripped.splitn(2, '/');
    let host = parts.next()?.trim();
    let path = parts.next()?.trim();
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some((host.to_string(), path.to_string()))
}

fn command_succeeds<const N: usize>(program: &str, args: [&str; N]) -> bool {
    let mut command = Command::new(program);
    command.args(args);
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.status().is_ok_and(|status| status.success())
}

fn run_command<const N: usize>(
    program: &str,
    args: [&str; N],
    current_dir: Option<&str>,
) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    command.stderr(Stdio::null());
    command.env("GIT_TERMINAL_PROMPT", "0");

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn run_command_with_stdin<const N: usize>(
    program: &str,
    args: [&str; N],
    stdin_body: String,
) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    stdin.write_all(stdin_body.as_bytes()).ok()?;
    drop(stdin);

    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests;

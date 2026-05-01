use std::collections::BTreeSet;
use std::sync::Arc;

use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModePoolSlotState, ParallelModeReadinessSnapshot, ParallelModeReadinessState,
    ParallelModeSlotLeaseState, ParallelModeSupervisorSnapshot,
};
use crate::domain::planning::PlanningOfficialCompletionRefreshContract;
use crate::domain::planning::PriorityQueueTask;

mod branch_names;
mod completion;
pub(crate) mod distributor;
mod git_sequence;
mod orchestration;
mod pool;
mod readiness;
mod session_detail;
mod slot_lifecycle;
pub(crate) mod supervisor;
mod support;
pub(crate) mod turn;

use self::branch_names::{allocate_agent_branch_name, branch_exists};
use self::distributor::ParallelModeDistributorService;
use self::orchestration::{
    inspect_akra_integration_worktree_blocker, parallel_dispatch_excluded_task_ids,
};
use self::pool::{
    PoolBoardWithContextResult, PoolRuntimeContext, WorkspaceSlotLeaseResolution,
    acquire_pool_allocation_lock, branch_is_cleanup_ready, branch_is_integrated_into,
    build_pool_board, build_pool_slots, cleanup_slot, inspect_pool_board_and_context,
    inspect_slot_git_status, load_pool_runtime_context, pool_operator_recovery_notice,
    reconcile_pool_board, reconcile_pool_board_and_context, resolve_workspace_head_sha,
    resolve_workspace_slot_lease, short_sha, write_slot_lease,
};
use self::readiness::{
    blocked_prerequisite_capability, command_succeeds, inspect_akra_branch,
    inspect_authority_store, inspect_gh_auth, inspect_gh_binary, inspect_git_worktree,
    inspect_planning, inspect_push_remote, run_command,
};
use self::session_detail::{
    default_authority_refresh_outcome, default_validation_summary,
    format_elapsed_label_from_timestamp, lease_session_key, record_assigned_session_detail,
    record_cleaned_session_detail, record_cleanup_pending_session_detail,
    record_distributor_failed_session_detail, record_failed_start_session_detail,
    record_integrating_session_detail, record_merge_pending_session_detail,
    record_merge_queued_session_detail, record_pr_pending_session_detail,
    record_pushing_session_detail, record_running_session_detail,
    record_thread_prepared_session_detail,
};
use self::supervisor::ParallelModeSupervisorService;
pub(super) use self::support::{
    current_branch_name, current_timestamp, discard_unstarted_slot_branch, ensure_directory_exists,
};

#[cfg(test)]
use self::branch_names::{sanitize_task_slug, short_branch_slug_hash};
#[cfg(test)]
use self::pool::detect_canonical_repo_root;
#[cfg(test)]
use self::pool::{derive_default_pool_root, slot_id, slot_lease_file_path};
#[cfg(test)]
use self::readiness::parse_https_remote;
#[cfg(test)]
use self::session_detail::{agent_session_detail_record_path, read_agent_session_detail_record};
const DISTRIBUTOR_INTEGRATION_BRANCH: &str = "prerelease";
const POOL_BASELINE_BRANCH: &str = DISTRIBUTOR_INTEGRATION_BRANCH;
const DEFAULT_PUSH_REMOTE_NAME: &str = "origin";
const DEFAULT_POOL_SIZE: usize = 3;
const AKRA_AGENT_BRANCH_PREFIX: &str = "akra-agent";
const MAX_AGENT_BRANCH_SLUG_LEN: usize = 96;
const AGENT_BRANCH_TRUNCATION_HASH_LEN: usize = 10;
const NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL: &str =
    "agent branch is not integrated into `prerelease` and has no lease metadata";
const NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_NEXT_ACTION: &str =
    "inspect the slot branch, merge or discard it manually, then rerun reconcile";

pub type ParallelModeOfficialCompletionReport = PlanningOfficialCompletionRefreshContract;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeDispatchPlan {
    pub idle_slot_count: usize,
    pub excluded_task_ids: Vec<String>,
    pub candidates: Vec<PriorityQueueTask>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeOrchestratorTrigger {
    MainTurnCompleted,
    PlanningRefreshCompleted,
    ManualDispatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeOrchestratorTickResult {
    pub trigger: ParallelModeOrchestratorTrigger,
    pub blocked: bool,
    pub notices: Vec<String>,
}

#[derive(Clone)]
pub struct ParallelModeService {
    distributor_service: ParallelModeDistributorService,
    supervisor_service: ParallelModeSupervisorService,
    planning_authority: Arc<dyn PlanningAuthorityPort>,
    parallel_runtime: Arc<dyn ParallelModeRuntimePort>,
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
        parallel_runtime: Arc<dyn ParallelModeRuntimePort>,
    ) -> Self {
        Self {
            distributor_service: ParallelModeDistributorService::with_planning_authority(
                github_automation,
                planning_authority.clone(),
            ),
            supervisor_service: ParallelModeSupervisorService::new(),
            planning_authority,
            parallel_runtime,
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
        let repo_root = self.parallel_runtime.detect_git_repo_root(workspace_dir);
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
            Some(repo_root) => inspect_git_worktree(self.parallel_runtime.as_ref(), repo_root),
            None => blocked_prerequisite_capability(
                ParallelModeCapabilityKey::GitWorktree,
                "waiting for git repository detection",
                "enter a git repository first",
            ),
        };
        let akra_branch = match &repo_root {
            Some(repo_root) => inspect_akra_branch(self.parallel_runtime.as_ref(), repo_root),
            None => blocked_prerequisite_capability(
                ParallelModeCapabilityKey::AkraBranch,
                "waiting for git repository detection",
                "enter a git repository first",
            ),
        };
        let push_remote = match &repo_root {
            Some(repo_root) => inspect_push_remote(self.parallel_runtime.as_ref(), repo_root),
            None => blocked_prerequisite_capability(
                ParallelModeCapabilityKey::PushRemote,
                "waiting for git repository detection",
                "enter a git repository first",
            ),
        };
        let gh_binary = inspect_gh_binary(self.parallel_runtime.as_ref());
        let gh_auth = inspect_gh_auth(
            self.parallel_runtime.as_ref(),
            &gh_binary,
            repo_root.as_deref(),
        );
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
        let readiness = ParallelModeReadinessState::derive_from_capabilities(&capabilities);
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

    pub fn build_dispatch_plan(
        &self,
        workspace_dir: &str,
        planning_snapshot: &PlanningRuntimeSnapshot,
        requested_count: usize,
    ) -> Result<ParallelModeDispatchPlan, String> {
        let _ = reconcile_pool_board(self.planning_authority.as_ref(), workspace_dir);
        let context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;
        let idle_slot_count = build_pool_slots(&context)
            .into_iter()
            .filter(|slot| slot.state == ParallelModePoolSlotState::Idle)
            .count();
        let capacity = requested_count.min(idle_slot_count);
        let excluded_task_ids = parallel_dispatch_excluded_task_ids(&context);
        let excluded = excluded_task_ids
            .iter()
            .map(|task_id| task_id.trim().to_string())
            .collect::<BTreeSet<_>>();
        let candidates = planning_snapshot
            .queue_projection()
            .map(|projection| {
                projection
                    .active_tasks
                    .iter()
                    .filter(|task| !excluded.contains(task.task_id.trim()))
                    .take(capacity)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(ParallelModeDispatchPlan {
            idle_slot_count,
            excluded_task_ids,
            candidates,
        })
    }

    pub fn run_orchestrator_tick(
        &self,
        workspace_dir: &str,
        trigger: ParallelModeOrchestratorTrigger,
    ) -> Result<ParallelModeOrchestratorTickResult, String> {
        if let Some(blocked_notice) = inspect_akra_integration_worktree_blocker(
            self.planning_authority.as_ref(),
            workspace_dir,
        ) {
            return Ok(ParallelModeOrchestratorTickResult {
                trigger,
                blocked: true,
                notices: vec![blocked_notice],
            });
        }

        let notices = self.distributor_service.process_queue(workspace_dir)?;
        Ok(ParallelModeOrchestratorTickResult {
            trigger,
            blocked: false,
            notices,
        })
    }
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

#[cfg(test)]
mod tests;

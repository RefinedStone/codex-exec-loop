use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;

use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeCapabilityKey,
    ParallelModeCapabilitySnapshot, ParallelModeCapabilityState, ParallelModePoolSlotState,
    ParallelModeReadinessSnapshot, ParallelModeReadinessState, ParallelModeSlotLeaseRequest,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState, ParallelModeSupervisorSnapshot,
};
use crate::domain::planning::PlanningOfficialCompletionRefreshContract;
use crate::domain::planning::PriorityQueueTask;

mod completion;
pub(crate) mod distributor;
mod git_sequence;
mod pool;
mod readiness;
mod session_detail;
pub(crate) mod supervisor;
pub(crate) mod turn;

use self::distributor::ParallelModeDistributorService;
use self::git_sequence::{GitCommandStep, run_git_sequence};
use self::pool::{
    PoolBoardWithContextResult, PoolRuntimeContext, WorkspaceSlotLeaseResolution,
    branch_is_cleanup_ready, branch_is_integrated_into, build_pool_board, build_pool_slots,
    cleanup_slot, detect_canonical_repo_root, inspect_pool_board_and_context,
    inspect_slot_git_status, load_pool_runtime_context, pool_operator_recovery_notice,
    reconcile_pool_board, reconcile_pool_board_and_context, reset_slot_worktree_to_akra,
    resolve_workspace_head_sha, resolve_workspace_slot_lease, short_sha, write_slot_lease,
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

#[cfg(test)]
use self::pool::{derive_default_pool_root, slot_id, slot_lease_file_path};
#[cfg(test)]
use self::readiness::parse_https_remote;
#[cfg(test)]
use self::session_detail::{agent_session_detail_record_path, read_agent_session_detail_record};
const AKRA_BRANCH: &str = "akra";
const DISTRIBUTOR_INTEGRATION_BRANCH: &str = "prerelease";
const DEFAULT_PUSH_REMOTE_NAME: &str = "origin";
const DEFAULT_POOL_SIZE: usize = 3;
const AKRA_AGENT_BRANCH_PREFIX: &str = "akra-agent";
const MAX_AGENT_BRANCH_SLUG_LEN: usize = 96;
const AGENT_BRANCH_TRUNCATION_HASH_LEN: usize = 10;
const NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL: &str =
    "agent branch is not integrated into `akra` and has no lease metadata";
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

fn parallel_dispatch_excluded_task_ids(context: &PoolRuntimeContext) -> Vec<String> {
    let mut task_ids = BTreeSet::new();
    task_ids.extend(
        context
            .slot_leases
            .values()
            .map(|lease| lease.task_id.trim().to_string())
            .filter(|task_id| !task_id.is_empty()),
    );
    task_ids.extend(
        context
            .distributor_queue_records
            .iter()
            .filter(|record| record.queue_state.is_active())
            .map(|record| record.task_id.trim().to_string())
            .filter(|task_id| !task_id.is_empty()),
    );

    task_ids.into_iter().collect()
}

fn inspect_akra_integration_worktree_blocker(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> Option<String> {
    let canonical_repo_root = detect_canonical_repo_root(planning_authority, workspace_dir)?;
    let branch_name = current_branch_name(&canonical_repo_root)?;
    if branch_name != DISTRIBUTOR_INTEGRATION_BRANCH {
        return Some(format!(
            "orchestrator blocked / integration worktree must be checked out to `{DISTRIBUTOR_INTEGRATION_BRANCH}` but is `{branch_name}`"
        ));
    }

    let status = inspect_slot_git_status(&canonical_repo_root)?;
    if !status.is_ready_for_integration() {
        return Some(format!(
            "orchestrator blocked / integration worktree must be clean before queue processing: {}",
            status.detail_label()
        ));
    }

    None
}

fn ensure_directory_exists(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }

    fs::create_dir_all(path)
}

fn current_timestamp() -> String {
    Utc::now().to_rfc3339()
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
    reset_slot_worktree_to_akra(slot_path).succeeded()
        && run_git_sequence(
            "delete unstarted slot branch",
            vec![GitCommandStep::new(
                "delete agent branch",
                ["-C", repo_root, "branch", "-D", branch_name],
            )],
        )
        .succeeded()
}

#[cfg(test)]
mod tests;

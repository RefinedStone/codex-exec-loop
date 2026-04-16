use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::Utc;

use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot,
    ParallelModeCapabilityState, ParallelModeCompletionFeedEntry, ParallelModeDistributorSnapshot,
    ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
    ParallelModeQueueItemState, ParallelModeReadinessSnapshot, ParallelModeReadinessState,
    ParallelModeSlotLeaseRequest, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
    ParallelModeSupervisorSnapshot, ParallelModeSupervisorState,
};

const AKRA_BRANCH: &str = "akra";
const DEFAULT_PUSH_REMOTE_NAME: &str = "origin";
const DEFAULT_POOL_SIZE: usize = 3;
const AKRA_AGENT_BRANCH_PREFIX: &str = "akra-agent";

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
}

#[derive(Debug, Clone, Default)]
pub struct ParallelModeService;

impl ParallelModeService {
    pub fn new() -> Self {
        Self
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

        let capabilities = vec![
            git_repository,
            git_worktree,
            akra_branch,
            push_remote,
            gh_binary,
            gh_auth,
            planning,
        ];
        let readiness = derive_readiness(&capabilities);
        let top_alert = capabilities
            .iter()
            .find(|capability| capability.state != ParallelModeCapabilityState::Ready)
            .map(ParallelModeCapabilitySnapshot::summary);

        ParallelModeReadinessSnapshot::new(workspace_dir, readiness, capabilities, top_alert)
    }

    pub fn build_supervisor_snapshot(
        &self,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    ) -> ParallelModeSupervisorSnapshot {
        let state = derive_supervisor_state(mode_enabled, readiness_snapshot);
        let workspace_path = readiness_snapshot
            .map(|snapshot| snapshot.workspace_path.clone())
            .unwrap_or_else(|| workspace_dir.to_string());
        let top_notice = readiness_snapshot
            .and_then(|snapshot| snapshot.top_alert.clone())
            .or_else(|| default_supervisor_notice(mode_enabled, readiness_snapshot));

        ParallelModeSupervisorSnapshot::new(
            state,
            workspace_path,
            build_pool_board(workspace_dir, readiness_snapshot),
            build_placeholder_roster(mode_enabled, readiness_snapshot),
            build_placeholder_distributor(mode_enabled, readiness_snapshot),
            top_notice,
        )
    }

    pub fn reconcile_supervisor_snapshot(
        &self,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    ) -> ParallelModeSupervisorSnapshot {
        let state = derive_supervisor_state(mode_enabled, readiness_snapshot);
        let workspace_path = readiness_snapshot
            .map(|snapshot| snapshot.workspace_path.clone())
            .unwrap_or_else(|| workspace_dir.to_string());
        let top_notice = readiness_snapshot
            .and_then(|snapshot| snapshot.top_alert.clone())
            .or_else(|| default_supervisor_notice(mode_enabled, readiness_snapshot));
        let pool = match readiness_snapshot {
            Some(snapshot) if mode_enabled && snapshot.allows_parallel_mode() => {
                reconcile_pool_board(workspace_dir)
            }
            _ => build_pool_board(workspace_dir, readiness_snapshot),
        };

        ParallelModeSupervisorSnapshot::new(
            state,
            workspace_path,
            pool,
            build_placeholder_roster(mode_enabled, readiness_snapshot),
            build_placeholder_distributor(mode_enabled, readiness_snapshot),
            top_notice,
        )
    }

    pub fn acquire_slot_lease(
        &self,
        workspace_dir: &str,
        request: ParallelModeSlotLeaseRequest,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let _ = reconcile_pool_board(workspace_dir);
        let context =
            load_pool_runtime_context(workspace_dir).map_err(|(_, detail)| detail.to_string())?;

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
        if let Err(error) = write_slot_lease(&context.pool_root, &lease) {
            let _ =
                discard_unstarted_slot_branch(&context.repo_root, &slot_path, branch_name.as_str());
            return Err(error);
        }

        Ok(lease)
    }

    pub fn mark_slot_running(
        &self,
        workspace_dir: &str,
        slot_id: &str,
        agent_id: &str,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let context =
            load_pool_runtime_context(workspace_dir).map_err(|(_, detail)| detail.to_string())?;
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
        write_slot_lease(&context.pool_root, &lease)?;
        Ok(lease)
    }

    pub fn mark_slot_cleanup_pending(
        &self,
        workspace_dir: &str,
        slot_id: &str,
        agent_id: &str,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let context =
            load_pool_runtime_context(workspace_dir).map_err(|(_, detail)| detail.to_string())?;
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
        write_slot_lease(&context.pool_root, &lease)?;
        Ok(lease)
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

    if !snapshot.plan_enabled() {
        return ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::Planning,
            ParallelModeCapabilityState::Blocked,
            "planning mode is off for this workspace",
            Some("run `:planning on` before assigning work in parallel mode".to_string()),
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
    workspace_dir: &str,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModePoolBoardSnapshot {
    match readiness_snapshot {
        Some(snapshot) if snapshot.allows_parallel_mode() => inspect_pool_board(workspace_dir),
        Some(snapshot) => build_unavailable_pool_board(
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
            workspace_dir,
            "reconcile pending / run readiness first",
            "not inspected",
            "readiness has not been checked",
            "n/a",
        ),
    }
}

fn reconcile_pool_board(workspace_dir: &str) -> ParallelModePoolBoardSnapshot {
    let Some(repo_root) = detect_git_repo_root(workspace_dir) else {
        return build_blocked_pool_board(
            workspace_dir,
            "reconcile failed / git repository is unavailable",
            "repository inspection failed",
        );
    };
    let Some(canonical_repo_root) = detect_canonical_repo_root(workspace_dir) else {
        return build_blocked_pool_board(
            workspace_dir,
            "reconcile failed / canonical repository root is unavailable",
            "canonical root inspection failed",
        );
    };
    let Ok((_akra_head, created_akra_branch)) = ensure_akra_branch(&repo_root) else {
        return build_blocked_pool_board(
            workspace_dir,
            "reconcile blocked / `akra` baseline could not be created",
            "`akra` is unavailable during reconcile",
        );
    };

    let pool_root = derive_default_pool_root(&canonical_repo_root);
    let pool_root_existed = pool_root.exists();
    if ensure_directory_exists(&pool_root).is_err() {
        return build_blocked_pool_board(
            workspace_dir,
            "reconcile failed / pool root could not be created",
            "pool root creation failed",
        );
    }
    let created_pool_root = !pool_root_existed;
    let Some(worktree_records) = load_worktree_records(&repo_root) else {
        return build_blocked_pool_board(
            workspace_dir,
            "reconcile failed / git worktree inventory could not be loaded",
            "worktree list inspection failed",
        );
    };

    let provisioned_slots = provision_missing_slots(&repo_root, &pool_root, &worktree_records);
    let Some(reloaded_worktree_records) = load_worktree_records(&repo_root) else {
        return build_blocked_pool_board(
            workspace_dir,
            "reconcile failed / git worktree inventory could not be reloaded",
            "worktree list reload failed",
        );
    };
    let cleaned_slots = cleanup_reusable_slots(&repo_root, &pool_root, &reloaded_worktree_records);

    let Ok(context) = load_pool_runtime_context_from_roots(&repo_root, &canonical_repo_root) else {
        return build_blocked_pool_board(
            workspace_dir,
            "reconcile failed / pool runtime state could not be loaded",
            "pool runtime load failed",
        );
    };

    build_pool_board_from_context(
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
    )
}

fn inspect_pool_board(workspace_dir: &str) -> ParallelModePoolBoardSnapshot {
    match load_pool_runtime_context(workspace_dir) {
        Ok(context) => build_pool_board_from_context(
            &context,
            summarize_pool_reconcile_status(&build_pool_slots(&context), &context.pool_root, None),
        ),
        Err((reconcile_status, detail)) => {
            build_blocked_pool_board(workspace_dir, reconcile_status, detail)
        }
    }
}

fn ensure_akra_branch(repo_root: &str) -> Result<(String, bool), ()> {
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
    workspace_dir: &str,
) -> Result<PoolRuntimeContext, (&'static str, &'static str)> {
    let Some(repo_root) = detect_git_repo_root(workspace_dir) else {
        return Err((
            "reconcile failed / git repository is unavailable",
            "repository inspection failed",
        ));
    };
    let Some(canonical_repo_root) = detect_canonical_repo_root(workspace_dir) else {
        return Err((
            "reconcile failed / canonical repository root is unavailable",
            "canonical root inspection failed",
        ));
    };

    load_pool_runtime_context_from_roots(&repo_root, &canonical_repo_root).map_err(|detail| {
        (
            "reconcile failed / pool runtime state could not be loaded",
            detail,
        )
    })
}

fn load_pool_runtime_context_from_roots(
    repo_root: &str,
    canonical_repo_root: &Path,
) -> Result<PoolRuntimeContext, &'static str> {
    let Some(akra_head) = resolve_branch_head(repo_root, AKRA_BRANCH) else {
        return Err("`akra` is unavailable during inspection");
    };
    let Some(worktree_records) = load_worktree_records(repo_root) else {
        return Err("worktree list inspection failed");
    };
    let pool_root = derive_default_pool_root(canonical_repo_root);
    let (slot_leases, invalid_slot_leases) = read_slot_leases(&pool_root);

    Ok(PoolRuntimeContext {
        repo_root: repo_root.to_string(),
        canonical_repo_root: canonical_repo_root.to_path_buf(),
        pool_root,
        akra_head,
        worktree_records,
        slot_leases,
        invalid_slot_leases,
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

fn cleanup_reusable_slots(
    repo_root: &str,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
) -> usize {
    let mut cleaned_slots = 0;
    let (slot_leases, _) = read_slot_leases(pool_root);

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
            Some(ParallelModeSlotLeaseState::CleanupPending) => true,
            Some(ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running) => false,
            None => {
                inspect_slot_git_status(&slot_path).is_some_and(SlotGitStatus::is_clean_baseline)
                    && branch_is_cleanup_ready(repo_root, branch_name)
            }
        };
        if !cleanup_ready {
            continue;
        }
        if cleanup_slot(repo_root, pool_root, &slot_id, &slot_path, branch_name) {
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
    if !remove_slot_lease(pool_root, slot_id) {
        return false;
    }

    inspect_slot_git_status(slot_path).is_some_and(SlotGitStatus::is_clean_baseline)
}

fn build_unavailable_pool_board(
    workspace_dir: &str,
    reconcile_status: impl Into<String>,
    branch_name: &str,
    worktree_label: &str,
    owner_label: &str,
) -> ParallelModePoolBoardSnapshot {
    let pool_root_label = derive_pool_root_label(workspace_dir);
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
    workspace_dir: &str,
    reconcile_status: impl Into<String>,
    detail: &str,
) -> ParallelModePoolBoardSnapshot {
    let pool_root_label = derive_pool_root_label(workspace_dir);
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
                        "agent branch exists without lease",
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

fn detect_canonical_repo_root(workspace_dir: &str) -> Option<PathBuf> {
    let repo_root = detect_git_repo_root(workspace_dir)?;
    let common_dir = run_command(
        "git",
        ["-C", workspace_dir, "rev-parse", "--git-common-dir"],
        None,
    )?;
    let common_dir_path = absolutize_path(Path::new(&repo_root), Path::new(&common_dir));
    let canonical_repo_root = common_dir_path.parent()?;

    fs::canonicalize(canonical_repo_root)
        .ok()
        .or_else(|| Some(canonical_repo_root.to_path_buf()))
}

fn derive_pool_root_label(workspace_dir: &str) -> String {
    detect_canonical_repo_root(workspace_dir)
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

fn write_slot_lease(pool_root: &Path, lease: &ParallelModeSlotLeaseSnapshot) -> Result<(), String> {
    let leases_root = slot_leases_root(pool_root);
    ensure_directory_exists(&leases_root)
        .map_err(|error| format!("failed to create lease directory: {error}"))?;
    let lease_path = slot_lease_file_path(pool_root, &lease.slot_id);
    let lease_body = serde_json::to_string_pretty(lease)
        .map_err(|error| format!("failed to serialize slot lease: {error}"))?;
    fs::write(&lease_path, lease_body)
        .map_err(|error| format!("failed to persist slot lease `{}`: {error}", lease.slot_id))
}

fn remove_slot_lease(pool_root: &Path, slot_id: &str) -> bool {
    let lease_path = slot_lease_file_path(pool_root, slot_id);
    !lease_path.exists() || fs::remove_file(lease_path).is_ok()
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
    let base_slug = sanitize_task_slug(task_slug)
        .or_else(|| sanitize_task_slug(task_id))
        .or_else(|| sanitize_task_slug(task_title))
        .unwrap_or_else(|| "task".to_string());
    let base_branch_name = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/{base_slug}");

    let mut candidate = base_branch_name.clone();
    let mut suffix = 2;
    while branch_exists(repo_root, &candidate) {
        candidate = format!("{base_branch_name}-{suffix}");
        suffix += 1;
    }

    candidate
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

fn build_placeholder_roster(
    mode_enabled: bool,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModeAgentRosterSnapshot {
    let empty_state = match (mode_enabled, readiness_snapshot) {
        (true, Some(snapshot)) if snapshot.allows_parallel_mode() => {
            "no agent sessions launched in this slice"
        }
        (true, Some(_)) => "readiness must recover before agent launch is allowed",
        (true, None) => "rerun readiness before agent launch is available",
        (false, Some(_)) => "parallel mode is off / agent roster is read-only",
        (false, None) => "parallel mode is off / no supervisor roster loaded",
    };

    ParallelModeAgentRosterSnapshot::new(Vec::new(), empty_state)
}

fn build_placeholder_distributor(
    mode_enabled: bool,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModeDistributorSnapshot {
    let (head_summary, note) = match (mode_enabled, readiness_snapshot) {
        (true, Some(snapshot)) if snapshot.allows_parallel_mode() => (
            ParallelModeQueueItemState::Idle.label(),
            "queue is read-only until distributor runtime lands",
        ),
        (true, Some(_)) => (
            "paused",
            "distributor waits for readiness recovery before queue processing",
        ),
        (true, None) => (
            "pending",
            "rerun readiness before distributor state can be trusted",
        ),
        (false, Some(_)) => (
            "inactive",
            "enable parallel mode to surface live distributor activity",
        ),
        (false, None) => ("inactive", "parallel mode is off"),
    };

    ParallelModeDistributorSnapshot::new(
        Vec::new(),
        vec![
            ParallelModeCompletionFeedEntry::new("reported", "no agent results reported yet"),
            ParallelModeCompletionFeedEntry::new(
                "ledger refreshing",
                "no official refresh workers are active",
            ),
            ParallelModeCompletionFeedEntry::new("official", "nothing is queued for merge"),
        ],
        head_summary,
        note,
    )
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
mod tests {
    use super::{
        DEFAULT_POOL_SIZE, ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot,
        ParallelModeCapabilityState, ParallelModeReadinessSnapshot, ParallelModeReadinessState,
        ParallelModeService, ParallelModeSupervisorState, build_pool_board,
        derive_default_pool_root, derive_readiness, inspect_planning, parse_https_remote,
        reconcile_pool_board, slot_id, slot_lease_file_path,
    };
    use crate::application::service::planning::PlanningRuntimeSnapshot;
    use crate::domain::parallel_mode::{
        ParallelModePoolSlotState, ParallelModeSlotLeaseRequest, ParallelModeSlotLeaseSnapshot,
        ParallelModeSlotLeaseState,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempGitRepo {
        root: PathBuf,
        repo_root: PathBuf,
    }

    impl TempGitRepo {
        fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be monotonic")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("parallel-mode-{prefix}-{unique}"));
            let repo_root = root.join("repo");
            fs::create_dir_all(&repo_root).expect("temp repo root should be created");

            run_git(&repo_root, &["init", "-q"]);
            run_git(&repo_root, &["config", "user.name", "RefinedStone"]);
            run_git(
                &repo_root,
                &["config", "user.email", "chem.en.9273@gmail.com"],
            );
            fs::write(repo_root.join("README.md"), "seed\n").expect("seed file should write");
            fs::write(repo_root.join(".gitignore"), "*.tmp\n").expect("gitignore should write");
            run_git(&repo_root, &["add", "README.md"]);
            run_git(&repo_root, &["add", ".gitignore"]);
            run_git(&repo_root, &["commit", "-qm", "init"]);
            run_git(&repo_root, &["branch", "akra"]);

            Self { root, repo_root }
        }

        fn workspace_dir(&self) -> String {
            self.repo_root.display().to_string()
        }

        fn pool_root(&self) -> PathBuf {
            derive_default_pool_root(&self.repo_root)
        }

        fn slot_lease_path(&self, slot_number: usize) -> PathBuf {
            slot_lease_file_path(&self.pool_root(), &slot_id(slot_number))
        }

        fn read_slot_lease(&self, slot_number: usize) -> ParallelModeSlotLeaseSnapshot {
            let lease_body = fs::read_to_string(self.slot_lease_path(slot_number))
                .expect("slot lease should be readable");
            serde_json::from_str(&lease_body).expect("slot lease should deserialize")
        }

        fn create_detached_slot(&self, slot_number: usize) -> PathBuf {
            let slot_path = self.pool_root().join(slot_id(slot_number));
            fs::create_dir_all(
                slot_path
                    .parent()
                    .expect("slot path should have a parent directory"),
            )
            .expect("pool root should be created");
            run_git(
                &self.repo_root,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    slot_path.to_str().expect("slot path should be valid utf-8"),
                    "akra",
                ],
            );
            slot_path
        }

        fn create_agent_slot(&self, slot_number: usize, task_slug: &str) -> PathBuf {
            let slot_path = self.pool_root().join(slot_id(slot_number));
            fs::create_dir_all(
                slot_path
                    .parent()
                    .expect("slot path should have a parent directory"),
            )
            .expect("pool root should be created");
            let branch_name = format!("akra-agent/{}/{}", slot_id(slot_number), task_slug);
            run_git(
                &self.repo_root,
                &[
                    "worktree",
                    "add",
                    "-b",
                    branch_name.as_str(),
                    slot_path.to_str().expect("slot path should be valid utf-8"),
                    "akra",
                ],
            );
            slot_path
        }

        fn delete_local_akra_branch(&self) {
            run_git(&self.repo_root, &["branch", "-D", "akra"]);
        }

        fn commit_file_in_slot(
            &self,
            slot_path: &Path,
            file_name: &str,
            contents: &str,
            message: &str,
        ) {
            fs::write(slot_path.join(file_name), contents).expect("slot file should be written");
            run_git(slot_path, &["add", file_name]);
            run_git(slot_path, &["commit", "-qm", message]);
        }

        fn merge_agent_slot_into_akra(&self, slot_path: &Path) {
            let branch_name = current_branch(slot_path);
            let original_branch = current_branch(&self.repo_root);
            run_git(&self.repo_root, &["checkout", "akra"]);
            run_git(
                &self.repo_root,
                &["merge", "--ff-only", branch_name.as_str()],
            );
            run_git(&self.repo_root, &["checkout", original_branch.as_str()]);
        }

        fn branch_exists(&self, branch_name: &str) -> bool {
            let output = Command::new("git")
                .current_dir(&self.repo_root)
                .args([
                    "show-ref",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{branch_name}"),
                ])
                .env("GIT_TERMINAL_PROMPT", "0")
                .status()
                .expect("git show-ref should spawn");
            output.success()
        }
    }

    impl Drop for TempGitRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn run_git(repo_root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repo_root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("git command should spawn");
        assert!(
            output.status.success(),
            "git command should succeed: git {:?}\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    fn current_branch(repo_root: &Path) -> String {
        let output = Command::new("git")
            .current_dir(repo_root)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("git rev-parse should spawn");
        assert!(
            output.status.success(),
            "git rev-parse should succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("branch name should be utf-8")
            .trim()
            .to_string()
    }

    fn sample_lease_request(
        task_id: &str,
        task_title: &str,
        agent_id: &str,
        task_slug: &str,
    ) -> ParallelModeSlotLeaseRequest {
        ParallelModeSlotLeaseRequest::new(task_id, task_title, agent_id, task_slug)
    }

    #[test]
    fn derive_readiness_marks_blocked_when_any_blocker_exists() {
        let readiness = derive_readiness(&[
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GitRepository,
                ParallelModeCapabilityState::Ready,
                "ready",
                None,
            ),
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::Planning,
                ParallelModeCapabilityState::Blocked,
                "planning invalid",
                Some("repair planning".to_string()),
            ),
        ]);

        assert_eq!(readiness, ParallelModeReadinessState::Blocked);
    }

    #[test]
    fn derive_readiness_marks_degraded_when_only_optional_capabilities_fail() {
        let readiness = derive_readiness(&[
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GitRepository,
                ParallelModeCapabilityState::Ready,
                "ready",
                None,
            ),
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Degraded,
                "push unavailable",
                Some("restore auth".to_string()),
            ),
        ]);

        assert_eq!(readiness, ParallelModeReadinessState::Degraded);
    }

    #[test]
    fn inspect_planning_blocks_when_plan_is_off() {
        let capability = inspect_planning(
            &PlanningRuntimeSnapshot::ready("prompt".into(), "queue".into(), None)
                .with_plan_enabled(false)
                .with_workspace_present(true),
        );

        assert_eq!(capability.key, ParallelModeCapabilityKey::Planning);
        assert_eq!(capability.state, ParallelModeCapabilityState::Blocked);
        assert!(capability.detail.contains("planning mode is off"));
    }

    #[test]
    fn parse_https_remote_extracts_host_and_path() {
        assert_eq!(
            parse_https_remote("https://github.com/RefinedStone/codex-exec-loop.git"),
            Some((
                "github.com".to_string(),
                "RefinedStone/codex-exec-loop.git".to_string()
            ))
        );
        assert_eq!(
            parse_https_remote("git@github.com:RefinedStone/codex-exec-loop.git"),
            None
        );
    }

    #[test]
    fn build_supervisor_snapshot_marks_prepare_when_mode_is_off() {
        let service = ParallelModeService::new();
        let snapshot = service.build_supervisor_snapshot("/tmp/root", false, None);

        assert_eq!(snapshot.state, ParallelModeSupervisorState::Prepare);
        assert_eq!(snapshot.pool.configured_size, DEFAULT_POOL_SIZE);
        assert_eq!(snapshot.roster.active_count(), 0);
        assert_eq!(snapshot.distributor.head_summary, "inactive");
    }

    #[test]
    fn build_supervisor_snapshot_uses_recover_when_mode_enabled_but_blocked() {
        let service = ParallelModeService::new();
        let readiness = ParallelModeReadinessSnapshot::new(
            "/tmp/root",
            ParallelModeReadinessState::Blocked,
            vec![],
            Some("planning: blocked".to_string()),
        );

        let snapshot = service.build_supervisor_snapshot("/tmp/root", true, Some(&readiness));

        assert_eq!(snapshot.state, ParallelModeSupervisorState::Recover);
        assert_eq!(snapshot.pool.unavailable_slots, DEFAULT_POOL_SIZE);
        assert_eq!(snapshot.distributor.head_summary, "paused");
    }

    #[test]
    fn unavailable_pool_board_does_not_report_exhausted() {
        let pool = build_pool_board("/tmp/root", None);

        assert_eq!(pool.unavailable_slots, DEFAULT_POOL_SIZE);
        assert!(!pool.exhausted);
    }

    #[test]
    fn reconcile_marks_missing_slots_when_pool_root_has_not_been_created() {
        let repo = TempGitRepo::new("missing-slots");
        let readiness = ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            vec![],
            None,
        );
        let pool = build_pool_board(&repo.workspace_dir(), Some(&readiness));

        assert_eq!(pool.missing_slots, DEFAULT_POOL_SIZE);
        assert_eq!(pool.idle_slots, 0);
        assert!(!pool.exhausted);
        assert!(pool.reconcile_status.contains("missing slot"));
    }

    #[test]
    fn detached_akra_slot_counts_as_idle_baseline() {
        let repo = TempGitRepo::new("idle-slot");
        repo.create_detached_slot(1);
        let readiness = ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            vec![],
            None,
        );

        let pool = build_pool_board(&repo.workspace_dir(), Some(&readiness));
        let slot = &pool.slots[0];

        assert_eq!(slot.state, ParallelModePoolSlotState::Idle);
        assert_eq!(slot.branch_name, "akra (detached)");
        assert_eq!(pool.idle_slots, 1);
        assert_eq!(pool.missing_slots, DEFAULT_POOL_SIZE - 1);
    }

    #[test]
    fn agent_branch_slot_is_marked_awaiting_cleanup() {
        let repo = TempGitRepo::new("cleanup-slot");
        repo.create_agent_slot(1, "task-one");
        let slot_path = repo.pool_root().join(slot_id(1));
        repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
        repo.merge_agent_slot_into_akra(&slot_path);
        let readiness = ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            vec![],
            None,
        );

        let pool = build_pool_board(&repo.workspace_dir(), Some(&readiness));
        let slot = &pool.slots[0];

        assert_eq!(slot.state, ParallelModePoolSlotState::AwaitingCleanup);
        assert!(slot.branch_name.starts_with("akra-agent/slot-1/"));
        assert_eq!(slot.owner_label, "cleanup pending");
        assert_eq!(pool.awaiting_cleanup_slots, 1);
    }

    #[test]
    fn dirty_akra_baseline_slot_is_blocked_for_operator_recovery() {
        let repo = TempGitRepo::new("dirty-slot");
        let slot_path = repo.create_detached_slot(1);
        fs::write(slot_path.join("README.md"), "dirty\n").expect("slot file should be updated");
        let readiness = ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            vec![],
            None,
        );

        let pool = build_pool_board(&repo.workspace_dir(), Some(&readiness));
        let slot = &pool.slots[0];

        assert_eq!(slot.state, ParallelModePoolSlotState::Blocked);
        assert_eq!(slot.owner_label, "operator recovery");
        assert!(slot.worktree_label.contains("unstaged changes"));
    }

    #[test]
    fn reconcile_provisions_missing_slots_into_idle_baselines() {
        let repo = TempGitRepo::new("provision-slots");

        let pool = reconcile_pool_board(&repo.workspace_dir());

        assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
        assert_eq!(pool.missing_slots, 0);
        assert!(pool.reconcile_status.contains("provisioned 3"));
        for slot_number in 1..=DEFAULT_POOL_SIZE {
            assert!(repo.pool_root().join(slot_id(slot_number)).exists());
        }
    }

    #[test]
    fn reconcile_creates_local_akra_branch_before_provisioning_slots() {
        let repo = TempGitRepo::new("create-akra");
        repo.delete_local_akra_branch();
        assert!(!repo.branch_exists("akra"));

        let pool = reconcile_pool_board(&repo.workspace_dir());

        assert!(repo.branch_exists("akra"));
        assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
        assert!(pool.reconcile_status.contains("created `akra`"));
    }

    #[test]
    fn reconcile_cleans_merged_agent_slot_back_to_idle() {
        let repo = TempGitRepo::new("cleanup-execution");
        let service = ParallelModeService::new();
        let lease = service
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
            )
            .expect("slot lease should be acquired");
        let slot_path = PathBuf::from(lease.worktree_path.clone());
        repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
        let branch_name = lease.branch_name.clone();
        service
            .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
            .expect("slot lease should transition to running");
        repo.merge_agent_slot_into_akra(&slot_path);
        service
            .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
            .expect("slot lease should transition to cleanup pending");
        fs::write(slot_path.join("scratch.tmp"), "transient\n")
            .expect("untracked file should be written");

        let pool = reconcile_pool_board(&repo.workspace_dir());
        let slot = &pool.slots[0];

        assert_eq!(slot.state, ParallelModePoolSlotState::Idle);
        assert!(slot.branch_name.starts_with("akra"));
        assert!(!slot_path.join("scratch.tmp").exists());
        assert!(!repo.branch_exists(&branch_name));
        assert!(!repo.slot_lease_path(1).exists());
        assert!(pool.reconcile_status.contains("cleaned 1"));
    }

    #[test]
    fn acquire_slot_lease_persists_metadata_and_marks_slot_leased() {
        let repo = TempGitRepo::new("lease-slot");
        let service = ParallelModeService::new();

        let lease = service
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request("task-1", "Task One", "agent-1", "task one"),
            )
            .expect("slot lease should be acquired");
        let readiness = ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            vec![],
            None,
        );
        let pool = build_pool_board(&repo.workspace_dir(), Some(&readiness));
        let persisted = repo.read_slot_lease(1);

        assert_eq!(lease.slot_id, "slot-1");
        assert_eq!(lease.state, ParallelModeSlotLeaseState::Leased);
        assert_eq!(persisted.state, ParallelModeSlotLeaseState::Leased);
        assert_eq!(persisted.agent_id, "agent-1");
        assert_eq!(persisted.task_id, "task-1");
        assert!(
            persisted
                .branch_name
                .starts_with("akra-agent/slot-1/task-one")
        );
        assert_eq!(pool.leased_slots, 1);
        assert_eq!(pool.running_slots, 0);
        assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Leased);
        assert_eq!(pool.slots[0].owner_label, "agent-1 / task-1");
    }

    #[test]
    fn mark_slot_running_updates_persisted_lease_and_pool_state() {
        let repo = TempGitRepo::new("running-slot");
        let service = ParallelModeService::new();

        let lease = service
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
            )
            .expect("slot lease should be acquired");
        let running_lease = service
            .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
            .expect("slot lease should transition to running");
        let readiness = ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            vec![],
            None,
        );
        let pool = build_pool_board(&repo.workspace_dir(), Some(&readiness));
        let persisted = repo.read_slot_lease(1);

        assert_eq!(running_lease.state, ParallelModeSlotLeaseState::Running);
        assert!(running_lease.running_started_at.is_some());
        assert_eq!(persisted.state, ParallelModeSlotLeaseState::Running);
        assert!(persisted.running_started_at.is_some());
        assert_eq!(pool.leased_slots, 0);
        assert_eq!(pool.running_slots, 1);
        assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Running);
    }

    #[test]
    fn mark_slot_cleanup_pending_requires_running_state_and_merged_branch() {
        let repo = TempGitRepo::new("cleanup-pending-guards");
        let service = ParallelModeService::new();

        let lease = service
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
            )
            .expect("slot lease should be acquired");
        let slot_path = PathBuf::from(lease.worktree_path.clone());
        repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");

        let not_running_error = service
            .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
            .expect_err("cleanup pending should require the running state");
        assert!(not_running_error.contains("has not entered running state"));

        service
            .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
            .expect("slot lease should transition to running");
        let not_merged_error = service
            .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
            .expect_err("cleanup pending should require an integrated branch");
        assert!(not_merged_error.contains("is not integrated into `akra` yet"));
    }

    #[test]
    fn mark_slot_cleanup_pending_updates_persisted_lease_and_pool_state() {
        let repo = TempGitRepo::new("cleanup-pending-slot");
        let service = ParallelModeService::new();

        let lease = service
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
            )
            .expect("slot lease should be acquired");
        let slot_path = PathBuf::from(lease.worktree_path.clone());
        repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
        service
            .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
            .expect("slot lease should transition to running");
        repo.merge_agent_slot_into_akra(&slot_path);

        let cleanup_pending_lease = service
            .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
            .expect("slot lease should transition to cleanup pending");
        let readiness = ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            vec![],
            None,
        );
        let pool = build_pool_board(&repo.workspace_dir(), Some(&readiness));
        let persisted = repo.read_slot_lease(1);

        assert_eq!(
            cleanup_pending_lease.state,
            ParallelModeSlotLeaseState::CleanupPending
        );
        assert_eq!(persisted.state, ParallelModeSlotLeaseState::CleanupPending);
        assert_eq!(pool.awaiting_cleanup_slots, 1);
        assert_eq!(pool.running_slots, 0);
        assert_eq!(
            pool.slots[0].state,
            ParallelModePoolSlotState::AwaitingCleanup
        );
        assert_eq!(pool.slots[0].owner_label, "agent-1 / task-1");
    }
}

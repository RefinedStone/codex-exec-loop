use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot,
    ParallelModeCapabilityState, ParallelModeCompletionFeedEntry, ParallelModeDistributorSnapshot,
    ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
    ParallelModeQueueItemState, ParallelModeReadinessSnapshot, ParallelModeReadinessState,
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

fn inspect_pool_board(workspace_dir: &str) -> ParallelModePoolBoardSnapshot {
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
    let Some(akra_head) = resolve_branch_head(&repo_root, AKRA_BRANCH) else {
        return build_blocked_pool_board(
            workspace_dir,
            "reconcile blocked / `akra` baseline could not be resolved",
            "`akra` is unavailable during reconcile",
        );
    };
    let Some(worktree_output) = run_command(
        "git",
        ["-C", repo_root.as_str(), "worktree", "list", "--porcelain"],
        None,
    ) else {
        return build_blocked_pool_board(
            workspace_dir,
            "reconcile failed / git worktree inventory could not be loaded",
            "worktree list inspection failed",
        );
    };

    let pool_root = derive_default_pool_root(&canonical_repo_root);
    let pool_root_label = display_pool_path(&canonical_repo_root, &pool_root);
    let worktree_records = parse_worktree_records(&worktree_output);
    let slots = (1..=DEFAULT_POOL_SIZE)
        .map(|slot_number| {
            inspect_pool_slot(
                &canonical_repo_root,
                &pool_root,
                &slot_id(slot_number),
                &akra_head,
                &worktree_records,
            )
        })
        .collect::<Vec<_>>();
    let reconcile_status = summarize_pool_reconcile_status(&slots, &pool_root);

    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
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

fn inspect_pool_slot(
    canonical_repo_root: &Path,
    pool_root: &Path,
    slot_id: &str,
    akra_head: &str,
    worktree_records: &[GitWorktreeRecord],
) -> ParallelModePoolSlotSnapshot {
    let slot_path = pool_root.join(slot_id);
    let base_worktree_label = display_pool_path(canonical_repo_root, &slot_path);
    let Some(worktree_record) = worktree_records
        .iter()
        .find(|record| record.path == slot_path)
    else {
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
            "unknown",
            annotate_worktree_label(base_worktree_label, "git status inspection failed"),
            "operator recovery",
        );
    };

    if worktree_record.branch_name.as_deref() == Some(AKRA_BRANCH)
        || (worktree_record.detached && worktree_record.head_sha == akra_head)
    {
        let branch_label = if worktree_record.detached {
            format!("{AKRA_BRANCH} (detached)")
        } else {
            AKRA_BRANCH.to_string()
        };

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
                    "operator recovery",
                );
            }

            return ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::AwaitingCleanup,
                branch_name,
                annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                "cleanup pending",
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
            "operator recovery",
        );
    }

    let detached_label = format!("detached@{}", short_sha(&worktree_record.head_sha));
    ParallelModePoolSlotSnapshot::new(
        slot_id,
        ParallelModePoolSlotState::Blocked,
        detached_label,
        annotate_worktree_label(base_worktree_label, "detached away from `akra` baseline"),
        "operator recovery",
    )
}

fn summarize_pool_reconcile_status(
    slots: &[ParallelModePoolSlotSnapshot],
    pool_root: &Path,
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

    if blocked_slots > 0 {
        return format!(
            "reconcile blocked / blocked: {blocked_slots} / missing: {missing_slots} / cleanup: {awaiting_cleanup_slots} / root {}",
            pool_root.display()
        );
    }

    if missing_slots > 0 && awaiting_cleanup_slots > 0 {
        return format!(
            "reconcile pending / missing: {missing_slots} / cleanup pending: {awaiting_cleanup_slots} / root {}",
            pool_root.display()
        );
    }

    if missing_slots > 0 {
        return format!(
            "reconcile pending / create {missing_slots} missing slot(s) under {}",
            pool_root.display()
        );
    }

    if awaiting_cleanup_slots > 0 {
        return format!(
            "cleanup pending / {awaiting_cleanup_slots} slot(s) still need reset to `{AKRA_BRANCH}`"
        );
    }

    if idle_slots == slots.len() && !slots.is_empty() {
        return format!("reconcile complete / all slots are clean on `{AKRA_BRANCH}` baseline");
    }

    format!("reconcile complete / pool root {}", pool_root.display())
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
        derive_default_pool_root, derive_readiness, inspect_planning, parse_https_remote, slot_id,
    };
    use crate::application::service::planning::PlanningRuntimeSnapshot;
    use crate::domain::parallel_mode::ParallelModePoolSlotState;
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
            run_git(&repo_root, &["add", "README.md"]);
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
}

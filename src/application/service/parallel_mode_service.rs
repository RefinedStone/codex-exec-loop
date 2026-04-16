use std::io::Write;
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
            build_placeholder_pool_board(readiness_snapshot),
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
            Some("control tower is live in read-only placeholder mode".to_string())
        }
        (true, Some(_)) => Some("repair readiness blockers before assigning agents".to_string()),
        (false, Some(_)) => Some("run `:parallel on` after reviewing the board".to_string()),
        (true, None) => Some("rerun readiness to hydrate the supervisor board".to_string()),
        (false, None) => None,
    }
}

fn build_placeholder_pool_board(
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModePoolBoardSnapshot {
    match readiness_snapshot {
        Some(snapshot) if snapshot.allows_parallel_mode() => {
            let slots = (1..=DEFAULT_POOL_SIZE)
                .map(|slot_number| {
                    let slot_id = format!("slot-{slot_number:02}");
                    ParallelModePoolSlotSnapshot::new(
                        slot_id.clone(),
                        ParallelModePoolSlotState::Idle,
                        format!("akra/{slot_id}"),
                        format!("pool/{slot_id} placeholder"),
                        "unassigned",
                    )
                })
                .collect::<Vec<_>>();
            let reconcile_status = match snapshot.readiness {
                ParallelModeReadinessState::Ready => {
                    "placeholder reconcile complete / all slots are synthetic".to_string()
                }
                ParallelModeReadinessState::Degraded => {
                    "placeholder reconcile complete / degraded capabilities may still pause later slices"
                        .to_string()
                }
                ParallelModeReadinessState::Blocked | ParallelModeReadinessState::Repairing => {
                    "placeholder reconcile pending".to_string()
                }
            };

            ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, reconcile_status, slots)
        }
        Some(snapshot) => {
            let slots = (1..=DEFAULT_POOL_SIZE)
                .map(|slot_number| {
                    let slot_id = format!("slot-{slot_number:02}");
                    ParallelModePoolSlotSnapshot::new(
                        slot_id,
                        ParallelModePoolSlotState::Unavailable,
                        "not leased",
                        "reconcile blocked by readiness gate",
                        "supervisor gate",
                    )
                })
                .collect::<Vec<_>>();

            ParallelModePoolBoardSnapshot::new(
                DEFAULT_POOL_SIZE,
                format!(
                    "reconcile blocked / readiness: {}",
                    snapshot.readiness_label()
                ),
                slots,
            )
        }
        None => {
            let slots = (1..=DEFAULT_POOL_SIZE)
                .map(|slot_number| {
                    let slot_id = format!("slot-{slot_number:02}");
                    ParallelModePoolSlotSnapshot::new(
                        slot_id,
                        ParallelModePoolSlotState::Unavailable,
                        "not inspected",
                        "readiness has not been checked",
                        "n/a",
                    )
                })
                .collect::<Vec<_>>();

            ParallelModePoolBoardSnapshot::new(
                DEFAULT_POOL_SIZE,
                "reconcile pending / run readiness first",
                slots,
            )
        }
    }
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
        ParallelModeService, ParallelModeSupervisorState, build_placeholder_pool_board,
        derive_readiness, inspect_planning, parse_https_remote,
    };
    use crate::application::service::planning::PlanningRuntimeSnapshot;

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
    fn placeholder_pool_board_is_idle_when_readiness_allows_parallel_mode() {
        let readiness = ParallelModeReadinessSnapshot::new(
            "/tmp/root",
            ParallelModeReadinessState::Ready,
            vec![],
            None,
        );

        let pool = build_placeholder_pool_board(Some(&readiness));

        assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
        assert_eq!(pool.blocked_slots, 0);
        assert!(
            pool.reconcile_status
                .contains("placeholder reconcile complete")
        );
    }
}

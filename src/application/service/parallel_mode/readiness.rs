use std::path::Path;
use std::process::{Command, Stdio};

use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
};

use super::{DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH};

const GITHUB_SCRIPT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/gh-refinedstone.sh");

pub(super) fn detect_git_repo_root(workspace_dir: &str) -> Option<String> {
    run_command(
        "git",
        ["-C", workspace_dir, "rev-parse", "--show-toplevel"],
        None,
    )
    .filter(|value| !value.is_empty())
}

pub(super) fn inspect_git_worktree(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
) -> ParallelModeCapabilitySnapshot {
    match runtime.run_command(
        "git",
        &["-C", repo_root, "worktree", "list", "--porcelain"],
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

pub(super) fn inspect_akra_branch(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
) -> ParallelModeCapabilitySnapshot {
    if runtime.command_succeeds(
        "git",
        &[
            "-C",
            repo_root,
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{POOL_BASELINE_BRANCH}"),
        ],
    ) || runtime.command_succeeds(
        "git",
        &[
            "-C",
            repo_root,
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{POOL_BASELINE_BRANCH}"),
        ],
    ) {
        return ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::AkraBranch,
            ParallelModeCapabilityState::Ready,
            format!("{POOL_BASELINE_BRANCH} is available"),
            None,
        );
    }

    if runtime.command_succeeds("git", &["-C", repo_root, "rev-parse", "--verify", "HEAD"]) {
        return ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::AkraBranch,
            ParallelModeCapabilityState::Ready,
            format!("{POOL_BASELINE_BRANCH} is missing locally but can be created from HEAD"),
            None,
        );
    }

    ParallelModeCapabilitySnapshot::new(
        ParallelModeCapabilityKey::AkraBranch,
        ParallelModeCapabilityState::Blocked,
        format!("{POOL_BASELINE_BRANCH} is missing and this repository has no usable HEAD yet"),
        Some("create an initial commit or restore the integration branch before enabling parallel mode".to_string()),
    )
}

pub(super) fn inspect_push_remote(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
) -> ParallelModeCapabilitySnapshot {
    let Some(push_url) = runtime.run_command(
        "git",
        &[
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

    let Some(credentials) = runtime.run_command_with_stdin(
        "git",
        &["credential", "fill"],
        &format!("protocol=https\nhost={host}\npath={path}\n\n"),
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

pub(super) fn inspect_gh_binary(
    runtime: &dyn ParallelModeRuntimePort,
) -> ParallelModeCapabilitySnapshot {
    match runtime.find_executable("gh") {
        Some(path) => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhBinary,
            ParallelModeCapabilityState::Ready,
            format!("gh found at {}", path.display()),
            None,
        ),
        None if github_fallback_script_available() => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhBinary,
            ParallelModeCapabilityState::Ready,
            format!(
                "gh is not installed; RefinedStone API fallback is available at {GITHUB_SCRIPT_PATH}"
            ),
            None,
        ),
        None => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhBinary,
            ParallelModeCapabilityState::Degraded,
            "gh is not installed on PATH and the RefinedStone fallback script is missing",
            Some("install GitHub CLI or restore scripts/gh-refinedstone.sh".to_string()),
        ),
    }
}

pub(super) fn inspect_gh_auth(
    runtime: &dyn ParallelModeRuntimePort,
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

    let auth_succeeded = if runtime.find_executable("gh").is_some() {
        runtime.gh_auth_status(repo_root)
    } else if github_fallback_script_available() {
        runtime
            .run_command("bash", &[GITHUB_SCRIPT_PATH, "auth", "status"], repo_root)
            .is_some()
    } else {
        false
    };

    match auth_succeeded {
        true => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhAuth,
            ParallelModeCapabilityState::Ready,
            "GitHub automation authentication succeeded",
            None,
        ),
        false => ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhAuth,
            ParallelModeCapabilityState::Degraded,
            "GitHub automation is not authenticated for this workspace",
            Some("verify gh auth or the repo-local RefinedStone credential".to_string()),
        ),
    }
}

fn github_fallback_script_available() -> bool {
    Path::new(GITHUB_SCRIPT_PATH).exists()
}

pub(super) fn inspect_planning(
    snapshot: &PlanningRuntimeSnapshot,
) -> ParallelModeCapabilitySnapshot {
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

pub(super) fn inspect_authority_store(
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

pub(super) fn blocked_prerequisite_capability(
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

pub(super) fn parse_https_remote(push_url: &str) -> Option<(String, String)> {
    let stripped = push_url.trim().strip_prefix("https://")?;
    let mut parts = stripped.splitn(2, '/');
    let host = parts.next()?.trim();
    let path = parts.next()?.trim();
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some((host.to_string(), path.to_string()))
}

pub(super) fn command_succeeds<const N: usize>(program: &str, args: [&str; N]) -> bool {
    let mut command = Command::new(program);
    command.args(args);
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.status().is_ok_and(|status| status.success())
}

pub(super) fn run_command<const N: usize>(
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

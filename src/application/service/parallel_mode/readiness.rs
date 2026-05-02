use super::{DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH};
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
};
use std::path::Path;
use std::process::{Command, Stdio};
const GITHUB_SCRIPT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/gh-refinedstone.sh");

/*
readiness의 첫 단계는 현재 workspace가 git repository 안에 있는지 찾는 것이다. 병렬 모드는 git
worktree와 branch를 강하게 전제하므로, repo root가 없으면 나머지 capability는 모두 prerequisite
blocked 상태가 된다.
*/
pub(super) fn detect_git_repo_root(workspace_dir: &str) -> Option<String> {
    run_command(
        "git",
        ["-C", workspace_dir, "rev-parse", "--show-toplevel"],
        None,
    )
    .filter(|value| !value.is_empty())
}

/*
git worktree capability는 이 repository가 `git worktree list --porcelain`을 실행할 수 있는지
확인한다. pool slot은 모두 git worktree로 만들어지므로, 이 명령이 실패하면
slot provision/reconcile/inspection 전체가 신뢰할 수 없다.
*/
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

/*
akra branch capability는 pool baseline이 될 integration branch를 찾는다. local `prerelease`가
있거나 origin의 remote tracking branch가 있으면 ready이고, 둘 다 없어도 HEAD가 있으면 최초
reconcile에서 baseline을 만들 수 있으므로 ready로 둔다. 완전히 HEAD가 없는 repo만 blocked다.
*/
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

/*
push remote capability는 distributor가 source branch와 integration branch를 원격에 push할 수
있는지를 미리 진단한다. remote URL이 없거나 HTTPS GitHub 형식이 아니거나 credential fill이
실패하면 degraded로 둔다. degraded는 병렬 모드 자체를 완전히 막지는 않지만, GitHub delivery
자동화가 나중에 blocked될 수 있음을 supervisor에 보여 준다.
*/
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

/*
GitHub automation은 `gh` CLI가 있으면 그것을 쓰고, 없으면 repo-local
`scripts/gh-refinedstone.sh` fallback을 허용한다. 이 capability는 실제 auth 여부가 아니라
"GitHub 조작을 시도할 실행 경로가 있는가"를 확인한다. auth 여부는 다음 `inspect_gh_auth`가
별도로 판단한다.
*/
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
        None if github_fallback_script_available(runtime) => ParallelModeCapabilitySnapshot::new(
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

/*
GitHub auth capability는 실제로 PR 생성/조회/close 같은 GitHub API 작업을 할 수 있는지 확인한다.
`gh`가 있으면 `gh auth status` 계열을, fallback script만 있으면 script의 auth status를 사용한다.
binary capability가 ready가 아니면 auth도 degraded로 두어 원인 체인이 화면에 드러나게 한다.
*/
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
    } else if github_fallback_script_available(runtime) {
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
fn github_fallback_script_available(runtime: &dyn ParallelModeRuntimePort) -> bool {
    runtime.path_exists(Path::new(GITHUB_SCRIPT_PATH))
}

/*
planning capability는 병렬 mode가 배정할 queue와 official completion ledger를 신뢰할 수 있는지
확인한다. workspace가 없거나 invalid면 blocked이고, ready 상태에서는 task가 있든 없든 병렬
모드의 나머지 capability 판단을 계속할 수 있다.
*/
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

/*
authority store capability는 planning 문서 mirror와 repo-scoped shadow store가 동기화되어 있는지
확인한다. 병렬 agent는 slot worktree에서 작업하지만 planning authority는 canonical repo root
기준으로 session/queue/lease projection을 읽으므로, shadow store parity가 깨지면 supervisor와
distributor가 다른 현실을 볼 수 있다.

git repository와 planning capability가 ready가 아니면 이 검사를 미루고 prerequisite blocked를
반환한다. 선행 조건이 없을 때 store parity 실패처럼 보이는 가짜 오류를 줄이기 위해서다.
*/
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

/*
prerequisite blocked capability는 어떤 capability가 자기 검사를 할 수 없을 때 쓰는 공통 helper다.
예를 들어 git repo root가 없으면 worktree, push remote, authority store는 각자 실패를 추측하지
않고 "git repository detection을 기다림"이라고 표시한다.
*/
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

/*
credential fill에는 protocol, host, path를 분리해 넘겨야 하므로 HTTPS remote URL을 간단히
파싱한다. SSH remote나 비 GitHub 형식은 여기서 None이 되어 push capability가 degraded로
표시된다. distributor 자동화는 현재 HTTPS credential flow를 기준으로 설계되어 있기 때문이다.
*/
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

/*
readiness와 pool helper의 low-level command 실행은 interactive prompt를 꺼야 한다.
`GIT_TERMINAL_PROMPT=0`, null stderr/stdout 정책을 통해 capability check가 사용자 입력을 기다리며
TUI를 멈추지 않게 한다. 값이 필요한 경우에는 `run_command`, 성공 여부만 필요한 경우에는 이
함수를 사용한다.
*/
pub(super) fn command_succeeds<const N: usize>(program: &str, args: [&str; N]) -> bool {
    let mut command = Command::new(program);
    command.args(args);
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.status().is_ok_and(|status| status.success())
}

/*
run_command는 readiness와 git helper가 짧은 stdout 값을 얻을 때 쓰는 공통 wrapper다. 명령이
실패하거나 stdout이 비어 있으면 None을 반환해 호출자가 capability degraded/blocked를 명시적으로
선택하게 한다. stderr는 숨겨 capability 화면에 raw command noise가 섞이지 않게 한다.
*/
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

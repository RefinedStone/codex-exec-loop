/*
GitHub automation outbound adapter다.

parallel-mode orchestration은 branch push, PR 생성/조회, capability inspection을 application port로만
바라본다. 이 파일은 그 port 호출을 repo-local git 명령과 `scripts/gh-refinedstone.sh` 실행으로 변환한다.
GitHub CLI가 있으면 인증 상태 확인에 활용하고, 실제 PR 조작은 RefinedStone wrapper script를 우선해
repo 규칙의 identity와 credential 경계를 한곳에 모은다.
*/
use std::path::Path;
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
};
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
};

const DEFAULT_PUSH_REMOTE_NAME: &str = "origin";
const GITHUB_SCRIPT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/gh-refinedstone.sh");

pub struct GithubAutomationAdapter;

impl Default for GithubAutomationAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl GithubAutomationAdapter {
    pub fn new() -> Self {
        Self
    }

    /*
    push remote capability는 supersession/parallel lane이 remote branch를 publish할 수 있는지 알려준다.

    GitHub HTTPS remote와 local/file remote를 모두 ready로 보는 이유는 local-only integration 테스트와
    실제 RefinedStone GitHub push 흐름을 같은 port로 다루기 위해서다. remote가 아예 없을 때만 degraded로
    내려, 상위 runtime이 PR 생성 대신 local inspection mode를 선택할 수 있게 한다.
    */
    fn inspect_push_remote(repo_root: &str) -> ParallelModeCapabilitySnapshot {
        let Some(push_url) = run_git_stdout(
            repo_root,
            &["remote", "get-url", "--push", DEFAULT_PUSH_REMOTE_NAME],
        )
        .ok() else {
            return ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Degraded,
                format!("push remote `{DEFAULT_PUSH_REMOTE_NAME}` is not configured"),
                Some(
                    "add a push remote or keep supersession in local-only inspection mode"
                        .to_string(),
                ),
            );
        };

        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::PushRemote,
            ParallelModeCapabilityState::Ready,
            format!("push remote is configured at {push_url}"),
            None,
        )
    }

    /*
    GitHub command capability는 두 실행 경로를 함께 본다.

    `gh`가 있으면 사람이 익숙한 GitHub CLI 상태를 보고하고, 없더라도 repo의 RefinedStone wrapper script가
    있으면 automation은 계속 가능하다. 둘 다 없을 때만 PR automation을 degraded로 표시한다.
    */
    fn inspect_gh_binary() -> ParallelModeCapabilitySnapshot {
        match which::which("gh") {
            Ok(path) => ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GhBinary,
                ParallelModeCapabilityState::Ready,
                format!("gh found at {}", path.display()),
                None,
            ),
            Err(_) if Path::new(GITHUB_SCRIPT_PATH).exists() => {
                ParallelModeCapabilitySnapshot::new(
                    ParallelModeCapabilityKey::GhBinary,
                    ParallelModeCapabilityState::Ready,
                    format!(
                        "gh is not installed; RefinedStone API fallback is available at {GITHUB_SCRIPT_PATH}"
                    ),
                    None,
                )
            }
            Err(_) => ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GhBinary,
                ParallelModeCapabilityState::Degraded,
                "gh is not installed on PATH and the RefinedStone fallback script is missing",
                Some("install GitHub CLI or restore scripts/gh-refinedstone.sh".to_string()),
            ),
        }
    }

    /*
    Authentication capability intentionally executes a no-output status command.

    The port only needs a stable ready/degraded signal, not the raw credential details. The adapter therefore suppresses
    stdout/stderr and maps either `gh auth status` or the RefinedStone script status check into a capability snapshot.
    */
    fn inspect_gh_auth(
        gh_binary: &ParallelModeCapabilitySnapshot,
        repo_root: &str,
    ) -> ParallelModeCapabilitySnapshot {
        if gh_binary.state != ParallelModeCapabilityState::Ready {
            return ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GhAuth,
                ParallelModeCapabilityState::Degraded,
                "gh auth is unavailable until the gh binary is installed",
                Some("install gh first, then run `gh auth login`".to_string()),
            );
        }

        let auth_status = if which::which("gh").is_ok() {
            Command::new("gh")
                .current_dir(repo_root)
                .args(["auth", "status"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .env("GIT_TERMINAL_PROMPT", "0")
                .status()
        } else {
            Command::new("bash")
                .current_dir(repo_root)
                .args([GITHUB_SCRIPT_PATH, "auth", "status"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .env("GIT_TERMINAL_PROMPT", "0")
                .status()
        };

        if auth_status.is_ok_and(|status| status.success()) {
            return ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GhAuth,
                ParallelModeCapabilityState::Ready,
                "GitHub automation authentication succeeded",
                None,
            );
        }

        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhAuth,
            ParallelModeCapabilityState::Degraded,
            "GitHub automation is not authenticated for this workspace",
            Some("verify gh auth or the repo-local RefinedStone credential".to_string()),
        )
    }

    /*
    PR lookup is the idempotency gate for `ensure_pull_request`.

    The wrapper script returns GitHub's PR JSON shape. This adapter immediately maps that external shape into the
    application port record so the rest of the system never depends on `baseRefName`/`headRefName` field spelling.
    */
    fn find_open_pull_request(
        &self,
        repo_root: &str,
        base_branch: &str,
        head_branch: &str,
    ) -> Result<Option<GithubAutomationPullRequest>> {
        let output = run_command(
            "bash",
            &[
                GITHUB_SCRIPT_PATH,
                "pr",
                "list",
                "--state",
                "open",
                "--base",
                base_branch,
                "--head",
                head_branch,
                "--json",
                "number,url,state,baseRefName,headRefName,isDraft",
            ],
            repo_root,
        )?;
        let pull_requests = serde_json::from_str::<Vec<GithubPullRequestJson>>(&output)
            .with_context(|| {
                format!("failed to parse `gh pr list` output while locating `{head_branch}`")
            })?;
        Ok(pull_requests.into_iter().next().map(Into::into))
    }
}

impl GithubAutomationPort for GithubAutomationAdapter {
    fn inspect_capabilities(&self, repo_root: &str) -> GithubAutomationCapabilities {
        let push_remote = Self::inspect_push_remote(repo_root);
        let gh_binary = Self::inspect_gh_binary();
        let gh_auth = Self::inspect_gh_auth(&gh_binary, repo_root);
        GithubAutomationCapabilities::new(push_remote, gh_binary, gh_auth)
    }

    fn push_branch(
        &self,
        repo_root: &str,
        branch_name: &str,
        force_with_lease: bool,
    ) -> Result<()> {
        if force_with_lease {
            run_git(
                repo_root,
                &[
                    "push",
                    "--force-with-lease",
                    DEFAULT_PUSH_REMOTE_NAME,
                    branch_name,
                ],
            )
        } else {
            run_git(
                repo_root,
                &["push", "-u", DEFAULT_PUSH_REMOTE_NAME, branch_name],
            )
        }
    }

    /*
    Ensure semantics keep PR creation retry-safe.

    The adapter checks for an existing open PR before creating one, then checks again after creation. The second lookup is
    deliberate: GitHub may return a URL, the wrapper may normalize output, or a concurrent actor may have created the PR
    between calls. Falling back to URL parsing is only the final recovery path.
    */
    fn ensure_pull_request(
        &self,
        repo_root: &str,
        base_branch: &str,
        head_branch: &str,
        title: &str,
        body: &str,
    ) -> Result<GithubAutomationPullRequest> {
        if let Some(existing) = self.find_open_pull_request(repo_root, base_branch, head_branch)? {
            return Ok(existing);
        }

        let create_output = run_command(
            "bash",
            &[
                GITHUB_SCRIPT_PATH,
                "pr",
                "create",
                "--base",
                base_branch,
                "--head",
                head_branch,
                "--title",
                title,
                "--body",
                body,
            ],
            repo_root,
        )?;

        if let Some(existing) = self.find_open_pull_request(repo_root, base_branch, head_branch)? {
            return Ok(existing);
        }
        if let Some(pr_number) = parse_pull_request_number_from_url(&create_output) {
            return self.inspect_pull_request(repo_root, pr_number);
        }

        Err(anyhow!(
            "pull request create succeeded but no open PR was found for `{head_branch}`"
        ))
    }

    fn inspect_pull_request(
        &self,
        repo_root: &str,
        pr_number: u64,
    ) -> Result<GithubAutomationPullRequest> {
        let output = run_command(
            "bash",
            &[
                GITHUB_SCRIPT_PATH,
                "pr",
                "view",
                &pr_number.to_string(),
                "--json",
                "number,url,state,baseRefName,headRefName,isDraft",
            ],
            repo_root,
        )?;
        let pull_request = serde_json::from_str::<GithubPullRequestJson>(&output)
            .with_context(|| format!("failed to parse `gh pr view` output for PR #{pr_number}"))?;
        Ok(pull_request.into())
    }

    fn push_integration_branch(&self, repo_root: &str, branch_name: &str) -> Result<()> {
        run_git(repo_root, &["push", DEFAULT_PUSH_REMOTE_NAME, branch_name])
    }

    fn close_pull_request(&self, repo_root: &str, pr_number: u64) -> Result<()> {
        run_command(
            "bash",
            &[GITHUB_SCRIPT_PATH, "pr", "close", &pr_number.to_string()],
            repo_root,
        )?;
        Ok(())
    }
}

/*
Subset of the GitHub PR JSON used by this adapter.

The external field names intentionally stay in this private DTO. The `From` implementation below is the only mapping
point into the application port type.
*/
#[derive(Debug, Deserialize)]
struct GithubPullRequestJson {
    number: u64,
    url: String,
    state: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
}

impl From<GithubPullRequestJson> for GithubAutomationPullRequest {
    fn from(value: GithubPullRequestJson) -> Self {
        GithubAutomationPullRequest::new(
            value.number,
            value.url,
            value.state,
            value.base_ref_name,
            value.head_ref_name,
            value.is_draft,
        )
    }
}

fn run_git(repo_root: &str, args: &[&str]) -> Result<()> {
    let output = run_process("git", args, repo_root)?;
    if output.status.success() {
        return Ok(());
    }

    bail!(
        "git {} failed in {}: {}",
        args.join(" "),
        repo_root,
        command_error_detail(&output)
    )
}

fn run_git_stdout(repo_root: &str, args: &[&str]) -> Result<String> {
    run_command("git", args, repo_root)
}

/*
Run a command and return trimmed stdout only on success.

All GitHub automation subprocesses pass through this helper so failures include the program, arguments, repo root, and the
best available command output. That context is more useful to the orchestration layer than a bare exit status.
*/
fn run_command(program: &str, args: &[&str], repo_root: &str) -> Result<String> {
    let output = run_process(program, args, repo_root)?;
    if !output.status.success() {
        bail!(
            "{} {} failed in {}: {}",
            program,
            args.join(" "),
            repo_root,
            command_error_detail(&output)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_process(program: &str, args: &[&str], repo_root: &str) -> Result<Output> {
    Command::new(program)
        .current_dir(repo_root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .with_context(|| {
            format!(
                "failed to spawn `{program} {}` in {repo_root}",
                args.join(" ")
            )
        })
}

fn command_error_detail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }

    "command exited without output".to_string()
}

fn parse_pull_request_number_from_url(output: &str) -> Option<u64> {
    output
        .trim()
        .rsplit('/')
        .next()
        .and_then(|value| value.parse::<u64>().ok())
}

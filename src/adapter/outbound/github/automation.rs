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

        if push_url.starts_with("https://github.com/") {
            return ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Ready,
                format!("push remote is configured at {push_url}"),
                None,
            );
        }

        if Path::new(push_url.as_str()).exists() || push_url.starts_with("file://") {
            return ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Ready,
                format!("push remote is configured at {push_url}"),
                None,
            );
        }

        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::PushRemote,
            ParallelModeCapabilityState::Ready,
            format!("push remote is configured at {push_url}"),
            None,
        )
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

        let output = Command::new("gh")
            .current_dir(repo_root)
            .args(["auth", "status"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0")
            .status();

        match output {
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

    fn find_open_pull_request(
        &self,
        repo_root: &str,
        base_branch: &str,
        head_branch: &str,
    ) -> Result<Option<GithubAutomationPullRequest>> {
        let output = run_command(
            "gh",
            &[
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

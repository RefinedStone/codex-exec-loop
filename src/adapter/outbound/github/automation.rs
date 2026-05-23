/*
GitHub automation outbound adapter다.

parallel-mode orchestration은 branch push, PR 생성/조회, capability inspection을 application port로만
바라본다. 이 파일은 그 port 호출을 repo-local git 명령과 `gh`/`scripts/gh-akra.sh` 실행으로 변환한다.
GitHub CLI가 있으면 로컬 인증을 그대로 활용하고, 없으면 wrapper script가 git credential 기반 REST
fallback을 제공한다.
*/
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use crate::application::port::outbound::github_automation_port::{
    DEFAULT_GITHUB_PUSH_REMOTE_NAME as DEFAULT_PUSH_REMOTE_NAME,
    GITHUB_AUTOMATION_SCRIPT_RELATIVE_PATH as GITHUB_SCRIPT_RELATIVE_PATH,
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
};
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
};
use crate::subprocess;

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

    remote 존재만으로는 사용자의 로컬 git 인증과 branch 권한을 예측하기 어렵다. 그래서 현재
    worktree branch를 대상으로 `git push --dry-run`을 실행해 실제 push와 같은 credential/helper
    경로를 미리 태운다. dry-run 대상 branch가 없을 때만 remote 존재를 ready로 남기고, 실제 push가
    최종 guard 역할을 한다.
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

        if let Ok(current_branch) = run_git_stdout(repo_root, &["branch", "--show-current"])
            && !current_branch.is_empty()
        {
            let refspec = format!("HEAD:refs/heads/{current_branch}");
            if run_git(
                repo_root,
                &[
                    "push",
                    "--dry-run",
                    DEFAULT_PUSH_REMOTE_NAME,
                    refspec.as_str(),
                ],
            )
            .is_ok()
            {
                return ParallelModeCapabilitySnapshot::new(
                    ParallelModeCapabilityKey::PushRemote,
                    ParallelModeCapabilityState::Ready,
                    format!("push dry-run succeeded for `{current_branch}`"),
                    None,
                );
            }
            return ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Degraded,
                format!("git push --dry-run failed for `{current_branch}` to `{push_url}`"),
                Some("repair git push credentials or remote branch permissions".to_string()),
            );
        }

        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::PushRemote,
            ParallelModeCapabilityState::Ready,
            format!("push remote is configured at {push_url}"),
            None,
        )
    }

    /*
    GitHub command capability는 두 실행 경로를 함께 본다.

    `gh`가 있으면 사람이 익숙한 GitHub CLI 상태를 보고하고, 없더라도 Akra GitHub wrapper script가
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
            Err(_) => {
                let script_path = github_script_path();
                if script_path.is_file() {
                    return ParallelModeCapabilitySnapshot::new(
                        ParallelModeCapabilityKey::GhBinary,
                        ParallelModeCapabilityState::Ready,
                        format!(
                            "gh is not installed; Akra GitHub API fallback is available at {}",
                            script_path.display()
                        ),
                        None,
                    );
                }
                ParallelModeCapabilitySnapshot::new(
                    ParallelModeCapabilityKey::GhBinary,
                    ParallelModeCapabilityState::Degraded,
                    "gh is not installed on PATH and the Akra GitHub fallback script is missing",
                    Some("install GitHub CLI or restore scripts/gh-akra.sh".to_string()),
                )
            }
        }
    }

    /*
    authentication capability는 의도적으로 output을 버리는 status command만 실행한다.

    application port가 필요한 것은 ready/degraded 신호와 operator-facing hint이지 raw credential detail이 아니다.
    그래서 adapter는 stdout/stderr를 숨기고, `gh auth status` 또는 Akra script의 auth check 결과를
    ParallelModeCapabilitySnapshot으로만 접는다. credential 위치와 token 문자열은 이 outbound boundary 밖으로 새지 않는다.
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
            /*
            `gh`가 있으면 표준 GitHub CLI 상태를 우선한다.
            operator가 `gh auth login` 같은 익숙한 도구로 직접 복구할 수 있기 때문이다.
            그래도 command output은 숨긴다. capability inspection은 interactive diagnostic log가 아니라 compact readiness board를
            채우는 입력이다.
            */
            let mut command = Command::new("gh");
            command
                .current_dir(repo_root)
                .args(["auth", "status"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .env("GIT_TERMINAL_PROMPT", "0");
            subprocess::command_output(&mut command, "gh auth status").map(|output| output.status)
        } else {
            /*
            Akra wrapper는 이 project의 supported fallback이다.
            CI나 `gh`가 없는 local machine도 아래 write operation과 같은 local git credential path를 사용하게 한다.
            capability check와 실제 PR write가 같은 wrapper contract를 공유해야 "ready" 판단과 실행 경로가 어긋나지 않는다.
            */
            let script_path = github_script_path();
            let script_path = script_path.to_string_lossy().into_owned();
            let mut command = Command::new("bash");
            command
                .current_dir(repo_root)
                .args([script_path.as_str(), "auth", "status"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .env("GIT_TERMINAL_PROMPT", "0");
            subprocess::command_output(&mut command, &format!("bash {script_path} auth status"))
                .map(|output| output.status)
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
            Some("verify gh auth or local git GitHub credentials".to_string()),
        )
    }

    /*
    PR lookup은 `ensure_pull_request`의 idempotency gate다.

    같은 base/head branch pair에 이미 open PR이 있으면 create를 다시 호출하지 않아야 review surface가 중복되지 않는다.
    wrapper script는 GitHub PR JSON shape를 돌려주지만, adapter는 즉시 application port record로 mapping한다.
    그 덕분에 application layer는 `baseRefName`/`headRefName` 같은 GitHub field spelling에 결합되지 않는다.
    */
    fn find_open_pull_request(
        &self,
        repo_root: &str,
        base_branch: &str,
        head_branch: &str,
    ) -> Result<Option<GithubAutomationPullRequest>> {
        let script_path = github_script_path();
        let script_path = script_path.to_string_lossy().into_owned();
        let output = run_command(
            "bash",
            &[
                script_path.as_str(),
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
        /*
        PR lookup은 application port가 노출하는 compact field만 요청한다.
        나중 코드가 GitHub 전용 세부 값에 branch하지 못하게 하려는 의도다.
        다른 provider-backed automation adapter가 추가되어도 number/url/state/base/head/draft contract만 맞추면 된다.
        */
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
        /*
        slot branch는 보통 upstream tracking과 함께 publish한다.
        이후 operator나 recovery command가 remote/refspec을 다시 입력하지 않고 branch 이름만 사용할 수 있게 하기 위해서다.
        rebased distributor recovery는 자신이 방금 검증한 branch만 rewrite하므로 force-with-lease를 쓴다.
        force push가 필요하지만, 다른 actor가 remote를 이동시킨 경우에는 lease가 실패해 안전하게 멈춘다.
        */
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
    ensure semantics는 PR creation을 retry-safe하게 만든다.

    adapter는 create 전에 같은 base/head open PR을 먼저 찾고, create 뒤에도 다시 찾는다.
    두 번째 lookup은 의도적이다. wrapper stdout은 URL일 수도 있고 future structured payload일 수도 있으며,
    두 호출 사이에 concurrent actor가 같은 PR을 만들 수도 있다. GitHub의 현재 PR 상태를 다시 읽는 것이 source of truth다.
    URL parsing은 그 다음의 recovery path일 뿐이다.
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

        /*
        create는 side-effectful이지만 public contract는 "ensure"다.
        timeout이나 transient wrapper failure 뒤 caller가 재시도해도 같은 branch pair에 중복 review surface를 만들지 않고
        기존 PR record를 받아야 한다.
        */
        let script_path = github_script_path();
        let script_path = script_path.to_string_lossy().into_owned();
        let create_output = run_command(
            "bash",
            &[
                script_path.as_str(),
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

        /*
        creation stdout을 신뢰하지 않고 다시 query한다.
        wrapper는 URL을 출력할 수도, 나중에 structured payload를 출력할 수도, 유용한 값을 출력하지 않을 수도 있다.
        distributor에 돌려줄 number/base/head/draft field의 source of truth는 GitHub에 다시 조회한 JSON이다.
        */
        if let Some(existing) = self.find_open_pull_request(repo_root, base_branch, head_branch)? {
            return Ok(existing);
        }
        if let Some(pr_number) = parse_pull_request_number_from_url(&create_output) {
            /*
            URL parsing은 흔한 CLI success shape를 위한 recovery path다.
            그래도 inspect_pull_request를 통과시켜 ordinary lookup과 같은 JSON-to-port mapping으로 반환 값을 만든다.
            */
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
        /*
        inspect는 creation fallback이나 이후 delivery check에서 쓰는 authoritative read path다.
        PR lookup과 같은 compact field set을 요청하므로, caller는 PR을 어떤 경로로 찾았는지와 무관하게 같은 port shape를 본다.
        */
        let script_path = github_script_path();
        let script_path = script_path.to_string_lossy().into_owned();
        let output = run_command(
            "bash",
            &[
                script_path.as_str(),
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
        /*
        integration branch는 이미 distributor worktree에서 합성된 결과다.
        upstream setup 없이 push하는 이유는 최종 integration이 계속 explicit branch/PR record를 통해 진행되어야 하기 때문이다.
        slot branch처럼 operator의 일상 작업 branch로 취급하지 않는다.
        */
        run_git(repo_root, &["push", DEFAULT_PUSH_REMOTE_NAME, branch_name])
    }

    fn close_pull_request(&self, repo_root: &str, pr_number: u64) -> Result<()> {
        /*
        close는 raw `gh` 대신 Akra wrapper에 위임한다.
        PR 생성/조회와 같은 script를 쓰면 write identity, token selection, repo-specific GitHub policy가 한 경계에 머문다.
        */
        let script_path = github_script_path();
        let script_path = script_path.to_string_lossy().into_owned();
        run_command(
            "bash",
            &[script_path.as_str(), "pr", "close", &pr_number.to_string()],
            repo_root,
        )?;
        Ok(())
    }
}

fn github_script_path() -> PathBuf {
    installed_github_script_path()
        .filter(|path| path.is_file())
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(GITHUB_SCRIPT_RELATIVE_PATH)
        })
}

fn installed_github_script_path() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|path| {
        path.parent()
            .map(|parent| parent.join(GITHUB_SCRIPT_RELATIVE_PATH))
    })
}

/*
GitHub PR JSON 중 이 adapter가 필요한 subset만 모델링한 private DTO다.

external field name은 여기 private type 안에 가둔다. application port record로 넘어가는 유일한 지점은 아래 `From`
implementation이며, 그 밖의 distributor/readiness code는 GitHub GraphQL/CLI JSON spelling을 알 필요가 없다.
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
        /*
        이 conversion은 GitHub camelCase JSON과 application port의 provider-neutral record 사이 membrane이다.
        mapping을 여기 고정하면 distributor나 readiness code가 `baseRefName` 같은 provider field에 직접 의존하지 않는다.
        */
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
    /*
    git command helper는 성공 시 unit을 반환한다.
    caller가 필요한 것은 side effect가 완료됐다는 사실이고, stdout payload가 아니다.
    실패 시 stderr/stdout을 error context로 확장해 distributor가 remote rejection, hook failure, auth failure 메시지를
    recovery note에 보존할 수 있게 한다.
    */
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
    /*
    git stdout caller는 capability/inspection 같은 read-only path다.
    run_command를 재사용해 GitHub wrapper invocation과 같은 non-interactive environment와 실패 context를 제공한다.
    이 통일 덕분에 git origin/branch lookup 실패도 PR automation 실패와 같은 방식으로 상위에 전달된다.
    */
    run_command("git", args, repo_root)
}

/*
command를 실행하고 성공한 경우에만 trimmed stdout을 반환한다.

모든 GitHub automation subprocess는 이 helper를 통과한다.
실패에는 program, argument, repo root, 그리고 가능한 command output을 함께 넣는다.
orchestration layer에는 bare exit status보다 "어떤 repo에서 어떤 wrapper/git command가 어떤 메시지로 실패했는가"가 필요하다.
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
    /*
    background parallel-mode delivery에서는 non-interactive execution이 필수다.
    terminal prompt를 막아 credential/network gap이 supervisor lane을 멈춰 세우는 interactive wait가 아니라
    일반 command failure로 드러나게 한다.
    */
    let mut command = Command::new(program);
    command
        .current_dir(repo_root)
        .args(args)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0");
    subprocess::command_output(&mut command, &format!("{program} {}", args.join(" "))).with_context(
        || {
            format!(
                "failed to run `{program} {}` in {repo_root}",
                args.join(" ")
            )
        },
    )
}

fn command_error_detail(output: &Output) -> String {
    /*
    대부분의 Git/GitHub failure는 stderr에 설명을 남기지만 wrapper script는 stdout으로 error를 normalize할 수 있다.
    stderr, stdout, stable fallback 순서로 읽어 가장 signal이 높은 메시지를 보존하면서 silent exit도 일정한 문장으로 만든다.
    */
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
    /*
    `gh pr create`와 wrapper는 흔히 생성된 PR URL을 출력한다.
    마지막 path segment만 parse해 이 logic을 narrow recovery path로 제한한다.
    structured PR lookup이 여전히 port data의 primary source다.
    */
    output
        .trim()
        /*
        slash로 나눈 마지막 segment만 PR number 후보로 삼는다.
        query string이나 URL이 아닌 wrapper chatter는 parse에 실패하고 structured lookup error로 이어진다.
        잘못된 숫자를 만들어 downstream inspect가 엉뚱한 PR을 보는 것보다 실패가 낫다.
        */
        .rsplit('/')
        .next()
        .and_then(|value| value.parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::{
        GithubAutomationAdapter, GithubPullRequestJson, parse_pull_request_number_from_url,
        run_command, run_git, run_git_stdout,
    };
    use crate::application::port::outbound::github_automation_port::{
        GithubAutomationPort, GithubAutomationPullRequest,
    };
    use crate::domain::parallel_mode::{
        ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    };

    #[test]
    fn pull_request_json_maps_only_the_application_port_contract() {
        let pull_request: GithubAutomationPullRequest =
            serde_json::from_value::<GithubPullRequestJson>(json!({
            "number": 1681,
            "url": "https://github.com/RefinedStone/codex-exec-loop/pull/1681",
            "state": "OPEN",
            "baseRefName": "prerelease",
            "headRefName": "feature/test-coverage",
            "isDraft": false
            }))
            .expect("GitHub PR JSON fixture should deserialize")
            .into();

        assert_eq!(pull_request.number, 1681);
        assert_eq!(
            pull_request.url,
            "https://github.com/RefinedStone/codex-exec-loop/pull/1681"
        );
        assert_eq!(pull_request.state, "OPEN");
        assert_eq!(pull_request.base_branch, "prerelease");
        assert_eq!(pull_request.head_branch, "feature/test-coverage");
        assert!(!pull_request.is_draft);
    }

    #[test]
    fn pull_request_number_parser_uses_only_the_last_path_segment() {
        assert_eq!(
            parse_pull_request_number_from_url(
                "https://github.com/RefinedStone/codex-exec-loop/pull/1681\n"
            ),
            Some(1681)
        );
        assert_eq!(parse_pull_request_number_from_url("1682"), Some(1682));
        assert_eq!(
            parse_pull_request_number_from_url(
                "https://github.com/RefinedStone/codex-exec-loop/pull/not-a-number"
            ),
            None
        );
        assert_eq!(
            parse_pull_request_number_from_url(
                "https://github.com/RefinedStone/codex-exec-loop/pull/1681/"
            ),
            None
        );
    }

    #[test]
    fn run_command_trims_stdout_and_reports_the_best_failure_detail() {
        let repo_root = unique_temp_dir("github-automation-command");
        let env_output = run_command(
            "sh",
            &["-c", "printf '  %s  \\n' \"$GIT_TERMINAL_PROMPT\""],
            path_str(&repo_root),
        )
        .expect("shell command should run");

        assert_eq!(env_output, "0");

        let stderr_error = run_command(
            "sh",
            &["-c", "printf 'stderr-detail' >&2; exit 7"],
            path_str(&repo_root),
        )
        .expect_err("stderr failure should be reported");
        assert!(stderr_error.to_string().contains("stderr-detail"));

        let stdout_error = run_command(
            "sh",
            &["-c", "printf 'stdout-detail'; exit 8"],
            path_str(&repo_root),
        )
        .expect_err("stdout failure should be reported when stderr is empty");
        assert!(stdout_error.to_string().contains("stdout-detail"));

        let silent_error = run_command("sh", &["-c", "exit 9"], path_str(&repo_root))
            .expect_err("silent failure should use a stable fallback");
        assert!(
            silent_error
                .to_string()
                .contains("command exited without output")
        );

        let missing_program = run_command(
            "__codex_exec_loop_missing_program__",
            &[],
            path_str(&repo_root),
        )
        .expect_err("spawn failure should keep command context");
        assert!(
            missing_program
                .to_string()
                .contains("failed to run `__codex_exec_loop_missing_program__")
        );
    }

    #[test]
    fn push_remote_capability_reports_ready_and_missing_origin_states() {
        let fixture = GitFixture::new("github-automation-capability");
        let capability = GithubAutomationAdapter::inspect_push_remote(path_str(&fixture.repo));

        assert_eq!(capability.key, ParallelModeCapabilityKey::PushRemote);
        assert_eq!(capability.state, ParallelModeCapabilityState::Ready);
        assert!(capability.detail.contains("push dry-run succeeded"));
        assert!(capability.next_action.is_none());

        let repo_without_origin = unique_temp_dir("github-automation-no-origin");
        git(&repo_without_origin, &["init"]);
        let missing = GithubAutomationAdapter::inspect_push_remote(path_str(&repo_without_origin));

        assert_eq!(missing.key, ParallelModeCapabilityKey::PushRemote);
        assert_eq!(missing.state, ParallelModeCapabilityState::Degraded);
        assert!(
            missing
                .detail
                .contains("push remote `origin` is not configured")
        );
        assert!(missing.next_action.is_some());
    }

    #[test]
    fn push_remote_capability_reports_dry_run_failures_for_current_branch() {
        let repo = unique_temp_dir("github-automation-bad-origin");
        git(&repo, &["init"]);
        git(&repo, &["config", "user.name", "RefinedStone"]);
        git(&repo, &["config", "user.email", "chem.en.9273@gmail.com"]);
        fs::write(repo.join("README.md"), "seed\n").expect("fixture file should be written");
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-m", "Initial commit"]);
        git(&repo, &["branch", "-M", "main"]);
        git(
            &repo,
            &["remote", "add", "origin", "/tmp/akra-missing-origin.git"],
        );

        let capability = GithubAutomationAdapter::inspect_push_remote(path_str(&repo));

        assert_eq!(capability.key, ParallelModeCapabilityKey::PushRemote);
        assert_eq!(capability.state, ParallelModeCapabilityState::Degraded);
        assert!(capability.detail.contains("git push --dry-run failed"));
        assert!(capability.next_action.is_some());
    }

    #[test]
    fn push_remote_capability_reports_configured_remote_without_current_branch() {
        let fixture = GitFixture::new("github-automation-detached-head-capability");
        git(&fixture.repo, &["checkout", "--detach"]);

        let capability = GithubAutomationAdapter::inspect_push_remote(path_str(&fixture.repo));

        assert_eq!(capability.key, ParallelModeCapabilityKey::PushRemote);
        assert_eq!(capability.state, ParallelModeCapabilityState::Ready);
        assert!(capability.detail.contains("push remote is configured at"));
        assert!(capability.next_action.is_none());
    }

    #[test]
    fn gh_auth_capability_degrades_when_command_surface_is_not_ready() {
        let gh_binary = ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhBinary,
            ParallelModeCapabilityState::Degraded,
            "gh missing",
            Some("install gh".to_string()),
        );

        let capability = GithubAutomationAdapter::inspect_gh_auth(&gh_binary, ".");

        assert_eq!(capability.key, ParallelModeCapabilityKey::GhAuth);
        assert_eq!(capability.state, ParallelModeCapabilityState::Degraded);
        assert!(capability.detail.contains("gh auth is unavailable"));
        assert!(capability.next_action.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn gh_capabilities_use_cli_when_gh_is_on_path() {
        let _guard = github_script_lock()
            .lock()
            .expect("github script fixture lock should not be poisoned");
        let root = unique_temp_dir("github-automation-fake-gh");
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).expect("fake gh bin directory should be created");
        write_fake_gh(&bin_dir, 0);
        let _path_guard = PathEnvGuard::prepend(&bin_dir);

        let gh_binary = GithubAutomationAdapter::inspect_gh_binary();

        assert_eq!(gh_binary.key, ParallelModeCapabilityKey::GhBinary);
        assert_eq!(gh_binary.state, ParallelModeCapabilityState::Ready);
        assert!(gh_binary.detail.contains("gh found at"));

        let gh_auth = GithubAutomationAdapter::inspect_gh_auth(&gh_binary, path_str(&root));
        assert_eq!(gh_auth.key, ParallelModeCapabilityKey::GhAuth);
        assert_eq!(gh_auth.state, ParallelModeCapabilityState::Ready);

        write_fake_gh(&bin_dir, 9);
        let failed_auth = GithubAutomationAdapter::inspect_gh_auth(&gh_binary, path_str(&root));
        assert_eq!(failed_auth.key, ParallelModeCapabilityKey::GhAuth);
        assert_eq!(failed_auth.state, ParallelModeCapabilityState::Degraded);
        assert!(failed_auth.detail.contains("not authenticated"));
        assert!(failed_auth.next_action.is_some());
    }

    #[test]
    fn default_adapter_inspects_capability_contract_shape() {
        let _guard = github_script_lock()
            .lock()
            .expect("github script fixture lock should not be poisoned");
        remove_fake_github_script();
        let fixture = GitFixture::new("github-automation-default-capabilities");
        #[allow(clippy::default_constructed_unit_structs)]
        let adapter = GithubAutomationAdapter::default();

        let capabilities = adapter.inspect_capabilities(path_str(&fixture.repo));

        assert_eq!(
            capabilities.push_remote.key,
            ParallelModeCapabilityKey::PushRemote
        );
        assert_eq!(
            capabilities.gh_binary.key,
            ParallelModeCapabilityKey::GhBinary
        );
        assert_eq!(capabilities.gh_auth.key, ParallelModeCapabilityKey::GhAuth);
    }

    #[test]
    fn pull_request_lifecycle_uses_wrapper_lookup_create_inspect_and_close() {
        let _guard = github_script_lock()
            .lock()
            .expect("github script fixture lock should not be poisoned");
        let script_path = install_fake_github_script();
        let repo = unique_temp_dir("github-automation-pr-lifecycle");
        let adapter = GithubAutomationAdapter::new();

        let existing = adapter
            .ensure_pull_request(
                path_str(&repo),
                "prerelease",
                "feature/existing",
                "Existing",
                "body",
            )
            .expect("existing PR should be returned from list");
        assert_eq!(existing.number, 41);
        assert_eq!(existing.head_branch, "feature/existing");

        let created = adapter
            .ensure_pull_request(path_str(&repo), "prerelease", "feature/new", "New", "body")
            .expect("create URL fallback should inspect created PR");
        assert_eq!(created.number, 42);
        assert_eq!(created.base_branch, "prerelease");

        let created_from_second_lookup = adapter
            .ensure_pull_request(
                path_str(&repo),
                "prerelease",
                "feature/race",
                "Race",
                "body",
            )
            .expect("second lookup should recover a PR created by the wrapper");
        assert_eq!(created_from_second_lookup.number, 43);
        assert_eq!(created_from_second_lookup.head_branch, "feature/race");

        let no_url = adapter
            .ensure_pull_request(
                path_str(&repo),
                "prerelease",
                "feature/no-url",
                "No URL",
                "body",
            )
            .expect_err("create without lookup or URL should report ensure failure");
        assert!(
            no_url
                .to_string()
                .contains("no open PR was found for `feature/no-url`")
        );

        let invalid_list = adapter
            .ensure_pull_request(
                path_str(&repo),
                "prerelease",
                "feature/bad-list",
                "Bad List",
                "body",
            )
            .expect_err("invalid PR list JSON should include parse context");
        assert!(
            invalid_list
                .to_string()
                .contains("failed to parse `gh pr list` output while locating `feature/bad-list`")
        );

        let list_failure = adapter
            .ensure_pull_request(
                path_str(&repo),
                "prerelease",
                "feature/list-fail",
                "List Fail",
                "body",
            )
            .expect_err("PR list command failure should stop ensure before create");
        assert!(list_failure.to_string().contains("list denied"));

        let create_failure = adapter
            .ensure_pull_request(
                path_str(&repo),
                "prerelease",
                "feature/create-fail",
                "Create Fail",
                "body",
            )
            .expect_err("PR create command failure should be reported");
        assert!(create_failure.to_string().contains("create denied"));

        let invalid_view = adapter
            .inspect_pull_request(path_str(&repo), 99)
            .expect_err("invalid PR JSON should include parse context");
        assert!(
            invalid_view
                .to_string()
                .contains("failed to parse `gh pr view` output for PR #99")
        );

        let view_failure = adapter
            .inspect_pull_request(path_str(&repo), 13)
            .expect_err("PR view command failure should be reported");
        assert!(view_failure.to_string().contains("view denied"));

        adapter
            .close_pull_request(path_str(&repo), 42)
            .expect("close should delegate to wrapper");

        let close_failure = adapter
            .close_pull_request(path_str(&repo), 13)
            .expect_err("PR close command failure should be reported");
        assert!(close_failure.to_string().contains("close denied"));

        let _ = fs::remove_file(script_path);
    }

    #[test]
    fn push_methods_publish_local_branches_to_origin() {
        let fixture = GitFixture::new("github-automation-push");
        let adapter = GithubAutomationAdapter::new();

        adapter
            .push_branch(path_str(&fixture.repo), "main", false)
            .expect("branch push should publish to local origin");
        assert_eq!(
            git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
            git_stdout(&fixture.repo, &["rev-parse", "main"])
        );

        fs::write(fixture.repo.join("README.md"), "updated\n")
            .expect("fixture file should be writable");
        git(&fixture.repo, &["add", "README.md"]);
        git(&fixture.repo, &["commit", "-m", "Update readme"]);

        adapter
            .push_branch(path_str(&fixture.repo), "main", true)
            .expect("force-with-lease push should publish rewritten branch");
        adapter
            .push_integration_branch(path_str(&fixture.repo), "main")
            .expect("integration push should use the same local origin");

        assert_eq!(
            git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
            git_stdout(&fixture.repo, &["rev-parse", "main"])
        );
    }

    #[test]
    fn git_helpers_trim_stdout_and_attach_failure_context() {
        let fixture = GitFixture::new("github-automation-git-helper");

        assert_eq!(
            run_git_stdout(path_str(&fixture.repo), &["branch", "--show-current"])
                .expect("git stdout helper should trim branch name"),
            "main"
        );

        let error = run_git(path_str(&fixture.repo), &["definitely-not-a-git-command"])
            .expect_err("git helper should reject failed commands");

        assert!(
            error
                .to_string()
                .contains("git definitely-not-a-git-command failed")
        );
    }

    struct GitFixture {
        repo: PathBuf,
        remote: PathBuf,
    }

    impl GitFixture {
        fn new(label: &str) -> Self {
            let root = unique_temp_dir(label);
            let remote = root.join("origin.git");
            let repo = root.join("repo");
            fs::create_dir_all(&repo).expect("repo directory should be created");

            git_in(&root, &["init", "--bare", "origin.git"]);
            git(&repo, &["init"]);
            git(&repo, &["config", "user.name", "RefinedStone"]);
            git(&repo, &["config", "user.email", "chem.en.9273@gmail.com"]);
            fs::write(repo.join("README.md"), "initial\n").expect("fixture file should be written");
            git(&repo, &["add", "README.md"]);
            git(&repo, &["commit", "-m", "Initial commit"]);
            git(&repo, &["branch", "-M", "main"]);
            git(&repo, &["remote", "add", "origin", path_str(&remote)]);

            Self { repo, remote }
        }
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "codex-exec-loop-{label}-{}-{now}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary directory should be created");
        path
    }

    fn github_script_lock() -> &'static Mutex<()> {
        static LOCK: Mutex<()> = Mutex::new(());
        &LOCK
    }

    fn install_fake_github_script() -> PathBuf {
        let script_path = fake_github_script_path();
        let script_dir = script_path
            .parent()
            .expect("fake github script path should have a parent");
        fs::create_dir_all(script_dir).expect("fake github script directory should be created");
        fs::write(
            &script_path,
            r#"#!/usr/bin/env bash
set -euo pipefail
args="$*"
case "$args" in
  "auth status")
    exit 0
    ;;
  pr\ list*feature/existing*)
    printf '%s\n' '[{"number":41,"url":"https://github.example/pull/41","state":"OPEN","baseRefName":"prerelease","headRefName":"feature/existing","isDraft":false}]'
    ;;
  pr\ list*feature/race*)
    if [[ -f .fake-gh-race-created ]]; then
      printf '%s\n' '[{"number":43,"url":"https://github.example/pull/43","state":"OPEN","baseRefName":"prerelease","headRefName":"feature/race","isDraft":false}]'
    else
      printf '%s\n' '[]'
    fi
    ;;
  pr\ list*feature/bad-list*)
    printf '%s\n' '[not-json'
    ;;
  pr\ list*feature/list-fail*)
    printf '%s\n' 'list denied' >&2
    exit 22
    ;;
  pr\ list*)
    printf '%s\n' '[]'
    ;;
  pr\ create*feature/race*)
    touch .fake-gh-race-created
    printf '%s\n' 'created without url'
    ;;
  pr\ create*feature/new*)
    printf '%s\n' 'https://github.example/pull/42'
    ;;
  pr\ create*feature/create-fail*)
    printf '%s\n' 'create denied' >&2
    exit 23
    ;;
  pr\ create*feature/no-url*)
    printf '%s\n' 'created without url'
    ;;
  pr\ view\ 13*)
    printf '%s\n' 'view denied' >&2
    exit 13
    ;;
  pr\ view\ 42*)
    printf '%s\n' '{"number":42,"url":"https://github.example/pull/42","state":"OPEN","baseRefName":"prerelease","headRefName":"feature/new","isDraft":false}'
    ;;
  pr\ view\ 99*)
    printf '%s\n' '{not-json'
    ;;
  pr\ close\ 13*)
    printf '%s\n' 'close denied' >&2
    exit 14
    ;;
  pr\ close\ 42*)
    printf '%s\n' 'closed'
    ;;
  *)
    printf 'unexpected fake gh-akra args: %s\n' "$args" >&2
    exit 12
    ;;
esac
"#,
        )
        .expect("fake github script should be written");
        script_path
    }

    fn remove_fake_github_script() {
        let _ = fs::remove_file(fake_github_script_path());
    }

    fn fake_github_script_path() -> PathBuf {
        std::env::current_exe()
            .expect("test binary path should be available")
            .parent()
            .expect("test binary should have a parent")
            .join("scripts")
            .join("gh-akra.sh")
    }

    #[cfg(unix)]
    fn write_fake_gh(bin_dir: &Path, auth_status_exit_code: i32) {
        let gh_path = bin_dir.join("gh");
        fs::write(
            &gh_path,
            format!(
                r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == "auth status" ]]; then
  exit {auth_status_exit_code}
fi
printf 'unexpected fake gh args: %s\n' "$*" >&2
exit 66
"#
            ),
        )
        .expect("fake gh should be written");
        let mut permissions = fs::metadata(&gh_path)
            .expect("fake gh metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&gh_path, permissions).expect("fake gh should be executable");
    }

    #[cfg(unix)]
    struct PathEnvGuard {
        previous: Option<std::ffi::OsString>,
    }

    #[cfg(unix)]
    impl PathEnvGuard {
        fn prepend(directory: &Path) -> Self {
            let previous = std::env::var_os("PATH");
            let mut paths = vec![directory.to_path_buf()];
            if let Some(path) = &previous {
                paths.extend(std::env::split_paths(path));
            }
            let joined_path = std::env::join_paths(paths).expect("test PATH should join");
            unsafe {
                std::env::set_var("PATH", joined_path);
            }
            Self { previous }
        }
    }

    #[cfg(unix)]
    impl Drop for PathEnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(path) => std::env::set_var("PATH", path),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }

    fn git(repo: &Path, args: &[&str]) {
        git_in(repo, args);
    }

    fn git_stdout(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {}: {error}", args.join(" ")));
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn git_in(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {}: {error}", args.join(" ")));
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn path_str(path: &Path) -> &str {
        path.to_str().expect("test path should be valid UTF-8")
    }
}

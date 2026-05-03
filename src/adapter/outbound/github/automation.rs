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
    authentication capability는 의도적으로 output을 버리는 status command만 실행한다.

    application port가 필요한 것은 ready/degraded 신호와 operator-facing hint이지 raw credential detail이 아니다.
    그래서 adapter는 stdout/stderr를 숨기고, `gh auth status` 또는 RefinedStone script의 auth check 결과를
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
            Command::new("gh")
                .current_dir(repo_root)
                .args(["auth", "status"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .env("GIT_TERMINAL_PROMPT", "0")
                .status()
        } else {
            /*
            repo wrapper는 이 project의 supported fallback이다.
            CI나 `gh`가 없는 local machine도 아래 write operation과 같은 RefinedStone credential path를 사용하게 한다.
            capability check와 실제 PR write가 같은 wrapper contract를 공유해야 "ready" 판단과 실행 경로가 어긋나지 않는다.
            */
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
        /*
        integration branch는 이미 distributor worktree에서 합성된 결과다.
        upstream setup 없이 push하는 이유는 최종 integration이 계속 explicit branch/PR record를 통해 진행되어야 하기 때문이다.
        slot branch처럼 operator의 일상 작업 branch로 취급하지 않는다.
        */
        run_git(repo_root, &["push", DEFAULT_PUSH_REMOTE_NAME, branch_name])
    }

    fn close_pull_request(&self, repo_root: &str, pr_number: u64) -> Result<()> {
        /*
        close는 raw `gh` 대신 RefinedStone wrapper에 위임한다.
        PR 생성/조회와 같은 script를 쓰면 write identity, token selection, repo-specific GitHub policy가 한 경계에 머문다.
        */
        run_command(
            "bash",
            &[GITHUB_SCRIPT_PATH, "pr", "close", &pr_number.to_string()],
            repo_root,
        )?;
        Ok(())
    }
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

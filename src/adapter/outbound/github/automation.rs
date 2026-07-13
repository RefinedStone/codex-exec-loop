/*
GitHub automation outbound adapter다.

parallel-mode orchestration은 branch push, PR 생성/조회, capability inspection을 application port로만
바라본다. 이 파일은 그 port 호출을 격리된 git 명령과 build-time에 검토·임베드된 `gh-akra.sh` 실행으로
변환한다. GitHub CLI가 신뢰된 시스템 위치에 있으면 로컬 인증을 가져오고, 없으면 같은 임베드 helper가
token 기반 REST fallback을 제공한다. 저장소나 설치 디렉터리의 script bytes와 PATH는 실행하지 않는다.
*/
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use rand::{RngCore, rngs::OsRng};

use serde::Deserialize;

use crate::application::port::outbound::github_automation_port::{
    AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY, AKRA_GITHUB_PUSH_REMOTE_ENV_VAR,
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
    GithubRepositoryVisibility, credential_redacted_canonical_github_push_url,
    parse_github_repository_identity, resolve_github_push_remote_name_strict,
};
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
};
use crate::git_subprocess;
use crate::subprocess;

pub struct GithubAutomationAdapter;

const FROZEN_GITHUB_REMOTE_NAME: &str = "akra-frozen-delivery-target";
const MAX_FROZEN_REMOTE_BRANCHES_PER_PREFIX: usize = 4096;
const FROZEN_GITHUB_TOKEN_ENV_VAR: &str = "AKRA_FROZEN_GITHUB_TOKEN";
const FROZEN_GITHUB_LOGIN_ENV_VAR: &str = "AKRA_FROZEN_GITHUB_LOGIN";
const EMBEDDED_GITHUB_HELPER: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/gh-akra.sh"));
const REQUIRED_GITHUB_HELPER_TOOLS: &[&str] = &[
    "sh", "git", "cat", "curl", "grep", "python3", "mktemp", "rm",
];

#[cfg(test)]
thread_local! {
    static TEST_GITHUB_HELPER_SOURCE: std::cell::RefCell<Option<&'static [u8]>> =
        const { std::cell::RefCell::new(None) };
}

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
        let Ok(push_remote) = configured_push_remote_name(repo_root) else {
            return ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Blocked,
                "configured push remote name is invalid",
                Some("fix AKRA_GITHUB_PUSH_REMOTE or akra.githubPushRemote".to_string()),
            );
        };
        let adapter = Self::new();
        let Ok(push_url) =
            adapter.credential_redacted_push_url_for_remote(repo_root, push_remote.as_str())
        else {
            return ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Degraded,
                format!(
                    "push remote `{}` is not a credential-free GitHub HTTPS delivery target",
                    push_remote
                ),
                Some(
                    "configure a credential-free GitHub HTTPS push remote for autonomous delivery"
                        .to_string(),
                ),
            );
        };

        if let Ok(current_branch) = run_git_stdout(repo_root, &["branch", "--show-current"])
            && !current_branch.is_empty()
        {
            let Ok(current_commit) = run_git_stdout(repo_root, &["rev-parse", "HEAD^{commit}"])
            else {
                return ParallelModeCapabilitySnapshot::new(
                    ParallelModeCapabilityKey::PushRemote,
                    ParallelModeCapabilityState::Degraded,
                    "current branch commit could not be frozen for push dry-run",
                    Some("repair the current Git branch before enabling delivery".to_string()),
                );
            };
            let refspec = format!("{current_commit}:refs/heads/{current_branch}");
            let push_error = match run_git_to_delivery_target(
                repo_root,
                &push_url,
                &[
                    "push",
                    "--dry-run",
                    FROZEN_GITHUB_REMOTE_NAME,
                    refspec.as_str(),
                ],
            ) {
                Ok(()) => {
                    return ParallelModeCapabilitySnapshot::new(
                        ParallelModeCapabilityKey::PushRemote,
                        ParallelModeCapabilityState::Ready,
                        format!("push dry-run succeeded for `{current_branch}`"),
                        None,
                    );
                }
                Err(error) => error,
            };
            let error_detail = sanitize_command_output(&format!("{push_error:#}"));
            return ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Degraded,
                format!(
                    "push readiness probe failed for `{current_branch}` via remote `{push_remote}`: {error_detail}"
                ),
                Some(
                    "repair the reported GitHub identity or git push failure before enabling delivery"
                        .to_string(),
                ),
            );
        }

        match verify_github_write_identity_for_delivery_target(repo_root, &push_url) {
            Ok(()) => ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Ready,
                format!("push remote `{}` is configured", push_remote),
                None,
            ),
            Err(_) => ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Degraded,
                format!("push remote `{}` identity verification failed", push_remote),
                Some("verify the pinned GitHub login and API token".to_string()),
            ),
        }
    }

    /*
    GitHub command capability는 두 실행 경로를 함께 본다.

    trusted system path에 `gh`가 있으면 사람이 익숙한 GitHub CLI 상태를 보고한다. 없더라도 binary에
    임베드된 helper의 REST fallback은 계속 가능하다. helper가 요구하는 trusted bash/toolchain 자체가
    없을 때만 PR automation을 blocked로 표시한다.
    */
    fn inspect_gh_binary(repo_root: &str) -> ParallelModeCapabilitySnapshot {
        let executables = match TrustedGithubExecutables::resolve(repo_root) {
            Ok(executables) => executables,
            Err(error) => {
                return ParallelModeCapabilitySnapshot::new(
                    ParallelModeCapabilityKey::GhBinary,
                    ParallelModeCapabilityState::Blocked,
                    format!("trusted GitHub automation runtime is unavailable: {error}"),
                    Some("install bash and the required GitHub helper tools in a trusted system location".to_string()),
                );
            }
        };
        match executables.gh {
            Some(path) => ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GhBinary,
                ParallelModeCapabilityState::Ready,
                format!("gh found at {}", path.display()),
                None,
            ),
            None => ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GhBinary,
                ParallelModeCapabilityState::Ready,
                "gh is not installed in a trusted system location; the embedded Akra GitHub API fallback is available",
                None,
            ),
        }
    }

    /*
    authentication capability는 의도적으로 output을 버리는 status command만 실행한다.

    application port가 필요한 것은 ready/degraded 신호와 operator-facing hint이지 raw credential detail이 아니다.
    그래서 adapter는 stdout/stderr를 숨기고, `gh auth status` 또는 Akra script의 auth check 결과를
    ParallelModeCapabilitySnapshot으로만 접는다. credential 위치와 token 문자열은 이 outbound boundary 밖으로 새지 않는다.
    */
    fn inspect_gh_auth(
        _gh_binary: &ParallelModeCapabilitySnapshot,
        repo_root: &str,
    ) -> ParallelModeCapabilitySnapshot {
        /*
        실제 PR 생성은 Akra wrapper의 token discovery 계약을 쓴다.
        readiness도 같은 wrapper를 따라야 `gh` binary 존재 여부와 상관없이 explicit env token과 trusted
        gh auth token 경로를 같은 기준으로 본다.
        */
        let Ok(push_remote) = configured_push_remote_name(repo_root) else {
            return ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GhAuth,
                ParallelModeCapabilityState::Blocked,
                "GitHub authentication was not attempted because the push remote is invalid",
                Some("fix AKRA_GITHUB_PUSH_REMOTE or akra.githubPushRemote".to_string()),
            );
        };
        let adapter = Self::new();
        let Ok(push_url) = adapter.credential_redacted_push_url_for_remote(repo_root, &push_remote)
        else {
            return ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GhAuth,
                ParallelModeCapabilityState::Degraded,
                "GitHub authentication requires a credential-free HTTPS delivery target",
                Some("configure the autonomous delivery push remote as GitHub HTTPS".to_string()),
            );
        };
        let auth_result = run_github_script_command_for_delivery_target(
            repo_root,
            &push_url,
            &["auth", "write-status"],
        );
        if auth_result.is_ok() {
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
            format!(
                "GitHub automation is not authenticated for this workspace: {}",
                auth_result.expect_err("failed authentication result should contain an error")
            ),
            Some(
                "verify trusted gh auth token or AKRA_GITHUB_TOKEN/GH_TOKEN/GITHUB_TOKEN"
                    .to_string(),
            ),
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
        credential_redacted_push_url: &str,
        base_branch: &str,
        head_branch: &str,
    ) -> Result<Option<GithubAutomationPullRequest>> {
        let output = run_github_script_command_for_delivery_target(
            repo_root,
            credential_redacted_push_url,
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
                "number,url,state,baseRefName,headRefName,headRefOid,isDraft,reviewDecision,mergeStateStatus,statusCheckRollup,approvedReviewCommitOids",
            ],
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
        let gh_binary = Self::inspect_gh_binary(repo_root);
        let gh_auth = Self::inspect_gh_auth(&gh_binary, repo_root);
        GithubAutomationCapabilities::new(push_remote, gh_binary, gh_auth)
    }

    fn repository_identity(&self, repo_root: &str) -> Result<String> {
        let push_remote = configured_push_remote_name(repo_root)?;
        self.repository_identity_for_remote(repo_root, &push_remote)
    }

    fn repository_identity_for_remote(&self, repo_root: &str, push_remote: &str) -> Result<String> {
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, push_remote)?;
        self.repository_identity_for_push_url(repo_root, push_remote, &push_url)
    }

    fn credential_redacted_push_url_for_remote(
        &self,
        repo_root: &str,
        push_remote: &str,
    ) -> Result<String> {
        let raw_push_url =
            run_git_stdout(repo_root, &["remote", "get-url", "--push", push_remote])?;
        let push_url = match credential_redacted_canonical_github_push_url(&raw_push_url) {
            Some(push_url) => push_url,
            #[cfg(test)]
            None if is_test_local_push_url(&raw_push_url) => raw_push_url.trim().to_string(),
            None => {
                return Err(anyhow!(
                    "configured push remote `{push_remote}` does not have a supported credential-redactable GitHub push URL"
                ));
            }
        };
        if !push_url.starts_with("https://github.com/") {
            #[cfg(test)]
            if is_test_local_push_url(&push_url) {
                return Ok(push_url);
            }
            bail!(
                "configured push remote `{push_remote}` must use credential-free GitHub HTTPS for autonomous delivery"
            );
        }
        Ok(push_url)
    }

    fn repository_identity_for_push_url(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        credential_redacted_push_url: &str,
    ) -> Result<String> {
        validate_credential_redacted_push_url(credential_redacted_push_url)?;
        parse_github_repository_identity(credential_redacted_push_url).ok_or_else(|| {
            anyhow!("frozen push URL does not identify a supported GitHub repository")
        })
    }

    fn repository_visibility(&self, repo_root: &str) -> Result<GithubRepositoryVisibility> {
        let push_remote = configured_push_remote_name(repo_root)?;
        self.repository_visibility_for_remote(repo_root, &push_remote)
    }

    fn repository_visibility_for_remote(
        &self,
        repo_root: &str,
        push_remote: &str,
    ) -> Result<GithubRepositoryVisibility> {
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, push_remote)?;
        self.repository_visibility_for_push_url(repo_root, push_remote, &push_url)
    }

    fn repository_visibility_for_push_url(
        &self,
        repo_root: &str,
        _push_remote: &str,
        credential_redacted_push_url: &str,
    ) -> Result<GithubRepositoryVisibility> {
        match run_github_script_command_for_delivery_target(
            repo_root,
            credential_redacted_push_url,
            &["repo", "visibility"],
        )?
        .trim()
        {
            "private" => Ok(GithubRepositoryVisibility::Private),
            "internal" => Ok(GithubRepositoryVisibility::Internal),
            "public" => Ok(GithubRepositoryVisibility::Public),
            _ => bail!("GitHub repository visibility response was unknown"),
        }
    }

    fn push_branch(
        &self,
        repo_root: &str,
        branch_name: &str,
        force_with_lease: bool,
    ) -> Result<()> {
        let push_remote = configured_push_remote_name(repo_root)?;
        self.push_branch_to_remote(repo_root, &push_remote, branch_name, force_with_lease)
    }

    fn push_branch_to_remote(
        &self,
        repo_root: &str,
        push_remote: &str,
        branch_name: &str,
        force_with_lease: bool,
    ) -> Result<()> {
        /*
        slot branch는 보통 upstream tracking과 함께 publish한다.
        이후 operator나 recovery command가 remote/refspec을 다시 입력하지 않고 branch 이름만 사용할 수 있게 하기 위해서다.
        rebased distributor recovery는 자신이 방금 검증한 branch만 rewrite하므로 force-with-lease를 쓴다.
        force push가 필요하지만, 다른 actor가 remote를 이동시킨 경우에는 lease가 실패해 안전하게 멈춘다.
        */
        run_git(repo_root, &["check-ref-format", "--branch", branch_name])?;
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, push_remote)?;
        let commit_ref = format!("{branch_name}^{{commit}}");
        let commit_sha = run_git_stdout(repo_root, &["rev-parse", "--verify", &commit_ref])?;
        let refspec = format!("{commit_sha}:refs/heads/{branch_name}");
        if force_with_lease {
            let expected_remote_head = self
                .remote_branch_head_for_delivery_target(
                    repo_root,
                    push_remote,
                    &push_url,
                    branch_name,
                )?
                .ok_or_else(|| anyhow!("force-with-lease target branch is absent"))?;
            let lease =
                format!("--force-with-lease=refs/heads/{branch_name}:{expected_remote_head}");
            run_git_to_delivery_target(
                repo_root,
                &push_url,
                &["push", &lease, FROZEN_GITHUB_REMOTE_NAME, &refspec],
            )
        } else {
            run_git_to_delivery_target(
                repo_root,
                &push_url,
                &["push", FROZEN_GITHUB_REMOTE_NAME, &refspec],
            )
        }
    }

    fn push_frozen_commit_to_branch(
        &self,
        repo_root: &str,
        push_remote: &str,
        source_commit_sha: &str,
        branch_name: &str,
    ) -> Result<()> {
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, push_remote)?;
        self.push_frozen_commit_to_delivery_target(
            repo_root,
            push_remote,
            &push_url,
            source_commit_sha,
            branch_name,
        )
    }

    fn push_frozen_commit_to_delivery_target(
        &self,
        repo_root: &str,
        _push_remote: &str,
        credential_redacted_push_url: &str,
        source_commit_sha: &str,
        branch_name: &str,
    ) -> Result<()> {
        // The source side of this refspec is the immutable queue OID rather than
        // a mutable local branch. A commit appended after queueing therefore
        // cannot be published even if the branch moves between validation and
        // `git push` process creation.
        let refspec = format!("{source_commit_sha}:refs/heads/{branch_name}");
        run_git_to_delivery_target(
            repo_root,
            credential_redacted_push_url,
            &["push", FROZEN_GITHUB_REMOTE_NAME, refspec.as_str()],
        )
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
        let push_remote = configured_push_remote_name(repo_root)?;
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, &push_remote)?;
        self.ensure_pull_request_for_delivery_target(
            repo_root,
            &push_remote,
            &push_url,
            base_branch,
            head_branch,
            title,
            body,
        )
    }

    fn ensure_pull_request_for_remote(
        &self,
        repo_root: &str,
        push_remote: &str,
        base_branch: &str,
        head_branch: &str,
        title: &str,
        body: &str,
    ) -> Result<GithubAutomationPullRequest> {
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, push_remote)?;
        self.ensure_pull_request_for_delivery_target(
            repo_root,
            push_remote,
            &push_url,
            base_branch,
            head_branch,
            title,
            body,
        )
    }

    fn ensure_pull_request_for_delivery_target(
        &self,
        repo_root: &str,
        push_remote: &str,
        credential_redacted_push_url: &str,
        base_branch: &str,
        head_branch: &str,
        title: &str,
        body: &str,
    ) -> Result<GithubAutomationPullRequest> {
        if let Some(existing) = self.find_open_pull_request(
            repo_root,
            credential_redacted_push_url,
            base_branch,
            head_branch,
        )? {
            return Ok(existing);
        }

        /*
        create는 side-effectful이지만 public contract는 "ensure"다.
        timeout이나 transient wrapper failure 뒤 caller가 재시도해도 같은 branch pair에 중복 review surface를 만들지 않고
        기존 PR record를 받아야 한다.
        */
        let title_file = TemporaryTextFile::new("github-pr-title", title)?;
        let title_file_path = title_file.path().to_string_lossy().into_owned();
        let body_file = TemporaryTextFile::new("github-pr-body", body)?;
        let body_file_path = body_file.path().to_string_lossy().into_owned();
        let create_args = vec![
            "pr".to_string(),
            "create".to_string(),
            "--base".to_string(),
            base_branch.to_string(),
            "--head".to_string(),
            head_branch.to_string(),
            "--title-file".to_string(),
            title_file_path,
            "--body-file".to_string(),
            body_file_path,
        ];
        let create_arg_refs = create_args.iter().map(String::as_str).collect::<Vec<_>>();
        let create_output = run_github_script_command_with_label_for_delivery_target(
            repo_root,
            credential_redacted_push_url,
            &create_arg_refs,
            &pull_request_create_command_label(base_branch, head_branch, title, body),
        )?;

        /*
        creation stdout을 신뢰하지 않고 다시 query한다.
        wrapper는 URL을 출력할 수도, 나중에 structured payload를 출력할 수도, 유용한 값을 출력하지 않을 수도 있다.
        distributor에 돌려줄 number/base/head/draft field의 source of truth는 GitHub에 다시 조회한 JSON이다.
        */
        if let Some(existing) = self.find_open_pull_request(
            repo_root,
            credential_redacted_push_url,
            base_branch,
            head_branch,
        )? {
            return Ok(existing);
        }
        if let Some(pr_number) = parse_pull_request_number_from_url(&create_output) {
            /*
            URL parsing은 흔한 CLI success shape를 위한 recovery path다.
            그래도 inspect_pull_request를 통과시켜 ordinary lookup과 같은 JSON-to-port mapping으로 반환 값을 만든다.
            */
            return self.inspect_pull_request_for_delivery_target(
                repo_root,
                push_remote,
                credential_redacted_push_url,
                pr_number,
            );
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
        let push_remote = configured_push_remote_name(repo_root)?;
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, &push_remote)?;
        self.inspect_pull_request_for_delivery_target(repo_root, &push_remote, &push_url, pr_number)
    }

    fn inspect_pull_request_for_remote(
        &self,
        repo_root: &str,
        push_remote: &str,
        pr_number: u64,
    ) -> Result<GithubAutomationPullRequest> {
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, push_remote)?;
        self.inspect_pull_request_for_delivery_target(repo_root, push_remote, &push_url, pr_number)
    }

    fn inspect_pull_request_for_delivery_target(
        &self,
        repo_root: &str,
        _push_remote: &str,
        credential_redacted_push_url: &str,
        pr_number: u64,
    ) -> Result<GithubAutomationPullRequest> {
        /*
        inspect는 creation fallback이나 이후 delivery check에서 쓰는 authoritative read path다.
        PR lookup과 같은 compact field set을 요청하므로, caller는 PR을 어떤 경로로 찾았는지와 무관하게 같은 port shape를 본다.
        */
        let output = run_github_script_command_for_delivery_target(
            repo_root,
            credential_redacted_push_url,
            &[
                "pr",
                "view",
                &pr_number.to_string(),
                "--json",
                "number,url,state,baseRefName,headRefName,headRefOid,isDraft,reviewDecision,mergeStateStatus,statusCheckRollup,approvedReviewCommitOids",
            ],
        )?;
        let pull_request = serde_json::from_str::<GithubPullRequestJson>(&output)
            .with_context(|| format!("failed to parse `gh pr view` output for PR #{pr_number}"))?;
        Ok(pull_request.into())
    }

    fn push_integration_branch(
        &self,
        repo_root: &str,
        branch_name: &str,
        expected_old_commit_sha: &str,
    ) -> Result<()> {
        let push_remote = configured_push_remote_name(repo_root)?;
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, &push_remote)?;
        self.push_integration_branch_to_delivery_target(
            repo_root,
            &push_remote,
            &push_url,
            branch_name,
            expected_old_commit_sha,
        )
    }

    fn push_integration_branch_to_remote(
        &self,
        repo_root: &str,
        push_remote: &str,
        branch_name: &str,
        expected_old_commit_sha: &str,
    ) -> Result<()> {
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, push_remote)?;
        self.push_integration_branch_to_delivery_target(
            repo_root,
            push_remote,
            &push_url,
            branch_name,
            expected_old_commit_sha,
        )
    }

    fn push_integration_branch_to_delivery_target(
        &self,
        repo_root: &str,
        _push_remote: &str,
        credential_redacted_push_url: &str,
        branch_name: &str,
        expected_old_commit_sha: &str,
    ) -> Result<()> {
        /*
        integration branch는 이미 distributor worktree에서 합성된 결과다.
        upstream setup 없이 push하는 이유는 최종 integration이 계속 explicit branch/PR record를 통해 진행되어야 하기 때문이다.
        slot branch처럼 operator의 일상 작업 branch로 취급하지 않는다.
        */
        if !matches!(expected_old_commit_sha.len(), 40 | 64)
            || !expected_old_commit_sha
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("integration push expected-old OID is invalid");
        }
        run_git(repo_root, &["check-ref-format", "--branch", branch_name])?;
        let local_result_commit_sha = run_git_stdout(repo_root, &["rev-parse", "HEAD^{commit}"])?;
        if run_git(
            repo_root,
            &[
                "merge-base",
                "--is-ancestor",
                expected_old_commit_sha,
                local_result_commit_sha.as_str(),
            ],
        )
        .is_err()
        {
            bail!("local integration result is not a descendant of the frozen remote base");
        }
        let target_ref = format!("refs/heads/{branch_name}");
        let force_with_lease = format!("--force-with-lease={target_ref}:{expected_old_commit_sha}");
        let target_refspec = format!("{local_result_commit_sha}:{target_ref}");
        run_git_to_delivery_target(
            repo_root,
            credential_redacted_push_url,
            &[
                "push",
                force_with_lease.as_str(),
                FROZEN_GITHUB_REMOTE_NAME,
                target_refspec.as_str(),
            ],
        )
    }

    fn close_pull_request(&self, repo_root: &str, pr_number: u64) -> Result<()> {
        let push_remote = configured_push_remote_name(repo_root)?;
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, &push_remote)?;
        self.close_pull_request_for_delivery_target(repo_root, &push_remote, &push_url, pr_number)
    }

    fn close_pull_request_for_remote(
        &self,
        repo_root: &str,
        push_remote: &str,
        pr_number: u64,
    ) -> Result<()> {
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, push_remote)?;
        self.close_pull_request_for_delivery_target(repo_root, push_remote, &push_url, pr_number)
    }

    fn close_pull_request_for_delivery_target(
        &self,
        repo_root: &str,
        _push_remote: &str,
        credential_redacted_push_url: &str,
        pr_number: u64,
    ) -> Result<()> {
        /*
        close는 raw `gh` 대신 Akra wrapper에 위임한다.
        PR 생성/조회와 같은 script를 쓰면 write identity, token selection, repo-specific GitHub policy가 한 경계에 머문다.
        */
        run_github_script_command_for_delivery_target(
            repo_root,
            credential_redacted_push_url,
            &["pr", "close", &pr_number.to_string()],
        )?;
        Ok(())
    }

    fn remote_branch_head(
        &self,
        repo_root: &str,
        push_remote: &str,
        branch_name: &str,
    ) -> Result<Option<String>> {
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, push_remote)?;
        self.remote_branch_head_for_delivery_target(repo_root, push_remote, &push_url, branch_name)
    }

    fn remote_branch_head_for_delivery_target(
        &self,
        repo_root: &str,
        _push_remote: &str,
        credential_redacted_push_url: &str,
        branch_name: &str,
    ) -> Result<Option<String>> {
        let remote_ref = format!("refs/heads/{branch_name}");
        let output = run_git_network_command_stdout(
            repo_root,
            credential_redacted_push_url,
            &[
                "ls-remote",
                "--heads",
                FROZEN_GITHUB_REMOTE_NAME,
                remote_ref.as_str(),
            ],
        );
        match output {
            Ok(output) => Ok(output.split_whitespace().next().map(str::to_string)),
            Err(error) if error.to_string().contains("command exited without output") => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn remote_branch_names_for_prefix_for_delivery_target(
        &self,
        repo_root: &str,
        _push_remote: &str,
        credential_redacted_push_url: &str,
        branch_prefix: &str,
    ) -> Result<Vec<String>> {
        if !branch_prefix.ends_with('/') {
            bail!("frozen remote branch prefix must end with `/`");
        }
        let probe_branch = format!("{branch_prefix}akra-prefix-probe");
        run_git(
            repo_root,
            &["check-ref-format", "--branch", probe_branch.as_str()],
        )?;
        let remote_pattern = format!("refs/heads/{branch_prefix}*");
        let output = run_git_network_command(
            repo_root,
            credential_redacted_push_url,
            &[
                "ls-remote",
                "--heads",
                FROZEN_GITHUB_REMOTE_NAME,
                remote_pattern.as_str(),
            ],
        )?;
        if !output.status.success() {
            bail!(
                "git ls-remote failed for frozen GitHub target: {}",
                command_error_detail(&output)
            );
        }
        let stdout = String::from_utf8(output.stdout)
            .context("frozen remote branch listing was not valid UTF-8")?;
        parse_frozen_remote_branch_names(&stdout, branch_prefix)
    }

    fn fetch_branch_to_tracking_ref_for_delivery_target(
        &self,
        repo_root: &str,
        _push_remote: &str,
        credential_redacted_push_url: &str,
        branch_name: &str,
        tracking_ref: &str,
    ) -> Result<String> {
        fetch_branch_to_tracking_ref_isolated(
            repo_root,
            credential_redacted_push_url,
            branch_name,
            tracking_ref,
        )
    }

    fn delete_branch_if_unchanged(
        &self,
        repo_root: &str,
        push_remote: &str,
        branch_name: &str,
        expected_sha: &str,
    ) -> Result<bool> {
        let push_url = self.credential_redacted_push_url_for_remote(repo_root, push_remote)?;
        self.delete_branch_if_unchanged_for_delivery_target(
            repo_root,
            push_remote,
            &push_url,
            branch_name,
            expected_sha,
        )
    }

    fn delete_branch_if_unchanged_for_delivery_target(
        &self,
        repo_root: &str,
        push_remote: &str,
        credential_redacted_push_url: &str,
        branch_name: &str,
        expected_sha: &str,
    ) -> Result<bool> {
        let Some(remote_head) = self.remote_branch_head_for_delivery_target(
            repo_root,
            push_remote,
            credential_redacted_push_url,
            branch_name,
        )?
        else {
            return Ok(true);
        };
        if remote_head != expected_sha {
            return Ok(false);
        }
        let lease = format!("--force-with-lease=refs/heads/{branch_name}:{expected_sha}");
        let delete_refspec = format!(":refs/heads/{branch_name}");
        run_git_to_delivery_target(
            repo_root,
            credential_redacted_push_url,
            &[
                "push",
                lease.as_str(),
                FROZEN_GITHUB_REMOTE_NAME,
                delete_refspec.as_str(),
            ],
        )?;
        Ok(true)
    }
}

fn parse_frozen_remote_branch_names(output: &str, branch_prefix: &str) -> Result<Vec<String>> {
    let expected_ref_prefix = format!("refs/heads/{branch_prefix}");
    let mut branch_names = BTreeSet::new();
    for line in output.lines() {
        let (object_id, reference) = line
            .split_once('\t')
            .ok_or_else(|| anyhow!("frozen remote branch listing contained a malformed row"))?;
        if !matches!(object_id.len(), 40 | 64)
            || !object_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("frozen remote branch listing contained an invalid object ID");
        }
        let branch_name = reference.strip_prefix("refs/heads/").ok_or_else(|| {
            anyhow!("frozen remote branch listing contained a non-branch reference")
        })?;
        if !reference.starts_with(&expected_ref_prefix) {
            bail!("frozen remote branch listing escaped the requested prefix");
        }
        branch_names.insert(branch_name.to_string());
        if branch_names.len() > MAX_FROZEN_REMOTE_BRANCHES_PER_PREFIX {
            bail!(
                "frozen remote branch listing exceeded the {}-branch limit",
                MAX_FROZEN_REMOTE_BRANCHES_PER_PREFIX
            );
        }
    }
    Ok(branch_names.into_iter().collect())
}

struct TrustedGithubExecutables {
    bash: PathBuf,
    git: PathBuf,
    gh: Option<PathBuf>,
    path: OsString,
}

impl TrustedGithubExecutables {
    fn resolve(repo_root: &str) -> Result<Self> {
        let directories = trusted_executable_directories(repo_root)?;
        for tool in REQUIRED_GITHUB_HELPER_TOOLS {
            resolve_trusted_executable(&directories, tool, repo_root).with_context(|| {
                format!("trusted GitHub helper dependency `{tool}` is unavailable")
            })?;
        }
        let bash = resolve_trusted_executable(&directories, "bash", repo_root)
            .context("trusted bash interpreter is unavailable")?;
        let git = resolve_trusted_executable(&directories, "git", repo_root)
            .context("trusted git executable is unavailable")?;
        let gh = resolve_trusted_executable(&directories, "gh", repo_root).ok();
        let path = std::env::join_paths(&directories)
            .context("trusted GitHub executable PATH could not be constructed")?;
        Ok(Self {
            bash,
            git,
            gh,
            path,
        })
    }

    fn configure_minimal_environment(&self, command: &mut Command) {
        command
            .env("PATH", &self.path)
            .env_remove("BASH_ENV")
            .env_remove("ENV")
            .env_remove("CDPATH")
            .env_remove("GLOBIGNORE")
            .env_remove("SHELLOPTS")
            .env_remove("PROMPT_COMMAND");
    }
}

fn trusted_executable_directories(repo_root: &str) -> Result<Vec<PathBuf>> {
    #[cfg(unix)]
    let candidates = [
        "/usr/bin",
        "/bin",
        "/usr/local/bin",
        "/opt/homebrew/bin",
        "/opt/local/bin",
        "/run/current-system/sw/bin",
    ];
    #[cfg(windows)]
    let candidates = [
        r"C:\Program Files\Git\bin",
        r"C:\Program Files\Git\usr\bin",
        r"C:\Windows\System32",
    ];

    let mut candidates_present = Vec::new();
    for candidate in candidates {
        let candidate = Path::new(candidate);
        if candidate.is_dir() {
            candidates_present.push(candidate.to_path_buf());
        }
    }
    if candidates_present.is_empty() {
        bail!("no trusted system executable directory is available")
    }
    let candidate_path = std::env::join_paths(candidates_present)
        .context("trusted system executable candidates could not be joined")?;
    let trusted_path =
        crate::trusted_executable::sanitized_path(&candidate_path, Path::new(repo_root))?;
    Ok(std::env::split_paths(&trusted_path).collect())
}

fn untrusted_executable_roots(repo_root: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for path in [
        PathBuf::from(repo_root),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    ] {
        let canonical = fs::canonicalize(&path).unwrap_or(path);
        if !roots.contains(&canonical) {
            roots.push(canonical.clone());
        }
        if let Some(parent) = canonical.parent()
            && !roots.iter().any(|root| root == parent)
        {
            roots.push(parent.to_path_buf());
        }
    }
    roots
}

fn resolve_trusted_executable(
    directories: &[PathBuf],
    name: &str,
    repo_root: &str,
) -> Result<PathBuf> {
    #[cfg(windows)]
    let names = [name.to_string(), format!("{name}.exe")];
    #[cfg(not(windows))]
    let names = [name.to_string()];

    for directory in directories {
        for name in &names {
            let candidate = directory.join(name);
            let Ok(canonical) =
                crate::trusted_executable::validate_absolute(&candidate, Path::new(repo_root))
            else {
                continue;
            };
            if crate::trusted_executable::validate_native_executable(&canonical).is_err() {
                continue;
            }
            return Ok(canonical);
        }
    }
    bail!("trusted executable `{name}` was not found")
}

fn github_helper_source() -> &'static [u8] {
    #[cfg(test)]
    if let Some(source) = TEST_GITHUB_HELPER_SOURCE.with(|source| *source.borrow()) {
        return source;
    }
    EMBEDDED_GITHUB_HELPER
}

#[cfg(test)]
fn test_github_log_directory() -> PathBuf {
    std::env::temp_dir().join(format!("codex-exec-loop-fake-gh-{}", std::process::id()))
}

fn configured_push_remote_name(repo_root: &str) -> Result<String> {
    let env_value = std::env::var(AKRA_GITHUB_PUSH_REMOTE_ENV_VAR).ok();
    let config_value = run_git_stdout(
        repo_root,
        &["config", "--get", AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY],
    )
    .ok();
    resolve_github_push_remote_name_strict(env_value.as_deref(), config_value.as_deref())
        .map_err(|detail| anyhow!(detail))
}

fn run_github_script_command_for_delivery_target(
    repo_root: &str,
    credential_redacted_push_url: &str,
    args: &[&str],
) -> Result<String> {
    let command_label = format!("embedded gh-akra {}", args.join(" "));
    run_github_script_command_with_label_for_delivery_target(
        repo_root,
        credential_redacted_push_url,
        args,
        &command_label,
    )
}

fn run_github_script_command_with_label_for_delivery_target(
    repo_root: &str,
    credential_redacted_push_url: &str,
    args: &[&str],
    command_label: &str,
) -> Result<String> {
    let network_context =
        IsolatedGithubNetworkContext::new(repo_root, credential_redacted_push_url)?;
    run_github_script_command_in_network_context(&network_context, args, command_label)
}

fn run_github_script_command_in_network_context(
    network_context: &IsolatedGithubNetworkContext,
    args: &[&str],
    command_label: &str,
) -> Result<String> {
    let mut command = Command::new(&network_context.executables.bash);
    command.env_clear();
    network_context.configure_command(&mut command);
    network_context
        .executables
        .configure_minimal_environment(&mut command);
    #[cfg(test)]
    command.env("AKRA_TEST_GITHUB_LOG_DIR", test_github_log_directory());
    command
        .current_dir(&network_context.repo_root)
        .args(["--noprofile", "--norc", "-s", "--"])
        .args(args)
        .env(AKRA_GITHUB_PUSH_REMOTE_ENV_VAR, FROZEN_GITHUB_REMOTE_NAME);
    let output =
        subprocess::command_output_with_input(&mut command, command_label, github_helper_source())
            .with_context(|| format!("failed to run `{command_label}` for frozen GitHub target"))?;
    if !output.status.success() {
        bail!(
            "{command_label} failed for frozen GitHub target: {}",
            command_error_detail(&output)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

struct IsolatedGithubNetworkContext {
    root: PathBuf,
    repo_root: PathBuf,
    home: PathBuf,
    temp: PathBuf,
    common_objects: PathBuf,
    credentials: GithubNetworkCredentials,
    executables: TrustedGithubExecutables,
    network_environment: TrustedGithubNetworkEnvironment,
}

impl IsolatedGithubNetworkContext {
    fn new(source_repo_root: &str, credential_redacted_push_url: &str) -> Result<Self> {
        validate_credential_redacted_push_url(credential_redacted_push_url)?;
        let executables = TrustedGithubExecutables::resolve(source_repo_root)?;
        let network_environment = TrustedGithubNetworkEnvironment::resolve(source_repo_root)?;
        let credentials = GithubNetworkCredentials::resolve(source_repo_root, &executables);
        let root = create_private_temp_directory("akra-github-network")?;
        let repo_root = root.join("repo");
        let git_dir = repo_root.join(".git");
        let home = root.join("home");
        let temp = root.join("tmp");
        let objects = git_dir.join("objects");
        let objects_info = objects.join("info");
        let refs = git_dir.join("refs");
        let refs_heads = refs.join("heads");
        fs::create_dir_all(&objects_info)?;
        fs::create_dir_all(&refs_heads)?;
        fs::create_dir_all(&home)?;
        fs::create_dir_all(&temp)?;
        for directory in [
            &repo_root,
            &git_dir,
            &objects,
            &objects_info,
            &refs,
            &refs_heads,
            &home,
            &temp,
        ] {
            secure_private_directory(directory)?;
        }

        let common_objects = validated_source_common_objects(source_repo_root)?;
        let common_objects_label = common_objects.to_string_lossy();
        if common_objects_label.chars().any(char::is_control) {
            bail!("source repository object directory is not safe for an isolated Git context");
        }

        let push_url = git_config_value(credential_redacted_push_url)?;
        let config = format!(
            "[core]\n\trepositoryformatversion = 0\n\tfilemode = false\n\tbare = false\n\tlogallrefupdates = false\n[http]\n\tfollowRedirects = false\n[remote \"{FROZEN_GITHUB_REMOTE_NAME}\"]\n\turl = \"{push_url}\"\n[credential]\n\thelper = \"!sh -c 'test \\\"$1\\\" = get || exit 0; printf \\\"username=%s\\\\npassword=%s\\\\n\\\" \\\"${{AKRA_FROZEN_GITHUB_LOGIN:-x-access-token}}\\\" \\\"${{AKRA_FROZEN_GITHUB_TOKEN:-}}\\\"' -\"\n\tuseHttpPath = true\n"
        );
        write_private_file(&git_dir.join("config"), &config)?;
        write_private_file(
            &git_dir.join("HEAD"),
            "ref: refs/heads/akra-frozen-network\n",
        )?;
        write_private_file(
            &git_dir.join("objects/info/alternates"),
            &format!("{common_objects_label}\n"),
        )?;

        Ok(Self {
            root,
            repo_root,
            home,
            temp,
            common_objects,
            credentials,
            executables,
            network_environment,
        })
    }

    fn configure_command(&self, command: &mut Command) {
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", &self.home)
            .env("TMPDIR", &self.temp)
            .env("TMP", &self.temp)
            .env("TEMP", &self.temp)
            .env("GIT_OBJECT_DIRECTORY", &self.common_objects)
            .env_remove("AKRA_GITHUB_LOGIN")
            .env_remove("AKRA_GITHUB_TOKEN")
            .env_remove("GH_REPO")
            .env_remove("GH_HOST")
            .env_remove("GH_CONFIG_DIR")
            .env_remove("GH_TOKEN")
            .env_remove("GH_ENTERPRISE_TOKEN")
            .env_remove("GITHUB_TOKEN")
            .env_remove("GITHUB_ENTERPRISE_TOKEN")
            .env_remove(FROZEN_GITHUB_TOKEN_ENV_VAR)
            .env_remove(FROZEN_GITHUB_LOGIN_ENV_VAR)
            .env("GH_HOST", "github.com");
        if let Some(token) = self.credentials.token.as_ref() {
            command
                .env("AKRA_GITHUB_TOKEN", token)
                .env(FROZEN_GITHUB_TOKEN_ENV_VAR, token);
        }
        if let Some(login) = self.credentials.login.as_ref() {
            command
                .env("AKRA_GITHUB_LOGIN", login)
                .env(FROZEN_GITHUB_LOGIN_ENV_VAR, login);
        }
        self.network_environment.configure_command(command);
    }
}

impl Drop for IsolatedGithubNetworkContext {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct GithubNetworkCredentials {
    token: Option<OsString>,
    login: Option<OsString>,
}

#[derive(Debug, Default)]
struct TrustedGithubNetworkEnvironment {
    variables: Vec<(&'static str, OsString)>,
}

impl TrustedGithubNetworkEnvironment {
    fn resolve(repo_root: &str) -> Result<Self> {
        let mut variables = Vec::new();
        if let Some(proxy) = resolve_environment_alias("HTTPS_PROXY", "https_proxy")? {
            validate_https_proxy(&proxy)?;
            variables.push(("HTTPS_PROXY", proxy));
        }
        if let Some(no_proxy) = resolve_environment_alias("NO_PROXY", "no_proxy")? {
            validate_no_proxy(&no_proxy)?;
            variables.push(("NO_PROXY", no_proxy));
        }

        let untrusted_roots = untrusted_executable_roots(repo_root);
        for (name, directory) in [
            ("SSL_CERT_FILE", false),
            ("CURL_CA_BUNDLE", false),
            ("GIT_SSL_CAINFO", false),
            ("SSL_CERT_DIR", true),
            ("GIT_SSL_CAPATH", true),
        ] {
            let Some(value) = std::env::var_os(name).filter(|value| !value.is_empty()) else {
                continue;
            };
            let canonical = validate_trusted_ca_path(name, &value, directory, &untrusted_roots)?;
            let forwarded_name = match name {
                "GIT_SSL_CAINFO" => "AKRA_TRUSTED_GIT_SSL_CAINFO",
                "GIT_SSL_CAPATH" => "AKRA_TRUSTED_GIT_SSL_CAPATH",
                other => other,
            };
            variables.push((forwarded_name, canonical.into_os_string()));
        }
        Ok(Self { variables })
    }

    fn configure_command(&self, command: &mut Command) {
        for (name, value) in &self.variables {
            command.env(name, value);
        }
    }
}

fn resolve_environment_alias(upper: &'static str, lower: &'static str) -> Result<Option<OsString>> {
    let upper_value = std::env::var_os(upper).filter(|value| !value.is_empty());
    let lower_value = std::env::var_os(lower).filter(|value| !value.is_empty());
    match (upper_value, lower_value) {
        (Some(upper_value), Some(lower_value)) if upper_value != lower_value => {
            bail!("conflicting `{upper}` and `{lower}` values are not safe to inherit")
        }
        (Some(value), _) | (_, Some(value)) => Ok(Some(value)),
        (None, None) => Ok(None),
    }
}

fn validate_https_proxy(value: &OsString) -> Result<()> {
    let value = value.to_str().context("HTTPS proxy must be valid UTF-8")?;
    if value.chars().any(char::is_control) || value.chars().any(char::is_whitespace) {
        bail!("HTTPS proxy contains whitespace or a control character")
    }
    let remainder = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .context("HTTPS proxy must be an absolute HTTP(S) URL")?;
    let authority = remainder.split('/').next().unwrap_or_default();
    if authority.is_empty()
        || authority.contains('@')
        || authority.contains('?')
        || authority.contains('#')
        || !authority.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '.' | '-' | '_' | ':' | '[' | ']')
        })
    {
        bail!("HTTPS proxy authority is invalid or contains credentials")
    }
    if remainder
        .strip_prefix(authority)
        .is_some_and(|suffix| !matches!(suffix, "" | "/"))
    {
        bail!("HTTPS proxy URL must not contain a path, query, or fragment")
    }
    Ok(())
}

fn validate_no_proxy(value: &OsString) -> Result<()> {
    let value = value.to_str().context("NO_PROXY must be valid UTF-8")?;
    if value.is_empty()
        || value.chars().any(char::is_control)
        || value.chars().any(char::is_whitespace)
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '.' | '-' | '_' | ':' | ',' | '*' | '[' | ']')
        })
    {
        bail!("NO_PROXY contains an unsupported character")
    }
    Ok(())
}

fn validate_trusted_ca_path(
    name: &str,
    value: &OsString,
    directory: bool,
    untrusted_roots: &[PathBuf],
) -> Result<PathBuf> {
    let path = Path::new(value);
    if !path.is_absolute() {
        bail!("{name} must be an absolute path")
    }
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("failed to resolve trusted CA path `{}`", path.display()))?;
    if untrusted_roots
        .iter()
        .any(|root| canonical == *root || canonical.starts_with(root))
    {
        bail!("{name} must not reference repository- or pool-controlled data")
    }
    let metadata = fs::metadata(&canonical).with_context(|| {
        format!(
            "failed to inspect trusted CA path `{}`",
            canonical.display()
        )
    })?;
    if metadata.is_dir() != directory || metadata.is_file() == directory {
        bail!("{name} has the wrong filesystem type")
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let owner = metadata.uid();
        // SAFETY: geteuid has no preconditions and does not retain pointers.
        let current = unsafe { libc::geteuid() };
        if (!matches!(owner, 0) && owner != current) || metadata.mode() & 0o022 != 0 {
            bail!("{name} ownership or permissions are unsafe")
        }
    }
    Ok(canonical)
}

impl GithubNetworkCredentials {
    fn resolve(repo_root: &str, executables: &TrustedGithubExecutables) -> Self {
        let token = ["AKRA_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"]
            .into_iter()
            .find_map(|name| safe_secret(std::env::var_os(name)))
            .or_else(|| github_cli_auth_token(executables));
        let login = safe_login(std::env::var_os("AKRA_GITHUB_LOGIN")).or_else(|| {
            run_git_stdout(repo_root, &["config", "--get", "akra.githubLogin"])
                .ok()
                .and_then(|value| safe_login(Some(value.into())))
        });
        Self { token, login }
    }
}

fn github_cli_auth_token(executables: &TrustedGithubExecutables) -> Option<OsString> {
    let gh = executables.gh.as_ref()?;
    let mut command = Command::new(gh);
    command.env_clear();
    executables.configure_minimal_environment(&mut command);
    for name in ["HOME", "USERPROFILE", "XDG_CONFIG_HOME"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .args(["auth", "token", "--hostname", "github.com"])
        .stdin(Stdio::null())
        .env("GH_HOST", "github.com");
    let output = subprocess::command_output(&mut command, "gh auth token").ok()?;
    if !output.status.success() {
        return None;
    }
    safe_secret(Some(
        String::from_utf8(output.stdout)
            .ok()?
            .trim()
            .to_string()
            .into(),
    ))
}

fn safe_secret(value: Option<OsString>) -> Option<OsString> {
    let value = value?;
    let text = value.to_str()?;
    (!text.is_empty() && !text.chars().any(char::is_whitespace)).then_some(value)
}

fn safe_login(value: Option<OsString>) -> Option<OsString> {
    let value = value?;
    let text = value.to_str()?;
    (!text.is_empty()
        && text.len() <= 128
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '[' | ']')))
    .then_some(value)
}

fn validate_credential_redacted_push_url(value: &str) -> Result<()> {
    #[cfg(test)]
    if is_test_local_push_url(value) {
        return Ok(());
    }
    if !value.starts_with("https://github.com/")
        || credential_redacted_canonical_github_push_url(value).as_deref() != Some(value)
    {
        bail!("frozen push URL is not a credential-redacted canonical GitHub URL");
    }
    Ok(())
}

#[cfg(test)]
fn is_test_local_push_url(value: &str) -> bool {
    !value.chars().any(char::is_control) && Path::new(value).is_absolute()
}

fn validated_source_common_objects(repo_root: &str) -> Result<PathBuf> {
    let common_git_dir = PathBuf::from(run_git_stdout(
        repo_root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?);
    if !common_git_dir.is_absolute() {
        bail!("source repository common Git directory is not absolute");
    }
    let common_metadata = fs::symlink_metadata(&common_git_dir)
        .context("failed to inspect source repository common Git directory")?;
    if !common_metadata.is_dir() || common_metadata.file_type().is_symlink() {
        bail!("source repository common Git directory must be a real directory");
    }
    let canonical_common_git_dir = fs::canonicalize(&common_git_dir)
        .context("failed to resolve source repository common Git directory")?;
    let objects_path = canonical_common_git_dir.join("objects");
    let objects_metadata = fs::symlink_metadata(&objects_path)
        .context("failed to inspect source repository object directory")?;
    if !objects_metadata.is_dir() || objects_metadata.file_type().is_symlink() {
        bail!("source repository object directory must be a real directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        // SAFETY: geteuid has no preconditions and does not retain pointers.
        if objects_metadata.uid() != unsafe { libc::geteuid() } {
            bail!("source repository object directory must be owned by the current user");
        }
    }
    #[cfg(windows)]
    validate_owned_windows_directory(&objects_path)?;
    let canonical_objects = fs::canonicalize(&objects_path)
        .context("failed to resolve source repository object directory")?;
    if canonical_objects.parent() != Some(canonical_common_git_dir.as_path()) {
        bail!("source repository object directory escaped its common Git directory");
    }
    Ok(canonical_objects)
}

#[cfg(windows)]
fn validate_owned_windows_directory(path: &Path) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
        validate_windows_path_identity,
    };

    let directory = OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .with_context(|| {
            format!(
                "failed to inspect Windows object directory `{}`",
                path.display()
            )
        })?;
    validate_windows_path_identity(path, &directory, true)
}

fn create_private_temp_directory(prefix: &str) -> Result<PathBuf> {
    let base = trusted_temp_directory_base()?;
    for _ in 0..32_u32 {
        let mut random = [0_u8; 16];
        OsRng.fill_bytes(&mut random);
        let nonce = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path = base.join(format!("{prefix}-{nonce}"));
        #[allow(unused_mut)]
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => {
                secure_private_directory(&path)?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("failed to create private temporary root"),
        }
    }
    bail!("failed to allocate a unique private temporary root")
}

#[cfg(unix)]
fn trusted_temp_directory_base() -> Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;

    let base =
        fs::canonicalize("/tmp").context("trusted system temporary directory is unavailable")?;
    let metadata =
        fs::metadata(&base).context("failed to inspect trusted system temporary directory")?;
    if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o1000 == 0 {
        bail!("system temporary directory must be a root-owned sticky directory")
    }
    Ok(base)
}

#[cfg(windows)]
fn trusted_temp_directory_base() -> Result<PathBuf> {
    let local_app_data = crate::private_fs::windows_local_app_data_path()?;
    let akra = local_app_data.join("Akra");
    let base = akra.join("trusted-temp");
    for directory in [&akra, &base] {
        match fs::create_dir(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to create trusted temp base `{}`",
                        directory.display()
                    )
                });
            }
        }
        secure_private_directory(directory)?;
    }
    Ok(base)
}

fn write_private_file(path: &Path, contents: &str) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to create isolated Git file `{}`", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("failed to write isolated Git file `{}`", path.display()))?;
    file.flush()
        .with_context(|| format!("failed to flush isolated Git file `{}`", path.display()))?;
    secure_private_file(path)
}

#[cfg(unix)]
fn secure_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect private directory `{}`", path.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("private directory must be a real directory")
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure private directory `{}`", path.display()))?;
    validate_private_directory(path)
}

#[cfg(unix)]
fn secure_private_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect private file `{}`", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("private file must be a real regular file")
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure private file `{}`", path.display()))?;
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("failed to reopen private file `{}`", path.display()))?;
    validate_private_file_identity(path, &file)
}

#[cfg(unix)]
fn validate_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to validate private directory `{}`", path.display()))?;
    // SAFETY: geteuid has no preconditions and does not retain pointers.
    let current = unsafe { libc::geteuid() };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != current
        || metadata.mode() & 0o777 != 0o700
    {
        bail!("private directory ownership, type, or permissions are unsafe")
    }
    Ok(())
}

#[cfg(unix)]
fn validate_private_file_identity(path: &Path, file: &File) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let path_metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to validate private file `{}`", path.display()))?;
    let opened_metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect opened private file `{}`", path.display()))?;
    // SAFETY: geteuid has no preconditions and does not retain pointers.
    let current = unsafe { libc::geteuid() };
    if !path_metadata.is_file()
        || path_metadata.file_type().is_symlink()
        || path_metadata.uid() != current
        || opened_metadata.uid() != current
        || path_metadata.mode() & 0o777 != 0o600
        || opened_metadata.mode() & 0o777 != 0o600
        || path_metadata.nlink() != 1
        || opened_metadata.nlink() != 1
        || path_metadata.dev() != opened_metadata.dev()
        || path_metadata.ino() != opened_metadata.ino()
    {
        bail!("private file must be an unchanged owner-private single-link regular file")
    }
    Ok(())
}

#[cfg(windows)]
fn secure_private_directory(path: &Path) -> Result<()> {
    secure_private_windows_path(path, true)?;
    validate_private_directory(path)
}

#[cfg(windows)]
fn secure_private_file(path: &Path) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL};

    secure_private_windows_path(path, false)?;
    let file = OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .open(path)
        .with_context(|| format!("failed to validate private file `{}`", path.display()))?;
    validate_private_file_identity(path, &file)
}

#[cfg(windows)]
fn validate_private_directory(path: &Path) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
        validate_windows_path_identity, validate_windows_private_owner_and_acl,
    };
    let directory = OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .with_context(|| format!("failed to validate private directory `{}`", path.display()))?;
    validate_windows_path_identity(path, &directory, true)?;
    validate_windows_private_owner_and_acl(path, &directory)
}

#[cfg(windows)]
fn validate_private_file_identity(path: &Path, file: &File) -> Result<()> {
    use crate::private_fs::{
        validate_windows_path_identity, validate_windows_private_owner_and_acl,
        windows_file_link_count,
    };
    validate_windows_path_identity(path, file, false)?;
    validate_windows_private_owner_and_acl(path, file)?;
    if windows_file_link_count(file)? != 1 {
        bail!("private file must have exactly one link")
    }
    Ok(())
}

#[cfg(windows)]
fn secure_private_windows_path(path: &Path, directory: bool) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC,
        set_windows_private_acl, validate_windows_path_identity,
        validate_windows_private_owner_and_acl,
    };

    let file = OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL | WINDOWS_WRITE_DAC)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(
            WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    WINDOWS_FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
        )
        .open(path)
        .with_context(|| format!("failed to secure private Windows path `{}`", path.display()))?;
    validate_windows_path_identity(path, &file, directory)?;
    set_windows_private_acl(&file, directory)?;
    validate_windows_private_owner_and_acl(path, &file)?;
    validate_windows_path_identity(path, &file, directory)
}

fn git_config_value(value: &str) -> Result<String> {
    if value.chars().any(char::is_control) {
        bail!("isolated Git config value contains a control character");
    }
    Ok(value.replace('\\', "\\\\").replace('"', "\\\""))
}

struct TemporaryTextFile {
    path: PathBuf,
}

impl TemporaryTextFile {
    fn new(prefix: &str, contents: &str) -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "codex-exec-loop-{prefix}-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).with_context(|| {
            format!(
                "failed to create temporary GitHub automation file `{}`",
                path.display()
            )
        })?;
        file.write_all(contents.as_bytes()).with_context(|| {
            format!(
                "failed to write temporary GitHub automation file `{}`",
                path.display()
            )
        })?;
        Ok(Self { path })
    }

    fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Drop for TemporaryTextFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn pull_request_create_command_label(
    base_branch: &str,
    head_branch: &str,
    title: &str,
    body: &str,
) -> String {
    format!(
        "embedded gh-akra pr create --base {base_branch} --head {head_branch} --title-file {} --body-file {}",
        redacted_argument_label(title),
        redacted_argument_label(body)
    )
}

fn redacted_argument_label(value: &str) -> String {
    format!("[redacted:{} chars]", value.chars().count())
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
    #[serde(default, rename = "headRefOid")]
    head_ref_oid: Option<String>,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    #[serde(default, rename = "reviewDecision")]
    review_decision: Option<String>,
    #[serde(default, rename = "mergeStateStatus")]
    merge_state_status: Option<String>,
    #[serde(default, rename = "statusCheckRollup")]
    status_check_rollup: Option<Vec<GithubStatusCheckJson>>,
    #[serde(default, rename = "approvedReviewCommitOids")]
    approved_review_commit_oids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GithubStatusCheckJson {
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

impl From<GithubPullRequestJson> for GithubAutomationPullRequest {
    fn from(value: GithubPullRequestJson) -> Self {
        /*
        이 conversion은 GitHub camelCase JSON과 application port의 provider-neutral record 사이 membrane이다.
        mapping을 여기 고정하면 distributor나 readiness code가 `baseRefName` 같은 provider field에 직접 의존하지 않는다.
        */
        let required_checks_passed = value.status_check_rollup.as_ref().map(|checks| {
            !checks.is_empty()
                && checks.iter().all(|check| {
                    check
                        .conclusion
                        .as_deref()
                        .or(check.state.as_deref())
                        .is_some_and(|state| {
                            matches!(
                                state.to_ascii_uppercase().as_str(),
                                "SUCCESS" | "NEUTRAL" | "SKIPPED"
                            )
                        })
                })
        });
        let mut pull_request = GithubAutomationPullRequest::new(
            value.number,
            value.url,
            value.state,
            value.base_ref_name,
            value.head_ref_name,
            value.is_draft,
        );
        pull_request.review_decision = value.review_decision;
        pull_request.merge_state_status = value.merge_state_status;
        pull_request.required_checks_passed = required_checks_passed;
        pull_request.head_commit_sha = value.head_ref_oid;
        pull_request.approved_review_commit_shas = value.approved_review_commit_oids;
        pull_request
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

fn run_git_to_delivery_target(
    repo_root: &str,
    credential_redacted_push_url: &str,
    args: &[&str],
) -> Result<()> {
    let network_context =
        IsolatedGithubNetworkContext::new(repo_root, credential_redacted_push_url)?;
    #[cfg(test)]
    let allow_test_local = is_test_local_push_url(credential_redacted_push_url);
    #[cfg(not(test))]
    let allow_test_local = false;
    verify_github_write_identity_in_network_context(&network_context, allow_test_local)?;
    let output = run_git_command_in_network_context(&network_context, args)?;
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "git {} failed for frozen GitHub target: {}",
        args.join(" "),
        command_error_detail(&output)
    )
}

fn run_git_network_command_stdout(
    repo_root: &str,
    credential_redacted_push_url: &str,
    args: &[&str],
) -> Result<String> {
    let output = run_git_network_command(repo_root, credential_redacted_push_url, args)?;
    if !output.status.success() {
        bail!(
            "git {} failed for frozen GitHub target: {}",
            args.join(" "),
            command_error_detail(&output)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_git_network_command(
    repo_root: &str,
    credential_redacted_push_url: &str,
    args: &[&str],
) -> Result<Output> {
    let network_context =
        IsolatedGithubNetworkContext::new(repo_root, credential_redacted_push_url)?;
    run_git_command_in_network_context(&network_context, args)
}

fn run_git_command_in_network_context(
    network_context: &IsolatedGithubNetworkContext,
    args: &[&str],
) -> Result<Output> {
    let mut command = git_subprocess::command_with_program(
        network_context.executables.git.as_os_str(),
        args.iter().copied(),
    );
    command.env_clear();
    network_context.configure_command(&mut command);
    network_context
        .executables
        .configure_minimal_environment(&mut command);
    configure_noninteractive_git_environment(&mut command);
    command.current_dir(&network_context.repo_root);
    let command_label = format!("git {}", args.join(" "));
    subprocess::command_output(&mut command, &command_label)
        .with_context(|| format!("failed to run `{command_label}` for frozen GitHub target"))
}

fn fetch_branch_to_tracking_ref_isolated(
    repo_root: &str,
    credential_redacted_push_url: &str,
    branch_name: &str,
    tracking_ref: &str,
) -> Result<String> {
    run_git(repo_root, &["check-ref-format", "--branch", branch_name])?;
    run_git(repo_root, &["check-ref-format", tracking_ref])?;
    let network_context =
        IsolatedGithubNetworkContext::new(repo_root, credential_redacted_push_url)?;
    let fetched_ref = "refs/heads/akra-frozen-fetch-result";
    let fetch_refspec = format!("+refs/heads/{branch_name}:{fetched_ref}");
    let output = run_git_command_in_network_context(
        &network_context,
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            FROZEN_GITHUB_REMOTE_NAME,
            &fetch_refspec,
        ],
    )?;
    if !output.status.success() {
        bail!(
            "git fetch failed for frozen GitHub target: {}",
            command_error_detail(&output)
        );
    }
    let commit_ref = format!("{fetched_ref}^{{commit}}");
    let output = run_git_command_in_network_context(
        &network_context,
        &["rev-parse", "--verify", &commit_ref],
    )?;
    if !output.status.success() {
        bail!("frozen GitHub target fetch did not produce a commit");
    }
    let commit_sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !matches!(commit_sha.len(), 40 | 64)
        || !commit_sha.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("frozen GitHub target fetch produced an invalid commit OID");
    }

    let expected_old = run_git_stdout(repo_root, &["rev-parse", "--verify", tracking_ref])
        .unwrap_or_else(|_| "0".repeat(commit_sha.len()));
    run_git(
        repo_root,
        &["update-ref", tracking_ref, &commit_sha, &expected_old],
    )?;
    Ok(commit_sha)
}

fn verify_github_write_identity_for_delivery_target(
    repo_root: &str,
    credential_redacted_push_url: &str,
) -> Result<()> {
    #[cfg(test)]
    if is_test_local_push_url(credential_redacted_push_url) {
        return Ok(());
    }
    let network_context =
        IsolatedGithubNetworkContext::new(repo_root, credential_redacted_push_url)?;
    verify_github_write_identity_in_network_context(&network_context, false)
}

fn verify_github_write_identity_in_network_context(
    network_context: &IsolatedGithubNetworkContext,
    allow_test_local: bool,
) -> Result<()> {
    if allow_test_local {
        return Ok(());
    }
    run_github_script_command_in_network_context(
        network_context,
        &["auth", "write-status"],
        "embedded gh-akra auth write-status",
    )
    .map(|_| ())
    .context("GitHub write identity verification failed before frozen-target git push")
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
    let command_label = format!("{program} {}", args.join(" "));
    run_command_with_label(program, args, command_label.as_str(), repo_root)
}

fn run_command_with_label(
    program: &str,
    args: &[&str],
    command_label: &str,
    repo_root: &str,
) -> Result<String> {
    let output = run_process_with_label(program, args, command_label, repo_root)?;
    if !output.status.success() {
        bail!(
            "{command_label} failed in {repo_root}: {}",
            command_error_detail(&output)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_process(program: &str, args: &[&str], repo_root: &str) -> Result<Output> {
    let command_label = format!("{program} {}", args.join(" "));
    run_process_with_label(program, args, command_label.as_str(), repo_root)
}

fn run_process_with_label(
    program: &str,
    args: &[&str],
    command_label: &str,
    repo_root: &str,
) -> Result<Output> {
    /*
    background parallel-mode delivery에서는 non-interactive execution이 필수다.
    terminal prompt를 막아 credential/network gap이 supervisor lane을 멈춰 세우는 interactive wait가 아니라
    일반 command failure로 드러나게 한다.
    */
    let directories = trusted_executable_directories(repo_root)?;
    let executable = resolve_trusted_executable(&directories, program, repo_root)
        .with_context(|| format!("trusted `{program}` executable is unavailable"))?;
    let safe_path = std::env::join_paths(&directories)
        .context("trusted local Git executable PATH could not be constructed")?;
    let mut command = git_subprocess::command_for_program(
        executable.to_string_lossy().as_ref(),
        args.iter().copied(),
    );
    command
        .current_dir(repo_root)
        .stdin(Stdio::null())
        .env("PATH", safe_path)
        .env("GIT_TERMINAL_PROMPT", "0");
    remove_github_credentials(&mut command);
    subprocess::command_output(&mut command, command_label)
        .with_context(|| format!("failed to run `{command_label}` in {repo_root}"))
}

fn configure_noninteractive_git_environment(command: &mut Command) {
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_EDITOR", ":")
        .env("GIT_SEQUENCE_EDITOR", ":")
        .env("GCM_INTERACTIVE", "Never")
        .env("GCM_GUI_PROMPT", "0");
}

fn remove_github_credentials(command: &mut Command) {
    for name in [
        "AKRA_GITHUB_TOKEN",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
        FROZEN_GITHUB_TOKEN_ENV_VAR,
        FROZEN_GITHUB_LOGIN_ENV_VAR,
    ] {
        command.env_remove(name);
    }
}

fn command_error_detail(output: &Output) -> String {
    /*
    대부분의 Git/GitHub failure는 stderr에 설명을 남기지만 wrapper script는 stdout으로 error를 normalize할 수 있다.
    stderr, stdout, stable fallback 순서로 읽어 가장 signal이 높은 메시지를 보존하면서 silent exit도 일정한 문장으로 만든다.
    */
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return sanitize_command_output(&stderr);
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return sanitize_command_output(&stdout);
    }

    "command exited without output".to_string()
}

fn sanitize_command_output(value: &str) -> String {
    let mut sanitized = value.to_string();
    redact_url_userinfo(&mut sanitized);
    redact_bearer_tokens(&mut sanitized);
    for prefix in ["ghp_", "gho_", "ghs_", "ghu_", "ghr_", "github_pat_"] {
        redact_prefixed_token(&mut sanitized, prefix);
    }
    sanitized
}

fn redact_url_userinfo(value: &mut String) {
    let mut search_from = 0;
    while let Some(relative_scheme) = value[search_from..].find("://") {
        let authority_start = search_from + relative_scheme + 3;
        let authority_end = value[authority_start..]
            .find(|ch: char| ch == '/' || ch.is_whitespace())
            .map(|offset| authority_start + offset)
            .unwrap_or(value.len());
        if let Some(relative_at) = value[authority_start..authority_end].rfind('@') {
            let at = authority_start + relative_at;
            value.replace_range(authority_start..at, "[redacted]");
            search_from = authority_start + "[redacted]@".len();
        } else {
            search_from = authority_end;
        }
    }
}

fn redact_bearer_tokens(value: &mut String) {
    let mut search_from = 0;
    loop {
        let lower = value.to_ascii_lowercase();
        let Some(relative_match) = lower[search_from..].find("bearer ") else {
            break;
        };
        let token_start = search_from + relative_match + "bearer ".len();
        let token_end = value[token_start..]
            .find(char::is_whitespace)
            .map(|offset| token_start + offset)
            .unwrap_or(value.len());
        value.replace_range(token_start..token_end, "[redacted]");
        search_from = token_start + "[redacted]".len();
    }
}

fn redact_prefixed_token(value: &mut String, prefix: &str) {
    let mut search_from = 0;
    while let Some(relative_match) = value[search_from..].find(prefix) {
        let token_start = search_from + relative_match;
        let token_end = value[token_start..]
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .map(|offset| token_start + offset)
            .unwrap_or(value.len());
        value.replace_range(token_start..token_end, "[redacted-token]");
        search_from = token_start + "[redacted-token]".len();
    }
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
        EMBEDDED_GITHUB_HELPER, FROZEN_GITHUB_REMOTE_NAME, GithubAutomationAdapter,
        GithubPullRequestJson, TEST_GITHUB_HELPER_SOURCE, TemporaryTextFile,
        TrustedGithubNetworkEnvironment, fetch_branch_to_tracking_ref_isolated,
        parse_frozen_remote_branch_names, parse_pull_request_number_from_url, run_command, run_git,
        run_git_stdout, sanitize_command_output,
    };
    #[cfg(unix)]
    use super::{
        IsolatedGithubNetworkContext, github_helper_source, run_git_command_in_network_context,
        run_github_script_command_for_delivery_target, trusted_executable_directories,
    };

    use crate::application::port::outbound::github_automation_port::{
        AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY, AKRA_GITHUB_PUSH_REMOTE_ENV_VAR, GithubAutomationPort,
        GithubAutomationPullRequest,
    };
    #[cfg(unix)]
    use crate::domain::parallel_mode::ParallelModeCapabilitySnapshot;
    use crate::domain::parallel_mode::{ParallelModeCapabilityKey, ParallelModeCapabilityState};
    use crate::subprocess::SUBPROCESS_TIMEOUT_ENV;

    #[test]
    fn frozen_remote_branch_listing_is_bounded_to_the_requested_prefix() {
        let object_id = "a".repeat(40);
        let output = format!(
            "{object_id}\trefs/heads/akra-agent/slot-1/task-two\n{object_id}\trefs/heads/akra-agent/slot-1/task-one\n"
        );
        assert_eq!(
            parse_frozen_remote_branch_names(&output, "akra-agent/slot-1/")
                .expect("matching remote branches should parse"),
            vec![
                "akra-agent/slot-1/task-one".to_string(),
                "akra-agent/slot-1/task-two".to_string(),
            ]
        );
        assert!(
            parse_frozen_remote_branch_names(
                &format!("{object_id}\trefs/heads/akra-agent/slot-2/task-one\n"),
                "akra-agent/slot-1/",
            )
            .is_err()
        );
        assert!(parse_frozen_remote_branch_names("malformed", "akra-agent/slot-1/").is_err());
    }

    #[test]
    fn pull_request_json_maps_only_the_application_port_contract() {
        let pull_request: GithubAutomationPullRequest =
            serde_json::from_value::<GithubPullRequestJson>(json!({
            "number": 1681,
            "url": "https://github.com/RefinedStone/codex-exec-loop/pull/1681",
            "state": "OPEN",
            "baseRefName": "prerelease",
            "headRefName": "feature/test-coverage",
            "headRefOid": "0123456789abcdef",
            "isDraft": false,
            "reviewDecision": "APPROVED",
            "mergeStateStatus": "CLEAN",
            "statusCheckRollup": [{"conclusion": "SUCCESS"}],
            "approvedReviewCommitOids": ["0123456789abcdef"]
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
        assert_eq!(pull_request.review_decision.as_deref(), Some("APPROVED"));
        assert_eq!(pull_request.merge_state_status.as_deref(), Some("CLEAN"));
        assert_eq!(pull_request.required_checks_passed, Some(true));
        assert_eq!(
            pull_request.head_commit_sha.as_deref(),
            Some("0123456789abcdef")
        );
        assert_eq!(
            pull_request.approved_review_commit_shas,
            ["0123456789abcdef"]
        );
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
    fn empty_status_check_rollup_is_not_a_vacuous_success() {
        let pull_request: GithubAutomationPullRequest =
            serde_json::from_value::<GithubPullRequestJson>(json!({
                "number": 7,
                "url": "https://github.com/acme/repo/pull/7",
                "state": "OPEN",
                "baseRefName": "prerelease",
                "headRefName": "feature/test",
                "headRefOid": "abc123",
                "isDraft": false,
                "reviewDecision": "APPROVED",
                "mergeStateStatus": "CLEAN",
                "statusCheckRollup": []
            }))
            .expect("empty status rollup fixture should deserialize")
            .into();

        assert_eq!(pull_request.required_checks_passed, Some(false));
    }

    #[test]
    fn subprocess_diagnostics_redact_credentials_and_github_tokens() {
        let sanitized = sanitize_command_output(
            "fatal: https://user:secret@github.com/acme/repo.git Authorization: Bearer top-secret ghp_abcdefghijklmnopqrstuvwxyz github_pat_1234567890",
        );

        assert!(!sanitized.contains("user:secret"));
        assert!(!sanitized.contains("top-secret"));
        assert!(!sanitized.contains("ghp_"));
        assert!(!sanitized.contains("github_pat_"));
        assert!(sanitized.contains("https://[redacted]@github.com/acme/repo.git"));
        assert!(sanitized.contains("Bearer [redacted]"));
    }

    #[cfg(unix)]
    #[test]
    fn embedded_helper_and_trusted_path_ignore_repository_controlled_executables() {
        let _lock = github_script_lock()
            .lock()
            .expect("GitHub test environment lock should not be poisoned");
        let fixture = GitFixture::new("github-embedded-helper-boundary");
        let scripts = fixture.repo.join("scripts");
        let bin = fixture.repo.join("bin");
        fs::create_dir_all(&scripts).expect("repository script directory should be created");
        fs::create_dir_all(&bin).expect("repository bin directory should be created");
        let marker = fixture
            .repo
            .parent()
            .expect("fixture repo should have a parent")
            .join("host-token-exfiltration-marker");
        fs::write(
            scripts.join("gh-akra.sh"),
            format!(
                "#!/bin/sh\nprintf '%s' \"${{AKRA_GITHUB_TOKEN:-}}\" > '{}'\n",
                marker.display()
            ),
        )
        .expect("hostile repository helper should be written");
        for program in ["bash", "sh", "git", "curl", "gh", "python3"] {
            write_token_exfiltrator(&bin.join(program), &marker);
        }

        let trusted_directories = trusted_executable_directories(path_str(&fixture.repo))
            .expect("trusted helper directories should resolve");
        let canonical_bin = fs::canonicalize(&bin).expect("repository bin should canonicalize");
        assert!(
            trusted_directories
                .iter()
                .all(|directory| directory != &canonical_bin),
            "repository executable directory must stay outside the fixed helper search path"
        );

        assert_eq!(github_helper_source(), EMBEDDED_GITHUB_HELPER);
        assert!(!marker.exists());

        const TRUSTED_PROBE: &[u8] = br#"#!/usr/bin/env bash
set -euo pipefail
git --version >/dev/null
curl --version >/dev/null
python3 -c 'print("trusted")' >/dev/null
if command -v gh >/dev/null 2>&1; then gh --version >/dev/null; fi
printf '%s\n' 'trusted-helper-executed'
"#;
        let _helper_guard = TestGithubHelperGuard::install(TRUSTED_PROBE);
        let _token_guard = EnvVarGuard::set("AKRA_GITHUB_TOKEN", "host-secret-token");
        let _gh_token_guard = EnvVarGuard::remove("GH_TOKEN");
        let _github_token_guard = EnvVarGuard::remove("GITHUB_TOKEN");

        let output = run_github_script_command_for_delivery_target(
            path_str(&fixture.repo),
            path_str(&fixture.remote),
            &["probe"],
        )
        .expect("trusted helper runtime should ignore repository PATH entries");
        assert_eq!(output, "trusted-helper-executed");
        assert!(!output.contains("host-secret-token"));
        assert!(
            !marker.exists(),
            "repository-controlled helper or executable must never receive the token"
        );
    }

    #[test]
    fn embedded_helper_bytes_match_the_reviewed_digest() {
        use sha2::{Digest, Sha256};

        let digest = Sha256::digest(EMBEDDED_GITHUB_HELPER)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            digest,
            "5b8fcc4df4f52d9af7820373e0f70fc35ac7dd8391a1d76d03994aa24074fb13"
        );
        assert_eq!(EMBEDDED_GITHUB_HELPER.len(), 37_404);
    }

    #[test]
    fn trusted_network_environment_accepts_credential_free_proxy_and_rejects_unsafe_inputs() {
        let _lock = github_script_lock()
            .lock()
            .expect("GitHub test environment lock should not be poisoned");
        let fixture = GitFixture::new("github-network-environment");
        let _lower_proxy = EnvVarGuard::remove("https_proxy");
        let _upper_proxy = EnvVarGuard::set("HTTPS_PROXY", "https://proxy.example:8443/");
        let _lower_no_proxy = EnvVarGuard::remove("no_proxy");
        let _upper_no_proxy = EnvVarGuard::set("NO_PROXY", "localhost,127.0.0.1,.internal");
        let environment = TrustedGithubNetworkEnvironment::resolve(path_str(&fixture.repo))
            .expect("credential-free proxy settings should be accepted");
        let mut command = Command::new("ignored");
        command.env_clear();
        environment.configure_command(&mut command);
        let configured = command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();
        assert!(configured.iter().any(|(name, value)| {
            name == "HTTPS_PROXY" && value.as_deref() == Some("https://proxy.example:8443/")
        }));
        assert!(configured.iter().any(|(name, value)| {
            name == "NO_PROXY" && value.as_deref() == Some("localhost,127.0.0.1,.internal")
        }));

        drop(_upper_proxy);
        let _unsafe_proxy = EnvVarGuard::set("HTTPS_PROXY", "https://user:secret@proxy.example");
        let proxy_error = TrustedGithubNetworkEnvironment::resolve(path_str(&fixture.repo))
            .expect_err("proxy credentials must fail closed");
        assert!(proxy_error.to_string().contains("credentials"));

        drop(_unsafe_proxy);
        let repository_ca = fixture.repo.join("repository-ca.pem");
        fs::write(&repository_ca, "not a trusted CA\n")
            .expect("repository CA fixture should be written");
        let _ca = EnvVarGuard::set("SSL_CERT_FILE", path_str(&repository_ca));
        let ca_error = TrustedGithubNetworkEnvironment::resolve(path_str(&fixture.repo))
            .expect_err("repository-controlled CA file must fail closed");
        assert!(
            ca_error
                .to_string()
                .contains("repository- or pool-controlled")
        );
    }

    #[cfg(unix)]
    #[test]
    fn isolated_network_context_ignores_hostile_repository_transport_config() {
        let _guard = github_script_lock()
            .lock()
            .expect("GitHub test environment lock should not be poisoned");
        let fixture = GitFixture::new("github-network-config-isolation");
        let fixture_root = fixture
            .repo
            .parent()
            .expect("fixture repo should have a parent");
        let marker = fixture_root.join("hostile-helper-marker");
        let redirected = fixture_root.join("redirected.git");
        git_in(fixture_root, &["init", "--bare", path_str(&redirected)]);
        git(
            &fixture.repo,
            &[
                "config",
                "credential.helper",
                &format!("!touch {}", marker.display()),
            ],
        );
        git(
            &fixture.repo,
            &[
                "config",
                &format!("url.{}.insteadOf", redirected.display()),
                "https://github.com/acme/widgets.git",
            ],
        );
        git(
            &fixture.repo,
            &[
                "config",
                "http.https://github.com.proxy",
                "http://127.0.0.1:9",
            ],
        );
        git(
            &fixture.repo,
            &[
                "config",
                "core.sshCommand",
                &format!("touch {}", marker.display()),
            ],
        );

        let context = IsolatedGithubNetworkContext::new(
            path_str(&fixture.repo),
            "https://github.com/acme/widgets.git",
        )
        .expect("isolated network context should be created");
        let output = run_git_command_in_network_context(
            &context,
            &["remote", "get-url", "--push", FROZEN_GITHUB_REMOTE_NAME],
        )
        .expect("isolated remote URL should resolve");
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "https://github.com/acme/widgets.git"
        );
        let helper =
            run_git_command_in_network_context(&context, &["config", "--get", "credential.helper"])
                .expect("controlled credential helper should be readable");
        let helper = String::from_utf8_lossy(&helper.stdout);
        assert!(helper.contains("AKRA_FROZEN_GITHUB_TOKEN"));
        assert!(!helper.contains("touch"));
        let redirects = run_git_command_in_network_context(
            &context,
            &["config", "--get", "http.followRedirects"],
        )
        .expect("isolated redirect policy should be readable");
        assert!(redirects.status.success());
        assert_eq!(String::from_utf8_lossy(&redirects.stdout).trim(), "false");
        assert!(!marker.exists());
        let mut routed_command = Command::new("sh");
        routed_command
            .env("AKRA_GITHUB_LOGIN", "attacker")
            .env("GH_REPO", "attacker/redirected")
            .env("GH_HOST", "enterprise.invalid")
            .env("GH_CONFIG_DIR", marker.as_os_str());
        context.configure_command(&mut routed_command);
        let routed_environment = routed_command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();
        assert!(
            routed_environment
                .iter()
                .any(|(key, value)| { key == "GH_REPO" && value.is_none() })
        );
        assert!(
            routed_environment
                .iter()
                .any(|(key, value)| { key == "GH_CONFIG_DIR" && value.is_none() })
        );
        assert!(
            routed_environment
                .iter()
                .any(|(key, value)| { key == "GH_HOST" && value.as_deref() == Some("github.com") })
        );
        assert!(routed_environment.iter().any(|(key, value)| {
            key == "AKRA_GITHUB_LOGIN" && value.as_deref() != Some("attacker")
        }));
        let config = fs::read_to_string(context.repo_root.join(".git/config"))
            .expect("isolated config should be readable");
        assert!(!config.contains("ghp_"));
        assert!(!config.contains("fixture-token"));

        let root = context.root.clone();
        drop(context);
        assert!(!root.exists(), "isolated network context should be removed");
    }

    #[cfg(unix)]
    #[test]
    fn isolated_network_context_rejects_symlinked_source_object_directory() {
        use std::os::unix::fs::symlink;

        let fixture = GitFixture::new("github-network-object-symlink");
        let git_dir = PathBuf::from(git_stdout(
            &fixture.repo,
            &["rev-parse", "--path-format=absolute", "--git-dir"],
        ));
        let objects = git_dir.join("objects");
        let redirected_objects = fixture
            .repo
            .parent()
            .expect("fixture repo should have a parent")
            .join("redirected-objects");
        fs::rename(&objects, &redirected_objects).expect("fixture object directory should move");
        symlink(&redirected_objects, &objects).expect("object directory symlink should be created");

        let error = IsolatedGithubNetworkContext::new(
            path_str(&fixture.repo),
            "https://github.com/acme/widgets.git",
        )
        .err()
        .expect("symlinked source object directory must fail closed");

        assert!(
            error
                .to_string()
                .contains("object directory must be a real directory")
        );
    }

    #[test]
    fn isolated_fetch_keeps_previously_unseen_objects_in_the_source_repository() {
        let fixture = GitFixture::new("github-network-fetch-objects");
        git(&fixture.repo, &["push", "origin", "main"]);
        let fixture_root = fixture
            .repo
            .parent()
            .expect("fixture repo should have a parent");
        let publisher = fixture_root.join("publisher");
        git_in(
            fixture_root,
            &["clone", path_str(&fixture.remote), path_str(&publisher)],
        );
        git(&publisher, &["checkout", "-b", "main", "origin/main"]);
        git(&publisher, &["config", "user.name", "Publisher"]);
        git(
            &publisher,
            &["config", "user.email", "publisher@example.com"],
        );
        fs::write(publisher.join("remote-only.txt"), "remote only\n")
            .expect("remote-only fixture should write");
        git(&publisher, &["add", "remote-only.txt"]);
        git(&publisher, &["commit", "-m", "Remote-only commit"]);
        git(&publisher, &["push", "origin", "main"]);
        let remote_only_sha = git_stdout(&publisher, &["rev-parse", "HEAD"]);
        assert!(
            !Command::new("git")
                .current_dir(&fixture.repo)
                .args(["cat-file", "-e", &format!("{remote_only_sha}^{{commit}}")])
                .status()
                .expect("git cat-file should launch")
                .success(),
            "source repository must not already contain the remote-only object"
        );

        let fetched_sha = fetch_branch_to_tracking_ref_isolated(
            path_str(&fixture.repo),
            path_str(&fixture.remote),
            "main",
            "refs/remotes/origin/isolated-fetch-proof",
        )
        .expect("isolated fetch should import the remote-only object");

        assert_eq!(fetched_sha, remote_only_sha);
        assert_eq!(
            git_stdout(
                &fixture.repo,
                &["rev-parse", "refs/remotes/origin/isolated-fetch-proof"]
            ),
            remote_only_sha
        );
        git(
            &fixture.repo,
            &["cat-file", "-e", &format!("{remote_only_sha}^{{commit}}")],
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
            missing_program.to_string().contains(
                "trusted `__codex_exec_loop_missing_program__` executable is unavailable"
            )
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
        assert!(missing.detail.contains(
            "push remote `origin` is not a credential-free GitHub HTTPS delivery target"
        ));
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
        assert!(capability.detail.contains("push readiness probe failed"));
        assert!(capability.detail.contains("frozen GitHub target"));
        assert!(capability.next_action.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn push_remote_capability_preserves_identity_probe_failure_detail() {
        const IDENTITY_FAILURE: &[u8] = br#"#!/usr/bin/env bash
printf '%s\n' 'identity-probe-marker' >&2
exit 23
"#;

        let _lock = github_script_lock()
            .lock()
            .expect("GitHub test environment lock should not be poisoned");
        let fixture = GitFixture::new("github-automation-identity-probe-failure");
        git(
            &fixture.repo,
            &[
                "remote",
                "set-url",
                "origin",
                "https://github.com/acme/widgets.git",
            ],
        );
        let _helper_guard = TestGithubHelperGuard::install(IDENTITY_FAILURE);

        let capability = GithubAutomationAdapter::inspect_push_remote(path_str(&fixture.repo));

        assert_eq!(capability.key, ParallelModeCapabilityKey::PushRemote);
        assert_eq!(capability.state, ParallelModeCapabilityState::Degraded);
        assert!(capability.detail.contains("push readiness probe failed"));
        assert!(capability.detail.contains("identity-probe-marker"));
        assert!(
            capability
                .detail
                .contains("GitHub write identity verification failed")
        );
        assert!(capability.next_action.is_some());
    }

    #[test]
    fn push_remote_capability_reports_configured_remote_without_current_branch() {
        let fixture = GitFixture::new("github-automation-detached-head-capability");
        git(&fixture.repo, &["checkout", "--detach"]);

        let capability = GithubAutomationAdapter::inspect_push_remote(path_str(&fixture.repo));

        assert_eq!(capability.key, ParallelModeCapabilityKey::PushRemote);
        assert_eq!(capability.state, ParallelModeCapabilityState::Ready);
        assert!(
            capability
                .detail
                .contains("push remote `origin` is configured")
        );
        assert!(capability.next_action.is_none());
    }

    #[test]
    fn push_remote_capability_uses_repo_configured_remote() {
        let _guard = github_script_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _env_guard = EnvVarGuard::set(AKRA_GITHUB_PUSH_REMOTE_ENV_VAR, "");
        let fixture = GitFixture::new("github-automation-configured-remote-capability");
        git(&fixture.repo, &["remote", "rename", "origin", "upstream"]);
        git(
            &fixture.repo,
            &["config", AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY, "upstream"],
        );

        let capability = GithubAutomationAdapter::inspect_push_remote(path_str(&fixture.repo));

        assert_eq!(capability.key, ParallelModeCapabilityKey::PushRemote);
        assert_eq!(capability.state, ParallelModeCapabilityState::Ready);
        assert!(capability.detail.contains("push dry-run succeeded"));
        assert!(capability.next_action.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn gh_auth_capability_degrades_when_command_surface_is_not_ready() {
        let gh_binary = ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhBinary,
            ParallelModeCapabilityState::Degraded,
            "gh missing",
            Some("install gh".to_string()),
        );

        let _guard = github_script_lock()
            .lock()
            .expect("github script fixture lock should not be poisoned");
        let fixture = GitFixture::new("github-automation-auth-missing");
        git(
            &fixture.repo,
            &[
                "remote",
                "set-url",
                "origin",
                "https://github.com/acme/widgets.git",
            ],
        );
        let _akra_token_guard = EnvVarGuard::set("AKRA_GITHUB_TOKEN", "");
        let _gh_token_guard = EnvVarGuard::set("GH_TOKEN", "");
        let _github_token_guard = EnvVarGuard::set("GITHUB_TOKEN", "");
        let _home_guard = EnvVarGuard::set(
            "HOME",
            fixture
                .repo
                .parent()
                .expect("fixture repo should have a parent")
                .to_string_lossy()
                .as_ref(),
        );
        let _git_config_global_guard = EnvVarGuard::set("GIT_CONFIG_GLOBAL", "/dev/null");
        let capability =
            GithubAutomationAdapter::inspect_gh_auth(&gh_binary, path_str(&fixture.repo));

        assert_eq!(capability.key, ParallelModeCapabilityKey::GhAuth);
        match capability.state {
            ParallelModeCapabilityState::Ready => {
                assert!(capability.detail.contains("authentication succeeded"));
                assert!(capability.next_action.is_none());
            }
            ParallelModeCapabilityState::Degraded => {
                assert!(
                    capability.detail.contains("not authenticated")
                        || capability
                            .detail
                            .contains("trusted GitHub automation runtime")
                );
                assert!(capability.next_action.is_some());
            }
            other => panic!("unexpected gh auth capability state: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn gh_capabilities_ignore_repo_parent_cli_outside_trusted_system_locations() {
        let _guard = github_script_lock()
            .lock()
            .expect("github script fixture lock should not be poisoned");
        let fixture = GitFixture::new("github-automation-fake-gh");
        git(
            &fixture.repo,
            &[
                "remote",
                "set-url",
                "origin",
                "https://github.com/acme/widgets.git",
            ],
        );
        git(
            &fixture.repo,
            &["config", "akra.githubLogin", "RefinedStone"],
        );
        git(
            &fixture.repo,
            &[
                "config",
                "credential.helper",
                "!f() { cat >/dev/null; printf 'username=RefinedStone\\npassword=fixture-token\\n'; }; f",
            ],
        );
        let bin_dir = fixture
            .repo
            .parent()
            .expect("fixture repo should have a parent")
            .join("bin");
        fs::create_dir_all(&bin_dir).expect("fake gh bin directory should be created");
        let marker = bin_dir.join("executed");
        write_token_exfiltrator(&bin_dir.join("gh"), &marker);
        write_token_exfiltrator(&bin_dir.join("curl"), &marker);
        let gh_binary = GithubAutomationAdapter::inspect_gh_binary(path_str(&fixture.repo));

        assert_eq!(gh_binary.key, ParallelModeCapabilityKey::GhBinary);
        assert_eq!(gh_binary.state, ParallelModeCapabilityState::Ready);
        assert!(
            !gh_binary
                .detail
                .contains(bin_dir.to_string_lossy().as_ref())
        );
        assert!(
            !marker.exists(),
            "repository parent CLI must not be executed"
        );
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
        let _script_path = install_fake_github_script();
        let fixture = GitFixture::new("github-automation-pr-lifecycle");
        let repo = fixture.repo;
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

        let created_title = "Created PR title stays out of argv";
        let created_body = "created PR body stays out of argv";
        let created = adapter
            .ensure_pull_request(
                path_str(&repo),
                "prerelease",
                "feature/new",
                created_title,
                created_body,
            )
            .expect("create URL fallback should inspect created PR");
        assert_eq!(created.number, 42);
        assert_eq!(created.base_branch, "prerelease");
        let create_args = read_fake_gh_args();
        assert!(create_args.iter().any(|arg| arg == "--title-file"));
        assert!(create_args.iter().any(|arg| arg == "--body-file"));
        assert!(!create_args.iter().any(|arg| arg == "--title"));
        assert!(!create_args.iter().any(|arg| arg == "--body"));
        assert!(!create_args.iter().any(|arg| arg == created_title));
        assert!(!create_args.iter().any(|arg| arg == created_body));

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

        let sensitive_title = "Create Fail Private Title";
        let sensitive_body = "private create body must stay out of surfaced errors";
        let create_failure = adapter
            .ensure_pull_request(
                path_str(&repo),
                "prerelease",
                "feature/create-fail",
                sensitive_title,
                sensitive_body,
            )
            .expect_err("PR create command failure should be reported");
        let create_failure_text = create_failure.to_string();
        assert!(create_failure_text.contains("create denied"));
        assert!(create_failure_text.contains(&format!(
            "--title-file [redacted:{} chars]",
            sensitive_title.chars().count()
        )));
        assert!(create_failure_text.contains(&format!(
            "--body-file [redacted:{} chars]",
            sensitive_body.chars().count()
        )));
        assert!(!create_failure_text.contains(sensitive_title));
        assert!(!create_failure_text.contains(sensitive_body));

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

        remove_fake_github_script();
    }

    #[test]
    fn pull_request_wrapper_is_pinned_to_the_repo_configured_push_remote() {
        let _guard = github_script_lock()
            .lock()
            .expect("github script fixture lock should not be poisoned");
        let _env_guard = EnvVarGuard::set(AKRA_GITHUB_PUSH_REMOTE_ENV_VAR, "");
        let _script_path = install_fake_github_script();
        let fixture = GitFixture::new("github-automation-wrapper-push-remote");
        git(&fixture.repo, &["remote", "rename", "origin", "upstream"]);
        git(
            &fixture.repo,
            &["config", AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY, "upstream"],
        );
        let adapter = GithubAutomationAdapter::new();

        adapter
            .ensure_pull_request(
                path_str(&fixture.repo),
                "prerelease",
                "feature/existing",
                "Existing",
                "body",
            )
            .expect("wrapper lookup should use the configured remote");

        assert_eq!(read_fake_gh_remote(), FROZEN_GITHUB_REMOTE_NAME);
        remove_fake_github_script();
    }

    #[test]
    fn pull_request_create_timeout_redacts_sensitive_fields() {
        let _guard = github_script_lock()
            .lock()
            .expect("github script fixture lock should not be poisoned");
        let _script_path = install_fake_github_script();
        let fixture = GitFixture::new("github-automation-pr-timeout");
        let repo = fixture.repo;
        let adapter = GithubAutomationAdapter::new();
        let _timeout_guard = EnvVarGuard::set(SUBPROCESS_TIMEOUT_ENV, "1");
        let sensitive_title = "Create Timeout Private Title";
        let sensitive_body = "private timeout body must stay out of surfaced errors";

        let error = adapter
            .ensure_pull_request(
                path_str(&repo),
                "prerelease",
                "feature/create-timeout",
                sensitive_title,
                sensitive_body,
            )
            .expect_err("slow PR create should time out");
        let error_text = error.to_string();
        let error_chain = error
            .chain()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" | ");

        assert!(error_chain.contains("timed out after"));
        assert!(error_text.contains(&format!(
            "--title-file [redacted:{} chars]",
            sensitive_title.chars().count()
        )));
        assert!(error_text.contains(&format!(
            "--body-file [redacted:{} chars]",
            sensitive_body.chars().count()
        )));
        assert!(!error_text.contains(sensitive_title));
        assert!(!error_text.contains(sensitive_body));

        remove_fake_github_script();
    }

    #[test]
    fn temporary_pr_body_file_uses_private_permissions() {
        let file = TemporaryTextFile::new("github-pr-body-test", "secret body")
            .expect("temporary body file should be created");
        let contents =
            fs::read_to_string(file.path()).expect("temporary body file should be readable");
        assert_eq!(contents, "secret body");
        #[cfg(unix)]
        {
            let mode = fs::metadata(file.path())
                .expect("temporary body file metadata should be readable")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
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
            .push_integration_branch(
                path_str(&fixture.repo),
                "main",
                &git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
            )
            .expect("integration push should use the same local origin");

        assert_eq!(
            git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
            git_stdout(&fixture.repo, &["rev-parse", "main"])
        );
    }

    #[test]
    fn push_methods_publish_local_branches_to_repo_configured_remote() {
        let _guard = github_script_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _env_guard = EnvVarGuard::set(AKRA_GITHUB_PUSH_REMOTE_ENV_VAR, "");
        let fixture = GitFixture::new("github-automation-configured-remote-push");
        let adapter = GithubAutomationAdapter::new();
        git(&fixture.repo, &["remote", "rename", "origin", "upstream"]);
        git(
            &fixture.repo,
            &["config", AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY, "upstream"],
        );

        adapter
            .push_branch(path_str(&fixture.repo), "main", false)
            .expect("branch push should publish to configured remote");
        adapter
            .push_integration_branch(
                path_str(&fixture.repo),
                "main",
                &git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
            )
            .expect("integration push should use the configured remote");

        assert_eq!(
            git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
            git_stdout(&fixture.repo, &["rev-parse", "main"])
        );
    }

    #[test]
    fn frozen_commit_push_does_not_publish_a_later_local_branch_tip() {
        let fixture = GitFixture::new("github-automation-frozen-source-push");
        let adapter = GithubAutomationAdapter::new();
        let frozen_commit = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);

        fs::write(fixture.repo.join("README.md"), "drifted\n")
            .expect("drift fixture should be writable");
        git(&fixture.repo, &["add", "README.md"]);
        git(&fixture.repo, &["commit", "-m", "Unreviewed later commit"]);
        let drifted_commit = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
        assert_ne!(frozen_commit, drifted_commit);

        adapter
            .push_frozen_commit_to_branch(
                path_str(&fixture.repo),
                "origin",
                &frozen_commit,
                "agent/frozen-result",
            )
            .expect("exact frozen refspec should publish");

        assert_eq!(
            git_stdout(
                &fixture.remote,
                &["rev-parse", "refs/heads/agent/frozen-result"]
            ),
            frozen_commit
        );
        assert_eq!(
            git_stdout(&fixture.repo, &["rev-parse", "HEAD"]),
            drifted_commit
        );
    }

    #[test]
    fn integration_push_exact_lease_rejects_a_remote_race() {
        let fixture = GitFixture::new("github-automation-integration-race");
        let adapter = GithubAutomationAdapter::new();
        adapter
            .push_branch(path_str(&fixture.repo), "main", false)
            .expect("initial integration branch should publish");
        let expected_old = git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]);

        fs::write(fixture.repo.join("RESULT.md"), "reviewed result\n")
            .expect("reviewed result fixture should write");
        git(&fixture.repo, &["add", "RESULT.md"]);
        git(
            &fixture.repo,
            &["commit", "-m", "Reviewed integration result"],
        );
        let reviewed_result = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);

        git(&fixture.repo, &["checkout", "--detach", &expected_old]);
        fs::write(fixture.repo.join("RACE.md"), "concurrent result\n")
            .expect("race fixture should write");
        git(&fixture.repo, &["add", "RACE.md"]);
        git(&fixture.repo, &["commit", "-m", "Concurrent remote result"]);
        git(
            &fixture.repo,
            &["push", "--force", "origin", "HEAD:refs/heads/main"],
        );
        let raced_remote = git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]);
        git(&fixture.repo, &["checkout", "--detach", &reviewed_result]);

        let error = adapter
            .push_integration_branch_to_remote(
                path_str(&fixture.repo),
                "origin",
                "main",
                &expected_old,
            )
            .expect_err("exact force-with-lease must reject a moved remote");

        assert!(error.to_string().contains("git push"));
        assert_eq!(
            git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
            raced_remote
        );
    }

    #[test]
    fn integration_push_rejects_a_local_result_outside_the_frozen_base_history() {
        let fixture = GitFixture::new("github-automation-integration-ancestry");
        let adapter = GithubAutomationAdapter::new();
        adapter
            .push_branch(path_str(&fixture.repo), "main", false)
            .expect("initial integration branch should publish");
        let expected_old = git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]);

        git(&fixture.repo, &["checkout", "--orphan", "unrelated-result"]);
        git(&fixture.repo, &["rm", "-rf", "."]);
        fs::write(fixture.repo.join("UNRELATED.md"), "unrelated\n")
            .expect("unrelated result fixture should write");
        git(&fixture.repo, &["add", "UNRELATED.md"]);
        git(
            &fixture.repo,
            &["commit", "-m", "Unrelated integration result"],
        );

        let error = adapter
            .push_integration_branch_to_remote(
                path_str(&fixture.repo),
                "origin",
                "main",
                &expected_old,
            )
            .expect_err("unrelated local result must not be pushed");

        assert!(
            error
                .to_string()
                .contains("not a descendant of the frozen remote base")
        );
        assert_eq!(
            git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
            expected_old
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
        crate::test_utils::process_environment_mutex()
    }

    fn install_fake_github_script() -> TestGithubHelperGuard {
        let source: &'static [u8] = br#"#!/usr/bin/env bash
	set -euo pipefail
	args="$*"
	log_dir="${AKRA_TEST_GITHUB_LOG_DIR:?}"
	mkdir -p "${log_dir}"
	printf '%s\n' "${AKRA_GITHUB_PUSH_REMOTE:-}" > "${log_dir}/last-remote"
	if [[ "${1-}" == "pr" && "${2-}" == "create" ]]; then
	  printf '%s\n' "$@" > "${log_dir}/last-args"
  case " $* " in
    *" --title-file "*) ;;
    *)
      printf '%s\n' 'missing --title-file' >&2
      exit 70
      ;;
  esac
  case " $* " in
    *" --body-file "*) ;;
    *)
      printf '%s\n' 'missing --body-file' >&2
      exit 71
      ;;
  esac
fi
case "$args" in
  "auth status"|"auth write-status")
    exit 0
    ;;
  "repo visibility")
    printf '%s\n' 'private'
    ;;
  pr\ list*feature/existing*)
    printf '%s\n' '[{"number":41,"url":"https://github.example/pull/41","state":"OPEN","baseRefName":"prerelease","headRefName":"feature/existing","isDraft":false}]'
    ;;
	  pr\ list*feature/race*)
	    if [[ -f "${log_dir}/race-created" ]]; then
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
	    touch "${log_dir}/race-created"
    printf '%s\n' 'created without url'
    ;;
  pr\ create*feature/new*)
    printf '%s\n' 'https://github.example/pull/42'
    ;;
  pr\ create*feature/create-fail*)
    printf '%s\n' 'create denied' >&2
    exit 23
    ;;
  pr\ create*feature/create-timeout*)
    sleep 2
    printf '%s\n' 'https://github.example/pull/77'
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
"#;
        TestGithubHelperGuard::install(source)
    }

    fn remove_fake_github_script() {
        TEST_GITHUB_HELPER_SOURCE.with(|slot| {
            slot.replace(None);
        });
        let _ = fs::remove_dir_all(fake_github_log_dir());
    }

    struct TestGithubHelperGuard;

    impl TestGithubHelperGuard {
        fn install(source: &'static [u8]) -> Self {
            TEST_GITHUB_HELPER_SOURCE.with(|slot| {
                assert!(slot.replace(Some(source)).is_none());
            });
            Self
        }
    }

    impl Drop for TestGithubHelperGuard {
        fn drop(&mut self) {
            TEST_GITHUB_HELPER_SOURCE.with(|slot| {
                slot.replace(None);
            });
            let _ = fs::remove_dir_all(fake_github_log_dir());
        }
    }

    fn read_fake_gh_args() -> Vec<String> {
        fs::read_to_string(fake_github_log_dir().join("last-args"))
            .expect("fake github script should record create arguments")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn read_fake_gh_remote() -> String {
        fs::read_to_string(fake_github_log_dir().join("last-remote"))
            .expect("fake github script should record the pinned remote")
            .trim()
            .to_string()
    }

    fn fake_github_log_dir() -> PathBuf {
        super::test_github_log_directory()
    }

    #[cfg(unix)]
    fn write_token_exfiltrator(path: &Path, marker: &Path) {
        fs::write(
            path,
            format!(
                "#!/bin/sh\nprintf '%s' \"${{AKRA_GITHUB_TOKEN:-${{GH_TOKEN:-${{GITHUB_TOKEN:-}}}}}}\" > '{}'\nexit 91\n",
                marker.display()
            ),
        )
        .expect("hostile executable should be written");
        let mut permissions = fs::metadata(path)
            .expect("hostile executable metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("hostile executable should be executable");
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
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

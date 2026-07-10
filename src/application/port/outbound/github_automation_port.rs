// GitHub automation은 git push, gh CLI/API, network/auth 경계에 닿으므로 실패가 정상적인 결과이다.
// distributor는 이 오류를 blocked delivery state로 바꾸기 위해 `anyhow::Result`를 받는다.
use anyhow::Result;
// capability snapshot은 supervisor/session detail에 저장되므로 serde round-trip이 필요하다.
use serde::{Deserialize, Serialize};

// GitHub automation readiness도 parallel mode capability projection vocabulary를 공유한다.
// push remote, gh binary, auth 상태를 같은 snapshot 구조로 표현하면 supervisor UI가 일관된 readiness line을 만들 수 있다.
use crate::domain::parallel_mode::{ParallelModeCapabilitySnapshot, ParallelModeCapabilityState};

pub const DEFAULT_GITHUB_PUSH_REMOTE_NAME: &str = "origin";
pub const AKRA_GITHUB_PUSH_REMOTE_ENV_VAR: &str = "AKRA_GITHUB_PUSH_REMOTE";
pub const AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY: &str = "akra.githubPushRemote";
pub const GITHUB_AUTOMATION_SCRIPT_RELATIVE_PATH: &str = "scripts/gh-akra.sh";

pub fn resolve_github_push_remote_name(
    env_value: Option<&str>,
    config_value: Option<&str>,
) -> String {
    normalize_github_push_remote_name(env_value)
        .or_else(|| normalize_github_push_remote_name(config_value))
        .unwrap_or_else(|| DEFAULT_GITHUB_PUSH_REMOTE_NAME.to_string())
}

pub fn resolve_github_push_remote_name_strict(
    env_value: Option<&str>,
    config_value: Option<&str>,
) -> std::result::Result<String, &'static str> {
    if let Some(value) = env_value.filter(|value| !value.trim().is_empty()) {
        return normalize_github_push_remote_name(Some(value))
            .ok_or("AKRA_GITHUB_PUSH_REMOTE is invalid");
    }
    if let Some(value) = config_value.filter(|value| !value.trim().is_empty()) {
        return normalize_github_push_remote_name(Some(value))
            .ok_or("akra.githubPushRemote is invalid");
    }
    Ok(DEFAULT_GITHUB_PUSH_REMOTE_NAME.to_string())
}

pub fn normalize_github_push_remote_name(value: Option<&str>) -> Option<String> {
    let raw_value = value?.trim();
    if raw_value.is_empty()
        || raw_value.starts_with('-')
        || raw_value.ends_with('.')
        || raw_value.ends_with(".lock")
        || raw_value.contains("..")
        || raw_value.contains("@{")
        || raw_value.chars().any(|ch| {
            ch.is_whitespace() || !matches!(ch, 'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-')
        })
    {
        return None;
    }
    Some(raw_value.to_string())
}

pub fn parse_github_repository_identity(remote_url: &str) -> Option<String> {
    let value = remote_url.trim();
    let repository_path = if let Some(path) = value.strip_prefix("git@github.com:") {
        path
    } else if let Some(path) = value.strip_prefix("ssh://git@github.com/") {
        path
    } else if let Some((_, remainder)) = value.split_once("://") {
        let (authority, path) = remainder.split_once('/')?;
        let host = authority.rsplit('@').next()?.split(':').next()?;
        if !host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        path
    } else {
        return None;
    };

    let repository_path = repository_path
        .split(['?', '#'])
        .next()?
        .trim_matches('/')
        .strip_suffix(".git")
        .unwrap_or_else(|| repository_path.trim_matches('/'));
    let mut segments = repository_path.split('/');
    let owner = segments.next()?;
    let repository = segments.next()?;
    if segments.next().is_some()
        || !valid_github_repository_segment(owner)
        || !valid_github_repository_segment(repository)
    {
        return None;
    }
    Some(format!("{owner}/{repository}"))
}

pub fn credential_redacted_canonical_github_push_url(remote_url: &str) -> Option<String> {
    let value = remote_url.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    let repository = parse_github_repository_identity(value)?;
    if value.starts_with("git@github.com:") || value.starts_with("ssh://git@github.com/") {
        return Some(format!("ssh://git@github.com/{repository}.git"));
    }

    let (scheme, remainder) = value.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") || value.contains(['?', '#']) {
        return None;
    }
    let (authority, _) = remainder.split_once('/')?;
    let host = authority.rsplit('@').next()?;
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    Some(format!("https://github.com/{repository}.git"))
}

fn valid_github_repository_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// `GithubAutomationCapabilities`는 distributor delivery가 GitHub write side를 사용할 수 있는지
// 판단하는 세 축의 readiness snapshot이다. push는 git remote 권한이고, PR 생성/조회/close는 gh binary와 auth가 필요하다.
pub struct GithubAutomationCapabilities {
    // agent branch를 origin에 push할 수 있는지 나타낸다.
    pub push_remote: ParallelModeCapabilitySnapshot,
    // `gh` CLI 또는 adapter가 요구하는 GitHub command surface가 있는지 나타낸다.
    pub gh_binary: ParallelModeCapabilitySnapshot,
    // GitHub auth가 현재 repo에서 유효한지 나타낸다.
    pub gh_auth: ParallelModeCapabilitySnapshot,
}

impl GithubAutomationCapabilities {
    // capability snapshots를 하나의 value object로 묶는 생성자이다.
    // 테스트 fake와 production adapter가 같은 생성자를 쓰면 readiness 축 순서가 어긋나지 않는다.
    pub fn new(
        // git push 가능성이다.
        push_remote: ParallelModeCapabilitySnapshot,
        // gh binary/command surface 가능성이다.
        gh_binary: ParallelModeCapabilitySnapshot,
        // GitHub authentication 가능성이다.
        gh_auth: ParallelModeCapabilitySnapshot,
    ) -> Self {
        Self {
            push_remote,
            gh_binary,
            gh_auth,
        }
    }

    // push readiness만 빠르게 확인하는 helper이다. distributor는 PR 단계 이전에
    // branch push가 막혀 있는지 별도로 판단해야 한다.
    pub fn push_ready(&self) -> bool {
        self.push_remote.state == ParallelModeCapabilityState::Ready
    }

    // PR 생성/조회/close 같은 GitHub API/CLI 작업 가능성을 판단한다.
    // push remote가 ready여도 gh binary/auth가 없으면 PR delivery는 별도 정책으로 처리해야 한다.
    pub fn pull_request_workflow_ready(&self) -> bool {
        self.gh_binary.state == ParallelModeCapabilityState::Ready
            && self.gh_auth.state == ParallelModeCapabilityState::Ready
    }

    // 기존 호출부와 serialized 의미를 깨지 않기 위한 호환 helper다.
    pub fn github_ready(&self) -> bool {
        self.pull_request_workflow_ready()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
// `GithubAutomationPullRequest`는 adapter가 GitHub PR 원문을 delivery orchestration이 쓰는 최소 shape로
// 정규화한 값이다. distributor는 이 값으로 PR 번호/URL을 session detail에 남기고, draft/open/base/head 상태를 검사한다.
pub struct GithubAutomationPullRequest {
    // GitHub PR number이다. 이후 inspect/close 호출의 key로 사용된다.
    pub number: u64,
    // 사람이 열 수 있는 PR URL이다. supervisor/detail UI에 노출된다.
    pub url: String,
    // GitHub가 돌려준 open/closed/merged 등 상태 문자열이다.
    pub state: String,
    // PR target branch이다. 이 repo의 delivery 흐름에서는 보통 `prerelease`여야 한다.
    pub base_branch: String,
    // PR source branch이다. slot/agent branch와 일치해야 distributor가 올바른 작업을 추적할 수 있다.
    pub head_branch: String,
    // draft PR 여부이다. delivery가 reviewable 상태인지 판단하는 데 사용된다.
    pub is_draft: bool,
    // GitHub review decision이다. 값이 없으면 review gate 상태를 알 수 없는 것으로 취급한다.
    pub review_decision: Option<String>,
    // GitHub merge queue/branch protection을 포함한 merge state이다.
    pub merge_state_status: Option<String>,
    // status check rollup이 명시적으로 모두 통과했는지 여부이다. `None`은 조회 불가다.
    pub required_checks_passed: Option<bool>,
    // PR head ref가 가리키는 immutable commit OID이다. queue의 frozen source SHA와 비교한다.
    pub head_commit_sha: Option<String>,
    // 현재 유효한 APPROVED review가 제출된 commit OID들이다. aggregate decision만으로는
    // stale approval이 새 head에 재사용되는 것을 막을 수 없어 frozen source와 별도로 결합한다.
    pub approved_review_commit_shas: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubRepositoryVisibility {
    Private,
    Internal,
    Public,
}

impl GithubAutomationPullRequest {
    // PR projection 생성자이다. 문자열 필드를 `Into<String>`으로 받아 production JSON mapping과
    // 테스트 fixture가 모두 간결하게 값을 만들 수 있게 한다.
    pub fn new(
        // GitHub PR number이다.
        number: u64,
        // PR URL이다.
        url: impl Into<String>,
        // PR state이다.
        state: impl Into<String>,
        // target/base branch이다.
        base_branch: impl Into<String>,
        // source/head branch이다.
        head_branch: impl Into<String>,
        // draft flag이다.
        is_draft: bool,
    ) -> Self {
        Self {
            number,
            url: url.into(),
            state: state.into(),
            base_branch: base_branch.into(),
            head_branch: head_branch.into(),
            is_draft,
            review_decision: None,
            merge_state_status: None,
            required_checks_passed: None,
            head_commit_sha: None,
            approved_review_commit_shas: Vec::new(),
        }
    }

    pub fn with_merge_gate(
        mut self,
        review_decision: impl Into<String>,
        merge_state_status: impl Into<String>,
        required_checks_passed: bool,
    ) -> Self {
        self.review_decision = Some(review_decision.into());
        self.merge_state_status = Some(merge_state_status.into());
        self.required_checks_passed = Some(required_checks_passed);
        self
    }

    pub fn with_head_commit_sha(mut self, head_commit_sha: impl Into<String>) -> Self {
        self.head_commit_sha = Some(head_commit_sha.into());
        self
    }

    pub fn with_approved_review_commit_sha(
        mut self,
        approved_review_commit_sha: impl Into<String>,
    ) -> Self {
        self.approved_review_commit_shas
            .push(approved_review_commit_sha.into());
        self
    }
}

// `GithubAutomationPort`는 parallel distributor가 GitHub write-side delivery를 수행하기 위해
// 사용하는 outbound 계약이다. push, PR ensure/inspect, integration branch push, PR close를 service 정책에서
// 순서대로 호출하고, adapter는 git/gh 명령과 GitHub 응답 parsing을 소유한다.
pub trait GithubAutomationPort: Send + Sync {
    // repo에서 delivery capability를 점검한다. distributor snapshot은 이 값을 이용해
    // push blocked, GitHub unavailable, auth missing 같은 상태를 operator에게 보여 준다.
    fn inspect_capabilities(&self, repo_root: &str) -> GithubAutomationCapabilities;

    // Configured push URL에서 credential을 제거한 stable `owner/repo` identity를 반환한다.
    fn repository_identity(&self, _repo_root: &str) -> Result<String> {
        anyhow::bail!("GitHub repository identity inspection is unavailable")
    }

    fn repository_visibility(&self, _repo_root: &str) -> Result<GithubRepositoryVisibility> {
        anyhow::bail!("GitHub repository visibility inspection is unavailable")
    }

    fn repository_identity_for_remote(
        &self,
        repo_root: &str,
        _push_remote: &str,
    ) -> Result<String> {
        self.repository_identity(repo_root)
    }

    fn repository_visibility_for_remote(
        &self,
        repo_root: &str,
        _push_remote: &str,
    ) -> Result<GithubRepositoryVisibility> {
        self.repository_visibility(repo_root)
    }

    // Mutable remote aliases are not sufficient delivery identities. New
    // leases persist this credential-free canonical URL and pass it back to
    // every network boundary so a later remote config swap cannot redirect a
    // push or GitHub wrapper invocation.
    fn credential_redacted_push_url_for_remote(
        &self,
        repo_root: &str,
        push_remote: &str,
    ) -> Result<String> {
        let repository = self.repository_identity_for_remote(repo_root, push_remote)?;
        Ok(format!("https://github.com/{repository}.git"))
    }

    fn repository_identity_for_push_url(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        credential_redacted_push_url: &str,
    ) -> Result<String> {
        parse_github_repository_identity(credential_redacted_push_url)
            .ok_or_else(|| anyhow::anyhow!("frozen push URL does not identify a GitHub repository"))
    }

    fn repository_visibility_for_push_url(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _credential_redacted_push_url: &str,
    ) -> Result<GithubRepositoryVisibility> {
        anyhow::bail!("exact frozen-target repository visibility inspection is unavailable")
    }

    // agent/slot branch를 remote에 push한다. rebase recovery나 retry에서는
    // `force_with_lease`를 사용해 원격 변경을 무작정 덮어쓰지 않는 안전장치를 유지한다.
    fn push_branch(&self, repo_root: &str, branch_name: &str, force_with_lease: bool)
    -> Result<()>;

    fn push_branch_to_remote(
        &self,
        repo_root: &str,
        _push_remote: &str,
        branch_name: &str,
        force_with_lease: bool,
    ) -> Result<()> {
        self.push_branch(repo_root, branch_name, force_with_lease)
    }

    // Frozen distributor 결과를 mutable local branch 이름으로 다시 해석하지 않고,
    // 검증된 commit OID 자체를 remote source branch에 publish한다. 구현하지 않은
    // adapter가 기존 branch-name push로 조용히 후퇴하면 enqueue 이후 commit drift가
    // 원격에 노출될 수 있으므로 기본 구현은 명시적으로 실패한다.
    fn push_frozen_commit_to_branch(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _source_commit_sha: &str,
        _branch_name: &str,
    ) -> Result<()> {
        anyhow::bail!("exact frozen-commit push is unavailable")
    }

    fn push_frozen_commit_to_delivery_target(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _credential_redacted_push_url: &str,
        _source_commit_sha: &str,
        _branch_name: &str,
    ) -> Result<()> {
        anyhow::bail!("exact frozen-target source push is unavailable")
    }

    // source branch에 대한 PR을 보장한다. 이미 열려 있으면 기존 PR을 반환하고,
    // 없으면 title/body로 새 PR을 만들어 delivery state가 PR number를 추적할 수 있게 한다.
    fn ensure_pull_request(
        &self,
        // GitHub remote가 설정된 repository root이다.
        repo_root: &str,
        // PR target branch이다.
        base_branch: &str,
        // PR source branch이다.
        head_branch: &str,
        // PR title이다.
        title: &str,
        // PR body이다. distributor는 worker result와 validation summary를 여기에 담는다.
        body: &str,
    ) -> Result<GithubAutomationPullRequest>;

    fn ensure_pull_request_for_remote(
        &self,
        repo_root: &str,
        _push_remote: &str,
        base_branch: &str,
        head_branch: &str,
        title: &str,
        body: &str,
    ) -> Result<GithubAutomationPullRequest> {
        self.ensure_pull_request(repo_root, base_branch, head_branch, title, body)
    }

    #[allow(clippy::too_many_arguments)]
    fn ensure_pull_request_for_delivery_target(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _credential_redacted_push_url: &str,
        _base_branch: &str,
        _head_branch: &str,
        _title: &str,
        _body: &str,
    ) -> Result<GithubAutomationPullRequest> {
        anyhow::bail!("exact frozen-target pull request ensure is unavailable")
    }

    // 이미 알고 있는 PR number의 현재 상태를 다시 읽는다. blocked/retry/recovery 흐름은
    // 이 값을 통해 PR이 여전히 같은 head/base를 가리키는지 확인한다.
    fn inspect_pull_request(
        &self,
        // repository root이다.
        repo_root: &str,
        // 조회할 PR number이다.
        pr_number: u64,
    ) -> Result<GithubAutomationPullRequest>;

    fn inspect_pull_request_for_remote(
        &self,
        repo_root: &str,
        _push_remote: &str,
        pr_number: u64,
    ) -> Result<GithubAutomationPullRequest> {
        self.inspect_pull_request(repo_root, pr_number)
    }

    fn inspect_pull_request_for_delivery_target(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _credential_redacted_push_url: &str,
        _pr_number: u64,
    ) -> Result<GithubAutomationPullRequest> {
        anyhow::bail!("exact frozen-target pull request inspection is unavailable")
    }

    // integration branch를 push한다. adapter는 local result가 frozen old OID의 후손인지
    // 확인하고 그 OID에 대한 exact force-with-lease CAS로만 원격 ref를 갱신한다.
    fn push_integration_branch(
        &self,
        repo_root: &str,
        branch_name: &str,
        expected_old_commit_sha: &str,
    ) -> Result<()>;

    fn push_integration_branch_to_remote(
        &self,
        repo_root: &str,
        _push_remote: &str,
        branch_name: &str,
        expected_old_commit_sha: &str,
    ) -> Result<()> {
        self.push_integration_branch(repo_root, branch_name, expected_old_commit_sha)
    }

    fn push_integration_branch_to_delivery_target(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _credential_redacted_push_url: &str,
        _branch_name: &str,
        _expected_old_commit_sha: &str,
    ) -> Result<()> {
        anyhow::bail!("exact frozen-target integration push is unavailable")
    }

    // 더 이상 필요 없는 PR을 닫는다. 통합 완료나 recovery cleanup에서 중복 PR을 정리할 때 쓰인다.
    fn close_pull_request(&self, repo_root: &str, pr_number: u64) -> Result<()>;

    fn close_pull_request_for_remote(
        &self,
        repo_root: &str,
        _push_remote: &str,
        pr_number: u64,
    ) -> Result<()> {
        self.close_pull_request(repo_root, pr_number)
    }

    fn close_pull_request_for_delivery_target(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _credential_redacted_push_url: &str,
        _pr_number: u64,
    ) -> Result<()> {
        anyhow::bail!("exact frozen-target pull request close is unavailable")
    }

    fn remote_branch_head(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _branch_name: &str,
    ) -> Result<Option<String>> {
        anyhow::bail!("remote branch head inspection is unavailable")
    }

    fn remote_branch_head_for_delivery_target(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _credential_redacted_push_url: &str,
        _branch_name: &str,
    ) -> Result<Option<String>> {
        anyhow::bail!("exact frozen-target remote head inspection is unavailable")
    }

    fn remote_branch_names_for_prefix_for_delivery_target(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _credential_redacted_push_url: &str,
        _branch_prefix: &str,
    ) -> Result<Vec<String>> {
        anyhow::bail!("exact frozen-target remote branch listing is unavailable")
    }

    fn fetch_branch_to_tracking_ref_for_delivery_target(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _credential_redacted_push_url: &str,
        _branch_name: &str,
        _tracking_ref: &str,
    ) -> Result<String> {
        anyhow::bail!("isolated frozen-target fetch is unavailable")
    }

    fn delete_branch_if_unchanged(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _branch_name: &str,
        _expected_sha: &str,
    ) -> Result<bool> {
        anyhow::bail!("conditional remote branch cleanup is unavailable")
    }

    fn delete_branch_if_unchanged_for_delivery_target(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _credential_redacted_push_url: &str,
        _branch_name: &str,
        _expected_sha: &str,
    ) -> Result<bool> {
        anyhow::bail!("exact frozen-target conditional branch cleanup is unavailable")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY, AKRA_GITHUB_PUSH_REMOTE_ENV_VAR,
        DEFAULT_GITHUB_PUSH_REMOTE_NAME, credential_redacted_canonical_github_push_url,
        normalize_github_push_remote_name, parse_github_repository_identity,
        resolve_github_push_remote_name, resolve_github_push_remote_name_strict,
    };

    #[test]
    fn github_push_remote_name_parser_accepts_simple_remote_aliases() {
        assert_eq!(
            normalize_github_push_remote_name(Some("origin")),
            Some("origin".to_string())
        );
        assert_eq!(
            normalize_github_push_remote_name(Some("upstream-enterprise_2")),
            Some("upstream-enterprise_2".to_string())
        );
    }

    #[test]
    fn github_push_remote_name_parser_rejects_invalid_aliases() {
        for value in [
            "",
            "origin mirror",
            "../origin",
            "origin.lock",
            "origin@{1}",
        ] {
            assert_eq!(
                normalize_github_push_remote_name(Some(value)),
                None,
                "value `{value}` should be rejected"
            );
        }
    }

    #[test]
    fn github_push_remote_name_resolution_prefers_env_then_config_then_default() {
        assert_eq!(
            resolve_github_push_remote_name(Some("env-remote"), Some("config-remote")),
            "env-remote".to_string()
        );
        assert_eq!(
            resolve_github_push_remote_name(None, Some("config-remote")),
            "config-remote".to_string()
        );
        assert_eq!(
            resolve_github_push_remote_name(Some("bad remote"), Some("config-remote")),
            "config-remote".to_string()
        );
        assert_eq!(
            resolve_github_push_remote_name(Some("bad remote"), Some("../bad")),
            DEFAULT_GITHUB_PUSH_REMOTE_NAME.to_string()
        );
    }

    #[test]
    fn explicit_invalid_push_remote_never_falls_back_to_another_target() {
        assert!(
            resolve_github_push_remote_name_strict(Some("bad remote"), Some("upstream")).is_err()
        );
        assert!(resolve_github_push_remote_name_strict(None, Some("../origin")).is_err());
        assert_eq!(
            resolve_github_push_remote_name_strict(Some(""), Some("upstream")),
            Ok("upstream".to_string())
        );
    }

    #[test]
    fn github_push_remote_setting_names_are_stable() {
        assert_eq!(AKRA_GITHUB_PUSH_REMOTE_ENV_VAR, "AKRA_GITHUB_PUSH_REMOTE");
        assert_eq!(AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY, "akra.githubPushRemote");
    }

    #[test]
    fn github_repository_identity_strips_credentials_and_transport_details() {
        for remote_url in [
            "git@github.com:acme/widgets.git",
            "ssh://git@github.com/acme/widgets.git",
            "https://token-user:secret-token@github.com/acme/widgets.git",
        ] {
            assert_eq!(
                parse_github_repository_identity(remote_url),
                Some("acme/widgets".to_string())
            );
        }
        assert_eq!(
            parse_github_repository_identity("https://gitlab.com/acme/widgets.git"),
            None
        );
        assert_eq!(parse_github_repository_identity("/tmp/widgets.git"), None);
    }

    #[test]
    fn canonical_push_url_never_retains_credentials_or_query_secrets() {
        assert_eq!(
            credential_redacted_canonical_github_push_url(
                "https://user:ghp_secret@github.com/acme/widgets.git"
            ),
            Some("https://github.com/acme/widgets.git".to_string())
        );
        assert_eq!(
            credential_redacted_canonical_github_push_url(
                "https://github.com/acme/widgets.git?token=secret"
            ),
            None
        );
        assert_eq!(
            credential_redacted_canonical_github_push_url("git@github.com:acme/widgets.git"),
            Some("ssh://git@github.com/acme/widgets.git".to_string())
        );
        assert_eq!(
            credential_redacted_canonical_github_push_url("https://github.com/../widgets.git"),
            None
        );
    }
}

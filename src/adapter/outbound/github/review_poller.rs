/*
GitHub review poller outbound adapter다.

application service는 "현재 branch의 PR을 찾고, 해당 PR의 review 활동을 시간순 domain snapshot으로
받는다"는 port 계약만 안다. 이 파일은 그 계약을 local git repository identity, RefinedStone credential,
GitHub REST endpoint, curl 실행, 응답 DTO mapping으로 풀어낸다. GitHub API JSON 구조는 private response
타입에 가두고, 바깥에는 `GithubPullRequestActivitySnapshot`만 노출한다.
*/
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::domain::github_review::{
    GithubPullRequestActivityEvent, GithubPullRequestActivityKind,
    GithubPullRequestActivitySnapshot, GithubPullRequestTarget,
};
use anyhow::{Context, Result, anyhow, bail};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
const GITHUB_API_BASE_URL: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2022-11-28";
const PER_PAGE: usize = 100;
const CURL_CONNECT_TIMEOUT_SECONDS: &str = "10";
const CURL_MAX_TIME_SECONDS: &str = "30";
const WINDOWS_USERS_ROOT: &str = "/mnt/c/Users";
// GitHub `head=owner:branch` 같은 query value는 branch slash, colon, and shell-sensitive 문자를 포함할 수 있다.
// endpoint path는 직접 조립하지만 query value는 이 set으로 percent-encode해 GitHub search 조건이 깨지지 않게 한다.
const GITHUB_QUERY_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b'/')
    .add(b':')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');
pub struct GithubReviewPollerAdapter {
    // 테스트와 production이 같은 request builder를 쓰되, 테스트는 curl path/base URL을 바꿀 수 있게 값으로 둔다.
    curl_path: String,
    api_base_url: String,
    user_agent: String,
    // RefinedStone credential에서 추출한 token이다. raw credential line은 이 adapter 밖으로 보존하지 않는다.
    token: String,
}

impl GithubReviewPollerAdapter {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            curl_path: "curl".to_string(),
            api_base_url: GITHUB_API_BASE_URL.to_string(),
            user_agent: format!("codex-exec-loop-native/{}", env!("CARGO_PKG_VERSION")),
            token: token.into(),
        }
    }
    pub fn from_refinedstone_credentials(repo_root: &Path) -> Result<Self> {
        /*
        poller는 repository automation과 같은 RefinedStone credential contract에 의도적으로 묶인다.
        repo-local credential이나 WSL fallback에서 찾은 credential line은 즉시 bearer token으로 변환한다.
        이후 HTTP code는 token이 어느 파일에서 왔는지 모르고, raw credential URL도 보존하지 않는다.
        */
        let credential_line = Self::read_refinedstone_credential_line(repo_root)?;
        Ok(Self::new(Self::parse_refinedstone_token(&credential_line)?))
    }

    /*
    현재 git branch에서 열린 PR을 찾는 discovery entrypoint다.

    repository full name은 origin remote에서, head branch는 현재 checkout에서 읽는다. detached HEAD, 빈 branch,
    base branch 자체는 review 대상 PR을 특정할 수 없으므로 `None`으로 접어 service가 polling을 건너뛰게 한다.
    */
    pub fn find_open_pull_request_for_current_branch(
        &self,
        repo_root: &Path,
        base_branch: &str,
    ) -> Result<Option<GithubPullRequestTarget>> {
        let repository = Self::resolve_repository_full_name(repo_root)?;
        let head_branch = Self::resolve_current_branch_name(repo_root)?;
        if head_branch == "HEAD" || head_branch.trim().is_empty() || head_branch == base_branch {
            return Ok(None);
        }

        self.find_open_pull_request_for_branch(&repository, &head_branch, base_branch)
    }
    fn find_open_pull_request_for_branch(
        &self,
        repository: &str,
        head_branch: &str,
        base_branch: &str,
    ) -> Result<Option<GithubPullRequestTarget>> {
        // GitHub pull request list API의 `head` filter는 `owner:branch` 형식이라 repo owner가 필요하다.
        // fork가 아닌 현재 repository branch만 찾는 정책이므로 repository full name의 owner를 그대로 사용한다.
        let owner = repository
            .split_once('/')
            .map(|(owner, _)| owner)
            .ok_or_else(|| anyhow!("failed to parse repository owner from {repository}"))?;
        let head = Self::encode_query_value(&format!("{owner}:{head_branch}"));
        let base = Self::encode_query_value(base_branch);
        let endpoint =
            format!("/repos/{repository}/pulls?state=open&head={head}&base={base}&per_page=1");
        let matches: Vec<PullRequestLocatorResponse> = self.fetch_object(&endpoint)?;
        Ok(matches
            .into_iter()
            .next()
            .map(|pull_request| GithubPullRequestTarget::new(repository, pull_request.number)))
    }
    fn resolve_git_dir(repo_root: &Path) -> Result<PathBuf> {
        Self::resolve_git_path(repo_root, "--git-dir", "git dir")
    }
    fn resolve_git_common_dir(repo_root: &Path) -> Result<PathBuf> {
        Self::resolve_git_path(repo_root, "--git-common-dir", "git common dir")
    }
    fn resolve_git_path(repo_root: &Path, flag: &str, label: &str) -> Result<PathBuf> {
        /*
        linked worktree에서는 `.git`이 directory가 아니라 pointer file일 수 있다.
        직접 path를 추측하면 credential lookup이 clone/worktree 형태마다 깨진다.
        `git rev-parse --path-format=absolute`로 git에게 canonical git dir 또는 common dir을 묻기 때문에
        일반 clone과 linked worktree 모두 같은 discovery path를 쓴다.
        */
        let path =
            Self::run_git_command(repo_root, &["rev-parse", "--path-format=absolute", flag])?;
        if path.is_empty() {
            bail!("resolved empty {label} from {}", repo_root.display());
        }
        Ok(PathBuf::from(path))
    }
    fn resolve_repository_full_name(repo_root: &Path) -> Result<String> {
        /*
        repository identity는 origin remote에서 얻는다.
        GitHub pull request API는 repository-scoped라 owner/repo path를 먼저 알아야 한다.
        이 lookup을 local git에 묶으면 어떤 installation을 검색해야 할지 모르는 상태에서 GitHub에 broad search를 하지 않아도 된다.
        */
        let origin_url = Self::run_git_command(repo_root, &["remote", "get-url", "origin"])?;
        Self::parse_repository_full_name(&origin_url)
    }
    fn resolve_current_branch_name(repo_root: &Path) -> Result<String> {
        /*
        current branch는 사용자의 review lane이다.
        detached HEAD는 `HEAD`를 반환하며, discovery entrypoint는 이를 non-pollable로 접는다.
        stable `owner:branch` head filter를 만들 수 없으면 어떤 PR activity를 가져와야 하는지 특정할 수 없다.
        */
        Self::run_git_command(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])
    }
    fn run_git_command(repo_root: &Path, args: &[&str]) -> Result<String> {
        /*
        git은 non-interactive command로 실행하고 stdout은 helper boundary에서 trim한다.
        이 helper 위쪽 caller는 repository/branch/credential 같은 domain-specific parse error를 붙이고,
        command 자체가 실패하면 stderr를 포함해 origin 설정이나 worktree 상태 문제를 진단할 수 있게 한다.
        */
        let output = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(args)
            .output()
            .with_context(|| {
                format!(
                    "failed to run git {} from {}",
                    args.join(" "),
                    repo_root.display()
                )
            })?;
        if !output.status.success() {
            bail!(
                "git {} failed from {}: {}",
                args.join(" "),
                repo_root.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
    fn parse_repository_full_name(origin_url: &str) -> Result<String> {
        // poller는 GitHub REST path의 `{owner}/{repo}`만 필요하므로 SSH/HTTPS origin을 같은 identity로 접는다.
        // 다른 hosting provider URL은 GitHub API로 안전하게 변환할 수 없어 명시적으로 거부한다.
        let repository = match origin_url {
            value if value.starts_with("git@github.com:") => value
                .trim_start_matches("git@github.com:")
                .trim_end_matches(".git")
                .to_string(),
            value if value.starts_with("https://github.com/") => value
                .trim_start_matches("https://github.com/")
                .trim_end_matches(".git")
                .to_string(),
            _ => bail!("unsupported GitHub origin URL {origin_url}"),
        };
        if repository.split('/').count() != 2 {
            bail!("failed to parse repository from {origin_url}");
        }
        Ok(repository)
    }
    fn read_refinedstone_credential_line(repo_root: &Path) -> Result<String> {
        /*
        credential 탐색 순서는 worktree와 WSL 사용 방식을 모두 반영한다.

        linked worktree에는 개별 git dir과 common git dir이 나뉠 수 있으므로 먼저 worktree-local credential을
        확인하고, 없으면 common dir을 확인한다. 마지막 Windows fallback은 WSL에서 repo-local credential이
        없더라도 Windows user home의 `.git-credentials`를 재사용하기 위한 경로다.
        */
        let credential_path = Self::resolve_git_dir(repo_root)?.join("refinedstone-credentials");
        if credential_path.is_file() {
            return Self::read_first_non_empty_line(&credential_path)
                .with_context(|| format!("failed to read {}", credential_path.display()));
        }
        let common_credential_path =
            Self::resolve_git_common_dir(repo_root)?.join("refinedstone-credentials");
        if common_credential_path != credential_path && common_credential_path.is_file() {
            return Self::read_first_non_empty_line(&common_credential_path)
                .with_context(|| format!("failed to read {}", common_credential_path.display()));
        }
        if let Some(credential_line) = Self::find_windows_refinedstone_credential_line()? {
            return Ok(credential_line);
        }
        let missing_paths = if credential_path == common_credential_path {
            credential_path.display().to_string()
        } else {
            format!(
                "{} and {}",
                credential_path.display(),
                common_credential_path.display()
            )
        };

        bail!(
            "missing {missing_paths} and no Windows RefinedStone credential was found in the current user's home directory"
        );
    }
    fn read_first_non_empty_line(path: &Path) -> Result<String> {
        /*
        credential file은 이 adapter 밖의 git/helper script가 관리하므로 trailing newline이나 빈 줄이 있을 수 있다.
        poller는 첫 non-empty line만 credential로 인정한다.
        RefinedStone credential contract는 lookup file 하나에 usable GitHub credential 한 줄만 둔다는 전제를 갖기 때문이다.
        */
        let contents = fs::read_to_string(path)?;
        contents
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| anyhow!("missing token line in {}", path.display()))
    }
    fn find_windows_refinedstone_credential_line() -> Result<Option<String>> {
        let Some(credential_path) = Self::resolve_windows_credential_path_for_current_user()?
        else {
            return Ok(None);
        };
        let contents = match fs::read_to_string(&credential_path) {
            Ok(contents) => contents,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                ) =>
            {
                return Ok(None);
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", credential_path.display()));
            }
        };
        // Windows credential file에는 여러 host credential이 섞일 수 있으므로 RefinedStone/GitHub line만 선택한다.
        Ok(contents.lines().map(str::trim).find_map(|line| {
            (line.starts_with("https://RefinedStone:") && line.contains("@github.com"))
                .then(|| line.to_string())
        }))
    }
    fn resolve_windows_credential_path_for_current_user() -> Result<Option<PathBuf>> {
        let users_root = Path::new(WINDOWS_USERS_ROOT);
        if !users_root.exists() {
            return Ok(None);
        }
        let Some(current_user) = Self::current_user_name() else {
            return Ok(None);
        };
        let Some(user_home) =
            Self::resolve_current_user_windows_home(users_root, current_user.as_str())?
        else {
            return Ok(None);
        };
        Ok(Some(user_home.join(".git-credentials")))
    }
    fn current_user_name() -> Option<String> {
        std::env::var("USER")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }
    fn resolve_current_user_windows_home(
        users_root: &Path,
        current_user: &str,
    ) -> Result<Option<PathBuf>> {
        /*
        WSL `$USER`는 보통 Windows profile directory 이름과 같지만 casing이 다를 수 있다.
        또한 `/mnt/c/Users` scan이 permission 때문에 막힐 수 있다.
        그래서 먼저 direct path 존재 여부를 기록하고, scan은 더 tolerant한 casing fallback으로만 사용한다.
        scan이 막혀도 direct match가 있으면 credential fallback을 계속 살릴 수 있다.
        */
        let direct_match = users_root.join(current_user);
        let direct_match_exists = direct_match.is_dir();
        let entries = match fs::read_dir(users_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                return Ok(direct_match_exists.then_some(direct_match));
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", users_root.display()));
            }
        };
        // Windows user directory casing은 WSL `$USER`와 다를 수 있으므로 case-insensitive scan으로 fallback을 보존한다.
        for entry in entries.flatten() {
            let entry_name = entry.file_name();
            if entry_name
                .to_string_lossy()
                .eq_ignore_ascii_case(current_user)
            {
                let path = entry.path();
                if path.is_dir() {
                    return Ok(Some(path));
                }
            }
        }
        if direct_match_exists {
            return Ok(Some(direct_match));
        }
        Ok(None)
    }
    fn parse_refinedstone_token(line: &str) -> Result<String> {
        // credential line은 `https://RefinedStone:<token>@github.com` 형태다. bearer token으로 쓰는 값은 password slot뿐이다.
        let token = line
            .strip_prefix("https://RefinedStone:")
            .and_then(|value| value.split_once("@github.com").map(|(token, _)| token))
            .ok_or_else(|| anyhow!("failed to parse RefinedStone token"))?;
        if token.trim().is_empty() {
            bail!("failed to parse RefinedStone token");
        }
        Ok(token.to_string())
    }
    fn encode_query_value(value: &str) -> String {
        /*
        GitHub의 head/base filter는 path가 아니라 query string에 들어간다.
        branch 이름에는 slash나 reserved character가 들어갈 수 있으므로 여기서만 percent-encode한다.
        endpoint path 자체는 이미 별도로 조립되므로 double-encoding하지 않는 것이 중요하다.
        */
        utf8_percent_encode(value, GITHUB_QUERY_ENCODE_SET).to_string()
    }
    fn fetch_pull_request_details(
        &self,
        target: &GithubPullRequestTarget,
    ) -> Result<PullRequestResponse> {
        self.fetch_object(&format!(
            "/repos/{}/pulls/{}",
            target.repository, target.number
        ))
    }
    fn fetch_pull_request_reviews(
        &self,
        target: &GithubPullRequestTarget,
    ) -> Result<Vec<PullRequestReviewResponse>> {
        self.fetch_paginated_array(&format!(
            "/repos/{}/pulls/{}/reviews",
            target.repository, target.number
        ))
    }
    fn fetch_review_comments(
        &self,
        target: &GithubPullRequestTarget,
    ) -> Result<Vec<PullRequestReviewCommentResponse>> {
        self.fetch_paginated_array(&format!(
            "/repos/{}/pulls/{}/comments",
            target.repository, target.number
        ))
    }
    fn fetch_issue_comments(
        &self,
        target: &GithubPullRequestTarget,
    ) -> Result<Vec<IssueCommentResponse>> {
        self.fetch_paginated_array(&format!(
            "/repos/{}/issues/{}/comments",
            target.repository, target.number
        ))
    }
    fn fetch_object<T>(&self, endpoint: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        /*
        Single-object endpoints still flow through fetch_json + parse_json so curl
        failure, HTTP status, and serde shape errors are reported with the same endpoint
        context as paginated array endpoints.
        */
        let body = self.fetch_json(endpoint)?;
        Self::parse_json(&body, endpoint)
    }
    fn fetch_paginated_array<T>(&self, endpoint: &str) -> Result<Vec<T>>
    where
        T: DeserializeOwned,
    {
        /*
        GitHub REST pagination stops when a page returns fewer than `PER_PAGE` items.

        This avoids depending on Link headers and keeps curl output parsing limited to JSON bodies. The tradeoff is one
        request per page, which is fine for PR review activity volumes and simpler to test with fixture JSON.
        */
        let mut items = Vec::new();
        let mut page = 1;
        loop {
            let page_endpoint = format!("{endpoint}?per_page={PER_PAGE}&page={page}");
            let body = self.fetch_json(&page_endpoint)?;
            let page_items: Vec<T> = Self::parse_json(&body, &page_endpoint)?;
            let count = page_items.len();
            items.extend(page_items);
            if count < PER_PAGE {
                return Ok(items);
            }

            page += 1;
        }
    }
    fn fetch_json(&self, endpoint: &str) -> Result<String> {
        /*
        curl is used instead of a persistent HTTP client so the adapter stays
        dependency-light and mirrors the repository's shell automation. The timeout
        flags are part of the TUI contract: review polling must fail with diagnostics
        instead of blocking the app-server loop indefinitely.
        */
        let url = format!("{}{}", self.api_base_url, endpoint);
        let authorization = format!("Authorization: Bearer {}", self.token);
        let user_agent = format!("User-Agent: {}", self.user_agent);
        let api_version = format!("X-GitHub-Api-Version: {}", GITHUB_API_VERSION);
        let output = Command::new(&self.curl_path)
            .args([
                "-sSfL",
                "--connect-timeout",
                CURL_CONNECT_TIMEOUT_SECONDS,
                "--max-time",
                CURL_MAX_TIME_SECONDS,
            ])
            .args(["-H", "Accept: application/vnd.github+json"])
            .args(["-H", api_version.as_str()])
            .args(["-H", authorization.as_str()])
            .args(["-H", user_agent.as_str()])
            .arg(&url)
            .output()
            .with_context(|| format!("failed to run {} for {url}", self.curl_path))?;
        if !output.status.success() {
            bail!(
                "github api request failed for {url}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
    fn parse_json<T>(body: &str, endpoint: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        serde_json::from_str(body)
            .with_context(|| format!("failed to parse GitHub response for {endpoint}"))
    }
    fn to_snapshot(
        target: &GithubPullRequestTarget,
        pull_request: PullRequestResponse,
        reviews: Vec<PullRequestReviewResponse>,
        review_comments: Vec<PullRequestReviewCommentResponse>,
        issue_comments: Vec<IssueCommentResponse>,
    ) -> GithubPullRequestActivitySnapshot {
        /*
        GitHub splits PR activity across review submissions, inline review comments,
        and issue comments. The domain service wants a single activity stream for
        polling diffs, so this adapter merges endpoint-specific DTOs first and sorts
        only after all event kinds have the same domain shape.
        */
        let mut events = reviews
            .into_iter()
            .filter_map(Self::to_review_event)
            .collect::<Vec<_>>();
        events.extend(
            review_comments
                .into_iter()
                .map(Self::to_review_comment_event)
                .collect::<Vec<_>>(),
        );
        events.extend(
            issue_comments
                .into_iter()
                .map(Self::to_issue_comment_event)
                .collect::<Vec<_>>(),
        );
        let mut snapshot = GithubPullRequestActivitySnapshot {
            target: target.clone(),
            title: pull_request.title,
            url: pull_request.html_url,
            head_branch: pull_request.head.ref_name,
            base_branch: pull_request.base.ref_name,
            events,
        };
        snapshot.sort_events();
        snapshot
    }
    fn to_review_event(
        review: PullRequestReviewResponse,
    ) -> Option<GithubPullRequestActivityEvent> {
        /*
        Pending reviews have no submitted timestamp and are not public review activity
        yet. Skipping them prevents the poller from announcing draft review state that
        the PR timeline itself would not show to the operator.
        */
        let submitted_at = review.submitted_at?;

        Some(GithubPullRequestActivityEvent {
            id: review.id,
            kind: GithubPullRequestActivityKind::Review,
            submitted_at,
            author_login: review
                .user
                .map(|user| user.login)
                .unwrap_or_else(|| "github".to_string()),
            body: review.body.unwrap_or_default(),
            state: Some(review.state),
            url: review.html_url,
            path: None,
        })
    }
    fn to_review_comment_event(
        comment: PullRequestReviewCommentResponse,
    ) -> GithubPullRequestActivityEvent {
        GithubPullRequestActivityEvent {
            id: comment.id,
            kind: GithubPullRequestActivityKind::ReviewComment,
            submitted_at: comment.updated_at,
            author_login: comment.user.login,
            body: comment.body,
            state: None,
            url: comment.html_url,
            path: Some(comment.path),
        }
    }
    fn to_issue_comment_event(comment: IssueCommentResponse) -> GithubPullRequestActivityEvent {
        GithubPullRequestActivityEvent {
            id: comment.id,
            kind: GithubPullRequestActivityKind::IssueComment,
            submitted_at: comment.updated_at,
            author_login: comment.user.login,
            body: comment.body,
            state: None,
            url: comment.html_url,
            path: None,
        }
    }
}
impl GithubReviewPollerPort for GithubReviewPollerAdapter {
    fn load_pull_request_activity(
        &self,
        target: &GithubPullRequestTarget,
    ) -> Result<GithubPullRequestActivitySnapshot> {
        /*
        Loading activity is intentionally a full refresh. The application service owns
        diffing against the previous snapshot, while this adapter keeps each poll
        stateless and reconstructs the authoritative PR timeline from GitHub.
        */
        let pull_request = self.fetch_pull_request_details(target)?;
        let reviews = self.fetch_pull_request_reviews(target)?;
        let review_comments = self.fetch_review_comments(target)?;
        let issue_comments = self.fetch_issue_comments(target)?;
        Ok(Self::to_snapshot(
            target,
            pull_request,
            reviews,
            review_comments,
            issue_comments,
        ))
    }
}

/*
Private GitHub REST response DTOs.

These structs intentionally model only the fields needed to construct the domain snapshot. Keeping them private prevents
GitHub's JSON names and nullable details from becoming application-layer contracts.
*/
#[derive(Debug, Clone, Deserialize)]
struct PullRequestResponse {
    title: String,
    html_url: String,
    head: PullRequestBranchRef,
    base: PullRequestBranchRef,
}
#[derive(Debug, Clone, Deserialize)]
struct PullRequestLocatorResponse {
    number: u64,
}
#[derive(Debug, Clone, Deserialize)]
struct PullRequestBranchRef {
    #[serde(rename = "ref")]
    ref_name: String,
}
#[derive(Debug, Clone, Deserialize)]
struct PullRequestReviewResponse {
    id: u64,
    body: Option<String>,
    state: String,
    submitted_at: Option<String>,
    html_url: String,
    user: Option<GitHubUser>,
}
#[derive(Debug, Clone, Deserialize)]
struct PullRequestReviewCommentResponse {
    id: u64,
    body: String,
    updated_at: String,
    html_url: String,
    path: String,
    user: GitHubUser,
}
#[derive(Debug, Clone, Deserialize)]
struct IssueCommentResponse {
    id: u64,
    body: String,
    updated_at: String,
    html_url: String,
    user: GitHubUser,
}
#[derive(Debug, Clone, Deserialize)]
struct GitHubUser {
    login: String,
}
#[cfg(test)]
mod tests;

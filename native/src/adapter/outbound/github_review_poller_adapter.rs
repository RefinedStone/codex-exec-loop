use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::domain::github_review::{
    GithubPullRequestActivityEvent, GithubPullRequestActivityKind,
    GithubPullRequestActivitySnapshot, GithubPullRequestTarget,
};

const GITHUB_API_BASE_URL: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2022-11-28";
const PER_PAGE: usize = 100;
const CURL_CONNECT_TIMEOUT_SECONDS: &str = "10";
const CURL_MAX_TIME_SECONDS: &str = "30";

pub struct GithubReviewPollerAdapter {
    curl_path: String,
    api_base_url: String,
    user_agent: String,
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
        let credential_path = Self::resolve_git_dir(repo_root)?.join("refinedstone-credentials");
        let line = fs::read_to_string(&credential_path)
            .with_context(|| format!("failed to read {}", credential_path.display()))?;
        let first_line = line
            .lines()
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("missing token line in {}", credential_path.display()))?;

        Ok(Self::new(Self::parse_refinedstone_token(first_line)?))
    }

    fn resolve_git_dir(repo_root: &Path) -> Result<PathBuf> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(["rev-parse", "--path-format=absolute", "--git-dir"])
            .output()
            .with_context(|| format!("failed to resolve git dir from {}", repo_root.display()))?;

        if !output.status.success() {
            bail!(
                "failed to resolve git dir from {}: {}",
                repo_root.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        let git_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if git_dir.is_empty() {
            bail!("resolved empty git dir from {}", repo_root.display());
        }

        Ok(PathBuf::from(git_dir))
    }

    fn parse_refinedstone_token(line: &str) -> Result<String> {
        let token = line
            .strip_prefix("https://RefinedStone:")
            .and_then(|value| value.split_once("@github.com").map(|(token, _)| token))
            .ok_or_else(|| anyhow!("failed to parse RefinedStone token"))?;

        if token.trim().is_empty() {
            bail!("failed to parse RefinedStone token");
        }

        Ok(token.to_string())
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
        let body = self.fetch_json(endpoint)?;
        Self::parse_json(&body, endpoint)
    }

    fn fetch_paginated_array<T>(&self, endpoint: &str) -> Result<Vec<T>>
    where
        T: DeserializeOwned,
    {
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

#[derive(Debug, Clone, Deserialize)]
struct PullRequestResponse {
    title: String,
    html_url: String,
    head: PullRequestBranchRef,
    base: PullRequestBranchRef,
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
mod tests {
    use super::{
        GithubReviewPollerAdapter, IssueCommentResponse, PullRequestResponse,
        PullRequestReviewCommentResponse, PullRequestReviewResponse,
    };
    use crate::domain::github_review::{GithubPullRequestActivityKind, GithubPullRequestTarget};

    #[test]
    fn parses_refinedstone_credential_lines() {
        let token = GithubReviewPollerAdapter::parse_refinedstone_token(
            "https://RefinedStone:abc123@github.com",
        )
        .expect("token should parse");

        assert_eq!(token, "abc123");
    }

    #[test]
    fn parses_pull_request_response_json() {
        let body = r#"{
            "title": "Add review polling",
            "html_url": "https://github.com/acme/widgets/pull/42",
            "head": { "ref": "feature/native-github-poller-port" },
            "base": { "ref": "prerelease" }
        }"#;

        let response: PullRequestResponse =
            GithubReviewPollerAdapter::parse_json(body, "/repos/acme/widgets/pulls/42")
                .expect("pull request response should parse");

        assert_eq!(response.title, "Add review polling");
        assert_eq!(response.head.ref_name, "feature/native-github-poller-port");
        assert_eq!(response.base.ref_name, "prerelease");
    }

    #[test]
    fn maps_and_sorts_review_activity_across_response_types() {
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let pull_request: PullRequestResponse = GithubReviewPollerAdapter::parse_json(
            r#"{
                    "title": "Add review polling",
                    "html_url": "https://github.com/acme/widgets/pull/42",
                    "head": { "ref": "feature/native-github-poller-port" },
                    "base": { "ref": "prerelease" }
                }"#,
            "/repos/acme/widgets/pulls/42",
        )
        .expect("pull request response should parse");
        let reviews: Vec<PullRequestReviewResponse> = GithubReviewPollerAdapter::parse_json(
            r#"[{
                "id": 500,
                "body": "Approved",
                "state": "APPROVED",
                "submitted_at": "2026-04-08T11:00:00Z",
                "html_url": "https://github.com/acme/widgets/pull/42#pullrequestreview-500",
                "user": { "login": "reviewer-a" }
            }]"#,
            "/repos/acme/widgets/pulls/42/reviews?page=1",
        )
        .expect("review page should parse");
        let review_comments: Vec<PullRequestReviewCommentResponse> =
            GithubReviewPollerAdapter::parse_json(
                r#"[{
                    "id": 300,
                    "body": "Please rename this field",
                    "updated_at": "2026-04-08T10:30:00Z",
                    "html_url": "https://github.com/acme/widgets/pull/42#discussion_r300",
                    "path": "native/src/application/service/github_review_poller_service.rs",
                    "user": { "login": "reviewer-b" }
                }]"#,
                "/repos/acme/widgets/pulls/42/comments?page=1",
            )
            .expect("review comment page should parse");
        let issue_comments: Vec<IssueCommentResponse> = GithubReviewPollerAdapter::parse_json(
            r#"[{
                "id": 200,
                "body": "Can you add a quick summary to the PR body?",
                "updated_at": "2026-04-08T10:30:00Z",
                "html_url": "https://github.com/acme/widgets/pull/42#issuecomment-200",
                "user": { "login": "reviewer-c" }
            }]"#,
            "/repos/acme/widgets/issues/42/comments?page=1",
        )
        .expect("issue comment page should parse");

        let snapshot = GithubReviewPollerAdapter::to_snapshot(
            &target,
            pull_request,
            reviews,
            review_comments,
            issue_comments,
        );

        assert_eq!(snapshot.events.len(), 3);
        assert_eq!(snapshot.events[0].id, 200);
        assert_eq!(
            snapshot.events[0].kind,
            GithubPullRequestActivityKind::IssueComment
        );
        assert_eq!(snapshot.events[1].id, 300);
        assert_eq!(
            snapshot.events[1].kind,
            GithubPullRequestActivityKind::ReviewComment
        );
        assert_eq!(snapshot.events[2].id, 500);
        assert_eq!(snapshot.events[2].state.as_deref(), Some("APPROVED"));
        assert_eq!(
            snapshot.events[1].path.as_deref(),
            Some("native/src/application/service/github_review_poller_service.rs")
        );
    }

    #[test]
    fn skips_pending_reviews_without_submitted_timestamp() {
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let pull_request: PullRequestResponse = GithubReviewPollerAdapter::parse_json(
            r#"{
                    "title": "Add review polling",
                    "html_url": "https://github.com/acme/widgets/pull/42",
                    "head": { "ref": "feature/native-github-poller-port" },
                    "base": { "ref": "prerelease" }
                }"#,
            "/repos/acme/widgets/pulls/42",
        )
        .expect("pull request response should parse");
        let reviews: Vec<PullRequestReviewResponse> = GithubReviewPollerAdapter::parse_json(
            r#"[{
                "id": 501,
                "body": "Still drafting",
                "state": "PENDING",
                "submitted_at": null,
                "html_url": "https://github.com/acme/widgets/pull/42#pullrequestreview-501",
                "user": { "login": "reviewer-a" }
            }]"#,
            "/repos/acme/widgets/pulls/42/reviews?page=1",
        )
        .expect("review page should parse");

        let snapshot = GithubReviewPollerAdapter::to_snapshot(
            &target,
            pull_request,
            reviews,
            Vec::new(),
            Vec::new(),
        );

        assert!(snapshot.events.is_empty());
    }
}

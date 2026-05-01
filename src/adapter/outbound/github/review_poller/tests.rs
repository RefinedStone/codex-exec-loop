use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    GithubReviewPollerAdapter, IssueCommentResponse, PullRequestLocatorResponse,
    PullRequestResponse, PullRequestReviewCommentResponse, PullRequestReviewResponse,
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
fn parses_repository_full_name_from_github_ssh_origin() {
    let repository =
        GithubReviewPollerAdapter::parse_repository_full_name("git@github.com:acme/widgets.git")
            .expect("repository should parse");

    assert_eq!(repository, "acme/widgets");
}

#[test]
fn parses_repository_full_name_from_github_https_origin() {
    let repository = GithubReviewPollerAdapter::parse_repository_full_name(
        "https://github.com/acme/widgets.git",
    )
    .expect("repository should parse");

    assert_eq!(repository, "acme/widgets");
}

#[test]
fn encodes_branch_head_filter_for_pull_request_lookup() {
    let encoded =
        GithubReviewPollerAdapter::encode_query_value("RefinedStone:feature/native-shell");

    assert_eq!(encoded, "RefinedStone%3Afeature%2Fnative-shell");
}

#[test]
fn resolves_windows_home_for_current_user_case_insensitively() {
    let users_root = unique_temp_dir("users-root");
    fs::create_dir_all(users_root.join("Akra")).expect("user home should be created");

    let resolved =
        GithubReviewPollerAdapter::resolve_current_user_windows_home(&users_root, "akra")
            .expect("user home lookup should succeed");

    assert_eq!(resolved, Some(users_root.join("Akra")));
    let _ = fs::remove_dir_all(&users_root);
}

#[test]
fn parses_pull_request_locator_response_json() {
    let body = r#"[{ "number": 64 }]"#;

    let response: Vec<PullRequestLocatorResponse> =
        GithubReviewPollerAdapter::parse_json(body, "/repos/acme/widgets/pulls")
            .expect("pull request locator response should parse");

    assert_eq!(response[0].number, 64);
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
                    "path": "src/application/service/github_review_poller_service.rs",
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
        Some("src/application/service/github_review_poller_service.rs")
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

fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"))
}

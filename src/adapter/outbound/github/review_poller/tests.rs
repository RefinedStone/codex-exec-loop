use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    GithubReviewPollerAdapter, IssueCommentResponse, PullRequestLocatorResponse,
    PullRequestResponse, PullRequestReviewCommentResponse, PullRequestReviewResponse,
};
use crate::domain::github_review::{GithubPullRequestActivityKind, GithubPullRequestTarget};

/*
review poller adapter는 GitHub REST API, local git origin, RefinedStone credential 위치를 domain
snapshot으로 바꾸는 outbound boundary다. 이 테스트 파일은 네트워크를 실제로 치지 않고도 "입력
문자열/JSON이 어떤 domain shape로 정규화되는가"를 고정한다.
*/

#[test]
fn parses_refinedstone_credential_lines() {
    // credential file에는 Git remote URL처럼 생긴 한 줄이 들어온다. adapter는 URL 전체를 저장하지
    // 않고 Basic-auth password 위치의 token만 curl bearer token으로 사용한다.
    let token = GithubReviewPollerAdapter::parse_refinedstone_token(
        "https://RefinedStone:abc123@github.com",
    )
    .expect("token should parse");

    assert_eq!(token, "abc123");
}

#[test]
fn parses_repository_full_name_from_github_ssh_origin() {
    // production repo origin은 SSH 형식일 수 있다. poller는 이 값을 GitHub API endpoint의
    // `{owner}/{repo}` segment로 바꿔야 PR lookup과 activity fetch를 수행할 수 있다.
    let repository =
        GithubReviewPollerAdapter::parse_repository_full_name("git@github.com:acme/widgets.git")
            .expect("repository should parse");

    assert_eq!(repository, "acme/widgets");
}

#[test]
fn parses_repository_full_name_from_github_https_origin() {
    // HTTPS origin도 같은 repository identity로 정규화한다. git transport 방식이 달라도 review polling
    // domain target은 같은 `owner/repo` 문자열이어야 한다.
    let repository = GithubReviewPollerAdapter::parse_repository_full_name(
        "https://github.com/acme/widgets.git",
    )
    .expect("repository should parse");

    assert_eq!(repository, "acme/widgets");
}

#[test]
fn encodes_branch_head_filter_for_pull_request_lookup() {
    // GitHub PR search의 `head` query는 `owner:branch` 형태인데 agent branch에는 slash가 들어간다.
    // 이 값을 percent-encode하지 않으면 branch lookup이 다른 query로 해석된다.
    let encoded =
        GithubReviewPollerAdapter::encode_query_value("RefinedStone:feature/native-shell");

    assert_eq!(encoded, "RefinedStone%3Afeature%2Fnative-shell");
}

#[test]
fn resolves_windows_home_for_current_user_case_insensitively() {
    // WSL 환경에서는 Windows user directory casing이 login casing과 다를 수 있다. credential fallback이
    // RefinedStone 토큰을 놓치지 않도록 user lookup을 case-insensitive로 유지한다.
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
    // branch-to-PR lookup endpoint는 PR number만 필요하다. locator response를 작게 유지하면 이후 full
    // PR fetch와 activity fetch를 명확히 분리할 수 있다.
    let body = r#"[{ "number": 64 }]"#;

    let response: Vec<PullRequestLocatorResponse> =
        GithubReviewPollerAdapter::parse_json(body, "/repos/acme/widgets/pulls")
            .expect("pull request locator response should parse");

    assert_eq!(response[0].number, 64);
}

#[test]
fn parses_pull_request_response_json() {
    // full PR response는 snapshot header를 구성하는 title/url/head/base만 추출한다. adapter가 GitHub
    // payload 전체에 결합되지 않도록 필요한 field subset만 deserialize한다.
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
    // GitHub는 PR conversation을 issue comments, review comments, review submissions 세 endpoint로
    // 나눠 제공한다. poller snapshot은 operator가 시간순으로 읽을 수 있도록 이 응답들을 하나의 activity
    // list로 합친다.
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
    // pending review는 아직 GitHub conversation에 공개되지 않은 draft 상태다. submitted_at이 없는
    // review를 snapshot에서 제외해야 operator에게 미공개 draft가 새 활동처럼 보이지 않는다.
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
    // Windows-home test는 실제 user directory를 건드리지 않아야 하므로 현재 epoch timestamp를 붙인
    // process-local temp root를 만들어 fixture 충돌을 피한다.
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"))
}

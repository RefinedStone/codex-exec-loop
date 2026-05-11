use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    GithubReviewPollerAdapter, IssueCommentResponse, PullRequestLocatorResponse,
    PullRequestResponse, PullRequestReviewCommentResponse, PullRequestReviewResponse,
};
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::domain::github_review::{GithubPullRequestActivityKind, GithubPullRequestTarget};

/*
review poller adapter는 GitHub REST API, local git origin, RefinedStone credential 위치를 domain
snapshot으로 바꾸는 outbound boundary다. 이 테스트 파일은 네트워크를 실제로 치지 않고도 "입력
문자열/JSON이 어떤 domain shape로 정규화되는가"를 고정한다.
*/

#[test]
fn parses_refinedstone_credential_lines() {
    // credential file에는 Git remote URL처럼 생긴 한 줄이 들어온다. adapter는 이 URL 전체를 저장하지 않고,
    // Basic-auth password 위치의 token만 curl bearer token으로 승격한다. 이 테스트는 credential format이 바뀌었을 때
    // token extraction boundary가 바로 실패하도록 고정한다.
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
    // SSH transport detail은 local git 경계에서 끝나고 domain target에는 repository identity만 남는다.
    let repository =
        GithubReviewPollerAdapter::parse_repository_full_name("git@github.com:acme/widgets.git")
            .expect("repository should parse");

    assert_eq!(repository, "acme/widgets");
}

#[test]
fn parses_repository_full_name_from_github_https_origin() {
    // HTTPS origin도 같은 repository identity로 정규화한다. git transport 방식이 달라도 review polling
    // domain target은 같은 `owner/repo` 문자열이어야 한다. 이 테스트는 SSH/HTTPS 차이가 GitHub REST path 조립으로
    // 새지 않게 하는 normalization contract를 잠근다.
    let repository = GithubReviewPollerAdapter::parse_repository_full_name(
        "https://github.com/acme/widgets.git",
    )
    .expect("repository should parse");

    assert_eq!(repository, "acme/widgets");
}

#[test]
fn encodes_branch_head_filter_for_pull_request_lookup() {
    // GitHub PR search의 `head` query는 `owner:branch` 형태인데 agent branch에는 slash가 들어간다.
    // 이 값을 percent-encode하지 않으면 branch lookup이 다른 query로 해석된다. path segment가 아니라 query value만
    // encode한다는 경계를 테스트한다.
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
    // PR fetch와 activity fetch를 명확히 분리할 수 있다. list endpoint의 DTO가 full PR payload에 결합되지 않게 하는 fixture다.
    let body = r#"[{ "number": 64 }]"#;

    let response: Vec<PullRequestLocatorResponse> =
        GithubReviewPollerAdapter::parse_json(body, "/repos/acme/widgets/pulls")
            .expect("pull request locator response should parse");

    assert_eq!(response[0].number, 64);
}

#[test]
fn parses_pull_request_response_json() {
    // full PR response는 snapshot header를 구성하는 title/url/head/base만 추출한다. adapter가 GitHub
    // payload 전체에 결합되지 않도록 필요한 field subset만 deserialize한다. GitHub가 다른 field를 추가해도 domain snapshot
    // contract가 커지지 않는지 확인한다.
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
    // list로 합친다. 동일 timestamp에서는 domain sort policy가 결정한 순서를 따르므로 endpoint별 수집 순서가 UI 순서가 되지 않는다.
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
    // review를 snapshot에서 제외해야 operator에게 미공개 draft가 새 활동처럼 보이지 않는다. 이 테스트는 GitHub nullable
    // timestamp가 domain event eligibility를 결정한다는 규칙을 고정한다.
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

#[test]
fn from_refinedstone_credentials_reads_repo_local_token() {
    // poller credential discovery는 git worktree의 actual git-dir을 기준으로 repo-local credential을 찾는다.
    // raw credential URL은 adapter 안에 보존하지 않고 bearer token만 남겨야 한다.
    let repo_root = init_git_repo("review-poller-credential");
    fs::write(
        repo_root.join(".git/refinedstone-credentials"),
        "\n  https://RefinedStone:repo-token-123@github.com  \n",
    )
    .expect("credential fixture should be written");

    let adapter = GithubReviewPollerAdapter::from_refinedstone_credentials(&repo_root)
        .expect("repo-local credential should initialize adapter");

    assert_eq!(adapter.token, "repo-token-123");
    assert_eq!(adapter.curl_path, "curl");
    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn read_first_non_empty_line_trims_blank_lines_and_rejects_empty_files() {
    // credential helper는 wrapper-managed file의 leading blank lines를 tolerate하지만,
    // usable token line이 없으면 silent empty token으로 진행하지 않는다.
    let root = unique_temp_dir("review-poller-line");
    fs::create_dir_all(&root).expect("fixture root should be created");
    let credential_path = root.join("refinedstone-credentials");
    fs::write(
        &credential_path,
        "\n \n  https://RefinedStone:file-token@github.com  \n",
    )
    .expect("credential fixture should be written");

    let line = GithubReviewPollerAdapter::read_first_non_empty_line(&credential_path)
        .expect("non-empty line should be found");

    assert_eq!(line, "https://RefinedStone:file-token@github.com");

    let empty_path = root.join("empty-credentials");
    fs::write(&empty_path, "\n  \n").expect("empty fixture should be written");
    let error = GithubReviewPollerAdapter::read_first_non_empty_line(&empty_path)
        .expect_err("blank credential file should fail");
    assert!(error.to_string().contains("missing token line"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn parse_helpers_reject_malformed_repository_and_credentials() {
    // unsupported remotes and empty token slots must fail before any GitHub request is built.
    // This keeps provider identity and credential shape errors on the outbound boundary.
    let unsupported = GithubReviewPollerAdapter::parse_repository_full_name(
        "https://gitlab.com/acme/widgets.git",
    )
    .expect_err("non-GitHub remotes should be rejected");
    assert!(
        unsupported
            .to_string()
            .contains("unsupported GitHub origin URL")
    );

    let malformed =
        GithubReviewPollerAdapter::parse_repository_full_name("https://github.com/acme.git")
            .expect_err("owner/repo path should be required");
    assert!(malformed.to_string().contains("failed to parse repository"));

    let empty_token =
        GithubReviewPollerAdapter::parse_refinedstone_token("https://RefinedStone:@github.com")
            .expect_err("empty token should be rejected");
    assert!(
        empty_token
            .to_string()
            .contains("failed to parse RefinedStone token")
    );
}

#[test]
fn find_current_branch_returns_none_for_base_branch_without_calling_github() {
    // current branch discovery uses local git first. When the operator is already on base,
    // the poller should skip PR lookup instead of issuing a broad GitHub search.
    let repo_root = init_git_repo("review-poller-base-branch");
    run_git(
        &repo_root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widgets.git",
        ],
    );
    run_git(&repo_root, &["checkout", "-b", "prerelease"]);
    let adapter = GithubReviewPollerAdapter::new("unused-token");

    let target = adapter
        .find_open_pull_request_for_current_branch(&repo_root, "prerelease")
        .expect("base branch should be a pollable local repo but not a PR target");

    assert_eq!(target, None);
    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn run_git_command_reports_git_stderr_for_invalid_repository() {
    // git failure context should retain stderr so the UI can explain broken origin/worktree state
    // instead of surfacing a bare exit code.
    let repo_root = unique_temp_dir("review-poller-not-git");
    fs::create_dir_all(&repo_root).expect("fixture root should be created");

    let error = GithubReviewPollerAdapter::run_git_command(
        &repo_root,
        &["rev-parse", "--abbrev-ref", "HEAD"],
    )
    .expect_err("git command should fail outside a repository");

    assert!(error.to_string().contains("git rev-parse"));
    assert!(
        error
            .to_string()
            .contains(repo_root.to_string_lossy().as_ref())
    );
    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn find_open_pull_request_for_branch_encodes_filters_and_maps_target() {
    // head/base filter 값은 query parameter라 slash와 spaces가 percent-encoded되어야 한다.
    // fake curl은 endpoint mismatch를 failure로 만들기 때문에 request construction까지 검증한다.
    let root = unique_temp_dir("review-poller-pr-lookup");
    fs::create_dir_all(&root).expect("fixture root should be created");
    fs::write(root.join("pulls.json"), r#"[{ "number": 77 }]"#)
        .expect("locator fixture should be written");
    let log_path = root.join("curl.log");
    let script = write_executable_script(
        &root,
        "fake-curl",
        &format!(
            r#"#!/bin/sh
set -eu
last=""
for arg in "$@"; do
  last="$arg"
done
printf '%s\n' "$@" >> "{log}"
case "$last" in
  *"/repos/acme/widgets/pulls?state=open&head=acme%3Afeature%2Freview%20gap&base=pre%2Frelease&per_page=1")
    cat "{root}/pulls.json"
    ;;
  *)
    echo "unexpected url: $last" >&2
    exit 64
    ;;
esac
"#,
            log = log_path.display(),
            root = root.display()
        ),
    );
    let adapter = fake_adapter(&script);

    let target = adapter
        .find_open_pull_request_for_branch("acme/widgets", "feature/review gap", "pre/release")
        .expect("locator response should parse")
        .expect("PR target should be found");

    assert_eq!(target, GithubPullRequestTarget::new("acme/widgets", 77));
    assert!(
        fs::read_to_string(&log_path)
            .expect("curl log should be readable")
            .contains("https://api.test/repos/acme/widgets/pulls?state=open&head=acme%3Afeature%2Freview%20gap&base=pre%2Frelease&per_page=1")
    );

    let error = adapter
        .find_open_pull_request_for_branch("malformed-repository", "feature/x", "prerelease")
        .expect_err("repository owner should be required");
    assert!(
        error
            .to_string()
            .contains("failed to parse repository owner")
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn fetch_paginated_array_reads_followup_pages_until_short_page() {
    // pagination stops only when GitHub returns fewer than PER_PAGE items. A full first page must
    // request page 2; otherwise old review activity can disappear from large PRs.
    let root = unique_temp_dir("review-poller-pagination");
    fs::create_dir_all(&root).expect("fixture root should be created");
    let page_one = (1..=100)
        .map(|number| format!(r#"{{ "number": {number} }}"#))
        .collect::<Vec<_>>()
        .join(",\n");
    fs::write(root.join("page-1.json"), format!("[{page_one}]"))
        .expect("page one fixture should be written");
    fs::write(root.join("page-2.json"), r#"[{ "number": 101 }]"#)
        .expect("page two fixture should be written");
    let log_path = root.join("curl.log");
    let script = write_executable_script(
        &root,
        "fake-curl",
        &format!(
            r#"#!/bin/sh
set -eu
last=""
for arg in "$@"; do
  last="$arg"
done
printf '%s\n' "$last" >> "{log}"
case "$last" in
  *"/repos/acme/widgets/pulls?per_page=100&page=1")
    cat "{root}/page-1.json"
    ;;
  *"/repos/acme/widgets/pulls?per_page=100&page=2")
    cat "{root}/page-2.json"
    ;;
  *)
    echo "unexpected url: $last" >&2
    exit 64
    ;;
esac
"#,
            log = log_path.display(),
            root = root.display()
        ),
    );
    let adapter = fake_adapter(&script);

    let items: Vec<PullRequestLocatorResponse> = adapter
        .fetch_paginated_array("/repos/acme/widgets/pulls")
        .expect("paginated locator response should parse");

    assert_eq!(items.len(), 101);
    assert_eq!(items[0].number, 1);
    assert_eq!(items[100].number, 101);
    let log = fs::read_to_string(&log_path).expect("curl log should be readable");
    assert!(log.contains("page=1"));
    assert!(log.contains("page=2"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn load_pull_request_activity_fetches_each_github_endpoint_through_curl_boundary() {
    // full activity load is a stateless refresh across PR details, reviews, review comments, and issue comments.
    // The fake curl script verifies request headers and returns endpoint-specific JSON without touching the network.
    let root = unique_temp_dir("review-poller-activity");
    fs::create_dir_all(&root).expect("fixture root should be created");
    fs::write(
        root.join("pull.json"),
        r#"{
            "title": "Review poller coverage",
            "html_url": "https://github.com/acme/widgets/pull/42",
            "head": { "ref": "feature/review-poller" },
            "base": { "ref": "prerelease" }
        }"#,
    )
    .expect("PR fixture should be written");
    fs::write(
        root.join("reviews.json"),
        r#"[{
            "id": 500,
            "body": null,
            "state": "COMMENTED",
            "submitted_at": "2026-04-08T11:00:00Z",
            "html_url": "https://github.com/acme/widgets/pull/42#pullrequestreview-500",
            "user": null
        }]"#,
    )
    .expect("reviews fixture should be written");
    fs::write(
        root.join("review-comments.json"),
        r#"[{
            "id": 300,
            "body": "Please tighten this test",
            "updated_at": "2026-04-08T10:30:00Z",
            "html_url": "https://github.com/acme/widgets/pull/42#discussion_r300",
            "path": "src/adapter/outbound/github/review_poller.rs",
            "user": { "login": "reviewer-b" }
        }]"#,
    )
    .expect("review comments fixture should be written");
    fs::write(
        root.join("issue-comments.json"),
        r#"[{
            "id": 200,
            "body": "Looks good overall",
            "updated_at": "2026-04-08T10:00:00Z",
            "html_url": "https://github.com/acme/widgets/pull/42#issuecomment-200",
            "user": { "login": "reviewer-c" }
        }]"#,
    )
    .expect("issue comments fixture should be written");
    let log_path = root.join("curl.log");
    let script = write_executable_script(
        &root,
        "fake-curl",
        &format!(
            r#"#!/bin/sh
set -eu
last=""
for arg in "$@"; do
  last="$arg"
done
printf '%s\n' "$@" >> "{log}"
case "$last" in
  "https://api.test/repos/acme/widgets/pulls/42")
    cat "{root}/pull.json"
    ;;
  *"/repos/acme/widgets/pulls/42/reviews?per_page=100&page=1")
    cat "{root}/reviews.json"
    ;;
  *"/repos/acme/widgets/pulls/42/comments?per_page=100&page=1")
    cat "{root}/review-comments.json"
    ;;
  *"/repos/acme/widgets/issues/42/comments?per_page=100&page=1")
    cat "{root}/issue-comments.json"
    ;;
  *)
    echo "unexpected url: $last" >&2
    exit 64
    ;;
esac
"#,
            log = log_path.display(),
            root = root.display()
        ),
    );
    let adapter = fake_adapter(&script);
    let target = GithubPullRequestTarget::new("acme/widgets", 42);

    let snapshot = adapter
        .load_pull_request_activity(&target)
        .expect("activity snapshot should load from fake curl");

    assert_eq!(snapshot.title, "Review poller coverage");
    assert_eq!(snapshot.head_branch, "feature/review-poller");
    assert_eq!(snapshot.base_branch, "prerelease");
    assert_eq!(snapshot.events.len(), 3);
    assert_eq!(snapshot.events[0].id, 200);
    assert_eq!(snapshot.events[1].id, 300);
    assert_eq!(snapshot.events[2].author_login, "github");
    assert_eq!(snapshot.events[2].body, "");
    assert_eq!(snapshot.events[2].state.as_deref(), Some("COMMENTED"));

    let log = fs::read_to_string(&log_path).expect("curl log should be readable");
    assert!(log.contains("Authorization: Bearer secret-token"));
    assert!(log.contains("User-Agent: akra-test"));
    assert!(log.contains("X-GitHub-Api-Version: 2022-11-28"));
    assert!(log.contains("--connect-timeout"));
    assert!(log.contains("--max-time"));
    assert!(log.contains("https://api.test/repos/acme/widgets/pulls/42"));
    assert!(
        log.contains("https://api.test/repos/acme/widgets/pulls/42/reviews?per_page=100&page=1")
    );
    assert!(
        log.contains("https://api.test/repos/acme/widgets/pulls/42/comments?per_page=100&page=1")
    );
    assert!(
        log.contains("https://api.test/repos/acme/widgets/issues/42/comments?per_page=100&page=1")
    );
    let _ = fs::remove_dir_all(&root);
}

fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
    // Windows-home test는 실제 user directory를 건드리지 않아야 하므로 현재 epoch timestamp를 붙인
    // process-local temp root를 만들어 fixture 충돌을 피한다. filesystem fallback 테스트가 host 환경을 오염시키지 않게 하는 helper다.
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"))
}

fn init_git_repo(prefix: &str) -> PathBuf {
    let repo_root = unique_temp_dir(prefix);
    fs::create_dir_all(&repo_root).expect("git fixture root should be created");
    let output = Command::new("git")
        .arg("init")
        .arg(&repo_root)
        .output()
        .expect("git init should run");
    assert!(
        output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    run_git(&repo_root, &["config", "user.name", "Akra Test"]);
    run_git(
        &repo_root,
        &["config", "user.email", "akra-test@example.com"],
    );
    run_git(&repo_root, &["commit", "--allow-empty", "-m", "initial"]);
    repo_root
}

fn run_git(repo_root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_executable_script(root: &Path, name: &str, body: &str) -> PathBuf {
    let script_path = root.join(name);
    fs::write(&script_path, body).expect("script fixture should be written");
    let mut permissions = fs::metadata(&script_path)
        .expect("script metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("script fixture should be executable");
    script_path
}

fn fake_adapter(curl_path: &Path) -> GithubReviewPollerAdapter {
    GithubReviewPollerAdapter {
        curl_path: curl_path.display().to_string(),
        api_base_url: "https://api.test".to_string(),
        user_agent: "akra-test".to_string(),
        token: "secret-token".to_string(),
    }
}

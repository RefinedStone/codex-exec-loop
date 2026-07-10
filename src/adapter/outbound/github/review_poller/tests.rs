use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    GithubReviewPollerAdapter, IssueCommentResponse, PullRequestLocatorResponse,
    PullRequestResponse, PullRequestReviewCommentResponse, PullRequestReviewResponse,
};
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::domain::github_review::{GithubPullRequestActivityKind, GithubPullRequestTarget};

/*
review poller adapter는 GitHub REST API, configured git push remote, local GitHub credential 위치를 domain
snapshot으로 바꾸는 outbound boundary다. 이 테스트 파일은 네트워크를 실제로 치지 않고도 "입력
문자열/JSON이 어떤 domain shape로 정규화되는가"를 고정한다.
*/

#[test]
fn curl_spawn_retry_error_classifier_is_narrow() {
    assert!(super::is_transient_curl_spawn_error(
        &io::Error::from_raw_os_error(26)
    ));
    assert!(!super::is_transient_curl_spawn_error(
        &io::Error::from_raw_os_error(2)
    ));
}

#[test]
fn parses_github_credential_lines() {
    // credential file에는 Git remote URL처럼 생긴 한 줄이 들어온다. adapter는 이 URL 전체를 저장하지 않고,
    // Basic-auth password 위치의 token만 curl bearer token으로 승격한다. 이 테스트는 credential format이 바뀌었을 때
    // token extraction boundary가 바로 실패하도록 고정한다.
    let token = GithubReviewPollerAdapter::parse_github_credential_token(
        "https://octo-user:abc123@github.com",
    )
    .expect("token should parse");

    assert_eq!(token, "abc123");
}

#[test]
fn local_credential_constructor_prefers_trimmed_environment_token() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let _env = EnvVarGuard::apply(&[
        ("AKRA_GITHUB_TOKEN", Some("  env-token-123  ")),
        ("GH_TOKEN", Some("ignored-gh-token")),
        ("GITHUB_TOKEN", Some("ignored-github-token")),
        ("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN", None),
    ]);

    let adapter = GithubReviewPollerAdapter::from_local_github_credentials(Path::new("."))
        .expect("environment token should build adapter");

    assert_eq!(adapter.token, "env-token-123");
}

#[test]
fn local_credential_constructor_rejects_removed_legacy_scan_even_when_a_named_file_exists() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let _env = EnvVarGuard::apply(&[
        ("AKRA_GITHUB_TOKEN", None),
        ("GH_TOKEN", None),
        ("GITHUB_TOKEN", None),
        ("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN", Some("1")),
    ]);
    let fake_gh_root = unique_temp_dir("review-poller-from-local-named-failed-gh");
    fs::create_dir_all(&fake_gh_root).expect("fake gh root should be created");
    let gh_program = write_executable_script(
        &fake_gh_root,
        "gh",
        r#"#!/bin/sh
set -eu
exit 2
"#,
    );
    let repo_root = init_git_repo("review-poller-from-local-named-credential");
    run_git(&repo_root, &["config", "credential.helper", ""]);
    fs::write(
        repo_root.join(".git/akra-github-credentials"),
        "named-token-123\n",
    )
    .expect("named credential fixture should be written");

    let error =
        GithubReviewPollerAdapter::read_local_github_token_with_gh_program(&repo_root, &gh_program)
            .expect_err("removed legacy credential scanning must fail closed");

    assert!(error.to_string().contains("no longer supported"));
    assert!(!error.to_string().contains("named-token-123"));
    let _ = fs::remove_dir_all(&fake_gh_root);
    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn local_credential_constructor_does_not_scan_named_files_by_default() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let _env = EnvVarGuard::apply(&[
        ("AKRA_GITHUB_TOKEN", None),
        ("GH_TOKEN", None),
        ("GITHUB_TOKEN", None),
        ("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN", None),
    ]);
    let fake_gh_root = unique_temp_dir("review-poller-default-no-direct-scan-gh");
    fs::create_dir_all(&fake_gh_root).expect("fake gh root should be created");
    let gh_program = write_executable_script(
        &fake_gh_root,
        "gh",
        r#"#!/bin/sh
set -eu
exit 2
"#,
    );
    let repo_root = init_git_repo("review-poller-default-no-direct-scan");
    run_git(&repo_root, &["config", "credential.helper", ""]);
    fs::write(
        repo_root.join(".git/akra-github-credentials"),
        "must-not-be-read\n",
    )
    .expect("named credential fixture should be written");

    let error = match GithubReviewPollerAdapter::read_local_github_token_with_gh_program(
        &repo_root,
        &gh_program,
    ) {
        Ok(_) => panic!("direct credential files must be ignored without explicit opt-in"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("trusted gh auth was unavailable")
    );
    assert!(
        error
            .to_string()
            .contains("direct credential-file scanning")
    );
    assert!(!error.to_string().contains("must-not-be-read"));
    let _ = fs::remove_dir_all(&fake_gh_root);
    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn local_credential_constructor_rejects_any_removed_legacy_scan_value_without_echoing_it() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let secret_value = "not-a-boolean-secret-value";
    let _env = EnvVarGuard::apply(&[
        ("AKRA_GITHUB_TOKEN", Some("token-must-not-be-echoed")),
        ("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN", Some(secret_value)),
    ]);

    let error = match GithubReviewPollerAdapter::from_local_github_credentials(Path::new(
        "/sensitive/credential/path-must-not-be-echoed",
    )) {
        Ok(_) => panic!("removed legacy scan configuration must fail closed"),
        Err(error) => error,
    };
    let message = error.to_string();

    assert!(message.contains("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN"));
    assert!(message.contains("no longer supported"));
    assert!(!message.contains(secret_value));
    assert!(!message.contains("token-must-not-be-echoed"));
    assert!(!message.contains("path-must-not-be-echoed"));
}

#[test]
fn local_credential_constructor_falls_back_to_gh_auth_token() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let _env = EnvVarGuard::apply(&[
        ("AKRA_GITHUB_TOKEN", None),
        ("GH_TOKEN", None),
        ("GITHUB_TOKEN", None),
        ("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN", None),
    ]);
    let root = unique_temp_dir("review-poller-from-local-gh-token");
    fs::create_dir_all(&root).expect("fixture root should be created");
    let gh_program = write_executable_script(
        &root,
        "gh",
        r#"#!/bin/sh
set -eu
printf 'gh-local-token\n'
"#,
    );
    let token =
        GithubReviewPollerAdapter::read_local_github_token_with_gh_program(&root, &gh_program)
            .expect("gh auth token should load");
    let adapter = GithubReviewPollerAdapter::new(token);

    assert_eq!(adapter.token, "gh-local-token");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn local_credential_constructor_never_executes_repository_credential_helper() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let _env = EnvVarGuard::apply(&[
        ("AKRA_GITHUB_TOKEN", None),
        ("GH_TOKEN", None),
        ("GITHUB_TOKEN", None),
        ("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN", None),
    ]);
    let fake_gh_root = unique_temp_dir("review-poller-from-local-failed-gh");
    fs::create_dir_all(&fake_gh_root).expect("fake gh root should be created");
    let gh_program = write_executable_script(
        &fake_gh_root,
        "gh",
        r#"#!/bin/sh
set -eu
exit 2
"#,
    );
    let repo_root = init_git_repo("review-poller-from-local-git-credential");
    let marker_path = repo_root.join("credential-helper-ran");
    run_git(
        &repo_root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widgets.git",
        ],
    );
    run_git(
        &repo_root,
        &[
            "config",
            "credential.helper",
            &format!(
                "!f() {{ : > '{}'; printf 'username=octo\\npassword=must-not-load\\n'; }}; f",
                marker_path.display()
            ),
        ],
    );

    let error =
        GithubReviewPollerAdapter::read_local_github_token_with_gh_program(&repo_root, &gh_program)
            .expect_err("repository credential helpers must not participate in token discovery");

    assert!(
        error
            .to_string()
            .contains("trusted gh auth was unavailable")
    );
    assert!(
        !marker_path.exists(),
        "credential.helper command was executed"
    );
    let _ = fs::remove_dir_all(&fake_gh_root);
    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn repository_path_cannot_replace_review_poller_gh_or_curl() {
    let repo_root = init_git_repo("review-poller-hostile-path");
    let repo_bin = repo_root.join("bin");
    fs::create_dir_all(&repo_bin).expect("repository bin should be created");
    let marker_path = repo_root.join("hostile-program-ran");
    let script = format!(
        "#!/bin/sh\nset -eu\n: > '{}'\nprintf 'attacker-token\\n'\n",
        marker_path.display()
    );
    write_executable_script(&repo_bin, "gh", &script);
    write_executable_script(&repo_bin, "curl", &script);
    let original_path = std::env::var_os("PATH").expect("PATH should be available");
    let hostile_path = std::env::join_paths(
        std::iter::once(repo_bin.clone()).chain(std::env::split_paths(&original_path)),
    )
    .expect("hostile PATH should join");
    let token_error =
        crate::trusted_executable::resolve_native_from_path("gh", &hostile_path, &repo_root)
            .expect_err("repository gh must make trusted discovery fail closed");
    assert!(
        token_error
            .to_string()
            .contains("PATH selected an unsafe `gh` executable")
    );

    let adapter = GithubReviewPollerAdapter::new_for_workspace_with_path(
        "host-token",
        &repo_root,
        &hostile_path,
    );
    assert!(adapter.curl_resolution_error.is_some());
    let request_error = adapter
        .fetch_object::<serde_json::Value>("/rate_limit")
        .expect_err("repository curl must make requests fail closed");
    assert!(
        request_error
            .to_string()
            .contains("trusted curl executable")
    );
    assert!(!marker_path.exists(), "repository gh or curl was executed");
    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn parses_repository_full_name_from_github_ssh_origin() {
    // production repo remote는 SSH 형식일 수 있다. poller는 이 값을 GitHub API endpoint의
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
fn parses_repository_full_name_from_github_credentialed_https_origin() {
    let repository = GithubReviewPollerAdapter::parse_repository_full_name(
        "https://greg:ghp_secret_token@github.com/acme/widgets.git",
    )
    .expect("credentialed HTTPS repository should parse");

    assert_eq!(repository, "acme/widgets");
}

#[test]
fn repository_parse_errors_do_not_echo_credentialed_remote_urls() {
    let sensitive_origin =
        "https://greg:token-must-not-be-echoed@github.com/acme/widgets/extra.git";

    let error = GithubReviewPollerAdapter::parse_repository_full_name(sensitive_origin)
        .expect_err("multi-segment repository identity should be rejected");
    let message = error.to_string();

    assert!(message.contains("failed to parse GitHub repository identity"));
    assert!(!message.contains("token-must-not-be-echoed"));
    assert!(!message.contains(sensitive_origin));
}

#[test]
fn parses_repository_full_name_from_github_ssh_protocol_origin() {
    let repository = GithubReviewPollerAdapter::parse_repository_full_name(
        "ssh://git@github.com/acme/widgets.git",
    )
    .expect("ssh protocol repository should parse");

    assert_eq!(repository, "acme/widgets");
}

#[test]
fn encodes_branch_head_filter_for_pull_request_lookup() {
    // GitHub PR search의 `head` query는 `owner:branch` 형태인데 agent branch에는 slash가 들어간다.
    // 이 값을 percent-encode하지 않으면 branch lookup이 다른 query로 해석된다. path segment가 아니라 query value만
    // encode한다는 경계를 테스트한다.
    let encoded = GithubReviewPollerAdapter::encode_query_value("acme:feature/native-shell");

    assert_eq!(encoded, "acme%3Afeature%2Fnative-shell");
}

#[test]
fn resolves_windows_home_for_current_user_case_insensitively() {
    // WSL 환경에서는 Windows user directory casing이 login casing과 다를 수 있다. credential fallback이
    // GitHub token을 놓치지 않도록 user lookup을 case-insensitive로 유지한다.
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
fn named_github_credentials_read_repo_local_token() {
    // poller credential discovery는 git worktree의 actual git-dir을 기준으로 repo-local credential을 찾는다.
    // raw credential URL은 adapter 안에 보존하지 않고 bearer token만 남겨야 한다.
    let repo_root = init_git_repo("review-poller-credential");
    fs::write(
        repo_root.join(".git/akra-github-credentials"),
        "\n  https://octo-user:repo-token-123@github.com  \n",
    )
    .expect("credential fixture should be written");

    let token = GithubReviewPollerAdapter::read_named_github_credential_token(&repo_root)
        .expect("repo-local credential lookup should not fail")
        .expect("repo-local credential should be found");

    assert_eq!(token, "repo-token-123");
    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn read_first_non_empty_line_trims_blank_lines_and_rejects_empty_files() {
    // credential helper는 wrapper-managed file의 leading blank lines를 tolerate하지만,
    // usable token line이 없으면 silent empty token으로 진행하지 않는다.
    let root = unique_temp_dir("review-poller-line");
    fs::create_dir_all(&root).expect("fixture root should be created");
    let credential_path = root.join("akra-github-credentials");
    fs::write(
        &credential_path,
        "\n \n  https://octo-user:file-token@github.com  \n",
    )
    .expect("credential fixture should be written");

    let line = GithubReviewPollerAdapter::read_first_non_empty_line(&credential_path)
        .expect("non-empty line should be found");

    assert_eq!(line, "https://octo-user:file-token@github.com");

    let empty_path = root.join("empty-credentials");
    fs::write(&empty_path, "\n  \n").expect("empty fixture should be written");
    let error = GithubReviewPollerAdapter::read_first_non_empty_line(&empty_path)
        .expect_err("blank credential file should fail");
    assert!(
        error
            .to_string()
            .contains("legacy GitHub credential file has no usable token line")
    );
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
            .contains("unsupported GitHub remote URL")
    );

    let malformed =
        GithubReviewPollerAdapter::parse_repository_full_name("https://github.com/acme.git")
            .expect_err("owner/repo path should be required");
    assert!(
        malformed
            .to_string()
            .contains("failed to parse GitHub repository identity")
    );

    let empty_token =
        GithubReviewPollerAdapter::parse_github_credential_token("https://octo-user:@github.com")
            .expect_err("empty token should be rejected");
    assert!(
        empty_token
            .to_string()
            .contains("failed to parse GitHub credential token")
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
fn find_current_branch_resolves_repository_branch_and_maps_open_pull_request() {
    // Non-base local branches go through origin parsing, branch lookup, and the same GitHub locator
    // request builder used by explicit branch lookup.
    let repo_root = init_git_repo("review-poller-current-branch");
    run_git(
        &repo_root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widgets.git",
        ],
    );
    run_git(&repo_root, &["checkout", "-b", "feature/review-poller"]);
    let root = unique_temp_dir("review-poller-current-branch-curl");
    fs::create_dir_all(&root).expect("fixture root should be created");
    fs::write(root.join("pulls.json"), r#"[{ "number": 88 }]"#)
        .expect("locator fixture should be written");
    let script = write_executable_script(
        &root,
        "fake-curl",
        &format!(
            r#"#!/bin/sh
set -eu
cat >/dev/null

last=""
for arg in "$@"; do
  last="$arg"
done
case "$last" in
  *"/repos/acme/widgets/pulls?state=open&head=acme%3Afeature%2Freview-poller&base=prerelease&per_page=1")
    cat "{root}/pulls.json"
    ;;
  *)
    echo "unexpected url: $last" >&2
    exit 64
    ;;
esac
"#,
            root = root.display()
        ),
    );
    let adapter = fake_adapter(&script);

    let target = adapter
        .find_open_pull_request_for_current_branch(&repo_root, "prerelease")
        .expect("current branch lookup should query fake GitHub")
        .expect("PR target should be found");

    assert_eq!(target, GithubPullRequestTarget::new("acme/widgets", 88));
    let _ = fs::remove_dir_all(&repo_root);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn find_current_branch_uses_configured_upstream_without_origin() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let _env = EnvVarGuard::apply(&[("AKRA_GITHUB_PUSH_REMOTE", None)]);
    let repo_root = init_git_repo("review-poller-configured-upstream");
    run_git(
        &repo_root,
        &[
            "remote",
            "add",
            "upstream",
            "https://github.com/acme/widgets.git",
        ],
    );
    run_git(&repo_root, &["config", "akra.githubPushRemote", "upstream"]);

    let repository = GithubReviewPollerAdapter::resolve_repository_full_name(&repo_root)
        .expect("configured upstream should resolve without an origin remote");

    assert_eq!(repository, "acme/widgets");
    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn configured_upstream_prevents_fork_origin_repository_mismatch() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let _env = EnvVarGuard::apply(&[("AKRA_GITHUB_PUSH_REMOTE", None)]);
    let repo_root = init_git_repo("review-poller-fork-origin-mismatch");
    run_git(
        &repo_root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/fork-owner/widgets.git",
        ],
    );
    run_git(
        &repo_root,
        &[
            "remote",
            "add",
            "upstream",
            "https://github.com/acme/widgets.git",
        ],
    );
    run_git(&repo_root, &["config", "akra.githubPushRemote", "upstream"]);

    let repository = GithubReviewPollerAdapter::resolve_repository_full_name(&repo_root)
        .expect("review polling should use the configured delivery remote");

    assert_eq!(repository, "acme/widgets");
    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn missing_or_invalid_push_remote_never_falls_back_to_origin() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let _env = EnvVarGuard::apply(&[("AKRA_GITHUB_PUSH_REMOTE", None)]);
    let repo_root = init_git_repo("review-poller-missing-push-remote");
    run_git(
        &repo_root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/fork-owner/widgets.git",
        ],
    );
    run_git(&repo_root, &["config", "akra.githubPushRemote", "upstream"]);

    let missing = GithubReviewPollerAdapter::resolve_repository_full_name(&repo_root)
        .expect_err("a missing configured remote must not fall back to origin");
    assert!(
        missing
            .to_string()
            .contains("push remote `upstream` is not available")
    );

    run_git(
        &repo_root,
        &["config", "akra.githubPushRemote", "../origin"],
    );
    let invalid = GithubReviewPollerAdapter::resolve_repository_full_name(&repo_root)
        .expect_err("an invalid configured remote must not fall back to origin");
    assert!(
        invalid
            .to_string()
            .contains("akra.githubPushRemote is invalid")
    );
    assert!(!invalid.to_string().contains("../origin"));
    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn default_origin_must_exist_for_review_repository_discovery() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let _env = EnvVarGuard::apply(&[("AKRA_GITHUB_PUSH_REMOTE", None)]);
    let repo_root = init_git_repo("review-poller-no-origin");

    let error = GithubReviewPollerAdapter::resolve_repository_full_name(&repo_root)
        .expect_err("missing default origin should fail closed");

    assert!(
        error
            .to_string()
            .contains("push remote `origin` is not available")
    );
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
fn gh_auth_token_reader_uses_cli_output_and_ignores_empty_or_failed_status() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let root = unique_temp_dir("review-poller-gh-auth-token");
    fs::create_dir_all(&root).expect("fixture root should be created");
    let mut gh_program = write_executable_script(
        &root,
        "gh",
        r#"#!/bin/sh
set -eu
printf '  gh-token-123  \n'
"#,
    );
    let token = GithubReviewPollerAdapter::read_gh_auth_token_with_program(
        &root,
        &gh_program,
        std::time::Duration::from_secs(1),
    )
    .expect("gh auth token command should be handled");
    assert_eq!(token.as_deref(), Some("gh-token-123"));

    gh_program = write_executable_script(
        &root,
        "gh",
        r#"#!/bin/sh
set -eu
printf '\n'
"#,
    );
    let empty = GithubReviewPollerAdapter::read_gh_auth_token_with_program(
        &root,
        &gh_program,
        std::time::Duration::from_secs(1),
    )
    .expect("empty gh token should not fail");
    assert_eq!(empty, None);

    gh_program = write_executable_script(
        &root,
        "gh",
        r#"#!/bin/sh
set -eu
exit 2
"#,
    );
    let failed = GithubReviewPollerAdapter::read_gh_auth_token_with_program(
        &root,
        &gh_program,
        std::time::Duration::from_secs(1),
    )
    .expect("failed gh token command should be ignored");
    assert_eq!(failed, None);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn named_credentials_cover_plain_token_missing_files_and_common_dir_fallback() {
    let missing_repo = init_git_repo("review-poller-no-named-credential");
    let missing = GithubReviewPollerAdapter::read_named_github_credential_token(&missing_repo)
        .expect("missing credential lookup should not fail");
    assert_eq!(missing, None);
    let _ = fs::remove_dir_all(&missing_repo);

    let plain_repo = init_git_repo("review-poller-plain-named-credential");
    fs::write(
        plain_repo.join(".git/github-credentials"),
        "\n plain-token-123 \n",
    )
    .expect("plain credential fixture should be written");
    let plain = GithubReviewPollerAdapter::read_named_github_credential_token(&plain_repo)
        .expect("plain credential lookup should not fail");
    assert_eq!(plain.as_deref(), Some("plain-token-123"));
    let _ = fs::remove_dir_all(&plain_repo);

    let common_repo = init_git_repo("review-poller-common-named-credential");
    let linked_worktree = unique_temp_dir("review-poller-linked-worktree");
    run_git(
        &common_repo,
        &["worktree", "add", path_str(&linked_worktree), "HEAD"],
    );
    let common_dir = GithubReviewPollerAdapter::resolve_git_common_dir(&linked_worktree)
        .expect("linked worktree common dir should resolve");
    fs::write(
        common_dir.join("refinedstone-credentials"),
        "common-token-123\n",
    )
    .expect("common credential fixture should be written");

    let common = GithubReviewPollerAdapter::read_named_github_credential_token(&linked_worktree)
        .expect("common credential lookup should not fail");

    assert_eq!(common.as_deref(), Some("common-token-123"));
    let _ = fs::remove_dir_all(&common_repo);
    let _ = fs::remove_dir_all(&linked_worktree);
}

#[test]
fn git_credential_files_cover_home_fallback() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let home_root = unique_temp_dir("review-poller-home-git-credentials");
    fs::create_dir_all(&home_root).expect("home credential fixture root should exist");
    fs::write(
        home_root.join(".git-credentials"),
        "https://ci:home-token-123@github.com\n",
    )
    .expect("home git credential fixture should be written");
    let _env = EnvVarGuard::apply(&[("HOME", Some(home_root.to_string_lossy().as_ref()))]);

    let token = GithubReviewPollerAdapter::read_git_credential_file_token_for_root(Path::new(
        "/mnt/c/Users",
    ))
    .expect("home git credential lookup should not fail");

    assert_eq!(token.as_deref(), Some("home-token-123"));
    let _ = fs::remove_dir_all(&home_root);
}

#[test]
fn windows_git_credential_candidates_stay_on_current_user_profile() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let users_root = unique_temp_dir("review-poller-windows-credential-candidates");
    let home_root = unique_temp_dir("review-poller-home-credential-priority");
    let userprofile_root = unique_temp_dir("review-poller-userprofile-credential-priority");
    let alice_home = users_root.join("Alice");
    let linux_home = users_root.join("linux-akra");
    let akra_home = users_root.join("akra");
    fs::create_dir_all(&alice_home).expect("other Windows home should exist");
    fs::create_dir_all(&linux_home).expect("wrong USER Windows home should exist");
    fs::create_dir_all(&akra_home).expect("current Windows home should exist");
    fs::create_dir_all(&home_root).expect("home credential root should exist");
    fs::create_dir_all(&userprofile_root).expect("userprofile credential root should exist");
    fs::write(
        home_root.join(".git-credentials"),
        "https://home:home-token-123@github.com\n",
    )
    .expect("home credential fixture should be written");
    fs::write(
        userprofile_root.join(".git-credentials"),
        "https://userprofile:userprofile-token-123@github.com\n",
    )
    .expect("userprofile credential fixture should be written");
    fs::write(
        alice_home.join(".git-credentials"),
        "https://alice:alice-token-123@github.com\n",
    )
    .expect("other-user credential fixture should be written");
    fs::write(
        linux_home.join(".git-credentials"),
        "https://linux-akra:linux-token-123@github.com\n",
    )
    .expect("wrong-user credential fixture should be written");
    fs::write(
        akra_home.join(".git-credentials"),
        "https://akra:akra-token-123@github.com\n",
    )
    .expect("current-user credential fixture should be written");
    let _env = EnvVarGuard::apply(&[
        ("HOME", Some(home_root.to_string_lossy().as_ref())),
        (
            "USERPROFILE",
            Some(userprofile_root.to_string_lossy().as_ref()),
        ),
        ("USER", Some("linux-akra")),
        ("USERNAME", Some("akra")),
    ]);

    let candidates =
        GithubReviewPollerAdapter::git_credential_file_candidates_for_root(&users_root)
            .expect("candidate lookup should not fail");
    assert_eq!(
        candidates,
        vec![
            akra_home.join(".git-credentials"),
            home_root.join(".git-credentials"),
            userprofile_root.join(".git-credentials"),
        ]
    );

    let token =
        GithubReviewPollerAdapter::read_git_credential_file_token_from_candidates(candidates)
            .expect("current-user credential lookup should not fail");
    assert_eq!(token.as_deref(), Some("akra-token-123"));
    let _ = fs::remove_dir_all(&users_root);
    let _ = fs::remove_dir_all(&home_root);
    let _ = fs::remove_dir_all(&userprofile_root);
}

#[test]
fn local_credential_constructor_does_not_restore_windows_file_scanning_with_legacy_flag() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let home_root = unique_temp_dir("review-poller-from-local-home-credential");
    let userprofile_root = unique_temp_dir("review-poller-from-local-userprofile-credential");
    fs::create_dir_all(&home_root).expect("home credential root should exist");
    fs::create_dir_all(&userprofile_root).expect("userprofile credential root should exist");
    fs::write(
        home_root.join(".git-credentials"),
        "https://home:home-token-123@github.com\n",
    )
    .expect("home credential fixture should be written");
    fs::write(
        userprofile_root.join(".git-credentials"),
        "https://userprofile:userprofile-token-123@github.com\n",
    )
    .expect("userprofile credential fixture should be written");
    let _env = EnvVarGuard::apply(&[
        ("AKRA_GITHUB_TOKEN", None),
        ("GH_TOKEN", None),
        ("GITHUB_TOKEN", None),
        ("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN", Some("1")),
        ("HOME", Some(home_root.to_string_lossy().as_ref())),
        (
            "USERPROFILE",
            Some(userprofile_root.to_string_lossy().as_ref()),
        ),
        ("USER", Some("linux-akra")),
        ("USERNAME", Some("akra")),
    ]);
    let fake_gh_root = unique_temp_dir("review-poller-from-local-windows-failed-gh");
    fs::create_dir_all(&fake_gh_root).expect("fake gh root should be created");
    let gh_program = write_executable_script(
        &fake_gh_root,
        "gh",
        r#"#!/bin/sh
set -eu
exit 2
"#,
    );
    let repo_root = init_git_repo("review-poller-from-local-windows-credential");
    run_git(&repo_root, &["config", "credential.helper", ""]);
    let users_root = unique_temp_dir("review-poller-from-local-windows-users");
    fs::create_dir_all(users_root.join("Alice")).expect("other Windows home should exist");
    fs::create_dir_all(users_root.join("linux-akra"))
        .expect("wrong USER Windows home should exist");
    fs::create_dir_all(users_root.join("akra")).expect("current Windows home should exist");
    fs::write(
        users_root.join("Alice/.git-credentials"),
        "https://alice:alice-token-123@github.com\n",
    )
    .expect("other-user credential fixture should be written");
    fs::write(
        users_root.join("linux-akra/.git-credentials"),
        "https://linux-akra:linux-token-123@github.com\n",
    )
    .expect("wrong-user credential fixture should be written");
    fs::write(
        users_root.join("akra/.git-credentials"),
        "https://akra:akra-token-123@github.com\n",
    )
    .expect("current-user credential fixture should be written");

    let error =
        GithubReviewPollerAdapter::read_local_github_token_with_gh_program(&repo_root, &gh_program)
            .expect_err("legacy flag must not restore Windows credential-file scanning");

    assert!(error.to_string().contains("no longer supported"));
    assert!(!error.to_string().contains("akra-token-123"));
    let _ = fs::remove_dir_all(&fake_gh_root);
    let _ = fs::remove_dir_all(&repo_root);
    let _ = fs::remove_dir_all(&users_root);
    let _ = fs::remove_dir_all(&home_root);
    let _ = fs::remove_dir_all(&userprofile_root);
}
#[test]
fn windows_home_resolution_covers_absent_permission_and_error_edges() {
    let empty_root = unique_temp_dir("review-poller-windows-users-empty");
    fs::create_dir_all(&empty_root).expect("empty users root should be created");
    let missing = GithubReviewPollerAdapter::resolve_current_user_windows_home(&empty_root, "akra")
        .expect("empty users root should resolve as no home");
    assert_eq!(missing, None);
    let _ = fs::remove_dir_all(&empty_root);

    let file_root = unique_temp_dir("review-poller-windows-users-file");
    fs::write(&file_root, "not a directory").expect("file fixture should be written");
    let error = GithubReviewPollerAdapter::resolve_current_user_windows_home(&file_root, "akra")
        .expect_err("non-directory users root should report read_dir failure");
    assert!(
        error
            .to_string()
            .contains("failed to inspect legacy Windows credential profiles")
    );
    let _ = fs::remove_file(&file_root);

    let users_root = unique_temp_dir("review-poller-windows-users-permission");
    fs::create_dir_all(users_root.join("akra")).expect("direct user home should be created");
    let mut permissions = fs::metadata(&users_root)
        .expect("users root metadata should be readable")
        .permissions();
    permissions.set_mode(0o300);
    fs::set_permissions(&users_root, permissions).expect("users root permissions should update");

    let resolved =
        GithubReviewPollerAdapter::resolve_current_user_windows_home(&users_root, "akra")
            .expect("permission denied scan should fall back to direct match");

    assert_eq!(resolved, Some(users_root.join("akra")));
    let mut permissions = fs::metadata(&users_root)
        .expect("users root metadata should be readable")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&users_root, permissions).expect("users root permissions should restore");
    let _ = fs::remove_dir_all(&users_root);

    let file_entry_root = unique_temp_dir("review-poller-windows-users-file-entry");
    fs::create_dir_all(&file_entry_root).expect("file entry root should be created");
    fs::write(file_entry_root.join("Akra"), "not a directory")
        .expect("file entry should be written");
    let no_home =
        GithubReviewPollerAdapter::resolve_current_user_windows_home(&file_entry_root, "akra")
            .expect("file entries matching the user should not count as homes");
    assert_eq!(no_home, None);
    let _ = fs::remove_dir_all(&file_entry_root);
}

#[test]
fn windows_credential_path_respects_empty_and_missing_user_names() {
    let _guard = env_lock()
        .lock()
        .expect("environment fixture lock should not be poisoned");
    let _env = EnvVarGuard::apply(&[("USER", Some("   "))]);

    let credential_line = GithubReviewPollerAdapter::find_windows_github_credential_line_in_root(
        Path::new("/mnt/c/Users"),
    )
    .expect("empty USER should be handled as absent");

    assert_eq!(credential_line, None);
    drop(_env);

    let _env = EnvVarGuard::apply(&[(
        "USER",
        Some("akra-user-that-should-not-exist-for-review-poller-test"),
    )]);
    let credential_path =
        GithubReviewPollerAdapter::resolve_windows_credential_path_for_current_user_in_root(
            Path::new("/mnt/c/Users"),
        )
        .expect("missing Windows home should not fail");
    assert_eq!(credential_path, None);
}

#[test]
fn fetch_json_reports_curl_stderr_for_failed_http_request() {
    let root = unique_temp_dir("review-poller-fetch-json-error");
    fs::create_dir_all(&root).expect("fixture root should be created");
    let script = write_executable_script(
        &root,
        "fake-curl",
        r#"#!/bin/sh
set -eu
cat >/dev/null

echo "api denied" >&2
exit 22
"#,
    );
    let adapter = fake_adapter(&script);

    let error = adapter
        .fetch_json("/repos/acme/widgets/pulls/42")
        .expect_err("curl failure should include stderr");

    assert!(error.to_string().contains("github api request failed"));
    assert!(error.to_string().contains("api denied"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn expired_activity_budget_fails_before_spawning_curl() {
    let root = unique_temp_dir("review-poller-expired-activity-budget");
    fs::create_dir_all(&root).expect("fixture root should be created");
    let marker = root.join("curl-ran");
    let script = write_executable_script(
        &root,
        "fake-curl",
        &format!(
            "#!/bin/sh\nset -eu\n: > '{}'\nprintf '{{}}'\n",
            marker.display()
        ),
    );
    let adapter = fake_adapter(&script);
    let budget = super::GithubActivityLoadBudget {
        deadline: std::time::Instant::now(),
    };

    let error = adapter
        .fetch_object_with_budget::<serde_json::Value>("/rate_limit", Some(budget))
        .expect_err("expired aggregate budget must fail closed before curl spawn");

    assert!(error.to_string().contains("aggregate deadline"));
    assert!(
        !marker.exists(),
        "curl must not run after aggregate deadline"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn fetch_json_reports_curl_spawn_failure_with_url_context() {
    let missing_curl = unique_temp_dir("review-poller-missing-curl").join("missing-curl");
    let adapter = fake_adapter(&missing_curl);

    let error = adapter
        .fetch_json("/repos/acme/widgets/pulls/42")
        .expect_err("missing curl path should fail with context");

    assert!(error.to_string().contains("failed to run"));
    assert!(
        error
            .to_string()
            .contains("https://api.test/repos/acme/widgets/pulls/42")
    );
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
cat >/dev/null

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
cat >/dev/null
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
fn fetch_paginated_array_fails_closed_when_every_allowed_page_is_full() {
    let root = unique_temp_dir("review-poller-pagination-budget");
    fs::create_dir_all(&root).expect("fixture root should be created");
    let full_page = (1..=100)
        .map(|number| format!(r#"{{ "number": {number} }}"#))
        .collect::<Vec<_>>()
        .join(",");
    fs::write(root.join("full-page.json"), format!("[{full_page}]"))
        .expect("full page fixture should be written");
    let log_path = root.join("curl.log");
    let script = write_executable_script(
        &root,
        "fake-curl",
        &format!(
            r#"#!/bin/sh
set -eu
cat >/dev/null
last=""
for arg in "$@"; do
  last="$arg"
done
printf '%s\n' "$last" >> "{log}"
cat "{root}/full-page.json"
"#,
            log = log_path.display(),
            root = root.display()
        ),
    );
    let adapter = fake_adapter(&script);

    let error = adapter
        .fetch_paginated_array::<PullRequestLocatorResponse>("/repos/acme/widgets/pulls")
        .expect_err("an exact-full-page stream must fail instead of returning a partial snapshot");

    assert!(error.to_string().contains("pagination remained full"));
    assert!(error.to_string().contains("refusing an incomplete"));
    let request_count = fs::read_to_string(&log_path)
        .expect("curl log should be readable")
        .lines()
        .count();
    assert_eq!(request_count, super::MAX_PAGINATED_PAGES);
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
stdin_payload="$(cat)"
last=""
for arg in "$@"; do
  last="$arg"
done
printf '%s\n' "$@" >> "{log}"
printf '%s\n' "$stdin_payload" >> "{log}"
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
        curl_resolution_error: None,
        api_base_url: "https://api.test".to_string(),
        user_agent: "akra-test".to_string(),
        token: "secret-token".to_string(),
        subprocess_timeout: std::time::Duration::from_secs(1),
    }
}

fn env_lock() -> &'static Mutex<()> {
    crate::test_utils::process_environment_mutex()
}

struct EnvVarGuard {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvVarGuard {
    fn apply(updates: &[(&'static str, Option<&str>)]) -> Self {
        let saved = updates
            .iter()
            .map(|(key, _)| (*key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        unsafe {
            for (key, value) in updates {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
        Self { saved }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            for (key, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("fixture path should be valid UTF-8")
}

use super::*;

fn commit_result(fixture: &GitFixture, name: &str) -> String {
    fs::write(fixture.repo.join(name), format!("{name}\n")).unwrap();
    git(&fixture.repo, &["add", name]);
    git(&fixture.repo, &["commit", "-qm", name]);
    git_stdout(&fixture.repo, &["rev-parse", "HEAD"])
}

fn prepare_remote(fixture: &GitFixture, adapter: &GithubAutomationAdapter) -> (String, String) {
    adapter
        .push_branch(path_str(&fixture.repo), "main", false)
        .unwrap();
    let base = git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]);
    let url = adapter
        .credential_redacted_push_url_for_remote(path_str(&fixture.repo), "origin")
        .unwrap();
    (base, url)
}

#[test]
fn exact_integration_result_rejects_head_drift_then_publishes_only_the_verified_oid() {
    let fixture = GitFixture::new("frozen-integration-result");
    let adapter = GithubAutomationAdapter::new();
    let (base, url) = prepare_remote(&fixture, &adapter);
    let verified = commit_result(&fixture, "VERIFIED.md");
    let drifted = commit_result(&fixture, "UNREVIEWED.md");
    assert_ne!(verified, drifted);

    let error = adapter
        .push_frozen_integration_commit_to_delivery_target(
            path_str(&fixture.repo),
            "origin",
            &url,
            "main",
            &base,
            &verified,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("changed after host verification")
    );
    assert_eq!(
        git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
        base
    );

    git(&fixture.repo, &["checkout", "--detach", &verified]);
    adapter
        .push_frozen_integration_commit_to_delivery_target(
            path_str(&fixture.repo),
            "origin",
            &url,
            "main",
            &base,
            &verified,
        )
        .unwrap();
    assert_eq!(
        git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
        verified
    );
    assert_eq!(git_stdout(&fixture.repo, &["rev-parse", "HEAD"]), verified);
}

#[test]
fn exact_integration_result_keeps_the_remote_compare_and_swap_fence() {
    let fixture = GitFixture::new("frozen-integration-remote-race");
    let adapter = GithubAutomationAdapter::new();
    let (base, url) = prepare_remote(&fixture, &adapter);
    let verified = commit_result(&fixture, "VERIFIED.md");
    git(&fixture.repo, &["checkout", "--detach", &base]);
    let concurrent = commit_result(&fixture, "CONCURRENT.md");
    git(
        &fixture.repo,
        &["push", "--force", "origin", "HEAD:refs/heads/main"],
    );
    git(&fixture.repo, &["checkout", "--detach", &verified]);

    assert!(
        adapter
            .push_frozen_integration_commit_to_delivery_target(
                path_str(&fixture.repo),
                "origin",
                &url,
                "main",
                &base,
                &verified,
            )
            .is_err()
    );
    assert_eq!(
        git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
        concurrent
    );
}

#[test]
fn exact_integration_result_rejects_non_commit_and_non_oid_inputs_without_publication() {
    let fixture = GitFixture::new("frozen-integration-invalid");
    let adapter = GithubAutomationAdapter::new();
    let (base, url) = prepare_remote(&fixture, &adapter);
    let verified = commit_result(&fixture, "VERIFIED.md");
    let tree = git_stdout(&fixture.repo, &["rev-parse", "HEAD^{tree}"]);
    for invalid in ["HEAD", "--all", "", &tree, &verified[..12]] {
        assert!(
            adapter
                .push_frozen_integration_commit_to_delivery_target(
                    path_str(&fixture.repo),
                    "origin",
                    &url,
                    "main",
                    &base,
                    invalid,
                )
                .is_err()
        );
        assert_eq!(
            git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
            base
        );
    }
}

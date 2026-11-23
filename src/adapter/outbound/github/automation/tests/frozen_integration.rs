use super::super::{
    IsolatedGithubNetworkContext, TEST_GITHUB_UNAVAILABLE_EXECUTABLE,
    run_git_command_in_network_context, run_github_script_command_in_network_context,
    verify_github_write_identity_in_network_context,
};
use super::*;

struct UnavailableExecutableGuard;

impl UnavailableExecutableGuard {
    fn python() -> Self {
        TEST_GITHUB_UNAVAILABLE_EXECUTABLE.with(|slot| {
            assert!(slot.replace(Some("python3")).is_none());
        });
        Self
    }
}

impl Drop for UnavailableExecutableGuard {
    fn drop(&mut self) {
        TEST_GITHUB_UNAVAILABLE_EXECUTABLE.with(|slot| {
            slot.replace(None);
        });
    }
}

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
fn canonical_source_objects_remain_readable_in_the_isolated_context_and_frozen_push() {
    let _python_unavailable = UnavailableExecutableGuard::python();
    let mut fixture = GitFixture::new("frozen-network-objects");
    // Windows canonicalization produces a verbatim path. Preserve that identity for the
    // adapter input while requiring ordinary Git-facing paths in env and alternates.
    fixture.repo = fs::canonicalize(&fixture.repo).unwrap();
    let context =
        IsolatedGithubNetworkContext::for_git(path_str(&fixture.repo), path_str(&fixture.remote))
            .unwrap();
    let mut command = Command::new(&context.executables.git);
    context.configure_command(&mut command).unwrap();
    let objects_argument = command
        .get_envs()
        .find_map(|(name, value)| (name == "GIT_OBJECT_DIRECTORY").then_some(value).flatten())
        .unwrap();
    assert_eq!(
        fs::canonicalize(objects_argument).unwrap(),
        context.common_objects
    );
    #[cfg(windows)]
    {
        assert!(
            context
                .common_objects
                .to_string_lossy()
                .starts_with(r"\\?\")
        );
        assert!(!objects_argument.to_string_lossy().starts_with(r"\\?\"));
    }
    assert_eq!(
        fs::read_to_string(context.repo_root.join(".git/objects/info/alternates")).unwrap(),
        format!("{}\n", objects_argument.to_str().unwrap())
    );
    let first = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    let output = run_git_command_in_network_context(&context, &["cat-file", "-t", &first]).unwrap();
    assert!(
        output.status.success(),
        "isolated object lookup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "commit");
    let private_root = context.root.clone();
    drop(context);
    assert!(!private_root.exists());

    let adapter = GithubAutomationAdapter::new();
    let (base, url) = prepare_remote(&fixture, &adapter);
    assert_eq!(base, first);
    let packed = commit_result(&fixture, "PACKED.md");
    git(&fixture.repo, &["repack", "-a", "-d"]);
    adapter
        .push_frozen_integration_commit_to_delivery_target(
            path_str(&fixture.repo),
            "origin",
            &url,
            "main",
            &base,
            &packed,
        )
        .unwrap();
    assert_eq!(
        git_stdout(&fixture.remote, &["rev-parse", "refs/heads/main"]),
        packed
    );
    assert_eq!(
        git_stdout(&fixture.remote, &["show", &format!("{packed}:PACKED.md")]),
        "PACKED.md"
    );
}

#[test]
fn exact_integration_result_rejects_head_drift_then_publishes_only_the_verified_oid() {
    // Direct frozen Git publication must not resolve the unused REST helper's Python.
    let _python_unavailable = UnavailableExecutableGuard::python();
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
    let _python_unavailable = UnavailableExecutableGuard::python();
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
    let _python_unavailable = UnavailableExecutableGuard::python();
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

#[test]
fn embedded_helper_rejects_missing_or_untrusted_dependencies_in_a_git_only_context() {
    let fixture = GitFixture::new("frozen-integration-helper-dependencies");
    // A harmless helper would succeed if invoked. Dependency failures must happen before it.
    let _helper = TestGithubHelperGuard::install(b"printf 'helper must not execute\\n'\n");
    let mut context =
        IsolatedGithubNetworkContext::for_git(path_str(&fixture.repo), path_str(&fixture.remote))
            .unwrap();
    {
        let _python_unavailable = UnavailableExecutableGuard::python();
        // Real GitHub publication must still pass its helper identity check; only the
        // existing local-remote fixture path is allowed to avoid that network operation.
        let error = verify_github_write_identity_in_network_context(&context, false).unwrap_err();
        assert!(
            format!("{error:#}")
                .contains("trusted GitHub helper dependency `python3` is unavailable")
        );
    }

    // A stale/forged dependency path cannot turn the original repository into a trusted root.
    let untrusted = fixture.repo.join("untrusted-tools");
    fs::create_dir(&untrusted).unwrap();
    for name in ["bash", "bash.exe"] {
        fs::write(untrusted.join(name), b"repository-controlled executable").unwrap();
    }
    context.executables.path = std::env::join_paths([untrusted]).unwrap();
    let error = run_github_script_command_in_network_context(
        &context,
        &["probe"],
        "untrusted dependency probe",
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("trusted bash interpreter is unavailable"));
}

use super::*;

#[test]
fn sha256_git_objects_use_the_same_fixed_recipe_with_a_distinct_environment_key() {
    let fixture = Fixture::with_format("sha256");
    fs::write(fixture.root.join("README.md"), "new text\n").unwrap();
    fixture.commit();
    let prepared = fixture.prepare(&fixture.source());
    assert_eq!(prepared.manifest.source.tree_sha.len(), 64);
    assert_eq!(prepared.manifest.base_tree_sha.len(), 64);
    let current = GitTaskVerificationRunnerAdapter::new()
        .environment_fingerprint(&prepared.manifest)
        .unwrap();
    assert_eq!(current, prepared.manifest.environment_sha256);
    let (observation, _, _) = executed(Box::new(prepared).run(&|| false));
    assert_eq!(
        observation.outcome,
        TaskVerificationExecutionOutcome::Exited { exit_code: 0 }
    );
}

#[test]
fn private_temporary_paths_do_not_change_environment_or_command_fingerprints() {
    let fixture = Fixture::new();
    let source = fixture.source();
    let first = fixture.prepare(&source);
    let first_manifest = first.manifest.clone();
    drop(first);
    let second = fixture.prepare(&source);
    assert_eq!(first_manifest, second.manifest);
    assert_eq!(
        GitTaskVerificationRunnerAdapter::new()
            .environment_fingerprint(&second.manifest)
            .unwrap(),
        second.manifest.environment_sha256
    );
}

#[test]
fn historical_receipt_fingerprints_do_not_poison_the_current_environment_key() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare(&fixture.source());
    let mut historical = prepared.manifest.clone();
    historical.base_commit_sha = "a".repeat(40);
    historical.base_tree_sha = "b".repeat(40);
    historical.source.head_commit_sha = "c".repeat(40);
    historical.source.tree_sha = "d".repeat(40);
    historical.git_binary_sha256 = "1".repeat(64);
    historical.command_sha256 = "2".repeat(64);
    historical.configuration_sha256 = "3".repeat(64);
    historical.environment_sha256 = "4".repeat(64);
    historical.validate().unwrap();
    assert_eq!(
        GitTaskVerificationRunnerAdapter::new()
            .environment_fingerprint(&historical)
            .unwrap(),
        prepared.manifest.environment_sha256,
    );
}

#[test]
fn source_changes_keep_the_environment_key_but_change_the_bound_command() {
    let fixture = Fixture::new();
    let first = fixture.prepare(&fixture.source()).manifest;
    fs::write(fixture.root.join("README.md"), "a second source\n").unwrap();
    fixture.commit();
    let second = fixture.prepare(&fixture.source());
    assert_eq!(first.environment_sha256, second.manifest.environment_sha256);
    assert_ne!(first.command_sha256, second.manifest.command_sha256);
}

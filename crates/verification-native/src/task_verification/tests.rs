use super::*;
use akra_verification::evidence::TaskEvidenceTree;
use akra_verification::verification::TaskVerificationRecipe;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn preparation_diagnostics_never_expose_unrecognized_error_contents() {
    let error = anyhow::anyhow!("secret command/path/env/output body")
        .context("failed to secure private Windows path `private-path-secret`");
    assert_eq!(
        diagnostics::classifications(&error),
        ["secure_private_path", "opaque_error"]
    );
    let error = std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "secret OS error path and content",
    );
    let classes = diagnostics::classifications(&error.into());
    assert_eq!(classes[0], "io(kind=PermissionDenied,os=None)");
    assert!(!classes.join(" ").contains("secret"));
}

struct Fixture {
    root: PathBuf,
    base: String,
}

impl Fixture {
    fn new() -> Self {
        Self::with_format("sha1")
    }
    fn with_format(format: &str) -> Self {
        let root = crate::test_support::platform_safe_temp_dir().join(format!(
            "akra-task-verification-test-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        akra_native::private_directory::ensure_private_directory(&root).unwrap();
        git(
            &root,
            &[
                "init",
                "-q",
                "-b",
                "main",
                &format!("--object-format={format}"),
            ],
        );
        // This fresh fixture keeps the same local policy without four setup processes.
        fs::OpenOptions::new()
            .append(true)
            .open(root.join(".git/config"))
            .expect("fresh Git fixture config should exist")
            .write_all(
                b"\n[user]\n\tname = Host Verification Fixture\n\temail = fixture@example.invalid\n[core]\n\tautocrlf = false\n[commit]\n\tgpgsign = false\n",
            )
            .expect("fixed local Git fixture policy should append");
        fs::write(root.join("README.md"), "base text\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "base"]);
        let base = git(&root, &["rev-parse", "HEAD"]);
        Self { root, base }
    }
    fn commit(&self) {
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-qm", "source"]);
    }
    fn source(&self) -> TaskEvidenceTree {
        TaskEvidenceTree {
            head_commit_sha: git(&self.root, &["rev-parse", "HEAD"]),
            tree_sha: git(&self.root, &["rev-parse", "HEAD^{tree}"]),
            worktree_clean: true,
        }
    }
    fn request<'a>(&'a self, source: &'a TaskEvidenceTree) -> TaskVerificationPrepareRequest<'a> {
        TaskVerificationPrepareRequest {
            workspace_directory: self.root.to_str().unwrap(),
            recipe: TaskVerificationRecipe::GitDiffCheckV1,
            base_commit_sha: &self.base,
            source,
        }
    }
    fn prepare(&self, source: &TaskEvidenceTree) -> PreparedVerification {
        let started = Instant::now();
        prepare(self.request(source), &|| false).unwrap_or_else(|reason| {
            panic!(
                "fixed recipe preparation failed: reason={reason:?}; elapsed_ms={}",
                started.elapsed().as_millis()
            )
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git(root: &Path, arguments: &[&str]) -> String {
    let output = crate::test_support::git_command()
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn executed(
    outcome: TaskVerificationRunOutcome,
) -> (TaskVerificationObservation, Vec<u8>, Vec<u8>) {
    match outcome {
        TaskVerificationRunOutcome::Executed {
            observation,
            stdout,
            stderr,
        } => {
            observation.validate().unwrap();
            assert_eq!(observation.stdout.sha256, digest(&stdout));
            assert_eq!(observation.stderr.sha256, digest(&stderr));
            (*observation, stdout, stderr)
        }
        TaskVerificationRunOutcome::NotRun { reason } => {
            panic!("fixed recipe did not execute: {reason:?}")
        }
    }
}

#[test]
fn actual_git_check_binds_the_frozen_full_range_without_changing_index_head_or_refs() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("README.md"), "earlier violation  \n").unwrap();
    fixture.commit();
    fs::write(fixture.root.join("last.txt"), "last commit is clean\n").unwrap();
    fixture.commit();
    let source = fixture.source();
    let index = fs::read(fixture.root.join(".git/index")).unwrap();
    let refs = git(&fixture.root, &["show-ref"]);
    let prepared = fixture.prepare(&source);
    assert_eq!(prepared.manifest.base_commit_sha, fixture.base);
    assert_eq!(prepared.manifest.source, source);
    let (observation, stdout, _) = executed(Box::new(prepared).run(&|| false));
    assert!(
        matches!(observation.outcome, TaskVerificationExecutionOutcome::Exited { exit_code } if exit_code != 0)
    );
    assert!(String::from_utf8(stdout).unwrap().contains("README.md"));
    assert!(observation.stdout.complete && observation.stderr.complete);
    assert_eq!(fs::read(fixture.root.join(".git/index")).unwrap(), index);
    assert_eq!(git(&fixture.root, &["show-ref"]), refs);
    assert_eq!(
        git(&fixture.root, &["rev-parse", "HEAD"]),
        source.head_commit_sha
    );
}

#[test]
fn source_attributes_info_attributes_and_whitespace_config_cannot_hide_a_failure() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.join("README.md"),
        "concealed violation  \n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n",
    )
    .unwrap();
    fs::write(
        fixture.root.join(".gitattributes"),
        "* -whitespace binary conflict-marker-size=32\n",
    )
    .unwrap();
    fs::write(
        fixture.root.join(".git/info/attributes"),
        "* -whitespace -diff\n",
    )
    .unwrap();
    git(
        &fixture.root,
        &[
            "config",
            "core.whitespace",
            "-blank-at-eol,-blank-at-eof,-space-before-tab",
        ],
    );
    fixture.commit();
    let prepared = fixture.prepare(&fixture.source());
    let (observation, stdout, _) = executed(Box::new(prepared).run(&|| false));
    assert_eq!(
        observation.outcome,
        TaskVerificationExecutionOutcome::Exited { exit_code: 2 },
    );
    let diagnostics = String::from_utf8(stdout).unwrap();
    assert!(diagnostics.contains("trailing whitespace"), "{diagnostics}");
    assert!(
        diagnostics.contains("leftover conflict marker"),
        "{diagnostics}"
    );
    assert!(observation.stdout.complete && observation.stderr.complete);
}

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

#[test]
fn binary_and_oversized_blobs_are_not_misrepresented_as_passing_text() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("binary.dat"), b"data\0hidden").unwrap();
    fixture.commit();
    assert!(matches!(
        prepare(fixture.request(&fixture.source()), &|| false),
        Err(Reason::InvalidInput)
    ));
    fs::remove_file(fixture.root.join("binary.dat")).unwrap();
    fs::write(
        fixture.root.join("large.txt"),
        vec![b'x'; environment::MAX_BLOB_BYTES + 1],
    )
    .unwrap();
    fixture.commit();
    assert!(matches!(
        prepare(fixture.request(&fixture.source()), &|| false),
        Err(Reason::InvalidInput)
    ));
}

#[test]
fn symlink_and_gitlink_diff_modes_are_rejected_on_every_platform() {
    for mode in ["120000", "160000"] {
        let fixture = Fixture::new();
        let oid = if mode == "160000" {
            fixture.base.clone()
        } else {
            git(&fixture.root, &["rev-parse", "HEAD:README.md"])
        };
        git(
            &fixture.root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("{mode},{oid},special"),
            ],
        );
        git(&fixture.root, &["commit", "-qm", "unsupported mode"]);
        assert!(matches!(
            prepare(fixture.request(&fixture.source()), &|| false),
            Err(Reason::InvalidInput)
        ));
    }
}

#[test]
fn source_execution_configuration_and_existing_alternates_fail_closed() {
    let fixture = Fixture::new();
    let source = fixture.source();
    git(
        &fixture.root,
        &["config", "diff.external", "unexpected-verifier-command"],
    );
    assert!(prepare(fixture.request(&source), &|| false).is_err());
    git(&fixture.root, &["config", "--unset", "diff.external"]);
    fs::write(fixture.root.join(".git/objects/info/alternates"), b"").unwrap();
    assert!(prepare(fixture.request(&source), &|| false).is_err());
}

#[test]
fn cancellation_after_spawn_is_an_observed_execution_with_incomplete_streams() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare(&fixture.source());
    let calls = AtomicUsize::new(0);
    let (observation, _, _) =
        executed(Box::new(prepared).run(&|| calls.fetch_add(1, Ordering::SeqCst) >= 2));
    assert_eq!(
        observation.outcome,
        TaskVerificationExecutionOutcome::Cancelled {}
    );
    assert!(!observation.stdout.complete && !observation.stderr.complete);
}

#[test]
fn an_expired_or_cancelled_preparation_cannot_start_the_recipe() {
    let fixture = Fixture::new();
    let mut prepared = fixture.prepare(&fixture.source());
    prepared.deadline = Instant::now();
    assert!(matches!(
        Box::new(prepared).run(&|| false),
        TaskVerificationRunOutcome::NotRun { .. }
    ));
    assert!(matches!(
        GitTaskVerificationRunnerAdapter::new()
            .prepare(fixture.request(&fixture.source()), &|| true),
        Err(Reason::Cancelled)
    ));
}

#[test]
fn excessive_real_git_diagnostics_are_bounded_and_cannot_pass() {
    let fixture = Fixture::new();
    let lines = (0..50_000)
        .map(|index| format!("line-{index}  \n"))
        .collect::<String>();
    fs::write(fixture.root.join("README.md"), lines).unwrap();
    fixture.commit();
    let (observation, stdout, stderr) =
        executed(Box::new(fixture.prepare(&fixture.source())).run(&|| false));
    assert_eq!(
        observation.outcome,
        TaskVerificationExecutionOutcome::OutputLimit {}
    );
    assert!(stdout.len() <= MAX_TASK_VERIFICATION_STREAM_BYTES as usize);
    assert!(stderr.len() <= MAX_TASK_VERIFICATION_STREAM_BYTES as usize);
    assert!(!observation.stdout.complete);
    assert!(!stdout.contains(&0));
}

#[test]
fn malformed_or_over_limit_diff_inventory_is_rejected_without_partial_results() {
    let valid = format!(
        ":100644 100644 {} {} M\0src/file.rs\0",
        "a".repeat(40),
        "b".repeat(40)
    );
    assert_eq!(
        input::parse_changed_blobs(valid.as_bytes(), 40)
            .unwrap()
            .len(),
        2
    );
    for malformed in [
        valid.trim_end_matches('\0').as_bytes(),
        b":100644 100644 a b M\0path\0",
        b"header\0",
    ] {
        assert_eq!(
            input::parse_changed_blobs(malformed, 40),
            Err(Reason::InvalidInput)
        );
    }
    let oversized = valid.repeat(environment::MAX_PATHS + 1);
    assert_eq!(
        input::parse_changed_blobs(oversized.as_bytes(), 40),
        Err(Reason::InvalidInput)
    );
}

#[cfg(windows)]
#[test]
fn canonical_windows_workspace_is_normalized_only_at_git_boundaries() {
    let fixture = Fixture::new();
    let canonical = fs::canonicalize(&fixture.root).unwrap();
    let source = fixture.source();
    let prepared = prepare(
        TaskVerificationPrepareRequest {
            workspace_directory: canonical.to_str().unwrap(),
            ..fixture.request(&source)
        },
        &|| false,
    )
    .unwrap();
    let command = prepared.context.command(&prepared.binary).unwrap();
    let program = Path::new(command.get_program());
    assert!(!program.to_string_lossy().starts_with(r"\\?\"));
    assert_eq!(
        fs::canonicalize(program).unwrap(),
        prepared.binary.path().to_path_buf()
    );
    let args = command.get_args().collect::<Vec<_>>();
    for name in ["--git-dir", "--work-tree"] {
        let position = args
            .iter()
            .position(|argument| *argument == std::ffi::OsStr::new(name))
            .unwrap();
        let path = Path::new(args[position + 1]);
        assert!(!path.to_string_lossy().starts_with(r"\\?\"));
        assert!(fs::canonicalize(path).unwrap().is_absolute());
    }
    let (observation, _, _) = executed(Box::new(prepared).run(&|| false));
    assert_eq!(
        observation.outcome,
        TaskVerificationExecutionOutcome::Exited { exit_code: 0 }
    );
}

#[test]
fn fixture_configuration_is_local_and_preserves_the_fixed_policy() {
    let fixture = Fixture::new();
    let local = git(&fixture.root, &["config", "--local", "--list"]);
    let settings = local
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect::<std::collections::BTreeMap<_, _>>();
    for (name, expected) in [
        ("user.name", "Host Verification Fixture"),
        ("user.email", "fixture@example.invalid"),
        ("core.autocrlf", "false"),
        ("commit.gpgsign", "false"),
    ] {
        assert_eq!(settings.get(name).copied(), Some(expected), "{name}");
    }
}

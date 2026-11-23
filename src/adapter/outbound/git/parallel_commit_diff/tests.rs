use super::*;
use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelModeRuntimePort, ParallelWorkerCommitDisposition, ParallelWorkerCommitOutcome,
    ParallelWorkerCommitRequest,
};
use crate::domain::issue_automation::delivery::{
    IssueDeliveryRiskReason, MAX_ISSUE_DELIVERY_DIFF_PATH_BYTES, assess_delivery_diff,
};
use crate::domain::issue_automation::policy::IssueAutomationPolicy;
use std::fs;

const BASE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SOURCE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn request() -> ParallelCommitDiffRequest<'static> {
    ParallelCommitDiffRequest {
        workspace_directory: "/unused",
        base_commit_sha: BASE,
        source_commit_sha: SOURCE,
    }
}

#[test]
fn nul_numstat_preserves_complete_paths_and_explicit_binary_counts() {
    let diff = parse_numstat(
        request(),
        "3\t2\tsrc/hello world.rs\0-\t-\tassets/icon.bin\00\t7\t문서/옛 문서.md\0".as_bytes(),
    )
    .unwrap();
    assert_eq!(diff.base_commit_sha, BASE);
    assert_eq!(diff.source_commit_sha, SOURCE);
    assert_eq!(diff.files.len(), 3);
    assert_eq!(diff.files[0].path, "assets/icon.bin");
    assert_eq!(diff.files[0].lines, IssueDeliveryDiffLines::Binary);
    assert_eq!(diff.files[2].path, "문서/옛 문서.md");
    assert_eq!(
        diff.files[2].lines,
        IssueDeliveryDiffLines::Text {
            added: 0,
            deleted: 7
        }
    );
}

#[test]
fn malformed_or_partially_read_numstat_never_returns_a_partial_diff() {
    for bytes in [
        &b""[..],
        b"1\t0\tsrc/lib.rs",
        b"1\t0\tsrc/lib.rs\0\0",
        b"1\tsrc/lib.rs\0",
        b"\t0\tsrc/lib.rs\0",
        b"-\t0\tassets/icon.bin\0",
        b"0\t-\tassets/icon.bin\0",
        b"+1\t0\tsrc/lib.rs\0",
        b"01\t0\tsrc/lib.rs\0",
        b"1.5\t0\tsrc/lib.rs\0",
        b"1\t0\t\0old\0new\0",
        b"1\t0\tsrc/lib.rs\01\t0\tsrc/lib.rs\0",
        b"1\t0\tsrc/lib.rs\0malformed-tail\0",
    ] {
        assert!(parse_numstat(request(), bytes).is_err(), "{bytes:?}");
    }
}

#[test]
fn non_utf8_and_control_paths_are_rejected_without_skipping_the_file() {
    for bytes in [
        &b"1\t0\tsrc/\xff.rs\0"[..],
        b"1\t0\tsrc/tab\tfile.rs\0",
        b"1\t0\tsrc/line\nfile.rs\0",
        b"1\t0\t../outside\0",
    ] {
        assert!(parse_numstat(request(), bytes).is_err());
    }
}

#[test]
fn numeric_overflow_and_collection_limits_fail_the_entire_inspection() {
    for data in [
        b"18446744073709551616\t0\tsrc/lib.rs\0".to_vec(),
        b"18446744073709551615\t1\tsrc/lib.rs\0".to_vec(),
        b"18446744073709551615\t0\ta\01\t0\tb\0".to_vec(),
        format!(
            "1\t0\t{}\0",
            "x".repeat(MAX_ISSUE_DELIVERY_DIFF_PATH_BYTES + 1)
        )
        .into_bytes(),
        (0..=MAX_ISSUE_DELIVERY_DIFF_FILES)
            .map(|index| format!("1\t0\tsrc/file-{index}.rs\0"))
            .collect::<String>()
            .into_bytes(),
        vec![b'x'; MAX_DIFF_STDOUT_BYTES + 1],
    ] {
        assert!(parse_numstat(request(), &data).is_err());
    }
}

struct Fixture {
    root: PathBuf,
    base: String,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-delivery-diff-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.name", "Delivery Diff Fixture"]);
        git(&root, &["config", "user.email", "fixture@example.invalid"]);
        git(&root, &["config", "core.autocrlf", "false"]);
        fs::write(root.join("README.md"), "base\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "base"]);
        let base = git(&root, &["rev-parse", "HEAD"]);
        Self { root, base }
    }

    fn write(&self, path: &str, content: impl AsRef<[u8]>) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn commit(&self) -> String {
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-q", "-m", "fixture result"]);
        git(&self.root, &["rev-parse", "HEAD"])
    }

    fn host_commit(&self) -> ParallelWorkerCommitOutcome {
        let head = git(&self.root, &["rev-parse", "HEAD"]);
        super::super::parallel_worker_commit::prepare_parallel_worker_commit(
            ParallelWorkerCommitRequest {
                workspace_directory: self.root.to_str().unwrap(),
                expected_branch_name: "main",
                expected_base_commit_sha: &self.base,
                expected_head_commit_sha: &head,
                commit_message: "host-owned fixture result",
                commit_timestamp: "2026-11-23T00:00:00Z",
            },
        )
        .unwrap()
    }

    fn inspect(&self, base: &str, source: &str) -> Result<IssueDeliveryDiff, String> {
        super::super::parallel_mode_runtime::GitParallelModeRuntimeAdapter::new()
            .inspect_parallel_commit_diff(ParallelCommitDiffRequest {
                workspace_directory: self.root.to_str().unwrap(),
                base_commit_sha: base,
                source_commit_sha: source,
            })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = crate::test_utils::git_command()
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[test]
fn the_full_frozen_range_preserves_protected_changes_before_the_final_host_commit() {
    let fixture = Fixture::new();
    fixture.write(
        ".github/workflows/check.yml",
        "name: earlier protected change\n",
    );
    let first = fixture.host_commit();
    assert_eq!(first.disposition, ParallelWorkerCommitDisposition::Created);
    fixture.write("src/final.rs", "pub fn result() {}\n");
    let last = fixture.host_commit();
    assert_eq!(last.disposition, ParallelWorkerCommitDisposition::Created);

    let final_commit_only = fixture
        .inspect(&first.commit_sha, &last.commit_sha)
        .unwrap();
    assert!(
        !assess_delivery_diff(&IssueAutomationPolicy::builtin(), &final_commit_only)
            .unwrap()
            .requires_human()
    );
    let complete = fixture.inspect(&fixture.base, &last.commit_sha).unwrap();
    assert_eq!(complete.files.len(), 2);
    assert_eq!(
        assess_delivery_diff(&IssueAutomationPolicy::builtin(), &complete)
            .unwrap()
            .reasons,
        [IssueDeliveryRiskReason::ProtectedPath]
    );

    let existing = fixture.host_commit();
    assert_eq!(
        existing.disposition,
        ParallelWorkerCommitDisposition::Existing
    );
    assert_eq!(existing.commit_sha, last.commit_sha);
    assert_eq!(
        fixture
            .inspect(&fixture.base, &existing.commit_sha)
            .unwrap(),
        complete
    );
}

#[test]
fn real_renames_and_deletions_keep_both_protected_names_in_the_inventory() {
    let fixture = Fixture::new();
    fixture.write(".github/workflows/old.yml", "protected workflow\n");
    fixture.write("ordinary.txt", "ordinary source\n");
    fixture.write("AGENTS.md", "protected instructions\n");
    let base = fixture.commit();
    fs::create_dir_all(fixture.root.join("docs")).unwrap();
    fs::rename(
        fixture.root.join(".github/workflows/old.yml"),
        fixture.root.join("docs/moved.txt"),
    )
    .unwrap();
    fs::rename(
        fixture.root.join("ordinary.txt"),
        fixture.root.join(".github/workflows/new.yml"),
    )
    .unwrap();
    fs::remove_file(fixture.root.join("AGENTS.md")).unwrap();
    let source = fixture.commit();

    let diff = fixture.inspect(&base, &source).unwrap();
    let paths = diff
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            ".github/workflows/new.yml",
            ".github/workflows/old.yml",
            "AGENTS.md",
            "docs/moved.txt",
            "ordinary.txt"
        ]
    );
    assert_eq!(
        assess_delivery_diff(&IssueAutomationPolicy::builtin(), &diff)
            .unwrap()
            .reasons,
        [IssueDeliveryRiskReason::ProtectedPath]
    );
    assert_eq!(
        diff.files[1].lines,
        IssueDeliveryDiffLines::Text {
            added: 0,
            deleted: 1
        }
    );
    assert_eq!(
        diff.files[2].lines,
        IssueDeliveryDiffLines::Text {
            added: 0,
            deleted: 1
        }
    );
}

#[test]
fn committed_binary_and_large_text_are_human_reasons_and_live_edits_are_not_diff_input() {
    let fixture = Fixture::new();
    fixture.write("assets/result.bin", [0, 1, 2, 3, 0, 255]);
    fixture.write("src/generated.rs", "generated\n".repeat(1_001));
    let source = fixture.host_commit().commit_sha;
    let expected = fixture.inspect(&fixture.base, &source).unwrap();
    assert_eq!(
        assess_delivery_diff(&IssueAutomationPolicy::builtin(), &expected)
            .unwrap()
            .reasons,
        [
            IssueDeliveryRiskReason::DiffTooLarge,
            IssueDeliveryRiskReason::BinaryDiff
        ]
    );
    fixture.write(
        ".github/workflows/uncommitted.yml",
        "not in the frozen source\n",
    );
    fixture.write("src/generated.rs", "changed after freezing\n");
    assert_eq!(fixture.inspect(&fixture.base, &source).unwrap(), expected);
}

#[test]
fn missing_nonancestor_or_symbolic_commit_inputs_do_not_produce_a_verdict() {
    let fixture = Fixture::new();
    fixture.write("one.txt", "first branch\n");
    let first = fixture.commit();
    git(
        &fixture.root,
        &["checkout", "-q", "-b", "other", &fixture.base],
    );
    fixture.write("two.txt", "second branch\n");
    let other = fixture.commit();
    assert!(fixture.inspect(&first, &other).is_err());
    assert!(fixture.inspect(&fixture.base, &"f".repeat(40)).is_err());
    assert!(fixture.inspect(&fixture.base, "HEAD").is_err());
    let subdirectory = fixture.root.join("nested");
    fs::create_dir_all(&subdirectory).unwrap();
    assert!(
        inspect_parallel_commit_diff(ParallelCommitDiffRequest {
            workspace_directory: subdirectory.to_str().unwrap(),
            base_commit_sha: &fixture.base,
            source_commit_sha: &other,
        })
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn a_real_non_utf8_path_fails_without_dropping_the_changed_file() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let name = std::ffi::OsString::from_vec(b"non-utf8-\xff.txt".to_vec());
    fs::write(fixture.root.join(name), b"changed\n").unwrap();
    let source = fixture.commit();
    assert!(fixture.inspect(&fixture.base, &source).is_err());
}

#[cfg(unix)]
#[test]
fn executable_diff_configuration_is_rejected_without_running_it() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.write("result.txt", "change\n");
    let source = fixture.commit();
    let command = fixture.root.join("diff-must-not-run.sh");
    let marker = fixture.root.join("external-diff-ran");
    fs::write(
        &command,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&command, fs::Permissions::from_mode(0o700)).unwrap();
    git(
        &fixture.root,
        &["config", "diff.external", command.to_str().unwrap()],
    );
    assert!(fixture.inspect(&fixture.base, &source).is_err());
    assert!(!marker.exists());
}

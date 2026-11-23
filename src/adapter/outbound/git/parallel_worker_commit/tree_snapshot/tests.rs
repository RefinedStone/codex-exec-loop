use super::*;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use std::fs;

struct Fixture {
    root: PathBuf,
    base: String,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-evidence-tree-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.name", "Evidence Tree Fixture"]);
        git(&root, &["config", "user.email", "fixture@example.invalid"]);
        git(&root, &["config", "core.autocrlf", "false"]);
        fs::write(root.join("README.md"), "base\n").unwrap();
        fs::write(root.join("removed.txt"), "old file\n").unwrap();
        fs::write(root.join(".gitignore"), "generated.bin\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "base"]);
        let base = git(&root, &["rev-parse", "HEAD"]);
        Self { root, base }
    }

    fn request(&self) -> ParallelWorkerTreeRequest<'_> {
        ParallelWorkerTreeRequest {
            workspace_directory: self.root.to_str().unwrap(),
            expected_branch_name: "main",
            expected_base_commit_sha: &self.base,
            expected_head_commit_sha: &self.base,
        }
    }
    fn snapshot(&self) -> Result<ParallelWorkerTreeSnapshot, String> {
        GitParallelModeRuntimeAdapter::new().snapshot_parallel_worker_tree(self.request())
    }
    fn index_path(&self) -> PathBuf {
        absolutize(
            &self.root,
            Path::new(&git(&self.root, &["rev-parse", "--git-path", "index"])),
        )
    }
    fn host_commit(&self) -> String {
        GitParallelModeRuntimeAdapter::new()
            .prepare_parallel_worker_commit(ParallelWorkerCommitRequest {
                workspace_directory: self.root.to_str().unwrap(),
                expected_branch_name: "main",
                expected_base_commit_sha: &self.base,
                expected_head_commit_sha: &self.base,
                commit_message: "host result",
                commit_timestamp: "2026-11-23T00:00:00Z",
            })
            .unwrap()
            .commit_sha
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git(root: &Path, arguments: &[&str]) -> String {
    let output = crate::test_utils::git_command()
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

#[test]
fn baseline_clean_tree_is_valid_and_leaves_the_real_index_head_and_refs_unchanged() {
    let fixture = Fixture::new();
    let index = fs::read(fixture.index_path()).unwrap();
    let refs = git(&fixture.root, &["show-ref"]);
    let snapshot = fixture.snapshot().unwrap();
    assert!(snapshot.worktree_clean);
    assert_eq!(snapshot.head_commit_sha, fixture.base);
    assert_eq!(
        snapshot.tree_sha,
        git(&fixture.root, &["rev-parse", "HEAD^{tree}"])
    );
    assert_eq!(fs::read(fixture.index_path()).unwrap(), index);
    assert_eq!(git(&fixture.root, &["rev-parse", "HEAD"]), fixture.base);
    assert_eq!(git(&fixture.root, &["show-ref"]), refs);
}

#[test]
fn dirty_snapshot_matches_the_actual_host_commit_including_staged_ignored_files() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("README.md"), "changed\n").unwrap();
    fs::rename(
        fixture.root.join("removed.txt"),
        fixture.root.join("renamed.txt"),
    )
    .unwrap();
    fs::write(fixture.root.join("untracked.txt"), "new file\n").unwrap();
    fs::write(fixture.root.join("generated.bin"), b"ignored\0staged\n").unwrap();
    git(&fixture.root, &["add", "-f", "generated.bin"]);
    let index = fs::read(fixture.index_path()).unwrap();
    let status = git(
        &fixture.root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    );
    let refs = git(&fixture.root, &["show-ref"]);
    let snapshot = fixture.snapshot().unwrap();
    assert!(!snapshot.worktree_clean);
    assert_eq!(fs::read(fixture.index_path()).unwrap(), index);
    assert_eq!(
        git(
            &fixture.root,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        status
    );
    assert_eq!(git(&fixture.root, &["show-ref"]), refs);
    let files = git(
        &fixture.root,
        &["ls-tree", "-r", "--name-only", &snapshot.tree_sha],
    );
    assert!(files.lines().any(|path| path == "generated.bin"));
    assert!(!files.lines().any(|path| path == "removed.txt"));
    let host_commit = fixture.host_commit();
    assert_eq!(
        snapshot.tree_sha,
        git(
            &fixture.root,
            &["rev-parse", &format!("{host_commit}^{{tree}}")]
        )
    );
}

#[test]
fn changing_bytes_between_staging_passes_rejects_the_snapshot_and_preserves_index() {
    let fixture = Fixture::new();
    let path = fixture.root.join("README.md");
    fs::write(&path, "first\n").unwrap();
    let index = fs::read(fixture.index_path()).unwrap();
    let error = snapshot_with_hook(fixture.request(), || fs::write(&path, "second\n").unwrap())
        .unwrap_err();
    assert!(error.contains("changed"), "{error}");
    assert_eq!(fs::read(fixture.index_path()).unwrap(), index);
    assert_eq!(git(&fixture.root, &["rev-parse", "HEAD"]), fixture.base);
}

#[test]
fn a_restored_worktree_value_and_mtime_still_fails_the_change_time_fence() {
    let fixture = Fixture::new();
    let path = fixture.root.join("README.md");
    fs::write(&path, "first\n").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let result = snapshot_with_hook(fixture.request(), || {
        fs::write(&path, "other\n").unwrap();
        fs::write(&path, "first\n").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
    });
    assert!(result.is_err());
}

#[test]
fn replacing_the_real_index_with_identical_bytes_invalidates_the_observation() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("README.md"), "changed\n").unwrap();
    let index = fixture.index_path();
    let bytes = fs::read(&index).unwrap();
    let result = snapshot_with_hook(fixture.request(), || {
        fs::rename(&index, index.with_file_name("previous-index")).unwrap();
        fs::write(&index, &bytes).unwrap();
    });
    assert!(result.is_err());
    assert_eq!(fs::read(index).unwrap(), bytes);
    assert_eq!(git(&fixture.root, &["rev-parse", "HEAD"]), fixture.base);
}

#[test]
fn branch_or_head_changes_are_not_accepted_as_the_prepared_source() {
    let fixture = Fixture::new();
    assert!(
        snapshot_parallel_worker_tree(ParallelWorkerTreeRequest {
            expected_branch_name: "other",
            ..fixture.request()
        })
        .is_err()
    );
    assert!(
        snapshot_parallel_worker_tree(ParallelWorkerTreeRequest {
            expected_head_commit_sha: &"1".repeat(40),
            ..fixture.request()
        })
        .is_err()
    );
    let result = snapshot_with_hook(fixture.request(), || {
        git(&fixture.root, &["branch", "-m", "renamed"]);
    });
    assert!(result.is_err());
}

#[test]
fn external_filter_attributes_fail_before_staging_and_leave_index_unchanged() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.join(".gitattributes"),
        "README.md filter=untrusted\n",
    )
    .unwrap();
    fs::write(fixture.root.join("README.md"), "changed\n").unwrap();
    let index = fs::read(fixture.index_path()).unwrap();
    let error = fixture.snapshot().unwrap_err();
    assert!(error.contains("filter"), "{error}");
    assert_eq!(fs::read(fixture.index_path()).unwrap(), index);
}

#[test]
fn hidden_and_split_indexes_are_rejected_without_mutating_them() {
    let fixture = Fixture::new();
    git(
        &fixture.root,
        &["update-index", "--assume-unchanged", "README.md"],
    );
    let hidden = fs::read(fixture.index_path()).unwrap();
    assert!(fixture.snapshot().unwrap_err().contains("assume-unchanged"));
    assert_eq!(fs::read(fixture.index_path()).unwrap(), hidden);
    git(
        &fixture.root,
        &["update-index", "--no-assume-unchanged", "README.md"],
    );
    git(&fixture.root, &["update-index", "--split-index"]);
    let split = fs::read(fixture.index_path()).unwrap();
    assert!(fixture.snapshot().unwrap_err().contains("split index"));
    assert_eq!(fs::read(fixture.index_path()).unwrap(), split);
}

#[test]
fn changed_file_limits_fail_closed_before_allocating_or_staging_large_bytes() {
    let fixture = Fixture::new();
    let index = fs::read(fixture.index_path()).unwrap();
    fs::File::create(fixture.root.join("too-large.bin"))
        .unwrap()
        .set_len(MAX_CHANGED_FILE_BYTES + 1)
        .unwrap();
    assert!(fixture.snapshot().is_err());
    assert_eq!(fs::read(fixture.index_path()).unwrap(), index);
}

#[test]
fn frozen_commit_tree_reader_uses_exact_local_objects_without_head_or_index_changes() {
    let fixture = Fixture::new();
    let tree = git(&fixture.root, &["rev-parse", "HEAD^{tree}"]);
    fs::write(fixture.root.join("README.md"), "dirty content\n").unwrap();
    let index = fs::read(fixture.index_path()).unwrap();
    let runtime = GitParallelModeRuntimeAdapter::new();
    assert_eq!(
        runtime
            .read_commit_tree(fixture.root.to_str().unwrap(), &fixture.base)
            .unwrap(),
        tree
    );
    for invalid in ["HEAD", "main", &fixture.base[..10], &"1".repeat(40), &tree] {
        assert!(
            runtime
                .read_commit_tree(fixture.root.to_str().unwrap(), invalid)
                .is_err(),
            "{invalid}"
        );
    }
    assert_eq!(fs::read(fixture.index_path()).unwrap(), index);
    assert_eq!(git(&fixture.root, &["rev-parse", "HEAD"]), fixture.base);
}

#[test]
fn temporary_index_drop_removes_only_its_owned_files() {
    let fixture = Fixture::new();
    let bytes = fs::read(fixture.index_path()).unwrap();
    let temporary = index::TemporaryIndex::new(&bytes).unwrap();
    let directory = temporary.path().parent().unwrap().to_path_buf();
    assert_eq!(fs::read(temporary.path()).unwrap(), bytes);
    drop(temporary);
    assert!(!directory.exists());
}

#[cfg(unix)]
#[test]
fn tracked_symlink_snapshot_preserves_link_bytes_without_reading_the_target() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    symlink("../not-an-artifact", fixture.root.join("link")).unwrap();
    let tree = fixture.snapshot().unwrap().tree_sha;
    assert_eq!(
        git(&fixture.root, &["show", &format!("{tree}:link")]),
        "../not-an-artifact"
    );
    assert!(git(&fixture.root, &["ls-tree", &tree, "--", "link"]).starts_with("120000 "));
}

#[cfg(unix)]
#[test]
fn special_untracked_files_fail_before_git_add() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = Fixture::new();
    let index = fs::read(fixture.index_path()).unwrap();
    let fifo = std::ffi::CString::new(fixture.root.join("fifo").as_os_str().as_bytes()).unwrap();
    // SAFETY: the fixture path is NUL-terminated and remains live for mkfifo.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(fixture.snapshot().is_err());
    assert_eq!(fs::read(fixture.index_path()).unwrap(), index);
}

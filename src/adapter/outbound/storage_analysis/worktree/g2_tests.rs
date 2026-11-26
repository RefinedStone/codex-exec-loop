//! Real Git / native metadata regressions. Every repository belongs to this fixture.
use super::super::{NativeStorageAnalysisAdapter, StorageRuntimeProbe, StorageRuntimeRoots};
use crate::application::port::outbound::storage_analysis_port::StorageAnalysisPort;
use crate::domain::storage_analysis::{ScanOptions, ScanRequest, StorageEvidence, StorageNode};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, SystemTime},
};

struct Fixture {
    base: PathBuf,
    source: PathBuf,
    adapter: NativeStorageAnalysisAdapter,
}
impl Fixture {
    fn new() -> Self {
        let base = crate::test_utils::platform_safe_temp_dir()
            .join(format!("storage-g2-{:032x}", rand::random::<u128>()));
        let source = base.join("source");
        fs::create_dir_all(&source).unwrap();
        git(&source, &["init", "-q", "-b", "main"]);
        let adapter = NativeStorageAnalysisAdapter::fixture(base.join("index"), source.clone());
        Self {
            base,
            source,
            adapter,
        }
    }
    fn write(&self, path: &str, contents: &str) {
        let path = self.source.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    fn commit(&self) {
        git(&self.source, &["add", "."]);
        git(
            &self.source,
            &[
                "-c",
                "user.name=Storage Fixture",
                "-c",
                "user.email=storage@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        );
    }
    fn scan(&self, id: &str) -> StorageNode {
        self.adapter
            .scan(
                &ScanRequest {
                    job_id: id.into(),
                    volume_id: "fixture-volume".into(),
                    options: ScanOptions::default(),
                },
                &AtomicBool::new(false),
                &|_| {},
            )
            .unwrap();
        self.adapter.get_node(id, "n1", None).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
fn git(path: &Path, args: &[&str]) -> Vec<u8> {
    let mut command = crate::test_utils::git_command();
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .args([
            "-c",
            if cfg!(windows) {
                "core.hooksPath=NUL"
            } else {
                "core.hooksPath=/dev/null"
            },
            "-c",
            "commit.gpgSign=false",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.autocrlf=false",
        ])
        .arg("-C")
        .arg(path)
        .args(args);
    let output = crate::subprocess::command_output_with_timeout(
        &mut command,
        "storage G2 fixture Git",
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn reason<'a>(node: &'a StorageNode, code: &str) -> &'a StorageEvidence {
    node.protection
        .iter()
        .find(|reason| reason.code == code)
        .unwrap_or_else(|| panic!("missing {code}: {:?}", node.protection))
}

#[test]
fn storage_analysis_g2_stat_and_repository_ignore_match_real_git_without_mutation() {
    let mut f = Fixture::new();
    for file in [
        "same.txt",
        "size.txt",
        "time.txt",
        "missing.txt",
        "assumed.txt",
        "skipped.txt",
    ] {
        f.write(file, "original");
    }
    f.write(
        ".gitignore",
        "target/\n*.tmp\n!keep.tmp\nignored/*\n!ignored/visible.txt\n",
    );
    f.write("nested/.gitignore", "*.bin\n!keep.bin\n");
    f.commit();
    git(
        &f.source,
        &["update-index", "--assume-unchanged", "assumed.txt"],
    );
    git(
        &f.source,
        &["update-index", "--skip-worktree", "skipped.txt"],
    );
    for file in ["missing.txt", "assumed.txt", "skipped.txt"] {
        fs::remove_file(f.source.join(file)).unwrap();
    }
    f.write("size.txt", "different file size");
    fs::OpenOptions::new()
        .write(true)
        .open(f.source.join("time.txt"))
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(SystemTime::now() + Duration::from_secs(2)))
        .unwrap();
    f.write(".git/info/exclude", "info-only.log\n");
    for file in [
        "target/cache",
        "ignored/no.txt",
        "ignored/visible.txt",
        "a b.tmp",
        "keep.tmp",
        "plain.txt",
        "nested/drop.bin",
        "nested/keep.bin",
        "info-only.log",
    ] {
        f.write(file, "local fixture data");
    }
    #[cfg(unix)]
    f.write("line\nname.tmp", "local fixture data");
    let index = fs::read(f.source.join(".git/index")).unwrap();
    let config = fs::read(f.source.join(".git/config")).unwrap();
    f.write(".git/index.lock", "another Git operation");
    f.adapter = NativeStorageAnalysisAdapter::fixture(f.base.join("index"), f.source.clone())
        .with_runtime_probe(StorageRuntimeProbe {
            known_roots: Arc::new(|_| StorageRuntimeRoots {
                paths: Vec::new(),
                complete: true,
            }),
            observe: Arc::new(|_| {
                vec![StorageEvidence {
                    code: "active_session".into(),
                    detail: "fixture active session".into(),
                    observed_at: "fixture".into(),
                }]
            }),
        });
    let node = f.scan("stat-ignore");
    let changes = &reason(&node, "worktree_stat_changes").detail;
    assert!(changes.contains("없음 1"), "{changes}");
    assert!(changes.contains("크기 1"), "{changes}");
    assert!(
        changes.contains("추적 경로 3개"),
        "unchanged and special index paths must not become dirty: {changes}"
    );
    assert!(
        reason(&node, "worktree_stat_unknown")
            .detail
            .contains("2개")
    );
    for code in [
        "main_checkout",
        "active_session",
        "git_operation_locked",
        "status_unknown",
        "upstream_unknown",
    ] {
        reason(&node, code);
    }
    assert!(
        !node
            .protection
            .iter()
            .any(|reason| reason.code == "ignore_unknown")
    );
    let actual = git(
        &f.source,
        &[
            "-c",
            if cfg!(windows) {
                "core.excludesFile=NUL"
            } else {
                "core.excludesFile=/dev/null"
            },
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    );
    let count = actual
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .count();
    assert!(
        reason(&node, "untracked_files")
            .detail
            .contains(&format!("{count}개"))
    );
    assert_eq!(count, 4);
    assert!(
        reason(&node, "ignore_rules_observed")
            .detail
            .contains("전역 exclude는 읽지 않았")
    );
    assert_eq!(index, fs::read(f.source.join(".git/index")).unwrap());
    assert_eq!(config, fs::read(f.source.join(".git/config")).unwrap());
    assert_eq!(
        fs::read_to_string(f.source.join(".git/index.lock")).unwrap(),
        "another Git operation"
    );
}

#[cfg(windows)]
#[test]
fn storage_analysis_g2_reads_stat_when_working_file_data_reads_are_denied() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE;
    let f = Fixture::new();
    f.write("tracked.txt", "original");
    f.commit();
    f.write("tracked.txt", "changed length without any content read");
    let file = f.source.join("tracked.txt");
    let guard = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_WRITE)
        .open(&file)
        .unwrap();
    assert!(fs::read(&file).is_err(), "fixture must deny data reads");
    let node = f.scan("read-denied");
    assert!(
        reason(&node, "worktree_stat_changes")
            .detail
            .contains("크기 1")
    );
    assert!(
        !node
            .protection
            .iter()
            .any(|reason| reason.code == "worktree_stat_unknown")
    );
    drop(guard);
}

#[test]
fn storage_analysis_g2_unavailable_ignore_rules_preserve_changes_and_unknown_ownership() {
    let f = Fixture::new();
    f.write("tracked.txt", "original");
    f.commit();
    f.write("tracked.txt", "changed size");
    f.write("untracked.txt", "local");
    let outside = f.base.join("outside-ignore");
    fs::write(&outside, "untracked.txt\n").unwrap();
    git(
        &f.source,
        &["config", "core.excludesFile", outside.to_str().unwrap()],
    );
    let node = f.scan("outside-exclude");
    for code in [
        "ignore_unknown",
        "untracked_observed",
        "worktree_stat_changes",
        "runtime_unknown",
        "upstream_unknown",
    ] {
        reason(&node, code);
    }
    assert!(
        !node
            .protection
            .iter()
            .any(|reason| reason.code == "ignore_rules_observed")
    );
    git(&f.source, &["config", "--unset", "core.excludesFile"]);
    f.write(".gitignore", &"#".repeat(64 * 1024 + 1));
    let node = f.scan("oversize-ignore");
    for code in [
        "ignore_unknown",
        "untracked_observed",
        "worktree_stat_changes",
    ] {
        reason(&node, code);
    }
    assert!(
        !node
            .protection
            .iter()
            .any(|reason| reason.code == "ignore_rules_observed")
    );
}

#[test]
fn storage_analysis_g2_local_upstream_evidence_is_not_publication_or_external_ownership() {
    let f = Fixture::new();
    f.write("tracked.txt", "first");
    f.commit();
    git(
        &f.source,
        &[
            "config",
            "remote.origin.url",
            "https://example.invalid/fixture.git",
        ],
    );
    git(
        &f.source,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    git(
        &f.source,
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    );
    git(&f.source, &["branch", "--set-upstream-to", "origin/main"]);
    f.write("tracked.txt", "second");
    f.commit();
    let node = f.scan("ahead");
    reason(&node, "local_commits");
    reason(&node, "runtime_unknown");
    git(&f.source, &["update-ref", "-d", "refs/remotes/origin/main"]);
    let node = f.scan("missing-upstream");
    reason(&node, "upstream_unknown");
    git(&f.source, &["checkout", "--detach", "-q"]);
    let node = f.scan("detached");
    reason(&node, "detached_head");
    reason(&node, "upstream_unknown");
    reason(&node, "runtime_unknown");
}

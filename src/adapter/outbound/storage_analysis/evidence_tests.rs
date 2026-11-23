//! Native boundary regressions for observed repository evidence and owned-file cleanup.
use super::*;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    base: PathBuf,
    source: PathBuf,
    index: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = crate::test_utils::platform_safe_temp_dir()
            .join(format!("storage-evidence-{:032x}", rand::random::<u128>()));
        let source = base.join("source");
        let index = base.join("index");
        fs::create_dir_all(&source).unwrap();
        Self {
            base,
            source,
            index,
        }
    }
    fn adapter(&self) -> NativeStorageAnalysisAdapter {
        NativeStorageAnalysisAdapter::fixture(self.index.clone(), self.source.clone())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
fn scan(adapter: &NativeStorageAnalysisAdapter, id: &str) {
    adapter
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
}
#[test]
fn storage_analysis_known_runtime_root_is_protected_without_git_discovery() {
    let fixture = Fixture::new();
    fs::write(fixture.source.join("data"), "fixture").unwrap();
    let root = fixture.source.clone();
    let outside = fixture.base.join("outside");
    let calls = Arc::new(AtomicUsize::new(0));
    let observations = calls.clone();
    let adapter = fixture.adapter().with_runtime_probe(StorageRuntimeProbe {
        known_roots: Arc::new(move |limit| {
            assert_eq!(limit, 256);
            StorageRuntimeRoots {
                paths: vec![root.clone(), outside.clone()],
                complete: false,
            }
        }),
        observe: Arc::new(move |_| {
            observations.fetch_add(1, Ordering::AcqRel);
            vec![StorageEvidence {
                code: "active_session".into(),
                detail: "fixture existing session".into(),
                observed_at: chrono::Utc::now().to_rfc3339(),
            }]
        }),
    });
    scan(&adapter, "known-root");
    assert_eq!(
        calls.load(Ordering::Acquire),
        1,
        "an outside-volume path must not be observed"
    );
    let node = adapter.get_node("known-root", "n1", None).unwrap();
    for code in [
        "akra_known_repository",
        "active_session",
        "runtime_candidates_partial",
        "git_metadata_unverified",
        "status_unknown",
    ] {
        assert!(
            node.protection.iter().any(|reason| reason.code == code),
            "missing {code}"
        );
    }
    let db = index::StorageIndex::open(&fixture.index.join("known-root")).unwrap();
    let queried: u64 = db
        .conn
        .query_row("SELECT count(*) FROM git_worktree_cache", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        queried, 0,
        "known roots without observed Git metadata must not launch Git"
    );
}
#[test]
fn storage_analysis_cleanup_preserves_file_created_after_validation() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    scan(&adapter, "cleanup-race");
    let dir = fixture.index.join("cleanup-race");
    validate_cleanup_entries(&dir).unwrap();
    let foreign = dir.join("new-user-note.txt");
    fs::write(&foreign, "keep this").unwrap();
    assert!(remove_validated_index_files(&dir).is_err());
    assert_eq!(fs::read_to_string(&foreign).unwrap(), "keep this");
    assert!(
        dir.is_dir(),
        "the foreign file prevents removal of its parent"
    );
}

fn fixture_git(root: &Path, args: &[&str]) {
    let mut command = crate::git_subprocess::command(["-C"]);
    command.arg(root).args(args);
    let output = crate::subprocess::command_output_with_timeout(
        &mut command,
        "storage registration fixture Git",
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn fixture_commit(root: &Path) {
    fixture_git(root, &["add", "."]);
    fixture_git(
        root,
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

#[test]
fn storage_analysis_submodule_requires_registered_parent_gitlink() {
    let fixture = Fixture::new();
    let upstream = fixture.base.join("upstream-child");
    fs::create_dir(&upstream).unwrap();
    fixture_git(&upstream, &["init", "-q"]);
    fs::write(upstream.join("tracked.txt"), "child fixture").unwrap();
    fixture_commit(&upstream);

    fixture_git(&fixture.source, &["init", "-q"]);
    fs::write(fixture.source.join("tracked.txt"), "parent fixture").unwrap();
    fixture_commit(&fixture.source);
    fixture_git(
        &fixture.source,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "--quiet",
            upstream.to_str().unwrap(),
            "registered-child",
        ],
    );
    // This repository has a genuine .git pointer into .git/modules, but the parent
    // index does not register its working directory as a gitlink.
    let lookalike = fixture.source.join("unregistered-child");
    let lookalike_metadata = fixture.source.join(".git/modules/unregistered-child");
    fixture_git(
        &fixture.source,
        &[
            "init",
            "-q",
            "--separate-git-dir",
            lookalike_metadata.to_str().unwrap(),
            lookalike.to_str().unwrap(),
        ],
    );
    // A separate Git directory needs an explicit main-worktree registration.
    // The checkout remains genuine, but no parent gitlink points to it.
    fixture_git(
        &lookalike,
        &["config", "core.worktree", lookalike.to_str().unwrap()],
    );
    fs::write(lookalike.join("tracked.txt"), "unregistered fixture").unwrap();
    fixture_commit(&lookalike);
    let index_before = fs::read(fixture.source.join(".git/index")).unwrap();

    let adapter = fixture.adapter();
    scan(&adapter, "submodule-registration");
    let db = index::StorageIndex::open(&fixture.index.join("submodule-registration")).unwrap();
    for (name, expected) in [("registered-child", true), ("unregistered-child", false)] {
        let id: i64 = db
            .conn
            .query_row(
                "SELECT id FROM nodes WHERE parent=1 AND name=?1 AND kind='directory'",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        let node = adapter
            .get_node("submodule-registration", &format!("n{id}"), None)
            .unwrap();
        assert!(
            node.protection
                .iter()
                .any(|reason| reason.code == "main_checkout"),
            "{name} must pass Git metadata and worktree-registration verification: {:?}",
            node.protection
        );
        assert_eq!(
            node.protection
                .iter()
                .any(|reason| reason.code == "submodule"),
            expected,
            "{name}: {:?}",
            node.protection
        );
        let gitlinks: u64 = db
            .conn
            .query_row(
                "SELECT count(*) FROM git_tracked_paths WHERE repository=1 AND relative_path=?1 AND stage=0 AND mode=?2",
                rusqlite::params![name,0o160000u32],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(gitlinks, u64::from(expected));
    }
    assert_eq!(
        fs::read(fixture.source.join(".git/index")).unwrap(),
        index_before,
        "observation must preserve the parent index"
    );
    let root = adapter
        .get_node("submodule-registration", "n1", None)
        .unwrap();
    assert!(
        !root
            .protection
            .iter()
            .any(|reason| reason.code == "submodule")
    );
}

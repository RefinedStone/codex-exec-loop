//! Native boundary regressions for observed runtime roots and owned-file cleanup.
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

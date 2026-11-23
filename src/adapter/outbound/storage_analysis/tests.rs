use super::index::{Observation, StorageIndex};
use super::*;
use crate::domain::storage_analysis::hotspots::should_expand;
use std::{fs, sync::atomic::Ordering};

struct Fixture {
    base: PathBuf,
    source: PathBuf,
    index: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base =
            std::env::temp_dir().join(format!("akra-storage-test-{}", rand::random::<u64>()));
        let source = base.join("source");
        let index = base.join("indexes");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&index).unwrap();
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
fn query(id: &str, node: &str) -> StorageQuery {
    StorageQuery {
        snapshot_id: id.into(),
        node_id: node.into(),
        limit: 100,
        cursor: None,
        sort: StorageSort::AllocatedDesc,
        name_filter: None,
        extension_filter: None,
        recursive: false,
        scope_root_id: None,
    }
}
fn request(id: &str) -> ScanRequest {
    ScanRequest {
        job_id: id.into(),
        volume_id: "fixture-volume".into(),
        options: ScanOptions { concurrency: 2 },
    }
}
fn run(adapter: &NativeStorageAnalysisAdapter, id: &str) -> StorageSnapshot {
    adapter
        .scan(&request(id), &AtomicBool::new(false), &|_| {})
        .unwrap()
}

#[test]
fn storage_analysis_real_metadata_hardlinks_and_no_reenumeration() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.source.join("branch")).unwrap();
    fs::write(fixture.source.join("a.bin"), vec![1_u8; 8192]).unwrap();
    fs::write(fixture.source.join("branch/b.bin"), vec![2_u8; 4096]).unwrap();
    fs::hard_link(
        fixture.source.join("a.bin"),
        fixture.source.join("branch/link.bin"),
    )
    .unwrap();
    let adapter = fixture.adapter();
    let s = run(&adapter, "fixture");
    assert_eq!(s.state, StorageState::Complete);
    assert_eq!(s.files_scanned, 3);
    let root = adapter.get_node("fixture", "n1", None).unwrap();
    let page = adapter.list_children(&query("fixture", "n1")).unwrap();
    let total: u64 = page.items.iter().map(StorageNode::known_bytes).sum();
    assert_eq!(root.known_bytes(), total);
    assert!(
        page.items
            .iter()
            .any(|n| n.kind == StorageNodeKind::SharedFiles)
    );
    // Removing original content cannot affect frozen query results or cause a second traversal.
    fs::remove_dir_all(&fixture.source).unwrap();
    let again = adapter.list_children(&query("fixture", "n1")).unwrap();
    assert_eq!(again, page);
    let mut files = query("fixture", "n1");
    files.recursive = true;
    assert_eq!(adapter.list_files(&files).unwrap().items.len(), 3);
    adapter.cleanup("fixture").unwrap();
    assert!(!fixture.index.join("fixture").exists());
}

#[test]
fn storage_analysis_paging_scope_and_cursor_are_bound() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.source.join("private")).unwrap();
    for name in ["a.bin", "b.bin", "c.txt"] {
        fs::write(fixture.source.join(name), [0; 4096]).unwrap();
    }
    fs::write(fixture.source.join("private/secret.txt"), [0; 4096]).unwrap();
    let adapter = fixture.adapter();
    run(&adapter, "pages");
    let mut q = query("pages", "n1");
    q.limit = 1;
    let first = adapter.list_files(&q).unwrap();
    q.cursor = first.next_cursor.clone();
    let second = adapter.list_files(&q).unwrap();
    assert_ne!(first.items[0].id, second.items[0].id);
    q.sort = StorageSort::Name;
    assert!(adapter.list_files(&q).is_err());
    let private = adapter
        .list_children(&query("pages", "n1"))
        .unwrap()
        .items
        .into_iter()
        .find(|n| n.name == "private")
        .unwrap();
    let scoped = adapter
        .get_node("pages", &private.id, Some(&private.id))
        .unwrap();
    assert!(scoped.parent_id.is_none());
    assert!(scoped.ratio.denominator_bytes.is_none());
    assert!(adapter.get_node("pages", "n1", Some(&private.id)).is_err());
    assert!(!adapter.is_descendant("pages", "n1", &private.id).unwrap());
    assert!(
        adapter
            .get_node("pages", "n9999", Some(&private.id))
            .is_err()
    );
}

#[test]
fn storage_analysis_cancel_before_dispatch_never_enumerates() {
    let fixture = Fixture::new();
    fs::write(fixture.source.join("file"), [0; 4096]).unwrap();
    let adapter = fixture.adapter();
    let cancelled = AtomicBool::new(true);
    let s = adapter
        .scan(&request("cancel"), &cancelled, &|_| {})
        .unwrap();
    assert_eq!(s.state, StorageState::Cancelled);
    assert_eq!(s.files_scanned, 0);
    assert_eq!(s.directories_scanned, 0);
    assert_eq!(
        adapter.get_node("cancel", "n1", None).unwrap().state,
        StorageState::Partial
    );
}
fn directory(db: &StorageIndex, parent: i64, name: &str) -> i64 {
    db.record(Observation {
        parent,
        name: name.into(),
        path: PathBuf::from(name),
        directory: true,
        logical: 0,
        allocated: Some(0),
        identity: Some(name.into()),
        issue: None,
    })
    .unwrap()
}
fn file(db: &StorageIndex, parent: i64, name: &str, bytes: u64, identity: &str) -> i64 {
    db.record(Observation {
        parent,
        name: name.into(),
        path: PathBuf::from(name),
        directory: false,
        logical: bytes,
        allocated: Some(bytes),
        identity: Some(identity.into()),
        issue: None,
    })
    .unwrap()
}
fn synthetic(fixture: &Fixture, id: &str) -> StorageIndex {
    let dir = fixture.index.join(id);
    fs::create_dir(&dir).unwrap();
    StorageIndex::create(
        &dir,
        id,
        &StorageVolume {
            id: "fixture-volume".into(),
            label: "fixture".into(),
            root_path: fixture.source.to_string_lossy().into_owned(),
            kind: "fixture".into(),
            supported: true,
        },
    )
    .unwrap()
}
fn finish(db: &mut StorageIndex) {
    db.conn.execute("UPDATE nodes SET pending=0", []).unwrap();
    db.aggregate().unwrap();
    db.finish(StorageState::Complete, None).unwrap();
}

#[test]
fn storage_analysis_hardlink_lca_is_independent_of_all_discovery_orders() {
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let f = Fixture::new();
        let mut db = synthetic(&f, "lca");
        let a = directory(&db, 1, "A");
        let b = directory(&db, a, "B");
        let c = directory(&db, 1, "C");
        let parents = [a, b, c];
        for i in order {
            file(&db, parents[i], &format!("link{i}"), 4096, "same-file");
        }
        finish(&mut db);
        assert_eq!(db.get_node("n1", None).unwrap().known_bytes(), 4096);
        assert_eq!(
            db.get_node(&format!("n{a}"), None).unwrap().known_bytes(),
            0
        );
        assert_eq!(
            db.get_node(&format!("n{b}"), None).unwrap().known_bytes(),
            0
        );
        let children = db.page(&query("lca", "n1"), false).unwrap();
        assert_eq!(
            children
                .items
                .iter()
                .filter(|n| n.kind == StorageNodeKind::SharedFiles)
                .map(StorageNode::known_bytes)
                .sum::<u64>(),
            4096
        );
    }
}

#[test]
fn storage_analysis_four_examples_conserve_nonoverlapping_hotspots() {
    let f = Fixture::new();
    let mut db = synthetic(&f, "examples");
    let gib = 1024_u64.pow(3);
    let mib = 1024_u64.pow(2);
    let cache = directory(&db, 1, "cache");
    for i in 0..16 {
        let c = directory(&db, cache, &format!("c{i}"));
        file(&db, c, "data", 64 * mib, &format!("cache{i}"));
    }
    let build = directory(&db, 1, "build");
    let obj = directory(&db, build, "obj");
    file(&db, obj, "data", 768 * mib, "object");
    file(&db, build, "direct", 256 * mib, "direct");
    let dev = directory(&db, 1, "dev");
    let tree = directory(&db, dev, "worktree");
    file(&db, tree, "data", 3 * gib, "worktree");
    file(&db, dev, "rest", 97 * gib, "dev-rest");
    let downloads = directory(&db, 1, "downloads");
    file(&db, downloads, "image", 8 * gib, "image");
    finish(&mut db);
    let summary = db
        .hotspots(&query("examples", "n1"), &HotspotThresholds::default())
        .unwrap();
    assert_eq!(
        summary
            .items
            .iter()
            .map(|x| x.included_bytes.parse::<u64>().unwrap())
            .sum::<u64>(),
        110 * gib
    );
    assert!(
        summary
            .items
            .iter()
            .any(|x| x.node.id == format!("n{cache}")
                && x.role == "folder"
                && x.included_bytes == gib.to_string())
    );
    assert!(
        summary
            .items
            .iter()
            .any(|x| x.node.id == format!("n{build}")
                && x.role == "remainder"
                && x.included_bytes == (256 * mib).to_string())
    );
    assert!(
        summary
            .items
            .iter()
            .any(|x| x.node.id == format!("n{tree}") && x.role == "folder")
    );
    assert!(
        summary
            .items
            .iter()
            .any(|x| x.node.id == format!("n{downloads}") && x.role == "folder")
    );
    let t = HotspotThresholds::default();
    assert!(should_expand(3 * gib, 100 * gib, &t).unwrap());
}

#[test]
fn storage_analysis_identity_unknown_is_not_a_known_unique_allocation() {
    let f = Fixture::new();
    let mut db = synthetic(&f, "unknown");
    db.record(Observation {
        parent: 1,
        name: "unknown".into(),
        path: "unknown".into(),
        directory: false,
        logical: 8000,
        allocated: Some(8192),
        identity: None,
        issue: Some(StorageIssue::one("identity_unknown", "Cannot deduplicate")),
    })
    .unwrap();
    file(&db, 1, "sparse", 0, "sparse");
    finish(&mut db);
    let root = db.get_node("n1", None).unwrap();
    assert!(root.allocated_bytes.is_none());
    assert_eq!(root.known_bytes(), 0);
    let entries = db.page(&query("unknown", "n1"), true).unwrap();
    assert_eq!(
        entries
            .items
            .iter()
            .find(|n| n.name == "sparse")
            .unwrap()
            .allocated_bytes
            .as_deref(),
        Some("0")
    );
}

#[test]
fn storage_analysis_cancel_during_scan_saves_terminal_state() {
    let fixture = Fixture::new();
    for i in 0..20 {
        fs::create_dir(fixture.source.join(format!("d{i}"))).unwrap();
    }
    let adapter = fixture.adapter();
    let cancel = AtomicBool::new(false);
    let s = adapter
        .scan(&request("cancel-progress"), &cancel, &|_| {
            cancel.store(true, Ordering::Release);
        })
        .unwrap();
    assert_eq!(s.state, StorageState::Cancelled);
    assert_eq!(s.directories_scanned, 0);
}

#[cfg(windows)]
#[test]
fn storage_analysis_windows_junction_does_not_escape_or_cycle() {
    let f = Fixture::new();
    let junction = f.source.join("loop");
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&f.source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "directory junction fixture should be available without symlink privilege"
    );
    let adapter = f.adapter();
    let s = run(&adapter, "junction");
    assert_eq!(s.state, StorageState::Partial);
    let loop_node = adapter
        .list_children(&query("junction", "n1"))
        .unwrap()
        .items
        .into_iter()
        .find(|n| n.name == "loop")
        .unwrap();
    assert_eq!(loop_node.state, StorageState::Partial);
    assert!(loop_node.allocated_bytes.is_none());
    fs::remove_dir(junction).unwrap();
}

#[test]
#[ignore = "explicit local filesystem measurement; never scans outside its generated fixture"]
fn storage_analysis_bounded_fixture_benchmark() {
    use std::sync::Mutex;
    use std::time::Instant;
    let fixture = Fixture::new();
    for group in 0..16 {
        let dir = fixture.source.join(format!("small-{group}"));
        fs::create_dir(&dir).unwrap();
        for i in 0..128 {
            fs::write(dir.join(format!("{i:04}.bin")), vec![1_u8; 1024]).unwrap();
        }
    }
    for group in 0..4 {
        let dir = fixture.source.join(format!("large-{group}"));
        fs::create_dir(&dir).unwrap();
        for i in 0..8 {
            fs::write(dir.join(format!("{i:04}.bin")), vec![2_u8; 1024 * 1024]).unwrap();
        }
    }
    let adapter = fixture.adapter();
    let mut expected = None;
    for concurrency in [1, 4] {
        for run in 0..3 {
            let id = format!("benchmark-{concurrency}-{run}");
            let started = Instant::now();
            let timing = Mutex::new((None, None));
            let req = ScanRequest {
                job_id: id.clone(),
                volume_id: "fixture-volume".into(),
                options: ScanOptions { concurrency },
            };
            let snapshot = adapter
                .scan(&req, &AtomicBool::new(false), &|progress| {
                    let mut t = timing.lock().unwrap();
                    if t.0.is_none() {
                        t.0 = Some(started.elapsed().as_micros());
                    }
                    if progress.files_scanned > 0 && t.1.is_none() {
                        t.1 = Some(started.elapsed().as_micros());
                    }
                })
                .unwrap();
            let elapsed = started.elapsed().as_micros();
            let root = adapter.get_node(&id, "n1", None).unwrap();
            let actual = (
                snapshot.files_scanned,
                root.known_allocated_bytes.clone(),
                snapshot.state.clone(),
            );
            if let Some(expected) = &expected {
                assert_eq!(&actual, expected);
            } else {
                expected = Some(actual.clone());
            }
            assert_eq!(actual.0, 2080);
            assert_eq!(actual.2, StorageState::Complete);
            let t = timing.lock().unwrap();
            let index_bytes = fs::metadata(fixture.index.join(&id).join("index.sqlite3"))
                .unwrap()
                .len();
            eprintln!(
                "{}",
                serde_json::json!({"kind":"storage_fixture","concurrency":concurrency,"run":run,
                "first_progress_us":t.0,"first_files_us":t.1,"elapsed_us":elapsed,"index_bytes":index_bytes,
                "files":actual.0,"known_allocated_bytes":actual.1,"device_kind":"unmeasured",
                "cold_cache_controlled":false,"hdd_measured":false})
            );
        }
    }
    let cancelled = AtomicBool::new(false);
    let cancel_at = Mutex::new(None);
    let started = Instant::now();
    let snapshot = adapter
        .scan(
            &ScanRequest {
                job_id: "benchmark-cancel".into(),
                volume_id: "fixture-volume".into(),
                options: ScanOptions { concurrency: 4 },
            },
            &cancelled,
            &|progress| {
                if progress.files_scanned > 0 && progress.state == StorageState::Running {
                    let mut at = cancel_at.lock().unwrap();
                    if at.is_none() {
                        *at = Some((Instant::now(), progress.files_scanned));
                        cancelled.store(true, Ordering::Release);
                    }
                }
            },
        )
        .unwrap();
    let at = cancel_at.lock().unwrap();
    let (cancel_time, observed) =
        at.expect("first observed file must publish progress before completion");
    assert_eq!(snapshot.state, StorageState::Cancelled);
    assert!(snapshot.files_scanned < 2080);
    eprintln!(
        "{}",
        serde_json::json!({"kind":"storage_fixture_cancel","cancel_to_terminal_us":cancel_time.elapsed().as_micros(),
        "elapsed_us":started.elapsed().as_micros(),"observed_when_cancelled":observed,"observed_at_terminal":snapshot.files_scanned,
        "directories_completed":snapshot.directories_scanned,"state":snapshot.state})
    );
}

#[test]
fn storage_analysis_reveal_rejects_replaced_paths_and_cleanup_keeps_unowned_data() {
    let f = Fixture::new();
    fs::write(f.source.join("file.bin"), [0; 4096]).unwrap();
    let adapter = f.adapter();
    run(&adapter, "ownership");
    let file = adapter
        .list_files(&query("ownership", "n1"))
        .unwrap()
        .items
        .remove(0);
    assert!(adapter.reveal_path("ownership", &file.id).is_ok());
    fs::rename(f.source.join("file.bin"), f.source.join("original.bin")).unwrap();
    fs::write(f.source.join("file.bin"), [1; 4096]).unwrap();
    assert!(adapter.reveal_path("ownership", &file.id).is_err());
    let unowned = f.index.join("ownership/user-note.txt");
    fs::write(&unowned, "keep").unwrap();
    assert!(adapter.cleanup("ownership").is_err());
    assert!(unowned.exists());
    fs::remove_file(unowned).unwrap();
    adapter.cleanup("ownership").unwrap();
}

#[test]
fn storage_analysis_volume_loss_is_distinct_from_missing_file() {
    let changed = anyhow!("storage_volume_changed");
    assert_eq!(native::classify_error(&changed).code, "volume_changed");
    let missing = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::NotFound));
    assert_eq!(native::classify_error(&missing).code, "disappeared");
    let device = anyhow::Error::from(std::io::Error::from_raw_os_error(1167));
    assert_eq!(native::classify_error(&device).code, "drive_disconnected");
}

#[test]
fn storage_analysis_orphan_scan_is_interrupted_without_automatic_restart() {
    let f = Fixture::new();
    drop(synthetic(&f, "orphan"));
    let adapter = f.adapter();
    assert_eq!(
        adapter.snapshot("orphan").unwrap().state,
        StorageState::Interrupted
    );
    assert!(adapter.get_node("orphan", "n1", None).is_err());
}

#[test]
fn storage_analysis_git_observation_preserves_index_and_protection_reasons() {
    let f = Fixture::new();
    let git = |args: &[&str]| {
        let mut command = crate::git_subprocess::command(["-C"]);
        command.arg(&f.source).args(args);
        let output = crate::subprocess::command_output_with_timeout(
            &mut command,
            "storage fixture Git",
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    fs::write(f.source.join("tracked.txt"), "original").unwrap();
    git(&["add", "tracked.txt"]);
    git(&[
        "-c",
        "user.name=Storage Fixture",
        "-c",
        "user.email=storage@example.invalid",
        "commit",
        "-qm",
        "fixture",
    ]);
    fs::write(f.source.join("tracked.txt"), "modified").unwrap();
    fs::write(f.source.join("untracked.txt"), "local data").unwrap();
    fs::write(f.source.join("Cargo.toml"), "[package]\nname='fixture'\n").unwrap();
    fs::create_dir(f.source.join("target")).unwrap();
    fs::write(f.source.join("target/cache"), "cache").unwrap();
    let before = fs::read(f.source.join(".git/index")).unwrap();
    let adapter = f.adapter().with_runtime_probe(Arc::new(|_| {
        vec![StorageEvidence {
            code: "active_slot".into(),
            detail: "fixture running slot".into(),
            observed_at: "fixture".into(),
        }]
    }));
    run(&adapter, "git-evidence");
    assert_eq!(before, fs::read(f.source.join(".git/index")).unwrap());
    let root = adapter.get_node("git-evidence", "n1", None).unwrap();
    for code in [
        "main_checkout",
        "active_slot",
        "modified_files",
        "untracked_files",
        "upstream_unknown",
    ] {
        assert!(
            root.protection.iter().any(|e| e.code == code),
            "missing {code}: {:?}",
            root.protection
        );
    }
    let target = adapter
        .list_children(&query("git-evidence", "n1"))
        .unwrap()
        .items
        .into_iter()
        .find(|n| n.name == "target")
        .unwrap();
    assert!(
        target
            .classification
            .iter()
            .any(|c| c.kind == "candidate_target")
    );
    assert!(
        !serde_json::to_string(&target)
            .unwrap()
            .contains("deletable")
    );
}

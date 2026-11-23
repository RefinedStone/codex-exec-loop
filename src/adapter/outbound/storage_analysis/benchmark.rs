//! Reproducible fixture comparison. This never enumerates a user's volume.
use super::*;
use std::fs;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Instant;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-storage-comparison-{:032x}",
            rand::random::<u128>()
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn source(
    fixture: &Fixture,
    name: &str,
    folders: usize,
    files_per_folder: usize,
    bytes: usize,
) -> PathBuf {
    let root = fixture.0.join(name);
    fs::create_dir(&root).unwrap();
    for folder in 0..folders {
        let dir = if folders == 1 {
            root.clone()
        } else {
            root.join(format!("d{folder:02}"))
        };
        fs::create_dir_all(&dir).unwrap();
        for file in 0..files_per_folder {
            fs::write(dir.join(format!("{file:04}.bin")), vec![1_u8; bytes]).unwrap();
        }
    }
    root
}
fn query(id: &str) -> StorageQuery {
    StorageQuery {
        snapshot_id: id.into(),
        node_id: "n1".into(),
        limit: 10,
        cursor: None,
        sort: StorageSort::AllocatedDesc,
        name_filter: None,
        extension_filter: Some("bin".into()),
        recursive: true,
        scope_root_id: None,
    }
}

#[test]
#[ignore = "explicit local fixture timing; no cold-cache or native UI-load claim"]
fn storage_analysis_comparison_benchmark() {
    let fixture = Fixture::new();
    let cases = [
        (
            "small_many",
            source(&fixture, "small", 32, 128, 1024),
            4096_u64,
            4096_u64 * 1024,
            33_u64,
        ),
        (
            "large_few",
            source(&fixture, "large", 4, 8, 1024 * 1024),
            32_u64,
            32_u64 * 1024 * 1024,
            5_u64,
        ),
        (
            "flat_many",
            source(&fixture, "flat", 1, 2048, 1024),
            2048_u64,
            2048_u64 * 1024,
            1_u64,
        ),
    ];
    for (name, root, files, logical_bytes, directories) in cases {
        let index = fixture.0.join(format!("index-{name}"));
        let adapter = NativeStorageAnalysisAdapter::fixture(index.clone(), root);
        let mut expected = None;
        for concurrency in [1, 4] {
            for repeat in 0..3 {
                let id = format!("case-{concurrency}-{repeat}");
                let started = Instant::now();
                let first = Mutex::new((None, None));
                let result = adapter
                    .scan(
                        &ScanRequest {
                            job_id: id.clone(),
                            volume_id: "fixture-volume".into(),
                            options: ScanOptions { concurrency },
                        },
                        &AtomicBool::new(false),
                        &|progress| {
                            let mut first = first.lock().unwrap();
                            if first.0.is_none() {
                                first.0 = Some(started.elapsed().as_micros());
                            }
                            if progress.files_scanned > 0 && first.1.is_none() {
                                first.1 = Some(started.elapsed().as_micros());
                            }
                        },
                    )
                    .unwrap();
                let elapsed_us = started.elapsed().as_micros();
                let root = adapter.get_node(&id, "n1", None).unwrap();
                assert_eq!(result.files_scanned, files);
                assert_eq!(result.directories_scanned, directories);
                assert_eq!(result.state, StorageState::Complete);
                assert!(result.issues.is_empty());
                assert_eq!(root.logical_bytes, logical_bytes.to_string());
                let quality = (
                    result.files_scanned,
                    root.logical_bytes.clone(),
                    root.allocated_bytes.clone(),
                );
                if let Some(expected) = &expected {
                    assert_eq!(&quality, expected);
                } else {
                    expected = Some(quality);
                }
                let read_started = Instant::now();
                let page = adapter.list_files(&query(&id)).unwrap();
                let page_us = read_started.elapsed().as_micros();
                assert_eq!(page.items.len(), 10);
                let mut filtered = query(&id);
                filtered.name_filter = Some("000".into());
                let filter_started = Instant::now();
                let filtered = adapter.list_files(&filtered).unwrap();
                let filter_us = filter_started.elapsed().as_micros();
                assert!(!filtered.items.is_empty());
                let first = first.lock().unwrap();
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "kind":"storage_case","case":name,"concurrency":concurrency,"repeat":repeat,
                        "files":files,"directories":directories,"logical_bytes":root.logical_bytes,
                        "known_allocated_bytes":root.known_allocated_bytes,"state":result.state,"issue_count":0,
                        "first_progress_us":first.0,"first_file_observation_us":first.1,"elapsed_us":elapsed_us,
                        "page_10_us":page_us,"filter_10_us":filter_us,
                        "index_bytes":fs::metadata(index.join(&id).join("index.sqlite3")).unwrap().len(),
                        "cold_cache_controlled":false,"hdd_measured":false
                    })
                );
            }
        }
        let cancelled = AtomicBool::new(false);
        let cancel_at = Mutex::new(None);
        let result = adapter
            .scan(
                &ScanRequest {
                    job_id: "cancel".into(),
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
        let (at, observed) = cancel_at.lock().unwrap().expect("first file");
        assert_eq!(result.state, StorageState::Cancelled);
        let elapsed_us = at.elapsed().as_micros();
        eprintln!(
            "{}",
            serde_json::json!({
                "kind":"storage_case_cancel","case":name,"cancel_to_terminal_us":elapsed_us,
                "new_dispatch_stop_upper_bound_us":elapsed_us,"dispatch_stop_separately_instrumented":false,
                "observed_when_cancelled":observed,"observed_at_terminal":result.files_scanned,
                "directories_completed":result.directories_scanned,"state":result.state
            })
        );
    }
}

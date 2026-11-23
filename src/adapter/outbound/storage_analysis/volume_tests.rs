use super::*;
use std::fs;
#[cfg(windows)]
use std::sync::atomic::{AtomicUsize, Ordering};

const VOLUME_A: &str = r"\\?\Volume{00112233-4455-6677-8899-aabbccddeeff}\";

struct Fixture {
    base: PathBuf,
    source: PathBuf,
    index: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base =
            std::env::temp_dir().join(format!("akra-storage-volume-{}", rand::random::<u64>()));
        let source = base.join("source");
        let index = base.join("indexes");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&index).unwrap();
        fs::write(source.join("sample.bin"), b"verified volume").unwrap();
        Self {
            base,
            source,
            index,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn storage_analysis_volume_guid_accepts_only_a_complete_guid_root() {
    assert_eq!(native::volume_guid_root(VOLUME_A), Some(VOLUME_A));
    let path = format!("{VOLUME_A}directory\\file.bin");
    assert_eq!(native::volume_guid_root(&path), Some(VOLUME_A));
    let uppercase = VOLUME_A.to_ascii_uppercase();
    assert_eq!(
        native::volume_guid_root(&uppercase),
        Some(uppercase.as_str())
    );
    for invalid in [
        r"C:\",
        r"\\server\share\",
        r"\\?\Volume{not-a-guid}\",
        r"\\?\Volume{00112233-4455-6677-8899-aabbccddeefg}\",
        r"\\?\Volume{00112233_4455-6677-8899-aabbccddeeff}\",
        r"\\?\Volume{00112233-4455-6677-8899-aabbccddeeff}",
        r"\\?\Volume{00112233-4455-6677-8899-aabbccddeeff}/",
        r"\\?\Volume{00112233-4455-6677-8899-aabbccddeeé}\",
        "fixture-volume",
    ] {
        assert_eq!(native::volume_guid_root(invalid), None, "{invalid}");
    }
}

#[test]
fn storage_analysis_fixture_volume_requires_explicit_test_binding() {
    let fixture = Fixture::new();
    let root = fs::canonicalize(&fixture.source).unwrap();
    let metadata = native::metadata(&root, &root, None).unwrap();
    let denied = native::verify_root_volume(
        &metadata,
        "fixture-volume",
        native::RootVolumeBinding::SelectedVolume,
    )
    .unwrap_err();
    assert_eq!(denied.to_string(), "storage_volume_unverified");
    assert!(
        native::verify_root_volume(&metadata, VOLUME_A, native::RootVolumeBinding::Fixture)
            .is_err()
    );

    let request = ScanRequest {
        job_id: "explicit-fixture".into(),
        volume_id: "fixture-volume".into(),
        options: ScanOptions { concurrency: 1 },
    };
    let production = NativeStorageAnalysisAdapter::new(fixture.index.clone());
    assert_eq!(
        production
            .scan(&request, &AtomicBool::new(false), &|_| {})
            .unwrap_err()
            .to_string(),
        "storage_volume_not_found"
    );
    let adapter =
        NativeStorageAnalysisAdapter::fixture(fixture.index.clone(), fixture.source.clone());
    let snapshot = adapter
        .scan(&request, &AtomicBool::new(false), &|_| {})
        .unwrap();
    assert_eq!(snapshot.state, StorageState::Complete);
    assert_eq!(snapshot.files_scanned, 1);
}

#[cfg(windows)]
fn catalog_volume_for(fixture: &Fixture) -> StorageVolume {
    let root = fs::canonicalize(&fixture.source).unwrap();
    let mut volume = native::list_volumes()
        .unwrap()
        .into_iter()
        .find(|volume| {
            volume.supported
                && fs::canonicalize(&volume.root_path)
                    .is_ok_and(|volume_root| root.starts_with(volume_root))
        })
        .expect("the fixture must be on a catalogued local volume");
    // Bound the test's enumeration to its directory; keep the real catalogued GUID.
    volume.root_path = root.to_string_lossy().into_owned();
    volume.label = "volume identity fixture".into();
    volume
}

#[cfg(windows)]
fn scan_selected(
    fixture: &Fixture,
    volume: &StorageVolume,
    job_id: &str,
) -> (index::StorageIndex, Result<()>, usize) {
    let dir = fixture.index.join(job_id);
    fs::create_dir(&dir).unwrap();
    let mut db = index::StorageIndex::create(&dir, job_id, volume).unwrap();
    let notifications = AtomicUsize::new(0);
    let request = ScanRequest {
        job_id: job_id.into(),
        volume_id: volume.id.clone(),
        options: ScanOptions { concurrency: 1 },
    };
    let result = scanner::scan(
        &mut db,
        Path::new(&volume.root_path),
        &fixture.index,
        &request,
        scanner::ScanControl {
            root_volume_binding: native::RootVolumeBinding::SelectedVolume,
            cancelled: &AtomicBool::new(false),
            progress: &|_| {
                notifications.fetch_add(1, Ordering::Relaxed);
            },
            runtime_probe: None,
            scheduling: &Mutex::new(live::ScanSchedulingState::default()),
        },
    );
    (db, result, notifications.load(Ordering::Relaxed))
}

#[cfg(windows)]
#[test]
fn storage_analysis_root_handle_matches_catalog_guid_and_scans_normally() {
    let fixture = Fixture::new();
    let volume = catalog_volume_for(&fixture);
    let root = Path::new(&volume.root_path);
    let metadata = native::metadata(root, root, None).unwrap();
    assert!(
        native::root_volume_id(&metadata)
            .unwrap()
            .eq_ignore_ascii_case(&volume.id)
    );
    let (db, result, notifications) = scan_selected(&fixture, &volume, "matching");
    result.unwrap();
    let snapshot = db.snapshot().unwrap();
    assert_eq!(snapshot.state, StorageState::Complete);
    assert_eq!(snapshot.files_scanned, 1);
    assert!(db.identity("n1").unwrap().is_some());
    assert!(notifications > 0);
}

#[cfg(windows)]
#[test]
fn storage_analysis_replaced_volume_is_rejected_before_identity_or_progress() {
    let fixture = Fixture::new();
    let mut selected = catalog_volume_for(&fixture);
    // Model the replacement boundary deterministically: catalog selection names A,
    // but the path opens B. The new handle's otherwise valid serial must not adopt B.
    selected.id = if selected.id.eq_ignore_ascii_case(VOLUME_A) {
        r"\\?\Volume{ffffffff-eeee-dddd-cccc-bbbbbbbbbbbb}\".into()
    } else {
        VOLUME_A.into()
    };
    let (db, result, notifications) = scan_selected(&fixture, &selected, "replaced");
    let error = result.unwrap_err();
    assert_eq!(error.to_string(), "storage_volume_changed");
    assert_eq!(native::classify_error(&error).code, "volume_changed");
    assert_eq!(notifications, 0);
    assert_eq!(db.identity("n1").unwrap(), None);
    assert_eq!(db.snapshot().unwrap().files_scanned, 0);
    let (nodes, pending): (i64, i64) = db
        .conn
        .query_row("SELECT COUNT(*), MAX(pending) FROM nodes", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!((nodes, pending), (1, 1));
}

#[cfg(windows)]
#[test]
fn storage_analysis_malformed_selected_guid_does_not_start_enumeration() {
    let fixture = Fixture::new();
    let selected = catalog_volume_for(&fixture);
    for (i, id) in [
        "fixture-volume".to_owned(),
        r"C:\".to_owned(),
        format!("{}nested", selected.id),
    ]
    .into_iter()
    .enumerate()
    {
        let volume = StorageVolume {
            id,
            ..selected.clone()
        };
        let (db, result, notifications) =
            scan_selected(&fixture, &volume, &format!("malformed-{i}"));
        assert_eq!(result.unwrap_err().to_string(), "storage_volume_unverified");
        assert_eq!(notifications, 0);
        assert_eq!(db.identity("n1").unwrap(), None);
    }
}

#[cfg(windows)]
#[test]
fn storage_analysis_selected_volume_requires_the_original_open_handle() {
    let fixture = Fixture::new();
    let selected = catalog_volume_for(&fixture);
    let root = Path::new(&selected.root_path);
    let mut metadata = native::metadata(root, root, None).unwrap();
    drop(metadata._handle.take());
    // A valid serial and a still-resolvable path cannot replace the held handle.
    assert!(!metadata.volume.is_empty());
    let error = native::verify_root_volume(
        &metadata,
        &selected.id,
        native::RootVolumeBinding::SelectedVolume,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "storage_volume_unverified");
}

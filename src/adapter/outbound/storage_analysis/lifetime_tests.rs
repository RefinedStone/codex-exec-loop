//! Physical index lifetime: cleanup waits for readers and invalid IDs cannot grow the lease cache.
use super::*;
use std::sync::{Barrier, mpsc};
use std::time::Duration;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-storage-reader-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::create_dir_all(path.join("source")).unwrap();
        std::fs::write(path.join("source/data.bin"), [0; 4096]).unwrap();
        Self(path)
    }
    fn adapter(&self) -> NativeStorageAnalysisAdapter {
        NativeStorageAnalysisAdapter::fixture(self.0.join("indexes"), self.0.join("source"))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn storage_cleanup_waits_for_the_actual_sqlite_reader_to_close() {
    let fixture = Fixture::new();
    let adapter = Arc::new(fixture.adapter());
    adapter
        .scan(
            &ScanRequest {
                job_id: "reader".into(),
                volume_id: "fixture-volume".into(),
                options: ScanOptions::default(),
            },
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let read_adapter = adapter.clone();
    let read_entered = entered.clone();
    let read_release = release.clone();
    let reading = std::thread::spawn(move || {
        read_adapter.query("reader", |db| {
            read_entered.wait();
            read_release.wait();
            assert_eq!(db.snapshot()?.id, "reader");
            Ok(())
        })
    });
    entered.wait();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let cleanup_adapter = adapter.clone();
    let cleanup = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx.send(cleanup_adapter.cleanup("reader")).unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(40)).is_err());
    assert!(fixture.0.join("indexes/reader/index.sqlite3").exists());
    release.wait();
    reading.join().unwrap().unwrap();
    done_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    cleanup.join().unwrap();
    assert!(!fixture.0.join("indexes/reader").exists());
}
#[test]
fn storage_missing_snapshot_queries_do_not_accumulate_lease_entries() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    for i in 0..512 {
        assert!(adapter.snapshot(&format!("missing-{i}")).is_err());
    }
    assert!(adapter.leases.lock().unwrap().len() <= 1);
}

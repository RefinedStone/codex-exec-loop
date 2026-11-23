use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use super::super::tests::record;
use super::*;
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::domain::issue_publish_intent::IssuePublishIntentState;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-publication-sqlite-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }

    fn adapter(&self) -> super::super::IssuePublishIntentJournalAdapter {
        let runtime = self.0.clone();
        super::super::IssuePublishIntentJournalAdapter::new(move |_| Ok(runtime.clone()))
    }

    fn connection(&self) -> Connection {
        Connection::open(self.0.join(DATABASE_NAME)).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn durable_records_keep_insertion_order_zero_sequence_and_separate_lease() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let lease = adapter.acquire_publication_lease("worktree-a").unwrap();
    for (id, state) in [
        ("z", IssuePublishIntentState::Sent),
        ("a", IssuePublishIntentState::Recorded),
        ("z", IssuePublishIntentState::Confirmed),
    ] {
        adapter.save("worktree-b", &record(id, state)).unwrap();
    }
    let records = fixture.adapter().load("another-worktree").unwrap();
    assert_eq!(
        records,
        [
            record("z", IssuePublishIntentState::Confirmed),
            record("a", IssuePublishIntentState::Recorded),
        ]
    );
    assert!(records.iter().all(|r| r.sequence == 0));
    lease.verify_identity().unwrap();
    assert!(PublicationLease::acquire(fixture.0.clone(), Duration::ZERO).is_err());
    let other = Fixture::new();
    let other_adapter = other.adapter();
    let independent = other_adapter
        .acquire_publication_lease("other-repository")
        .unwrap();
    independent.verify_identity().unwrap();
    drop(independent);
    drop(lease);
    fixture
        .adapter()
        .acquire_publication_lease("after-release")
        .unwrap();
    assert!(!fixture.0.join(JOURNAL_FILE_NAME).exists());
    assert!(!fixture.0.join(PUBLICATION_LOCK_NAME).exists());
}

#[test]
fn legacy_files_are_never_deleted_imported_or_ignored() {
    for name in [JOURNAL_FILE_NAME, PUBLICATION_LOCK_NAME] {
        let fixture = Fixture::new();
        let path = fixture.0.join(name);
        fs::write(&path, b"retained legacy data").unwrap();
        let adapter = fixture.adapter();
        assert!(adapter.load("repo").is_err());
        assert!(
            adapter
                .save("repo", &record("one", IssuePublishIntentState::Sent))
                .is_err()
        );
        assert!(adapter.acquire_publication_lease("repo").is_err());
        assert_eq!(fs::read(path).unwrap(), b"retained legacy data");
        assert!(!fixture.0.join(DATABASE_NAME).exists());
    }
}

#[test]
fn corrupt_records_schema_and_record_identity_fail_closed_on_load_and_save() {
    for corrupt in [
        "UPDATE issue_publish_intents SET record = '{not json'",
        "UPDATE issue_publish_intents SET intent_id = 'different'",
        "UPDATE issue_publish_intents SET position = -1",
        "PRAGMA user_version = 7",
        "PRAGMA application_id = 9",
        "CREATE TABLE unexpected (value TEXT)",
        "CREATE TRIGGER unexpected AFTER INSERT ON issue_publish_intents BEGIN DELETE FROM issue_publish_intents; END",
    ] {
        let fixture = Fixture::new();
        let adapter = fixture.adapter();
        adapter
            .save("repo", &record("retained", IssuePublishIntentState::Sent))
            .unwrap();
        fixture.connection().execute_batch(corrupt).unwrap();
        let before = fs::read(fixture.0.join(DATABASE_NAME)).unwrap();
        assert!(adapter.load("repo").is_err(), "{corrupt}");
        assert!(
            adapter
                .save("repo", &record("new", IssuePublishIntentState::Recorded))
                .is_err(),
            "{corrupt}"
        );
        assert_eq!(
            fs::read(fixture.0.join(DATABASE_NAME)).unwrap(),
            before,
            "{corrupt}"
        );
    }
}

#[test]
fn aggregate_serialized_limit_rejects_growth_without_losing_existing_records() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let mut first = record("first", IssuePublishIntentState::Sent);
    first.issue.issue_id = "x".repeat(MAX_BYTES / 2);
    adapter.save("repo", &first).unwrap();
    let mut second = first.clone();
    second.intent_id = "second".to_string();
    assert!(adapter.save("repo", &second).is_err());
    assert_eq!(adapter.load("repo").unwrap(), vec![first]);
}

#[test]
fn real_sqlite_commit_failure_does_not_replace_the_record() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let initial = record("one", IssuePublishIntentState::Recorded);
    adapter.save("repo", &initial).unwrap();
    let reader = fixture.connection();
    reader
        .execute_batch("BEGIN; SELECT * FROM issue_publish_intents;")
        .unwrap();
    let error = adapter
        .save("repo", &record("one", IssuePublishIntentState::Sent))
        .unwrap_err();
    assert!(format!("{error:#}").contains("locked"), "{error:#}");
    reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(adapter.load("repo").unwrap(), vec![initial]);
}

#[test]
fn private_database_hardlinks_junctions_and_acl_changes_are_rejected() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    adapter.load("repo").unwrap();
    let alias = fixture.0.join("alias.db");
    fs::hard_link(fixture.0.join(DATABASE_NAME), &alias).unwrap();
    assert!(adapter.load("repo").is_err());
    fs::remove_file(&alias).unwrap();

    let junction = fixture.0.join("junction");
    let output = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&fixture.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(read_records(&junction).is_err());
    fs::remove_dir(&junction).unwrap();

    let lease = adapter.acquire_publication_lease("repo").unwrap();
    let lease_path = fixture.0.join("issue-publication-lease.db");
    let output = Command::new("icacls.exe")
        .arg(&lease_path)
        .args(["/grant", "*S-1-1-0:(R)"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(lease.verify_identity().is_err());
    drop(lease);
    assert!(adapter.acquire_publication_lease("repo").is_err());
}

#[test]
fn a_legacy_file_appearing_while_reserved_is_detected_before_remote_work() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let lease = adapter.acquire_publication_lease("repo").unwrap();
    fs::write(fixture.0.join(PUBLICATION_LOCK_NAME), b"unexpected").unwrap();
    assert!(lease.verify_identity().is_err());
    assert!(
        adapter
            .save("repo", &record("one", IssuePublishIntentState::Sent))
            .is_err()
    );
}

const CHILD_RUNTIME: &str = "AKRA_PUBLICATION_LEASE_TEST_RUNTIME";
const CHILD_READY: &str = "AKRA_PUBLICATION_LEASE_TEST_READY";

#[test]
fn process_lease_survives_contention_and_a_killed_owner_preserves_sent() {
    if let Some(runtime) = std::env::var_os(CHILD_RUNTIME) {
        let runtime = PathBuf::from(runtime);
        let _lease = PublicationLease::acquire(runtime.clone(), Duration::ZERO).unwrap();
        save_record(
            &runtime,
            &record("durable-sent", IssuePublishIntentState::Sent),
        )
        .unwrap();
        fs::write(std::env::var_os(CHILD_READY).unwrap(), b"sent committed").unwrap();
        std::thread::sleep(Duration::from_secs(60));
        panic!("parent should have terminated this worker after observing committed Sent");
    }

    let fixture = Fixture::new();
    let ready = fixture.0.join("worker-ready");
    let mut child = ChildGuard(Command::new(std::env::current_exe().unwrap())
        .arg("adapter::outbound::filesystem::issue_publish_intent_journal::windows::tests::process_lease_survives_contention_and_a_killed_owner_preserves_sent")
        .args(["--exact", "--nocapture"])
        .env(CHILD_RUNTIME, &fixture.0)
        .env(CHILD_READY, &ready)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "worker exited before committing Sent"
        );
        assert!(
            Instant::now() < deadline,
            "worker did not reach durable Sent"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(PublicationLease::acquire(fixture.0.clone(), Duration::from_millis(50)).is_err());
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let adapter = fixture.adapter();
    let lease = adapter.acquire_publication_lease("second-worker").unwrap();
    assert_eq!(
        fixture.adapter().load("second-worker").unwrap(),
        vec![record("durable-sent", IssuePublishIntentState::Sent)]
    );
    lease.verify_identity().unwrap();
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

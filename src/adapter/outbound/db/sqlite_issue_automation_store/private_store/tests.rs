#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use rusqlite::{Connection, Error, ErrorCode, OpenFlags};

use super::{DATABASE_NAME, open};

const CHILD_TEST: &str = "adapter::outbound::db::sqlite_issue_automation_store::private_store::tests::sqlite_write_lock_subprocess_probe";
const CHILD_DATABASE: &str = "AKRA_TEST_ISSUE_AUTOMATION_LOCK_DATABASE";
const CHILD_EXPECTATION: &str = "AKRA_TEST_ISSUE_AUTOMATION_LOCK_EXPECTATION";

struct RuntimeFixture(PathBuf);

impl RuntimeFixture {
    fn new() -> Self {
        let runtime = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-issue-automation-lock-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&runtime).unwrap();
        Self(std::fs::canonicalize(runtime).unwrap())
    }
}

impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn probe_write_lock(database: &Path, expected: &str) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .env(CHILD_DATABASE, database)
        .env(CHILD_EXPECTATION, expected)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = crate::subprocess::spawn(&mut command).unwrap();
    let output = crate::subprocess::wait_with_output_timeout(
        child,
        "issue automation SQLite write lock probe",
        Duration::from_secs(10),
    )
    .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "child lock probe failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        stdout.contains(&format!("issue automation write lock: {expected}")),
        "the exact child probe must have executed: {stdout}",
    );
}

#[test]
fn closing_another_connection_keeps_the_live_sqlite_write_lock() {
    let runtime = RuntimeFixture::new();
    let mut first = open(&runtime.0).unwrap();
    {
        let (connection, guard) = first.parts();
        connection
            .execute_batch("CREATE TABLE lock_probe (id INTEGER PRIMARY KEY);")
            .unwrap();
        guard.verify_identity().unwrap();
        connection.execute_batch("BEGIN IMMEDIATE;").unwrap();
    }

    // Closing the second wrapper must not close an independently opened descriptor of
    // the database inode while the first SQLite connection still owns a POSIX write lock.
    let second = open(&runtime.0).unwrap();
    drop(second);
    let database = runtime.0.join(DATABASE_NAME);
    probe_write_lock(&database, "blocked");

    {
        let (connection, guard) = first.parts();
        guard.verify_identity().unwrap();
        connection.execute_batch("ROLLBACK;").unwrap();
        guard.verify_identity().unwrap();
    }
    probe_write_lock(&database, "acquired");
}

#[test]
fn sqlite_write_lock_subprocess_probe() {
    let Some(database) = std::env::var_os(CHILD_DATABASE) else {
        return;
    };
    let expected = std::env::var(CHILD_EXPECTATION).unwrap();
    let connection = Connection::open_with_flags(
        PathBuf::from(database),
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .unwrap();
    connection.busy_timeout(Duration::ZERO).unwrap();
    let result = connection.execute_batch("BEGIN IMMEDIATE;");
    match expected.as_str() {
        "blocked" => {
            assert!(
                matches!(&result, Err(Error::SqliteFailure(error, _)) if error.code == ErrorCode::DatabaseBusy),
                "another process must not acquire the live writer lock: {result:?}",
            );
        }
        "acquired" => {
            result.expect("the writer lock must be released after the first transaction ends");
            connection.execute_batch("ROLLBACK;").unwrap();
        }
        _ => panic!("unexpected lock probe expectation"),
    }
    println!("issue automation write lock: {expected}");
}

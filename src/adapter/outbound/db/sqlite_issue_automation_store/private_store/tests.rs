use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
#[cfg(unix)]
use rusqlite::{Error, ErrorCode};

use super::{DATABASE_NAME, open};

#[cfg(unix)]
const CHILD_TEST: &str = "adapter::outbound::db::sqlite_issue_automation_store::private_store::tests::sqlite_write_lock_subprocess_probe";
#[cfg(unix)]
const CHILD_DATABASE: &str = "AKRA_TEST_ISSUE_AUTOMATION_LOCK_DATABASE";
#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

const HOT_JOURNAL_CHILD_TEST: &str = "adapter::outbound::db::sqlite_issue_automation_store::private_store::tests::sqlite_hot_journal_subprocess_fixture";
const HOT_JOURNAL_CHILD_DATABASE: &str = "AKRA_TEST_ISSUE_AUTOMATION_HOT_JOURNAL_DATABASE";

fn leave_sqlite_generated_hot_journal(database: &Path) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", HOT_JOURNAL_CHILD_TEST, "--nocapture"])
        .env(HOT_JOURNAL_CHILD_DATABASE, database)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = crate::subprocess::spawn(&mut command).unwrap();
    let output = crate::subprocess::wait_with_output_timeout(
        child,
        "issue automation SQLite hot journal fixture",
        Duration::from_secs(10),
    )
    .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "child hot journal fixture failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        stdout.contains("issue automation hot journal: flushed without commit"),
        "the exact child fixture must have executed: {stdout}",
    );
}

fn assert_sqlite_generated_page_one_journal(runtime: &Path) {
    let journal = std::fs::read(runtime.join(super::ROLLBACK_JOURNAL_NAME)).unwrap();
    assert!(journal.len() > 512, "the journal must contain page records");
    assert_eq!(
        &journal[..8],
        &[0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7],
        "SQLite must have synced a valid rollback journal header",
    );
    let field = |offset| u32::from_be_bytes(journal[offset..offset + 4].try_into().unwrap());
    assert_ne!(field(8), 0, "the synced journal must have page records");
    let sector_size = field(20) as usize;
    let page_size = field(24) as usize;
    assert!((512..=65536).contains(&sector_size) && sector_size.is_power_of_two());
    assert!((512..=65536).contains(&page_size) && page_size.is_power_of_two());
    assert!(
        journal.len() >= sector_size + 4 + page_size + 4,
        "the first record must contain its complete page and checksum",
    );
    assert_eq!(
        field(sector_size),
        1,
        "changing user_version first must journal the original page 1",
    );
    let page_one = &journal[sector_size + 4..sector_size + 4 + page_size];
    assert_eq!(&page_one[..16], b"SQLite format 3\0");
    assert_eq!(&page_one[18..20], &[1, 1]);
    assert_eq!(
        u32::from_be_bytes(page_one[60..64].try_into().unwrap()),
        17,
        "the journal must preserve the committed user_version",
    );
}

fn assert_committed_recovery_probe(connection: &Connection) {
    // Opening a handle or querying journal_mode alone need not read the B-tree.
    // A real table read must first recover the hot journal and repaired page 1.
    let marker: i64 = connection
        .query_row(
            "SELECT marker FROM recovery_probe WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        marker, 7,
        "the uncommitted table change must be rolled back"
    );
    let user_version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, 17, "page 1 must be restored from the journal");
}

fn damage_main_header(runtime: &Path, damage: &str) {
    use std::io::{Seek, SeekFrom, Write};

    let mut database = std::fs::OpenOptions::new()
        .write(true)
        .open(runtime.join(DATABASE_NAME))
        .unwrap();
    match damage {
        "magic" => database.write_all(b"Torn").unwrap(),
        "short" => database.set_len(19).unwrap(),
        "format_zero" | "format_wal" => {
            database.seek(SeekFrom::Start(18)).unwrap();
            let format = if damage == "format_wal" { 2 } else { 0 };
            database.write_all(&[format, format]).unwrap();
        }
        _ => panic!("unexpected header damage fixture"),
    }
    database.sync_all().unwrap();
}

fn copy_private_crash_fixture(source: &Path, destination: &Path) {
    std::fs::copy(source, destination).unwrap();
    #[cfg(windows)]
    {
        use crate::private_fs::{
            WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC, set_windows_private_acl,
            validate_windows_path_identity,
        };
        use std::os::windows::fs::OpenOptionsExt;
        // CopyFile preserves bytes but the new destination inherits its directory ACL.
        // Model an already-private crashed database; production must not repair that ACL.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL | WINDOWS_WRITE_DAC)
            .open(destination)
            .unwrap();
        validate_windows_path_identity(destination, &file, false).unwrap();
        set_windows_private_acl(&file, false).unwrap();
    }
}

fn assert_recovered_header(runtime: &Path) {
    let database = std::fs::read(runtime.join(DATABASE_NAME)).unwrap();
    assert_eq!(&database[..16], b"SQLite format 3\0");
    assert_eq!(&database[18..20], &[1, 1]);
    assert!(!runtime.join(super::ROLLBACK_JOURNAL_NAME).exists());
}

#[test]
fn hot_rollback_journal_recovers_a_torn_database_header() {
    let source = RuntimeFixture::new();
    {
        let mut baseline = open(&source.0).unwrap();
        let (connection, guard) = baseline.parts();
        connection
            .execute_batch(
                "CREATE TABLE recovery_probe (id INTEGER PRIMARY KEY, marker INTEGER NOT NULL);
                 INSERT INTO recovery_probe (id, marker) VALUES (1, 7);
                 PRAGMA user_version = 17;",
            )
            .unwrap();
        guard.verify_identity().unwrap();
    }

    leave_sqlite_generated_hot_journal(&source.0.join(DATABASE_NAME));
    assert_sqlite_generated_page_one_journal(&source.0);

    for damage in ["magic", "short", "format_zero", "format_wal"] {
        let plain = RuntimeFixture::new();
        let protected = RuntimeFixture::new();
        for name in [DATABASE_NAME, super::ROLLBACK_JOURNAL_NAME] {
            copy_private_crash_fixture(&source.0.join(name), &plain.0.join(name));
            copy_private_crash_fixture(&source.0.join(name), &protected.0.join(name));
        }
        // The child has exited, so its locks are gone. Only the main file is
        // damaged; both copies retain SQLite's journal bytes and checksums.
        damage_main_header(&plain.0, damage);
        damage_main_header(&protected.0, damage);
        for name in [DATABASE_NAME, super::ROLLBACK_JOURNAL_NAME] {
            assert_eq!(
                std::fs::read(plain.0.join(name)).unwrap(),
                std::fs::read(protected.0.join(name)).unwrap(),
                "both recovery paths must start with the same {damage} crash state",
            );
        }

        let plain_connection = Connection::open_with_flags(
            plain.0.join(DATABASE_NAME),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .unwrap();
        // Positive control: the existing journal_mode query loads the schema
        // and recovers this crash state before any application table SELECT.
        let mode: String = plain_connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "delete");
        assert_recovered_header(&plain.0);
        assert_committed_recovery_probe(&plain_connection);
        drop(plain_connection);

        if damage == "format_wal" {
            // We cannot distinguish a torn version byte from a legacy WAL file.
            // Even this valid rollback journal must not waive the format-2 boundary.
            let before = std::fs::read(protected.0.join(DATABASE_NAME)).unwrap();
            let journal = std::fs::read(protected.0.join(super::ROLLBACK_JOURNAL_NAME)).unwrap();
            assert!(open(&protected.0).is_err());
            assert_eq!(
                std::fs::read(protected.0.join(DATABASE_NAME)).unwrap(),
                before
            );
            assert_eq!(
                std::fs::read(protected.0.join(super::ROLLBACK_JOURNAL_NAME)).unwrap(),
                journal,
            );
            continue;
        }

        let mut recovered = open(&protected.0).unwrap_or_else(|error| {
            panic!("a valid hot journal must repair {damage} before format validation: {error}")
        });
        // open itself must complete recovery and validate the repaired header;
        // the later application SELECT must not be needed to finish recovery.
        assert_recovered_header(&protected.0);
        let (connection, guard) = recovered.parts();
        assert_committed_recovery_probe(connection);
        guard.verify_identity().unwrap();
    }
}

#[test]
fn invalid_journals_do_not_bypass_legacy_wal_header_rejection() {
    use std::io::Write;

    let source = RuntimeFixture::new();
    {
        let mut baseline = open(&source.0).unwrap();
        let (connection, _) = baseline.parts();
        connection
            .execute_batch(
                "CREATE TABLE retained (value INTEGER); INSERT INTO retained VALUES (7);",
            )
            .unwrap();
    }
    let database = source.0.join(DATABASE_NAME);
    let wal = Connection::open(&database).unwrap();
    let mode: String = wal
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    drop(wal);
    let original = std::fs::read(&database).unwrap();
    assert_eq!(&original[18..20], &[2, 2]);
    assert!(!source.0.join("issue-automation.db-wal").exists());
    assert!(!source.0.join("issue-automation.db-shm").exists());

    for journal in [Vec::new(), vec![0; 1024], vec![0x11; 1024]] {
        let runtime = RuntimeFixture::new();
        copy_private_crash_fixture(&database, &runtime.0.join(DATABASE_NAME));
        let journal_path = runtime.0.join(super::ROLLBACK_JOURNAL_NAME);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&journal_path).unwrap();
        file.write_all(&journal).unwrap();
        file.sync_all().unwrap();
        drop(file);

        let error = match open(&runtime.0) {
            Ok(_) => panic!("invalid or empty journals must not admit a legacy WAL database"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("unsupported file format"));
        assert_eq!(
            std::fs::read(runtime.0.join(DATABASE_NAME)).unwrap(),
            original
        );
        assert_eq!(std::fs::read(journal_path).unwrap(), journal);
        assert!(!runtime.0.join("issue-automation.db-wal").exists());
        assert!(!runtime.0.join("issue-automation.db-shm").exists());
    }
}

#[test]
fn sqlite_hot_journal_subprocess_fixture() {
    use std::io::Write;

    let Some(database) = std::env::var_os(HOT_JOURNAL_CHILD_DATABASE) else {
        return;
    };
    let connection = Connection::open_with_flags(
        PathBuf::from(database),
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode = DELETE;
             PRAGMA synchronous = FULL;
             BEGIN IMMEDIATE;
             PRAGMA user_version = 29;
             UPDATE recovery_probe SET marker = 99 WHERE id = 1;",
        )
        .unwrap();
    // Page 1 remains pinned by SQLite. Flushing the separate dirty table page
    // forces syncJournal to make its original page 1 record durable as well.
    connection.cache_flush().unwrap();
    assert!(!connection.is_autocommit());
    println!("issue automation hot journal: flushed without commit");
    std::io::stdout().flush().unwrap();

    // Process exit skips Connection::drop and its rollback, leaving only a
    // journal produced and synced by SQLite for the parent recovery assertions.
    std::process::exit(0);
}

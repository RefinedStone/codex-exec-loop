use std::path::PathBuf;

use super::{DatabaseFiles, Synchronous, open};

const FILES: DatabaseFiles = DatabaseFiles::new(
    "coordination.db",
    "coordination.db-journal",
    "coordination.db-wal",
    "coordination.db-shm",
);

struct RuntimeFixture(PathBuf);

impl RuntimeFixture {
    fn new() -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-private-sqlite-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&path).unwrap();
        Self(std::fs::canonicalize(path).unwrap())
    }
}

impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn configured_file_names_and_synchronous_policies_are_applied() {
    for (policy, expected) in [(Synchronous::Full, 2_i64), (Synchronous::Extra, 3_i64)] {
        let fixture = RuntimeFixture::new();
        let mut opened = open(&fixture.0, FILES, policy).unwrap();
        let (connection, guard) = opened.parts();
        let mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        let synchronization: i64 = connection
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "delete");
        assert_eq!(synchronization, expected);
        connection.execute_batch("CREATE TABLE probe (value INTEGER); BEGIN IMMEDIATE; INSERT INTO probe VALUES (7); COMMIT;").unwrap();
        guard.verify_identity().unwrap();
        assert!(fixture.0.join(FILES.database).is_file());
        for name in [FILES.rollback_journal, FILES.wal, FILES.shm] {
            assert!(!fixture.0.join(name).exists());
        }
    }
}

#[test]
fn invalid_names_are_rejected_before_database_creation() {
    let invalid = [
        DatabaseFiles::new(
            "../outside.db",
            "../outside.db-journal",
            "../outside.db-wal",
            "../outside.db-shm",
        ),
        DatabaseFiles::new(
            "name.db:stream",
            "name.db:stream-journal",
            "name.db:stream-wal",
            "name.db:stream-shm",
        ),
        DatabaseFiles::new("CON.db", "CON.db-journal", "CON.db-wal", "CON.db-shm"),
        DatabaseFiles::new(
            "coordination.db",
            "other.db-journal",
            "coordination.db-wal",
            "coordination.db-shm",
        ),
        DatabaseFiles::new(
            "coordination.db",
            "coordination.db-journal",
            "other.db-wal",
            "coordination.db-shm",
        ),
    ];
    for files in invalid {
        let fixture = RuntimeFixture::new();
        assert!(open(&fixture.0, files, Synchronous::Extra).is_err());
        assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 0);
    }
}

#[test]
fn configured_wal_and_shm_paths_are_rejected_without_creating_the_database() {
    for name in [FILES.wal, FILES.shm] {
        let fixture = RuntimeFixture::new();
        let sidecar = fixture.0.join(name);
        std::fs::write(&sidecar, b"retained").unwrap();
        assert!(open(&fixture.0, FILES, Synchronous::Extra).is_err());
        assert!(!fixture.0.join(FILES.database).exists());
        assert_eq!(std::fs::read(sidecar).unwrap(), b"retained");
    }
}

#[test]
fn configured_journal_hardlink_is_rejected_without_touching_its_target() {
    let fixture = RuntimeFixture::new();
    let target = fixture.0.join("retained");
    std::fs::write(&target, b"retained").unwrap();
    std::fs::hard_link(&target, fixture.0.join(FILES.rollback_journal)).unwrap();
    assert!(open(&fixture.0, FILES, Synchronous::Extra).is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"retained");
}

#[test]
fn creates_private_nested_runtime_and_reopens_committed_data() {
    let fixture = RuntimeFixture::new();
    let runtime = fixture
        .0
        .join("global")
        .join("coordination")
        .join("runtime");
    assert!(open(&runtime, FILES, Synchronous::Extra).is_err());
    assert!(!fixture.0.join("global").exists());
    {
        let mut opened = super::open_or_create(&runtime, FILES, Synchronous::Extra).unwrap();
        let (connection, guard) = opened.parts();
        connection
            .execute_batch(
                "CREATE TABLE retained (value INTEGER NOT NULL); INSERT INTO retained VALUES (7);",
            )
            .unwrap();
        guard.verify_identity().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in runtime
                .ancestors()
                .take_while(|path| *path != fixture.0.as_path())
            {
                assert_eq!(
                    std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o700
                );
            }
        }
    }
    let mut reopened = super::open_or_create(&runtime, FILES, Synchronous::Extra).unwrap();
    let (connection, guard) = reopened.parts();
    assert_eq!(
        connection
            .query_row("SELECT value FROM retained", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        7,
    );
    guard.verify_identity().unwrap();
}

#[test]
fn rejects_invalid_runtime_and_names_before_creating_any_component() {
    let fixture = RuntimeFixture::new();
    // PathBuf::join normalizes ".." when Windows canonicalize supplied a verbatim prefix.
    // Append to OsString so this fixture reaches the boundary with its invalid input intact.
    let raw_runtime = |components: &[&str]| {
        let mut raw = fixture.0.as_os_str().to_os_string();
        for component in components {
            raw.push(std::path::MAIN_SEPARATOR_STR);
            raw.push(component);
        }
        PathBuf::from(raw)
    };
    let parent_alias = raw_runtime(&["must-not-create", "..", "runtime"]);
    assert!(
        parent_alias
            .components()
            .any(|component| component == std::path::Component::ParentDir),
        "the fixture must retain its parent-directory alias",
    );
    let embedded_nul = raw_runtime(&["must-not-create", "bad\0component"]);
    assert!(embedded_nul.as_os_str().as_encoded_bytes().contains(&0));
    for runtime in [parent_alias, embedded_nul] {
        assert!(
            super::open_or_create(&runtime, FILES, Synchronous::Extra).is_err(),
            "synthetic runtime: {:?}; components: {:?}; encoded: {:?}",
            runtime,
            runtime.components().collect::<Vec<_>>(),
            runtime.as_os_str().as_encoded_bytes(),
        );
        assert!(!fixture.0.join("must-not-create").exists());
    }
    let files = DatabaseFiles::new("bad.db", "wrong-journal", "bad.db-wal", "bad.db-shm");
    assert!(
        super::open_or_create(
            &fixture.0.join("must-not-create"),
            files,
            Synchronous::Extra,
        )
        .is_err()
    );
    assert!(!fixture.0.join("must-not-create").exists());
}

#[cfg(unix)]
#[test]
fn rejects_directory_symlinks_before_creating_children_in_the_target() {
    use std::os::unix::fs::symlink;

    let fixture = RuntimeFixture::new();
    let target = fixture.0.join("retained-target");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("sentinel"), b"unchanged").unwrap();
    let link = fixture.0.join("redirect");
    symlink(&target, &link).unwrap();
    assert!(
        super::open_or_create(
            &link.join("coordination").join("runtime"),
            FILES,
            Synchronous::Extra,
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(target.join("sentinel")).unwrap(),
        b"unchanged"
    );
    assert_eq!(std::fs::read_dir(target).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn rejects_existing_nonprivate_runtime_without_repair() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = RuntimeFixture::new();
    let runtime = fixture.0.join("runtime");
    std::fs::create_dir(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(super::open_or_create(&runtime, FILES, Synchronous::Extra).is_err());
    assert_eq!(
        std::fs::metadata(&runtime).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(!runtime.join(FILES.database).exists());
}

#[cfg(windows)]
#[test]
fn rejects_directory_junctions_before_creating_children_in_the_target() {
    use std::process::{Command, Stdio};
    use std::time::Duration;

    let fixture = RuntimeFixture::new();
    let target = fixture.0.join("retained-target");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("sentinel"), b"unchanged").unwrap();
    let link = fixture.0.join("redirect");
    // cmd's built-in mklink expects the drive path rather than the canonical verbatim prefix.
    let link_text = link.to_string_lossy();
    let target_text = target.to_string_lossy();
    let mut command = Command::new("cmd");
    command
        .args([
            "/C",
            "mklink",
            "/J",
            link_text.strip_prefix(r"\\?\").unwrap_or(&link_text),
            target_text.strip_prefix(r"\\?\").unwrap_or(&target_text),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = crate::subprocess::wait_with_output_timeout(
        crate::subprocess::spawn(&mut command).unwrap(),
        "private SQLite junction fixture",
        Duration::from_secs(10),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "junction creation failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        super::open_or_create(
            &link.join("coordination").join("runtime"),
            FILES,
            Synchronous::Extra,
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(target.join("sentinel")).unwrap(),
        b"unchanged"
    );
    assert_eq!(std::fs::read_dir(&target).unwrap().count(), 1);
    std::fs::remove_dir(link).unwrap();
}

#[cfg(windows)]
#[test]
fn creation_guard_pins_runtime_path_until_the_connection_closes() {
    let fixture = RuntimeFixture::new();
    let runtime = fixture.0.join("runtime");
    let mut opened = super::open_or_create(&runtime, FILES, Synchronous::Extra).unwrap();
    let mut another = super::open_or_create(&runtime, FILES, Synchronous::Extra).unwrap();
    another.parts().1.verify_identity().unwrap();
    drop(another);
    let renamed = fixture.0.join("renamed-runtime");
    assert!(std::fs::rename(&runtime, &renamed).is_err());
    opened.parts().1.verify_identity().unwrap();
    drop(opened);
    std::fs::rename(&runtime, &renamed).unwrap();
    std::fs::rename(&renamed, &runtime).unwrap();
}

#[cfg(windows)]
#[test]
fn rejects_existing_inherited_runtime_acl_without_repair() {
    let fixture = RuntimeFixture::new();
    let runtime = fixture.0.join("runtime");
    // Inheritance supplies the correct principal but not the protected owner-only DACL
    // required by the runtime boundary. Existing objects must not be silently repaired.
    std::fs::create_dir(&runtime).unwrap();
    assert!(super::open_or_create(&runtime, FILES, Synchronous::Extra).is_err());
    assert!(!runtime.join(FILES.database).exists());
}

#[cfg(windows)]
#[test]
fn rejects_existing_database_acl_without_repair() {
    for inherited_acl in [true, false] {
        let fixture = RuntimeFixture::new();
        let path = fixture.0.join(FILES.database);
        if inherited_acl {
            std::fs::write(&path, b"").unwrap();
        } else {
            drop(open(&fixture.0, FILES, Synchronous::Extra).unwrap());
            let mut command = std::process::Command::new("icacls.exe");
            command.arg(&path).args(["/grant", "*S-1-1-0:(R)"]);
            let output = crate::subprocess::wait_with_output_timeout(
                crate::subprocess::spawn(&mut command).unwrap(),
                "private SQLite ACL fixture",
                std::time::Duration::from_secs(10),
            )
            .unwrap();
            assert!(output.status.success());
        }
        let before = std::fs::read(&path).unwrap();
        assert!(open(&fixture.0, FILES, Synchronous::Extra).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let file = std::fs::File::open(&path).unwrap();
        assert!(crate::private_fs::validate_windows_private_owner_and_acl(&path, &file).is_err());
    }
}

#[test]
fn existing_only_missing_paths_do_not_create_any_component() {
    let fixture = RuntimeFixture::new();
    let missing = fixture.0.join("absent").join("runtime");
    assert!(
        super::open_existing(&missing, FILES, Synchronous::Extra)
            .unwrap()
            .is_none()
    );
    assert!(!fixture.0.join("absent").exists());
    assert!(
        super::open_existing(&fixture.0, FILES, Synchronous::Extra)
            .unwrap()
            .is_none()
    );
    assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 0);
}

#[test]
fn existing_only_rejects_orphan_sidecars_without_creating_database() {
    for name in [FILES.rollback_journal, FILES.wal, FILES.shm] {
        let fixture = RuntimeFixture::new();
        std::fs::write(fixture.0.join(name), b"retained").unwrap();
        assert!(super::open_existing(&fixture.0, FILES, Synchronous::Extra).is_err());
        assert!(!fixture.0.join(FILES.database).exists());
        assert_eq!(std::fs::read(fixture.0.join(name)).unwrap(), b"retained");
    }
}

#[test]
fn existing_only_reopens_committed_data_and_rejects_hardlinks() {
    let fixture = RuntimeFixture::new();
    {
        let mut opened =
            super::open_or_create_strict(&fixture.0, FILES, Synchronous::Extra).unwrap();
        opened
            .parts()
            .0
            .execute_batch("CREATE TABLE retained(value INTEGER); INSERT INTO retained VALUES(7);")
            .unwrap();
    }
    let before = std::fs::read(fixture.0.join(FILES.database)).unwrap();
    {
        let mut opened = super::open_existing(&fixture.0, FILES, Synchronous::Extra)
            .unwrap()
            .unwrap();
        let (connection, guard) = opened.parts();
        assert_eq!(
            connection
                .query_row("SELECT value FROM retained", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            7
        );
        guard.verify_identity().unwrap();
    }
    assert_eq!(
        std::fs::read(fixture.0.join(FILES.database)).unwrap(),
        before
    );
    let retained = fixture.0.join("hardlink-target");
    std::fs::hard_link(fixture.0.join(FILES.database), &retained).unwrap();
    assert!(super::open_existing(&fixture.0, FILES, Synchronous::Extra).is_err());
    assert_eq!(std::fs::read(retained).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn strict_existing_and_creation_do_not_repair_permissions_but_legacy_contract_is_preserved() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = RuntimeFixture::new();
    drop(super::open_or_create_strict(&fixture.0, FILES, Synchronous::Extra).unwrap());
    let database = fixture.0.join(FILES.database);
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(super::open_existing(&fixture.0, FILES, Synchronous::Extra).is_err());
    assert!(super::open_or_create_strict(&fixture.0, FILES, Synchronous::Extra).is_err());
    assert_eq!(
        std::fs::metadata(&database).unwrap().permissions().mode() & 0o777,
        0o644
    );
    drop(open(&fixture.0, FILES, Synchronous::Extra).unwrap());
    assert_eq!(
        std::fs::metadata(database).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn existing_only_dangling_directory_and_database_symlinks_are_not_absence() {
    use std::os::unix::fs::symlink;
    let fixture = RuntimeFixture::new();
    let link = fixture.0.join("redirect");
    symlink(fixture.0.join("missing-target"), &link).unwrap();
    assert!(super::open_existing(&link, FILES, Synchronous::Extra).is_err());
    assert!(super::open_existing(&link.join("missing-child"), FILES, Synchronous::Extra).is_err());
    std::fs::remove_file(&link).unwrap();
    symlink(
        fixture.0.join("missing-target"),
        fixture.0.join(FILES.database),
    )
    .unwrap();
    assert!(super::open_existing(&fixture.0, FILES, Synchronous::Extra).is_err());
    assert!(!fixture.0.join("missing-target").exists());
}

#[cfg(unix)]
const EXISTING_LOCK_PROBE: &str =
    "private_sqlite::tests::existing_only_write_lock_subprocess_probe";
#[cfg(unix)]
const EXISTING_LOCK_DATABASE: &str = "AKRA_TEST_EXISTING_SQLITE_LOCK_DATABASE";
#[cfg(unix)]
const EXISTING_LOCK_EXPECTATION: &str = "AKRA_TEST_EXISTING_SQLITE_LOCK_EXPECTATION";

#[cfg(unix)]
fn probe_existing_lock(database: &std::path::Path, expectation: &str) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", EXISTING_LOCK_PROBE, "--nocapture"])
        .env(EXISTING_LOCK_DATABASE, database)
        .env(EXISTING_LOCK_EXPECTATION, expectation)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = crate::subprocess::wait_with_output_timeout(
        crate::subprocess::spawn(&mut command).unwrap(),
        "existing-only SQLite lock probe",
        std::time::Duration::from_secs(10),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("existing SQLite lock: {expectation}"))
    );
}

#[cfg(unix)]
#[test]
fn dropping_existing_reader_keeps_another_connections_posix_write_lock() {
    let fixture = RuntimeFixture::new();
    let mut writer = open(&fixture.0, FILES, Synchronous::Extra).unwrap();
    writer
        .parts()
        .0
        .execute_batch("CREATE TABLE lock_probe(value INTEGER); BEGIN IMMEDIATE;")
        .unwrap();
    let reader = super::open_existing(&fixture.0, FILES, Synchronous::Extra)
        .unwrap()
        .unwrap();
    drop(reader);
    probe_existing_lock(&fixture.0.join(FILES.database), "blocked");
    writer.parts().0.execute_batch("ROLLBACK;").unwrap();
    probe_existing_lock(&fixture.0.join(FILES.database), "acquired");
}

#[cfg(unix)]
#[test]
fn existing_only_write_lock_subprocess_probe() {
    let Some(database) = std::env::var_os(EXISTING_LOCK_DATABASE) else {
        return;
    };
    let expectation = std::env::var(EXISTING_LOCK_EXPECTATION).unwrap();
    let connection = rusqlite::Connection::open_with_flags(
        PathBuf::from(database),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    connection.busy_timeout(std::time::Duration::ZERO).unwrap();
    let result = connection.execute_batch("BEGIN IMMEDIATE;");
    if expectation == "blocked" {
        assert!(
            matches!(&result, Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::DatabaseBusy)
        );
    } else {
        assert_eq!(expectation, "acquired");
        result.unwrap();
        connection.execute_batch("ROLLBACK;").unwrap();
    }
    println!("existing SQLite lock: {expectation}");
}

#[cfg(windows)]
#[test]
fn existing_only_reader_pins_parents_until_connection_drop() {
    let fixture = RuntimeFixture::new();
    let runtime = fixture.0.join("runtime");
    drop(super::open_or_create_strict(&runtime, FILES, Synchronous::Extra).unwrap());
    let mut opened = super::open_existing(&runtime, FILES, Synchronous::Extra)
        .unwrap()
        .unwrap();
    let renamed = fixture.0.join("renamed-runtime");
    assert!(std::fs::rename(&runtime, &renamed).is_err());
    opened.parts().1.verify_identity().unwrap();
    drop(opened);
    std::fs::rename(&runtime, &renamed).unwrap();
    std::fs::rename(&renamed, &runtime).unwrap();
}

#[cfg(windows)]
#[test]
fn existing_only_rejects_junctions_even_when_the_target_or_descendant_is_missing() {
    let fixture = RuntimeFixture::new();
    let target = fixture.0.join("target");
    std::fs::create_dir(&target).unwrap();
    let link = fixture.0.join("redirect");
    let link_text = link.to_string_lossy().replace('/', "\\");
    let target_text = target.to_string_lossy().replace('/', "\\");
    let mut command = std::process::Command::new("cmd");
    command
        .args([
            "/C",
            "mklink",
            "/J",
            link_text.strip_prefix(r"\\?\").unwrap_or(&link_text),
            target_text.strip_prefix(r"\\?\").unwrap_or(&target_text),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = crate::subprocess::wait_with_output_timeout(
        crate::subprocess::spawn(&mut command).unwrap(),
        "existing-only SQLite junction fixture",
        std::time::Duration::from_secs(10),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(super::open_existing(&link, FILES, Synchronous::Extra).is_err());
    assert!(super::open_existing(&link.join("missing-child"), FILES, Synchronous::Extra).is_err());
    std::fs::remove_dir(&target).unwrap();
    assert!(super::open_existing(&link, FILES, Synchronous::Extra).is_err());
    assert!(super::open_existing(&link.join("missing-child"), FILES, Synchronous::Extra).is_err());
    std::fs::remove_dir(&link).unwrap();
    assert!(!fixture.0.join("target").exists());
}

#[cfg(windows)]
#[test]
fn existing_only_rejects_inherited_acl_without_repair_or_creation() {
    let fixture = RuntimeFixture::new();
    let runtime = fixture.0.join("inherited");
    std::fs::create_dir(&runtime).unwrap();
    assert!(super::open_existing(&runtime, FILES, Synchronous::Extra).is_err());
    assert!(!runtime.join(FILES.database).exists());
    let database = fixture.0.join(FILES.database);
    std::fs::write(&database, b"").unwrap();
    assert!(super::open_existing(&fixture.0, FILES, Synchronous::Extra).is_err());
    let file = std::fs::File::open(&database).unwrap();
    assert!(crate::private_fs::validate_windows_private_owner_and_acl(&database, &file).is_err());
}

const EXISTING_HOT_JOURNAL_PROBE: &str =
    "private_sqlite::tests::existing_only_hot_journal_subprocess_probe";
const EXISTING_HOT_JOURNAL_DATABASE: &str = "AKRA_TEST_EXISTING_SQLITE_HOT_JOURNAL_DATABASE";

#[test]
fn existing_only_recovers_a_sqlite_generated_hot_journal_after_process_exit() {
    let fixture = RuntimeFixture::new();
    {
        let mut opened =
            super::open_or_create_strict(&fixture.0, FILES, Synchronous::Extra).unwrap();
        opened
            .parts()
            .0
            .execute_batch(
                "CREATE TABLE recovery_probe(id INTEGER PRIMARY KEY, marker INTEGER NOT NULL);
             INSERT INTO recovery_probe VALUES(1,7);
             PRAGMA user_version=3;",
            )
            .unwrap();
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", EXISTING_HOT_JOURNAL_PROBE, "--nocapture"])
        .env(
            EXISTING_HOT_JOURNAL_DATABASE,
            fixture.0.join(FILES.database),
        )
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = crate::subprocess::wait_with_output_timeout(
        crate::subprocess::spawn(&mut command).unwrap(),
        "existing-only SQLite hot journal fixture",
        std::time::Duration::from_secs(10),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("existing SQLite hot journal: flushed without commit")
    );
    assert!(fixture.0.join(FILES.rollback_journal).exists());
    let mut reopened = super::open_existing(&fixture.0, FILES, Synchronous::Extra)
        .unwrap()
        .unwrap();
    let (connection, guard) = reopened.parts();
    assert_eq!(
        connection
            .query_row("SELECT marker FROM recovery_probe WHERE id=1", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        7
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        3
    );
    guard.verify_identity().unwrap();
}

#[test]
fn existing_only_hot_journal_subprocess_probe() {
    use std::io::Write;
    let Some(database) = std::env::var_os(EXISTING_HOT_JOURNAL_DATABASE) else {
        return;
    };
    let connection = rusqlite::Connection::open_with_flags(
        PathBuf::from(database),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
         BEGIN IMMEDIATE; PRAGMA user_version=29;
         UPDATE recovery_probe SET marker=99 WHERE id=1;",
        )
        .unwrap();
    connection.cache_flush().unwrap();
    assert!(!connection.is_autocommit());
    println!("existing SQLite hot journal: flushed without commit");
    std::io::stdout().flush().unwrap();
    std::process::exit(0);
}

#[cfg(windows)]
#[test]
fn verifying_a_private_live_journal_does_not_rewrite_its_metadata() {
    let fixture = RuntimeFixture::new();
    let mut opened = open(&fixture.0, FILES, Synchronous::Extra).unwrap();
    let (connection, guard) = opened.parts();
    connection
        .execute_batch("CREATE TABLE retained(value INTEGER);")
        .unwrap();
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    transaction
        .execute("INSERT INTO retained VALUES(7)", [])
        .unwrap();
    // Normalize the journal created by SQLite once, then verify without any SQL writes.
    guard.verify_identity().unwrap();
    let journal = std::fs::File::open(fixture.0.join(FILES.rollback_journal)).unwrap();
    let before = crate::private_fs::windows_file_identity_snapshot(&journal).unwrap();
    // Keep the two metadata observations in distinct Windows clock ticks.
    std::thread::sleep(std::time::Duration::from_millis(20));
    guard.verify_identity().unwrap();
    let after = crate::private_fs::windows_file_identity_snapshot(&journal).unwrap();
    assert_eq!(
        before, after,
        "journal verification must not change a live writer's metadata"
    );
    drop(journal);
    transaction.commit().unwrap();
    guard.verify_identity().unwrap();
    assert_eq!(
        connection
            .query_row("SELECT value FROM retained", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        7
    );
}

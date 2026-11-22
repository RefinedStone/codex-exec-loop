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

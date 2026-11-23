use super::*;
use std::sync::{Arc, Barrier};

struct Fixture {
    root: PathBuf,
    runtime: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-provider-ledger-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let runtime = root
            .join("profile")
            .join("provider-credentials")
            .join("runtime");
        Self { root, runtime }
    }
    fn adapter(&self) -> SqliteProviderCredentialLedgerAdapter {
        SqliteProviderCredentialLedgerAdapter::with_runtime("a".repeat(64), self.runtime.clone())
    }
    fn seed(&self, provider: Provider) -> CredentialLedger {
        let adapter = self.adapter();
        let previous = adapter.load(provider).unwrap();
        let mut next = previous.clone();
        next.row_version += 1;
        assert!(adapter.compare_exchange(&previous, &next).unwrap());
        next
    }
    fn edit(&self, edit: impl FnOnce(&mut Connection)) {
        let mut opened =
            crate::private_sqlite::open_existing(&self.runtime, FILES, Synchronous::Extra)
                .unwrap()
                .unwrap();
        let (connection, guard) = opened.parts();
        guard.verify_identity().unwrap();
        edit(connection);
        guard.verify_identity().unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn constructor_and_missing_state_are_lazy_and_do_not_create_storage() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    assert!(!fixture.runtime.exists());
    assert_eq!(
        adapter.load(Provider::Linear).unwrap(),
        CredentialLedger::empty("a".repeat(64), Provider::Linear)
    );
    assert_eq!(
        adapter.load(Provider::Jira).unwrap(),
        CredentialLedger::empty("a".repeat(64), Provider::Jira)
    );
    assert!(!fixture.runtime.exists());
    assert!(
        SqliteProviderCredentialLedgerAdapter::new("invalid".into(), || panic!(
            "resolver must remain lazy"
        ))
        .is_err()
    );
}

#[test]
fn empty_database_read_keeps_zero_bytes_and_no_schema() {
    let fixture = Fixture::new();
    drop(
        crate::private_sqlite::open_or_create_strict(&fixture.runtime, FILES, Synchronous::Extra)
            .unwrap(),
    );
    let database = fixture.runtime.join(DATABASE_NAME);
    assert_eq!(std::fs::metadata(&database).unwrap().len(), 0);
    assert_eq!(
        fixture
            .adapter()
            .load(Provider::Linear)
            .unwrap()
            .row_version,
        0
    );
    assert_eq!(std::fs::metadata(&database).unwrap().len(), 0);
}

#[test]
fn cas_rejects_a_stale_snapshot_and_profile_rebinding() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let previous = adapter.load(Provider::Linear).unwrap();
    let next = fixture.seed(Provider::Linear);
    assert_eq!(adapter.load(Provider::Linear).unwrap(), next);
    assert!(!adapter.compare_exchange(&previous, &next).unwrap());
    let rebound = SqliteProviderCredentialLedgerAdapter::with_runtime(
        "b".repeat(64),
        fixture.runtime.clone(),
    );
    assert_eq!(
        rebound.load(Provider::Linear),
        Err(ProviderCredentialError::Corrupt)
    );
    assert_eq!(adapter.load(Provider::Linear).unwrap(), next);
}

#[test]
fn profile_runtimes_are_independent_even_for_the_same_provider() {
    let fixture = Fixture::new();
    let first = fixture.adapter();
    let second = SqliteProviderCredentialLedgerAdapter::with_runtime(
        "b".repeat(64),
        fixture.root.join("other-profile/runtime"),
    );
    let expected = first.load(Provider::Linear).unwrap();
    let mut next = expected.clone();
    next.row_version = 1;
    next.tombstone = true;
    assert!(first.compare_exchange(&expected, &next).unwrap());
    assert_eq!(second.load(Provider::Linear).unwrap().row_version, 0);
    let other = second.load(Provider::Linear).unwrap();
    let mut other_next = other.clone();
    other_next.row_version = 1;
    assert!(second.compare_exchange(&other, &other_next).unwrap());
    assert!(first.load(Provider::Linear).unwrap().tombstone);
    assert!(!second.load(Provider::Linear).unwrap().tombstone);
}

#[test]
fn concurrent_first_creation_preserves_both_provider_documents() {
    let fixture = Fixture::new();
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = [Provider::Jira, Provider::Linear]
        .into_iter()
        .map(|provider| {
            let runtime = fixture.runtime.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let adapter =
                    SqliteProviderCredentialLedgerAdapter::with_runtime("a".repeat(64), runtime);
                let previous = adapter.load(provider).unwrap();
                let mut next = previous.clone();
                next.row_version = 1;
                barrier.wait();
                assert!(adapter.compare_exchange(&previous, &next).unwrap());
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(
        fixture.adapter().load(Provider::Jira).unwrap().row_version,
        1
    );
    assert_eq!(
        fixture
            .adapter()
            .load(Provider::Linear)
            .unwrap()
            .row_version,
        1
    );
}

#[test]
fn concurrent_same_provider_cas_has_exactly_one_winner() {
    let fixture = Fixture::new();
    fixture.seed(Provider::Linear);
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let runtime = fixture.runtime.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let adapter =
                    SqliteProviderCredentialLedgerAdapter::with_runtime("a".repeat(64), runtime);
                let previous = adapter.load(Provider::Linear).unwrap();
                let mut next = previous.clone();
                next.row_version += 1;
                barrier.wait();
                adapter.compare_exchange(&previous, &next).unwrap()
            })
        })
        .collect();
    let wins = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .filter(|won| *won)
        .count();
    assert_eq!(wins, 1);
    assert_eq!(
        fixture
            .adapter()
            .load(Provider::Linear)
            .unwrap()
            .row_version,
        2
    );
}

#[test]
fn unexpected_objects_including_sqlitex_names_and_future_versions_fail_closed() {
    for sql in [
        "CREATE TABLE sqliteXhidden(value TEXT)",
        "CREATE TRIGGER sqliteXskip BEFORE INSERT ON provider_credentials BEGIN SELECT RAISE(IGNORE); END",
        "PRAGMA user_version=99",
    ] {
        let fixture = Fixture::new();
        fixture.seed(Provider::Linear);
        fixture.edit(|connection| connection.execute_batch(sql).unwrap());
        assert_eq!(
            fixture.adapter().load(Provider::Linear),
            Err(ProviderCredentialError::Corrupt)
        );
    }
}

#[test]
fn malformed_oversized_and_cross_profile_documents_are_rejected_without_repair() {
    for document in [
        "{}".to_owned(),
        "x".repeat(MAX_DOCUMENT_BYTES + 1),
        serde_json::to_string(&CredentialLedger {
            row_version: 1,
            ..CredentialLedger::empty("b".repeat(64), Provider::Linear)
        })
        .unwrap(),
    ] {
        let fixture = Fixture::new();
        fixture.seed(Provider::Linear);
        fixture.edit(|connection| {
            connection
                .execute_batch("PRAGMA ignore_check_constraints=ON")
                .unwrap();
            connection
                .execute(
                    "UPDATE provider_credentials SET document=?1",
                    params![document],
                )
                .unwrap();
        });
        assert_eq!(
            fixture.adapter().load(Provider::Linear),
            Err(ProviderCredentialError::Corrupt)
        );
    }
}

#[test]
fn hardlinked_database_is_rejected_without_mutating_the_target() {
    let fixture = Fixture::new();
    fixture.seed(Provider::Linear);
    let database = fixture.runtime.join(DATABASE_NAME);
    let target = fixture.root.join("retained");
    std::fs::hard_link(&database, &target).unwrap();
    let before = std::fs::read(&target).unwrap();
    assert!(fixture.adapter().load(Provider::Linear).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn existing_database_mode_is_never_repaired() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.seed(Provider::Linear);
    let database = fixture.runtime.join(DATABASE_NAME);
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(fixture.adapter().load(Provider::Linear).is_err());
    assert_eq!(
        std::fs::metadata(&database).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

#[cfg(windows)]
#[test]
fn existing_database_acl_is_never_repaired() {
    use std::process::{Command, Stdio};
    let fixture = Fixture::new();
    fixture.seed(Provider::Linear);
    let database = fixture.runtime.join(DATABASE_NAME);
    let mut command = Command::new("icacls.exe");
    command
        .arg(&database)
        .args(["/grant", "*S-1-1-0:(R)"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = crate::subprocess::wait_with_output_timeout(
        crate::subprocess::spawn(&mut command).unwrap(),
        "provider ledger ACL fixture",
        std::time::Duration::from_secs(10),
    )
    .unwrap();
    assert!(output.status.success());
    assert!(fixture.adapter().load(Provider::Linear).is_err());
    let file = std::fs::File::open(&database).unwrap();
    assert!(crate::private_fs::validate_windows_private_owner_and_acl(&database, &file).is_err());
}

#[test]
fn publication_guard_is_checked_again_before_commit_and_cancellation_rolls_back() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let expected = fixture.seed(Provider::Linear);
    let mut next = expected.clone();
    next.row_version += 1;
    let checks = AtomicUsize::new(0);
    let result = adapter.compare_exchange_guarded(&expected, &next, &|| {
        checks.fetch_add(1, Ordering::SeqCst) == 0
    });
    assert_eq!(result, Err(ProviderCredentialError::Cancelled));
    assert!(checks.load(Ordering::SeqCst) >= 2);
    assert_eq!(adapter.load(Provider::Linear).unwrap(), expected);
}

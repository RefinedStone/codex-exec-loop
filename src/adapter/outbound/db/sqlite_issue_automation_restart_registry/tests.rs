use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;

const WORKER: &str = "adapter::outbound::db::sqlite_issue_automation_restart_registry::tests::registry_subprocess_worker";
const WORKER_RUNTIME: &str = "AKRA_TEST_RESTART_REGISTRY_RUNTIME";
const WORKER_START: &str = "AKRA_TEST_RESTART_REGISTRY_START";
const WORKER_ACTION: &str = "AKRA_TEST_RESTART_REGISTRY_ACTION";
const WORKER_BARRIER: &str = "AKRA_TEST_RESTART_REGISTRY_BARRIER";

struct Fixture {
    root: PathBuf,
    runtime: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-restart-registry-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let runtime = root
            .join("profile")
            .join("issue-automation")
            .join("runtime");
        Self { root, runtime }
    }

    fn adapter(&self) -> SqliteIssueAutomationRestartRegistryAdapter {
        SqliteIssueAutomationRestartRegistryAdapter::with_runtime(self.runtime.clone())
    }

    fn entry(&self, number: usize) -> IssueAutomationRestartEntry {
        entry(&self.root, number)
    }

    fn edit(&self, work: impl FnOnce(&mut Connection)) {
        let mut opened =
            crate::private_sqlite::open_existing(&self.runtime, FILES, Synchronous::Extra)
                .unwrap()
                .unwrap();
        let (connection, guard) = opened.parts();
        guard.verify_identity().unwrap();
        work(connection);
        guard.verify_identity().unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn entry(root: &Path, number: usize) -> IssueAutomationRestartEntry {
    IssueAutomationRestartEntry {
        repository_key: format!("akra-repository-incarnation-v1:{number:064x}"),
        workspace_root: root
            .join(format!("checkout-{number}"))
            .display()
            .to_string(),
    }
}

#[test]
fn construction_is_lazy_and_invalid_registration_does_not_resolve_storage() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observer = calls.clone();
    let adapter = SqliteIssueAutomationRestartRegistryAdapter::new(move || {
        observer.fetch_add(1, Ordering::SeqCst);
        bail!("synthetic unavailable profile")
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        adapter
            .register(&IssueAutomationRestartEntry {
                repository_key: String::new(),
                workspace_root: "relative".into(),
            })
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(adapter.list().is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn missing_list_and_unregister_create_neither_directories_nor_database() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    assert!(adapter.list().unwrap().is_empty());
    adapter
        .unregister(&fixture.entry(1).repository_key)
        .unwrap();
    assert!(!fixture.root.join("profile").exists());

    crate::private_directory::ensure_private_directory(&fixture.runtime).unwrap();
    assert!(adapter.list().unwrap().is_empty());
    adapter
        .unregister(&fixture.entry(1).repository_key)
        .unwrap();
    assert_eq!(std::fs::read_dir(&fixture.runtime).unwrap().count(), 0);
}

#[test]
fn register_reopen_update_and_unregister_are_idempotent_and_sorted() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let first = fixture.entry(1);
    let second = fixture.entry(2);
    adapter.register(&second).unwrap();
    adapter.register(&first).unwrap();
    assert_eq!(adapter.list().unwrap(), vec![first.clone(), second.clone()]);

    let before = std::fs::read(fixture.runtime.join(DATABASE_NAME)).unwrap();
    fixture.adapter().register(&first).unwrap();
    fixture
        .adapter()
        .unregister(&fixture.entry(3).repository_key)
        .unwrap();
    assert_eq!(
        std::fs::read(fixture.runtime.join(DATABASE_NAME)).unwrap(),
        before
    );

    let mut moved = first.clone();
    moved.workspace_root = fixture.root.join("moved-checkout").display().to_string();
    fixture.adapter().register(&moved).unwrap();
    assert_eq!(
        fixture.adapter().list().unwrap(),
        vec![moved, second.clone()]
    );
    fixture.adapter().unregister(&first.repository_key).unwrap();
    fixture.adapter().unregister(&first.repository_key).unwrap();
    assert_eq!(fixture.adapter().list().unwrap(), vec![second]);
}

#[test]
fn authority_profiles_with_the_same_repository_key_remain_independent() {
    let fixture = Fixture::new();
    let first = fixture.adapter();
    let other_runtime = fixture
        .root
        .join("other-profile")
        .join("issue-automation")
        .join("runtime");
    let second = SqliteIssueAutomationRestartRegistryAdapter::with_runtime(other_runtime);
    let candidate = fixture.entry(1);
    let mut other_candidate = candidate.clone();
    other_candidate.workspace_root = fixture.root.join("other-checkout").display().to_string();
    first.register(&candidate).unwrap();
    second.register(&other_candidate).unwrap();
    first.unregister(&candidate.repository_key).unwrap();
    assert!(first.list().unwrap().is_empty());
    assert_eq!(second.list().unwrap(), vec![other_candidate]);
}

#[test]
fn missing_checkout_is_preserved_for_the_startup_identity_check() {
    let fixture = Fixture::new();
    let candidate = fixture.entry(1);
    assert!(!Path::new(&candidate.workspace_root).exists());
    fixture.adapter().register(&candidate).unwrap();
    assert_eq!(fixture.adapter().list().unwrap(), vec![candidate]);
}

#[test]
fn empty_database_reads_do_not_initialize_the_schema() {
    let fixture = Fixture::new();
    drop(
        crate::private_sqlite::open_or_create(&fixture.runtime, FILES, Synchronous::Extra).unwrap(),
    );
    let database = fixture.runtime.join(DATABASE_NAME);
    let before = std::fs::read(&database).unwrap();
    assert!(
        before.is_empty(),
        "the fixture must start as a zero-byte database"
    );
    assert!(fixture.adapter().list().unwrap().is_empty());
    assert_eq!(
        std::fs::metadata(&database).unwrap().len(),
        0,
        "list initialized an empty database"
    );
    fixture
        .adapter()
        .unregister(&fixture.entry(1).repository_key)
        .unwrap();
    assert_eq!(
        std::fs::metadata(&database).unwrap().len(),
        0,
        "unregister initialized an empty database"
    );
    assert_eq!(std::fs::read(&database).unwrap(), before);
    fixture.adapter().register(&fixture.entry(1)).unwrap();
    assert_eq!(fixture.adapter().list().unwrap(), vec![fixture.entry(1)]);
}

#[test]
fn invalid_and_oversized_inputs_are_rejected_before_creating_storage() {
    let fixture = Fixture::new();
    let candidate = fixture.entry(1);
    for key in [
        "".to_owned(),
        "x".repeat(MAX_KEY_BYTES + 1),
        "bad\nkey".into(),
        " key".into(),
    ] {
        assert!(
            fixture
                .adapter()
                .register(&IssueAutomationRestartEntry {
                    repository_key: key,
                    ..candidate.clone()
                })
                .is_err()
        );
    }
    let separator = std::path::MAIN_SEPARATOR;
    let parent_alias = format!("{}{separator}..{separator}other", fixture.root.display());
    assert!(
        Path::new(&parent_alias)
            .components()
            .any(|part| part == Component::ParentDir)
    );
    for workspace in [
        "relative".to_owned(),
        parent_alias,
        "bad\0path".into(),
        "x".repeat(MAX_WORKSPACE_BYTES + 1),
    ] {
        assert!(
            fixture
                .adapter()
                .register(&IssueAutomationRestartEntry {
                    workspace_root: workspace,
                    ..candidate.clone()
                })
                .is_err()
        );
    }
    assert!(!fixture.root.join("profile").exists());
}

#[test]
fn byte_length_boundaries_and_unicode_checkout_round_trip() {
    let fixture = Fixture::new();
    let mut candidate = fixture.entry(1);
    candidate.repository_key = "k".repeat(MAX_KEY_BYTES);
    let prefix = format!("{}{}", fixture.root.display(), std::path::MAIN_SEPARATOR);
    candidate.workspace_root =
        format!("{prefix}{}", "x".repeat(MAX_WORKSPACE_BYTES - prefix.len()));
    assert_eq!(candidate.workspace_root.len(), MAX_WORKSPACE_BYTES);
    fixture.adapter().register(&candidate).unwrap();
    assert_eq!(fixture.adapter().list().unwrap(), vec![candidate.clone()]);
    candidate.workspace_root = fixture.root.join("한글 저장소").display().to_string();
    fixture.adapter().register(&candidate).unwrap();
    assert_eq!(fixture.adapter().list().unwrap(), vec![candidate]);
}

#[test]
fn count_limit_allows_updates_and_removals_but_no_silent_truncation() {
    let fixture = Fixture::new();
    fixture.adapter().register(&fixture.entry(0)).unwrap();
    fixture.edit(|connection| {
        let transaction = connection.transaction().unwrap();
        for number in 1..MAX_ENTRIES {
            let candidate = fixture.entry(number);
            transaction
                .execute(
                    "INSERT INTO restart_candidates(repository_key,workspace_root) VALUES(?1,?2)",
                    params![candidate.repository_key, candidate.workspace_root],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
    });
    assert_eq!(fixture.adapter().list().unwrap().len(), MAX_ENTRIES);
    assert!(
        fixture
            .adapter()
            .register(&fixture.entry(MAX_ENTRIES))
            .is_err()
    );
    let mut moved = fixture.entry(0);
    moved.workspace_root = fixture.root.join("moved").display().to_string();
    fixture.adapter().register(&moved).unwrap();
    fixture.adapter().unregister(&moved.repository_key).unwrap();
    fixture
        .adapter()
        .register(&fixture.entry(MAX_ENTRIES))
        .unwrap();
    fixture.edit(|connection| {
        let candidate = fixture.entry(MAX_ENTRIES + 1);
        connection
            .execute(
                "INSERT INTO restart_candidates(repository_key,workspace_root) VALUES(?1,?2)",
                params![candidate.repository_key, candidate.workspace_root],
            )
            .unwrap();
    });
    assert!(fixture.adapter().list().is_err());
    assert!(
        fixture
            .adapter()
            .register(&fixture.entry(MAX_ENTRIES + 2))
            .is_err()
    );
}

#[test]
fn wrong_identity_future_schema_and_extra_objects_are_not_repaired() {
    for mutation in [
        "PRAGMA application_id=7;",
        "PRAGMA user_version=2;",
        "PRAGMA application_id=0; PRAGMA user_version=0;",
        "ALTER TABLE restart_candidates ADD COLUMN unexpected TEXT;",
        "CREATE TABLE unexpected(value TEXT);",
        "CREATE TABLE sqliteXhidden(value TEXT);",
        "CREATE TRIGGER sqliteXskip BEFORE INSERT ON restart_candidates BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let fixture = Fixture::new();
        fixture.adapter().register(&fixture.entry(1)).unwrap();
        fixture.edit(|connection| connection.execute_batch(mutation).unwrap());
        let before = std::fs::read(fixture.runtime.join(DATABASE_NAME)).unwrap();
        assert!(fixture.adapter().list().is_err(), "{mutation}");
        assert!(
            fixture
                .adapter()
                .unregister(&fixture.entry(1).repository_key)
                .is_err(),
            "{mutation}"
        );
        assert!(
            fixture.adapter().register(&fixture.entry(2)).is_err(),
            "{mutation}"
        );
        assert_eq!(
            std::fs::read(fixture.runtime.join(DATABASE_NAME)).unwrap(),
            before
        );
    }
}

#[test]
fn malformed_rows_fail_instead_of_becoming_restart_candidates() {
    for mutation in [
        "UPDATE restart_candidates SET workspace_root='relative';",
        "UPDATE restart_candidates SET workspace_root=char(0);",
        "UPDATE restart_candidates SET repository_key=CAST(x'ff' AS TEXT);",
        "UPDATE restart_candidates SET repository_key=x'0102';",
        "UPDATE restart_candidates SET repository_key=replace(hex(zeroblob(129)),'0','k');",
        "UPDATE restart_candidates SET workspace_root=replace(hex(zeroblob(8193)),'0','x');",
    ] {
        let fixture = Fixture::new();
        fixture.adapter().register(&fixture.entry(1)).unwrap();
        fixture.edit(|connection| {
            connection
                .execute_batch("PRAGMA ignore_check_constraints=ON;")
                .unwrap();
            connection.execute_batch(mutation).unwrap();
        });
        let before = std::fs::read(fixture.runtime.join(DATABASE_NAME)).unwrap();
        assert!(fixture.adapter().list().is_err(), "{mutation}");
        assert!(
            fixture
                .adapter()
                .unregister(&fixture.entry(1).repository_key)
                .is_err(),
            "{mutation}"
        );
        assert!(
            fixture.adapter().register(&fixture.entry(2)).is_err(),
            "{mutation}"
        );
        assert_eq!(
            std::fs::read(fixture.runtime.join(DATABASE_NAME)).unwrap(),
            before
        );
    }
}

#[test]
fn corrupt_database_and_orphan_sidecars_are_not_replaced_or_hidden() {
    let fixture = Fixture::new();
    fixture.adapter().register(&fixture.entry(1)).unwrap();
    let database = fixture.runtime.join(DATABASE_NAME);
    std::fs::write(&database, b"corrupt registry").unwrap();
    assert!(fixture.adapter().list().is_err());
    assert!(
        fixture
            .adapter()
            .unregister(&fixture.entry(1).repository_key)
            .is_err()
    );
    assert!(fixture.adapter().register(&fixture.entry(2)).is_err());
    assert_eq!(std::fs::read(&database).unwrap(), b"corrupt registry");

    for sidecar in [
        "restart-registry.db-journal",
        "restart-registry.db-wal",
        "restart-registry.db-shm",
    ] {
        let fixture = Fixture::new();
        crate::private_directory::ensure_private_directory(&fixture.runtime).unwrap();
        std::fs::write(fixture.runtime.join(sidecar), b"retained").unwrap();
        assert!(fixture.adapter().list().is_err());
        assert!(
            fixture
                .adapter()
                .unregister(&fixture.entry(1).repository_key)
                .is_err()
        );
        assert!(fixture.adapter().register(&fixture.entry(1)).is_err());
        assert!(!fixture.runtime.join(DATABASE_NAME).exists());
        assert_eq!(
            std::fs::read(fixture.runtime.join(sidecar)).unwrap(),
            b"retained"
        );
    }
}

#[test]
fn hardlinked_database_is_rejected_without_changing_its_target() {
    let fixture = Fixture::new();
    fixture.adapter().register(&fixture.entry(1)).unwrap();
    let database = fixture.runtime.join(DATABASE_NAME);
    let other = fixture.root.join("retained-db");
    std::fs::hard_link(&database, &other).unwrap();
    let before = std::fs::read(&other).unwrap();
    assert!(fixture.adapter().list().is_err());
    assert!(
        fixture
            .adapter()
            .unregister(&fixture.entry(1).repository_key)
            .is_err()
    );
    assert!(fixture.adapter().register(&fixture.entry(2)).is_err());
    assert_eq!(std::fs::read(other).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn unsafe_existing_permissions_are_not_repaired() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.adapter().register(&fixture.entry(1)).unwrap();
    let database = fixture.runtime.join(DATABASE_NAME);
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(fixture.adapter().list().is_err());
    assert!(
        fixture
            .adapter()
            .unregister(&fixture.entry(1).repository_key)
            .is_err()
    );
    assert!(fixture.adapter().register(&fixture.entry(2)).is_err());
    assert_eq!(
        std::fs::metadata(database).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

#[cfg(windows)]
#[test]
fn unsafe_existing_acl_is_not_repaired() {
    let fixture = Fixture::new();
    fixture.adapter().register(&fixture.entry(1)).unwrap();
    let database = fixture.runtime.join(DATABASE_NAME);
    let mut command = Command::new("icacls.exe");
    command
        .arg(&database)
        .args(["/grant", "*S-1-1-0:(R)"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = crate::subprocess::wait_with_output_timeout(
        crate::subprocess::spawn(&mut command).unwrap(),
        "restart registry ACL fixture",
        Duration::from_secs(10),
    )
    .unwrap();
    assert!(output.status.success());
    assert!(fixture.adapter().list().is_err());
    assert!(
        fixture
            .adapter()
            .unregister(&fixture.entry(1).repository_key)
            .is_err()
    );
    assert!(fixture.adapter().register(&fixture.entry(2)).is_err());
    let file = std::fs::File::open(&database).unwrap();
    assert!(crate::private_fs::validate_windows_private_owner_and_acl(&database, &file).is_err());
}

fn spawn_worker(runtime: &Path, start: usize, action: &str) -> crate::subprocess::ManagedChild {
    spawn_worker_with_barrier(runtime, start, action, None)
}

fn spawn_worker_with_barrier(
    runtime: &Path,
    start: usize,
    action: &str,
    barrier: Option<&Path>,
) -> crate::subprocess::ManagedChild {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", WORKER, "--nocapture"])
        .env(WORKER_RUNTIME, runtime)
        .env(WORKER_START, start.to_string())
        .env(WORKER_ACTION, action)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(barrier) = barrier {
        command.env(WORKER_BARRIER, barrier);
    } else {
        command.env_remove(WORKER_BARRIER);
    }
    crate::subprocess::spawn(&mut command).unwrap()
}

fn finish_worker(child: crate::subprocess::ManagedChild) {
    let output = crate::subprocess::wait_with_output_timeout(
        child,
        "restart registry concurrent writer",
        Duration::from_secs(30),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("restart registry worker completed"));
}

fn await_worker_signal(path: &Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !path.is_file() {
        assert!(
            std::time::Instant::now() < deadline,
            "registry worker signal timed out: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn independent_processes_create_a_missing_registry_together() {
    let fixture = Fixture::new();
    let barrier = fixture.root.join("first-registration-barrier");
    std::fs::create_dir(&barrier).unwrap();
    let first = spawn_worker_with_barrier(&fixture.runtime, 0, "register", Some(&barrier));
    let second = spawn_worker_with_barrier(&fixture.runtime, 16, "register", Some(&barrier));
    await_worker_signal(&barrier.join("ready-0"));
    await_worker_signal(&barrier.join("ready-16"));
    // Both separate processes have started; neither may create the profile before release.
    assert!(!fixture.root.join("profile").exists());
    assert!(!fixture.runtime.join(DATABASE_NAME).exists());
    std::fs::write(barrier.join("released"), b"start together").unwrap();
    finish_worker(first);
    finish_worker(second);
    let expected = (0..32)
        .map(|number| entry(&fixture.runtime, number))
        .collect::<Vec<_>>();
    assert_eq!(fixture.adapter().list().unwrap(), expected);
}

#[test]
fn independent_process_updates_keep_other_repository_entries() {
    let fixture = Fixture::new();
    fixture.adapter().register(&fixture.entry(1000)).unwrap();
    let first = spawn_worker(&fixture.runtime, 0, "register");
    let second = spawn_worker(&fixture.runtime, 16, "register");
    finish_worker(first);
    finish_worker(second);
    assert_eq!(fixture.adapter().list().unwrap().len(), 33);

    let removal = spawn_worker(&fixture.runtime, 0, "unregister");
    let addition = spawn_worker(&fixture.runtime, 32, "register");
    finish_worker(removal);
    finish_worker(addition);
    let entries = fixture.adapter().list().unwrap();
    assert_eq!(entries.len(), 33);
    assert!(
        entries
            .iter()
            .any(|item| item.repository_key == fixture.entry(1000).repository_key)
    );
    for number in 0..16 {
        assert!(
            !entries
                .iter()
                .any(|item| item.repository_key == fixture.entry(number).repository_key)
        );
    }
    for number in 16..48 {
        assert!(
            entries
                .iter()
                .any(|item| item.repository_key == fixture.entry(number).repository_key)
        );
    }
}

#[test]
fn registry_subprocess_worker() {
    let Some(runtime) = std::env::var_os(WORKER_RUNTIME) else {
        return;
    };
    let runtime = PathBuf::from(runtime);
    let start: usize = std::env::var(WORKER_START).unwrap().parse().unwrap();
    let action = std::env::var(WORKER_ACTION).unwrap();
    if let Some(barrier) = std::env::var_os(WORKER_BARRIER) {
        let barrier = PathBuf::from(barrier);
        std::fs::write(barrier.join(format!("ready-{start}")), b"ready").unwrap();
        await_worker_signal(&barrier.join("released"));
    }
    let adapter = SqliteIssueAutomationRestartRegistryAdapter::with_runtime(runtime.clone());
    for number in start..start + 16 {
        let candidate = entry(&runtime, number);
        match action.as_str() {
            "register" => adapter.register(&candidate).unwrap(),
            "unregister" => adapter.unregister(&candidate.repository_key).unwrap(),
            _ => panic!("unsupported registry fixture action"),
        }
    }
    println!("restart registry worker completed");
}

#[test]
fn concurrent_first_registrations_create_one_registry_without_losing_either_entry() {
    for _ in 0..4 {
        let fixture = Fixture::new();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let workers = (1..=2)
            .map(|number| {
                let adapter = fixture.adapter();
                let candidate = fixture.entry(number);
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    adapter.register(&candidate)
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        assert_eq!(
            fixture.adapter().list().unwrap(),
            vec![fixture.entry(1), fixture.entry(2)]
        );
    }
}

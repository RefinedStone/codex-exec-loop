use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::Instant;

use super::*;

const WORKER_TEST: &str = "adapter::outbound::filesystem::evidence_vault_store::binding::tests::processes::binding_process_worker";
const TEST_WAIT: Duration = Duration::from_secs(15);

struct Worker(Option<Child>);

impl Worker {
    fn start(
        fixture: &Fixture,
        mode: &str,
        label: &str,
        scope: &VaultRecordScope,
        expected: Option<&VaultBindingToken>,
    ) -> Self {
        let digest = expected
            .and_then(VaultBindingToken::ciphertext_sha256)
            .map(|digest| {
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            })
            .unwrap_or_default();
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", WORKER_TEST, "--nocapture"])
            .env("AKRA_VAULT_BINDING_TEST_ROOT", &fixture.root)
            .env("AKRA_VAULT_BINDING_TEST_MODE", mode)
            .env("AKRA_VAULT_BINDING_TEST_LABEL", label)
            .env("AKRA_VAULT_BINDING_TEST_ISSUE", &scope.issue.issue_id)
            .env(
                "AKRA_VAULT_BINDING_TEST_WORKSPACE",
                &scope.issue.project_scope.workspace_dir,
            )
            .env("AKRA_VAULT_BINDING_TEST_EXPECTED", digest)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Self(Some(child))
    }

    fn finish(mut self) -> Output {
        let mut child = self.0.take().unwrap();
        let deadline = Instant::now() + TEST_WAIT;
        loop {
            if child.try_wait().unwrap().is_some() {
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "child failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                return output;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "child timed out: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn kill_and_wait(mut self) {
        let mut child = self.0.take().unwrap();
        child.kill().unwrap();
        let status = child.wait().unwrap();
        assert!(!status.success());
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + TEST_WAIT;
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "child did not reach {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn marker(root: &Path, label: &str, phase: &str) -> PathBuf {
    root.join(format!("test-{label}-{phase}"))
}

fn process_request(expected: VaultBindingToken, label: &str) -> VaultBindingCommitRequest {
    request(expected, &format!("process-{label}"), desired_fields(label))
}

fn child_expected(scope: VaultRecordScope) -> VaultBindingToken {
    let digest = std::env::var("AKRA_VAULT_BINDING_TEST_EXPECTED").unwrap();
    let digest = if digest.is_empty() {
        None
    } else {
        assert_eq!(digest.len(), 64);
        let mut bytes = [0; 32];
        for (index, value) in bytes.iter_mut().enumerate() {
            *value = u8::from_str_radix(&digest[index * 2..index * 2 + 2], 16).unwrap();
        }
        Some(bytes)
    };
    VaultBindingToken::observed(scope, digest)
}

fn pause_until_killed(root: PathBuf, label: String, phase: &str) -> ! {
    fs::write(marker(&root, &label, "paused"), phase.as_bytes()).unwrap();
    let deadline = Instant::now() + TEST_WAIT * 2;
    loop {
        assert!(
            Instant::now() < deadline,
            "the synthetic crash worker outlived its parent",
        );
        thread::park_timeout(Duration::from_secs(1));
    }
}

#[test]
fn binding_process_worker() {
    // Environment controls only this test binary entry; the production adapter has no override.
    let Some(root) = std::env::var_os("AKRA_VAULT_BINDING_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let mode = std::env::var("AKRA_VAULT_BINDING_TEST_MODE").unwrap();
    let label = std::env::var("AKRA_VAULT_BINDING_TEST_LABEL").unwrap();
    let mut load = load_request();
    load.scope.issue.issue_id = std::env::var("AKRA_VAULT_BINDING_TEST_ISSUE").unwrap();
    load.scope.issue.project_scope.workspace_dir =
        std::env::var("AKRA_VAULT_BINDING_TEST_WORKSPACE").unwrap();
    let store = FilesystemEvidenceVaultStoreAdapter::new(root.clone());
    fs::write(marker(&root, &label, "attempted"), b"ready").unwrap();

    if mode == "probe" {
        let result = PrivateRuntimeLease::acquire(
            root.clone(),
            binding_lock_name(&load.scope),
            Duration::ZERO,
        );
        fs::write(
            marker(&root, &label, "result"),
            if result.is_err() {
                b"blocked".as_slice()
            } else {
                b"acquired".as_slice()
            },
        )
        .unwrap();
        return;
    }
    if mode == "hold" {
        let tx = store.begin_binding(load).unwrap();
        fs::write(marker(&root, &label, "ready"), b"held").unwrap();
        wait_for(&marker(&root, &label, "release"));
        drop(tx);
        return;
    }
    let expected = child_expected(load.scope.clone());
    if mode == "race" {
        wait_for(&root.join("test-race-start"));
    } else if mode == "crash-field" {
        let hook_root = root.clone();
        let hook_label = label.clone();
        test_hooks::install_after_field(move |field| {
            assert_eq!(field, VaultRecordField::Title);
            pause_until_killed(hook_root, hook_label, "title");
        });
    } else if mode == "crash-pointer" {
        let hook_root = root.clone();
        let hook_label = label.clone();
        test_hooks::install_before_pointer(move || {
            pause_until_killed(hook_root, hook_label, "fields");
        });
    } else {
        panic!("unknown synthetic process scenario");
    }
    let result = store
        .begin_binding(load)
        .unwrap()
        .commit(process_request(expected, &label))
        .unwrap();
    let outcome = match result {
        VaultBindingCommitResult::Committed(_) => "committed",
        VaultBindingCommitResult::AlreadyCommitted(_) => "already",
        VaultBindingCommitResult::Conflict => "conflict",
    };
    fs::write(marker(&root, &label, "result"), outcome).unwrap();
}

#[test]
fn two_processes_with_the_same_review_commit_only_one_complete_mapping() {
    let fixture = Fixture::new("process-race");
    let store = fixture.store();
    let expected = seed_legacy(&store);
    let scope = load_request().scope;
    let first = Worker::start(&fixture, "race", "A", &scope, Some(&expected));
    let second = Worker::start(&fixture, "race", "B", &scope, Some(&expected));
    wait_for(&marker(&fixture.root, "A", "attempted"));
    wait_for(&marker(&fixture.root, "B", "attempted"));
    fs::write(fixture.root.join("test-race-start"), b"go").unwrap();
    first.finish();
    second.finish();
    let outcomes =
        ["A", "B"].map(|label| fs::read_to_string(marker(&fixture.root, label, "result")).unwrap());
    assert_eq!(
        outcomes
            .iter()
            .filter(|value| value.as_str() == "committed")
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|value| value.as_str() == "conflict")
            .count(),
        1
    );
    let winner = if outcomes[0] == "committed" { "A" } else { "B" };
    let loser = if winner == "A" { "B" } else { "A" };
    assert_eq!(current_fields(&store), desired_fields(winner));
    let before = all_records(&fixture.root);
    assert_eq!(
        store
            .begin_binding(load_request())
            .unwrap()
            .commit(process_request(expected.clone(), loser))
            .unwrap(),
        VaultBindingCommitResult::Conflict
    );
    assert!(matches!(
        store
            .begin_binding(load_request())
            .unwrap()
            .commit(process_request(expected, winner))
            .unwrap(),
        VaultBindingCommitResult::AlreadyCommitted(_)
    ));
    assert_eq!(all_records(&fixture.root), before);
}

#[test]
fn different_full_scopes_can_hold_transactions_while_the_same_scope_is_locked() {
    let fixture = Fixture::new("scope-locks");
    let scope = load_request().scope;
    let first = Worker::start(&fixture, "hold", "A", &scope, None);
    wait_for(&marker(&fixture.root, "A", "ready"));
    let mut other_scope = scope.clone();
    // Remote coordinates are identical. Workspace identity must still select another lease.
    other_scope
        .issue
        .project_scope
        .workspace_dir
        .push_str("-other");
    let other = Worker::start(&fixture, "hold", "B", &other_scope, None);
    wait_for(&marker(&fixture.root, "B", "ready"));
    let same = Worker::start(&fixture, "probe", "C", &scope, None);
    same.finish();
    assert_eq!(
        fs::read_to_string(marker(&fixture.root, "C", "result")).unwrap(),
        "blocked"
    );
    fs::write(marker(&fixture.root, "A", "release"), b"release").unwrap();
    fs::write(marker(&fixture.root, "B", "release"), b"release").unwrap();
    first.finish();
    other.finish();
    let released = Worker::start(&fixture, "probe", "D", &scope, None);
    released.finish();
    assert_eq!(
        fs::read_to_string(marker(&fixture.root, "D", "result")).unwrap(),
        "acquired"
    );
}

#[test]
fn killed_processes_leave_old_bindings_readable_and_partial_generations_retryable() {
    for mode in ["crash-field", "crash-pointer"] {
        let fixture = Fixture::new("process-crash");
        let store = fixture.store();
        let expected = seed_legacy(&store);
        let operation = process_request(expected.clone(), "A");
        let before = fs::read(pointer_path(&fixture.root)).unwrap();
        let child = Worker::start(&fixture, mode, "A", &load_request().scope, Some(&expected));
        wait_for(&marker(&fixture.root, "A", "paused"));
        child.kill_and_wait();
        assert_eq!(fs::read(pointer_path(&fixture.root)).unwrap(), before);
        assert_eq!(current_fields(&store), old_fields());
        let mut tx = store.begin_binding(load_request()).unwrap();
        let partial = tx
            .load_version_fields(&operation.generation_version(&key()).unwrap())
            .unwrap();
        assert_eq!(partial.title, operation.fields.title);
        if mode == "crash-field" {
            assert_eq!(partial.body, None);
        } else {
            assert_eq!(partial.body, operation.fields.body);
        }
        assert!(matches!(
            tx.commit(operation.clone()).unwrap(),
            VaultBindingCommitResult::Committed(_)
        ));
        assert_eq!(current_fields(&store), operation.fields);
        assert!(matches!(
            store
                .begin_binding(load_request())
                .unwrap()
                .commit(operation)
                .unwrap(),
            VaultBindingCommitResult::AlreadyCommitted(_)
        ));
    }
}

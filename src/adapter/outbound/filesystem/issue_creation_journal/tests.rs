#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;
use crate::adapter::outbound::filesystem::secure_fs;
use crate::domain::issue_creation::{
    IssueCreationApproval, IssueCreationApprovedCopy, IssueCreationIntentRecord,
    IssueCreationIntentState, IssueRepositoryReference,
};
use crate::domain::issue_work::{
    IssueFieldHash, IssueProjectScope, IssueProvider, IssueSharedCopy,
};

struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    runtime: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "akra-create-journal-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let workspace = root.join("workspace");
        let runtime = root.join("runtime");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            root,
            workspace,
            runtime,
        }
    }

    fn adapter(&self) -> IssueCreationJournalAdapter {
        let runtime = self.runtime.clone();
        IssueCreationJournalAdapter::new(move |_| Ok(runtime.clone()))
    }

    fn record(&self) -> IssueCreationIntentRecord {
        IssueCreationIntentRecord {
            intent_id: "intent-1".to_string(),
            repository: IssueRepositoryReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".to_string(),
                repository_id: "R_1".to_string(),
                project_scope: IssueProjectScope {
                    workspace_dir: self.workspace.to_str().unwrap().to_string(),
                },
            },
            state: IssueCreationIntentState::Prepared,
            approved: None,
            pending_vault_fields: Vec::new(),
            created: None,
            observed_hash: None,
            write_response_mismatch: false,
            restore_link_finalized: false,
            shared_copy_only: false,
            predecessor_intent_id: None,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn approve(record: &mut IssueCreationIntentRecord) {
    let copy = IssueSharedCopy {
        title: "T".to_string(),
        body: "masked".to_string(),
    };
    record.approved = Some(IssueCreationApprovedCopy {
        approval: IssueCreationApproval {
            repository: record.repository.clone(),
            intent_id: record.intent_id.clone(),
            shared_copy_hash: IssueFieldHash::of_fields(&copy.title, &copy.body),
            policy_version: "policy-1".to_string(),
            template_version: None,
        },
        shared_copy: copy,
    });
    record.state = IssueCreationIntentState::Recorded;
}

mod workspace_identity {
    use std::sync::Arc;

    use super::*;
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::application::port::inbound::issue_draft_port::IssueCreationTarget;
    use crate::application::port::outbound::evidence_vault_port::{
        EvidenceVaultStorePort, VaultPlaintext, VaultRecordLoadRequest, VaultRecordSaveRequest,
        VaultStoreError,
    };
    use crate::application::port::outbound::issue_create_port::{
        IssueCreatePort, IssueCreateRequest, IssueCreateResponse, IssueRepositoryResolveRequest,
    };
    use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
    use crate::application::port::outbound::issue_snapshot_port::{
        IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest,
        IssueSnapshotResponse,
    };
    use crate::application::port::outbound::issue_update_port::{
        IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
    };
    use crate::application::service::issue_creation::IssueCreationService;
    use crate::application::service::issue_publish_journal::IssueJournaledPublishService;
    use crate::domain::issue_publish_intent::IssuePublishIntentRecord;

    struct RepositoryOnly;

    impl IssueCreatePort for RepositoryOnly {
        fn resolve_repository(
            &self,
            request: IssueRepositoryResolveRequest,
        ) -> Result<IssueRepositoryReference> {
            Ok(IssueRepositoryReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".to_string(),
                repository_id: "R_1".to_string(),
                project_scope: request.project_scope,
            })
        }
        fn create_issue(&self, _: IssueCreateRequest) -> Result<IssueCreateResponse> {
            panic!("opening a draft must never create an issue")
        }
    }

    impl IssueSnapshotPort for RepositoryOnly {
        fn resolve(&self, _: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
            panic!("opening a creation intent must not resolve an existing issue")
        }
        fn read(&self, _: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
            panic!("opening a creation intent must not read an existing issue")
        }
    }

    impl IssueUpdatePort for RepositoryOnly {
        fn update_title_and_body(&self, _: IssueUpdateRequest) -> Result<IssueUpdateResponse> {
            panic!("opening a draft must never update an issue")
        }
    }

    impl EvidenceVaultStorePort for RepositoryOnly {
        fn save(&self, _: VaultRecordSaveRequest) -> Result<(), VaultStoreError> {
            panic!("opening a draft must not write the Vault")
        }
        fn load(&self, _: VaultRecordLoadRequest) -> Result<VaultPlaintext, VaultStoreError> {
            panic!("opening a draft must not read the Vault")
        }
    }

    impl IssuePublishIntentJournalPort for RepositoryOnly {
        fn save(&self, _: &str, _: &IssuePublishIntentRecord) -> Result<()> {
            panic!("opening a draft must not write the publish journal")
        }
        fn load(&self, _: &str) -> Result<Vec<IssuePublishIntentRecord>> {
            panic!("opening a draft must not read the publish journal")
        }
    }

    fn service(fixture: &Fixture) -> (IssueCreationService, Arc<IssueCreationJournalAdapter>) {
        let runtime = fixture.runtime.clone();
        let journal = Arc::new(
            IssueCreationJournalAdapter::new(move |_| Ok(runtime.clone())).with_workspace_resolver(
                |cwd| {
                    SqlitePlanningAuthorityAdapter::resolve_primary_checkout_root(cwd)?
                        .and_then(|path| path.to_str().map(str::to_string))
                        .ok_or_else(|| anyhow::anyhow!("the repository has no primary checkout"))
                },
            ),
        );
        let ports = Arc::new(RepositoryOnly);
        let publisher = Arc::new(IssueJournaledPublishService::new(
            ports.clone(),
            ports.clone(),
            ports.clone(),
            ports.clone(),
        ));
        (
            IssueCreationService::new(
                ports.clone(),
                ports.clone(),
                journal.clone(),
                ports,
                publisher,
            ),
            journal,
        )
    }

    fn git(workspace: &Path, args: &[&str]) {
        let output = crate::test_utils::git_command()
            .current_dir(workspace)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=Akra Scope Test",
                "-c",
                "user.email=scope@example.invalid",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn linked_worktree_subdirectory_and_alias_reopen_one_durable_creation_intent() {
        let fixture = Fixture::new("kernel-project-scope");
        git(&fixture.workspace, &["init", "--quiet"]);
        git(
            &fixture.workspace,
            &["commit", "--allow-empty", "--quiet", "-m", "scope fixture"],
        );
        let linked = fixture.root.join("linked-worktree");
        git(
            &fixture.workspace,
            &[
                "worktree",
                "add",
                "--quiet",
                "--detach",
                linked.to_str().unwrap(),
                "HEAD",
            ],
        );
        let nested = linked.join("nested");
        fs::create_dir(&nested).unwrap();
        let alias = fixture.root.join("linked-alias");
        symlink(&linked, &alias).unwrap();
        let target = |path: &Path| IssueCreationTarget {
            cwd: path.to_str().unwrap().to_string(),
            owner: "sample".to_string(),
            repository: "widgets".to_string(),
        };
        let (first, journal) = service(&fixture);
        let opened = first.open(&target(&fixture.workspace)).unwrap();
        for workspace in [&linked, &nested, &alias] {
            assert_eq!(first.open(&target(workspace)).unwrap(), opened);
        }
        let (restarted, _) = service(&fixture);
        assert_eq!(restarted.open(&target(&alias)).unwrap(), opened);
        assert_eq!(
            opened.repository.project_scope.workspace_dir,
            fixture.workspace.to_str().unwrap()
        );
        assert_eq!(
            journal
                .begin(alias.to_str().unwrap())
                .unwrap()
                .records()
                .unwrap(),
            vec![opened],
        );
    }
}

#[test]
fn durable_states_survive_a_new_adapter_and_aliases_share_workspace_identity() {
    let fixture = Fixture::new("restart");
    let alias = fixture.root.join("workspace-alias");
    symlink(&fixture.workspace, &alias).unwrap();
    let adapter = fixture.adapter();
    let workspace = fixture.workspace.to_str().unwrap();
    assert_eq!(
        adapter
            .canonical_workspace(alias.to_str().unwrap())
            .unwrap(),
        adapter.canonical_workspace(workspace).unwrap()
    );
    let mut record = fixture.record();
    {
        let mut transaction = adapter.begin(alias.to_str().unwrap()).unwrap();
        transaction.save(&record).unwrap();
        approve(&mut record);
        transaction.save(&record).unwrap();
        record.state = IssueCreationIntentState::Sent;
        transaction.save(&record).unwrap();
    }
    let restarted = fixture.adapter();
    let records = restarted.begin(workspace).unwrap().records().unwrap();
    assert_eq!(records, vec![record]);
}

#[test]
fn corrupt_oversized_and_approval_mismatched_journals_fail_before_transaction() {
    let fixture = Fixture::new("corrupt");
    let adapter = fixture.adapter();
    let path = fixture.runtime.join(unix::JOURNAL_NAME);
    fs::write(&path, "{\"unexpected-remote-plaintext\":").unwrap();
    let error = adapter
        .begin(fixture.workspace.to_str().unwrap())
        .err()
        .unwrap();
    assert!(!format!("{error:#}").contains("unexpected-remote-plaintext"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\"unexpected-remote-plaintext\":"
    );

    let oversized = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    oversized.set_len(8 * 1024 * 1024 + 1).unwrap();
    assert!(adapter.begin(fixture.workspace.to_str().unwrap()).is_err());
    assert_eq!(fs::metadata(&path).unwrap().len(), 8 * 1024 * 1024 + 1);

    let mut record = fixture.record();
    approve(&mut record);
    record.approved.as_mut().unwrap().shared_copy.body = "different".to_string();
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(adapter.begin(fixture.workspace.to_str().unwrap()).is_err());
}

#[test]
fn approved_copy_cannot_be_replaced_even_if_a_new_hash_is_supplied() {
    let fixture = Fixture::new("frozen");
    let adapter = fixture.adapter();
    let mut transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
    let mut record = fixture.record();
    approve(&mut record);
    transaction.save(&record).unwrap();
    let before = transaction.records().unwrap();
    let approved = record.approved.as_mut().unwrap();
    approved.shared_copy.body.push_str("changed");
    approved.approval.shared_copy_hash =
        IssueFieldHash::of_fields(&approved.shared_copy.title, &approved.shared_copy.body);
    assert!(transaction.save(&record).is_err());
    assert_eq!(transaction.records().unwrap(), before);
}

#[test]
fn held_copy_can_resume_shared_only_and_keep_sent_evidence_when_restarted() {
    let fixture = Fixture::new("held-resume");
    let adapter = fixture.adapter();
    let mut transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
    let mut held = fixture.record();
    approve(&mut held);
    held.state = IssueCreationIntentState::Held;
    held.pending_vault_fields =
        vec![crate::domain::issue_publish_intent::IssuePublishVaultField::Body];
    transaction.save(&held).unwrap();
    let frozen = held.approved.clone();
    held.pending_vault_fields.clear();
    held.shared_copy_only = true;
    held.state = IssueCreationIntentState::Recorded;
    transaction.save(&held).unwrap();
    held.state = IssueCreationIntentState::Sent;
    transaction.save(&held).unwrap();
    let mut next = fixture.record();
    next.intent_id = "intent-2".to_string();
    next.predecessor_intent_id = Some(held.intent_id.clone());
    transaction.save(&next).unwrap();
    let records = transaction.records().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].approved, frozen);
    assert_eq!(records[0].state, IssueCreationIntentState::Sent);
    assert_eq!(records[1], next);
}

#[test]
fn duplicate_intents_and_foreign_workspace_writes_are_rejected() {
    let fixture = Fixture::new("identity");
    let adapter = fixture.adapter();
    let record = fixture.record();
    {
        let mut transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
        let mut other = record.clone();
        other.repository.project_scope.workspace_dir = "/different-workspace".to_string();
        assert!(transaction.save(&other).is_err());
        transaction.save(&record).unwrap();
    }
    let line = serde_json::to_string(&record).unwrap();
    fs::write(
        fixture.runtime.join(unix::JOURNAL_NAME),
        format!("{line}\n{line}\n"),
    )
    .unwrap();
    assert!(adapter.begin(fixture.workspace.to_str().unwrap()).is_err());
}

#[test]
fn a_saved_state_remains_readable_after_a_post_replacement_error() {
    let fixture = Fixture::new("durability");
    let adapter = fixture.adapter();
    let mut transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
    let record = fixture.record();
    secure_fs::install_after_atomic_replace_error_hook("synthetic sync failure");
    assert!(transaction.save(&record).is_err());
    assert_eq!(transaction.records().unwrap(), vec![record]);
}

#[test]
fn lock_symlinks_fifos_hardlinks_and_nonprivate_runtime_are_rejected() {
    for kind in ["symlink", "fifo", "hardlink", "permissions"] {
        let fixture = Fixture::new(kind);
        let lock = fixture.runtime.join(unix::LOCK_NAME);
        match kind {
            "symlink" => {
                let target = fixture.root.join("external");
                fs::write(&target, "unchanged").unwrap();
                symlink(target, &lock).unwrap();
            }
            "fifo" => {
                use std::ffi::CString;
                use std::os::unix::ffi::OsStrExt;
                let path = CString::new(lock.as_os_str().as_bytes()).unwrap();
                // SAFETY: the fixture path is a live NUL-terminated string.
                assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
            }
            "hardlink" => {
                let target = fixture.root.join("external");
                fs::write(&target, "unchanged").unwrap();
                fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
                fs::hard_link(target, &lock).unwrap();
            }
            _ => fs::set_permissions(&fixture.runtime, fs::Permissions::from_mode(0o755)).unwrap(),
        }
        let adapter = fixture.adapter();
        assert!(
            adapter.begin(fixture.workspace.to_str().unwrap()).is_err(),
            "{kind}"
        );
    }
}

#[test]
fn journal_symlinks_fifos_and_runtime_directory_replacement_are_rejected() {
    let fixture = Fixture::new("journal-alias");
    let adapter = fixture.adapter();
    let journal = fixture.runtime.join(unix::JOURNAL_NAME);
    let outside = fixture.root.join("external");
    fs::write(&outside, "unchanged").unwrap();
    symlink(&outside, &journal).unwrap();
    assert!(adapter.begin(fixture.workspace.to_str().unwrap()).is_err());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "unchanged");
    fs::remove_file(&journal).unwrap();

    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let fifo = CString::new(journal.as_os_str().as_bytes()).unwrap();
    // SAFETY: the fixture path is a live NUL-terminated string.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(adapter.begin(fixture.workspace.to_str().unwrap()).is_err());
    fs::remove_file(&journal).unwrap();

    let mut transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
    let previous = fixture.root.join("previous-runtime");
    fs::rename(&fixture.runtime, &previous).unwrap();
    fs::create_dir(&fixture.runtime).unwrap();
    fs::set_permissions(&fixture.runtime, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(transaction.save(&fixture.record()).is_err());
    assert!(!fixture.runtime.join(unix::JOURNAL_NAME).exists());
}

#[test]
fn another_process_cannot_enter_until_the_transaction_is_dropped() {
    let fixture = Fixture::new("process-lease");
    let adapter = fixture.adapter();
    let transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
    for (locked, held) in [(true, Some(transaction)), (false, None)] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "adapter::outbound::filesystem::issue_creation_journal::tests::child_checks_creation_lease",
                "--nocapture",
            ])
            .env("AKRA_TEST_CREATE_LEASE_RUNTIME", &fixture.runtime)
            .env("AKRA_TEST_CREATE_LEASE_WORKSPACE", &fixture.workspace)
            .env("AKRA_TEST_CREATE_LEASE_EXPECT_LOCKED", if locked { "1" } else { "0" })
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("creation-lease-child-completed"));
        drop(held);
    }
}

#[test]
fn child_checks_creation_lease() {
    let Some(runtime) = std::env::var_os("AKRA_TEST_CREATE_LEASE_RUNTIME") else {
        return;
    };
    let workspace = std::env::var("AKRA_TEST_CREATE_LEASE_WORKSPACE").unwrap();
    let expected_locked = std::env::var("AKRA_TEST_CREATE_LEASE_EXPECT_LOCKED").unwrap() == "1";
    let result = unix::Transaction::open(PathBuf::from(runtime), workspace, Duration::ZERO);
    assert_eq!(result.is_err(), expected_locked);
    println!("creation-lease-child-completed");
}

mod provenance;

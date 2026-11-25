//! Real Windows storage and independent worker processes; the synthetic provider
//! stays alive when a creation worker is terminated after its remote side effect.
use super::*;
use crate::application::service::issue_publish::IssueApprovedPublishRequest;
use crate::application::service::issue_publish_journal::{
    IssueJournaledPublishOutcome, IssueJournaledPublishRequest,
};
use crate::domain::issue_creation::IssueCreationIntentRecord;
use crate::domain::issue_work::{IssuePublishApproval, IssueWriteApproval, IssueWriteOperation};
use std::mem::ManuallyDrop;
use std::process::{Child, Output};
use std::thread;

mod remote;
use remote::{Client, Server};

const WORKER: &str = "adapter::outbound::filesystem::issue_creation_journal::windows::tests::processes::creation_process_worker";
const ROOT: &str = "AKRA_TEST_CREATION_PROCESS_ROOT";
const ADDRESS: &str = "AKRA_TEST_CREATION_PROCESS_ADDRESS";
const ROLE: &str = "AKRA_TEST_CREATION_PROCESS_ROLE";
const WAIT: Duration = Duration::from_secs(45);
const CREATED_URL: &str = "https://github.com/acme/widget/issues/7";
const FIRST_ORIGINAL: &str = "synthetic-original-before-create-worker-termination";
const UPDATED_ORIGINAL: &str = "synthetic-newer-original-from-independent-update";

fn key() -> VaultRecordKey {
    VaultRecordKey {
        key_id: "synthetic-process-key".into(),
        key: VaultKey::new(vec![19; 32]),
    }
}

fn issue(fixture: &Fixture) -> IssueReference {
    let repository = fixture.repository();
    IssueReference {
        provider: repository.provider,
        server_id: repository.server_id,
        repository_id: repository.repository_id,
        project_scope: repository.project_scope,
        issue_id: "I_CREATED".into(),
    }
}

fn target(fixture: &Fixture) -> IssueCreationTarget {
    IssueCreationTarget {
        cwd: fixture.workspace(),
        owner: "acme".into(),
        repository: "widget".into(),
    }
}

fn request(fixture: &Fixture, record: &IssueCreationIntentRecord) -> IssueCreationApprovedRequest {
    let copy = copy();
    IssueCreationApprovedRequest {
        target: target(fixture),
        intent_id: record.intent_id.clone(),
        validated: validate_issue(&copy, &IssueRequirements::default()).unwrap(),
        approval: IssueCreationApproval {
            repository: record.repository.clone(),
            intent_id: record.intent_id.clone(),
            shared_copy_hash: IssueFieldHash::of_fields(&copy.title, &copy.body),
            policy_version: "process-policy".into(),
            template_version: None,
        },
        policy_version: "process-policy".into(),
        template_version: None,
        restorable: vec![IssueRestorableField {
            field: IssuePublishVaultField::Body,
            plaintext: VaultPlaintext::new(FIRST_ORIGINAL.into()),
        }],
        vault_key: Some(key()),
        shared_copy_only: false,
    }
}

fn services(
    fixture: &Fixture,
    address: &str,
) -> (IssueCreationService, Arc<IssueJournaledPublishService>) {
    let remote = Arc::new(Client::new(address, fixture));
    let vault = Arc::new(FilesystemEvidenceVaultStoreAdapter::new(
        fixture.0.join("vault"),
    ));
    let runtime = fixture.runtime();
    let publisher = Arc::new(IssueJournaledPublishService::new(
        remote.clone(),
        remote.clone(),
        Arc::new(IssuePublishIntentJournalAdapter::new(move |_| {
            Ok(runtime.clone())
        })),
        vault.clone(),
    ));
    let creator = IssueCreationService::new(
        remote.clone(),
        remote,
        Arc::new(fixture.adapter()),
        vault,
        publisher.clone(),
    );
    (creator, publisher)
}

fn current_original(fixture: &Fixture) -> String {
    let vault = FilesystemEvidenceVaultStoreAdapter::new(fixture.0.join("vault"));
    let reference = issue(fixture);
    let current = vault
        .load(VaultRecordLoadRequest {
            scope: VaultRecordScope::current_version(reference.clone()),
            key: key(),
        })
        .unwrap();
    vault
        .load(VaultRecordLoadRequest {
            scope: VaultRecordScope {
                issue: reference,
                field: VaultRecordField::Body,
                binding: VaultRecordBinding::ConfirmedVersion(current.expose().into()),
            },
            key: key(),
        })
        .unwrap()
        .expose()
        .into()
}

fn creation_record(fixture: &Fixture) -> IssueCreationIntentRecord {
    let records = fixture
        .adapter()
        .read_records(&fixture.workspace())
        .unwrap();
    assert_eq!(records.len(), 1);
    let encoded = serde_json::to_string(&records).unwrap();
    assert!(!encoded.contains(FIRST_ORIGINAL));
    assert!(!encoded.contains(UPDATED_ORIGINAL));
    records.into_iter().next().unwrap()
}

struct Worker(Option<Child>);
impl Worker {
    fn spawn(fixture: &Fixture, server: &Server, role: &str) -> Self {
        Self(Some(
            Command::new(std::env::current_exe().unwrap())
                .args(["--ignored", "--exact", WORKER, "--nocapture"])
                .env(ROOT, &fixture.0)
                .env(ADDRESS, server.address())
                .env(ROLE, role)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ))
    }

    fn kill_and_wait(&mut self) {
        let child = self.0.as_mut().unwrap();
        assert!(
            child.try_wait().unwrap().is_none(),
            "creation exited before the crash boundary"
        );
        child.kill().unwrap();
        let status = child.wait().unwrap();
        assert!(
            !status.success(),
            "the worker must be terminated, not return normally"
        );
        self.0 = None;
    }

    fn finish(&mut self) -> Output {
        let deadline = Instant::now() + WAIT;
        loop {
            let child = self.0.as_mut().unwrap();
            if child.try_wait().unwrap().is_some() {
                let output = self.0.take().unwrap().wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "worker failed:\n{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                return output;
            }
            assert!(
                Instant::now() < deadline,
                "worker did not finish within its deadline"
            );
            thread::sleep(Duration::from_millis(20));
        }
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

#[test]
#[ignore = "child process fixture"]
fn creation_process_worker() {
    let fixture = ManuallyDrop::new(Fixture(PathBuf::from(std::env::var_os(ROOT).unwrap())));
    let address = std::env::var(ADDRESS).unwrap();
    let role = std::env::var(ROLE).unwrap();
    let (creator, publisher) = services(&fixture, &address);
    match role.as_str() {
        "create" => {
            let opened = creator.open(&target(&fixture)).unwrap();
            let outcome = creator.publish(request(&fixture, &opened)).unwrap();
            let IssueCreationOutcome::Intent(record) = outcome else {
                panic!("creation was unexpectedly refused");
            };
            assert_eq!(record.state, IssueCreationIntentState::Confirmed);
            // In the race scenario the independent update already owns the original.
            assert!(!record.restore_link_finalized);
            assert_eq!(current_original(&fixture), UPDATED_ORIGINAL);
        }
        "recover" => {
            let opened = creator.open(&target(&fixture)).unwrap();
            assert!(matches!(
                opened.state,
                IssueCreationIntentState::Sent
                    | IssueCreationIntentState::NeedsConfirmation
                    | IssueCreationIntentState::Confirmed
            ));
            // Reopening and repeating the old approved click cannot POST again.
            let repeated = creator.publish(request(&fixture, &opened)).unwrap();
            assert!(matches!(repeated, IssueCreationOutcome::Intent(_)));
            creator
                .recheck(&target(&fixture), &opened.intent_id, Some(key()))
                .unwrap();
            let confirmed = creator
                .confirm(
                    &target(&fixture),
                    &opened.intent_id,
                    CREATED_URL,
                    Some(key()),
                )
                .unwrap();
            assert_eq!(confirmed.state, IssueCreationIntentState::Confirmed);
            assert!(confirmed.restore_link_finalized);
            assert_eq!(confirmed.created.unwrap().issue, issue(&fixture));
            assert_eq!(current_original(&fixture), FIRST_ORIGINAL);
        }
        "update" => {
            // Keep the public copy identical: its hash alone must not let an older
            // creation replace a later, different private original.
            let copy = copy();
            let reference = issue(&fixture);
            let hash = IssueFieldHash::of_fields(&copy.title, &copy.body);
            let outcome = publisher
                .publish(IssueJournaledPublishRequest {
                    publish: IssueApprovedPublishRequest {
                        issue: reference.clone(),
                        validated: validate_issue(&copy, &IssueRequirements::default()).unwrap(),
                        approval: IssuePublishApproval {
                            write: IssueWriteApproval {
                                issue: reference,
                                operation: IssueWriteOperation::Body,
                            },
                            shared_copy_hash: hash.clone(),
                            policy_version: "process-policy".into(),
                            template_version: None,
                        },
                        edit_basis: hash,
                        policy_version: "process-policy".into(),
                        template_version: None,
                    },
                    restorable: vec![IssueRestorableField {
                        field: IssuePublishVaultField::Body,
                        plaintext: VaultPlaintext::new(UPDATED_ORIGINAL.into()),
                    }],
                    vault_key: Some(key()),
                    shared_copy_only: false,
                })
                .unwrap();
            let IssueJournaledPublishOutcome::Confirmed(record) = outcome else {
                panic!("concurrent update was not confirmed: {outcome:?}");
            };
            publisher
                .finalize_restore_link(&fixture.workspace(), &record.intent_id, key())
                .unwrap();
            assert_eq!(current_original(&fixture), UPDATED_ORIGINAL);
        }
        "recheck-late" => {
            let opened = creator.open(&target(&fixture)).unwrap();
            let record = creator
                .recheck(&target(&fixture), &opened.intent_id, Some(key()))
                .unwrap();
            assert_eq!(record.state, IssueCreationIntentState::Confirmed);
            assert!(!record.restore_link_finalized);
            assert_eq!(current_original(&fixture), UPDATED_ORIGINAL);
        }
        _ => panic!("unknown creation worker role"),
    }
}

#[test]
fn remote_applied_creation_survives_worker_termination_without_duplicate_posts() {
    let fixture = Fixture::new();
    let server = Server::start();
    let mut creating = Worker::spawn(&fixture, &server, "create");
    server.wait_for_creation();
    // The provider applied the write, but the worker has not received its stable ID.
    let sent = creation_record(&fixture);
    assert_eq!(sent.state, IssueCreationIntentState::Sent);
    assert!(sent.created.is_none());
    assert!(
        fixture
            .adapter()
            .load(&fixture.source())
            .unwrap()
            .unresolved
    );
    creating.kill_and_wait();
    server.release_creation_response();

    // Both independent recovery clients share the durable reservation. Neither
    // gets to turn the lost response into a second remote creation.
    let mut first = Worker::spawn(&fixture, &server, "recover");
    let mut second = Worker::spawn(&fixture, &server, "recover");
    first.finish();
    second.finish();

    let confirmed = creation_record(&fixture);
    assert_eq!(confirmed.state, IssueCreationIntentState::Confirmed);
    assert!(confirmed.restore_link_finalized);
    assert_eq!(current_original(&fixture), FIRST_ORIGINAL);
    let provenance = fixture.adapter().load(&fixture.source()).unwrap();
    assert!(!provenance.unresolved);
    assert!(provenance.created_issue_ids.contains("I_CREATED"));
    assert_eq!(server.mutations(), (1, 0));
}

#[test]
fn concurrent_update_retains_its_original_when_creation_returns_late() {
    let fixture = Fixture::new();
    let server = Server::start();
    let mut creating = Worker::spawn(&fixture, &server, "create");
    server.wait_for_creation();
    let mut updating = Worker::spawn(&fixture, &server, "update");
    updating.finish();
    assert_eq!(current_original(&fixture), UPDATED_ORIGINAL);

    server.release_creation_response();
    creating.finish();
    let mut restarted = Worker::spawn(&fixture, &server, "recheck-late");
    restarted.finish();

    assert_eq!(current_original(&fixture), UPDATED_ORIGINAL);
    let creation = creation_record(&fixture);
    assert_eq!(creation.state, IssueCreationIntentState::Confirmed);
    assert!(!creation.restore_link_finalized);
    let runtime = fixture.runtime();
    let publication = IssuePublishIntentJournalAdapter::new(move |_| Ok(runtime.clone()));
    let records = publication.load(&fixture.workspace()).unwrap();
    assert_eq!(
        records.len(),
        1,
        "late adoption must not add a newer ownership sequence"
    );
    assert!(records[0].restore_link_finalized);
    assert_ne!(records[0].intent_id, creation.adoption_intent_id());
    assert_eq!(server.mutations(), (1, 1));
}

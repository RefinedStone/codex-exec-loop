use super::*;
use crate::application::port::outbound::issue_creation_provenance_port::IssueCreationProvenancePort;
use crate::domain::issue_automation::source::{IssueCreationProvenance, IssueSourceRepository};
use crate::domain::issue_creation::IssueCreatedLocation;
use crate::domain::issue_work::{IssueLocator, IssueReference};

fn repository(record: &IssueCreationIntentRecord) -> IssueSourceRepository {
    IssueSourceRepository {
        provider: record.repository.provider,
        server_id: record.repository.server_id.clone(),
        repository_id: record.repository.repository_id.clone(),
        project_scope: record.repository.project_scope.clone(),
    }
}

fn reference(record: &IssueCreationIntentRecord, id: &str) -> IssueReference {
    IssueReference {
        provider: record.repository.provider,
        server_id: record.repository.server_id.clone(),
        repository_id: record.repository.repository_id.clone(),
        issue_id: id.into(),
        project_scope: record.repository.project_scope.clone(),
    }
}

#[test]
fn provenance_empty_snapshot_does_not_create_a_journal_or_writer_lock() {
    let fixture = Fixture::new("provenance-empty");
    let adapter = fixture.adapter();
    let snapshot = adapter.load(&repository(&fixture.record())).unwrap();
    assert!(snapshot.created_issue_ids.is_empty());
    assert!(!snapshot.unresolved);
    assert_eq!(fs::read_dir(&fixture.runtime).unwrap().count(), 0);
}

#[test]
fn provenance_reads_durable_sent_while_the_writer_keeps_its_remote_request_lease() {
    let fixture = Fixture::new("provenance-sent");
    let adapter = fixture.adapter();
    let mut record = fixture.record();
    approve(&mut record);
    record.state = IssueCreationIntentState::Sent;
    let mut transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
    transaction.save(&record).unwrap();
    let before = fs::read(fixture.runtime.join("issue-create-intents.jsonl")).unwrap();

    // 트랜잭션이 살아 있는 동안에도 snapshot 읽기는 lease를 기다리지 않는다.
    let snapshot = adapter.load(&repository(&record)).unwrap();
    assert!(snapshot.unresolved);
    assert_eq!(
        snapshot.classify(&reference(&record, "I_unknown")),
        IssueCreationProvenance::Unresolved
    );
    assert_eq!(
        fs::read(fixture.runtime.join("issue-create-intents.jsonl")).unwrap(),
        before
    );
    drop(transaction);
}

#[test]
fn provenance_uses_exact_created_identity_and_ignores_other_repository_attempts() {
    let fixture = Fixture::new("provenance-exact");
    let adapter = fixture.adapter();
    let mut record = fixture.record();
    approve(&mut record);
    record.state = IssueCreationIntentState::NeedsConfirmation;
    record.created = Some(IssueCreatedLocation {
        issue: reference(&record, "I_created"),
        locator: IssueLocator {
            owner: "owner".into(),
            repository: "repo".into(),
            number: 17,
        },
        url: "https://github.com/owner/repo/issues/17".into(),
    });
    let mut unrelated = fixture.record();
    unrelated.intent_id = "other-repository".into();
    unrelated.repository.repository_id = "R_other".into();
    approve(&mut unrelated);
    unrelated.state = IssueCreationIntentState::Sent;
    let mut transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
    transaction.save(&record).unwrap();
    transaction.save(&unrelated).unwrap();
    drop(transaction);

    let snapshot = adapter.load(&repository(&record)).unwrap();
    assert!(!snapshot.unresolved);
    assert_eq!(
        snapshot.classify(&reference(&record, "I_created")),
        IssueCreationProvenance::AkraCreated
    );
    assert_eq!(
        snapshot.classify(&reference(&record, "I_other")),
        IssueCreationProvenance::Unattributed
    );
    let mut other_scope = reference(&record, "I_created");
    other_scope.project_scope.workspace_dir = "/other/workspace".into();
    assert_eq!(
        snapshot.classify(&other_scope),
        IssueCreationProvenance::Unresolved
    );
}

#[test]
fn provenance_rejects_corrupt_and_linked_journals_without_exposing_the_content() {
    let fixture = Fixture::new("provenance-corrupt");
    let adapter = fixture.adapter();
    let record = fixture.record();
    let path = fixture.runtime.join("issue-create-intents.jsonl");
    secure_fs::write_file_atomic(
        &fixture.runtime,
        Path::new("issue-create-intents.jsonl"),
        b"private-raw-draft",
    )
    .unwrap();
    let error = adapter.load(&repository(&record)).unwrap_err().to_string();
    assert!(!error.contains("private-raw-draft"));
    fs::remove_file(&path).unwrap();
    let outside = fixture.root.join("outside");
    fs::write(&outside, "private-raw-draft").unwrap();
    symlink(&outside, &path).unwrap();
    assert!(adapter.load(&repository(&record)).is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "private-raw-draft");
}

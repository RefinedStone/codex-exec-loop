use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use super::*;
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::application::port::outbound::issue_snapshot_port::{
    IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest, IssueSnapshotResponse,
};
use crate::application::port::outbound::issue_update_port::{
    IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
};
use crate::application::service::issue_publish_journal::IssueJournaledPublishService;
use crate::domain::issue_publish_intent::{
    IssuePublishIntentRecord, IssuePublishIntentState, IssuePublishRestoreBindingAttempt,
    IssuePublishVaultField,
};
use crate::domain::issue_work::IssueFieldHash;

const PENDING_BODY: &str = "synthetic-published-original";

struct NoProviderCalls;

impl IssueSnapshotPort for NoProviderCalls {
    fn resolve(&self, _: IssueSnapshotResolveRequest) -> anyhow::Result<IssueSnapshotResponse> {
        panic!("finalizing a stored publication must not resolve an issue")
    }

    fn read(&self, _: IssueSnapshotReadRequest) -> anyhow::Result<IssueSnapshotResponse> {
        panic!("finalizing a stored publication must not read the provider")
    }
}

impl IssueUpdatePort for NoProviderCalls {
    fn update_title_and_body(&self, _: IssueUpdateRequest) -> anyhow::Result<IssueUpdateResponse> {
        panic!("finalizing a stored publication must not repeat its external write")
    }
}

#[derive(Default)]
struct Journal {
    records: Mutex<Vec<IssuePublishIntentRecord>>,
    fail_completion_once: AtomicBool,
}

impl IssuePublishIntentJournalPort for Journal {
    fn save(&self, workspace: &str, record: &IssuePublishIntentRecord) -> anyhow::Result<()> {
        assert_eq!(workspace, record.issue.project_scope.workspace_dir);
        if record.restore_link_finalized && self.fail_completion_once.swap(false, Ordering::SeqCst)
        {
            anyhow::bail!("synthetic completion acknowledgement failure")
        }
        let mut records = self.records.lock().unwrap();
        records.retain(|existing| existing.intent_id != record.intent_id);
        records.push(record.clone());
        Ok(())
    }

    fn load(&self, _: &str) -> anyhow::Result<Vec<IssuePublishIntentRecord>> {
        Ok(self.records.lock().unwrap().clone())
    }
}

fn record(expected: Option<&VaultBindingToken>) -> IssuePublishIntentRecord {
    let approved = IssueFieldHash::of_fields("synthetic title", "synthetic shared body");
    IssuePublishIntentRecord {
        intent_id: "synthetic-delayed-publication".to_string(),
        issue: issue(),
        previous_hash: IssueFieldHash::of_fields("old title", "old body"),
        approved_hash: approved.clone(),
        pending_vault_fields: vec![IssuePublishVaultField::Body],
        state: IssuePublishIntentState::Confirmed,
        observed_hash: Some(approved),
        restore_link_finalized: false,
        restore_binding_attempt: expected.map(|token| IssuePublishRestoreBindingAttempt {
            expected_ciphertext_sha256: token.ciphertext_sha256(),
        }),
        write_response_mismatch: false,
        write_attempted: Some(true),
        sequence: 1,
    }
}

fn publication(
    fixture: &Fixture,
    journal: Arc<Journal>,
    record: &IssuePublishIntentRecord,
) -> IssueJournaledPublishService {
    let store = fixture.store();
    store
        .save(VaultRecordSaveRequest {
            scope: VaultRecordScope {
                issue: record.issue.clone(),
                field: VaultRecordField::Body,
                binding: VaultRecordBinding::PublishIntent(record.intent_id.clone()),
            },
            key: key(),
            plaintext: VaultPlaintext::new(PENDING_BODY.to_string()),
        })
        .unwrap();
    journal
        .save(&record.issue.project_scope.workspace_dir, record)
        .unwrap();
    let remote = Arc::new(NoProviderCalls);
    IssueJournaledPublishService::new(remote.clone(), remote, journal, Arc::new(fixture.store()))
}

#[test]
fn late_publication_finalize_preserves_the_newer_on_disk_binding() {
    let fixture = Fixture::new("late-publication");
    let store = fixture.store();
    let expected = seed_legacy(&store);
    let record = record(Some(&expected));
    let journal = Arc::new(Journal::default());
    let service = publication(&fixture, journal.clone(), &record);
    let mut later = request(expected, "later-reconnect", desired_fields("later"));
    later.base_version = record.confirmed_version();
    assert!(matches!(
        store
            .begin_binding(load_request())
            .unwrap()
            .commit(later)
            .unwrap(),
        VaultBindingCommitResult::Committed(_)
    ));
    let before = all_records(&fixture.root);
    let error = service
        .finalize_restore_link(
            &record.issue.project_scope.workspace_dir,
            &record.intent_id,
            key(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("restore binding changed"));
    assert_eq!(all_records(&fixture.root), before);
    assert_eq!(current_fields(&store), desired_fields("later"));
    assert!(
        !journal
            .records
            .lock()
            .unwrap()
            .first()
            .unwrap()
            .restore_link_finalized
    );
}

#[test]
fn lost_publication_completion_ack_reuses_the_installed_generation_after_reopen() {
    let fixture = Fixture::new("publication-retry");
    let store = fixture.store();
    let expected = seed_legacy(&store);
    let record = record(Some(&expected));
    let journal = Arc::new(Journal::default());
    let service = publication(&fixture, journal.clone(), &record);
    journal.fail_completion_once.store(true, Ordering::SeqCst);
    assert!(
        service
            .finalize_restore_link(
                &record.issue.project_scope.workspace_dir,
                &record.intent_id,
                key()
            )
            .is_err()
    );
    assert_eq!(current_fields(&store), fields(None, Some(PENDING_BODY)));
    let installed = all_records(&fixture.root);
    assert!(
        !journal
            .records
            .lock()
            .unwrap()
            .first()
            .unwrap()
            .restore_link_finalized
    );

    let remote = Arc::new(NoProviderCalls);
    let reopened = IssueJournaledPublishService::new(
        remote.clone(),
        remote,
        journal.clone(),
        Arc::new(fixture.store()),
    );
    let receipt = reopened
        .finalize_restore_link(
            &record.issue.project_scope.workspace_dir,
            &record.intent_id,
            key(),
        )
        .unwrap();
    assert!(receipt.restore_link_finalized);
    assert_eq!(
        receipt.restore_binding_attempt,
        record.restore_binding_attempt
    );
    assert_eq!(all_records(&fixture.root), installed);
}

#[test]
fn legacy_publication_without_an_expected_token_only_acknowledges_exact_on_disk_fields() {
    for matching in [true, false] {
        let fixture = Fixture::new("legacy-publication");
        let store = fixture.store();
        let record = record(None);
        let journal = Arc::new(Journal::default());
        let service = publication(&fixture, journal, &record);
        let saved = fields(
            None,
            Some(if matching {
                PENDING_BODY
            } else {
                "synthetic-unrelated"
            }),
        );
        seed_fields(&store, &record.confirmed_version(), &saved);
        store
            .seed_record_for_test(VaultRecordSaveRequest {
                scope: load_request().scope,
                key: key(),
                plaintext: VaultPlaintext::new(record.confirmed_version()),
            })
            .unwrap();
        let before = all_records(&fixture.root);
        let result = service.finalize_restore_link(
            &record.issue.project_scope.workspace_dir,
            &record.intent_id,
            key(),
        );
        assert_eq!(result.is_ok(), matching);
        if let Ok(receipt) = result {
            assert!(receipt.restore_link_finalized);
            assert!(receipt.restore_binding_attempt.is_none());
        }
        assert_eq!(all_records(&fixture.root), before);
        assert_eq!(current_fields(&store), saved);
    }
}

//! Creation approvals remain authoritative without a successful Vault adoption.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Result;

use super::support::{
    CreateBehavior, Fixture, RAW_FIRST, WORKSPACE, intent, issue, location, request, shared_copy,
    snapshot, target,
};
use crate::application::port::inbound::issue_handoff_port::{
    IssueHandoffPort, IssueHandoffRequest,
};
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::service::issue_handoff::IssueHandoffService;
use crate::application::service::issue_publish::IssueApprovedPublishRequest;
use crate::application::service::issue_publish_journal::IssueJournaledPublishRequest;
use crate::domain::issue_creation::IssueCreationIntentState;
use crate::domain::issue_publish_intent::IssuePublishIntentState;
use crate::domain::issue_validation::{IssueRequirements, validate_issue};
use crate::domain::issue_work::{
    IssueFieldHash, IssuePublishApproval, IssueSharedCopy, IssueWriteApproval, IssueWriteOperation,
};

struct PublicPolicy;
impl IssuePolicySourcePort for PublicPolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(None)
    }

    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

fn handoff(fixture: &Fixture) -> IssueHandoffService {
    IssueHandoffService::new(fixture.remote.clone(), Arc::new(PublicPolicy))
        .with_publish_journal(fixture.publish_journal.clone(), |cwd| Ok(cwd.to_string()))
        .with_creation_journal(fixture.journal.clone())
}

fn handoff_request() -> IssueHandoffRequest {
    IssueHandoffRequest {
        locator: location().locator,
        workspace_dir: Some(WORKSPACE.to_string()),
        approved: None,
    }
}

#[test]
fn creation_handoff_checks_shared_copy_only_and_pending_vault_adoption() {
    for pending_adoption in [false, true] {
        let fixture = Fixture::new(CreateBehavior::Answer);
        let opened = fixture.service().open(&target()).unwrap();
        fixture
            .publish_journal
            .fail_save
            .store(pending_adoption, Ordering::SeqCst);
        let mut creation = request(&opened.intent_id, pending_adoption.then_some(RAW_FIRST));
        creation.shared_copy_only = !pending_adoption;
        let created = intent(fixture.service().publish(creation).unwrap());
        assert_eq!(created.state, IssueCreationIntentState::Confirmed);
        assert!(fixture.publish_journal.records.lock().unwrap().is_empty());
        assert_eq!(created.restore_link_finalized, !pending_adoption);

        // Fresh hand-off services must recover the creation's approval without Vault access.
        assert_eq!(
            handoff(&fixture).handoff_copy(handoff_request()).unwrap(),
            shared_copy()
        );
        let private = "/home/synthetic-customer/private-report";
        let edited = IssueSharedCopy {
            body: private.to_string(),
            ..shared_copy()
        };
        fixture
            .remote
            .snapshots
            .lock()
            .unwrap()
            .insert(7, snapshot(issue(), edited));
        let error = handoff(&fixture)
            .handoff_copy(handoff_request())
            .unwrap_err();
        assert!(!format!("{error:#}").contains(private));
        assert_eq!(fixture.creates(), 1);
        assert_eq!(fixture.remote.updates.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn creation_handoff_holds_unknown_or_inconsistent_creation_approval() {
    let fixture = Fixture::new(CreateBehavior::ApplyAndLoseAnswer);
    let opened = fixture.service().open(&target()).unwrap();
    let pending = intent(
        fixture
            .service()
            .publish(request(&opened.intent_id, None))
            .unwrap(),
    );
    assert_eq!(pending.state, IssueCreationIntentState::NeedsConfirmation);
    assert!(pending.created.is_none());
    assert!(handoff(&fixture).handoff_copy(handoff_request()).is_err());
    assert_eq!(fixture.creates(), 1);

    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    let confirmed = intent(
        fixture
            .service()
            .publish(request(&opened.intent_id, None))
            .unwrap(),
    );
    for state in [
        IssueCreationIntentState::Sent,
        IssueCreationIntentState::NeedsConfirmation,
    ] {
        fixture.journal.records.lock().unwrap()[0].state = state;
        assert!(handoff(&fixture).handoff_copy(handoff_request()).is_err());
    }
    let mutations: [fn(&mut crate::domain::issue_creation::IssueCreationIntentRecord); 3] = [
        |record| record.observed_hash = None,
        |record: &mut crate::domain::issue_creation::IssueCreationIntentRecord| {
            record.write_response_mismatch = true
        },
        |record: &mut crate::domain::issue_creation::IssueCreationIntentRecord| {
            record.approved.as_mut().unwrap().approval.intent_id = "different-intent".to_string();
        },
    ];
    for mutate in mutations {
        let mut changed = confirmed.clone();
        mutate(&mut changed);
        *fixture.journal.records.lock().unwrap() = vec![changed];
        assert!(handoff(&fixture).handoff_copy(handoff_request()).is_err());
    }
    *fixture.journal.records.lock().unwrap() = vec![confirmed.clone(), confirmed];
    assert!(handoff(&fixture).handoff_copy(handoff_request()).is_err());
}

#[test]
fn creation_handoff_uses_a_later_confirmed_update_instead_of_the_original_creation() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture
        .service()
        .publish(request(&opened.intent_id, None))
        .unwrap();
    fixture.remote.allow_updates.store(true, Ordering::SeqCst);
    let updated = IssueSharedCopy {
        body: "A later separately approved shared body".to_string(),
        ..shared_copy()
    };
    let approved_hash = IssueFieldHash::of_fields(&updated.title, &updated.body);
    fixture
        .publisher()
        .publish(IssueJournaledPublishRequest {
            publish: IssueApprovedPublishRequest {
                issue: issue(),
                validated: validate_issue(&updated, &IssueRequirements::default()).unwrap(),
                approval: IssuePublishApproval {
                    write: IssueWriteApproval {
                        issue: issue(),
                        operation: IssueWriteOperation::Body,
                    },
                    shared_copy_hash: approved_hash,
                    policy_version: "policy-v1".to_string(),
                    template_version: None,
                },
                edit_basis: IssueFieldHash::of_fields(&shared_copy().title, &shared_copy().body),
                policy_version: "policy-v1".to_string(),
                template_version: None,
            },
            restorable: Vec::new(),
            vault_key: None,
            shared_copy_only: true,
        })
        .unwrap();
    assert_eq!(
        fixture.publish_journal.records.lock().unwrap()[0].state,
        IssuePublishIntentState::Confirmed
    );
    assert_eq!(
        handoff(&fixture).handoff_copy(handoff_request()).unwrap(),
        updated
    );
}

#[test]
fn creation_handoff_does_not_fall_back_when_the_snapshot_is_unavailable() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture
        .service()
        .publish(request(&opened.intent_id, None))
        .unwrap();
    let _unavailable = fixture.journal.records.lock().unwrap();
    let error = handoff(&fixture)
        .handoff_copy(handoff_request())
        .unwrap_err();
    assert!(error.to_string().contains("creation state is unavailable"));

    // The explicit non-project manual-read contract remains independent of project approvals.
    let unscoped = IssueHandoffRequest {
        workspace_dir: None,
        ..handoff_request()
    };
    assert_eq!(
        handoff(&fixture).handoff_copy(unscoped).unwrap(),
        shared_copy()
    );
    assert_eq!(fixture.creates(), 1);
}

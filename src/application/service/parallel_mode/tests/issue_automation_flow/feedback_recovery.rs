use super::support::{FlowFixture, FlowServices};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::application::service::issue_collection::IssueCollectionCancellation;
use crate::domain::issue_automation::feedback::{
    IssueFeedbackAction, IssueFeedbackEvidence, IssueFeedbackIntentRecord,
    IssueFeedbackIntentState, IssueFeedbackOperation,
};
use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;
use std::sync::Arc;

fn runtime_directory(workspace: &str) -> PathBuf {
    PathBuf::from(
        SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)
            .unwrap()
            .runtime_dir,
    )
}

/// Reopen the on-disk journal for evidence; no fixture inserts or updates a feedback row.
fn persisted_feedback_intents(services: &FlowServices) -> Vec<(String, IssueFeedbackIntentRecord)> {
    let path = runtime_directory(&services.workspace).join("issue-automation.db");
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut statement = connection.prepare(
        "SELECT status, record FROM issue_feedback_intents WHERE owner_workspace=?1 ORDER BY intent_id",
    ).unwrap();
    let rows: Vec<(String, String)> = statement
        .query_map([&services.workspace], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    rows.into_iter()
        .map(|(status, encoded)| {
            let record: IssueFeedbackIntentRecord = serde_json::from_str(&encoded).unwrap();
            record.validate().unwrap();
            assert_eq!(record.intent.issue, services.issue);
            (status, record)
        })
        .collect()
}

#[test]
fn issue_automation_flow_feedback_reconciles_lost_comment_ack_after_sqlite_restart_without_resending()
 {
    let fixture = FlowFixture::new("issue-flow-comment-ack-loss");
    let services = fixture.services();
    services.collect_and_triage().unwrap();
    services.tick(1).unwrap();
    services.approve_start().unwrap();
    assert!(fixture.provider.writes().is_empty());

    fixture.provider.lose_next_comment_ack();
    fixture.provider.fail_next_comment_readback();
    services.tick(2).unwrap();
    assert_eq!(fixture.provider.create_count(), 1);
    assert_eq!(fixture.provider.writes().len(), 1);
    assert_eq!(fixture.provider.reconcile_count(), 1);
    let comments = fixture.provider.comments();
    assert_eq!(
        comments.len(),
        1,
        "the remote create succeeded before its ACK was lost"
    );
    let comment = comments[0].clone();
    let rows = persisted_feedback_intents(&services);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "needs_confirmation");
    let original = rows[0].1.clone();
    assert_eq!(original.state, IssueFeedbackIntentState::NeedsConfirmation);
    assert!(original.receipt.is_none());
    assert!(original.acknowledged_comment_id.is_none());
    assert!(!original.contradictory_response);
    assert!(matches!(
        original.intent.operation,
        IssueFeedbackOperation::CreateComment { .. }
    ));
    assert_eq!(fixture.provider.writes(), vec![original.intent.clone()]);
    assert_eq!(comment.issue, original.intent.issue);
    assert_eq!(comment.intent_id, original.intent.intent_id);
    assert_eq!(comment.actor_id, original.intent.actor_id);
    assert_eq!(
        services.feedback.unresolved(&fixture.issue).unwrap(),
        vec![original.clone()]
    );
    let uncertain = services.feedback.snapshot(&fixture.issue).unwrap().unwrap();
    assert_eq!(uncertain.publication_attempts, 1);
    assert!(uncertain.state.comment_owner.is_none());
    assert!(services.feedback.pending_events().unwrap().is_empty());

    // Another status sync cannot replace or resend the unresolved create.
    services
        .feedback
        .sync_issue(&fixture.issue, 3, &IssueCollectionCancellation::new())
        .unwrap();
    assert_eq!(fixture.provider.writes().len(), 1);
    assert_eq!(fixture.provider.reconcile_count(), 1);
    assert_eq!(persisted_feedback_intents(&services), rows);

    // Comments live in SQLite. The real filesystem journal supplies the shared publication lease,
    // while its separate issue-body publication records remain unchanged.
    let publication_before = services.publication.load(&services.workspace).unwrap();
    assert!(publication_before.is_empty());
    let runtime = runtime_directory(&services.workspace);
    assert!(runtime.join("issue-publish-intents.lock").is_file());
    let old_feedback = Arc::downgrade(&services.feedback);
    let old_store = Arc::downgrade(&services.store);
    let old_publication = Arc::downgrade(&services.publication);
    let workspace = services.workspace.clone();
    drop(services);
    assert!(old_feedback.upgrade().is_none());
    assert!(old_store.upgrade().is_none());
    assert!(old_publication.upgrade().is_none());

    let restarted = fixture.services();
    assert_eq!(restarted.workspace, workspace);
    assert_eq!(runtime_directory(&restarted.workspace), runtime);
    assert_eq!(persisted_feedback_intents(&restarted), rows);
    assert_eq!(
        restarted.feedback.unresolved(&fixture.issue).unwrap(),
        vec![original.clone()]
    );
    let writes_before_recovery = fixture.provider.writes();
    restarted
        .feedback
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    restarted
        .feedback
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    assert_eq!(
        fixture.provider.reconcile_count(),
        2,
        "only the pending intent needs a read"
    );
    assert_eq!(
        fixture.provider.writes(),
        writes_before_recovery,
        "restart recovery performs no mutation"
    );
    assert_eq!(fixture.provider.create_count(), 1);
    assert_eq!(fixture.provider.comments(), comments);

    let confirmed_rows = persisted_feedback_intents(&restarted);
    assert_eq!(
        confirmed_rows.len(),
        1,
        "recovery updates the same durable intent"
    );
    assert_eq!(confirmed_rows[0].0, "confirmed");
    let confirmed = &confirmed_rows[0].1;
    assert_eq!(confirmed.state, IssueFeedbackIntentState::Confirmed);
    assert_eq!(confirmed.intent, original.intent);
    assert!(confirmed.revision > original.revision);
    let receipt = confirmed
        .receipt
        .as_ref()
        .expect("readback confirmation is durable");
    receipt.validate_for(&original.intent).unwrap();
    assert_eq!(receipt.intent_id, comment.intent_id);
    assert_eq!(
        receipt.evidence,
        IssueFeedbackEvidence::Comment {
            comment_id: comment.comment_id.clone(),
            body_sha256: comment.body_sha256.clone(),
        }
    );
    assert!(
        restarted
            .feedback
            .unresolved(&fixture.issue)
            .unwrap()
            .is_empty()
    );
    let snapshot = restarted
        .feedback
        .snapshot(&fixture.issue)
        .unwrap()
        .unwrap();
    assert_eq!(
        snapshot.publication_attempts, 1,
        "readback does not consume another publication attempt"
    );
    let owner = snapshot.state.comment_owner.unwrap();
    assert_eq!(owner.comment_id, comment.comment_id);
    assert_eq!(owner.latest_intent_id, original.intent.intent_id);
    assert_eq!(owner.actor_id, comment.actor_id);
    assert_eq!(owner.body_sha256, comment.body_sha256);
    let events = restarted.feedback.pending_events().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].action, IssueFeedbackAction::CommentCreated);
    assert_eq!(events[0].intent_id, original.intent.intent_id);
    assert_eq!(&events[0].receipt, receipt);
    assert_eq!(
        restarted.publication.load(&restarted.workspace).unwrap(),
        publication_before
    );
}

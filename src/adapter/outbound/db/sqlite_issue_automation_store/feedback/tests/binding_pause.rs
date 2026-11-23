use super::*;
use crate::domain::issue_automation::binding_pause::*;

fn command(
    adapter: &SqliteIssueAutomationStoreAdapter,
    issue: &IssueReference,
    id: &str,
    paused: bool,
) -> IssueBindingPauseIntent {
    let snapshot = current(adapter, issue);
    let workspace = &issue.project_scope.workspace_dir;
    IssueBindingPauseIntent {
        command_id: id.into(),
        binding_id: snapshot.item.binding_id.clone(),
        paused,
        expected_revision: snapshot.binding_pause.revision,
        resume_basis: (!paused).then(|| IssueBindingResumeBasis {
            binding_revision: adapter
                .get_binding(workspace, &snapshot.item.binding_id)
                .unwrap()
                .unwrap()
                .revision,
            autonomy_revision: snapshot.autonomy.revision,
            policy_version: snapshot
                .item
                .triage
                .as_ref()
                .unwrap()
                .assessment
                .basis()
                .automation_policy_version
                .clone(),
            effective_level: snapshot
                .item
                .triage
                .as_ref()
                .unwrap()
                .assessment
                .effective_mode(),
        }),
    }
}

#[test]
fn feedback_recorded_intent_cannot_be_sent_after_pause_resume_even_with_a_fresh_fence() {
    let fixture = Fixture::new("feedback-binding-aba");
    let issue = seed(&fixture, "I_feedback");
    let adapter = fixture.adapter();
    save_desired(&adapter, &issue, "상태: 준비됨");
    let intent = comment_intent(&current(&adapter, &issue), "recorded-before-pause");
    let record = recorded(&adapter, &intent);
    let pause = command(&adapter, &issue, "pause", true);
    adapter
        .begin_issue_binding_pause(fixture.cwd(), &pause)
        .unwrap();
    let pending = current(&adapter, &issue);
    assert!(!pending.binding_pause.allows_automation());
    assert_eq!(pending.fence().binding_run_revision, 1);
    assert!(matches!(
        adapter
            .claim_sent(&issue, &intent.intent_id, record.revision, pending.fence())
            .unwrap(),
        IssueFeedbackClaimResult::Conflict
    ));
    adapter
        .finish_issue_binding_pause(fixture.cwd(), &pause, IssueBindingPauseCompletion::Apply)
        .unwrap();
    let resume = command(&adapter, &issue, "resume", false);
    adapter
        .begin_issue_binding_pause(fixture.cwd(), &resume)
        .unwrap();
    adapter
        .finish_issue_binding_pause(fixture.cwd(), &resume, IssueBindingPauseCompletion::Apply)
        .unwrap();
    let fresh = current(&adapter, &issue);
    assert!(fresh.binding_pause.allows_automation());
    assert_eq!(fresh.fence().binding_run_revision, 2);
    assert!(matches!(
        adapter
            .claim_sent(&issue, &intent.intent_id, record.revision, fresh.fence())
            .unwrap(),
        IssueFeedbackClaimResult::Conflict
    ));
    adapter
        .complete(
            &issue,
            &intent.intent_id,
            record.revision,
            &IssueFeedbackCompletion::Abandoned,
        )
        .unwrap();
    let next = comment_intent(&current(&adapter, &issue), "after-resume");
    let next = recorded(&adapter, &next);
    let encoded: String = Connection::open(fixture.database())
        .unwrap()
        .query_row(
            "SELECT recorded_fence FROM issue_feedback_intents WHERE intent_id=?1",
            [&next.intent.intent_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<serde_json::Value>>(&encoded)
            .unwrap()
            .len(),
        9
    );
    let reopened = fixture.adapter();
    assert_eq!(sent(&reopened, &next).state, IssueFeedbackIntentState::Sent);
}

#[test]
fn paused_binding_blocks_new_record_and_first_grant_without_hiding_prior_facts() {
    let fixture = Fixture::new("feedback-binding-closed");
    let issue = seed(&fixture, "I_feedback");
    let adapter = fixture.adapter();
    let work = existing_work(&fixture, &issue);
    refresh_assessment(&fixture, &issue);
    save_desired(&adapter, &issue, "상태: 준비됨");
    let pause = command(&adapter, &issue, "pause", true);
    adapter
        .begin_issue_binding_pause(fixture.cwd(), &pause)
        .unwrap();
    let fresh = current(&adapter, &issue);
    let intent = comment_intent(&fresh, "while-pausing");
    assert!(matches!(
        adapter.record(fresh.fence(), &intent).unwrap(),
        IssueFeedbackRecordResult::Conflict
    ));
    assert_eq!(
        adapter
            .mark_first_grant(&issue, &work.order.work_order_id, fresh.fence())
            .unwrap(),
        IssueFeedbackGrantResult::Conflict
    );
    assert_eq!(count(&fixture, "issue_feedback_intents"), 0);
    assert_eq!(count(&fixture, "issue_feedback_grant_entries"), 0);
}

#[test]
fn a_sent_feedback_result_can_be_settled_while_the_binding_is_closed() {
    let fixture = Fixture::new("feedback-binding-settle");
    let issue = seed(&fixture, "I_feedback");
    let adapter = fixture.adapter();
    save_desired(&adapter, &issue, "상태: 실행 중");
    let intent = comment_intent(&current(&adapter, &issue), "sent-before-pause");
    let record = sent(&adapter, &recorded(&adapter, &intent));
    let pause = command(&adapter, &issue, "pause", true);
    adapter
        .begin_issue_binding_pause(fixture.cwd(), &pause)
        .unwrap();
    let receipt = comment_receipt(&record, "IC_owned");
    let completed = confirmed(&adapter, &record, receipt.clone());
    assert_eq!(completed.receipt, Some(receipt));
    assert_eq!(current(&adapter, &issue).publication_attempts, 1);
    assert_eq!(adapter.pending_events(fixture.cwd()).unwrap().len(), 1);
    assert!(!current(&adapter, &issue).binding_pause.allows_automation());
}

#[test]
fn legacy_eight_element_feedback_fences_are_preserved_by_v14_migration_and_can_finish() {
    let fixture = Fixture::new("feedback-binding-legacy");
    let issue = seed(&fixture, "I_feedback");
    let adapter = fixture.adapter();
    save_desired(&adapter, &issue, "상태: 준비됨");
    let intent = comment_intent(&current(&adapter, &issue), "legacy-recorded");
    let record = recorded(&adapter, &intent);
    let connection = Connection::open(fixture.database()).unwrap();
    let before: String = connection
        .query_row(
            "SELECT recorded_fence FROM issue_feedback_intents",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<serde_json::Value>>(&before)
            .unwrap()
            .len(),
        8
    );
    super::super::super::tests::make_pre_binding_pause_schema(&connection);
    connection.pragma_update(None, "user_version", 13).unwrap();
    drop(connection);
    let reopened = fixture.adapter();
    assert_eq!(
        sent(&reopened, &record).state,
        IssueFeedbackIntentState::Sent
    );
    let after: String = Connection::open(fixture.database())
        .unwrap()
        .query_row(
            "SELECT recorded_fence FROM issue_feedback_intents",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, before);
}

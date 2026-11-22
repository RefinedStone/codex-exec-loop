use super::*;
use crate::domain::issue_automation::collection::IssueCollectionMode;
use std::sync::{Arc, Barrier};

#[test]
fn feedback_record_and_send_claim_each_have_one_concurrent_winner() {
    let fixture = Arc::new(Fixture::new("feedback-concurrent-claim"));
    let issue = seed(&fixture, "I_1");
    let adapter = fixture.adapter();
    save_desired(&adapter, &issue, "상태: 준비됨");
    let snapshot = current(&adapter, &issue);
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = ["concurrent-a", "concurrent-b"]
        .into_iter()
        .map(|id| {
            let fixture = fixture.clone();
            let barrier = barrier.clone();
            let intent = comment_intent(&snapshot, id);
            let fence = snapshot.fence();
            std::thread::spawn(move || {
                barrier.wait();
                fixture.adapter().record(fence, &intent).unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, IssueFeedbackRecordResult::Created(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, IssueFeedbackRecordResult::Blocked))
            .count(),
        1
    );
    let record = results
        .into_iter()
        .find_map(|result| match result {
            IssueFeedbackRecordResult::Created(record) => Some(record),
            _ => None,
        })
        .unwrap();
    assert_eq!(count(&fixture, "issue_feedback_intents"), 1);

    let barrier = Arc::new(Barrier::new(2));
    let fence = current(&adapter, &issue).fence();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let fixture = fixture.clone();
            let barrier = barrier.clone();
            let record = record.clone();
            std::thread::spawn(move || {
                barrier.wait();
                fixture
                    .adapter()
                    .claim_sent(
                        &record.intent.issue,
                        &record.intent.intent_id,
                        record.revision,
                        fence,
                    )
                    .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, IssueFeedbackClaimResult::Claimed(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, IssueFeedbackClaimResult::Existing(_)))
            .count(),
        1
    );
    assert_eq!(
        adapter.pending(fixture.cwd()).unwrap()[0].state,
        IssueFeedbackIntentState::Sent
    );
    assert_eq!(current(&adapter, &issue).state.desired_revision, 1);
}

#[test]
fn feedback_recorded_fence_rejects_a_fresh_snapshot_after_authority_aba() {
    let fixture = Fixture::new("feedback-recorded-fence");
    let issue = seed(&fixture, "I_1");
    let adapter = fixture.adapter();
    save_desired(&adapter, &issue, "상태: 준비됨");
    let intent = comment_intent(&current(&adapter, &issue), "before-control-change");
    let record = recorded(&adapter, &intent);

    // Restore the same visible flags, but retain the two committed revisions.
    let mut tracked = adapter.get_issue(fixture.cwd(), &issue).unwrap().unwrap();
    tracked.set_flag(IssueAutomationFlag::Held, true);
    let mut tracked = super::super::super::tests::save_changed(&adapter, fixture.cwd(), &tracked);
    tracked.set_flag(IssueAutomationFlag::Held, false);
    super::super::super::tests::save_changed(&adapter, fixture.cwd(), &tracked);
    let fresh = current(&adapter, &issue);
    assert!(!fresh.item.tracked.as_ref().unwrap().flags.held);
    assert!(matches!(
        adapter
            .claim_sent(&issue, &intent.intent_id, record.revision, fresh.fence())
            .unwrap(),
        IssueFeedbackClaimResult::Conflict
    ));
    assert_eq!(
        adapter.pending(fixture.cwd()).unwrap(),
        vec![record.clone()]
    );

    let abandoned = adapter
        .complete(
            &issue,
            &intent.intent_id,
            record.revision,
            &IssueFeedbackCompletion::Abandoned,
        )
        .unwrap()
        .unwrap();
    assert_eq!(abandoned.state, IssueFeedbackIntentState::Abandoned);
    let replacement = comment_intent(&current(&adapter, &issue), "after-control-change");
    let replacement = recorded(&adapter, &replacement);
    assert_eq!(
        sent(&adapter, &replacement).state,
        IssueFeedbackIntentState::Sent
    );
    assert_eq!(current(&adapter, &issue).state.desired_revision, 1);
}

#[test]
fn feedback_disabled_or_unsafe_current_tracking_can_reserve_but_cannot_record_a_write() {
    for (name, flag) in [
        ("off", None),
        ("held", Some(IssueAutomationFlag::Held)),
        ("stale", Some(IssueAutomationFlag::Stale)),
        ("unknown", Some(IssueAutomationFlag::Unknown)),
    ] {
        let fixture = Fixture::new(&format!("feedback-blocked-{name}"));
        let issue = seed(&fixture, "I_1");
        let adapter = fixture.adapter();
        if let Some(flag) = flag {
            let mut tracked = adapter.get_issue(fixture.cwd(), &issue).unwrap().unwrap();
            tracked.set_flag(flag, true);
            super::super::super::tests::save_changed(&adapter, fixture.cwd(), &tracked);
        } else {
            let collection = adapter.get_collection(fixture.cwd()).unwrap().unwrap();
            adapter
                .set_collection_mode(
                    fixture.cwd(),
                    Some(collection.epoch),
                    IssueCollectionMode::Off,
                )
                .unwrap();
        }
        save_desired(&adapter, &issue, "상태: 확인 대기");
        let snapshot = current(&adapter, &issue);
        let intent = comment_intent(&snapshot, "blocked-write");
        assert!(
            matches!(
                adapter.record(snapshot.fence(), &intent).unwrap(),
                IssueFeedbackRecordResult::Conflict
            ),
            "{name}"
        );
        assert_eq!(count(&fixture, "issue_feedback_intents"), 0, "{name}");
        assert_eq!(snapshot.state.desired_revision, 1, "{name}");
    }
}

#[test]
fn feedback_unconfirmed_intent_blocks_new_commands_even_when_desired_advances() {
    let fixture = Fixture::new("feedback-unknown-blocks");
    let issue = seed(&fixture, "I_1");
    let adapter = fixture.adapter();
    save_desired(&adapter, &issue, "상태: 실행 중");
    let original = comment_intent(&current(&adapter, &issue), "lost-ack");
    let record = sent(&adapter, &recorded(&adapter, &original));
    let unknown = adapter
        .complete(
            &issue,
            &original.intent_id,
            record.revision,
            &IssueFeedbackCompletion::NeedsConfirmation {
                contradictory: false,
                acknowledged_comment_id: None,
            },
        )
        .unwrap()
        .unwrap();
    save_desired(&adapter, &issue, "상태: 완료됨");
    let snapshot = current(&adapter, &issue);
    let replacement = comment_intent(&snapshot, "new-command");
    assert!(matches!(
        adapter.record(snapshot.fence(), &replacement).unwrap(),
        IssueFeedbackRecordResult::Blocked
    ));
    assert!(matches!(
        adapter.claim_sent(&issue, &original.intent_id, record.revision, snapshot.fence()).unwrap(),
        IssueFeedbackClaimResult::Existing(existing) if existing == unknown
    ));
    assert!(
        adapter
            .complete(
                &issue,
                &original.intent_id,
                unknown.revision,
                &IssueFeedbackCompletion::Abandoned,
            )
            .is_err()
    );
    assert_eq!(count(&fixture, "issue_feedback_intents"), 1);
    assert_eq!(adapter.pending_events(fixture.cwd()).unwrap().len(), 0);
}

#[test]
fn feedback_remote_aliases_share_blockers_and_ack_but_cannot_rebind_comment_ownership() {
    let fixture = Fixture::new("feedback-alias");
    let issue = seed(&fixture, "I_1");
    let adapter = fixture.adapter();
    save_desired(&adapter, &issue, "상태: 실행 중");
    let intent = comment_intent(&current(&adapter, &issue), "alias-original");
    let record = sent(&adapter, &recorded(&adapter, &intent));
    let alias_workspace = fixture.root.join("historical-alias");
    crate::private_directory::ensure_private_directory(&alias_workspace).unwrap();
    let mut alias = issue.clone();
    alias.project_scope.workspace_dir = alias_workspace.to_string_lossy().into_owned();
    assert_eq!(adapter.unresolved(&alias).unwrap(), vec![record.clone()]);
    assert_eq!(
        adapter.pending(&alias.project_scope.workspace_dir).unwrap(),
        vec![record.clone()]
    );
    assert!(adapter.snapshot(&alias).is_err());
    let mut aliased_intent = intent.clone();
    aliased_intent.issue = alias.clone();
    assert!(
        adapter
            .record(current(&adapter, &issue).fence(), &aliased_intent)
            .is_err()
    );
    let receipt = comment_receipt(&record, "IC_status");
    assert!(
        adapter
            .complete(
                &alias,
                &intent.intent_id,
                record.revision,
                &IssueFeedbackCompletion::Confirmed(Box::new(receipt.clone())),
            )
            .is_err()
    );
    confirmed(&adapter, &record, receipt);
    assert!(adapter.snapshot(&alias).is_err());
    assert_eq!(count(&fixture, "issue_feedback_state"), 1);
    let events = adapter
        .pending_events(&alias.project_scope.workspace_dir)
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].issue, issue);
    adapter
        .acknowledge_event(&alias.project_scope.workspace_dir, &events[0].event_id)
        .unwrap();
    assert!(adapter.pending_events(fixture.cwd()).unwrap().is_empty());
    assert_eq!(
        current(&adapter, &issue)
            .state
            .comment_owner
            .unwrap()
            .comment_id,
        "IC_status"
    );
}

#[test]
fn feedback_distinct_remote_issues_can_each_own_an_unconfirmed_intent() {
    let fixture = Fixture::new("feedback-distinct-issues");
    let first = seed(&fixture, "I_1");
    let second = seed(&fixture, "I_2");
    let adapter = fixture.adapter();
    for (issue, command) in [(&first, "remote-one"), (&second, "remote-two")] {
        save_desired(&adapter, issue, "상태: 실행 중");
        let intent = comment_intent(&current(&adapter, issue), command);
        let record = sent(&adapter, &recorded(&adapter, &intent));
        assert_eq!(adapter.unresolved(issue).unwrap(), vec![record]);
    }
    assert_eq!(adapter.pending(fixture.cwd()).unwrap().len(), 2);
    assert_eq!(count(&fixture, "issue_feedback_state"), 2);
}

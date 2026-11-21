use super::*;

#[test]
fn pending_feedback_label_defers_fresh_resync_without_replacing_the_start_approval() {
    let h = Harness::new();
    h.tick(true).unwrap();
    let decision_id = h.decision().request.id().clone();
    h.answer("start");
    let before = page(&h.triage).items.remove(0);
    *h.source.read.lock().unwrap() = IssueSourceRead::Unknown {
        issue: h.fixture.reference("I_001"),
        reason: IssueSourceFailure::FeedbackUnconfirmed,
    };
    h.tick(true).unwrap();
    let after = page(&h.triage).items.remove(0);
    assert_eq!(after.current, before.current);
    assert_eq!(after.tracked, before.tracked);
    assert_eq!(
        after.triage.as_ref().unwrap().assessment,
        before.triage.as_ref().unwrap().assessment,
    );
    assert!(h.work().is_none());
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);

    *h.source.read.lock().unwrap() =
        IssueSourceRead::Observed(Box::new(h.fixture.observation("I_001")));
    h.tick(true).unwrap();
    let work = h.work().unwrap();
    assert_eq!(work.decision_id, decision_id);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
}

#[test]
fn a_human_edit_after_feedback_deferral_still_invalidates_the_old_approval() {
    for hold in [false, true] {
        let h = Harness::new();
        h.tick(true).unwrap();
        h.answer("start");
        *h.source.read.lock().unwrap() = IssueSourceRead::Unknown {
            issue: h.fixture.reference("I_001"),
            reason: IssueSourceFailure::FeedbackUnconfirmed,
        };
        h.tick(true).unwrap();
        assert_eq!(h.execution.state.lock().unwrap().registrations, 0);

        let mut changed = h.fixture.observation("I_001");
        if hold {
            changed.labels.items.push(IssueSourceLabel {
                id: "L_hold".into(),
                name: "akra:hold".into(),
            });
        } else if let IssueSourceContent::Shared(shared) = &mut changed.content {
            shared.shared_copy.title = "사람이 바꾼 작업 범위".into();
            shared.basis.field_hash =
                IssueFieldHash::of_fields(&shared.shared_copy.title, &shared.shared_copy.body);
        }
        *h.source.read.lock().unwrap() = IssueSourceRead::Observed(Box::new(changed));
        h.tick(true).unwrap();
        assert!(h.work().is_none());
        assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
    }
}

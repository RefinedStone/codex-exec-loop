use super::*;

#[test]
fn pause_during_feedback_preparation_starts_no_external_write() {
    let fixture = Fixture::new(true, false);
    let admission = fixture.admission.clone();
    let mut state = fixture.store.snapshot_value().binding_pause;
    state.paused = true;
    state.revision += 1;
    *fixture.provider.after_actor.lock().unwrap() = Some(Box::new(move || {
        admission
            .begin_transition(&state.binding_id.clone(), || Ok(((), state)))
            .unwrap();
    }));
    assert!(fixture.sync().is_err());
    assert!(fixture.writes().is_empty());
    assert!(!fixture.admission.has_inflight());
}

#[test]
fn feedback_ack_or_unknown_must_settle_before_pause_can_complete() {
    for behavior in [WriteBehavior::Ack, WriteBehavior::Lost] {
        let fixture = Fixture::new(true, false);
        *fixture.provider.write_behavior.lock().unwrap() = behavior;
        let admission = fixture.admission.clone();
        let mut state = fixture.store.snapshot_value().binding_pause;
        state.paused = true;
        state.revision += 1;
        let paused = state.clone();
        *fixture.provider.before_write.lock().unwrap() = Some(Box::new(move || {
            admission
                .begin_transition(&state.binding_id.clone(), || Ok(((), state.clone())))
                .unwrap();
            assert!(admission.has_inflight());
            assert!(admission.finish_if_drained(&state.binding_id, state.revision, || -> Result<((), crate::domain::issue_automation::binding_pause::IssueBindingPauseState)> {
                panic!("feedback response or Unknown has not settled")
            }).unwrap().is_none());
        }));
        fixture.sync().unwrap();
        assert_eq!(fixture.writes().len(), 1);
        assert!(!fixture.admission.has_inflight());
        assert!(
            fixture
                .admission
                .finish_if_drained(&paused.binding_id, paused.revision, || Ok((
                    (),
                    paused.clone()
                )))
                .unwrap()
                .is_some()
        );
        // Read-only Unknown reconciliation remains available after admission closes.
        fixture
            .service()
            .reconcile_pending(&IssueCollectionCancellation::new())
            .unwrap();
        assert_eq!(fixture.writes().len(), 1);
        assert!(fixture.sync().is_err());
        assert_eq!(fixture.writes().len(), 1);
    }
}

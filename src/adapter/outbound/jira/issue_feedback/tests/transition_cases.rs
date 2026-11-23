use super::*;

#[test]
fn explicit_transition_uses_the_exact_status_ids_and_a_typed_204_acknowledgement() {
    let fixture = Fixture::new();
    let intent = fixture.close_intent();
    assert_eq!(
        fixture.adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Acknowledged { comment_id: None }
    );
    assert!(
        matches!(fixture.adapter.reconcile(&intent).unwrap(), IssueFeedbackReadResult::Confirmed(receipt)
        if matches!(receipt.evidence, IssueFeedbackEvidence::Closed { ref event_id } if event_id == "2000"))
    );
    assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 1);
    assert!(fixture.adapter.write(&intent).is_err());
    assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 1);
}

#[test]
fn transitions_off_wrong_target_required_fields_or_changed_source_send_nothing() {
    for change in 0..5 {
        let fixture = Fixture::new();
        let intent = fixture.close_intent();
        match change {
            0 => fixture.policy.lock().unwrap().close_transition = None,
            1 => fixture.api.remote.lock().unwrap().transitions[0]["to"]["id"] = json!("4"),
            2 => {
                fixture.api.remote.lock().unwrap().transitions[0]["to"]["statusCategory"]["key"] =
                    json!("indeterminate")
            }
            3 => {
                fixture.api.remote.lock().unwrap().transitions[0]["fields"] =
                    json!({"resolution":{"required":true,"hasDefaultValue":true}})
            }
            _ => fixture.api.remote.lock().unwrap().status = "2".into(),
        }
        assert!(fixture.adapter.write(&intent).is_err());
        assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn same_status_after_an_intervening_workflow_event_does_not_reuse_the_preflight() {
    let fixture = Fixture::new();
    let intent = fixture.close_intent();
    fixture.api.remote.lock().unwrap().history = vec![
        history("100", "1", "2", "person"),
        history("101", "2", "1", "person"),
    ];
    assert_eq!(
        fixture.adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Contradictory
    );
    assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 0);
}

#[test]
fn lost_transition_ack_is_read_only_and_readback_never_sends_again() {
    let fixture = Fixture::new();
    let intent = fixture.close_intent();
    fixture.api.lose_ack.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture.adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Unconfirmed
    );
    fixture.credentials.rotate();
    assert!(matches!(
        fixture.adapter.reconcile(&intent).unwrap(),
        IssueFeedbackReadResult::Confirmed(_)
    ));
    assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 1);
    // The application journal additionally requires its durable CloseAcknowledged bit;
    // this remote observation alone does not settle a lost ACK as an owned close.
}

#[test]
fn foreign_status_actor_reopen_and_partial_history_do_not_prove_the_transition() {
    for change in 0..3 {
        let fixture = Fixture::new();
        let intent = fixture.close_intent();
        fixture.adapter.write(&intent).unwrap();
        let mut remote = fixture.api.remote.lock().unwrap();
        match change {
            0 => remote.history[0]["author"]["accountId"] = json!("person"),
            1 => {
                remote.history.push(history("2001", "3", "1", ACTOR));
                remote.status = "1".into();
                remote.done = false;
            }
            _ => remote.incomplete = true,
        }
        drop(remote);
        assert!(!matches!(
            fixture.adapter.reconcile(&intent).unwrap(),
            IssueFeedbackReadResult::Confirmed(_)
        ));
        assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 1);
    }
}

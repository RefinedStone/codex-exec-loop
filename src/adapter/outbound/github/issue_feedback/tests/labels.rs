use super::*;

#[test]
fn missing_progress_label_is_reported_without_creating_it() {
    let (adapter, calls) = adapter_with(vec![Ok(resolve_label(Value::Null))]);
    let snapshot = adapter
        .read_label(&issue(), ISSUE_FEEDBACK_PROGRESS_LABEL)
        .unwrap();
    assert!(snapshot.complete);
    assert!(snapshot.label.is_none());
    assert!(!snapshot.present);
    assert!(snapshot.latest_event.is_none());
    assert_eq!(mutation_count(&calls), 0);
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[test]
fn an_existing_label_remains_a_noop_observation_without_owning_its_effect() {
    let (adapter, calls) = adapter_with(label_pages(
        true,
        vec![label_event("LE_existing", "U_1", true)],
    ));
    let observed = adapter
        .read_label(&issue(), ISSUE_FEEDBACK_PROGRESS_LABEL)
        .unwrap();
    assert!(observed.complete && observed.present);
    assert_eq!(observed.label, Some(progress_label()));
    assert_eq!(observed.latest_event.unwrap().event_id, "LE_existing");
    assert_eq!(mutation_count(&calls), 0);
}

#[test]
fn label_presence_is_checked_after_every_page_including_the_last() {
    let mut first = issue_node();
    let others: Vec<Value> = (0..100).map(|index| json!({
        "id":format!("LA_other_{index}"),"name":format!("other-{index}"),"repository":{"id":"R_1"}
    })).collect();
    first["labels"] = json!({
        "nodes":others,"totalCount":101,
        "pageInfo":{"hasNextPage":true,"endCursor":"labels-1"}
    });
    let mut rest = label_pages(true, vec![label_event("LE_existing", "U_other", true)]);
    let timeline = rest.pop().unwrap();
    let mut last = rest.pop().unwrap().unwrap();
    last["data"]["node"]["labels"]["totalCount"] = json!(101);
    let (adapter, calls) = adapter_with(vec![
        Ok(resolve_label(remote_label())),
        Ok(node_response(first)),
        Ok(last),
        timeline,
    ]);
    let observed = adapter
        .read_label(&issue(), ISSUE_FEEDBACK_PROGRESS_LABEL)
        .unwrap();
    assert!(observed.present && observed.complete);
    assert_eq!(calls.lock().unwrap()[2].last().unwrap(), "after=labels-1");
    assert_eq!(mutation_count(&calls), 0);
}

#[test]
fn lost_label_ack_requires_one_new_exact_actor_event_and_never_resends() {
    let intent = label_intent(true, None);
    let mut responses = vec![Ok(viewer("U_1"))];
    responses.extend(label_pages(false, Vec::new()));
    responses.push(Err(anyhow!("fixture label ACK lost")));
    responses.extend(label_pages(true, vec![label_event("LE_own", "U_1", true)]));
    let (adapter, calls) = adapter_with(responses);
    assert_eq!(
        adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Unconfirmed
    );
    let receipt = confirmed_receipt(adapter.reconcile(&intent).unwrap());
    receipt.validate_for(&intent).unwrap();
    assert_eq!(
        receipt.evidence,
        IssueFeedbackEvidence::Label {
            label: progress_label(),
            event_id: "LE_own".to_string(),
            applied: true,
        }
    );
    assert_eq!(mutation_count(&calls), 1);
    assert!(
        calls.lock().unwrap()[4]
            .iter()
            .any(|argument| argument.contains("addLabelsToLabelable"))
    );
}

#[test]
fn removing_a_label_uses_the_exact_owned_event_and_returns_a_new_removal_proof() {
    let intent = label_intent(false, Some("LE_own"));
    let mut responses = vec![Ok(viewer("U_1"))];
    responses.extend(label_pages(true, vec![label_event("LE_own", "U_1", true)]));
    responses.push(Ok(label_ack()));
    responses.extend(label_pages(
        false,
        vec![
            label_event("LE_own", "U_1", true),
            label_event("LE_removed", "U_1", false),
        ],
    ));
    let (adapter, calls) = adapter_with(responses);
    assert_eq!(
        adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Acknowledged { comment_id: None }
    );
    let receipt = confirmed_receipt(adapter.reconcile(&intent).unwrap());
    receipt.validate_for(&intent).unwrap();
    assert_eq!(
        receipt.evidence,
        IssueFeedbackEvidence::Label {
            label: progress_label(),
            event_id: "LE_removed".to_string(),
            applied: false,
        }
    );
    assert_eq!(mutation_count(&calls), 1);
    assert!(
        calls.lock().unwrap()[4]
            .iter()
            .any(|argument| argument.contains("removeLabelsFromLabelable"))
    );
}

#[test]
fn a_changed_label_preflight_does_not_write_or_claim_an_existing_effect() {
    let cases = [
        (
            label_intent(true, None),
            vec![label_event("LE_existing", "U_1", true)],
        ),
        (
            label_intent(false, Some("LE_own")),
            vec![
                label_event("LE_own", "U_1", true),
                label_event("LE_removed", "U_1", false),
                label_event("LE_person", "U_1", true),
            ],
        ),
        (
            label_intent(false, Some("LE_own")),
            vec![label_event("LE_own", "U_other", true)],
        ),
    ];
    for (intent, events) in cases {
        let mut responses = vec![Ok(viewer("U_1"))];
        responses.extend(label_pages(true, events));
        let (adapter, calls) = adapter_with(responses);
        assert_eq!(
            adapter.write(&intent).unwrap(),
            IssueFeedbackWriteResult::Contradictory
        );
        assert_eq!(mutation_count(&calls), 0);
    }
}

#[test]
fn missing_ambiguous_or_foreign_label_effects_cannot_be_confirmed() {
    let mut missing_actor = label_event("LE_new", "U_1", true);
    missing_actor["actor"] = Value::Null;
    let cases = vec![
        (
            true,
            vec![label_event("LE_new", "U_1", true)],
            IssueFeedbackReadResult::NeedsConfirmation,
        ),
        (
            false,
            vec![label_event("LE_previous", "U_1", false)],
            IssueFeedbackReadResult::NeedsConfirmation,
        ),
        (
            true,
            vec![
                label_event("LE_previous", "U_1", false),
                label_event("LE_new", "U_other", true),
            ],
            IssueFeedbackReadResult::Contradictory,
        ),
        (
            true,
            vec![
                label_event("LE_previous", "U_1", false),
                label_event("LE_own", "U_1", true),
                label_event("LE_removed", "U_1", false),
                label_event("LE_person", "U_1", true),
            ],
            IssueFeedbackReadResult::Contradictory,
        ),
        (
            true,
            vec![label_event("LE_previous", "U_1", false), missing_actor],
            IssueFeedbackReadResult::NeedsConfirmation,
        ),
        (
            true,
            vec![
                label_event("LE_previous", "U_1", true),
                label_event("LE_new", "U_1", true),
            ],
            IssueFeedbackReadResult::Contradictory,
        ),
    ];
    for (present, events, expected) in cases {
        let (adapter, calls) = adapter_with(label_pages(present, events));
        assert_eq!(
            adapter
                .reconcile(&label_intent(true, Some("LE_previous")))
                .unwrap(),
            expected
        );
        assert_eq!(mutation_count(&calls), 0);
    }
}

#[test]
fn incomplete_label_membership_blocks_mutation() {
    let mut labels = issue_node();
    labels["labels"] = json!({
        "nodes":[remote_label()],"totalCount":2,
        "pageInfo":{"hasNextPage":true,"endCursor":null}
    });
    let (adapter, calls) = adapter_with(vec![
        Ok(viewer("U_1")),
        Ok(resolve_label(remote_label())),
        Ok(node_response(labels)),
    ]);
    assert_eq!(
        adapter.write(&label_intent(true, None)).unwrap(),
        IssueFeedbackWriteResult::Unconfirmed
    );
    assert_eq!(mutation_count(&calls), 0);
}

#[test]
fn incomplete_label_history_never_confirms_the_first_matching_event() {
    let mut responses = label_pages(true, vec![label_event("LE_own", "U_1", true)]);
    let mut first = responses.pop().unwrap().unwrap();
    first["data"]["node"]["timelineItems"]["totalCount"] = json!(2);
    first["data"]["node"]["timelineItems"]["pageInfo"] = json!({
        "hasNextPage":true,"endCursor":"events-1"
    });
    responses.push(Ok(first));
    responses.push(Err(anyhow!("fixture remaining events unavailable")));
    let (adapter, calls) = adapter_with(responses);
    assert_eq!(
        adapter.reconcile(&label_intent(true, None)).unwrap(),
        IssueFeedbackReadResult::NeedsConfirmation
    );
    assert_eq!(mutation_count(&calls), 0);
}

#[test]
fn label_ack_for_a_different_issue_is_contradictory() {
    let mut responses = vec![Ok(viewer("U_1"))];
    responses.extend(label_pages(false, Vec::new()));
    let mut wrong = label_ack();
    wrong["data"]["feedback"]["labelable"]["id"] = json!("I_other");
    responses.push(Ok(wrong));
    let (adapter, calls) = adapter_with(responses);
    assert_eq!(
        adapter.write(&label_intent(true, None)).unwrap(),
        IssueFeedbackWriteResult::Contradictory
    );
    assert_eq!(mutation_count(&calls), 1);
}

#[test]
fn label_identity_and_repository_are_checked_before_use() {
    let mut wrong_repository = remote_label();
    wrong_repository["repository"]["id"] = json!("R_other");
    let mut wrong_name = remote_label();
    wrong_name["name"] = json!("akra:auto");
    for response in [wrong_repository, wrong_name] {
        let (adapter, calls) = adapter_with(vec![Ok(resolve_label(response))]);
        assert!(
            adapter
                .read_label(&issue(), ISSUE_FEEDBACK_PROGRESS_LABEL)
                .is_err()
        );
        assert_eq!(mutation_count(&calls), 0);
    }
}

#[test]
fn current_membership_and_latest_label_event_must_agree() {
    let (adapter, calls) =
        adapter_with(label_pages(false, vec![label_event("LE_new", "U_1", true)]));
    assert_eq!(
        adapter.reconcile(&label_intent(true, None)).unwrap(),
        IssueFeedbackReadResult::NeedsConfirmation
    );
    assert_eq!(mutation_count(&calls), 0);
}

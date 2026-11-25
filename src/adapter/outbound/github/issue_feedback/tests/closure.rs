use super::*;

fn close_intent(previous: Option<&str>) -> IssueFeedbackIntent {
    let mut intent = create_intent();
    intent.operation = IssueFeedbackOperation::CloseIssue {
        integration_record_id: "integration-1".to_string(),
        integration_sha: "a".repeat(40),
        decision_id: "decision-1".to_string(),
        jira_transition: None,
        linear_state: None,
        previous_event_id: previous.map(str::to_string),
    };
    intent
}

fn closure_event(id: &str, actor_id: &str, closed: bool) -> Value {
    json!({
        "__typename": if closed { "ClosedEvent" } else { "ReopenedEvent" },
        "id": id, "actor": actor(actor_id), "closable": issue_node()
    })
}

fn closure_page(closed: bool, events: Vec<Value>, total: usize, next: Option<&str>) -> Value {
    let mut node = issue_node();
    node["state"] = json!(if closed { "CLOSED" } else { "OPEN" });
    node["closureEvents"] = json!({
        "nodes": events, "totalCount": total,
        "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next}
    });
    node_response(node)
}

fn closure_snapshot(closed: bool, events: Vec<Value>) -> Value {
    let count = events.len();
    closure_page(closed, events, count, None)
}

fn close_ack() -> Value {
    let mut node = issue_node();
    node["state"] = json!("CLOSED");
    json!({"data":{"feedback":{"clientMutationId":"intent-1","issue":node}}})
}

#[test]
fn closure_observation_is_read_only_and_keeps_an_existing_closed_state() {
    for (closed, events) in [
        (false, Vec::new()),
        (true, vec![closure_event("CE_existing", "U_other", true)]),
    ] {
        let (adapter, calls) = adapter_with(vec![Ok(closure_snapshot(closed, events))]);
        let snapshot = adapter.read_closure(&issue()).unwrap();
        assert_eq!(snapshot.issue, issue());
        assert!(snapshot.complete);
        assert_eq!(snapshot.closed, closed);
        assert_eq!(snapshot.latest_event.is_some(), closed);
        assert_eq!(mutation_count(&calls), 0);
    }
}

#[test]
fn close_writes_once_and_requires_a_new_event_after_the_exact_reopen_basis() {
    let intent = close_intent(Some("RE_prior"));
    let prior = closure_event("RE_prior", "U_other", false);
    let (adapter, calls) = adapter_with(vec![
        Ok(viewer("U_1")),
        Ok(closure_snapshot(false, vec![prior.clone()])),
        Ok(close_ack()),
        Ok(closure_snapshot(
            true,
            vec![prior, closure_event("CE_own", "U_1", true)],
        )),
    ]);
    assert_eq!(
        adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Acknowledged { comment_id: None }
    );
    let receipt = confirmed_receipt(adapter.reconcile(&intent).unwrap());
    receipt.validate_for(&intent).unwrap();
    assert_eq!(
        receipt.evidence,
        IssueFeedbackEvidence::Closed {
            event_id: "CE_own".to_string()
        }
    );
    assert_eq!(mutation_count(&calls), 1);
    let calls = calls.lock().unwrap();
    assert!(
        calls[2]
            .iter()
            .any(|arg| arg.contains("stateReason: COMPLETED"))
    );
    assert!(calls[2].iter().any(|arg| arg == "intent=intent-1"));
}

#[test]
fn lost_close_ack_is_reconciled_by_readback_without_a_second_mutation() {
    let intent = close_intent(None);
    let (adapter, calls) = adapter_with(vec![
        Ok(viewer("U_1")),
        Ok(closure_snapshot(false, Vec::new())),
        Err(anyhow!("fixture close ACK lost")),
        Ok(closure_snapshot(
            true,
            vec![closure_event("CE_own", "U_1", true)],
        )),
    ]);
    assert_eq!(
        adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Unconfirmed
    );
    confirmed_receipt(adapter.reconcile(&intent).unwrap())
        .validate_for(&intent)
        .unwrap();
    assert_eq!(mutation_count(&calls), 1);
}

#[test]
fn lost_close_ack_and_failed_reads_remain_unconfirmed_without_resending() {
    let intent = close_intent(None);
    let (adapter, calls) = adapter_with(vec![
        Ok(viewer("U_1")),
        Ok(closure_snapshot(false, Vec::new())),
        Err(anyhow!("fixture close ACK lost")),
        Err(anyhow!("fixture lookup unavailable")),
        Err(anyhow!("fixture lookup still unavailable")),
    ]);
    assert_eq!(
        adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Unconfirmed
    );
    for _ in 0..2 {
        assert_eq!(
            adapter.reconcile(&intent).unwrap(),
            IssueFeedbackReadResult::NeedsConfirmation
        );
    }
    assert_eq!(mutation_count(&calls), 1);
}

#[test]
fn changed_or_already_closed_preflight_never_mutates_the_issue() {
    for (intent, observed) in [
        (
            close_intent(None),
            closure_snapshot(true, vec![closure_event("CE_existing", "U_1", true)]),
        ),
        (
            close_intent(Some("RE_prior")),
            closure_snapshot(false, vec![closure_event("RE_later", "U_1", false)]),
        ),
    ] {
        let (adapter, calls) = adapter_with(vec![Ok(viewer("U_1")), Ok(observed)]);
        assert_eq!(
            adapter.write(&intent).unwrap(),
            IssueFeedbackWriteResult::Contradictory
        );
        assert_eq!(mutation_count(&calls), 0);
    }
}

#[test]
fn foreign_or_changed_close_effects_are_not_owned_by_this_intent() {
    let prior = closure_event("RE_prior", "U_other", false);
    let mut missing_actor = closure_event("CE_new", "U_1", true);
    missing_actor["actor"] = Value::Null;
    for (closed, events, expected) in [
        (
            true,
            vec![closure_event("CE_new", "U_1", true)],
            IssueFeedbackReadResult::NeedsConfirmation,
        ),
        (
            false,
            vec![prior.clone()],
            IssueFeedbackReadResult::NeedsConfirmation,
        ),
        (
            true,
            vec![prior.clone(), closure_event("CE_foreign", "U_other", true)],
            IssueFeedbackReadResult::Contradictory,
        ),
        (
            true,
            vec![prior.clone(), missing_actor],
            IssueFeedbackReadResult::NeedsConfirmation,
        ),
        (
            false,
            vec![
                prior.clone(),
                closure_event("CE_own", "U_1", true),
                closure_event("RE_later", "U_1", false),
            ],
            IssueFeedbackReadResult::Contradictory,
        ),
        (
            true,
            vec![
                prior,
                closure_event("CE_own", "U_1", true),
                closure_event("RE_later", "U_1", false),
                closure_event("CE_person", "U_1", true),
            ],
            IssueFeedbackReadResult::Contradictory,
        ),
    ] {
        let (adapter, calls) = adapter_with(vec![Ok(closure_snapshot(closed, events))]);
        assert_eq!(
            adapter.reconcile(&close_intent(Some("RE_prior"))).unwrap(),
            expected
        );
        assert_eq!(mutation_count(&calls), 0);
    }
}

#[test]
fn closure_identity_must_match_the_issue_and_repository_at_every_boundary() {
    for field in ["issue", "repository", "closable"] {
        let mut response = closure_snapshot(true, vec![closure_event("CE_own", "U_1", true)]);
        match field {
            "issue" => response["data"]["node"]["id"] = json!("I_other"),
            "repository" => response["data"]["node"]["repository"]["id"] = json!("R_other"),
            _ => {
                response["data"]["node"]["closureEvents"]["nodes"][0]["closable"]["id"] =
                    json!("I_other")
            }
        }
        let (adapter, calls) = adapter_with(vec![Ok(response)]);
        assert_eq!(
            adapter.reconcile(&close_intent(None)).unwrap(),
            IssueFeedbackReadResult::NeedsConfirmation
        );
        assert_eq!(mutation_count(&calls), 0);
    }
}

#[test]
fn close_ack_identity_or_intent_mismatch_is_contradictory() {
    for field in ["issue", "repository", "intent", "state"] {
        let mut ack = close_ack();
        match field {
            "issue" => ack["data"]["feedback"]["issue"]["id"] = json!("I_other"),
            "repository" => ack["data"]["feedback"]["issue"]["repository"]["id"] = json!("R_other"),
            "intent" => ack["data"]["feedback"]["clientMutationId"] = json!("intent-other"),
            _ => ack["data"]["feedback"]["issue"]["state"] = json!("OPEN"),
        }
        let (adapter, calls) = adapter_with(vec![
            Ok(viewer("U_1")),
            Ok(closure_snapshot(false, Vec::new())),
            Ok(ack),
        ]);
        assert_eq!(
            adapter.write(&close_intent(None)).unwrap(),
            IssueFeedbackWriteResult::Contradictory
        );
        assert_eq!(mutation_count(&calls), 1);
    }
}

#[test]
fn close_account_change_prevents_the_mutation() {
    let (adapter, calls) = adapter_with(vec![Ok(viewer("U_other"))]);
    assert_eq!(
        adapter.write(&close_intent(None)).unwrap(),
        IssueFeedbackWriteResult::Contradictory
    );
    assert_eq!(mutation_count(&calls), 0);
}

#[test]
fn closure_history_must_complete_before_confirming_a_matching_event() {
    let first = closure_page(
        true,
        vec![closure_event("CE_own", "U_1", true)],
        2,
        Some("events-1"),
    );
    let (adapter, calls) = adapter_with(vec![
        Ok(first),
        Err(anyhow!("fixture next page unavailable")),
    ]);
    assert_eq!(
        adapter.reconcile(&close_intent(None)).unwrap(),
        IssueFeedbackReadResult::NeedsConfirmation
    );
    assert_eq!(mutation_count(&calls), 0);
}

#[test]
fn closure_history_reads_the_final_page_and_rejects_state_changes_between_pages() {
    let first_events: Vec<Value> = (0..100)
        .map(|index| closure_event(&format!("CE_{index}"), "U_1", index % 2 == 0))
        .collect();
    let first = closure_page(true, first_events, 101, Some("events-100"));
    let last = closure_page(true, vec![closure_event("CE_100", "U_1", true)], 101, None);
    let (adapter, calls) = adapter_with(vec![Ok(first.clone()), Ok(last)]);
    let receipt = confirmed_receipt(adapter.reconcile(&close_intent(Some("CE_99"))).unwrap());
    assert_eq!(
        receipt.evidence,
        IssueFeedbackEvidence::Closed {
            event_id: "CE_100".to_string()
        }
    );
    assert_eq!(calls.lock().unwrap()[1].last().unwrap(), "after=events-100");
    assert_eq!(mutation_count(&calls), 0);

    let changed = closure_page(
        false,
        vec![closure_event("RE_changed", "U_1", false)],
        101,
        None,
    );
    let (adapter, _) = adapter_with(vec![Ok(first), Ok(changed)]);
    assert_eq!(
        adapter.reconcile(&close_intent(Some("CE_99"))).unwrap(),
        IssueFeedbackReadResult::NeedsConfirmation
    );
}

#[test]
fn missing_or_inconsistent_closure_state_cannot_confirm_or_write() {
    let mut missing = closure_snapshot(false, Vec::new());
    missing["data"]["node"]["state"] = Value::Null;
    for response in [
        missing,
        closure_snapshot(true, Vec::new()),
        closure_snapshot(false, vec![closure_event("CE_own", "U_1", true)]),
    ] {
        let (adapter, calls) = adapter_with(vec![Ok(viewer("U_1")), Ok(response)]);
        assert_eq!(
            adapter.write(&close_intent(None)).unwrap(),
            IssueFeedbackWriteResult::Unconfirmed
        );
        assert_eq!(mutation_count(&calls), 0);
    }
}

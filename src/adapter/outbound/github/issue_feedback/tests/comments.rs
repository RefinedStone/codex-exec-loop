use super::*;

#[test]
fn lost_create_ack_is_confirmed_only_after_every_comment_page_without_resending() {
    let intent = create_intent();
    let (adapter, calls) = adapter_with(vec![
        Ok(viewer("U_1")),
        Err(anyhow!("fixture ACK was lost")),
        Ok(comment_page(
            vec![comment("IC_old", "intent-old", false)],
            2,
            Some("cursor-1"),
        )),
        Ok(comment_page(
            vec![comment("IC_1", "intent-1", false)],
            2,
            None,
        )),
    ]);
    assert_eq!(
        adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Unconfirmed
    );
    let receipt = confirmed_receipt(adapter.reconcile(&intent).unwrap());
    receipt.validate_for(&intent).unwrap();
    assert_eq!(
        receipt.evidence,
        IssueFeedbackEvidence::Comment {
            comment_id: "IC_1".to_string(),
            body_sha256: feedback_body_hash(&body("intent-1")),
        }
    );
    assert_eq!(mutation_count(&calls), 1);
    assert!(
        calls
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains(&"after=cursor-1".to_string())
    );
}

#[test]
fn lost_ack_and_failed_readback_remain_unknown_and_later_reads_never_write() {
    let (adapter, calls) = adapter_with(vec![
        Ok(viewer("U_1")),
        Err(anyhow!("fixture ACK was lost")),
        Err(anyhow!("fixture readback failed")),
        Ok(comment_page(Vec::new(), 0, None)),
    ]);
    assert_eq!(
        adapter.write(&create_intent()).unwrap(),
        IssueFeedbackWriteResult::Unconfirmed
    );
    for _ in 0..2 {
        assert_eq!(
            adapter.reconcile(&create_intent()).unwrap(),
            IssueFeedbackReadResult::NeedsConfirmation
        );
    }
    assert_eq!(mutation_count(&calls), 1);
}

#[test]
fn duplicate_owned_intent_comments_are_not_adopted() {
    let (adapter, calls) = adapter_with(vec![Ok(comment_page(
        vec![
            comment("IC_1", "intent-1", false),
            comment("IC_2", "intent-1", false),
        ],
        2,
        None,
    ))]);
    assert_eq!(
        adapter.reconcile(&create_intent()).unwrap(),
        IssueFeedbackReadResult::Contradictory
    );
    assert_eq!(mutation_count(&calls), 0);
}

#[test]
fn incomplete_or_unstable_comment_pagination_cannot_confirm_an_early_match() {
    let first = comment_page(
        vec![comment("IC_1", "intent-1", false)],
        2,
        Some("cursor-1"),
    );
    let mut missing_cursor = first.clone();
    missing_cursor["data"]["node"]["comments"]["pageInfo"]["endCursor"] = Value::Null;
    let null_node = comment_page(vec![Value::Null], 1, None);
    let cases = vec![
        vec![
            Ok(first.clone()),
            Err(anyhow!("fixture second page unavailable")),
        ],
        vec![
            Ok(first.clone()),
            Ok(comment_page(
                vec![comment("IC_old", "intent-old", false)],
                3,
                None,
            )),
        ],
        vec![
            Ok(first.clone()),
            Ok(comment_page(
                vec![comment("IC_1", "intent-1", false)],
                2,
                None,
            )),
        ],
        vec![Ok(missing_cursor)],
        vec![Ok(null_node)],
        vec![Ok(comment_page(
            vec![comment("IC_1", "intent-1", false)],
            2,
            None,
        ))],
    ];
    for responses in cases {
        let (adapter, calls) = adapter_with(responses);
        assert_eq!(
            adapter.reconcile(&create_intent()).unwrap(),
            IssueFeedbackReadResult::NeedsConfirmation
        );
        assert_eq!(mutation_count(&calls), 0);
    }
}

#[test]
fn a_repeated_cursor_never_turns_a_partial_scan_into_a_receipt() {
    let (adapter, calls) = adapter_with(vec![
        Ok(comment_page(
            vec![comment("IC_1", "intent-1", false)],
            3,
            Some("same"),
        )),
        Ok(comment_page(
            vec![comment("IC_old", "intent-old", false)],
            3,
            Some("same"),
        )),
    ]);
    assert_eq!(
        adapter.reconcile(&create_intent()).unwrap(),
        IssueFeedbackReadResult::NeedsConfirmation
    );
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert_eq!(mutation_count(&calls), 0);
}

#[test]
fn a_copied_marker_from_a_different_account_is_not_comment_ownership() {
    let mut copied = comment("IC_other", "intent-1", false);
    copied["author"] = actor("U_other");
    let (adapter, calls) = adapter_with(vec![Ok(comment_page(vec![copied.clone()], 1, None))]);
    assert_eq!(
        adapter.reconcile(&create_intent()).unwrap(),
        IssueFeedbackReadResult::NeedsConfirmation
    );
    assert_eq!(mutation_count(&calls), 0);
    let (adapter, _) = adapter_with(vec![Ok(comment_page(
        vec![copied, comment("IC_1", "intent-1", false)],
        2,
        None,
    ))]);
    let receipt = confirmed_receipt(adapter.reconcile(&create_intent()).unwrap());
    assert!(
        matches!(receipt.evidence, IssueFeedbackEvidence::Comment { comment_id, .. } if comment_id == "IC_1")
    );
}

#[test]
fn explicit_wrong_comment_ack_is_contradictory_and_is_never_retried() {
    let correct = comment_ack(comment("IC_1", "intent-1", false), false);
    let mut cases = Vec::new();
    for path in [
        vec![
            "data",
            "feedback",
            "commentEdge",
            "node",
            "repository",
            "id",
        ],
        vec!["data", "feedback", "commentEdge", "node", "issue", "id"],
        vec!["data", "feedback", "commentEdge", "node", "author", "id"],
        vec!["data", "feedback", "commentEdge", "node", "body"],
        vec!["data", "feedback", "clientMutationId"],
    ] {
        let mut wrong = correct.clone();
        let mut target = &mut wrong;
        for key in path {
            target = &mut target[key];
        }
        *target = json!("different");
        cases.push(wrong);
    }
    let mut wrong_editor = correct;
    wrong_editor["data"]["feedback"]["commentEdge"]["node"]["editor"] = actor("U_other");
    wrong_editor["data"]["feedback"]["commentEdge"]["node"]["lastEditedAt"] =
        json!("2026-11-21T14:01:00Z");
    cases.push(wrong_editor);
    for response in cases {
        let (adapter, calls) = adapter_with(vec![Ok(viewer("U_1")), Ok(response)]);
        assert_eq!(
            adapter.write(&create_intent()).unwrap(),
            IssueFeedbackWriteResult::Contradictory
        );
        assert_eq!(mutation_count(&calls), 1);
        assert_eq!(calls.lock().unwrap().len(), 2);
    }
}

#[test]
fn partial_ack_is_unknown_even_when_its_data_looks_successful() {
    let mut response = comment_ack(comment("IC_1", "intent-1", false), false);
    response["errors"] = json!([{"message":"FAKE_GH_RESPONSE_SECRET"}]);
    let (adapter, calls) = adapter_with(vec![Ok(viewer("U_1")), Ok(response)]);
    assert_eq!(
        adapter.write(&create_intent()).unwrap(),
        IssueFeedbackWriteResult::Unconfirmed
    );
    assert_eq!(mutation_count(&calls), 1);
}

#[test]
fn updating_an_owned_comment_uses_the_fixed_id_and_confirms_the_editor() {
    let intent = update_intent();
    let (adapter, calls) = adapter_with(vec![
        Ok(viewer("U_1")),
        Ok(node_response(comment("IC_1", "intent-old", false))),
        Ok(comment_ack(comment("IC_1", "intent-1", true), true)),
        Ok(node_response(comment("IC_1", "intent-1", true))),
    ]);
    assert_eq!(
        adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Acknowledged {
            comment_id: Some("IC_1".to_string())
        }
    );
    confirmed_receipt(adapter.reconcile(&intent).unwrap())
        .validate_for(&intent)
        .unwrap();
    assert_eq!(mutation_count(&calls), 1);
    let calls = calls.lock().unwrap();
    assert!(
        calls[2]
            .iter()
            .any(|argument| argument.contains("mutation AkraFeedbackUpdateComment"))
    );
    assert!(calls[2].contains(&"id=IC_1".to_string()));
    assert!(
        !calls[2]
            .iter()
            .any(|argument| argument.contains("addComment"))
    );
}

#[test]
fn a_changed_comment_owner_body_or_editor_blocks_update_before_mutation() {
    let old = comment("IC_1", "intent-old", false);
    let mut other_body = old.clone();
    other_body["body"] = json!("A person changed this status.");
    let mut other_owner = old.clone();
    other_owner["author"] = actor("U_other");
    let mut other_editor = old.clone();
    other_editor["editor"] = actor("U_other");
    other_editor["lastEditedAt"] = json!("2026-11-21T14:01:00Z");
    let mut other_issue = old;
    other_issue["issue"]["id"] = json!("I_other");
    for response in [
        other_body,
        other_owner,
        other_editor,
        other_issue,
        Value::Null,
    ] {
        let (adapter, calls) = adapter_with(vec![Ok(viewer("U_1")), Ok(node_response(response))]);
        assert_eq!(
            adapter.write(&update_intent()).unwrap(),
            IssueFeedbackWriteResult::Contradictory
        );
        assert_eq!(mutation_count(&calls), 0);
    }
}

#[test]
fn update_readback_needs_exact_id_hash_parent_and_editor() {
    let valid = comment("IC_1", "intent-1", true);
    let mut wrong_id = valid.clone();
    wrong_id["id"] = json!("IC_other");
    let mut wrong_parent = valid.clone();
    wrong_parent["issue"]["repository"]["id"] = json!("R_other");
    let mut wrong_body = valid.clone();
    wrong_body["body"] = json!(body("different-intent"));
    let mut wrong_editor = valid.clone();
    wrong_editor["editor"] = actor("U_other");
    for response in [wrong_id, wrong_parent, wrong_body, wrong_editor] {
        let (adapter, calls) = adapter_with(vec![Ok(node_response(response))]);
        assert_eq!(
            adapter.reconcile(&update_intent()).unwrap(),
            IssueFeedbackReadResult::Contradictory
        );
        assert_eq!(mutation_count(&calls), 0);
    }
    let mut missing_editor = valid;
    missing_editor["editor"] = Value::Null;
    let (adapter, calls) = adapter_with(vec![Ok(node_response(missing_editor))]);
    assert_eq!(
        adapter.reconcile(&update_intent()).unwrap(),
        IssueFeedbackReadResult::NeedsConfirmation
    );
    assert_eq!(mutation_count(&calls), 0);
}

#[test]
fn comment_body_is_a_literal_form_field_and_never_an_at_file_argument() {
    let mut intent = create_intent();
    let text = format!("@/fixture/not-a-file\n\n{}", feedback_marker("intent-1"));
    let IssueFeedbackOperation::CreateComment {
        body,
        body_sha256,
        content_sha256,
    } = &mut intent.operation
    else {
        unreachable!()
    };
    *body = text.clone();
    *body_sha256 = feedback_body_hash(&text);
    *content_sha256 = feedback_body_hash("@/fixture/not-a-file");
    let mut returned = comment("IC_1", "intent-1", false);
    returned["body"] = json!(text);
    let (adapter, calls) = adapter_with(vec![Ok(viewer("U_1")), Ok(comment_ack(returned, false))]);
    assert!(matches!(
        adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Acknowledged { .. }
    ));
    let calls = calls.lock().unwrap();
    assert!(
        calls[1]
            .windows(2)
            .any(|pair| pair[0] == "-f" && pair[1].starts_with("body=@/fixture/"))
    );
    assert!(
        !calls[1]
            .windows(2)
            .any(|pair| pair[0] == "-F" && pair[1].starts_with("body="))
    );
}

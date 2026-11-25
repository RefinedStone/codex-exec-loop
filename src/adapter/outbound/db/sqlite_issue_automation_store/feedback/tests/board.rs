//! 보드 필드 쓰기의 일지와 확인 (#2565).
use super::*;

fn board_intent(
    snapshot: &IssueFeedbackSnapshot,
    id: &str,
    option_id: &str,
    from: Option<&str>,
    origin: Option<&str>,
) -> IssueFeedbackIntent {
    make_intent(
        snapshot,
        id,
        IssueFeedbackOperation::SetBoardField {
            project_id: "PVT_1".into(),
            item_id: "PVTI_1".into(),
            field_id: "PVTSSF_1".into(),
            option_id: option_id.into(),
            from_option_id: from.map(Into::into),
            origin_option_id: origin.map(Into::into),
        },
    )
}

fn board_receipt(record: &IssueFeedbackIntentRecord) -> IssueFeedbackReceipt {
    let IssueFeedbackOperation::SetBoardField {
        project_id,
        item_id,
        field_id,
        option_id,
        origin_option_id,
        ..
    } = &record.intent.operation
    else {
        panic!("expected a board intent");
    };
    IssueFeedbackReceipt {
        issue: record.intent.issue.clone(),
        intent_id: record.intent.intent_id.clone(),
        actor_id: record.intent.actor_id.clone(),
        evidence: IssueFeedbackEvidence::BoardField {
            project_id: project_id.clone(),
            item_id: item_id.clone(),
            field_id: field_id.clone(),
            option_id: option_id.clone(),
            origin_option_id: origin_option_id.clone(),
        },
    }
}

#[test]
fn a_confirmed_board_write_is_owned_durably_and_continues_its_origin() {
    let fixture = Fixture::new("feedback-board-field");
    let issue = seed(&fixture, "I_board");
    let adapter = fixture.adapter();

    let first = board_intent(
        &current(&adapter, &issue),
        "board-running",
        "running",
        Some("ready"),
        Some("ready"),
    );
    let sent_record = sent(&adapter, &recorded(&adapter, &first));
    confirmed(&adapter, &sent_record, board_receipt(&sent_record));
    let events = adapter.pending_events(fixture.cwd()).unwrap();
    assert_eq!(
        events.last().unwrap().action,
        IssueFeedbackAction::BoardFieldSet
    );

    // 다시 연 장부도 확인된 자기 쓰기를 영수증과 함께 읽는다.
    let reopened = fixture.adapter();
    let state = current(&reopened, &issue).state;
    assert!(matches!(
        state.board_proof.as_ref().map(|proof| &proof.evidence),
        Some(IssueFeedbackEvidence::BoardField { option_id, .. }) if option_id == "running"
    ));
    assert_eq!(
        state.board_origin("PVT_1", "PVTI_1", "PVTSSF_1", Some("running")),
        Some("ready".to_string())
    );

    // 자기 쓰기 위에서 이어 쓰는 의도는 원래 값을 바꾸지 못한다.
    let wrong_origin = board_intent(
        &current(&reopened, &issue),
        "board-wrong",
        "done",
        Some("running"),
        Some("running"),
    );
    assert!(!matches!(
        reopened
            .record(current(&reopened, &issue).fence(), &wrong_origin)
            .unwrap(),
        IssueFeedbackRecordResult::Created(_)
    ));
    let next = board_intent(
        &current(&reopened, &issue),
        "board-done",
        "done",
        Some("running"),
        Some("ready"),
    );
    let sent_record = sent(&reopened, &recorded(&reopened, &next));
    confirmed(&reopened, &sent_record, board_receipt(&sent_record));
    assert!(matches!(
        current(&reopened, &issue).state.board_proof.map(|proof| proof.evidence),
        Some(IssueFeedbackEvidence::BoardField { option_id, origin_option_id: Some(origin), .. })
            if option_id == "done" && origin == "ready"
    ));
}

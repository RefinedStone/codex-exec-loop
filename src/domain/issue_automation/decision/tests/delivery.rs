use super::*;
use crate::domain::issue_automation::delivery::{
    IssueDeliveryDiffVerdict, IssueDeliveryRiskReason,
};

fn delivery_definition() -> IssueDecisionDefinition {
    let mut value = definition();
    value.kind = IssueDecisionKind::RiskException;
    value.gate = IssueAutomationGate::Delivery;
    value.basis.work_order_id = Some("issue-work-delivery".into());
    value.basis.delivery = Some(IssueDecisionDeliveryContext {
        base_commit_sha: "a".repeat(40),
        source_commit_sha: "b".repeat(40),
    });
    value.delivery_review = Some(IssueDeliveryDiffVerdict {
        changed_files: 1,
        changed_lines: Some(2),
        reasons: vec![IssueDeliveryRiskReason::ProtectedPath],
    });
    value
}

#[test]
fn delivery_risk_always_requires_human_even_for_an_auto_gate() {
    let request = IssueDecisionRequest::new(
        IssueDecisionId::new("delivery-risk").unwrap(),
        delivery_definition(),
    )
    .unwrap();
    assert_eq!(
        request.answer_agent(&request.definition().basis, input("proceed")),
        Err(IssueDecisionError::HumanConfirmationRequired)
    );
    assert!(
        request
            .timeout_for_gate(
                &request.definition().basis,
                IssueAutomationGateAction::TimeoutDefault,
                2_000_000_000_000,
            )
            .unwrap()
            .is_none()
    );
    let confirmed = request
        .answer_for_gate(
            &request.definition().basis,
            IssueAutomationGateAction::Auto,
            input("proceed"),
            Some("desktop-confirmation:0123456789abcdef0123456789abcdef"),
        )
        .unwrap();
    assert_eq!(
        confirmed.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );
}

#[test]
fn delivery_review_requires_its_exact_work_gate_and_wait_policy() {
    for change in 0..8 {
        let mut value = delivery_definition();
        match change {
            0 => value.kind = IssueDecisionKind::AgentQuestion,
            1 => value.gate = IssueAutomationGate::Start,
            2 => value.basis.work_order_id = None,
            3 => value.basis.delivery = None,
            4 => value.delivery_review = None,
            5 => value.on_timeout = IssueDecisionTimeout::Default,
            6 => value.delivery_review.as_mut().unwrap().reasons.clear(),
            7 => value.basis.delivery.as_mut().unwrap().source_commit_sha = "a".repeat(40),
            _ => unreachable!(),
        }
        assert!(
            IssueDecisionRequest::new(IssueDecisionId::new("invalid-delivery").unwrap(), value)
                .is_err(),
            "invalid delivery variant {change} must fail before persistence"
        );
    }
}

#[test]
fn the_source_range_and_immutable_review_both_participate_in_request_identity() {
    let original = IssueDecisionRequest::new(
        IssueDecisionId::new("delivery-risk").unwrap(),
        delivery_definition(),
    )
    .unwrap();
    for change in 0..3 {
        let mut changed = original.definition().clone();
        match change {
            0 => changed.basis.delivery.as_mut().unwrap().source_commit_sha = "c".repeat(40),
            1 => changed.basis.delivery.as_mut().unwrap().base_commit_sha = "c".repeat(40),
            2 => changed.delivery_review.as_mut().unwrap().changed_files = 2,
            _ => unreachable!(),
        }
        let changed = IssueDecisionRequest::new(original.id().clone(), changed).unwrap();
        assert_ne!(original.uniqueness_key(), changed.uniqueness_key());
        assert_eq!(
            changed.validate_successor(&original),
            Err(IssueDecisionError::DefinitionChanged)
        );
    }
    assert_eq!(
        original.fact().delivery_review,
        original.definition().delivery_review
    );
}

#[test]
fn legacy_basis_bytes_and_start_decision_ids_do_not_gain_delivery_fields() {
    let legacy = format!(
        "{{\"policyVersion\":\"{}\",\"fieldHash\":{{\"version\":1,\"sha256\":\"{}\"}},\"workOrderId\":null,\"gateRevision\":1}}",
        "a".repeat(64),
        "b".repeat(64),
    );
    let basis: IssueDecisionBasis = serde_json::from_str(&legacy).unwrap();
    basis.validate().unwrap();
    assert!(basis.delivery.is_none());
    assert_eq!(serde_json::to_string(&basis).unwrap(), legacy);
    let original_bytes = format!("[{},{}]", serde_json::to_string(&issue()).unwrap(), legacy);
    let expected = format!(
        "issue-start-{:x}",
        Sha256::digest(original_bytes.as_bytes())
    );
    assert_eq!(
        crate::domain::issue_automation::coordinator::start_decision_id(&issue(), &basis).as_str(),
        expected,
    );
    assert!(
        serde_json::to_value(definition())
            .unwrap()
            .get("deliveryReview")
            .is_none()
    );
}

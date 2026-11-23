use super::*;

fn basis() -> PlanningWorkPermitBasis {
    PlanningWorkPermitBasis {
        delivery_review: None,
        authorization_epoch: 0,
        policy_version: "policy-1".into(),
        field_hash: "field-1".into(),
        gate_revision: 1,
        source_sha: "a".repeat(40),
    }
}

fn record(stage: PlanningWorkPermitStage) -> PlanningWorkOrderRecord {
    PlanningWorkOrderRecord {
        link_reference: None,
        work_order_id: "order-1".into(),
        origin: "synthetic".into(),
        task_ids: vec!["task-1".into()],
        permits: vec![PlanningWorkPermit {
            work_order_id: "order-1".into(),
            stage,
            basis: basis(),
            requires_human_review: true,
        }],
    }
}

#[test]
fn work_permit_missing_stage_never_inherits_another_stage() {
    let record = record(PlanningWorkPermitStage::Dispatch);
    for stage in [
        PlanningWorkPermitStage::Push,
        PlanningWorkPermitStage::PullRequest,
        PlanningWorkPermitStage::Integrate,
    ] {
        assert_eq!(
            evaluate_work_order_permit(&record, Some(&basis()), stage, Some(&basis().source_sha)),
            PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Missing)
        );
    }
}

#[test]
fn work_permit_compares_every_current_basis_field() {
    let record = record(PlanningWorkPermitStage::Integrate);
    let original = basis();
    let mut changed = Vec::new();
    let mut next = original.clone();
    next.policy_version = "policy-2".into();
    changed.push(next);
    let mut next = original.clone();
    next.field_hash = "field-2".into();
    changed.push(next);
    let mut next = original.clone();
    next.gate_revision += 1;
    changed.push(next);
    let mut next = original.clone();
    next.source_sha = "b".repeat(40);
    changed.push(next);
    for current in changed {
        assert_eq!(
            evaluate_work_order_permit(
                &record,
                Some(&current),
                PlanningWorkPermitStage::Integrate,
                Some(&original.source_sha)
            ),
            PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale)
        );
    }
    assert_eq!(
        evaluate_work_order_permit(
            &record,
            None,
            PlanningWorkPermitStage::Integrate,
            Some(&original.source_sha)
        ),
        PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale)
    );
}

#[test]
fn work_permit_delivery_requires_the_exact_live_source_sha() {
    let current = basis();
    for stage in PlanningWorkPermitStage::ALL {
        let record = record(stage);
        assert_eq!(
            evaluate_work_order_permit(&record, Some(&current), stage, Some(&"b".repeat(40))),
            PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale)
        );
        let decision =
            evaluate_work_order_permit(&record, Some(&current), stage, Some(&current.source_sha));
        assert!(decision.requires_human_review());
        assert_eq!(decision.blocked_reason(), None);
        if stage != PlanningWorkPermitStage::Dispatch {
            assert_eq!(
                evaluate_work_order_permit(&record, Some(&current), stage, None).blocked_reason(),
                Some("permit_stale")
            );
        }
    }
}

#[test]
fn work_permit_record_rejects_cross_order_duplicate_and_malformed_grants() {
    let mut value = record(PlanningWorkPermitStage::Dispatch);
    value.validate().unwrap();
    value.permits[0].work_order_id = "other-order".into();
    assert!(value.validate().is_err());
    value.permits[0].work_order_id = value.work_order_id.clone();
    value.permits.push(value.permits[0].clone());
    assert!(value.validate().is_err());
    value.permits.pop();
    value.permits[0].basis.source_sha = "HEAD".into();
    assert!(value.validate().is_err());
}

#[test]
fn work_permit_wire_format_keeps_bound_evidence_at_the_grant_level() {
    let original = record(PlanningWorkPermitStage::PullRequest);
    let encoded = serde_json::to_value(&original).unwrap();
    assert_eq!(encoded["permits"][0]["stage"], "pull_request");
    assert_eq!(encoded["permits"][0]["policy_version"], "policy-1");
    assert!(encoded["permits"][0].get("basis").is_none());
    assert_eq!(
        serde_json::from_value::<PlanningWorkOrderRecord>(encoded).unwrap(),
        original
    );
}

fn reviewed_basis() -> PlanningWorkPermitBasis {
    let mut value = basis();
    value.delivery_review = Some(PlanningWorkDeliveryReview {
        authority: None,
        base_commit_sha: "b".repeat(40),
        source_commit_sha: value.source_sha.clone(),
        gate_revision: value.gate_revision,
    });
    value
}

#[test]
fn issue_delivery_requires_review_while_dispatch_and_other_origins_keep_existing_grants() {
    for stage in PlanningWorkPermitStage::ALL {
        let mut issue = record(stage);
        issue.origin = "issue_automation".into();
        issue.validate().unwrap();
        let decision =
            evaluate_work_order_permit(&issue, Some(&basis()), stage, Some(&basis().source_sha));
        if stage == PlanningWorkPermitStage::Dispatch {
            assert!(matches!(
                decision,
                PlanningWorkPermitDecision::Granted { .. }
            ));
        } else {
            assert_eq!(
                decision,
                PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Missing)
            );
        }
        let ordinary = record(stage);
        assert!(matches!(
            evaluate_work_order_permit(&ordinary, Some(&basis()), stage, Some(&basis().source_sha)),
            PlanningWorkPermitDecision::Granted { .. },
        ));
    }
}

#[test]
fn reviewed_issue_permits_remain_bound_to_the_whole_current_evidence_and_source() {
    let current = reviewed_basis();
    for stage in [
        PlanningWorkPermitStage::Push,
        PlanningWorkPermitStage::PullRequest,
        PlanningWorkPermitStage::Integrate,
    ] {
        let mut issue = record(stage);
        issue.origin = "issue_automation".into();
        issue.permits[0].basis = current.clone();
        assert!(matches!(
            evaluate_work_order_permit(&issue, Some(&current), stage, Some(&current.source_sha)),
            PlanningWorkPermitDecision::Granted { .. },
        ));
        let mut other_base = current.clone();
        other_base.delivery_review.as_mut().unwrap().base_commit_sha = "c".repeat(40);
        assert_eq!(
            evaluate_work_order_permit(&issue, Some(&other_base), stage, Some(&current.source_sha)),
            PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale),
        );
        assert_eq!(
            evaluate_work_order_permit(&issue, Some(&current), stage, Some(&"c".repeat(40))),
            PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale),
        );
    }
}

#[test]
fn delivery_review_rejects_incomplete_oids_or_a_different_source_or_revision() {
    let original = reviewed_basis();
    original.validate().unwrap();
    let mut invalid = Vec::new();
    let mut value = original.clone();
    value.delivery_review.as_mut().unwrap().base_commit_sha = "HEAD".into();
    invalid.push(value);
    let mut value = original.clone();
    value.delivery_review.as_mut().unwrap().source_commit_sha = "c".repeat(40);
    invalid.push(value);
    let mut value = original.clone();
    value.delivery_review.as_mut().unwrap().gate_revision += 1;
    invalid.push(value);
    let mut value = original;
    value.delivery_review.as_mut().unwrap().base_commit_sha = "g".repeat(40);
    invalid.push(value);
    for value in invalid {
        assert!(value.validate().is_err());
    }
}

#[test]
fn old_payload_without_delivery_review_hydrates_and_reviewed_payload_roundtrips() {
    let legacy = serde_json::to_value(basis()).unwrap();
    assert!(legacy.get("delivery_review").is_none());
    let decoded: PlanningWorkPermitBasis = serde_json::from_value(legacy).unwrap();
    decoded.validate().unwrap();
    assert!(decoded.delivery_review.is_none());
    let reviewed = reviewed_basis();
    let encoded = serde_json::to_value(&reviewed).unwrap();
    assert_eq!(
        encoded["delivery_review"]["source_commit_sha"],
        reviewed.source_sha
    );
    assert_eq!(
        serde_json::from_value::<PlanningWorkPermitBasis>(encoded).unwrap(),
        reviewed
    );
}

mod auto_delivery;

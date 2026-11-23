use super::*;

fn automatic() -> PlanningWorkPermitBasis {
    let mut value = reviewed_basis();
    value.delivery_review.as_mut().unwrap().authority = Some(PlanningWorkDeliveryAuthority::Auto {
        merge_gate_revision: 7,
        source_repository: "owner/repo".into(),
    });
    value
}

#[test]
fn auto_review_preserves_legacy_wire_and_closes_new_authority_fields() {
    let legacy = reviewed_basis();
    let json = serde_json::to_value(&legacy).unwrap();
    assert!(json["delivery_review"].get("authority").is_none());
    assert_eq!(
        serde_json::from_value::<PlanningWorkPermitBasis>(json).unwrap(),
        legacy
    );
    let value = automatic();
    value.validate().unwrap();
    let mut encoded = serde_json::to_value(&value).unwrap();
    assert_eq!(
        serde_json::from_value::<PlanningWorkPermitBasis>(encoded.clone()).unwrap(),
        value
    );
    encoded["delivery_review"]["authority"]["auto"]["approved"] = serde_json::json!(true);
    assert!(serde_json::from_value::<PlanningWorkPermitBasis>(encoded).is_err());
    let mut human = serde_json::to_value(PlanningWorkDeliveryAuthority::HumanReview).unwrap();
    assert_eq!(human, "human_review");
    human = serde_json::json!({"human_review":{"approved":true}});
    assert!(serde_json::from_value::<PlanningWorkDeliveryAuthority>(human).is_err());
}

#[test]
fn issue_integration_without_human_review_requires_explicit_auto_authority() {
    for authority in [None, Some(PlanningWorkDeliveryAuthority::HumanReview)] {
        let mut current = reviewed_basis();
        current.delivery_review.as_mut().unwrap().authority = authority;
        for stage in [
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
            PlanningWorkPermitStage::Integrate,
        ] {
            let mut work = record(stage);
            work.origin = "issue_automation".into();
            work.permits[0].basis = current.clone();
            work.permits[0].requires_human_review = false;
            let decision =
                evaluate_work_order_permit(&work, Some(&current), stage, Some(&current.source_sha));
            if stage == PlanningWorkPermitStage::Integrate {
                assert_eq!(
                    decision,
                    PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Missing)
                );
                work.permits[0].requires_human_review = true;
                assert!(
                    evaluate_work_order_permit(
                        &work,
                        Some(&current),
                        stage,
                        Some(&current.source_sha)
                    )
                    .requires_human_review()
                );
            } else {
                assert_eq!(
                    decision,
                    PlanningWorkPermitDecision::Granted {
                        requires_human_review: false
                    }
                );
            }
        }
    }
}

#[test]
fn auto_decision_is_explicit_for_every_delivery_stage_and_cannot_be_a_dispatch_grant() {
    let current = automatic();
    for stage in PlanningWorkPermitStage::ALL {
        let mut work = record(stage);
        work.origin = "issue_automation".into();
        work.permits[0].basis = current.clone();
        work.permits[0].requires_human_review = false;
        let decision =
            evaluate_work_order_permit(&work, Some(&current), stage, Some(&current.source_sha));
        if stage == PlanningWorkPermitStage::Dispatch {
            assert!(work.permits[0].validate().is_err());
            assert_eq!(
                decision,
                PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale)
            );
        } else {
            assert_eq!(decision, PlanningWorkPermitDecision::GrantedAuto);
            assert!(decision.requires_auto_delivery());
            assert!(!decision.requires_human_review());
            assert!(decision.blocked_reason().is_none());
            work.permits[0].requires_human_review = true;
            assert!(work.permits[0].validate().is_err());
        }
    }
}

#[test]
fn auto_merge_revision_and_repository_are_part_of_exact_basis_equality() {
    let current = automatic();
    let mut work = record(PlanningWorkPermitStage::Integrate);
    work.origin = "issue_automation".into();
    work.permits[0].basis = current.clone();
    work.permits[0].requires_human_review = false;
    for authority in [
        PlanningWorkDeliveryAuthority::HumanReview,
        PlanningWorkDeliveryAuthority::Auto {
            merge_gate_revision: 8,
            source_repository: "owner/repo".into(),
        },
        PlanningWorkDeliveryAuthority::Auto {
            merge_gate_revision: 7,
            source_repository: "owner/other".into(),
        },
    ] {
        let mut changed = current.clone();
        changed.delivery_review.as_mut().unwrap().authority = Some(authority);
        assert_eq!(
            evaluate_work_order_permit(
                &work,
                Some(&changed),
                PlanningWorkPermitStage::Integrate,
                Some(&current.source_sha)
            ),
            PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale)
        );
    }
    for repository in [
        "",
        "owner",
        "owner/repo/extra",
        "owner/..",
        "owner/repo name",
        "owner/repo\n",
        "https://github.com/owner/repo",
    ] {
        let value = PlanningWorkDeliveryAuthority::Auto {
            merge_gate_revision: 7,
            source_repository: repository.into(),
        };
        assert!(value.validate().is_err(), "{repository:?}");
    }
}

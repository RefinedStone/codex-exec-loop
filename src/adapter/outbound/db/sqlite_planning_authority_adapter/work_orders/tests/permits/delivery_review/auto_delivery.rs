use super::*;
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkDeliveryAuthority;

fn automatic_basis(revision: u64) -> PlanningWorkPermitBasis {
    let mut current = reviewed_basis(revision);
    current.delivery_review.as_mut().unwrap().authority =
        Some(PlanningWorkDeliveryAuthority::Auto {
            merge_gate_revision: 7,
            source_repository: "owner/repo".into(),
        });
    current
}

#[test]
fn issue_non_human_integration_requires_explicit_auto_at_both_grant_and_reopened_evaluation() {
    for explicit_human in [false, true] {
        let (workspace, adapter) = issue_fixture(&format!("auto-explicit-grant-{explicit_human}"));
        let mut current = reviewed_basis(1);
        if explicit_human {
            current.delivery_review.as_mut().unwrap().authority =
                Some(PlanningWorkDeliveryAuthority::HumanReview);
        }
        adapter
            .set_work_order_permit_basis(&workspace, "order-1", &current)
            .unwrap();
        for stage in [
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
        ] {
            adapter
                .grant_work_order_permit(
                    &workspace,
                    &PlanningWorkPermit {
                        basis: current.clone(),
                        requires_human_review: false,
                        ..permit(stage, 1)
                    },
                )
                .unwrap();
        }
        let invalid = PlanningWorkPermit {
            basis: current.clone(),
            requires_human_review: false,
            ..permit(PlanningWorkPermitStage::Integrate, 1)
        };
        let before = stored(&workspace, "order-1");
        assert!(
            adapter
                .grant_work_order_permit(&workspace, &invalid)
                .unwrap_err()
                .to_string()
                .contains("permit_missing")
        );
        assert_eq!(stored(&workspace, "order-1"), before);
        // An older binary could persist this boolean; hydration must never promote it to Auto.
        let mut legacy = before.permits;
        legacy.push(invalid.clone());
        connection(&workspace)
            .execute(
                "UPDATE planning_work_orders SET permits_json=?1 WHERE work_order_id='order-1'",
                [serde_json::to_string(&legacy).unwrap()],
            )
            .unwrap();
        let reopened = SqlitePlanningAuthorityAdapter::new();
        assert_eq!(
            check(
                &reopened,
                &workspace,
                PlanningWorkPermitStage::Integrate,
                Some(&current.source_sha)
            ),
            PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Missing)
        );
        assert!(
            reopened
                .grant_work_order_permit(&workspace, &invalid)
                .is_err()
        );
    }
}

#[test]
fn explicit_auto_persists_all_delivery_stages_and_changes_require_a_new_basis() {
    let (workspace, adapter) = issue_fixture("auto-persisted-delivery");
    let current = automatic_basis(1);
    adapter
        .set_work_order_permit_basis(&workspace, "order-1", &current)
        .unwrap();
    for stage in [
        PlanningWorkPermitStage::Push,
        PlanningWorkPermitStage::PullRequest,
        PlanningWorkPermitStage::Integrate,
    ] {
        let grant = PlanningWorkPermit {
            basis: current.clone(),
            requires_human_review: false,
            ..permit(stage, 1)
        };
        adapter.grant_work_order_permit(&workspace, &grant).unwrap();
        adapter.grant_work_order_permit(&workspace, &grant).unwrap();
    }
    let reopened = SqlitePlanningAuthorityAdapter::new();
    assert_eq!(
        reopened
            .load_work_order_permit_basis(&workspace, "order-1")
            .unwrap(),
        Some(current.clone())
    );
    for stage in [
        PlanningWorkPermitStage::Push,
        PlanningWorkPermitStage::PullRequest,
        PlanningWorkPermitStage::Integrate,
    ] {
        assert_eq!(
            check(&reopened, &workspace, stage, Some(&current.source_sha)),
            PlanningWorkPermitDecision::GrantedAuto
        );
    }
    let mut human = current.clone();
    human.delivery_review.as_mut().unwrap().authority =
        Some(PlanningWorkDeliveryAuthority::HumanReview);
    assert!(
        reopened
            .set_work_order_permit_basis(&workspace, "order-1", &human)
            .is_err()
    );
    human.gate_revision += 1;
    human.delivery_review.as_mut().unwrap().gate_revision = human.gate_revision;
    reopened
        .set_work_order_permit_basis(&workspace, "order-1", &human)
        .unwrap();
    assert_eq!(
        check(
            &reopened,
            &workspace,
            PlanningWorkPermitStage::Integrate,
            Some(&human.source_sha)
        ),
        PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale)
    );
    reopened
        .grant_work_order_permit(
            &workspace,
            &PlanningWorkPermit {
                basis: human.clone(),
                ..permit(PlanningWorkPermitStage::Integrate, 2)
            },
        )
        .unwrap();
    assert!(
        check(
            &reopened,
            &workspace,
            PlanningWorkPermitStage::Integrate,
            Some(&human.source_sha)
        )
        .requires_human_review()
    );
    assert!(
        reopened
            .grant_work_order_permit(
                &workspace,
                &PlanningWorkPermit {
                    basis: current,
                    requires_human_review: false,
                    ..permit(PlanningWorkPermitStage::Integrate, 1)
                }
            )
            .is_err()
    );
}

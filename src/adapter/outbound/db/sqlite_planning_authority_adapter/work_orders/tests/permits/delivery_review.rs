use super::super::{candidate, register_document};
use super::*;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkDeliveryReview, PlanningWorkOrderRepositoryPort,
};

fn issue_fixture(label: &str) -> (String, SqlitePlanningAuthorityAdapter) {
    let workspace = workspace(label);
    let before = seed(&workspace, vec![task("ordinary-task")]);
    register_document(
        &workspace,
        "order-1",
        "issue_automation",
        "controlled-task",
        before.planning_revision,
        &candidate(&before, "controlled-task"),
    )
    .unwrap();
    (workspace, SqlitePlanningAuthorityAdapter::new())
}

fn reviewed_basis(revision: u64) -> PlanningWorkPermitBasis {
    let mut value = basis(revision);
    value.delivery_review = Some(PlanningWorkDeliveryReview {
        base_commit_sha: "b".repeat(40),
        source_commit_sha: value.source_sha.clone(),
        gate_revision: revision,
    });
    value
}

fn reviewed_permit(stage: PlanningWorkPermitStage, revision: u64) -> PlanningWorkPermit {
    PlanningWorkPermit {
        basis: reviewed_basis(revision),
        ..permit(stage, revision)
    }
}

fn install_legacy_grants(workspace: &str, adapter: &SqlitePlanningAuthorityAdapter) {
    adapter
        .set_work_order_permit_basis(workspace, "order-1", &basis(1))
        .unwrap();
    let grants = PlanningWorkPermitStage::ALL.map(|stage| permit(stage, 1));
    let encoded = serde_json::to_value(&grants).unwrap();
    assert!(
        encoded
            .as_array()
            .unwrap()
            .iter()
            .all(|grant| grant.get("delivery_review").is_none())
    );
    // This is the old on-disk shape. The new grant boundary deliberately cannot create it.
    connection(workspace)
        .execute(
            "UPDATE planning_work_orders SET permits_json=?2 WHERE work_order_id=?1",
            rusqlite::params!["order-1", serde_json::to_string(&encoded).unwrap()],
        )
        .unwrap();
}

#[test]
fn legacy_issue_delivery_grants_hydrate_after_reopen_but_cannot_execute_or_be_regranted() {
    let (workspace, adapter) = issue_fixture("legacy-issue-delivery-review");
    install_legacy_grants(&workspace, &adapter);
    let original = stored(&workspace, "order-1");
    assert_eq!(original.permits.len(), 4);
    let reopened = SqlitePlanningAuthorityAdapter::new();
    let current = reopened
        .load_work_order_permit_basis(&workspace, "order-1")
        .unwrap()
        .unwrap();
    assert!(current.delivery_review.is_none());
    assert!(matches!(
        check(
            &reopened,
            &workspace,
            PlanningWorkPermitStage::Dispatch,
            Some(&current.source_sha)
        ),
        PlanningWorkPermitDecision::Granted { .. }
    ));
    for stage in [
        PlanningWorkPermitStage::Push,
        PlanningWorkPermitStage::PullRequest,
        PlanningWorkPermitStage::Integrate,
    ] {
        assert_eq!(
            check(&reopened, &workspace, stage, Some(&current.source_sha)),
            PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Missing)
        );
        let error = reopened
            .grant_work_order_permit(&workspace, &permit(stage, 1))
            .unwrap_err();
        assert!(error.to_string().contains("permit_missing"));
    }
    assert_eq!(stored(&workspace, "order-1"), original);
}

#[test]
fn review_evidence_allows_the_exact_source_but_preserves_the_revocation_fence() {
    let (workspace, adapter) = issue_fixture("reviewed-issue-delivery");
    adapter
        .set_work_order_permit_basis(&workspace, "order-1", &reviewed_basis(1))
        .unwrap();
    let push = reviewed_permit(PlanningWorkPermitStage::Push, 1);
    adapter.grant_work_order_permit(&workspace, &push).unwrap();
    assert!(matches!(
        check(
            &adapter,
            &workspace,
            PlanningWorkPermitStage::Push,
            Some(&push.basis.source_sha)
        ),
        PlanningWorkPermitDecision::Granted { .. }
    ));
    assert_eq!(
        check(
            &adapter,
            &workspace,
            PlanningWorkPermitStage::Push,
            Some(&"c".repeat(40))
        ),
        PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale)
    );
    adapter
        .revoke_work_order_permits(
            &workspace,
            "order-1",
            &[PlanningWorkPermitStage::Push],
            "review revoked",
        )
        .unwrap();
    assert!(adapter.grant_work_order_permit(&workspace, &push).is_err());
    adapter
        .set_work_order_permit_basis(&workspace, "order-1", &reviewed_basis(2))
        .unwrap();
    adapter
        .grant_work_order_permit(
            &workspace,
            &reviewed_permit(PlanningWorkPermitStage::Push, 2),
        )
        .unwrap();
    let reopened = SqlitePlanningAuthorityAdapter::new();
    assert!(matches!(
        check(
            &reopened,
            &workspace,
            PlanningWorkPermitStage::Push,
            Some(&push.basis.source_sha)
        ),
        PlanningWorkPermitDecision::Granted { .. }
    ));
}

#[test]
fn new_review_does_not_implicitly_upgrade_a_retained_legacy_grant() {
    let (workspace, adapter) = issue_fixture("legacy-issue-review-refresh");
    install_legacy_grants(&workspace, &adapter);
    adapter
        .set_work_order_permit_basis(&workspace, "order-1", &reviewed_basis(2))
        .unwrap();
    for stage in [
        PlanningWorkPermitStage::Push,
        PlanningWorkPermitStage::PullRequest,
        PlanningWorkPermitStage::Integrate,
    ] {
        assert_eq!(
            check(&adapter, &workspace, stage, Some(&basis(1).source_sha)),
            PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale)
        );
        adapter
            .grant_work_order_permit(&workspace, &reviewed_permit(stage, 2))
            .unwrap();
        assert!(matches!(
            check(&adapter, &workspace, stage, Some(&basis(1).source_sha)),
            PlanningWorkPermitDecision::Granted { .. }
        ));
    }
}

#[test]
fn a_new_lineage_removes_review_before_advancing_the_permit_revision() {
    let (workspace, adapter) = issue_fixture("issue-review-new-lineage");
    adapter
        .set_work_order_permit_basis(&workspace, "order-1", &reviewed_basis(1))
        .unwrap();
    adapter
        .grant_work_order_permit(
            &workspace,
            &reviewed_permit(PlanningWorkPermitStage::Push, 1),
        )
        .unwrap();
    adapter
        .link_supplemental_task(&workspace, "order-1", "ordinary-task")
        .unwrap();
    let current = adapter
        .load_work_order_permit_basis(&workspace, "order-1")
        .unwrap()
        .unwrap();
    current.validate().unwrap();
    assert_eq!(current.gate_revision, 2);
    assert!(current.delivery_review.is_none());
    assert!(stored(&workspace, "order-1").permits.is_empty());
    let unreviewed = PlanningWorkPermit {
        basis: current,
        ..permit(PlanningWorkPermitStage::Push, 2)
    };
    assert!(
        adapter
            .grant_work_order_permit(&workspace, &unreviewed)
            .unwrap_err()
            .to_string()
            .contains("permit_missing")
    );
}

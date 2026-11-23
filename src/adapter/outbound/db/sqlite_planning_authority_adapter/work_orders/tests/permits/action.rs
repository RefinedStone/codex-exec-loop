use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControlAction, PlanningWorkControlRepositoryPort,
};

#[test]
fn final_start_requires_the_claimed_seal_and_exact_current_lease_without_reclaiming() {
    let (workspace, adapter) = fixture("binding-final-start-seal");
    grant_all(&adapter, &workspace, 1);
    let expected = lease("controlled-task", "slot-1");
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &expected).unwrap();
    let identity = adapter
        .worker_action_identity_for_task(&workspace, "controlled-task")
        .unwrap()
        .unwrap();
    assert_eq!(identity.work_order_id, "order-1");
    assert_eq!(identity.authorization_epoch, 0);
    assert!(
        adapter
            .validate_work_order_launch_start(&workspace, &expected, Some(&identity))
            .is_err()
    );
    adapter
        .claim_work_order_launch(&workspace, &expected)
        .unwrap();
    let before = count(&workspace, "planning_work_order_launch_seals");
    for _ in 0..3 {
        adapter
            .validate_work_order_launch_start(&workspace, &expected, Some(&identity))
            .unwrap();
    }
    assert_eq!(
        count(&workspace, "planning_work_order_launch_seals"),
        before
    );
    let next = expected.clone().with_lease_generation("c".repeat(64));
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &next).unwrap();
    assert!(
        adapter
            .validate_work_order_launch_start(&workspace, &expected, Some(&identity))
            .is_err()
    );
    assert!(
        adapter
            .validate_work_order_launch_start(&workspace, &next, Some(&identity))
            .is_err()
    );
    assert!(
        adapter
            .worker_action_identity_for_task(&workspace, "ordinary-task")
            .unwrap()
            .is_none()
    );
}

#[test]
fn renewed_permissions_do_not_revive_a_delayed_callback_with_the_old_authorization_epoch() {
    let (workspace, adapter) = fixture("binding-final-start-epoch");
    grant_all(&adapter, &workspace, 1);
    let expected = lease("controlled-task", "slot-1");
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &expected).unwrap();
    let identity = adapter
        .worker_action_identity_for_task(&workspace, "controlled-task")
        .unwrap()
        .unwrap();
    adapter
        .claim_work_order_launch(&workspace, &expected)
        .unwrap();
    adapter
        .apply_work_order_control(
            &workspace,
            "order-1",
            "pause-fence",
            PlanningWorkControlAction::Hold,
        )
        .unwrap();
    adapter
        .apply_work_order_control(
            &workspace,
            "order-1",
            "resume-fence",
            PlanningWorkControlAction::Release,
        )
        .unwrap();
    let renewed = PlanningWorkPermitBasis {
        authorization_epoch: 2,
        ..basis(2)
    };
    adapter
        .set_work_order_permit_basis(&workspace, "order-1", &renewed)
        .unwrap();
    adapter
        .grant_work_order_permit(
            &workspace,
            &PlanningWorkPermit {
                basis: renewed,
                ..permit(PlanningWorkPermitStage::Dispatch, 2)
            },
        )
        .unwrap();
    assert!(
        adapter
            .validate_work_order_launch_start(&workspace, &expected, Some(&identity))
            .is_err()
    );
    assert_eq!(
        adapter
            .worker_action_identity_for_task(&workspace, "controlled-task")
            .unwrap()
            .unwrap()
            .authorization_epoch,
        2
    );
}

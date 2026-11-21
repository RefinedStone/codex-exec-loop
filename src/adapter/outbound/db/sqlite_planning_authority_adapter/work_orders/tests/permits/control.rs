use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControlAction, PlanningWorkControlRepositoryPort, PlanningWorkInstruction,
    PlanningWorkInstructionAdmission, PlanningWorkOrderRepositoryPort,
};
use crate::domain::planning::TaskStatus;

#[test]
fn work_control_hold_and_release_fence_old_approval_even_after_source_change() {
    let (workspace, adapter) = fixture("control-hold-release");
    grant_all(&adapter, &workspace, 1);
    let held = adapter
        .apply_work_order_control(
            &workspace,
            "order-1",
            "hold-1",
            PlanningWorkControlAction::Hold,
        )
        .unwrap();
    assert_eq!(held.authorization_epoch, 1);
    assert!(held.held);
    for stage in PlanningWorkPermitStage::ALL {
        assert!(
            check(&adapter, &workspace, stage, Some(&basis(1).source_sha))
                .blocked_reason()
                .is_some()
        );
    }
    let mut changed = basis(500);
    changed.source_sha = "b".repeat(40);
    assert!(
        adapter
            .set_work_order_permit_basis(&workspace, "order-1", &changed)
            .is_err()
    );
    assert!(
        SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(
            &workspace,
            &lease("controlled-task", "slot-1")
        )
        .is_err()
    );
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(
        &workspace,
        &lease("ordinary-task", "slot-2"),
    )
    .unwrap();

    let released = adapter
        .apply_work_order_control(
            &workspace,
            "order-1",
            "release-1",
            PlanningWorkControlAction::Release,
        )
        .unwrap();
    assert_eq!(released.authorization_epoch, 2);
    assert!(released.allows_automation());
    assert!(
        adapter
            .grant_work_order_permit(&workspace, &permit(PlanningWorkPermitStage::Dispatch, 1))
            .is_err()
    );
    assert!(
        adapter
            .set_work_order_permit_basis(&workspace, "order-1", &changed)
            .is_err()
    );
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
        check(
            &adapter,
            &workspace,
            PlanningWorkPermitStage::Dispatch,
            Some(&basis(1).source_sha)
        )
        .blocked_reason()
        .is_none()
    );
    assert_eq!(
        adapter
            .apply_work_order_control(
                &workspace,
                "order-1",
                "hold-1",
                PlanningWorkControlAction::Hold
            )
            .unwrap(),
        held
    );
    assert_eq!(
        adapter
            .load_work_order_control(&workspace, "order-1")
            .unwrap(),
        released
    );
    assert!(
        adapter
            .apply_work_order_control(
                &workspace,
                "order-1",
                "hold-1",
                PlanningWorkControlAction::Cancel
            )
            .is_err()
    );
}

#[test]
fn work_control_takeover_preserves_lease_and_disables_cleanup_after_restart() {
    let (workspace, adapter) = fixture("control-takeover");
    grant_all(&adapter, &workspace, 1);
    let lease = lease("controlled-task", "slot-1");
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &lease).unwrap();
    adapter
        .apply_work_order_control(
            &workspace,
            "order-1",
            "takeover",
            PlanningWorkControlAction::TakeOver,
        )
        .unwrap();
    let restarted = SqlitePlanningAuthorityAdapter::new();
    assert_eq!(
        restarted
            .load_runtime_projections(&workspace)
            .unwrap()
            .slot_leases
            .get("slot-1"),
        Some(&lease)
    );
    assert!(
        !PlanningWorkControlRepositoryPort::work_order_cleanup_allowed_for_task(
            &restarted,
            &workspace,
            "controlled-task"
        )
        .unwrap()
    );
    adapter
        .apply_work_order_control(
            &workspace,
            "order-1",
            "untrack",
            PlanningWorkControlAction::Untrack,
        )
        .unwrap();
    adapter
        .apply_work_order_control(
            &workspace,
            "order-1",
            "track",
            PlanningWorkControlAction::Track,
        )
        .unwrap();
    assert!(
        adapter
            .load_work_order_control(&workspace, "order-1")
            .unwrap()
            .taken_over
    );
    assert!(
        !PlanningWorkControlRepositoryPort::work_order_cleanup_allowed_for_task(
            &adapter,
            &workspace,
            "controlled-task"
        )
        .unwrap()
    );
}

#[test]
fn work_control_untrack_preserves_cleanup_ownership_without_takeover_after_restart() {
    let (workspace, adapter) = fixture("control-untrack-cleanup");
    grant_all(&adapter, &workspace, 1);
    let lease = lease("controlled-task", "slot-1");
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &lease).unwrap();
    let untracked = adapter
        .apply_work_order_control(
            &workspace,
            "order-1",
            "untrack",
            PlanningWorkControlAction::Untrack,
        )
        .unwrap();
    assert!(!untracked.tracked);
    assert!(!untracked.taken_over);

    let restarted = SqlitePlanningAuthorityAdapter::new();
    assert_eq!(
        restarted
            .load_work_order_control(&workspace, "order-1")
            .unwrap(),
        untracked
    );
    assert!(
        !PlanningWorkControlRepositoryPort::work_order_cleanup_allowed_for_task(
            &restarted,
            &workspace,
            "controlled-task",
        )
        .unwrap()
    );
    assert!(
        PlanningWorkControlRepositoryPort::work_order_cleanup_allowed_for_task(
            &restarted,
            &workspace,
            "ordinary-task",
        )
        .unwrap()
    );
    assert_eq!(
        restarted
            .load_runtime_projections(&workspace)
            .unwrap()
            .slot_leases
            .get("slot-1"),
        Some(&lease)
    );

    let tracked = restarted
        .apply_work_order_control(
            &workspace,
            "order-1",
            "track",
            PlanningWorkControlAction::Track,
        )
        .unwrap();
    assert!(tracked.tracked);
    assert!(!tracked.taken_over);
    assert_eq!(
        tracked.authorization_epoch,
        untracked.authorization_epoch + 1
    );
    assert!(
        PlanningWorkControlRepositoryPort::work_order_cleanup_allowed_for_task(
            &restarted,
            &workspace,
            "controlled-task",
        )
        .unwrap()
    );
    assert!(stored(&workspace, "order-1").permits.is_empty());
    for stage in PlanningWorkPermitStage::ALL {
        assert!(
            check(&restarted, &workspace, stage, Some(&basis(1).source_sha))
                .blocked_reason()
                .is_some()
        );
    }
    assert_eq!(
        restarted
            .load_runtime_projections(&workspace)
            .unwrap()
            .slot_leases
            .get("slot-1"),
        Some(&lease)
    );
}

#[test]
fn work_control_cancel_covers_current_and_late_supplemental_lineage_only() {
    let workspace = workspace("control-cancel-lineage");
    let mut running = task("supplement");
    running.status = TaskStatus::InProgress;
    seed(&workspace, vec![task("unrelated"), running, task("late")]);
    register_next(&workspace, "order-1", "controlled-task");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .link_supplemental_task(&workspace, "order-1", "supplement")
        .unwrap();
    adapter
        .apply_work_order_control(
            &workspace,
            "order-1",
            "cancel",
            PlanningWorkControlAction::Cancel,
        )
        .unwrap();
    adapter
        .link_supplemental_task(&workspace, "order-1", "late")
        .unwrap();
    let tasks = SqlitePlanningAuthorityAdapter::load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap()
        .task_authority
        .tasks;
    for task in tasks {
        let expected = if task.id == "unrelated" {
            TaskStatus::Ready
        } else {
            TaskStatus::Cancelled
        };
        assert_eq!(task.status, expected, "{}", task.id);
    }
    assert!(
        adapter
            .set_work_order_permit_basis(&workspace, "order-1", &basis(1000))
            .is_err()
    );
}

#[test]
fn work_control_queued_instruction_is_applied_exactly_once_to_future_registration() {
    let workspace = workspace("control-instruction-before-registration");
    seed(&workspace, vec![task("unrelated")]);
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let instruction = PlanningWorkInstruction {
        command_id: "steer-1".into(),
        message: "추가 검증을 수행하세요.".into(),
    };
    assert_eq!(
        adapter
            .append_work_order_instruction(&workspace, "order-1", &instruction)
            .unwrap(),
        PlanningWorkInstructionAdmission::Queued { changed: true }
    );
    register_next(&workspace, "order-1", "controlled-task");
    assert_eq!(
        adapter
            .append_work_order_instruction(&workspace, "order-1", &instruction)
            .unwrap(),
        PlanningWorkInstructionAdmission::Queued { changed: false }
    );
    let tasks = SqlitePlanningAuthorityAdapter::load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap()
        .task_authority
        .tasks;
    let controlled = tasks
        .iter()
        .find(|task| task.id == "controlled-task")
        .unwrap();
    assert_eq!(
        controlled.description.matches(&instruction.message).count(),
        1
    );
    assert!(
        !tasks
            .iter()
            .find(|task| task.id == "unrelated")
            .unwrap()
            .description
            .contains(&instruction.message)
    );
    assert!(
        adapter
            .append_work_order_instruction(
                &workspace,
                "order-1",
                &PlanningWorkInstruction {
                    message: "다른 글".into(),
                    ..instruction
                }
            )
            .is_err()
    );
}

#[test]
fn work_control_launch_seal_separates_waiting_instructions_from_live_steering() {
    let (workspace, adapter) = fixture("control-launch-seal");
    grant_all(&adapter, &workspace, 1);
    let first = lease("controlled-task", "slot-1");
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &first).unwrap();
    let queued = PlanningWorkInstruction {
        command_id: "before-seal".into(),
        message: "실행 전 추가 지시".into(),
    };
    assert_eq!(
        adapter
            .append_work_order_instruction(&workspace, "order-1", &queued)
            .unwrap(),
        PlanningWorkInstructionAdmission::Queued { changed: true }
    );
    assert_eq!(
        adapter.claim_work_order_launch(&workspace, &first).unwrap(),
        vec![queued.clone()]
    );
    assert_eq!(
        adapter.claim_work_order_launch(&workspace,&first).unwrap_err().downcast_ref::<
            crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderLaunchError
        >(),
        Some(&crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderLaunchError::AlreadyClaimed),
    );
    let after = PlanningWorkInstruction {
        command_id: "after-seal".into(),
        message: "실행 중 추가 지시".into(),
    };
    assert_eq!(
        adapter
            .append_work_order_instruction(&workspace, "order-1", &after)
            .unwrap(),
        PlanningWorkInstructionAdmission::Active
    );
    assert_eq!(
        adapter
            .load_work_order_instructions(&workspace, "order-1")
            .unwrap(),
        vec![queued]
    );
    let next = first.clone().with_lease_generation("c".repeat(64));
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &next).unwrap();
    assert_eq!(
        adapter
            .append_work_order_instruction(&workspace, "order-1", &after)
            .unwrap(),
        PlanningWorkInstructionAdmission::Queued { changed: true }
    );
    assert!(adapter.claim_work_order_launch(&workspace, &first).is_err());
    assert_eq!(
        adapter
            .claim_work_order_launch(&workspace, &next)
            .unwrap()
            .len(),
        2
    );
    adapter
        .apply_work_order_control(
            &workspace,
            "order-1",
            "hold-sealed",
            PlanningWorkControlAction::Hold,
        )
        .unwrap();
    assert!(adapter.claim_work_order_launch(&workspace, &next).is_err());
    adapter
        .apply_work_order_control(
            &workspace,
            "order-1",
            "release-sealed",
            PlanningWorkControlAction::Release,
        )
        .unwrap();
    assert!(adapter.claim_work_order_launch(&workspace, &next).is_err());
}

#[test]
fn work_control_audit_failure_rolls_back_fence_and_revocation() {
    let (workspace, adapter) = fixture("control-audit-failure");
    grant_all(&adapter, &workspace, 1);
    let db = connection(&workspace);
    db.execute_batch("CREATE TRIGGER fail_control_audit BEFORE INSERT ON runtime_events BEGIN SELECT RAISE(ABORT,'synthetic audit failure'); END;").unwrap();
    assert!(
        adapter
            .apply_work_order_control(
                &workspace,
                "order-1",
                "hold",
                PlanningWorkControlAction::Hold
            )
            .is_err()
    );
    assert_eq!(
        adapter
            .load_work_order_control(&workspace, "order-1")
            .unwrap()
            .authorization_epoch,
        0
    );
    assert_eq!(stored(&workspace, "order-1").permits.len(), 4);
    assert_eq!(count(&workspace, "planning_work_order_control_commands"), 0);
}

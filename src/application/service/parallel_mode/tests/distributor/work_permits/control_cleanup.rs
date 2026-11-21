use super::*;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControlAction, PlanningWorkControlRepositoryPort,
};

fn unstarted_fixture(
    prefix: &str,
) -> (
    TempGitRepo,
    ParallelModeService,
    ParallelModeSlotLeaseSnapshot,
    PlanningWorkPermitBasis,
) {
    let repo = TempGitRepo::new(prefix);
    register_delivery_work_order(&repo);
    let basis = PlanningWorkPermitBasis {
        authorization_epoch: 0,
        policy_version: "control-policy".to_string(),
        field_hash: "control-fields".to_string(),
        gate_revision: 1,
        source_sha: repo.head_sha(),
    };
    set_basis_and_grant(
        &repo.workspace_dir(),
        &basis,
        &[PlanningWorkPermitStage::Dispatch],
        false,
    );
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request(TASK_ID, "Task One", "agent-1", "task-one"),
        )
        .unwrap();
    (repo, service, lease, basis)
}

#[test]
fn work_control_unstarted_hold_releases_without_startup_failure_and_fresh_approval_can_redispatch()
{
    let (repo, service, first, basis) = unstarted_fixture("control-release-fresh-approval");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .apply_work_order_control(
            &repo.workspace_dir(),
            WORK_ORDER_ID,
            "hold",
            PlanningWorkControlAction::Hold,
        )
        .unwrap();
    assert!(
        service
            .release_unstarted_work_order_lease_for_control(&first)
            .unwrap()
            .is_some()
    );
    assert!(!repo.branch_exists(&first.branch_name));
    let projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir()).unwrap();
    assert!(projection.task_dispatch_blocks.is_empty());
    assert!(!projection.slot_leases.contains_key(&first.slot_id));

    let control = adapter
        .apply_work_order_control(
            &repo.workspace_dir(),
            WORK_ORDER_ID,
            "release",
            PlanningWorkControlAction::Release,
        )
        .unwrap();
    let refreshed = PlanningWorkPermitBasis {
        authorization_epoch: control.authorization_epoch,
        gate_revision: 2,
        ..basis
    };
    set_basis_and_grant(
        &repo.workspace_dir(),
        &refreshed,
        &[PlanningWorkPermitStage::Dispatch],
        false,
    );
    let next = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request(TASK_ID, "Task One", "agent-1", "task-one"),
        )
        .unwrap();
    assert_eq!(first.slot_id, next.slot_id);
    assert!(!first.same_generation_as(&next));
    assert!(
        service
            .release_unstarted_work_order_lease_for_control(&first)
            .unwrap()
            .is_none()
    );
    service
        .mark_slot_running(&repo.workspace_dir(), &next.slot_id, &next.agent_id)
        .unwrap();
    adapter
        .apply_work_order_control(
            &repo.workspace_dir(),
            WORK_ORDER_ID,
            "hold-running",
            PlanningWorkControlAction::Hold,
        )
        .unwrap();
    assert!(
        service
            .release_unstarted_work_order_lease_for_control(&next)
            .unwrap()
            .is_none()
    );
    let projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir()).unwrap();
    assert_eq!(
        projection.slot_leases[&next.slot_id].state,
        ParallelModeSlotLeaseState::Running
    );
    assert!(projection.task_dispatch_blocks.is_empty());
    assert!(repo.branch_exists(&next.branch_name));
}

#[test]
fn work_control_unstarted_takeover_preserves_slot_branch_and_worktree() {
    let (repo, service, mut lease, _) = unstarted_fixture("control-takeover-unstarted");
    persist_exited_owner(&repo, &mut lease);
    SqlitePlanningAuthorityAdapter::new()
        .apply_work_order_control(
            &repo.workspace_dir(),
            WORK_ORDER_ID,
            "take-over",
            PlanningWorkControlAction::TakeOver,
        )
        .unwrap();
    assert!(
        service
            .release_unstarted_work_order_lease_for_control(&lease)
            .unwrap()
            .is_none()
    );
    reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir()).unwrap();
    assert_eq!(projection.slot_leases.get(&lease.slot_id), Some(&lease));
    assert!(projection.task_dispatch_blocks.is_empty());
    assert!(repo.branch_exists(&lease.branch_name));
    assert!(Path::new(&lease.worktree_path).is_dir());
}

#[test]
fn work_control_unstarted_hold_preserves_dirty_work_without_a_startup_failure_block() {
    let (repo, service, lease, _) = unstarted_fixture("control-hold-dirty-unstarted");
    fs::write(
        Path::new(&lease.worktree_path).join("operator.txt"),
        "preserve this work
",
    )
    .unwrap();
    SqlitePlanningAuthorityAdapter::new()
        .apply_work_order_control(
            &repo.workspace_dir(),
            WORK_ORDER_ID,
            "hold",
            PlanningWorkControlAction::Hold,
        )
        .unwrap();
    assert!(
        service
            .release_unstarted_work_order_lease_for_control(&lease)
            .is_err()
    );
    let projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir()).unwrap();
    assert_eq!(projection.slot_leases.get(&lease.slot_id), Some(&lease));
    assert!(projection.task_dispatch_blocks.is_empty());
    assert_eq!(
        fs::read_to_string(Path::new(&lease.worktree_path).join("operator.txt")).unwrap(),
        "preserve this work
"
    );
}

fn persist_exited_owner(repo: &TempGitRepo, lease: &mut ParallelModeSlotLeaseSnapshot) {
    lease.owner_process = Some(
        crate::domain::parallel_mode::ParallelModeSlotLeaseOwnerProcess {
            process_id: std::process::id(),
            start_identity: "exited-control-owner".into(),
        },
    );
    crate::application::service::parallel_mode::write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        lease,
    )
    .unwrap();
}

#[test]
fn work_control_stale_startup_recovers_hold_without_permanently_blocking_later_approval() {
    for release_before_restart in [false, true] {
        let (repo, service, mut lease, basis) = unstarted_fixture(&format!(
            "control-stale-policy-stop-{release_before_restart}"
        ));
        let adapter = SqlitePlanningAuthorityAdapter::new();
        persist_exited_owner(&repo, &mut lease);
        adapter
            .apply_work_order_control(
                &repo.workspace_dir(),
                WORK_ORDER_ID,
                "hold",
                PlanningWorkControlAction::Hold,
            )
            .unwrap();
        if release_before_restart {
            adapter
                .apply_work_order_control(
                    &repo.workspace_dir(),
                    WORK_ORDER_ID,
                    "release",
                    PlanningWorkControlAction::Release,
                )
                .unwrap();
        }
        reconcile_pool_board(&adapter, &test_parallel_runtime(), &repo.workspace_dir());
        let projection =
            SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
                .unwrap();
        assert!(projection.task_dispatch_blocks.is_empty());
        assert!(!projection.slot_leases.contains_key(&lease.slot_id));
        assert!(!repo.branch_exists(&lease.branch_name));
        assert!(
            projection
                .session_details
                .iter()
                .all(|detail| detail.state_label != "failed")
        );
        if !release_before_restart {
            adapter
                .apply_work_order_control(
                    &repo.workspace_dir(),
                    WORK_ORDER_ID,
                    "release",
                    PlanningWorkControlAction::Release,
                )
                .unwrap();
        }
        let control = adapter
            .load_work_order_control(&repo.workspace_dir(), WORK_ORDER_ID)
            .unwrap();
        let refreshed = PlanningWorkPermitBasis {
            authorization_epoch: control.authorization_epoch,
            gate_revision: 2,
            ..basis
        };
        set_basis_and_grant(
            &repo.workspace_dir(),
            &refreshed,
            &[PlanningWorkPermitStage::Dispatch],
            false,
        );
        let resumed = service
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request(TASK_ID, "Task One", "agent-1", "task-one"),
            )
            .unwrap();
        assert_eq!(resumed.slot_id, lease.slot_id);
        assert!(!resumed.same_generation_as(&lease));
    }
}

#[test]
fn work_control_untrack_preserves_clean_integrated_cleanup_pending_slot_after_restart() {
    let (repo, _, mut lease, _) = unstarted_fixture("control-untrack-cleanup-pending");
    lease.state = ParallelModeSlotLeaseState::CleanupPending;
    crate::application::service::parallel_mode::write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .unwrap();
    let mirror_before = fs::read(repo.slot_lease_path(1)).unwrap();
    let untracked = SqlitePlanningAuthorityAdapter::new()
        .apply_work_order_control(
            &repo.workspace_dir(),
            WORK_ORDER_ID,
            "untrack",
            PlanningWorkControlAction::Untrack,
        )
        .unwrap();
    assert!(!untracked.tracked);
    assert!(!untracked.taken_over);

    let restarted = SqlitePlanningAuthorityAdapter::new();
    for _ in 0..2 {
        reconcile_pool_board(&restarted, &test_parallel_runtime(), &repo.workspace_dir());
        let projection =
            SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
                .unwrap();
        assert_eq!(projection.slot_leases.get(&lease.slot_id), Some(&lease));
        assert_eq!(fs::read(repo.slot_lease_path(1)).unwrap(), mirror_before);
        assert!(repo.branch_exists(&lease.branch_name));
        assert!(Path::new(&lease.worktree_path).is_dir());
        assert!(projection.task_dispatch_blocks.is_empty());
    }

    // 같은 깨끗한 통합 슬롯은 명시적으로 다시 추적한 뒤 정상 정리할 수 있다.
    restarted
        .apply_work_order_control(
            &repo.workspace_dir(),
            WORK_ORDER_ID,
            "track",
            PlanningWorkControlAction::Track,
        )
        .unwrap();
    reconcile_pool_board(&restarted, &test_parallel_runtime(), &repo.workspace_dir());
    let projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir()).unwrap();
    assert!(!projection.slot_leases.contains_key(&lease.slot_id));
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.branch_exists(&lease.branch_name));
}

#[test]
fn work_control_takeover_preserves_a_branchless_cleanup_pending_lease_after_restart() {
    let (repo, _, mut lease, basis) = unstarted_fixture("control-takeover-branchless");
    lease.state = ParallelModeSlotLeaseState::CleanupPending;
    crate::application::service::parallel_mode::write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .unwrap();
    run_git(
        Path::new(&lease.worktree_path),
        &["checkout", "--detach", &basis.source_sha],
    );
    run_git(&repo.repo_root, &["branch", "-D", &lease.branch_name]);
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .apply_work_order_control(
            &repo.workspace_dir(),
            WORK_ORDER_ID,
            "take-over",
            PlanningWorkControlAction::TakeOver,
        )
        .unwrap();
    reconcile_pool_board(&adapter, &test_parallel_runtime(), &repo.workspace_dir());
    let projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir()).unwrap();
    assert_eq!(projection.slot_leases.get(&lease.slot_id), Some(&lease));
    assert!(repo.slot_lease_path(1).is_file());
    assert!(Path::new(&lease.worktree_path).is_dir());
    assert!(projection.task_dispatch_blocks.is_empty());
}

use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkPermit, PlanningWorkPermitBasis, PlanningWorkPermitBlockReason,
    PlanningWorkPermitDecision, PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
};
use crate::domain::parallel_mode::{
    ParallelModeDeliveryTargetSnapshot, ParallelModeRepositoryVisibility,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};

use super::{
    SqlitePlanningAuthorityAdapter, connection, count, register_next, seed, stored, task, workspace,
};

mod delivery_evidence;

fn basis(revision: u64) -> PlanningWorkPermitBasis {
    PlanningWorkPermitBasis {
        delivery_review: None,
        authorization_epoch: 0,
        policy_version: "policy-1".into(),
        field_hash: "field-1".into(),
        gate_revision: revision,
        source_sha: "a".repeat(40),
    }
}

fn permit(stage: PlanningWorkPermitStage, revision: u64) -> PlanningWorkPermit {
    PlanningWorkPermit {
        work_order_id: "order-1".into(),
        stage,
        basis: basis(revision),
        requires_human_review: stage == PlanningWorkPermitStage::Integrate,
    }
}

fn fixture(name: &str) -> (String, SqlitePlanningAuthorityAdapter) {
    let workspace = workspace(name);
    seed(&workspace, vec![task("ordinary-task")]);
    register_next(&workspace, "order-1", "controlled-task");
    (workspace, SqlitePlanningAuthorityAdapter::new())
}

fn check(
    adapter: &SqlitePlanningAuthorityAdapter,
    workspace: &str,
    stage: PlanningWorkPermitStage,
    source: Option<&str>,
) -> PlanningWorkPermitDecision {
    PlanningWorkPermitRepositoryPort::check_work_order_permit_for_task(
        adapter,
        workspace,
        "controlled-task",
        stage,
        source,
    )
    .unwrap()
}

fn grant_all(adapter: &SqlitePlanningAuthorityAdapter, workspace: &str, revision: u64) {
    adapter
        .set_work_order_permit_basis(workspace, "order-1", &basis(revision))
        .unwrap();
    for stage in PlanningWorkPermitStage::ALL {
        adapter
            .grant_work_order_permit(workspace, &permit(stage, revision))
            .unwrap();
    }
}

fn lease(task_id: &str, slot: &str) -> ParallelModeSlotLeaseSnapshot {
    ParallelModeSlotLeaseSnapshot::new(
        slot,
        task_id,
        "Synthetic task",
        slot,
        format!("agent/{slot}"),
        format!("/tmp/synthetic/{slot}"),
        ParallelModeSlotLeaseState::Leased,
        "2026-11-21T00:00:00Z",
        None,
    )
    .with_lease_generation("b".repeat(64))
    .with_delivery_target(ParallelModeDeliveryTargetSnapshot::new(
        "origin",
        "synthetic/repository",
        ParallelModeRepositoryVisibility::Private,
        "prerelease",
        "a".repeat(40),
    ))
}

#[test]
fn work_permit_durable_grants_are_stage_scoped_and_legacy_tasks_remain_unrestricted() {
    let (workspace, adapter) = fixture("permit-basic");
    assert_eq!(
        PlanningWorkPermitRepositoryPort::check_work_order_permit_for_task(
            &adapter,
            &workspace,
            "ordinary-task",
            PlanningWorkPermitStage::Integrate,
            None,
        )
        .unwrap(),
        PlanningWorkPermitDecision::Legacy
    );
    assert_eq!(
        check(
            &adapter,
            &workspace,
            PlanningWorkPermitStage::Dispatch,
            None
        ),
        PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Missing)
    );
    assert!(
        adapter
            .grant_work_order_permit(&workspace, &permit(PlanningWorkPermitStage::Push, 1))
            .is_err()
    );
    grant_all(&adapter, &workspace, 1);
    let restarted = SqlitePlanningAuthorityAdapter::new();
    for stage in PlanningWorkPermitStage::ALL {
        assert_eq!(
            check(&restarted, &workspace, stage, Some(&basis(1).source_sha)),
            PlanningWorkPermitDecision::Granted {
                requires_human_review: stage == PlanningWorkPermitStage::Integrate
            }
        );
    }
    assert_eq!(
        adapter
            .load_work_order_permit_basis(&workspace, "order-1")
            .unwrap(),
        Some(basis(1))
    );
    assert_eq!(stored(&workspace, "order-1").permits.len(), 4);
}

#[test]
fn work_permit_basis_updates_invalidate_old_grants_and_cannot_roll_back() {
    let (workspace, adapter) = fixture("permit-stale");
    grant_all(&adapter, &workspace, 1);
    let mut next = basis(2);
    next.field_hash = "field-2".into();
    adapter
        .set_work_order_permit_basis(&workspace, "order-1", &next)
        .unwrap();
    for stage in PlanningWorkPermitStage::ALL {
        assert_eq!(
            check(&adapter, &workspace, stage, Some(&next.source_sha)).blocked_reason(),
            Some("permit_stale")
        );
    }
    assert!(
        adapter
            .set_work_order_permit_basis(&workspace, "order-1", &basis(1))
            .is_err()
    );
    assert!(
        adapter
            .set_work_order_permit_basis(&workspace, "order-1", &basis(2))
            .is_err()
    );
    assert!(
        adapter
            .grant_work_order_permit(&workspace, &permit(PlanningWorkPermitStage::Push, 1))
            .is_err()
    );
    adapter
        .set_work_order_permit_basis(&workspace, "order-1", &next)
        .unwrap();
    let renewed = PlanningWorkPermit {
        basis: next,
        ..permit(PlanningWorkPermitStage::Push, 2)
    };
    adapter
        .grant_work_order_permit(&workspace, &renewed)
        .unwrap();
    assert_eq!(
        check(
            &adapter,
            &workspace,
            PlanningWorkPermitStage::Push,
            Some(&basis(1).source_sha)
        ),
        PlanningWorkPermitDecision::Granted {
            requires_human_review: false
        }
    );
}

#[test]
fn work_permit_revocation_survives_restart_and_fences_delayed_grant_replays() {
    let (workspace, adapter) = fixture("permit-revoke");
    grant_all(&adapter, &workspace, 1);
    assert_eq!(
        adapter
            .revoke_work_order_permits(
                &workspace,
                "order-1",
                &[PlanningWorkPermitStage::Push],
                "operator hold",
            )
            .unwrap(),
        1
    );
    let restarted = SqlitePlanningAuthorityAdapter::new();
    assert_eq!(
        check(
            &restarted,
            &workspace,
            PlanningWorkPermitStage::Push,
            Some(&basis(1).source_sha)
        )
        .blocked_reason(),
        Some("permit_missing")
    );
    assert!(
        restarted
            .grant_work_order_permit(&workspace, &permit(PlanningWorkPermitStage::Push, 1))
            .is_err()
    );
    assert_eq!(
        check(
            &restarted,
            &workspace,
            PlanningWorkPermitStage::Integrate,
            Some(&basis(1).source_sha)
        ),
        PlanningWorkPermitDecision::Granted {
            requires_human_review: true
        }
    );
    assert_eq!(
        restarted
            .revoke_work_order_permits(
                &workspace,
                "order-1",
                &[PlanningWorkPermitStage::Push],
                "replayed hold",
            )
            .unwrap(),
        0
    );
    restarted
        .set_work_order_permit_basis(&workspace, "order-1", &basis(2))
        .unwrap();
    restarted
        .grant_work_order_permit(&workspace, &permit(PlanningWorkPermitStage::Push, 2))
        .unwrap();
    assert_eq!(
        check(
            &restarted,
            &workspace,
            PlanningWorkPermitStage::Push,
            Some(&basis(2).source_sha)
        ),
        PlanningWorkPermitDecision::Granted {
            requires_human_review: false
        }
    );
}

#[test]
fn work_permit_revoking_absent_grants_fences_initial_delayed_grants() {
    let (workspace, adapter) = fixture("permit-revoke-absent");
    adapter
        .set_work_order_permit_basis(&workspace, "order-1", &basis(1))
        .unwrap();
    assert_eq!(
        adapter
            .revoke_work_order_permits(
                &workspace,
                "order-1",
                &PlanningWorkPermitStage::ALL,
                "take over before grant",
            )
            .unwrap(),
        0
    );
    for stage in PlanningWorkPermitStage::ALL {
        assert!(
            adapter
                .grant_work_order_permit(&workspace, &permit(stage, 1))
                .is_err()
        );
    }
    grant_all(&adapter, &workspace, 2);
}

#[test]
fn work_permit_cannot_downgrade_human_review_within_the_same_revision() {
    let (workspace, adapter) = fixture("permit-review");
    grant_all(&adapter, &workspace, 1);
    let mut unsafe_replay = permit(PlanningWorkPermitStage::Integrate, 1);
    unsafe_replay.requires_human_review = false;
    assert!(
        adapter
            .grant_work_order_permit(&workspace, &unsafe_replay)
            .is_err()
    );
    assert!(
        check(
            &adapter,
            &workspace,
            PlanningWorkPermitStage::Integrate,
            Some(&basis(1).source_sha)
        )
        .requires_human_review()
    );
}

#[test]
fn work_permit_audit_failure_rolls_back_grant_and_revocation() {
    let (workspace, adapter) = fixture("permit-rollback");
    adapter
        .set_work_order_permit_basis(&workspace, "order-1", &basis(1))
        .unwrap();
    let db = connection(&workspace);
    db.execute_batch(
        "CREATE TRIGGER fail_permit_audit BEFORE INSERT ON runtime_events
         BEGIN SELECT RAISE(ABORT, 'synthetic audit failure'); END;",
    )
    .unwrap();
    assert!(
        adapter
            .grant_work_order_permit(&workspace, &permit(PlanningWorkPermitStage::Push, 1))
            .is_err()
    );
    assert!(stored(&workspace, "order-1").permits.is_empty());
    db.execute_batch("DROP TRIGGER fail_permit_audit").unwrap();
    adapter
        .grant_work_order_permit(&workspace, &permit(PlanningWorkPermitStage::Push, 1))
        .unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_permit_audit BEFORE INSERT ON runtime_events
         BEGIN SELECT RAISE(ABORT, 'synthetic audit failure'); END;",
    )
    .unwrap();
    assert!(
        adapter
            .revoke_work_order_permits(
                &workspace,
                "order-1",
                &[PlanningWorkPermitStage::Push],
                "hold"
            )
            .is_err()
    );
    assert_eq!(stored(&workspace, "order-1").permits.len(), 1);
    assert_eq!(
        count(&workspace, "planning_work_order_permit_revocations"),
        0
    );
}

#[test]
fn work_permit_atomic_slot_claim_blocks_only_the_controlled_task() {
    let (workspace, adapter) = fixture("permit-claim");
    let blocked = lease("controlled-task", "slot-controlled");
    let error = SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &blocked)
        .unwrap_err();
    assert!(error.to_string().contains("permit_missing"));
    assert_eq!(count(&workspace, "runtime_slot_leases"), 0);
    let mut ordinary = lease("ordinary-task", "slot-ordinary");
    ordinary.delivery_target = None;
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &ordinary).unwrap();
    assert_eq!(count(&workspace, "runtime_slot_leases"), 1);
    grant_all(&adapter, &workspace, 1);
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &blocked).unwrap();
    assert_eq!(count(&workspace, "runtime_slot_leases"), 2);
    let reasons: i64 = connection(&workspace)
        .query_row(
            "SELECT COUNT(*) FROM runtime_events
         WHERE event_kind = 'work_order_permit_blocked' AND summary = 'permit_missing'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reasons, 1);
}

#[test]
fn work_permit_claim_requires_frozen_sha_and_preserves_existing_turns_after_revoke() {
    let (workspace, adapter) = fixture("permit-claim-sha");
    grant_all(&adapter, &workspace, 1);
    let mut granted = lease("controlled-task", "slot-controlled");
    let mut wrong_source = granted.clone();
    wrong_source
        .delivery_target
        .as_mut()
        .unwrap()
        .integration_base_commit_sha = "c".repeat(40);
    assert!(
        SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &wrong_source,)
            .unwrap_err()
            .to_string()
            .contains("permit_stale")
    );
    let mut no_source = granted.clone();
    no_source.delivery_target = None;
    assert!(
        SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &no_source,)
            .unwrap_err()
            .to_string()
            .contains("permit_stale")
    );
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &granted).unwrap();
    adapter
        .revoke_work_order_permits(
            &workspace,
            "order-1",
            &PlanningWorkPermitStage::ALL,
            "operator hold",
        )
        .unwrap();
    granted.state = ParallelModeSlotLeaseState::Running;
    granted.running_started_at = Some("2026-11-21T00:01:00Z".into());
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &granted).unwrap();
    let changed_generation = granted.with_lease_generation("c".repeat(64));
    assert!(
        SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &changed_generation,)
            .unwrap_err()
            .to_string()
            .contains("permit_missing")
    );
    assert_eq!(count(&workspace, "runtime_slot_leases"), 1);
}

#[test]
fn work_permit_invalid_saved_grants_fail_closed() {
    let (workspace, adapter) = fixture("permit-malformed");
    connection(&workspace)
        .execute(
            "UPDATE planning_work_orders SET permits_json = '[{\"stage\":\"integrate\"}]'
         WHERE work_order_id = 'order-1'",
            [],
        )
        .unwrap();
    assert!(
        PlanningWorkPermitRepositoryPort::check_work_order_permit_for_task(
            &adapter,
            &workspace,
            "controlled-task",
            PlanningWorkPermitStage::Integrate,
            Some(&basis(1).source_sha),
        )
        .is_err()
    );
}

#[test]
fn work_permit_concurrent_independent_grants_preserve_every_stage() {
    let (workspace, adapter) = fixture("permit-concurrent-grants");
    adapter
        .set_work_order_permit_basis(&workspace, "order-1", &basis(1))
        .unwrap();
    let barrier = std::sync::Barrier::new(4);
    std::thread::scope(|scope| {
        let handles = PlanningWorkPermitStage::ALL.map(|stage| {
            let workspace = &workspace;
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                SqlitePlanningAuthorityAdapter::new()
                    .grant_work_order_permit(workspace, &permit(stage, 1))
                    .unwrap();
            })
        });
        for handle in handles {
            handle.join().unwrap();
        }
    });
    assert_eq!(stored(&workspace, "order-1").permits.len(), 4);
}

mod control;
mod delivery_review;

mod execution;
mod task_evidence;
mod task_verification;

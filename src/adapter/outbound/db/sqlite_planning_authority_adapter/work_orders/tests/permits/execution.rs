use std::sync::{Arc, Barrier};

use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControlRepositoryPort, PlanningWorkOrderRepositoryPort,
};
use crate::domain::planning::TaskStatus;

fn add_second_task(workspace: &str, adapter: &SqlitePlanningAuthorityAdapter) {
    let before = super::super::snapshot(workspace);
    super::super::commit_document(
        workspace,
        Some(before.planning_revision),
        &super::super::candidate(&before, "second-task"),
    );
    adapter
        .link_supplemental_task(workspace, "order-1", "second-task")
        .unwrap();
    grant_all(adapter, workspace, 2);
}

#[test]
fn zero_cap_denies_first_launch_and_same_task_resume_never_spends_another_attempt() {
    let (workspace, adapter) = fixture("execution-limit-resume");
    grant_all(&adapter, &workspace, 1);
    let first = lease("controlled-task", "slot-1");
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &first).unwrap();
    adapter
        .set_work_order_execution_limit(&workspace, "order-1", 0)
        .unwrap();
    assert!(adapter.claim_work_order_launch(&workspace, &first).is_err());
    assert!(
        adapter
            .started_work_order_task_ids(&workspace, "order-1")
            .unwrap()
            .is_empty()
    );
    assert_eq!(count(&workspace, "planning_work_order_launch_seals"), 0);
    assert_eq!(
        check(
            &adapter,
            &workspace,
            PlanningWorkPermitStage::Dispatch,
            Some(&basis(1).source_sha)
        ),
        PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::ExecutionAttemptLimit)
    );

    adapter
        .set_work_order_execution_limit(&workspace, "order-1", 1)
        .unwrap();
    adapter.claim_work_order_launch(&workspace, &first).unwrap();
    let resumed = first.with_lease_generation("c".repeat(64));
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &resumed).unwrap();
    adapter
        .claim_work_order_launch(&workspace, &resumed)
        .unwrap();
    assert_eq!(
        adapter
            .started_work_order_task_ids(&workspace, "order-1")
            .unwrap(),
        ["controlled-task"]
    );

    add_second_task(&workspace, &adapter);
    let second = lease("second-task", "slot-2");
    assert!(
        SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &second).is_err()
    );
    adapter
        .set_work_order_execution_limit(&workspace, "order-1", 2)
        .unwrap();
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &second).unwrap();
    adapter
        .claim_work_order_launch(&workspace, &second)
        .unwrap();
    assert_eq!(
        adapter
            .started_work_order_task_ids(&workspace, "order-1")
            .unwrap(),
        ["controlled-task", "second-task"]
    );
}

#[test]
fn first_launch_and_seal_rollback_as_one_transaction() {
    let (workspace, adapter) = fixture("execution-limit-rollback");
    grant_all(&adapter, &workspace, 1);
    adapter
        .set_work_order_execution_limit(&workspace, "order-1", 1)
        .unwrap();
    let first = lease("controlled-task", "slot-1");
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &first).unwrap();
    connection(&workspace).execute_batch(
        "CREATE TRIGGER fail_launch_seal BEFORE INSERT ON planning_work_order_launch_seals BEGIN SELECT RAISE(ABORT,'synthetic seal failure'); END;"
    ).unwrap();
    assert!(adapter.claim_work_order_launch(&workspace, &first).is_err());
    assert!(
        adapter
            .started_work_order_task_ids(&workspace, "order-1")
            .unwrap()
            .is_empty()
    );
    assert_eq!(count(&workspace, "planning_work_order_launch_seals"), 0);
    connection(&workspace)
        .execute_batch("DROP TRIGGER fail_launch_seal;")
        .unwrap();
    adapter.claim_work_order_launch(&workspace, &first).unwrap();
    assert_eq!(
        adapter
            .started_work_order_task_ids(&workspace, "order-1")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn concurrent_first_launches_cannot_spend_the_same_remaining_attempt() {
    let (workspace, adapter) = fixture("execution-limit-race");
    grant_all(&adapter, &workspace, 1);
    add_second_task(&workspace, &adapter);
    let first = lease("controlled-task", "slot-1");
    let second = lease("second-task", "slot-2");
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &first).unwrap();
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &second).unwrap();
    adapter
        .set_work_order_execution_limit(&workspace, "order-1", 1)
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [first, second]
        .into_iter()
        .map(|lease| {
            let workspace = workspace.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                SqlitePlanningAuthorityAdapter::new().claim_work_order_launch(&workspace, &lease)
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(Result::is_ok)
            .count(),
        1
    );
    assert_eq!(
        adapter
            .started_work_order_task_ids(&workspace, "order-1")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(count(&workspace, "planning_work_order_launch_seals"), 1);
}

#[test]
fn awaiting_user_does_not_use_the_ordinary_launch_boundary() {
    let (workspace, adapter) = fixture("execution-waiting-user");
    grant_all(&adapter, &workspace, 1);
    let first = lease("controlled-task", "slot-1");
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&workspace, &first).unwrap();
    let before = super::super::snapshot(&workspace);
    let mut waiting = before.task_authority.clone();
    waiting
        .tasks
        .iter_mut()
        .find(|task| task.id == "controlled-task")
        .unwrap()
        .status = TaskStatus::AwaitingUser;
    super::super::commit_document(&workspace, Some(before.planning_revision), &waiting);
    assert!(adapter.claim_work_order_launch(&workspace, &first).is_err());
    assert!(
        adapter
            .started_work_order_task_ids(&workspace, "order-1")
            .unwrap()
            .is_empty()
    );
}

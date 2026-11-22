use std::sync::mpsc;
use std::time::Duration;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskAuthorityMutationAudit;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkOrderRegistration, PlanningWorkOrderRegistrationResult,
    PlanningWorkOrderRepositoryPort,
};
use crate::domain::planning::{OriginSessionKind, TaskMutationProvenance};

use super::{
    SqlitePlanningAuthorityAdapter, candidate, connection, count, ensure_work_order_tables,
    metadata, projection, register_document, register_next, seed, snapshot, stored, task,
    workspace,
};

#[test]
fn work_order_concurrent_independent_connections_register_exactly_one_task() {
    let workspace = workspace("concurrent-registration");
    let before = seed(&workspace, vec![task("legacy-task")]);
    ensure_work_order_tables(&connection(&workspace)).unwrap();
    let before_audit = count(&workspace, "planning_task_mutation_events");
    let (ready, ready_rx) = mpsc::channel();
    let mut starts = Vec::new();
    let mut workers = Vec::new();
    for task_id in ["candidate-a", "candidate-b"] {
        let workspace = workspace.clone();
        let before = before.clone();
        let ready = ready.clone();
        let (start, start_rx) = mpsc::channel();
        starts.push(start);
        workers.push(std::thread::spawn(move || {
            let next = candidate(&before, task_id);
            ready.send(()).expect("parent should wait for candidates");
            start_rx
                .recv_timeout(Duration::from_secs(30))
                .expect("parent should release candidates");
            // Each call resolves and opens its own real SQLite connection; both candidates
            // observed the same revision before either transaction can start.
            register_document(
                &workspace,
                "same-order",
                "synthetic-source",
                task_id,
                before.planning_revision,
                &next,
            )
        }));
    }
    for _ in 0..2 {
        ready_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("both candidates should become ready");
    }
    for start in starts {
        start.send(()).expect("candidate should wait for release");
    }
    let results = workers
        .into_iter()
        .map(|worker| {
            worker
                .join()
                .expect("candidate should not panic")
                .expect("registration should succeed")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(
                result,
                PlanningWorkOrderRegistrationResult::Registered { .. }
            ))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, PlanningWorkOrderRegistrationResult::Existing { .. }))
            .count(),
        1
    );
    let records = results
        .into_iter()
        .map(|result| match result {
            PlanningWorkOrderRegistrationResult::Registered { work_order, .. }
            | PlanningWorkOrderRegistrationResult::Existing { work_order } => work_order,
            other => panic!("same order must win before stale revision: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(records[0], records[1]);
    assert_eq!(count(&workspace, "planning_tasks"), 2);
    assert_eq!(count(&workspace, "planning_work_orders"), 1);
    assert_eq!(count(&workspace, "planning_work_order_tasks"), 1);
    assert_eq!(
        count(&workspace, "planning_task_mutation_events"),
        before_audit + 1
    );
    assert_eq!(
        snapshot(&workspace).planning_revision,
        before.planning_revision + 1
    );
}

#[test]
fn work_order_write_failure_rolls_back_task_queue_revision_audit_and_lineage() {
    for failing_table in ["planning_work_orders", "planning_work_order_tasks"] {
        let workspace = workspace(&format!("rollback-{failing_table}"));
        let before = seed(&workspace, vec![task("legacy-task")]);
        ensure_work_order_tables(&connection(&workspace)).unwrap();
        let before_metadata = metadata(&workspace);
        let before_audit = count(&workspace, "planning_task_mutation_events");
        connection(&workspace)
            .execute_batch(&format!(
                "CREATE TRIGGER fail_work_order_write BEFORE INSERT ON {failing_table}
                 BEGIN SELECT RAISE(ABORT, 'injected work-order write failure'); END;"
            ))
            .expect("fixture should inject a real SQLite write failure");
        let error = register_document(
            &workspace,
            "order-a",
            "synthetic-source",
            "task-a",
            before.planning_revision,
            &candidate(&before, "task-a"),
        )
        .expect_err("failure after task writes must abort the whole transaction");
        assert!(format!("{error:#}").contains("injected work-order write failure"));
        assert_eq!(snapshot(&workspace), before);
        assert_eq!(metadata(&workspace), before_metadata);
        assert_eq!(
            count(&workspace, "planning_task_mutation_events"),
            before_audit
        );
        assert_eq!(count(&workspace, "planning_work_orders"), 0);
        assert_eq!(count(&workspace, "planning_work_order_tasks"), 0);
        connection(&workspace)
            .execute_batch("DROP TRIGGER fail_work_order_write")
            .unwrap();
        let retry = register_next(&workspace, "order-a", "task-a");
        assert_eq!(retry.task_ids, vec!["task-a"]);
        assert_eq!(count(&workspace, "planning_tasks"), 2);
        assert_eq!(
            count(&workspace, "planning_task_mutation_events"),
            before_audit + 1
        );
    }
}

#[test]
fn work_order_admin_guard_blocks_new_writes_but_not_existing_identity_reads() {
    let workspace = workspace("authority-guard");
    seed(&workspace, vec![task("supplement-a")]);
    let first = register_next(&workspace, "order-a", "initial-a");
    let current = snapshot(&workspace);
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .acquire_admin_authority_mutation_guard(
            &workspace,
            "operator-fixture",
            "synthetic authority edit",
        )
        .expect("operator guard should acquire");
    assert_eq!(
        register_document(
            &workspace,
            "order-a",
            "synthetic-source",
            "",
            -1,
            &current.task_authority,
        )
        .unwrap(),
        PlanningWorkOrderRegistrationResult::Existing {
            work_order: first.clone()
        }
    );
    let already_linked = adapter
        .link_supplemental_task(&workspace, "order-a", "initial-a")
        .unwrap();
    assert!(!already_linked.linked);
    register_document(
        &workspace,
        "order-b",
        "synthetic-source",
        "initial-b",
        current.planning_revision,
        &candidate(&current, "initial-b"),
    )
    .expect_err("new work order must respect the authority guard");
    adapter
        .link_supplemental_task(&workspace, "order-a", "supplement-a")
        .expect_err("new lineage must respect the authority guard");
    assert_eq!(snapshot(&workspace), current);
    assert_eq!(stored(&workspace, "order-a"), first);
    assert_eq!(count(&workspace, "planning_work_orders"), 1);
    adapter
        .release_admin_authority_mutation_guard(&workspace, "operator-fixture")
        .unwrap();
    assert!(
        adapter
            .link_supplemental_task(&workspace, "order-a", "supplement-a")
            .unwrap()
            .linked
    );
}

#[test]
fn work_order_registration_rejects_an_audit_for_another_task() {
    let workspace = workspace("audit-boundary");
    let before = seed(&workspace, Vec::new());
    let next = candidate(&before, "initial-a");
    let queue = projection(&next);
    let provenance = TaskMutationProvenance::new(OriginSessionKind::System);
    let wrong_ids = ["unrelated-task".to_string()];
    SqlitePlanningAuthorityAdapter::new()
        .register_work_order(
            &workspace,
            PlanningWorkOrderRegistration {
                link_reference: None,
                work_order_id: "order-a",
                origin: "synthetic-source",
                initial_task_id: "initial-a",
                observed_planning_revision: before.planning_revision,
                task_authority: &next,
                queue_projection: &queue,
                audit: PlanningTaskAuthorityMutationAudit {
                    task_ids: &wrong_ids,
                    legacy_source_turn_id: None,
                    provenance: &provenance,
                },
            },
        )
        .expect_err("a different task must not receive the new task's attribution");
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(count(&workspace, "planning_work_orders"), 0);
}

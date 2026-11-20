use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkOrderError, PlanningWorkOrderRegistrationResult, PlanningWorkOrderRepositoryPort,
};
use crate::domain::planning::TaskAuthorityDocument;

use super::{
    SqlitePlanningAuthorityAdapter, candidate, commit_document, count, metadata, register_document,
    register_next, seed, snapshot, stored, task, workspace,
};

#[test]
fn work_order_supplemental_lineage_is_ordered_bidirectional_and_idempotent() {
    let workspace = workspace("supplemental-lineage");
    seed(&workspace, vec![task("supplement-a"), task("supplement-b")]);
    let first = register_next(&workspace, "order-a", "initial-a");
    let second = register_next(&workspace, "order-b", "initial-b");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let before_link = snapshot(&workspace);
    let linked = adapter
        .link_supplemental_task(&workspace, "order-a", "supplement-a")
        .unwrap();
    assert!(linked.linked);
    assert_eq!(linked.planning_revision, before_link.planning_revision + 1);
    assert_eq!(
        linked.work_order.task_ids,
        vec!["initial-a", "supplement-a"]
    );
    assert_eq!(linked.work_order.origin, first.origin);
    assert!(linked.work_order.permits.is_empty());
    let next = adapter
        .link_supplemental_task(&workspace, "order-a", "supplement-b")
        .unwrap();
    assert_eq!(
        next.work_order.task_ids,
        vec!["initial-a", "supplement-a", "supplement-b"]
    );
    let after_link = snapshot(&workspace);
    assert_eq!(after_link.task_authority, before_link.task_authority);
    assert_eq!(after_link.queue_projection, before_link.queue_projection);
    for id in ["initial-a", "supplement-a", "supplement-b"] {
        assert_eq!(
            adapter.load_work_order_for_task(&workspace, id).unwrap(),
            Some(next.work_order.clone())
        );
    }

    let no_op_metadata = metadata(&workspace);
    let replay = adapter
        .link_supplemental_task(&workspace, "order-a", "supplement-a")
        .unwrap();
    assert!(!replay.linked);
    assert_eq!(replay.work_order, next.work_order);
    assert_eq!(replay.planning_revision, after_link.planning_revision);
    assert_eq!(metadata(&workspace), no_op_metadata);

    let error = adapter
        .link_supplemental_task(&workspace, "order-b", "supplement-a")
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<PlanningWorkOrderError>(),
        Some(&PlanningWorkOrderError::TaskAlreadyLinked {
            task_id: "supplement-a".to_string(),
            work_order_id: "order-a".to_string(),
        })
    );
    let missing = adapter
        .link_supplemental_task(&workspace, "order-a", "missing-task")
        .unwrap_err();
    assert_eq!(
        missing.downcast_ref::<PlanningWorkOrderError>(),
        Some(&PlanningWorkOrderError::TaskNotFound {
            task_id: "missing-task".to_string()
        })
    );
    let missing_order = adapter
        .link_supplemental_task(&workspace, "missing-order", "supplement-a")
        .unwrap_err();
    assert_eq!(
        missing_order.downcast_ref::<PlanningWorkOrderError>(),
        Some(&PlanningWorkOrderError::WorkOrderNotFound {
            work_order_id: "missing-order".to_string()
        })
    );
    assert_eq!(stored(&workspace, "order-b"), second);
    assert_eq!(snapshot(&workspace), after_link);
}

#[test]
fn work_order_replay_and_links_survive_task_deletion_and_authority_clear() {
    let workspace = workspace("delete-clear");
    seed(&workspace, vec![task("supplement-a")]);
    register_next(&workspace, "order-a", "initial-a");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let linked = adapter
        .link_supplemental_task(&workspace, "order-a", "supplement-a")
        .unwrap();
    let before_delete = snapshot(&workspace);
    let empty = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    commit_document(&workspace, Some(before_delete.planning_revision), &empty);
    let after_delete = snapshot(&workspace);
    assert_eq!(
        register_document(&workspace, "order-a", "synthetic-source", "", -1, &empty).unwrap(),
        PlanningWorkOrderRegistrationResult::Existing {
            work_order: linked.work_order.clone()
        }
    );
    assert_eq!(snapshot(&workspace), after_delete);
    let retry_link = adapter
        .link_supplemental_task(&workspace, "order-a", "supplement-a")
        .unwrap();
    assert!(!retry_link.linked);
    assert_eq!(retry_link.planning_revision, after_delete.planning_revision);

    adapter.clear_task_authority_snapshot(&workspace).unwrap();
    let cleared_metadata = metadata(&workspace);
    assert!(
        adapter
            .load_task_authority_snapshot(&workspace)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        register_document(&workspace, "order-a", "synthetic-source", "", -1, &empty).unwrap(),
        PlanningWorkOrderRegistrationResult::Existing {
            work_order: linked.work_order.clone()
        }
    );
    assert!(
        adapter
            .load_task_authority_snapshot(&workspace)
            .unwrap()
            .is_none()
    );
    assert_eq!(metadata(&workspace), cleared_metadata);
    assert_eq!(count(&workspace, "planning_tasks"), 0);
    for id in ["initial-a", "supplement-a"] {
        assert_eq!(
            adapter.load_work_order_for_task(&workspace, id).unwrap(),
            Some(linked.work_order.clone())
        );
    }
}

#[test]
fn work_order_deleted_task_identity_cannot_be_reassigned_to_another_order() {
    let workspace = workspace("permanent-task-identity");
    seed(&workspace, Vec::new());
    let first = register_next(&workspace, "order-a", "initial-a");
    let before_clear = snapshot(&workspace);
    commit_document(
        &workspace,
        Some(before_clear.planning_revision),
        &TaskAuthorityDocument {
            version: 1,
            tasks: Vec::new(),
        },
    );
    let current = snapshot(&workspace);
    assert_eq!(
        register_document(
            &workspace,
            "order-b",
            "synthetic-source",
            "initial-a",
            current.planning_revision,
            &candidate(&current, "initial-a"),
        )
        .unwrap(),
        PlanningWorkOrderRegistrationResult::TaskIdConflict {
            task_id: "initial-a".to_string()
        }
    );
    assert_eq!(snapshot(&workspace), current);
    assert_eq!(stored(&workspace, "order-a"), first);
    assert_eq!(count(&workspace, "planning_work_orders"), 1);
    assert_eq!(
        register_next(&workspace, "order-b", "initial-b").task_ids,
        vec!["initial-b"]
    );
    assert_eq!(stored(&workspace, "order-a"), first);
    assert_eq!(count(&workspace, "planning_tasks"), 1);
}

#[test]
fn work_order_lookup_is_scoped_to_its_authority_database() {
    let first = workspace("scope-first");
    let second = workspace("scope-second");
    seed(&first, Vec::new());
    seed(&second, Vec::new());
    let record = register_next(&first, "same-order", "same-task");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    assert_eq!(
        adapter.load_work_order(&second, "same-order").unwrap(),
        None
    );
    assert_eq!(
        adapter
            .load_work_order_for_task(&second, "same-task")
            .unwrap(),
        None
    );
    let independent = register_next(&second, "same-order", "different-task");
    assert_ne!(record.task_ids, independent.task_ids);
    assert_eq!(stored(&first, "same-order"), record);
}

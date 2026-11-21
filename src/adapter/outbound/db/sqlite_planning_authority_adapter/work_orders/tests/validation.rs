use super::*;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityPort,
};
use crate::domain::parallel_mode::ParallelModeQueueItemState;

fn queue_record(key: &str, task_id: &str) -> PlanningAuthorityDistributorQueueRecord {
    serde_json::from_value(serde_json::json!({
        "queue_item_id":key,"session_key":format!("session-{key}"),"agent_id":"agent",
        "task_id":task_id,"task_title":"Synthetic result","branch_name":"agent/branch",
        "worktree_path":"/tmp/synthetic-branch","commit_sha":"a".repeat(40),
        "validation_summary":"","authority_refresh_outcome":"",
        "queue_state":ParallelModeQueueItemState::Queued,"integration_note":"",
        "enqueued_at":"2026-11-21T00:00:00Z","updated_at":"2026-11-21T00:00:00Z"
    }))
    .unwrap()
}

#[test]
fn distributor_queue_and_immutable_validation_source_are_saved_together() {
    let workspace = workspace("validation-source-atomic");
    seed(&workspace, vec![task("legacy-source")]);
    register_next(&workspace, "order-a", "owned-source");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let record = queue_record("validation-owned", "owned-source");
    adapter
        .upsert_runtime_distributor_queue_record(&workspace, &record)
        .unwrap();
    let source = adapter
        .load_work_order_validation_source(&workspace, &record.queue_item_id)
        .unwrap()
        .unwrap();
    assert_eq!(source.work_order_id.as_deref(), Some("order-a"));
    assert_eq!(source.source_task_id, "owned-source");
    let mut rebound = record.clone();
    rebound.task_id = "legacy-source".into();
    assert!(
        adapter
            .upsert_runtime_distributor_queue_record(&workspace, &rebound)
            .is_err()
    );
    assert_eq!(
        adapter
            .load_runtime_projections(&workspace)
            .unwrap()
            .distributor_queue_records,
        vec![record.clone()]
    );
    connection(&workspace)
        .execute("DELETE FROM runtime_distributor_queue", [])
        .unwrap();
    assert_eq!(
        adapter
            .load_work_order_validation_source(&workspace, &record.queue_item_id)
            .unwrap(),
        Some(source)
    );
    assert_eq!(
        adapter
            .work_order_validation_record_keys(&workspace, "order-a")
            .unwrap(),
        ["validation-owned"]
    );
}

#[test]
fn missing_source_is_distinct_from_verified_legacy_and_cannot_silently_rebind() {
    let workspace = workspace("validation-source-legacy");
    seed(&workspace, vec![task("legacy-source")]);
    register_next(&workspace, "order-a", "owned-source");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    assert!(
        adapter
            .load_work_order_validation_source(&workspace, "unknown-record")
            .unwrap()
            .is_none()
    );
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace,
            &queue_record("legacy-record", "legacy-source"),
        )
        .unwrap();
    let legacy = adapter
        .load_work_order_validation_source(&workspace, "legacy-record")
        .unwrap()
        .unwrap();
    assert!(legacy.work_order_id.is_none());
    assert_eq!(legacy.source_task_id, "legacy-source");
    assert!(
        adapter
            .load_work_order_validation_link(&workspace, "legacy-record")
            .unwrap()
            .is_none()
    );
    assert!(
        adapter
            .link_supplemental_task(&workspace, "order-a", "legacy-source")
            .is_err()
    );
}

#[test]
fn failed_validation_source_write_rolls_back_queue_and_source() {
    let workspace = workspace("validation-source-rollback");
    seed(&workspace, vec![task("source")]);
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .load_work_order_validation_source(&workspace, "unused")
        .unwrap();
    connection(&workspace).execute_batch(
        "CREATE TRIGGER fail_validation_source BEFORE INSERT ON planning_work_order_validation_links BEGIN SELECT RAISE(ABORT,'synthetic source failure'); END;"
    ).unwrap();
    assert!(
        adapter
            .upsert_runtime_distributor_queue_record(
                &workspace,
                &queue_record("failed-source", "source")
            )
            .is_err()
    );
    assert!(
        adapter
            .load_runtime_projections(&workspace)
            .unwrap()
            .distributor_queue_records
            .is_empty()
    );
    assert_eq!(count(&workspace, "planning_work_order_validation_links"), 0);
}

#[test]
fn preupgrade_queue_repairs_source_but_missing_queue_never_proves_legacy() {
    let workspace = workspace("validation-source-upgrade");
    seed(&workspace, vec![]);
    register_next(&workspace, "order-a", "owned-source");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let record = queue_record("old-record", "owned-source");
    adapter
        .upsert_runtime_distributor_queue_record(&workspace, &record)
        .unwrap();
    connection(&workspace)
        .execute("DELETE FROM planning_work_order_validation_links", [])
        .unwrap();
    assert_eq!(
        adapter
            .load_work_order_validation_source(&workspace, "old-record")
            .unwrap()
            .unwrap()
            .work_order_id
            .as_deref(),
        Some("order-a")
    );
    connection(&workspace)
        .execute("DELETE FROM planning_work_order_validation_links", [])
        .unwrap();
    connection(&workspace)
        .execute("DELETE FROM runtime_distributor_queue", [])
        .unwrap();
    assert!(
        adapter
            .load_work_order_validation_source(&workspace, "old-record")
            .unwrap()
            .is_none()
    );
}

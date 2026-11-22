use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::Connection;

use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit,
    PlanningTaskAuthorityMutationAudit, PlanningTaskAuthoritySnapshot, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkOrderError, PlanningWorkOrderRecord, PlanningWorkOrderRegistration,
    PlanningWorkOrderRegistrationResult, PlanningWorkOrderRepositoryPort,
};
use crate::domain::kernel_event::{KernelEvent, RecordedKernelEvent};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
    PriorityQueueProjection, PriorityQueueTask, QueueIdleConfig, QueueIdlePolicy, TaskActor,
    TaskAuthorityDocument, TaskDefinition, TaskMutationProvenance, TaskStatus,
};

use super::super::{open_authority_connection, table_exists};
use super::{SqlitePlanningAuthorityAdapter, ensure_work_order_tables};

mod lineage;
mod permits;
mod publication_reference;
mod transactions;
mod validation_recovery;

fn workspace(prefix: &str) -> String {
    let path = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-work-order-{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("fixture workspace should create");
    path.display().to_string()
}

fn connection(workspace: &str) -> Connection {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)
            .expect("fixture authority location should resolve");
    open_authority_connection(&location).expect("fixture authority should open")
}

fn task(task_id: &str) -> TaskDefinition {
    TaskDefinition {
        id: task_id.to_string(),
        direction_id: "direction-work-order".to_string(),
        direction_relation_note: "synthetic work-order test".to_string(),
        title: format!("Task {task_id}"),
        description: "synthetic authority test task".to_string(),
        status: TaskStatus::Ready,
        base_priority: 50,
        dynamic_priority_delta: 0,
        priority_reason: String::new(),
        depends_on: Vec::new(),
        blocked_by: Vec::new(),
        created_by: TaskActor::System,
        last_updated_by: TaskActor::System,
        source_turn_id: None,
        provenance: TaskMutationProvenance::new(OriginSessionKind::System),
        updated_at: "2026-11-20T00:00:00Z".to_string(),
    }
}

fn projection(document: &TaskAuthorityDocument) -> PriorityQueueProjection {
    let active_tasks = document
        .tasks
        .iter()
        .enumerate()
        .map(|(index, task)| PriorityQueueTask {
            rank: index + 1,
            task_id: task.id.clone(),
            direction_id: task.direction_id.clone(),
            direction_title: "Work order fixture".to_string(),
            task_title: task.title.clone(),
            status: task.status,
            combined_priority: 50,
            updated_at: task.updated_at.clone(),
            rank_reasons: vec!["synthetic fixture order".to_string()],
        })
        .collect::<Vec<_>>();
    PriorityQueueProjection {
        next_task: active_tasks.first().cloned(),
        active_tasks,
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    }
}

fn seed(workspace: &str, tasks: Vec<TaskDefinition>) -> PlanningTaskAuthoritySnapshot {
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let directions = DirectionCatalogDocument {
        version: 1,
        queue_idle: QueueIdleConfig {
            policy: QueueIdlePolicy::Stop,
            prompt_path: String::new(),
        },
        directions: vec![DirectionDefinition {
            id: "direction-work-order".to_string(),
            title: "Work order fixture".to_string(),
            summary: "synthetic direction".to_string(),
            success_criteria: vec!["one stable task".to_string()],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state: DirectionState::Active,
        }],
    };
    adapter
        .commit_direction_authority_snapshot(
            workspace,
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                directions: &directions,
                authority_mutation_owner_token: None,
            },
        )
        .expect("fixture directions should commit");
    commit_document(
        workspace,
        None,
        &TaskAuthorityDocument { version: 1, tasks },
    );
    snapshot(workspace)
}

fn snapshot(workspace: &str) -> PlanningTaskAuthoritySnapshot {
    SqlitePlanningAuthorityAdapter::new()
        .load_task_authority_snapshot(workspace)
        .expect("task authority should load")
        .expect("task authority should exist")
}

fn commit_document(workspace: &str, revision: Option<i64>, document: &TaskAuthorityDocument) {
    SqlitePlanningAuthorityAdapter::new()
        .commit_task_authority_snapshot(
            workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: revision,
                task_authority: document,
                queue_projection: &projection(document),
            },
        )
        .expect("ordinary task authority should commit");
}

fn candidate(snapshot: &PlanningTaskAuthoritySnapshot, task_id: &str) -> TaskAuthorityDocument {
    let mut next = snapshot.task_authority.clone();
    next.tasks.push(task(task_id));
    next
}

fn register_document(
    workspace: &str,
    order_id: &str,
    origin: &str,
    task_id: &str,
    revision: i64,
    document: &TaskAuthorityDocument,
) -> Result<PlanningWorkOrderRegistrationResult> {
    let task_ids = [task_id.to_string()];
    let provenance = TaskMutationProvenance::new(OriginSessionKind::System);
    SqlitePlanningAuthorityAdapter::new().register_work_order(
        workspace,
        PlanningWorkOrderRegistration {
            link_reference: None,
            work_order_id: order_id,
            origin,
            initial_task_id: task_id,
            observed_planning_revision: revision,
            task_authority: document,
            queue_projection: &projection(document),
            audit: PlanningTaskAuthorityMutationAudit {
                task_ids: &task_ids,
                legacy_source_turn_id: None,
                provenance: &provenance,
            },
        },
    )
}

fn register_next(workspace: &str, order_id: &str, task_id: &str) -> PlanningWorkOrderRecord {
    let before = snapshot(workspace);
    let result = register_document(
        workspace,
        order_id,
        "synthetic-source",
        task_id,
        before.planning_revision,
        &candidate(&before, task_id),
    )
    .expect("new work order should register");
    let PlanningWorkOrderRegistrationResult::Registered { work_order, .. } = result else {
        panic!("new work order should create a task");
    };
    work_order
}

fn stored(workspace: &str, order_id: &str) -> PlanningWorkOrderRecord {
    SqlitePlanningAuthorityAdapter::new()
        .load_work_order(workspace, order_id)
        .expect("work order should load")
        .expect("work order should exist")
}

fn metadata(workspace: &str) -> BTreeMap<String, String> {
    connection(workspace)
        .prepare("SELECT key, value FROM authority_metadata ORDER BY key")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<BTreeMap<_, _>>>()
        })
        .expect("metadata should load")
}

fn count(workspace: &str, table: &str) -> i64 {
    connection(workspace)
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("fixture row count should load")
}

#[test]
fn work_order_registration_survives_provenance_changes_event_pruning_and_restart() {
    let workspace = workspace("provenance-receipt-restart");
    let original = seed(&workspace, vec![task("legacy-task")]);
    let first = register_next(&workspace, "order-a", "task-a");
    assert_eq!(first.task_ids, vec!["task-a"]);
    assert!(first.permits.is_empty());
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let before_edit = snapshot(&workspace);
    let mut edited = before_edit.task_authority.clone();
    let original_task = edited
        .tasks
        .iter_mut()
        .find(|task| task.id == "task-a")
        .unwrap();
    original_task.title = "Operator changed the task".to_string();
    original_task.provenance = TaskMutationProvenance::new(OriginSessionKind::Planner)
        .with_thread_turn(
            Some("other-thread".to_string()),
            Some("other-turn".to_string()),
        );
    commit_document(&workspace, Some(before_edit.planning_revision), &edited);
    adapter
        .append_kernel_events(
            &workspace,
            &[RecordedKernelEvent {
                sequence: 1,
                recorded_at: "2026-11-20T00:00:00Z".to_string(),
                event: KernelEvent::TurnStarted {
                    thread_id: "synthetic-thread".to_string(),
                    turn_id: "synthetic-turn".to_string(),
                    op_id: None,
                },
            }],
        )
        .expect("synthetic receipt should persist");
    connection(&workspace)
        .execute("DELETE FROM kernel_events", [])
        .expect("fixture kernel records should prune");
    let after_edit = snapshot(&workspace);
    let after_edit_metadata = metadata(&workspace);

    // Deliberately stale revision and invalid replacement candidate must not affect a replay.
    let replay = register_document(
        &workspace,
        "order-a",
        "synthetic-source",
        "invalid candidate task",
        original.planning_revision,
        &TaskAuthorityDocument {
            version: 999,
            tasks: Vec::new(),
        },
    )
    .expect("durable identity should survive every mutable source");
    assert_eq!(
        replay,
        PlanningWorkOrderRegistrationResult::Existing {
            work_order: first.clone()
        }
    );
    assert_eq!(snapshot(&workspace), after_edit);
    assert_eq!(metadata(&workspace), after_edit_metadata);
    assert_eq!(count(&workspace, "planning_tasks"), 2);
    assert_eq!(count(&workspace, "kernel_events"), 0);
    assert_eq!(
        SqlitePlanningAuthorityAdapter::new()
            .load_work_order_for_task(&workspace, "task-a")
            .unwrap(),
        Some(first.clone())
    );

    let error = register_document(
        &workspace,
        "order-a",
        "different-origin",
        "task-a",
        -1,
        &edited,
    )
    .expect_err("same identity cannot move to another origin");
    assert_eq!(
        error.downcast_ref::<PlanningWorkOrderError>(),
        Some(&PlanningWorkOrderError::OriginConflict {
            work_order_id: "order-a".to_string()
        })
    );
    assert_eq!(stored(&workspace, "order-a"), first);
}

#[test]
fn work_order_revision_conflict_has_no_partial_task_and_retry_creates_only_one() {
    let workspace = workspace("revision-conflict");
    let observed = seed(&workspace, vec![task("legacy-task")]);
    let stale_candidate = candidate(&observed, "task-a");
    let mut concurrent = observed.task_authority.clone();
    concurrent.tasks[0].title = "Concurrent ordinary edit".to_string();
    commit_document(&workspace, Some(observed.planning_revision), &concurrent);
    let current = snapshot(&workspace);
    let before_metadata = metadata(&workspace);
    let before_audit = count(&workspace, "planning_task_mutation_events");
    assert_eq!(
        register_document(
            &workspace,
            "order-a",
            "synthetic-source",
            "task-a",
            observed.planning_revision,
            &stale_candidate,
        )
        .unwrap(),
        PlanningWorkOrderRegistrationResult::RevisionConflict {
            observed_planning_revision: observed.planning_revision,
            current_planning_revision: current.planning_revision,
        }
    );
    assert_eq!(snapshot(&workspace), current);
    assert_eq!(metadata(&workspace), before_metadata);
    assert_eq!(
        count(&workspace, "planning_task_mutation_events"),
        before_audit
    );
    assert!(
        SqlitePlanningAuthorityAdapter::new()
            .load_work_order(&workspace, "order-a")
            .unwrap()
            .is_none()
    );

    let record = register_next(&workspace, "order-a", "task-a");
    assert_eq!(count(&workspace, "planning_tasks"), 2);
    assert_eq!(
        register_document(
            &workspace,
            "order-a",
            "synthetic-source",
            "task-a",
            observed.planning_revision,
            &stale_candidate,
        )
        .unwrap(),
        PlanningWorkOrderRegistrationResult::Existing { work_order: record }
    );
    assert_eq!(count(&workspace, "planning_tasks"), 2);
}

#[test]
fn work_order_registration_rejects_changes_to_other_tasks_or_extra_creations() {
    let workspace = workspace("candidate-boundary");
    let before = seed(&workspace, vec![task("legacy-task")]);
    let mut changed_existing = candidate(&before, "task-a");
    changed_existing.tasks[0].title = "Unrelated replacement".to_string();
    let mut extra_creation = candidate(&before, "task-a");
    extra_creation.tasks.push(task("task-extra"));
    let absent_initial = candidate(&before, "different-task");
    let mut duplicate_initial = candidate(&before, "task-a");
    duplicate_initial.tasks[0] = task("task-a");
    for document in [
        changed_existing,
        extra_creation,
        absent_initial,
        duplicate_initial,
    ] {
        register_document(
            &workspace,
            "order-a",
            "synthetic-source",
            "task-a",
            before.planning_revision,
            &document,
        )
        .expect_err("registration may only append its one new task");
        assert_eq!(snapshot(&workspace), before);
        assert!(
            SqlitePlanningAuthorityAdapter::new()
                .load_work_order(&workspace, "order-a")
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn work_order_tables_are_additive_to_the_current_authority_schema() {
    // This probes additive work-order registration on the current authority schema.
    let schema_version = super::super::AUTHORITY_STORE_SCHEMA_VERSION;
    let expected_schema_version = schema_version.to_string();
    let workspace = workspace("work-order-additive-current-schema");
    let before = seed(&workspace, vec![task("legacy-task")]);
    let initial_metadata = metadata(&workspace);
    assert_eq!(
        initial_metadata.get("schema_version").map(String::as_str),
        Some(expected_schema_version.as_str())
    );
    {
        let connection = connection(&workspace);
        assert!(!table_exists(&connection, "planning_work_orders").unwrap());
        assert!(!table_exists(&connection, "planning_work_order_tasks").unwrap());
    }

    let record = register_next(&workspace, "compatibility-order", "compatibility-task");
    let after = snapshot(&workspace);
    assert_eq!(
        after.task_authority.tasks[0],
        before.task_authority.tasks[0]
    );
    assert_eq!(after.task_authority.tasks.len(), 2);
    assert_eq!(after.planning_revision, before.planning_revision + 1);
    assert_eq!(
        metadata(&workspace).get("schema_version"),
        initial_metadata.get("schema_version")
    );
    assert_eq!(
        SqlitePlanningAuthorityAdapter::new()
            .load_work_order_for_task(&workspace, "legacy-task")
            .unwrap(),
        None
    );
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace)
            .unwrap();
    println!(
        "work_order_additive_current_schema_probe={}",
        serde_json::json!({
            "workspace": workspace,
            "authority_store_path": location.authority_store_path,
            "runtime_dir": location.runtime_dir,
            "repository_identity": location.repository_identity,
            "work_order_id": record.work_order_id,
            "initial_task_id": record.initial_task_id().unwrap(),
            "planning_revision": after.planning_revision,
            "schema_version": schema_version,
            "probe": "work_order_additive_current_schema",
        })
    );
}

#[test]
fn work_order_unknown_reads_do_not_change_legacy_tasks_or_revision() {
    let workspace = workspace("legacy-unchanged");
    let before = seed(&workspace, vec![task("legacy-task")]);
    let before_metadata = metadata(&workspace);
    let before_audit = count(&workspace, "planning_task_mutation_events");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    assert_eq!(
        adapter
            .load_work_order(&workspace, "unknown-order")
            .unwrap(),
        None
    );
    assert_eq!(
        adapter
            .load_work_order_for_task(&workspace, "legacy-task")
            .unwrap(),
        None
    );
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(metadata(&workspace), before_metadata);
    assert_eq!(
        count(&workspace, "planning_task_mutation_events"),
        before_audit
    );
    let mut changed = before.task_authority.clone();
    changed.tasks[0].title = "Legacy edit remains available".to_string();
    commit_document(&workspace, Some(before.planning_revision), &changed);
    assert_eq!(snapshot(&workspace).task_authority, changed);
    assert_eq!(count(&workspace, "planning_work_orders"), 0);
}

mod admission;

mod remediation;
mod validation;

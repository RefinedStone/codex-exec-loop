use std::collections::HashMap;

use super::{PriorityQueueBuildError, PriorityQueueService};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, QueueIdleConfig, TaskActor,
    TaskAuthorityDocument, TaskDefinition, TaskStatus,
};

/*
 * Queue projection is the domain contract that converts accepted planning
 * authority into the next executable task list. These tests keep application
 * services from smuggling orchestration policy into the domain layer: the
 * service supplies accepted documents, and the queue decides ordering,
 * eligibility, skipped reasons, and invalid authority failures.
 */
fn directions(states: &[(&str, DirectionState)]) -> DirectionCatalogDocument {
    // Most tests vary only direction state. The remaining catalog fields still
    // need realistic values because prompt-facing projections carry titles,
    // summaries, success criteria, and scope hints through the same document.
    DirectionCatalogDocument {
        version: 1,
        queue_idle: QueueIdleConfig::default(),
        directions: states
            .iter()
            .map(|(id, state)| DirectionDefinition {
                id: (*id).to_string(),
                title: format!("{id} title"),
                summary: format!("{id} summary"),
                success_criteria: vec![format!("{id} done")],
                scope_hints: vec![format!("{id} hint")],
                detail_doc_path: String::new(),
                state: *state,
            })
            .collect(),
    }
}

fn task(
    id: &str,
    direction_id: &str,
    status: TaskStatus,
    base_priority: i32,
    dynamic_priority_delta: i32,
    updated_at: &str,
) -> TaskDefinition {
    // The fixture preserves all fields the production authority document owns,
    // while exposing only the queue-sensitive knobs used by the tests below.
    TaskDefinition {
        id: id.to_string(),
        direction_id: direction_id.to_string(),
        direction_relation_note: "fits direction".to_string(),
        title: format!("{id} title"),
        description: format!("{id} description"),
        status,
        base_priority,
        dynamic_priority_delta,
        priority_reason: if dynamic_priority_delta == 0 {
            String::new()
        } else {
            "recent result raised urgency".to_string()
        },
        depends_on: Vec::new(),
        blocked_by: Vec::new(),
        created_by: TaskActor::User,
        last_updated_by: TaskActor::User,
        source_turn_id: None,
        updated_at: updated_at.to_string(),
    }
}

#[test]
fn prefers_in_progress_tasks_before_ready_tasks() {
    // A resumed task represents active operator intent, so it remains the queue
    // head even when a ready task has a higher numeric priority.
    let queue_service = PriorityQueueService::new();
    let directions = directions(&[("direction-a", DirectionState::Active)]);
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![
            task(
                "task-ready-high",
                "direction-a",
                TaskStatus::Ready,
                80,
                0,
                "2026-04-09T09:00:00Z",
            ),
            task(
                "task-in-progress",
                "direction-a",
                TaskStatus::InProgress,
                10,
                0,
                "2026-04-09T10:00:00Z",
            ),
            task(
                "task-ready-low",
                "direction-a",
                TaskStatus::Ready,
                20,
                0,
                "2026-04-09T08:00:00Z",
            ),
        ],
    };
    let snapshot = queue_service
        .build_projection(&directions, &task_authority)
        .expect("queue projection should build");

    assert_eq!(
        snapshot
            .next_task
            .as_ref()
            .map(|task| task.task_id.as_str()),
        Some("task-in-progress")
    );
    assert_eq!(
        snapshot
            .active_tasks
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-in-progress", "task-ready-high", "task-ready-low"]
    );
    assert!(
        snapshot.active_tasks[0]
            .rank_reasons
            .iter()
            .any(|reason| reason == "status=in_progress")
    );
}

#[test]
fn skips_tasks_when_direction_is_inactive_or_dependencies_are_unresolved() {
    // Skipped tasks are not silently discarded. They stay visible with reasons
    // so the TUI and planning worker can explain why only the runnable subset is
    // eligible for the next handoff.
    let queue_service = PriorityQueueService::new();
    let directions = directions(&[
        ("direction-a", DirectionState::Active),
        ("direction-b", DirectionState::Paused),
    ]);
    let mut waiting_on_dependency = task(
        "waiting-on-dependency",
        "direction-a",
        TaskStatus::Ready,
        40,
        0,
        "2026-04-09T09:00:00Z",
    );
    waiting_on_dependency.depends_on = vec!["dependency-open".to_string()];
    let mut blocked_by_review = task(
        "blocked-by-review",
        "direction-a",
        TaskStatus::Ready,
        30,
        0,
        "2026-04-09T09:30:00Z",
    );
    blocked_by_review.blocked_by = vec!["review-open".to_string()];
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![
            task(
                "dependency-open",
                "direction-a",
                TaskStatus::Ready,
                90,
                0,
                "2026-04-09T08:00:00Z",
            ),
            waiting_on_dependency,
            task(
                "paused-task",
                "direction-b",
                TaskStatus::Ready,
                100,
                0,
                "2026-04-09T07:00:00Z",
            ),
            task(
                "review-open",
                "direction-a",
                TaskStatus::InProgress,
                20,
                0,
                "2026-04-09T10:00:00Z",
            ),
            blocked_by_review,
            task(
                "proposed-followup",
                "direction-a",
                TaskStatus::Proposed,
                10,
                0,
                "2026-04-09T11:00:00Z",
            ),
        ],
    };
    let snapshot = queue_service
        .build_projection(&directions, &task_authority)
        .expect("queue projection should build");

    assert_eq!(
        snapshot
            .active_tasks
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["review-open", "dependency-open"]
    );
    assert_eq!(
        snapshot
            .proposed_tasks
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["proposed-followup"]
    );
    assert_eq!(snapshot.proposed_tasks[0].combined_priority, 10);
    let skipped = snapshot
        .skipped_tasks
        .iter()
        .map(|task| (task.task_id.as_str(), task.reason.as_str()))
        .collect::<HashMap<_, _>>();
    assert_eq!(skipped.len(), 3);
    assert!(skipped["paused-task"].contains("paused"));
    assert!(skipped["waiting-on-dependency"].contains("dependency-open(ready)"));
    assert!(skipped["blocked-by-review"].contains("review-open(in_progress)"));
}

#[test]
fn excludes_non_promotable_proposals_from_proposed_queue() {
    // Proposed work is displayed only when it could be promoted by policy. A
    // paused direction or unresolved dependency turns it into an explanatory
    // skipped item instead of an operator-choice candidate.
    let queue_service = PriorityQueueService::new();
    let directions = directions(&[
        ("direction-a", DirectionState::Active),
        ("direction-b", DirectionState::Paused),
    ]);
    let mut blocked_proposal = task(
        "blocked-proposal",
        "direction-a",
        TaskStatus::Proposed,
        70,
        5,
        "2026-04-09T09:30:00Z",
    );
    blocked_proposal.depends_on = vec!["blocking-ready".to_string()];
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![
            task(
                "blocking-ready",
                "direction-a",
                TaskStatus::Ready,
                95,
                0,
                "2026-04-09T07:30:00Z",
            ),
            task(
                "ready-proposal",
                "direction-a",
                TaskStatus::Proposed,
                50,
                10,
                "2026-04-09T08:00:00Z",
            ),
            blocked_proposal,
            task(
                "paused-proposal",
                "direction-b",
                TaskStatus::Proposed,
                90,
                0,
                "2026-04-09T07:00:00Z",
            ),
        ],
    };
    let snapshot = queue_service
        .build_projection(&directions, &task_authority)
        .expect("queue projection should build");

    assert_eq!(
        snapshot
            .proposed_tasks
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["ready-proposal"]
    );
    let skipped = snapshot
        .skipped_tasks
        .iter()
        .map(|task| (task.task_id.as_str(), task.reason.as_str()))
        .collect::<HashMap<_, _>>();
    assert!(skipped["blocked-proposal"].contains("blocking-ready(ready)"));
    assert!(skipped["paused-proposal"].contains("direction direction-b is paused"));
}

#[test]
fn trims_direction_and_task_ids_for_queue_resolution() {
    // Resolution trims ids for matching but keeps original task ids in the
    // projection. That lets legacy or hand-authored documents remain resolvable
    // without rewriting the accepted authority identity.
    let queue_service = PriorityQueueService::new();
    let directions = directions(&[("direction-a", DirectionState::Active)]);
    let mut runnable_task = task(
        "  runnable-task  ",
        " direction-a ",
        TaskStatus::Ready,
        50,
        0,
        "2026-04-09T11:00:00Z",
    );
    runnable_task.depends_on = vec!["dependency-task".to_string()];
    runnable_task.blocked_by = vec![" blocker-task ".to_string()];
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![
            task(
                " dependency-task ",
                "direction-a",
                TaskStatus::Done,
                10,
                0,
                "2026-04-09T09:00:00Z",
            ),
            task(
                "blocker-task",
                "direction-a",
                TaskStatus::Done,
                20,
                0,
                "2026-04-09T10:00:00Z",
            ),
            runnable_task,
        ],
    };
    let snapshot = queue_service
        .build_projection(&directions, &task_authority)
        .expect("queue projection should build");

    assert_eq!(
        snapshot
            .next_task
            .as_ref()
            .map(|task| task.task_id.as_str()),
        Some("  runnable-task  ")
    );
    assert!(
        snapshot
            .active_tasks
            .iter()
            .any(|task| task.task_id == "  runnable-task  ")
    );
    assert!(
        snapshot
            .skipped_tasks
            .iter()
            .all(|task| task.task_id != "  runnable-task  ")
    );
}

#[test]
fn breaks_priority_ties_with_oldest_update_time() {
    // Combined priority decides the main rank. When two tasks tie, the older
    // update wins to avoid starving work that has waited longer in the ledger.
    let queue_service = PriorityQueueService::new();
    let directions = directions(&[("direction-a", DirectionState::Active)]);
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![
            task(
                "recently-updated",
                "direction-a",
                TaskStatus::Ready,
                50,
                5,
                "2026-04-09T12:00:00Z",
            ),
            task(
                "older-task",
                "direction-a",
                TaskStatus::Ready,
                45,
                10,
                "2026-04-09T08:00:00Z",
            ),
        ],
    };
    let snapshot = queue_service
        .build_projection(&directions, &task_authority)
        .expect("queue projection should build");

    assert_eq!(
        snapshot
            .active_tasks
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["older-task", "recently-updated"]
    );
    assert!(
        snapshot.active_tasks[0]
            .rank_reasons
            .iter()
            .any(|reason| reason.contains("priority_reason=recent result raised urgency"))
    );
    assert_eq!(snapshot.visible_tasks(1)[0].task_id, "older-task");
}

#[test]
fn awaiting_user_blockers_clear_for_downstream_queue_tasks() {
    // Awaiting-user blockers are informational gates for the operator, not hard
    // dependencies for later automation. Downstream executable work may proceed
    // once every non-user blocker is cleared.
    let queue_service = PriorityQueueService::new();
    let directions = directions(&[("direction-a", DirectionState::Active)]);
    let mut blocked_task = task(
        "blocked-task",
        "direction-a",
        TaskStatus::Ready,
        40,
        0,
        "2026-04-09T09:30:00Z",
    );
    blocked_task.blocked_by = vec!["user-input".to_string()];
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![
            task(
                "user-input",
                "direction-a",
                TaskStatus::AwaitingUser,
                20,
                0,
                "2026-04-09T09:00:00Z",
            ),
            blocked_task,
        ],
    };
    let snapshot = queue_service
        .build_projection(&directions, &task_authority)
        .expect("queue projection should build");

    assert_eq!(
        snapshot
            .active_tasks
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["blocked-task"]
    );
    assert!(
        snapshot
            .skipped_tasks
            .iter()
            .all(|task| task.task_id != "blocked-task")
    );
}

#[test]
fn rejects_unknown_direction_references_instead_of_skipping_them() {
    // A task pointing at a missing direction means the accepted authority graph
    // is corrupt. Failing the build is safer than hiding the task as skipped.
    let queue_service = PriorityQueueService::new();
    let directions = directions(&[("direction-a", DirectionState::Active)]);
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![task(
            "task-1",
            "missing-direction",
            TaskStatus::Ready,
            30,
            0,
            "2026-04-09T09:00:00Z",
        )],
    };
    let error = queue_service
        .build_projection(&directions, &task_authority)
        .expect_err("queue build should reject unknown directions");

    assert_eq!(
        error,
        PriorityQueueBuildError::UnknownDirection {
            task_id: "task-1".to_string(),
            direction_id: "missing-direction".to_string(),
        }
    );
}

#[test]
fn rejects_missing_dependency_references_instead_of_skipping_them() {
    // Dependency ids form the graph used for runnable decisions. Missing nodes
    // would make blocked/runnable classification guesswork, so the queue rejects
    // the authority document before producing a projection.
    let queue_service = PriorityQueueService::new();
    let directions = directions(&[("direction-a", DirectionState::Active)]);
    let mut blocked_task = task(
        "blocked-task",
        "direction-a",
        TaskStatus::Ready,
        30,
        0,
        "2026-04-09T09:00:00Z",
    );
    blocked_task.depends_on = vec!["missing-task".to_string()];
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![blocked_task],
    };
    let error = queue_service
        .build_projection(&directions, &task_authority)
        .expect_err("queue build should reject missing dependency references");

    assert_eq!(
        error,
        PriorityQueueBuildError::MissingDependency {
            task_id: "blocked-task".to_string(),
            dependency_id: "missing-task".to_string(),
        }
    );
}

#[test]
fn rejects_invalid_updated_at_instead_of_silently_reordering() {
    // Tie-breaking depends on timestamps. Invalid updated_at values therefore
    // fail closed rather than falling back to input order or current time.
    let queue_service = PriorityQueueService::new();
    let directions = directions(&[("direction-a", DirectionState::Active)]);
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![task(
            "task-1",
            "direction-a",
            TaskStatus::Ready,
            30,
            0,
            "not-a-timestamp",
        )],
    };
    let error = queue_service
        .build_projection(&directions, &task_authority)
        .expect_err("queue build should reject invalid updated_at values");

    assert_eq!(
        error,
        PriorityQueueBuildError::InvalidUpdatedAt {
            task_id: "task-1".to_string(),
            updated_at: "not-a-timestamp".to_string(),
        }
    );
}

#[test]
fn rejects_multiple_in_progress_tasks_during_queue_build() {
    // The queue can hand off only one active execution lane. Multiple
    // in_progress records indicate ledger drift that orchestration must repair
    // before the domain can choose a next task.
    let queue_service = PriorityQueueService::new();
    let directions = directions(&[("direction-a", DirectionState::Active)]);
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![
            task(
                "task-1",
                "direction-a",
                TaskStatus::InProgress,
                30,
                0,
                "2026-04-09T09:00:00Z",
            ),
            task(
                "task-2",
                "direction-a",
                TaskStatus::InProgress,
                20,
                0,
                "2026-04-09T09:10:00Z",
            ),
        ],
    };
    let error = queue_service
        .build_projection(&directions, &task_authority)
        .expect_err("queue build should reject multiple in_progress tasks");

    assert_eq!(
        error,
        PriorityQueueBuildError::MultipleInProgressTasks {
            task_ids: vec!["task-1".to_string(), "task-2".to_string()],
        }
    );
}

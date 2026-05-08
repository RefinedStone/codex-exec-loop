use std::collections::HashMap;

use super::{PriorityQueueBuildError, PriorityQueueService};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, QueueIdleConfig, TaskActor,
    TaskAuthorityDocument, TaskDefinition, TaskStatus,
};

/*
 * queue projection은 accepted planning authority를 다음 executable task list로 바꾸는
 * domain contract다. application service는 이미 승인된 direction/task 문서를 넘기고,
 * queue domain은 ordering, eligibility, skipped reason, invalid authority failure만 판단한다.
 * 이 테스트들은 orchestration policy가 domain layer로 새어 들어오지 않게 하는 방어선이다.
 */
fn directions(states: &[(&str, DirectionState)]) -> DirectionCatalogDocument {
    // 대부분의 테스트는 direction state만 바꾼다. 그래도 prompt-facing projection이 title,
    // summary, success criteria, scope hints를 같은 document에서 운반하므로 나머지 catalog
    // field도 현실적인 값으로 채운다.
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
    // production authority document가 가진 모든 field를 보존하되, 호출부에는 queue-sensitive
    // knob인 status/priority/timestamp만 노출한다. fixture noise 없이 queue 정책만 읽히게 한다.
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
        provenance: Default::default(),
        updated_at: updated_at.to_string(),
    }
}

#[test]
fn prefers_in_progress_tasks_before_ready_tasks() {
    // resumed task는 이미 시작된 operator intent를 뜻한다. ready task의 numeric priority가 더
    // 높더라도 실행 중이던 일을 먼저 이어가야 하므로 queue head에 남는다.
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
    // skipped task는 조용히 버려지지 않는다. TUI와 planning worker가 왜 runnable subset만
    // 다음 handoff 후보인지 설명할 수 있도록 reason과 함께 projection에 남긴다.
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
    // proposed work는 policy가 실제로 promote할 수 있을 때만 proposal queue에 보인다.
    // paused direction이나 unresolved dependency가 있으면 operator-choice candidate가 아니라
    // 설명 가능한 skipped item으로 내려간다.
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
fn keeps_executable_queue_and_proposal_queue_separate_when_both_have_candidates() {
    /*
     * proposed task priority가 ready task보다 높아도 자동 실행 queue head가 되면 안 된다.
     * proposal classification은 domain queue projection의 별도 lane이며, application/TUI는
     * 이 분리를 다시 계산하지 않고 active/proposed list를 그대로 표시해야 한다.
     */
    let queue_service = PriorityQueueService::new();
    let directions = directions(&[("direction-a", DirectionState::Active)]);
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![
            task(
                "ready-low",
                "direction-a",
                TaskStatus::Ready,
                40,
                0,
                "2026-04-09T09:00:00Z",
            ),
            task(
                "proposal-high",
                "direction-a",
                TaskStatus::Proposed,
                100,
                0,
                "2026-04-09T07:00:00Z",
            ),
            task(
                "ready-high",
                "direction-a",
                TaskStatus::Ready,
                80,
                0,
                "2026-04-09T10:00:00Z",
            ),
            task(
                "proposal-low",
                "direction-a",
                TaskStatus::Proposed,
                60,
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
        Some("ready-high")
    );
    assert_eq!(
        snapshot
            .active_tasks
            .iter()
            .map(|task| (task.rank, task.task_id.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "ready-high"), (2, "ready-low")]
    );
    assert_eq!(
        snapshot
            .proposed_tasks
            .iter()
            .map(|task| (task.rank, task.task_id.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "proposal-high"), (2, "proposal-low")]
    );
}

#[test]
fn trims_direction_and_task_ids_for_queue_resolution() {
    // resolution은 matching할 때 id를 trim하지만 projection에는 원래 task id를 보존한다.
    // legacy/hand-authored 문서를 accepted authority identity rewrite 없이 해석하기 위한 절충이다.
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
    // main rank는 combined priority가 결정한다. 동점이면 더 오래 ledger에 머문 작업을 굶기지
    // 않기 위해 older updated_at이 앞선다.
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
    // AwaitingUser blocker는 operator에게 보이는 informational gate이지 이후 automation을
    // 막는 hard dependency가 아니다. user blocker 외의 blocker가 정리됐다면 downstream work는 진행할 수 있다.
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
    // missing direction을 가리키는 task는 accepted authority graph 자체가 깨졌다는 신호다.
    // 이를 skipped로 숨기기보다 build를 실패시켜 repair 경로가 authority 부패를 보게 한다.
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
    // dependency id는 runnable decision에 쓰이는 graph edge다. missing node가 있으면
    // blocked/runnable 분류가 추측이 되므로 projection을 만들기 전에 authority document를 거부한다.
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
    // tie-breaking은 timestamp에 의존한다. invalid updated_at을 input order나 current time으로
    // 대체하면 ranking이 숨은 정책을 갖게 되므로 fail-closed로 처리한다.
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
    // queue는 하나의 active execution lane만 handoff할 수 있다. 여러 in_progress record는
    // domain이 queue head를 고르기 전에 orchestration이 repair해야 하는 ledger drift다.
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

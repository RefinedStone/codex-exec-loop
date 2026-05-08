use super::{PlanningChangeSet, PlanningExecutionSnapshot, execution_snapshot_to_workspace_record};
use super::{PlanningRepairRequest, build_planning_repair_prompt};
use crate::domain::planning::repair_candidate::{
    PlanningRepairCandidatePolicy, PlanningRepairPreviousHandoff,
};
use crate::domain::planning::{
    PLANNING_FORMAT_VERSION, PriorityQueueProjection, PriorityQueueTask, TaskActor,
    TaskAuthorityDocument, TaskDefinition, TaskStatus,
};

/*
 * 이 파일은 복구 워커가 생성한 planning_task_commands 주변의 방어 계약을 고정한다.
 * 현재 운영 경로는 보호 파일을 복원하는 수준에 머물러 있지만, 테스트는 이후 DB 권한
 * 후보를 직접 조정하게 될 때도 지켜야 하는 조건을 먼저 문서화한다. 거절된 후보는
 * 이미 승인된 DB 작업을 지우거나, 완료된 상태를 되돌리거나, 이전 ready handoff를
 * 그대로 반복하면서 진행된 것처럼 보이면 안 된다.
 */

/*
 * 복구 프롬프트는 전체 권한 문서 교체가 아니라 planning_task_commands payload를 요구해야 한다.
 * 이렇게 해야 복구 워커가 일반 작업 도구와 같은 mutation 계약 안에 머물며, 예전 파일 기반
 * task-ledger/queue-snapshot 표현이 생성 프롬프트에 다시 들어오지 않는다.
 */
#[test]
fn repair_prompt_requests_task_command_payload_from_db_authority() {
    let prompt = build_planning_repair_prompt(
        &PlanningRepairRequest {
            failure_summary:
                "planning worker returned invalid planning_task_commands: missing field `op`"
                    .to_string(),
            validation_errors: vec![
                "planning worker returned invalid planning_task_commands: missing field `op`"
                    .to_string(),
            ],
            direction_authority_json: "{\"version\":1,\"directions\":[]}".to_string(),
            accepted_task_authority_json: "{\"version\":1,\"tasks\":[]}".to_string(),
            accepted_queue_projection_json:
                "{\"next_task\":null,\"active_tasks\":[],\"proposed_tasks\":[],\"skipped_tasks\":[]}"
                    .to_string(),
            rejected_task_authority_json: Some(
                "{\"planning_task_commands\":{\"version\":1,\"commands\":[{\"create_task\":{\"title\":\"Queue follow-up\"}}]}}"
                    .to_string(),
            ),
            rejected_archive_path: None,
        },
        None,
        1,
        2,
        None,
    );

    assert!(prompt.contains("\"planning_task_commands\""));
    assert!(prompt.contains("\"op\":\"create_task\""));
    assert!(prompt.contains("Do not wrap commands"));
    assert!(prompt.contains("preserve the same task intent"));
    assert!(prompt.contains("[rejected-candidate]"));
    assert!(prompt.contains("\"create_task\""));
    assert!(prompt.contains("Do not return `task_authority`"));
    assert!(prompt.contains("[accepted-db-queue-projection]"));
    assert!(prompt.contains("last accepted DB snapshot"));
    assert!(!prompt.contains("task-ledger.json"));
    assert!(!prompt.contains("task authority schema file"));
    assert!(!prompt.contains("queue snapshot artifact"));
}

// post-turn reconciliation은 active result-output 변경만 복구 대상으로 본다.
#[test]
fn reconciliation_change_set_only_tracks_active_result_output_paths() {
    let changed_paths = vec![
        "/tmp/workspace/.codex-exec-loop/planning/result-output.md".to_string(),
        ".codex-exec-loop/planning/DB task authority".to_string(),
        "src/main.rs".to_string(),
    ];
    let change_set = PlanningChangeSet::from_paths(&changed_paths);

    assert!(change_set.result_output_changed);

    let unrelated_paths = vec![
        ".codex-exec-loop/planning/prompts/queue-idle-review.md".to_string(),
        ".codex-exec-loop/planning/directions/core.md".to_string(),
    ];
    let unrelated_change_set = PlanningChangeSet::from_paths(&unrelated_paths);

    assert!(!unrelated_change_set.has_relevant_changes());
}

// protected-file restore payload는 pre-turn snapshot을 그대로 workspace port 계약으로 낮춘다.
#[test]
fn execution_snapshot_restore_payload_preserves_absent_result_output() {
    let record = execution_snapshot_to_workspace_record(&PlanningExecutionSnapshot {
        result_output_markdown: None,
    });

    assert_eq!(record.result_output_markdown, None);
}

// 이전 ready handoff를 변경 없이 다시 내보내면 자동 후속 복구 루프가 생긴다.
#[test]
fn queue_advancement_guard_rejects_unchanged_previous_handoff_head() {
    let accepted = TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: vec![task("task-1", "ready", "2026-04-29T00:00:00Z")],
    };
    let projection = PriorityQueueProjection {
        next_task: Some(queue_task("task-1", TaskStatus::Ready)),
        active_tasks: vec![queue_task("task-1", TaskStatus::Ready)],
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };
    let failure = PlanningRepairCandidatePolicy::new().queue_advancement_failure(
        Some(PlanningRepairPreviousHandoff {
            task_id: "task-1",
            task_title: "Task 1",
            updated_at: "2026-04-29T00:00:00Z",
            status_label: "ready",
        }),
        Some(&accepted),
        &accepted,
        &projection,
    );

    assert_eq!(
        failure.as_deref(),
        Some(
            "planning worker refresh kept previous handoff `task-1` unchanged as the ready queue head"
        )
    );
}

// 같은 큐 head라도 후보 권한에 실제 작업 갱신이 기록되어 있으면 진행으로 인정한다.
#[test]
fn queue_advancement_guard_allows_updated_same_head() {
    let accepted = TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: vec![task("task-1", "ready", "2026-04-29T00:00:00Z")],
    };
    let candidate = TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: vec![task("task-1", "ready", "2026-04-29T00:01:00Z")],
    };
    let projection = PriorityQueueProjection {
        next_task: Some(queue_task("task-1", TaskStatus::Ready)),
        active_tasks: vec![queue_task("task-1", TaskStatus::Ready)],
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };
    let failure = PlanningRepairCandidatePolicy::new().queue_advancement_failure(
        Some(PlanningRepairPreviousHandoff {
            task_id: "task-1",
            task_title: "Task 1",
            updated_at: "2026-04-29T00:00:00Z",
            status_label: "ready",
        }),
        Some(&accepted),
        &candidate,
        &projection,
    );

    assert_eq!(failure, None);
}

// 후보 권한은 이미 승인된 terminal 작업을 ready/proposed 상태로 되돌릴 수 없다.
#[test]
fn stale_candidate_guard_rejects_accepted_db_status_regression() {
    let accepted = TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: vec![
            task(
                "planning-prompt-assembly-remaining-surface-slice",
                "done",
                "2026-04-29T03:00:32Z",
            ),
            task(
                "planning-prompt-shared-section-catalog-slice",
                "ready",
                "2026-04-29T03:00:32Z",
            ),
        ],
    };
    let stale_candidate = TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: vec![
            task(
                "planning-prompt-assembly-remaining-surface-slice",
                "ready",
                "2026-04-29T01:43:52Z",
            ),
            task(
                "planning-prompt-shared-section-catalog-slice",
                "proposed",
                "2026-04-29T01:43:52Z",
            ),
        ],
    };
    let failure = PlanningRepairCandidatePolicy::new()
        .stale_candidate_failure(Some(&accepted), &stale_candidate);

    assert_eq!(
        failure.as_deref(),
        Some(
            "planning worker task authority candidate regressed accepted DB task `planning-prompt-assembly-remaining-surface-slice` from `done` to `ready`"
        )
    );
}

// 승인된 DB row가 더 최신이면 상태가 같아도 생성 후보는 stale로 취급한다.
#[test]
fn stale_candidate_guard_rejects_older_accepted_db_timestamp() {
    let accepted = TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: vec![task("task-1", "ready", "2026-04-29T03:00:32Z")],
    };
    let stale_candidate = TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: vec![task("task-1", "ready", "2026-04-29T01:43:52Z")],
    };
    let failure = PlanningRepairCandidatePolicy::new()
        .stale_candidate_failure(Some(&accepted), &stale_candidate);

    assert_eq!(
        failure.as_deref(),
        Some(
            "planning worker task authority candidate regressed accepted DB task `task-1` updated_at from `2026-04-29T03:00:32Z` to `2026-04-29T01:43:52Z`"
        )
    );
}

// RFC3339 비교는 문자열 순서가 아니라 시각 기준이므로, 더 늦은 fractional seconds 값은 안전하다.
#[test]
fn stale_candidate_guard_compares_rfc3339_timestamps_by_time() {
    let accepted = TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: vec![task("task-1", "ready", "2026-04-29T03:00:32+00:00")],
    };
    let candidate = TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: vec![task("task-1", "ready", "2026-04-29T03:00:32.500Z")],
    };
    let failure =
        PlanningRepairCandidatePolicy::new().stale_candidate_failure(Some(&accepted), &candidate);

    assert_eq!(failure, None);
}

// 작은 작업 fixture로 각 가드 테스트의 관심사를 id/status/update-time 차이에 묶어 둔다.
fn task(id: &str, status: &str, updated_at: &str) -> TaskDefinition {
    TaskDefinition {
        id: id.to_string(),
        direction_id: "direction-a".to_string(),
        direction_relation_note: "supports direction".to_string(),
        title: "Task 1".to_string(),
        description: "Do task 1".to_string(),
        status: match status {
            "ready" => TaskStatus::Ready,
            "done" => TaskStatus::Done,
            "proposed" => TaskStatus::Proposed,
            _ => panic!("unexpected status"),
        },
        base_priority: 10,
        dynamic_priority_delta: 0,
        priority_reason: String::new(),
        depends_on: Vec::new(),
        blocked_by: Vec::new(),
        created_by: TaskActor::Worker,
        last_updated_by: TaskActor::Worker,
        source_turn_id: None,
        provenance: Default::default(),
        updated_at: updated_at.to_string(),
    }
}

// 큐 fixture는 전체 projection 서비스를 다시 만들지 않고 advancement 테스트에 ready head를 제공한다.
fn queue_task(id: &str, status: TaskStatus) -> PriorityQueueTask {
    PriorityQueueTask {
        rank: 1,
        task_id: id.to_string(),
        direction_id: "direction-a".to_string(),
        direction_title: "Direction A".to_string(),
        task_title: "Task 1".to_string(),
        status,
        combined_priority: 10,
        updated_at: "2026-04-29T00:00:00Z".to_string(),
        rank_reasons: vec!["status=ready".to_string()],
    }
}

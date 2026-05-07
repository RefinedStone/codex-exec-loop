use super::{PlanningRepairPromptHandoff, PlanningRepairRequest, build_planning_repair_prompt};
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

// 복구 오케스트레이션의 stale-candidate 진단 문구를 테스트 안에 고정한 미러 가드다.
fn stale_candidate_guard_failure(
    accepted_task_authority: Option<&TaskAuthorityDocument>,
    candidate_task_authority: &TaskAuthorityDocument,
) -> Option<String> {
    let accepted_task_authority = accepted_task_authority?;
    for accepted_task in &accepted_task_authority.tasks {
        let task_id = accepted_task.id.trim();
        let Some(candidate_task) = find_task(candidate_task_authority, task_id) else {
            return Some(format!(
                "planner task authority candidate removed accepted DB task `{task_id}`"
            ));
        };
        if terminal_status(accepted_task.status) && candidate_task.status != accepted_task.status {
            return Some(format!(
                "planner task authority candidate regressed accepted DB task `{task_id}` from `{}` to `{}`",
                accepted_task.status.label(),
                candidate_task.status.label()
            ));
        }
        if timestamp_regressed(&candidate_task.updated_at, &accepted_task.updated_at) {
            return Some(format!(
                "planner task authority candidate regressed accepted DB task `{task_id}` updated_at from `{}` to `{}`",
                accepted_task.updated_at.trim(),
                candidate_task.updated_at.trim()
            ));
        }
    }
    None
}

// 복구 후보는 완료/취소된 작업을 다시 진행 가능한 상태로 되돌릴 수 없다.
fn terminal_status(status: TaskStatus) -> bool {
    matches!(status, TaskStatus::Done | TaskStatus::Cancelled)
}

// 날짜 형식 검증은 호출자 책임이므로, 비어 있거나 파싱 불가능한 값은 회귀 판정에서 제외한다.
fn timestamp_regressed(candidate_updated_at: &str, accepted_updated_at: &str) -> bool {
    let candidate_updated_at = candidate_updated_at.trim();
    let accepted_updated_at = accepted_updated_at.trim();
    if candidate_updated_at.is_empty() || accepted_updated_at.is_empty() {
        return false;
    }
    let Ok(candidate_updated_at) = chrono::DateTime::parse_from_rfc3339(candidate_updated_at)
    else {
        return false;
    };
    let Ok(accepted_updated_at) = chrono::DateTime::parse_from_rfc3339(accepted_updated_at) else {
        return false;
    };

    candidate_updated_at < accepted_updated_at
}

/*
 * ready 큐 head를 그대로 다시 내보내는 복구 루프를 막는 가드다.
 * 같은 작업이 계속 head에 남아도, 승인된 DB 기준선과 비교해 해당 권한 row가 실제로
 * 바뀌었다면 허용된다. 그렇지 않으면 워커가 이전 handoff를 소비하지 못한 상태에서
 * 자동 후속 복구만 반복하게 된다.
 */
fn queue_advancement_guard_failure(
    previous_handoff: Option<PlanningRepairPromptHandoff<'_>>,
    accepted_task_authority: Option<&TaskAuthorityDocument>,
    candidate_task_authority: &TaskAuthorityDocument,
    queue_projection: &PriorityQueueProjection,
) -> Option<String> {
    let previous_handoff = previous_handoff?;
    let queue_head = queue_projection.next_task.as_ref()?;
    if queue_head.task_id.trim() != previous_handoff.task_id.trim() {
        return None;
    }
    let accepted_task = accepted_task_authority
        .and_then(|task_authority| find_task(task_authority, previous_handoff.task_id));
    let candidate_task = find_task(candidate_task_authority, previous_handoff.task_id)?;
    match accepted_task {
        Some(accepted_task)
            if accepted_task.normalized() == candidate_task.normalized()
                && queue_head.status.label() == previous_handoff.status_label.trim() =>
        {
            Some(format!(
                "planner refresh kept previous handoff `{}` unchanged as the ready queue head",
                previous_handoff.task_id.trim()
            ))
        }
        None if candidate_task.updated_at.trim() == previous_handoff.updated_at.trim() => {
            Some(format!(
                "planner refresh returned previous handoff `{}` as the queue head without DB baseline evidence of a task update",
                previous_handoff.task_id.trim()
            ))
        }
        _ => None,
    }
}

// 프롬프트와 JSON 후보에는 공백이 섞일 수 있으므로, 운영 코드처럼 id를 trim해서 찾는다.
fn find_task<'a>(
    task_authority: &'a TaskAuthorityDocument,
    task_id: &str,
) -> Option<&'a TaskDefinition> {
    let task_id = task_id.trim();
    task_authority
        .tasks
        .iter()
        .find(|task| task.id.trim() == task_id)
}

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
    let failure = queue_advancement_guard_failure(
        Some(PlanningRepairPromptHandoff {
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
        Some("planner refresh kept previous handoff `task-1` unchanged as the ready queue head")
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
    let failure = queue_advancement_guard_failure(
        Some(PlanningRepairPromptHandoff {
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
    let failure = stale_candidate_guard_failure(Some(&accepted), &stale_candidate);

    assert_eq!(
        failure.as_deref(),
        Some(
            "planner task authority candidate regressed accepted DB task `planning-prompt-assembly-remaining-surface-slice` from `done` to `ready`"
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
    let failure = stale_candidate_guard_failure(Some(&accepted), &stale_candidate);

    assert_eq!(
        failure.as_deref(),
        Some(
            "planner task authority candidate regressed accepted DB task `task-1` updated_at from `2026-04-29T03:00:32Z` to `2026-04-29T01:43:52Z`"
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
    let failure = stale_candidate_guard_failure(Some(&accepted), &candidate);

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
        created_by: TaskActor::Llm,
        last_updated_by: TaskActor::Llm,
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

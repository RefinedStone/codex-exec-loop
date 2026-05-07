use super::{
    PlanningAutoFollowBlockReason, PlanningAutoFollowPolicyDecision, PlanningAutoFollowPromptMode,
    PlanningRuntimePolicyService, PlanningRuntimeRepairAttempt,
    PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
    PlanningRuntimeSummaryRequest,
};
use crate::application::service::planning::runtime::prompt::PlanningRuntimeSnapshot;
use crate::domain::planning::{
    PlanningWorkspaceState, PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask,
    TaskStatus,
};

// runtime policy가 필요로 하는 가장 작은 actionable queue head fixture다. direction authority와
// ranking metadata를 가진 ready task만 만들고 full catalog는 만들지 않아, 실패 원인이
// queue construction이 아니라 policy projection 자체에 머물게 한다.
fn queue_head() -> PriorityQueueTask {
    PriorityQueueTask {
        rank: 1,
        task_id: "task-1".to_string(),
        direction_id: "general-workstream".to_string(),
        direction_title: "General workstream".to_string(),
        task_title: "Implement queue-aware policy".to_string(),
        status: TaskStatus::Ready,
        combined_priority: 10,
        updated_at: "2026-04-10T00:00:00Z".to_string(),
        rank_reasons: vec!["status=ready".to_string()],
    }
}

/*
 * auto follow-up 허용 조건은 "planning이 valid하다"보다 좁다. snapshot에 아직 handoff되지
 * 않은 actionable queue head가 있을 때만 generated continuation을 만든다는 queue-driven
 * 계약을 이 테스트 묶음이 고정한다.
 */
#[test]
fn builtin_next_task_blocks_when_planning_is_uninitialized() {
    /*
     * uninitialized planning file은 queue authority 자체가 없다. policy는 이를 empty ready
     * workspace와 같은 actionable-queue gate로 접어, caller가 bootstrap 전 상태를 task 생성
     * 허가처럼 특수 처리하지 못하게 한다.
     */
    let service = PlanningRuntimePolicyService::new();
    let snapshot = PlanningRuntimeSnapshot::uninitialized();
    let decision = service.decide_auto_follow(&snapshot);

    assert_eq!(
        decision,
        PlanningAutoFollowPolicyDecision::Blocked(
            PlanningAutoFollowBlockReason::ActionableQueueRequired
        )
    );
    assert_eq!(
        service
            .build_preview_view_for_decision(decision, &snapshot)
            .status_label,
        "queue-empty"
    );
}

#[test]
fn builtin_next_task_blocks_main_prompt_when_queue_is_empty_with_proposals() {
    /*
     * proposal summary는 advisory inventory이지 executable queue head가 아니다. main prompt
     * path가 promote 가능한 아이디어를 이미 승인된 작업으로 취급하지 않게 막으면서도, preview에는
     * proposal detail을 남겨 operator가 다음 행동을 볼 수 있게 한다.
     */
    let service = PlanningRuntimePolicyService::new();
    let snapshot = PlanningRuntimeSnapshot::ready_with_details(
        "Planning Context".to_string(),
        "queue idle: no executable planning task".to_string(),
        Some("2 promotable follow-up proposals available: Plan A | +1 more".to_string()),
        None,
    );
    let decision = service.decide_auto_follow(&snapshot);

    assert_eq!(
        decision,
        PlanningAutoFollowPolicyDecision::Blocked(
            PlanningAutoFollowBlockReason::ActionableQueueRequired
        )
    );
    let preview = service.build_preview_view_for_decision(decision, &snapshot);

    assert_eq!(preview.status_label, "queue-empty");
    assert!(preview.detail.as_deref().is_some_and(|detail| {
        detail.contains("queue-driven auto follow-up requires an actionable planning queue head")
            && detail.contains("promotable follow-up proposals available")
    }));
}

#[test]
fn builtin_next_task_blocks_ready_no_task_state_without_existing_proposals() {
    /*
     * proposal도 없는 ReadyNoTask는 조용한 idle case다. planning file은 valid하지만 다음
     * assistant turn에 넘길 direction-owned task가 없다는 점을 automation gate가 표현한다.
     */
    let service = PlanningRuntimePolicyService::new();
    let snapshot = PlanningRuntimeSnapshot::ready_with_details(
        "Planning Context".to_string(),
        "queue idle: no executable planning task".to_string(),
        None,
        None,
    );
    let decision = service.decide_auto_follow(&snapshot);

    assert_eq!(
        decision,
        PlanningAutoFollowPolicyDecision::Blocked(
            PlanningAutoFollowBlockReason::ActionableQueueRequired
        )
    );
    assert_eq!(
        service
            .build_preview_view_for_decision(decision, &snapshot)
            .status_label,
        "queue-empty"
    );
}

#[test]
fn ready_no_task_snapshot_is_drained_only_when_remaining_work_is_terminal() {
    let drained_snapshot = PlanningRuntimeSnapshot::ready_with_queue_projection(
        "Planning Context".to_string(),
        "queue idle: no executable planning task".to_string(),
        None,
        None,
        PriorityQueueProjection {
            next_task: None,
            active_tasks: Vec::new(),
            proposed_tasks: Vec::new(),
            skipped_tasks: vec![PriorityQueueSkippedTask {
                task_id: "done-task".to_string(),
                task_title: "Finished slice".to_string(),
                direction_id: "general-workstream".to_string(),
                status: TaskStatus::Done,
                reason: "status done is not executable".to_string(),
            }],
        },
    );
    assert!(drained_snapshot.queue_is_drained());

    let blocked_snapshot = PlanningRuntimeSnapshot::ready_with_queue_projection(
        "Planning Context".to_string(),
        "queue idle: no executable planning task".to_string(),
        None,
        None,
        PriorityQueueProjection {
            next_task: None,
            active_tasks: Vec::new(),
            proposed_tasks: Vec::new(),
            skipped_tasks: vec![PriorityQueueSkippedTask {
                task_id: "blocked-task".to_string(),
                task_title: "Waiting slice".to_string(),
                direction_id: "general-workstream".to_string(),
                status: TaskStatus::Blocked,
                reason: "blocked by tasks: task-1(in_progress)".to_string(),
            }],
        },
    );
    assert!(!blocked_snapshot.queue_is_drained());
}

#[test]
fn builtin_next_task_blocks_when_queue_head_and_proposals_are_both_missing() {
    /*
     * uninitialized snapshot의 preview detail도 actionable queue head 요구를 설명해야 한다.
     * 이 회귀는 queue/proposal이 모두 없을 때 caller가 빈 detail을 받아 TUI 안내를 잃지 않게 한다.
     */
    let service = PlanningRuntimePolicyService::new();
    let snapshot = PlanningRuntimeSnapshot::uninitialized();
    let decision = service.decide_auto_follow(&snapshot);

    assert_eq!(
        decision,
        PlanningAutoFollowPolicyDecision::Blocked(
            PlanningAutoFollowBlockReason::ActionableQueueRequired
        )
    );
    assert!(
        service
            .build_preview_view_for_decision(decision, &snapshot)
            .detail
            .as_deref()
            .is_some_and(|detail| {
                detail.contains(
                    "queue-driven auto follow-up requires an actionable planning queue head",
                )
            })
    );
}

#[test]
fn repeated_queue_head_blocks_queue_driven_automation() {
    /*
     * pause reason은 handoff 이후 prompt snapshot에서 온다. 이를 hard block으로 유지해야
     * planning refresh가 queue head를 진전시키지 못한 상황에서 runtime loop가 같은 task를
     * 반복 continuation하지 않는다.
     */
    let service = PlanningRuntimePolicyService::new();
    let snapshot = PlanningRuntimeSnapshot::ready(
        "Planning Context".to_string(),
        "next task: rank 1 / task-1".to_string(),
        Some(queue_head()),
    )
    .with_auto_followup_pause_reason(
        "planning worker refresh kept the previously handed-off task as the queue head",
    );

    assert_eq!(
        service.decide_auto_follow(&snapshot),
        PlanningAutoFollowPolicyDecision::Blocked(PlanningAutoFollowBlockReason::RepeatedQueueHead)
    );
}

#[test]
fn builtin_next_task_never_builds_main_refresh_prompt_when_queue_is_idle() {
    /*
     * queue idle 상태에서는 proposal이 있어도 main refresh prompt를 만들지 않는다. proposal을
     * 실행하려면 먼저 promote/queue intent를 거쳐 authority에 반영되어야 한다는 정책을 반복 확인한다.
     */
    let service = PlanningRuntimePolicyService::new();
    let snapshot = PlanningRuntimeSnapshot::ready_with_details(
        "Planning Context".to_string(),
        "queue idle: no executable planning task".to_string(),
        Some("2 promotable follow-up proposals available: Plan A | +1 more".to_string()),
        None,
    );

    assert_eq!(
        service.decide_auto_follow(&snapshot),
        PlanningAutoFollowPolicyDecision::Blocked(
            PlanningAutoFollowBlockReason::ActionableQueueRequired
        )
    );
}

#[test]
fn ready_queue_head_uses_continue_mode() {
    /*
     * 유일한 positive auto-follow path다. 실제 queue head가 있는 valid snapshot만 continuation
     * prompt로 변환된다. proposal이나 idle policy는 먼저 queue_head로 promote되지 않으면 이
     * branch에 도달할 수 없다.
     */
    let service = PlanningRuntimePolicyService::new();
    let snapshot = PlanningRuntimeSnapshot::ready(
        "Planning Context".to_string(),
        "next task: rank 1 / task-1".to_string(),
        Some(queue_head()),
    );

    assert_eq!(
        service.decide_auto_follow(&snapshot),
        PlanningAutoFollowPolicyDecision::QueuePrompt(
            PlanningAutoFollowPromptMode::ContinueQueuedTask
        )
    );
}

/*
 * summary projection은 정적인 snapshot 위에 live runtime state를 덮어쓴다. planning file이
 * ready여도 현재 앱이 turn 실행 중이거나 repair 중이면 TUI는 file validity보다 현재 실행 상태를
 * 우선해서 보여 줘야 한다.
 */
#[test]
fn summary_view_marks_running_ready_planning_as_executing() {
    /*
     * queue head가 있는 ready snapshot도 running turn overlay가 있으면 Executing으로 보인다.
     * status_label의 stale 표현은 planning authority가 아니라 현재 turn 결과를 기다리는 상태임을 나타낸다.
     */
    let service = PlanningRuntimePolicyService::new();
    let snapshot = PlanningRuntimeSnapshot::ready(
        "Planning Context".to_string(),
        "next task: rank 1 / task-1".to_string(),
        Some(queue_head()),
    );
    let summary = service.build_summary_view(PlanningRuntimeSummaryRequest {
        snapshot: &snapshot,
        has_running_turn: true,
        is_repairing: false,
        repair_failure_summary: None,
    });

    assert_eq!(summary.workspace_state, PlanningWorkspaceState::Executing);
    assert_eq!(summary.status_label, "stale");
    assert_eq!(
        summary.queue_summary.as_deref(),
        Some("next task: rank 1 / task-1")
    );
}

#[test]
fn summary_view_keeps_proposal_summary_when_present() {
    /*
     * queue가 idle이어도 proposal context는 summary projection에서 살아남는다. TUI는 review
     * affordance를 위해 이 정보를 필요로 하지만, automation은 위의 missing queue head 테스트처럼
     * 계속 block되어야 한다.
     */
    let service = PlanningRuntimePolicyService::new();
    let snapshot = PlanningRuntimeSnapshot::ready_with_details(
        "Planning Context".to_string(),
        "queue idle: no executable planning task".to_string(),
        Some("1 promotable follow-up proposal available: Draft sushi roadmap".to_string()),
        None,
    );
    let summary = service.build_summary_view(PlanningRuntimeSummaryRequest {
        snapshot: &snapshot,
        has_running_turn: false,
        is_repairing: false,
        repair_failure_summary: None,
    });

    assert_eq!(summary.workspace_state, PlanningWorkspaceState::Ready);
    assert_eq!(
        summary.proposal_summary.as_deref(),
        Some("1 promotable follow-up proposal available: Draft sushi roadmap")
    );
}

#[test]
fn summary_view_prefers_repair_failure_when_present() {
    /*
     * live repair error는 snapshot validation text보다 우선한다. visible failure가 오래된
     * file-load diagnostic이 아니라 가장 최근 automatic repair attempt에 묶이게 하기 위한 순서다.
     */
    let service = PlanningRuntimePolicyService::new();
    let snapshot =
        PlanningRuntimeSnapshot::invalid("planning validation failed: task authority".to_string());
    let summary = service.build_summary_view(PlanningRuntimeSummaryRequest {
        snapshot: &snapshot,
        has_running_turn: false,
        is_repairing: true,
        repair_failure_summary: Some("task authority is missing direction_id"),
    });

    assert_eq!(summary.workspace_state, PlanningWorkspaceState::Repairing);
    assert_eq!(summary.status_label, "repairing");
    assert_eq!(
        summary.failure_summary.as_deref(),
        Some("task authority is missing direction_id")
    );
}

/*
 * compact status surface는 좁은 TUI footer와 command diagnostic에서 쓰인다. 이 assertion들은
 * repair attempt, queue state, proposal, actionable queue label을 짧은 문자열로 줄이는
 * lossy formatting 규칙을 고정한다.
 */
#[test]
fn summary_line_compacts_repair_queue_and_proposal_details() {
    /*
     * footer line은 repair progress, compact queue, proposal fragment를 일부러 한 줄에 섞는다.
     * 각 segment가 서로 다른 next action을 설명하므로 truncation 이후에도 세 신호가 모두 남아야 한다.
     */
    let service = PlanningRuntimePolicyService::new();
    let snapshot = PlanningRuntimeSnapshot::ready_with_details(
        "Planning Context".to_string(),
        "queue idle: no executable planning task".to_string(),
        Some(
            "2 promotable follow-up proposals available: Draft roadmap | Draft checklist"
                .to_string(),
        ),
        None,
    );
    let summary_line = service.build_summary_line(PlanningRuntimeSummaryLineRequest {
        snapshot: &snapshot,
        has_running_turn: false,
        is_repairing: true,
        repair_failure_summary: Some(
            "task authority is missing direction_id and contains extra trailing data",
        ),
        repair_attempt: Some(PlanningRuntimeRepairAttempt {
            attempts_used: 1,
            max_attempts: 2,
        }),
        has_notice: false,
        max_detail_len: 24,
        always_show: true,
    });
    let summary_line = summary_line.expect("summary line should be projected");
    assert!(summary_line.contains("planning: repairing"));
    assert!(summary_line.contains("repair: 1/2"));
    assert!(summary_line.contains("failure: task authority"));
    assert!(summary_line.contains("queue: queue idle:"));
    assert!(summary_line.contains("proposals: 2 promotable"));
}

#[test]
fn status_projection_uses_queue_head_label_when_actionable_work_exists() {
    /*
     * expanded status projection은 실제 queue head가 있을 때 prefix를 바꾼다. consumer는
     * queue_summary 문자열을 파싱해 work availability를 추론하지 않고, "planning queue head"를
     * actionable label로 바로 렌더링할 수 있다.
     */
    let service = PlanningRuntimePolicyService::new();
    let snapshot = PlanningRuntimeSnapshot::ready(
        "Planning Context".to_string(),
        "next task: rank 1 / task-1".to_string(),
        Some(queue_head()),
    );
    let projection = service.build_status_projection(PlanningRuntimeStatusProjectionRequest {
        snapshot: &snapshot,
        has_running_turn: false,
        is_repairing: false,
        repair_failure_summary: None,
        repair_attempt: None,
        max_detail_len: 48,
    });

    assert_eq!(
        projection.queue_head_line.as_deref(),
        Some("planning queue head: next task: rank 1 / task-1")
    );
}

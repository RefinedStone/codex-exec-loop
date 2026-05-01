use super::{
    PlanningAutoFollowBlockReason, PlanningAutoFollowPolicyDecision, PlanningAutoFollowPromptMode,
    PlanningRuntimePolicyService, PlanningRuntimeRepairAttempt,
    PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
    PlanningRuntimeSummaryRequest,
};
use crate::application::service::planning::runtime::prompt::PlanningRuntimeSnapshot;
use crate::domain::planning::{PlanningWorkspaceState, PriorityQueueTask, TaskStatus};

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

#[test]
fn builtin_next_task_blocks_when_planning_is_uninitialized() {
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
fn builtin_next_task_blocks_when_queue_head_and_proposals_are_both_missing() {
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
    let service = PlanningRuntimePolicyService::new();
    let snapshot = PlanningRuntimeSnapshot::ready(
        "Planning Context".to_string(),
        "next task: rank 1 / task-1".to_string(),
        Some(queue_head()),
    )
    .with_auto_followup_pause_reason(
        "planner refresh kept the previously handed-off task as the queue head",
    );

    assert_eq!(
        service.decide_auto_follow(&snapshot),
        PlanningAutoFollowPolicyDecision::Blocked(PlanningAutoFollowBlockReason::RepeatedQueueHead)
    );
}

#[test]
fn builtin_next_task_never_builds_main_refresh_prompt_when_queue_is_idle() {
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

#[test]
fn summary_view_marks_running_ready_planning_as_executing() {
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

#[test]
fn summary_line_compacts_repair_queue_and_proposal_details() {
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

use super::{
    AutoFollowState, AutoFollowupDecision, AutoFollowupSkipReason, ConversationInputState,
    ConversationMessage, ConversationMessageKind, ConversationViewModel, StopKeywordRule,
    TurnActivityState, format_conversation_lines,
};
use crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot;
use crate::application::service::planning_reconciliation_service::PlanningRepairRequest;
use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationSnapshot,
};
use crate::domain::followup_template::{
    FollowupTemplateCatalog, FollowupTemplateCatalogLoadResult, FollowupTemplateDefinition,
    FollowupTemplateSource,
};
use crate::domain::planning::{PlanningWorkspaceState, PriorityQueueTask, TaskStatus};

fn sample_template_catalog() -> FollowupTemplateCatalog {
    FollowupTemplateCatalog {
        items: vec![
            FollowupTemplateDefinition {
                id: "builtin-next-task".to_string(),
                label: "builtin next-task".to_string(),
                body: "대리인입니다.\n자동 후속 {auto_turn}/{max_auto_turns} 입니다.\n\n직전 답변:\n{last_message}\n{stop_keyword}".to_string(),
                source: FollowupTemplateSource::Builtin,
            },
            FollowupTemplateDefinition {
                id: "builtin-plan-queue".to_string(),
                label: "builtin plan-queue".to_string(),
                body: "plan_priority_queue.md\n{last_message}\n{stop_keyword}".to_string(),
                source: FollowupTemplateSource::Builtin,
            },
            FollowupTemplateDefinition {
                id: "workspace-custom-review".to_string(),
                label: "workspace custom-review".to_string(),
                body: "workspace custom body\n{last_message}".to_string(),
                source: FollowupTemplateSource::WorkspaceFile {
                    path: "/tmp/workspace/.codex-exec-loop/followups/custom-review.md"
                        .to_string(),
                },
            },
        ],
    }
}

fn ready_conversation() -> ConversationViewModel {
    ConversationViewModel {
        thread_id: "thread-1".to_string(),
        title: "Existing session".to_string(),
        cwd: "/tmp/workspace".to_string(),
        messages: Vec::new(),
        cached_conversation_lines: format_conversation_lines(&[]),
        live_agent_message: None,
        buffered_tool_messages: Vec::new(),
        base_warnings: Vec::new(),
        template_warnings: Vec::new(),
        warnings: Vec::new(),
        runtime_notices: Vec::new(),
        input_buffer: String::new(),
        startup_submit_armed: false,
        active_turn_id: None,
        planning_repair_state: None,
        input_state: ConversationInputState::ReadyToContinue,
        auto_follow_state: AutoFollowState::new(sample_template_catalog()),
        planning_runtime_snapshot: PlanningRuntimeSnapshot::uninitialized(),
        turn_activity: TurnActivityState::default(),
        approval_review: None,
        last_auto_followup_activity: None,
        status_text: "thread loaded".to_string(),
    }
}

fn turn_prompt_assembly_service() -> TurnPromptAssemblyService {
    TurnPromptAssemblyService::new()
}

fn sample_queue_head() -> PriorityQueueTask {
    PriorityQueueTask {
        rank: 1,
        task_id: "task-1".to_string(),
        direction_id: "general-workstream".to_string(),
        direction_title: "General workstream".to_string(),
        task_title: "Implement shell planning status".to_string(),
        status: TaskStatus::Ready,
        combined_priority: 10,
        updated_at: "2026-04-10T00:00:00Z".to_string(),
        rank_reasons: vec!["status=ready".to_string()],
    }
}

fn sample_planning_runtime_snapshot(
    prompt_fragment: &str,
    queue_summary: &str,
) -> PlanningRuntimeSnapshot {
    PlanningRuntimeSnapshot::ready(
        prompt_fragment.to_string(),
        queue_summary.to_string(),
        Some(sample_queue_head()),
    )
}

#[test]
fn auto_followup_prompt_renders_builtin_template() {
    let mut conversation = ready_conversation();
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    let AutoFollowupDecision::QueuePrompt(prompt) =
        conversation.decide_auto_followup(&turn_prompt_assembly_service())
    else {
        panic!("auto follow-up prompt should render");
    };

    assert!(prompt.contains("대리인입니다."));
    assert!(prompt.contains("자동 후속 1/3 입니다."));
    assert!(prompt.contains("latest answer"));
    assert!(prompt.contains("AUTO_STOP"));
}

#[test]
fn warning_summary_prefers_runtime_warning_detail_and_truncates() {
    let mut conversation = ready_conversation();
    conversation.base_warnings = vec![
        "first warning".to_string(),
        "shared runtime busy with an active turn stream; request used an isolated app-server connection".to_string(),
    ];
    conversation.warnings = conversation.base_warnings.clone();

    let summary = conversation.warning_summary(36);

    assert_eq!(
        summary,
        "runtime warnings (2): shared runtime busy with an activ..."
    );
}

#[test]
fn runtime_notice_summary_is_separate_from_warning_summary() {
    let mut conversation = ready_conversation();
    conversation.template_warnings = vec!["workspace template warning".to_string()];
    conversation.warnings = conversation.template_warnings.clone();
    conversation.runtime_notices = vec![
        "shared runtime reset after recent sessions request failure; retrying with a fresh app-server connection (boom)"
            .to_string(),
    ];

    assert_eq!(
        conversation.warning_summary(40),
        "template warning: workspace template warning"
    );
    let runtime_summary = conversation
        .runtime_notice_summary(40)
        .expect("runtime summary should exist");
    assert!(runtime_summary.starts_with("runtime: shared runtime reset"));
}

#[test]
fn from_snapshot_keeps_runtime_notices_out_of_status_text() {
    let conversation = ConversationViewModel::from_snapshot(
        ConversationSnapshot {
            thread_id: "thread-1".to_string(),
            title: "Existing session".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: vec![
                "shared runtime reconnected after the previous app-server process exited"
                    .to_string(),
            ],
        },
        FollowupTemplateCatalogLoadResult {
            catalog: sample_template_catalog(),
            warnings: Vec::new(),
        },
    );

    assert_eq!(conversation.status_text, "thread loaded / templates: 3");
    assert!(
        conversation
            .runtime_notice_summary(36)
            .expect("runtime summary should exist")
            .starts_with("runtime: shared runtime reconnected")
    );
}

#[test]
fn approval_review_status_preserves_warning_suffix() {
    let mut conversation = ready_conversation();
    conversation.template_warnings = vec!["workspace template warning".to_string()];
    conversation.warnings = conversation.template_warnings.clone();

    conversation.update_approval_review(ConversationApprovalReview {
        target_item_id: "command-1".to_string(),
        status: ConversationApprovalReviewStatus::InProgress,
        risk_level: Some("high".to_string()),
        rationale: None,
    });

    assert_eq!(
        conversation.status_text,
        "approval review in progress / target: command-1 / risk: high / template warning"
    );
}

#[test]
fn warning_summary_reports_runtime_and_template_counts_when_both_exist() {
    let mut conversation = ready_conversation();
    conversation.base_warnings = vec![
        "shared runtime reset after turn stream failure; the next request will reconnect"
            .to_string(),
    ];
    conversation.template_warnings = vec![
        "workspace template missing".to_string(),
        "template catalog reloaded with fallback".to_string(),
    ];
    conversation.warnings = conversation
        .base_warnings
        .iter()
        .chain(conversation.template_warnings.iter())
        .cloned()
        .collect();

    assert_eq!(
        conversation.warning_summary(48),
        "warnings: runtime 1, template 2 / shared runtime reset after turn stream failur..."
    );
}

#[test]
fn snapshot_status_keeps_base_status_with_compact_warning_label() {
    let conversation = ConversationViewModel::from_snapshot(
        ConversationSnapshot {
            thread_id: "thread-1".to_string(),
            title: "Existing session".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: vec!["shared runtime reset after startup checks failure".to_string()],
        },
        FollowupTemplateCatalogLoadResult {
            catalog: sample_template_catalog(),
            warnings: vec!["workspace template missing".to_string()],
        },
    );

    assert_eq!(
        conversation.status_text,
        "thread loaded / templates: 3 / template warning"
    );
    assert!(
        conversation
            .runtime_notice_summary(48)
            .expect("runtime summary should exist")
            .starts_with("runtime: shared runtime reset after startup checks")
    );
}

#[test]
fn auto_followup_prompt_skips_when_manual_input_is_buffered() {
    let mut conversation = ready_conversation();
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.input_buffer = "manual prompt".to_string();

    assert_eq!(
        conversation.decide_auto_followup(&turn_prompt_assembly_service()),
        AutoFollowupDecision::Skip(AutoFollowupSkipReason::ManualInputBuffered)
    );
}

#[test]
fn auto_followup_template_cycles_across_builtin_and_workspace_items() {
    let mut state = AutoFollowState::new(sample_template_catalog());

    assert_eq!(state.template_label(), "builtin next-task");
    state.cycle_template_kind();
    assert_eq!(state.template_label(), "builtin plan-queue");
    state.cycle_template_kind();
    assert_eq!(state.template_label(), "workspace custom-review");
    state.cycle_template_kind();
    assert_eq!(state.template_label(), "builtin next-task");
}

#[test]
fn auto_followup_prompt_uses_selected_template_item() {
    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 1;
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    let AutoFollowupDecision::QueuePrompt(prompt) =
        conversation.decide_auto_followup(&turn_prompt_assembly_service())
    else {
        panic!("plan queue prompt should render");
    };

    assert!(prompt.contains("plan_priority_queue.md"));
    assert!(prompt.contains("latest answer"));
}

#[test]
fn auto_followup_activity_exposes_workspace_template_source() {
    let mut state = AutoFollowState::new(sample_template_catalog());
    state.template_state.selected_index = 2;

    assert_eq!(state.template_label(), "workspace custom-review");
    assert!(
        state
            .template_source_label()
            .contains(".codex-exec-loop/followups/custom-review.md")
    );
}

#[test]
fn stop_keyword_rule_normalizes_valid_identifier_like_values() {
    assert_eq!(
        StopKeywordRule::normalize_candidate(" AUTO_STOP_2 "),
        Some("AUTO_STOP_2".to_string())
    );
    assert_eq!(StopKeywordRule::normalize_candidate("two words"), None);
    assert_eq!(StopKeywordRule::normalize_candidate(""), None);
    assert_eq!(StopKeywordRule::normalize_candidate("stop!"), None);
}

#[test]
fn max_auto_turn_candidate_requires_value_between_one_and_fifty() {
    assert_eq!(
        AutoFollowState::normalize_max_auto_turns_candidate(" 7 "),
        Some(7)
    );
    assert_eq!(
        AutoFollowState::normalize_max_auto_turns_candidate("50"),
        Some(50)
    );
    assert_eq!(
        AutoFollowState::normalize_max_auto_turns_candidate("0"),
        None
    );
    assert_eq!(
        AutoFollowState::normalize_max_auto_turns_candidate("51"),
        None
    );
    assert_eq!(
        AutoFollowState::normalize_max_auto_turns_candidate("three"),
        None
    );
}

#[test]
fn reloading_template_catalog_preserves_selected_template_when_id_still_exists() {
    let mut state = AutoFollowState::new(sample_template_catalog());
    state.template_state.selected_index = 1;

    state.reload_template_catalog(FollowupTemplateCatalog {
        items: vec![
            FollowupTemplateDefinition {
                id: "builtin-next-task".to_string(),
                label: "builtin next-task".to_string(),
                body: "next".to_string(),
                source: FollowupTemplateSource::Builtin,
            },
            FollowupTemplateDefinition {
                id: "builtin-plan-queue".to_string(),
                label: "builtin plan-queue".to_string(),
                body: "reloaded".to_string(),
                source: FollowupTemplateSource::Builtin,
            },
        ],
    });

    assert_eq!(state.template_label(), "builtin plan-queue");
}

#[test]
fn reloading_template_catalog_falls_back_to_first_template_when_selection_disappears() {
    let mut state = AutoFollowState::new(sample_template_catalog());
    state.template_state.selected_index = 2;

    state.reload_template_catalog(FollowupTemplateCatalog {
        items: vec![FollowupTemplateDefinition {
            id: "builtin-next-task".to_string(),
            label: "builtin next-task".to_string(),
            body: "next".to_string(),
            source: FollowupTemplateSource::Builtin,
        }],
    });

    assert_eq!(state.template_label(), "builtin next-task");
    assert_eq!(state.selected_template_index(), 0);
}

#[test]
fn auto_followup_stops_when_stop_keyword_is_present() {
    let mut conversation = ready_conversation();
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "Work is complete.\nAUTO_STOP",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    assert_eq!(
        conversation.decide_auto_followup(&turn_prompt_assembly_service()),
        AutoFollowupDecision::Skip(AutoFollowupSkipReason::StopKeywordMatched)
    );
}

#[test]
fn auto_followup_stops_when_stop_keyword_case_varies() {
    let mut conversation = ready_conversation();
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "Work is complete.\nauto_stop!",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    assert_eq!(
        conversation.decide_auto_followup(&turn_prompt_assembly_service()),
        AutoFollowupDecision::Skip(AutoFollowupSkipReason::StopKeywordMatched)
    );
}

#[test]
fn auto_followup_stops_when_custom_stop_keyword_is_present() {
    let mut conversation = ready_conversation();
    conversation
        .auto_follow_state
        .set_stop_keyword_value("DONE".to_string());
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "Work is complete.\ndone!",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    assert_eq!(
        conversation.decide_auto_followup(&turn_prompt_assembly_service()),
        AutoFollowupDecision::Skip(AutoFollowupSkipReason::StopKeywordMatched)
    );
}

#[test]
fn auto_followup_stops_without_file_changes_when_rule_is_enabled() {
    let mut conversation = ready_conversation();
    conversation
        .auto_follow_state
        .stop_rules
        .stop_on_no_file_changes = true;
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    assert_eq!(
        conversation.decide_auto_followup(&turn_prompt_assembly_service()),
        AutoFollowupDecision::Skip(AutoFollowupSkipReason::NoFileChanges)
    );
}

#[test]
fn auto_followup_continues_when_file_changes_exist_and_stop_rule_is_enabled() {
    let mut conversation = ready_conversation();
    conversation
        .auto_follow_state
        .stop_rules
        .stop_on_no_file_changes = true;
    conversation
        .turn_activity
        .last_completed_turn_file_change_count = 2;
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    let AutoFollowupDecision::QueuePrompt(prompt) =
        conversation.decide_auto_followup(&turn_prompt_assembly_service())
    else {
        panic!("auto follow-up should continue when file changes exist");
    };

    assert!(prompt.contains("latest answer"));
}

#[test]
fn auto_followup_prompt_appends_planning_prompt_fragment_when_ready() {
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context\nQueue Summary",
        "next task: task-1",
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    let AutoFollowupDecision::QueuePrompt(prompt) =
        conversation.decide_auto_followup(&turn_prompt_assembly_service())
    else {
        panic!("planning-aware auto follow-up prompt should render");
    };

    assert!(prompt.contains("latest answer"));
    assert!(prompt.contains("Planning Context"));
    assert!(prompt.contains("Queue Summary"));
}

#[test]
fn auto_followup_skips_when_planning_runtime_snapshot_is_invalid() {
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(PlanningRuntimeSnapshot::invalid(
        "planning validation failed: task-ledger.json is invalid",
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    assert_eq!(
        conversation.decide_auto_followup(&turn_prompt_assembly_service()),
        AutoFollowupDecision::Skip(AutoFollowupSkipReason::PlanningBlocked)
    );
}

#[test]
fn planning_workspace_state_marks_ready_context_as_stale_during_running_turn() {
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: task-1",
    ));
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());

    assert_eq!(
        conversation.planning_workspace_state(),
        PlanningWorkspaceState::Executing
    );
    assert_eq!(conversation.planning_status_label(), "stale");
    assert_eq!(
        conversation.planning_queue_summary(),
        Some("next task: task-1")
    );
}

#[test]
fn planning_workspace_state_prefers_repairing_failure_over_ready_queue_context() {
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: task-1",
    ));
    conversation.planning_repair_state = Some(super::PlanningRepairState {
        root_turn_id: "turn-root".to_string(),
        attempts_used: 1,
        max_attempts: 2,
        latest_request: PlanningRepairRequest {
            failure_summary: "task-ledger.json is missing direction_id".to_string(),
            validation_errors: vec!["task-ledger.json is missing direction_id".to_string()],
            directions_toml: "version = 1".to_string(),
            task_ledger_schema_json: "{\"type\":\"object\"}".to_string(),
            accepted_task_ledger_json: "{\"version\":1,\"tasks\":[]}".to_string(),
            rejected_task_ledger_json: None,
            rejected_archive_path: None,
        },
    });

    assert_eq!(
        conversation.planning_workspace_state(),
        PlanningWorkspaceState::Repairing
    );
    assert_eq!(conversation.planning_status_label(), "repairing");
    assert_eq!(
        conversation.planning_failure_summary(),
        Some("task-ledger.json is missing direction_id")
    );
}

#[test]
fn planning_notice_summary_filters_non_planning_runtime_notices() {
    let mut conversation = ready_conversation();
    conversation.runtime_notices = vec![
        "shared runtime reconnected after app-server exit".to_string(),
        "planning reconciliation restored protected directions.toml".to_string(),
        "planning repair queued retry 1/2 for task-ledger.json".to_string(),
    ];

    assert_eq!(
        conversation.planning_notice_summary(64),
        Some(
            "planning notices (2): planning repair queued retry 1/2 for task-ledger.json"
                .to_string()
        )
    );
}

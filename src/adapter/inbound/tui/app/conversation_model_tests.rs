use std::sync::Arc;

use anyhow::{Result, anyhow};

use super::{
    AutoFollowState, AutoFollowupDecision, AutoFollowupSkipReason, ConversationInputState,
    ConversationMessage, ConversationMessageKind, ConversationViewModel, StopKeywordRule,
    TurnActivityState, format_conversation_lines,
};
use crate::adapter::inbound::tui::app::test_helpers::{
    sample_planning_runtime_snapshot, sample_proposal_only_planning_runtime_snapshot,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
    PlanningStagedFileRecord, PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning_prompt_service::PlanningPromptService;
use crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot;
use crate::application::service::planning_reconciliation_service::PlanningReconciliationService;
use crate::application::service::planning_runtime_facade_service::PlanningRuntimeFacadeService;
use crate::application::service::planning_runtime_policy_service::PlanningRuntimePolicyService;
use crate::application::service::planning_validation_service::PlanningValidationService;
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationSnapshot,
};
use crate::domain::followup_template::{
    FollowupTemplateCatalog, FollowupTemplateCatalogLoadResult, FollowupTemplateDefinition,
    FollowupTemplateSource,
};

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
        draft_workspace_directory: "/tmp/workspace".to_string(),
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
        active_turn_workspace_directory: None,
        active_turn_started_at: None,
        planning_repair_state: None,
        input_state: ConversationInputState::ReadyToContinue,
        auto_follow_state: AutoFollowState::new(sample_template_catalog()),
        planning_runtime_snapshot: PlanningRuntimeSnapshot::uninitialized(),
        turn_activity: TurnActivityState::default(),
        approval_review: None,
        last_auto_followup_activity: None,
        last_planning_task_handoff: None,
        status_text: "thread loaded".to_string(),
    }
}

struct FakePlanningWorkspacePort;

impl PlanningWorkspacePort for FakePlanningWorkspacePort {
    fn stage_planning_draft_files(
        &self,
        _workspace_dir: &str,
        draft_name: &str,
        _files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord> {
        Ok(PlanningDraftStageRecord {
            draft_name: draft_name.to_string(),
            draft_directory: "/tmp/drafts".to_string(),
            staged_files: vec![PlanningStagedFileRecord {
                active_path: "task-ledger.json".to_string(),
                staged_path: ".codex-exec-loop/planning/drafts/task-ledger.json".to_string(),
            }],
        })
    }

    fn load_planning_draft_files(
        &self,
        _workspace_dir: &str,
        _draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord> {
        Err(anyhow!("unused in test"))
    }

    fn replace_planning_draft_file(
        &self,
        _workspace_dir: &str,
        _draft_name: &str,
        _active_path: &str,
        _body: &str,
    ) -> Result<String> {
        Err(anyhow!("unused in test"))
    }

    fn load_planning_workspace_files(
        &self,
        _workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        Ok(PlanningWorkspaceLoadRecord::default())
    }

    fn load_optional_planning_file(
        &self,
        _workspace_dir: &str,
        _relative_path: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    fn replace_planning_workspace_file(
        &self,
        _workspace_dir: &str,
        _relative_path: &str,
        _body: Option<&str>,
    ) -> Result<()> {
        Err(anyhow!("unused in test"))
    }

    fn archive_rejected_planning_file(
        &self,
        _workspace_dir: &str,
        _archive_name: &str,
        _active_path: &str,
        _body: &str,
    ) -> Result<String> {
        Err(anyhow!("unused in test"))
    }
}

fn planning_runtime_facade_service() -> PlanningRuntimeFacadeService {
    let port = Arc::new(FakePlanningWorkspacePort);
    PlanningRuntimeFacadeService::new(
        PlanningPromptService::new(
            port.clone(),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        ),
        PlanningReconciliationService::new(
            port,
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        ),
        PlanningRuntimePolicyService::new(),
        TurnPromptAssemblyService::new(),
    )
}

#[test]
fn auto_followup_prompt_renders_builtin_template() {
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: task-1",
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    let AutoFollowupDecision::QueuePrompt(prompt) =
        conversation.decide_auto_followup(&planning_runtime_facade_service())
    else {
        panic!("auto follow-up prompt should render");
    };

    assert!(
        prompt
            .prompt
            .contains("Continue the next highest-priority task.")
    );
    assert!(prompt.prompt.contains("Implement shell planning status"));
    assert!(prompt.prompt.contains("General workstream"));
    assert_eq!(
        prompt.transcript_text,
        "다음 queued task 1개를 이어서 진행합니다."
    );
    assert_eq!(
        prompt
            .handoff_task
            .as_ref()
            .map(|task| task.task_id.as_str()),
        Some("task-1")
    );
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
        "/tmp/draft-workspace".to_string(),
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
        "/tmp/draft-workspace".to_string(),
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
fn auto_followup_prompt_ignores_buffered_manual_input() {
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: task-1",
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.input_buffer = "manual prompt".to_string();

    let AutoFollowupDecision::QueuePrompt(prompt) =
        conversation.decide_auto_followup(&planning_runtime_facade_service())
    else {
        panic!("buffered manual input should not block auto follow-up");
    };

    assert!(
        prompt
            .prompt
            .contains("Continue the next highest-priority task.")
    );
}

#[test]
fn record_submitted_prompt_refreshes_cached_lines_and_tracks_turn_workspace() {
    let mut conversation = ready_conversation();

    conversation.record_submitted_prompt(
        ConversationMessage::new(ConversationMessageKind::User, "ship it", None, None),
        "/tmp/turn-workspace".to_string(),
        true,
    );

    assert_eq!(
        conversation.cached_conversation_lines,
        format_conversation_lines(&conversation.messages)
    );
    assert_eq!(
        conversation.active_turn_workspace_directory.as_deref(),
        Some("/tmp/turn-workspace")
    );
}

#[test]
fn planning_workspace_directory_prefers_draft_workspace_for_new_threads() {
    let mut conversation = ready_conversation();
    conversation.thread_id.clear();
    conversation.cwd = "/tmp/thread-workspace".to_string();
    conversation.draft_workspace_directory = "/tmp/draft-workspace".to_string();

    assert_eq!(
        conversation.planning_workspace_directory(),
        "/tmp/draft-workspace"
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
        conversation.decide_auto_followup(&planning_runtime_facade_service())
    else {
        panic!("plan queue prompt should render");
    };

    assert!(prompt.prompt.contains("plan_priority_queue.md"));
    assert!(prompt.prompt.contains("latest answer"));
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
        conversation.decide_auto_followup(&planning_runtime_facade_service()),
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
        conversation.decide_auto_followup(&planning_runtime_facade_service()),
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
        conversation.decide_auto_followup(&planning_runtime_facade_service()),
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
        conversation.decide_auto_followup(&planning_runtime_facade_service()),
        AutoFollowupDecision::Skip(AutoFollowupSkipReason::NoFileChanges)
    );
}

#[test]
fn auto_followup_continues_when_file_changes_exist_and_stop_rule_is_enabled() {
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: task-1",
    ));
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
        conversation.decide_auto_followup(&planning_runtime_facade_service())
    else {
        panic!("auto follow-up should continue when file changes exist");
    };

    assert!(
        prompt
            .prompt
            .contains("Continue the next highest-priority task.")
    );
}

#[test]
fn auto_followup_prompt_appends_planning_prompt_fragment_when_ready() {
    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 1;
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
        conversation.decide_auto_followup(&planning_runtime_facade_service())
    else {
        panic!("planning-aware auto follow-up prompt should render");
    };

    assert!(prompt.prompt.contains("plan_priority_queue.md"));
    assert!(prompt.prompt.contains("latest answer"));
    assert!(prompt.prompt.contains("Planning Context"));
    assert!(prompt.prompt.contains("Queue Summary"));
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
        conversation.decide_auto_followup(&planning_runtime_facade_service()),
        AutoFollowupDecision::Skip(AutoFollowupSkipReason::PlanningBlocked)
    );
}

#[test]
fn builtin_next_task_blocks_when_planning_queue_has_no_next_task() {
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(PlanningRuntimeSnapshot::ready(
        "Planning Context".to_string(),
        "next_task: none".to_string(),
        None,
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    assert_eq!(
        conversation.decide_auto_followup(&planning_runtime_facade_service()),
        AutoFollowupDecision::Skip(AutoFollowupSkipReason::PlanningQueueHeadRequired)
    );
}

#[test]
fn builtin_next_task_blocks_when_only_proposals_remain() {
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_proposal_only_planning_runtime_snapshot(
        "Planning Context\nRuntime Follow-up Proposal Rules",
        "queue idle: no executable planning task",
        "2 promotable follow-up proposals available: Plan A | +1 more",
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    assert_eq!(
        conversation.decide_auto_followup(&planning_runtime_facade_service()),
        AutoFollowupDecision::Skip(AutoFollowupSkipReason::PlanningQueueHeadRequired)
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

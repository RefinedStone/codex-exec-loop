use super::{
    AutoFollowDecision, AutoFollowSkipReason, AutoFollowState, ConversationMessage,
    ConversationMessageKind, ConversationViewModel, StopKeywordRule,
};
use crate::adapter::inbound::tui::app::INFINITE_AUTO_FOLLOW_MAX_TURNS;
use crate::adapter::inbound::tui::app::test_helpers::{
    sample_planning_runtime_projection, sample_proposal_only_planning_runtime_projection,
    test_planning_services,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
    PlanningStagedFileRecord, PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::{PlanningRuntimeProjection, PlanningRuntimeUseCases};
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationSnapshot,
};
use anyhow::{Result, anyhow};
use std::sync::Arc;

// ConversationViewModel tests start from a fully loaded thread rather than a
// startup shell. That keeps each assertion focused on reducer-visible
// presentation state: warnings, notices, approval status, and auto-follow
// decisions.
fn ready_conversation() -> ConversationViewModel {
    ConversationViewModel::from_snapshot(
        ConversationSnapshot {
            thread_id: "thread-1".to_string(),
            title: "Existing session".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
        },
        "/tmp/workspace".to_string(),
    )
}

// Auto-follow prompt rendering consults the planning runtime, but these tests
// only need deterministic file-change evidence. The fake port succeeds on the
// staging/read paths used by prompt composition and rejects unrelated mutation
// paths so accidental expansion of the fixture is visible.
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
                active_path: ".codex-exec-loop/planning/result-output.md".to_string(),
                staged_path: ".codex-exec-loop/planning/drafts/result-output.md".to_string(),
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
    fn load_planning_workspace_candidate_files(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        self.load_planning_workspace_files(workspace_dir)
    }
    fn commit_planning_workspace_files(
        &self,
        _workspace_dir: &str,
        _record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()> {
        Err(anyhow!("unused in test"))
    }
    fn load_optional_planning_file(
        &self,
        _workspace_dir: &str,
        _relative_path: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }
    fn load_optional_planning_candidate_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        self.load_optional_planning_file(workspace_dir, relative_path)
    }
    fn replace_planning_workspace_file(
        &self,
        _workspace_dir: &str,
        _relative_path: &str,
        _body: Option<&str>,
    ) -> Result<()> {
        Err(anyhow!("unused in test"))
    }
    fn remove_planning_workspace_entry(
        &self,
        _workspace_dir: &str,
        _relative_path: &str,
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

fn planning_runtime() -> PlanningRuntimeUseCases {
    test_planning_services(Arc::new(FakePlanningWorkspacePort)).runtime
}

// This group protects the handoff boundary from "final answer received" to
// "queue the next planning task". The prompt must include the planning
// handoff context, while the transcript text stays compact because it is shown
// inline in the TUI conversation stream.
#[test]
fn queue_handoff_prompt_renders_for_auto_follow() {
    let mut conversation = ready_conversation();
    conversation.replace_cached_planning_runtime_projection(sample_planning_runtime_projection(
        "Planning Context",
        "queue head: task-1",
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    let AutoFollowDecision::QueuePrompt(prompt) =
        conversation.decide_auto_follow(&planning_runtime())
    else {
        panic!("auto-follow prompt should render");
    };

    assert!(
        prompt
            .prompt
            .contains("Continue the next highest-priority task.")
    );
    assert!(prompt.prompt.contains("Implement shell planning status"));
    assert_eq!(
        prompt.transcript_text,
        "다음 queued-task 1개를 이어서 진행합니다."
    );
}

// Warning summaries are shell chrome, not transcript content. These tests pin
// down the priority order: user-facing warnings remain separate from runtime
// reconnection notices, and status text keeps its warning suffix when approval
// review state changes.
#[test]
fn warning_summary_prefers_latest_warning_and_truncates() {
    let mut conversation = ready_conversation();
    conversation.base_warnings = vec![
        "first warning".to_string(),
        "shared runtime busy with an active turn stream; request used an isolated app-server connection".to_string(),
    ];
    conversation.warnings = conversation.base_warnings.clone();
    let summary = conversation.warning_summary(36);

    assert_eq!(
        summary,
        "warnings (2): shared runtime busy with an activ..."
    );
}

#[test]
fn runtime_notice_summary_is_separate_from_warning_summary() {
    let mut conversation = ready_conversation();
    conversation.base_warnings = vec!["workspace planning warning".to_string()];
    conversation.warnings = conversation.base_warnings.clone();
    conversation.runtime_notices = vec![
        "shared runtime reset after recent sessions request failure; retrying with a fresh app-server connection (boom)"
            .to_string(),
    ];

    assert_eq!(
        conversation.warning_summary(40),
        "warning: workspace planning warning"
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
        "/tmp/draft-workspace".to_string(),
    );

    assert_eq!(conversation.status_text, "thread loaded");
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
    conversation.base_warnings = vec!["planning warning".to_string()];
    conversation.warnings = conversation.base_warnings.clone();

    conversation.update_approval_review(ConversationApprovalReview {
        target_item_id: "command-1".to_string(),
        status: ConversationApprovalReviewStatus::InProgress,
        risk_level: Some("high".to_string()),
        rationale: None,
    });

    assert_eq!(
        conversation.status_text,
        "approval review in progress / target: command-1 / risk: high / handling: manual handoff / warning"
    );
}

// Auto-follow settings are typed state once parsed, but the TUI receives raw
// input strings from inline controls. These tests keep normalization narrow so
// arbitrary prose cannot become a stop keyword or an unbounded turn count.
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
fn max_auto_turn_candidate_accepts_positive_numbers_and_infinite() {
    assert_eq!(
        AutoFollowState::normalize_max_auto_turns_candidate(" 7 "),
        Some(7)
    );
    assert_eq!(
        AutoFollowState::normalize_max_auto_turns_candidate("51"),
        Some(51)
    );
    assert_eq!(
        AutoFollowState::normalize_max_auto_turns_candidate("infinite"),
        Some(INFINITE_AUTO_FOLLOW_MAX_TURNS)
    );
    assert_eq!(
        AutoFollowState::normalize_max_auto_turns_candidate("0"),
        None
    );
    assert_eq!(
        AutoFollowState::normalize_max_auto_turns_candidate("three"),
        None
    );
}

// Stop rules are evaluated before creating another app-server turn. The cases
// below cover explicit stop keywords, no-change protection, queue-idle
// snapshots, and invalid planning authority snapshots so the runtime does not
// keep looping when the planning state says there is no executable queue head.
#[test]
fn auto_follow_stops_when_stop_keyword_is_present() {
    let mut conversation = ready_conversation();
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "Work is complete.\nAUTO_STOP",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    assert_eq!(
        conversation.decide_auto_follow(&planning_runtime()),
        AutoFollowDecision::Skip(AutoFollowSkipReason::StopKeywordMatched)
    );
}

#[test]
fn auto_follow_stops_when_custom_stop_keyword_is_present() {
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
        conversation.decide_auto_follow(&planning_runtime()),
        AutoFollowDecision::Skip(AutoFollowSkipReason::StopKeywordMatched)
    );
}

#[test]
fn auto_follow_stops_without_file_changes_when_rule_is_enabled() {
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
        conversation.decide_auto_follow(&planning_runtime()),
        AutoFollowDecision::Skip(AutoFollowSkipReason::NoFileChanges)
    );
}

#[test]
fn auto_follow_continues_when_file_changes_exist_and_stop_rule_is_enabled() {
    let mut conversation = ready_conversation();
    conversation.replace_cached_planning_runtime_projection(sample_planning_runtime_projection(
        "Planning Context",
        "queue head: task-1",
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
    let AutoFollowDecision::QueuePrompt(prompt) =
        conversation.decide_auto_follow(&planning_runtime())
    else {
        panic!("auto-follow should continue when file changes exist");
    };

    assert!(
        prompt
            .prompt
            .contains("Continue the next highest-priority task.")
    );
}

#[test]
fn auto_follow_skips_main_refresh_prompt_when_queue_is_idle() {
    let mut conversation = ready_conversation();
    conversation.replace_cached_planning_runtime_projection(
        sample_proposal_only_planning_runtime_projection(
            "Planning Context\nRuntime Follow-up Proposal Rules",
            "queue idle: no executable planning task",
            "2 promotable follow-up proposals available: Plan A | +1 more",
        ),
    );
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    assert_eq!(
        conversation.decide_auto_follow(&planning_runtime()),
        AutoFollowDecision::Skip(AutoFollowSkipReason::PlanningQueueHeadRequired)
    );
}

#[test]
fn auto_follow_skips_when_planning_runtime_projection_is_invalid() {
    let mut conversation = ready_conversation();
    conversation.replace_cached_planning_runtime_projection(PlanningRuntimeProjection::invalid(
        "planning validation failed: task authority is invalid",
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    assert_eq!(
        conversation.decide_auto_follow(&planning_runtime()),
        AutoFollowDecision::Skip(AutoFollowSkipReason::PlanningBlocked)
    );
}

// Planning notices are filtered out of generic runtime notices before they
// reach the shell tail. The most recent planning-specific entry should be what
// operators see when repair/reconciliation state changes quickly.
#[test]
fn planning_notice_summary_filters_non_planning_runtime_notices() {
    let mut conversation = ready_conversation();
    conversation.runtime_notices = vec![
        "shared runtime reconnected after app-server exit".to_string(),
        "planning reconciliation restored protected DB direction authority".to_string(),
        "planning repair queued retry 1/2 for task authority".to_string(),
    ];

    assert_eq!(
        conversation.planning_notice_summary(64),
        Some(
            "planning notices (2): planning repair queued retry 1/2 for task authority".to_string()
        )
    );
}

use super::{
    AutoFollowSnapshotPresentation, ConversationMessageKind, ConversationViewModel,
    normalize_max_auto_turns_candidate,
};
use crate::adapter::inbound::tui::app::INFINITE_AUTO_FOLLOW_MAX_TURNS;
use crate::core::app::{
    ActiveTurnPhase, ActiveTurnSnapshot, CorePromptOrigin, PostTurnAuthoritySnapshot,
    PostTurnEvaluationCorrelation, PostTurnRouteResolution, TurnSubmissionCorrelation,
};
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationSnapshot,
};

const TEST_AUTO_FOLLOW_MAX_TURNS: usize = 20;

// ConversationViewModel tests start from a fully loaded thread rather than a
// startup shell. That keeps each assertion focused on reducer-visible
// presentation state: warnings, notices, approval status, and auto-follow state.
fn ready_conversation() -> ConversationViewModel {
    let mut conversation = ConversationViewModel::from_snapshot(
        ConversationSnapshot {
            thread_id: "thread-1".to_string(),
            title: "Existing session".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        },
        "/tmp/workspace".to_string(),
    );
    let mut runtime = conversation.runtime_snapshot().clone();
    runtime.auto_follow.max_auto_turns = TEST_AUTO_FOLLOW_MAX_TURNS;
    conversation.apply_runtime_snapshot(runtime);
    conversation
}

fn start_running_turn(conversation: &mut ConversationViewModel, turn_id: &str) {
    let mut runtime = conversation.runtime_snapshot().clone();
    runtime.active_turn = Some(ActiveTurnSnapshot {
        correlation: TurnSubmissionCorrelation::new(1),
        phase: ActiveTurnPhase::Running,
        workspace_directory: conversation.cwd.clone(),
        turn_id: Some(turn_id.to_string()),
        prompt_origin: CorePromptOrigin::Manual,
        started_at: std::time::Instant::now(),
    });
    conversation.apply_runtime_snapshot(runtime);
    conversation.record_turn_started(turn_id.to_string());
}

fn settle_post_turn(conversation: &mut ConversationViewModel, completed_turn_id: &str) {
    let correlation = PostTurnEvaluationCorrelation::new(
        1,
        conversation.thread_id.clone(),
        completed_turn_id,
        conversation.cwd.clone(),
        conversation.planning_workspace_directory(),
    );
    let mut runtime = conversation.runtime_snapshot().clone();
    runtime.active_turn = None;
    runtime.post_turn = PostTurnAuthoritySnapshot::Settled {
        correlation,
        resolution: PostTurnRouteResolution::NoContinuation,
    };
    conversation.apply_runtime_snapshot(runtime);
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
            item_lifecycle: Default::default(),
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
        "approval review in progress / target: command-1 / risk: high / warning"
    );
}

#[test]
fn max_auto_turn_candidate_accepts_positive_infinite_and_disable_tokens() {
    assert_eq!(normalize_max_auto_turns_candidate(" 7 "), Some(7));
    assert_eq!(normalize_max_auto_turns_candidate("51"), Some(51));
    assert_eq!(
        normalize_max_auto_turns_candidate("infinite"),
        Some(INFINITE_AUTO_FOLLOW_MAX_TURNS)
    );
    assert_eq!(normalize_max_auto_turns_candidate("0"), Some(0));
    assert_eq!(normalize_max_auto_turns_candidate(" OFF "), Some(0));
    assert_eq!(normalize_max_auto_turns_candidate("three"), None);
}

#[test]
fn new_and_resumed_conversations_default_auto_follow_to_off() {
    let draft = ConversationViewModel::new_draft("/tmp/workspace".to_string());
    assert!(!draft.auto_follow_state().is_enabled());
    assert!(!draft.auto_follow_state().can_queue_next());
    assert_eq!(draft.auto_follow_state().max_auto_turns_label(), "off");

    let resumed = ConversationViewModel::from_snapshot(
        ConversationSnapshot {
            thread_id: "thread-off".to_string(),
            title: "Existing session".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        },
        "/tmp/workspace".to_string(),
    );
    assert!(!resumed.auto_follow_state().is_enabled());
    assert!(!resumed.auto_follow_state().can_queue_next());
    assert_eq!(resumed.auto_follow_state().progress_label(), "off");
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

#[test]
fn next_live_agent_item_keeps_the_earliest_handoff_out_of_history() {
    let mut conversation = ready_conversation();
    start_running_turn(&mut conversation, "turn-1");
    conversation.push_live_agent_delta(
        "commentary-1".to_string(),
        Some("commentary".to_string()),
        "completed commentary".to_string(),
    );
    conversation.complete_live_agent_message(
        "commentary-1".to_string(),
        Some("commentary".to_string()),
        "completed commentary".to_string(),
    );

    assert!(
        !conversation
            .host_scrollback_messages()
            .iter()
            .any(|message| message.text == "completed commentary")
    );
    assert_eq!(
        conversation
            .viewport_transcript_handoff_messages()
            .map(|messages| messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>()),
        Some(vec!["completed commentary"])
    );

    conversation.push_live_agent_delta(
        "answer-2".to_string(),
        Some("final_answer".to_string()),
        "new live answer".to_string(),
    );

    assert!(
        !conversation
            .host_scrollback_messages()
            .iter()
            .any(|message| message.text == "completed commentary")
    );
    assert_eq!(
        conversation
            .viewport_transcript_handoff_messages()
            .map(|messages| messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>()),
        Some(vec!["completed commentary"])
    );
    assert_eq!(
        conversation
            .live_agent_message
            .as_ref()
            .map(|message| message.text.as_str()),
        Some("new live answer")
    );
}

#[test]
fn completed_settlement_waits_for_history_flush_ack_before_unlocking_navigation() {
    let mut conversation = ready_conversation();
    start_running_turn(&mut conversation, "turn-1");
    conversation.push_live_agent_delta(
        "answer-1".to_string(),
        Some("final_answer".to_string()),
        "durable final answer".to_string(),
    );
    conversation.complete_live_agent_message(
        "answer-1".to_string(),
        Some("final_answer".to_string()),
        "durable final answer".to_string(),
    );
    conversation.finish_turn("turn-1", &[]);
    conversation.begin_post_turn_settlement("turn-1");
    settle_post_turn(&mut conversation, "turn-1");

    assert!(conversation.complete_post_turn_settlement("turn-1"));
    assert!(conversation.has_pending_viewport_transcript_handoff());
    assert!(
        conversation
            .viewport_transcript_handoff_messages()
            .is_none()
    );
    assert!(
        conversation
            .host_scrollback_messages()
            .iter()
            .any(|message| message.text == "durable final answer")
    );
    assert!(!conversation.can_accept_manual_prompt());

    let correlation = conversation
        .viewport_transcript_handoff_correlation()
        .expect("released handoff should have a correlation");
    assert!(conversation.acknowledge_viewport_transcript_handoff_flush(&correlation));
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    assert!(conversation.can_accept_manual_prompt());
}

#[test]
fn agentless_failure_waits_for_transcript_delivery_before_unlocking_navigation() {
    let mut conversation = ready_conversation();
    conversation.record_turn_started("turn-1".to_string());
    conversation.fail_turn(Some("turn-1"), "agentless runtime failure".to_string());

    assert_eq!(
        conversation
            .viewport_transcript_handoff_release_messages()
            .map(|messages| messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>()),
        Some(vec!["agentless runtime failure"])
    );
    assert!(!conversation.can_accept_manual_prompt());

    let correlation = conversation
        .viewport_transcript_handoff_correlation()
        .expect("released handoff should have a correlation");
    assert!(conversation.acknowledge_viewport_transcript_handoff_flush(&correlation));
    assert!(conversation.can_accept_manual_prompt());
}

#[test]
fn manual_preparation_failure_waits_for_delivery_and_restores_its_status() {
    let mut conversation = ready_conversation();
    conversation.record_manual_preparation_failure(
        "submitted prompt".to_string(),
        "turn preparation failed / workspace unavailable".to_string(),
    );

    assert_eq!(
        conversation
            .viewport_transcript_handoff_release_messages()
            .map(|messages| messages
                .iter()
                .map(|message| (message.kind, message.text.as_str()))
                .collect::<Vec<_>>()),
        Some(vec![(ConversationMessageKind::User, "submitted prompt")])
    );
    assert!(!conversation.can_accept_manual_prompt());

    conversation
        .record_status_message("conversation is busy; wait before opening a new draft".to_string());
    assert_eq!(
        conversation.status_text_for_viewport(),
        "turn preparation failed / workspace unavailable"
    );
    let correlation = conversation
        .viewport_transcript_handoff_correlation()
        .expect("released handoff should have a correlation");
    assert!(conversation.acknowledge_viewport_transcript_handoff_flush(&correlation));
    assert_eq!(
        conversation.status_text,
        "turn preparation failed / workspace unavailable"
    );
    assert!(conversation.can_accept_manual_prompt());
}

#[test]
fn tool_only_turn_waits_for_transcript_delivery_before_unlocking_navigation() {
    let mut conversation = ready_conversation();
    start_running_turn(&mut conversation, "turn-1");
    conversation.buffer_tool_message("tool-only completion");
    conversation.finish_turn("turn-1", &[]);
    conversation.begin_post_turn_settlement("turn-1");
    settle_post_turn(&mut conversation, "turn-1");

    assert!(conversation.complete_post_turn_settlement("turn-1"));
    assert_eq!(
        conversation
            .viewport_transcript_handoff_release_messages()
            .map(|messages| messages
                .iter()
                .map(|message| (message.kind, message.text.as_str()))
                .collect::<Vec<_>>()),
        Some(vec![(
            ConversationMessageKind::Tool,
            "tool-only completion"
        )])
    );
    assert!(!conversation.can_accept_manual_prompt());

    let correlation = conversation
        .viewport_transcript_handoff_correlation()
        .expect("released handoff should have a correlation");
    assert!(conversation.acknowledge_viewport_transcript_handoff_flush(&correlation));
    assert!(conversation.can_accept_manual_prompt());
}

#[test]
fn stale_handoff_ack_cannot_clear_a_newer_transcript_frontier() {
    let mut conversation = ready_conversation();
    start_running_turn(&mut conversation, "turn-1");
    conversation.push_live_agent_delta(
        "answer-1".to_string(),
        Some("final_answer".to_string()),
        "first terminal answer".to_string(),
    );
    conversation.finish_turn("turn-1", &[]);
    conversation.begin_post_turn_settlement("turn-1");

    let stale = conversation
        .viewport_transcript_handoff_correlation()
        .expect("released handoff should have a correlation");
    assert!(conversation.append_status_message("late terminal notice"));

    assert!(!conversation.acknowledge_viewport_transcript_handoff_flush(&stale));
    assert!(conversation.has_pending_viewport_transcript_handoff());
    let current = conversation
        .viewport_transcript_handoff_correlation()
        .expect("new transcript frontier should remain correlated");
    assert_ne!(current, stale);
    assert!(conversation.acknowledge_viewport_transcript_handoff_flush(&current));
}

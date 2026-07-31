use super::{ProgressiveActivityItemKind, ProgressiveActivityState, bounded_item_id_digest};
use crate::domain::conversation_item_lifecycle::{
    ConversationItemKind, ConversationItemLifecycleConsistency,
    ConversationItemLifecycleObservation, ConversationItemLifecyclePhase,
    ConversationItemLifecycleSource, ConversationItemOutcome,
    MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS,
};
use crate::domain::conversation_progressive_activity::{
    ConversationProgressiveActivityBatch, ConversationProgressiveActivityKind,
    ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
    ConversationProgressiveActivityProjection, ConversationProgressiveFileChange,
    ConversationProgressiveFileChangeKind, ConversationProgressivePlanStep,
    ConversationProgressivePlanStepStatus, ConversationProgressiveTokenUsage,
    ConversationProgressiveTokenUsageBreakdown,
};

fn lifecycle_observation(
    item_id: &str,
    kind: ConversationItemKind,
    phase: ConversationItemLifecyclePhase,
) -> ConversationItemLifecycleObservation {
    ConversationItemLifecycleObservation {
        thread_id: "thread-rail".to_string(),
        turn_id: "turn-rail".to_string(),
        item_id: item_id.to_string(),
        kind,
        phase,
        source: ConversationItemLifecycleSource::Live,
        observed_at_ms: Some(1),
        outcome: if phase == ConversationItemLifecyclePhase::Started {
            ConversationItemOutcome::InProgress
        } else {
            ConversationItemOutcome::Completed
        },
        summary: "raw lifecycle summary must not be retained".to_string(),
    }
}

fn command_observation(sequence: u64, text: &str) -> ConversationProgressiveActivityObservation {
    command_observation_for(sequence, "command-rail", text)
}

fn command_observation_for(
    sequence: u64,
    item_id: &str,
    text: &str,
) -> ConversationProgressiveActivityObservation {
    ConversationProgressiveActivityObservation {
        sequence,
        thread_id: "thread-rail".to_string(),
        turn_id: Some("turn-rail".to_string()),
        item_id: Some(item_id.to_string()),
        kind: ConversationProgressiveActivityKind::CommandOutput,
        payload: ConversationProgressiveActivityPayload::CommandOutput {
            tail: text.to_string(),
            chunk_count: 1,
            source_bytes: text.len() as u64,
            newline_count: text.bytes().filter(|byte| *byte == b'\n').count() as u64,
            ends_with_newline: text.ends_with('\n'),
            truncated_bytes: 0,
        },
    }
}

fn progressive_observation(
    sequence: u64,
    item_id: Option<&str>,
    kind: ConversationProgressiveActivityKind,
    payload: ConversationProgressiveActivityPayload,
) -> ConversationProgressiveActivityObservation {
    ConversationProgressiveActivityObservation {
        sequence,
        thread_id: "thread-rail".to_string(),
        turn_id: Some("turn-rail".to_string()),
        item_id: item_id.map(str::to_string),
        kind,
        payload,
    }
}

fn start_item(state: &mut ProgressiveActivityState, item_id: &str, kind: ConversationItemKind) {
    state.observe_item_lifecycle(
        &lifecycle_observation(item_id, kind, ConversationItemLifecyclePhase::Started),
        Some(ConversationItemLifecycleConsistency::Accepted),
    );
}

fn complete_item(state: &mut ProgressiveActivityState, item_id: &str, kind: ConversationItemKind) {
    state.observe_item_lifecycle(
        &lifecycle_observation(item_id, kind, ConversationItemLifecyclePhase::Completed),
        Some(ConversationItemLifecycleConsistency::Accepted),
    );
}

fn apply_batch(
    projection: &mut ConversationProgressiveActivityProjection,
    observation: ConversationProgressiveActivityObservation,
) {
    projection
        .apply_batch_correlated(
            Some("thread-rail"),
            Some("turn-rail"),
            ConversationProgressiveActivityBatch::single(observation)
                .expect("test observation should form a valid batch"),
        )
        .expect("test batch should enter the projection");
}

#[test]
fn lifecycle_tracks_only_bounded_identity_and_kind_then_clears_matching_completion() {
    let mut state = ProgressiveActivityState::default();
    let started = lifecycle_observation(
        "secret-item-id",
        ConversationItemKind::CommandExecution,
        ConversationItemLifecyclePhase::Started,
    );
    state.observe_item_lifecycle(&started, None);
    state.assign_observation(&command_observation_for(0, "secret-item-id", "stale line"));
    assert_eq!(state.active_item_kind(), None);
    assert_eq!(state.command_line_count(), 0);

    state.observe_item_lifecycle(
        &started,
        Some(ConversationItemLifecycleConsistency::Accepted),
    );
    assert_eq!(state.command_line_count(), 0);
    state.assign_observation(&command_observation_for(1, "secret-item-id", "one line"));

    assert_eq!(
        state.active_item_kind(),
        Some(ProgressiveActivityItemKind::CommandExecution)
    );
    assert_eq!(state.active_terminal_count(), 1);
    assert_eq!(state.command_line_count(), 1);
    assert!(!format!("{state:?}").contains("secret-item-id"));
    assert!(!format!("{state:?}").contains(&started.summary));

    let duplicate_started = lifecycle_observation(
        "ignored-duplicate",
        ConversationItemKind::FileChange,
        ConversationItemLifecyclePhase::Started,
    );
    state.observe_item_lifecycle(
        &duplicate_started,
        Some(ConversationItemLifecycleConsistency::DuplicateStart),
    );
    assert_eq!(
        state.active_item_kind(),
        Some(ProgressiveActivityItemKind::CommandExecution)
    );

    let unrelated_completion = lifecycle_observation(
        "other-item",
        ConversationItemKind::CommandExecution,
        ConversationItemLifecyclePhase::Completed,
    );
    state.observe_item_lifecycle(
        &unrelated_completion,
        Some(ConversationItemLifecycleConsistency::CompletionWithoutStart),
    );
    assert_eq!(
        state.active_item_kind(),
        Some(ProgressiveActivityItemKind::CommandExecution)
    );

    let completed = lifecycle_observation(
        "secret-item-id",
        ConversationItemKind::CommandExecution,
        ConversationItemLifecyclePhase::Completed,
    );
    state.observe_item_lifecycle(
        &completed,
        Some(ConversationItemLifecycleConsistency::Accepted),
    );
    assert_eq!(state.active_item_kind(), None);
    assert_eq!(state.active_terminal_count(), 0);
    assert_eq!(state.command_line_count(), 0);
}

#[test]
fn projection_update_assigns_coalesced_cumulative_command_lines() {
    let mut projection = ConversationProgressiveActivityProjection::default();
    let mut state = ProgressiveActivityState::default();
    start_item(
        &mut state,
        "command-rail",
        ConversationItemKind::CommandExecution,
    );

    apply_batch(&mut projection, command_observation(0, "first\n"));
    state.apply_projection_update(&projection.snapshot(), Some(0), Some(0), 0, 0, 0, 0);
    assert_eq!(state.command_line_count(), 1);

    apply_batch(&mut projection, command_observation(1, "second\nthird"));
    state.apply_projection_update(&projection.snapshot(), Some(1), Some(1), 0, 0, 0, 0);

    assert_eq!(state.command_line_count(), 3);
}

#[test]
fn history_only_update_sets_bounded_flag_without_fabricating_detail() {
    let mut batch =
        ConversationProgressiveActivityBatch::single(command_observation(4, "discarded secret\n"))
            .expect("test observation should form a valid batch");
    batch.discard_retained_records();
    let mut projection = ConversationProgressiveActivityProjection::default();
    projection
        .apply_batch_correlated(Some("thread-rail"), Some("turn-rail"), batch)
        .expect("history-only batch should enter the projection");

    let mut state = ProgressiveActivityState::default();
    state.apply_projection_update(&projection.snapshot(), Some(4), Some(4), 0, 1, 0, 0);

    assert!(state.bounded_history());
    assert_eq!(state.command_line_count(), 0);
    assert!(!format!("{state:?}").contains("discarded secret"));
}

#[test]
fn superseded_publication_alone_does_not_mark_history_bounded() {
    let mut batch = ConversationProgressiveActivityBatch::single(command_observation(0, "kept\n"))
        .expect("test observation should form a valid batch");
    batch
        .record_superseded_publication()
        .expect("test counter should remain in range");
    let mut projection = ConversationProgressiveActivityProjection::default();
    projection
        .apply_batch_correlated(Some("thread-rail"), Some("turn-rail"), batch)
        .expect("test batch should enter the projection");

    let mut state = ProgressiveActivityState::default();
    start_item(
        &mut state,
        "command-rail",
        ConversationItemKind::CommandExecution,
    );
    state.apply_projection_update(&projection.snapshot(), Some(0), Some(0), 0, 0, 0, 0);

    assert!(!state.bounded_history());
    assert_eq!(state.command_line_count(), 1);
}

#[test]
fn projection_side_coalescing_truncation_marks_history_bounded() {
    let mut projection = ConversationProgressiveActivityProjection::default();
    apply_batch(
        &mut projection,
        command_observation(0, &"a".repeat(40 * 1024)),
    );
    apply_batch(
        &mut projection,
        command_observation(1, &"b".repeat(40 * 1024)),
    );
    let snapshot = projection.snapshot();
    assert_eq!(snapshot.payload_truncation_count, 1);

    let mut state = ProgressiveActivityState::default();
    start_item(
        &mut state,
        "command-rail",
        ConversationItemKind::CommandExecution,
    );
    state.apply_projection_update(&snapshot, Some(1), Some(1), 0, 0, 0, 0);

    assert!(state.bounded_history());
}

#[test]
fn compact_projection_never_retains_raw_progressive_text() {
    const SECRET: &str = "RAW_PROGRESSIVE_SECRET_CANARY";
    let mut state = ProgressiveActivityState::default();

    start_item(
        &mut state,
        "command-secret-id",
        ConversationItemKind::CommandExecution,
    );
    start_item(
        &mut state,
        "patch-secret-id",
        ConversationItemKind::FileChange,
    );
    start_item(
        &mut state,
        "mcp-secret-id",
        ConversationItemKind::McpToolCall,
    );
    state.assign_observation(&command_observation_for(0, "command-secret-id", SECRET));
    state.assign_observation(&progressive_observation(
        1,
        Some("patch-secret-id"),
        ConversationProgressiveActivityKind::FileChangePatch,
        ConversationProgressiveActivityPayload::FileChangePatch {
            changes: vec![ConversationProgressiveFileChange {
                path: SECRET.to_string(),
                diff: SECRET.to_string(),
                kind: ConversationProgressiveFileChangeKind::Add,
            }],
            omitted_change_count: 2,
            source_bytes: (SECRET.len() * 2) as u64,
            truncated_bytes: 0,
        },
    ));
    state.assign_observation(&progressive_observation(
        2,
        Some("mcp-secret-id"),
        ConversationProgressiveActivityKind::McpProgress,
        ConversationProgressiveActivityPayload::McpProgress {
            message: SECRET.to_string(),
            update_count: 7,
            source_bytes: SECRET.len() as u64,
            truncated_bytes: 0,
        },
    ));
    state.assign_observation(&progressive_observation(
        3,
        None,
        ConversationProgressiveActivityKind::TurnDiff,
        ConversationProgressiveActivityPayload::TurnDiff {
            detail: SECRET.to_string(),
            source_bytes: SECRET.len() as u64,
            line_count: 6,
            addition_count: 3,
            deletion_count: 2,
            hunk_count: 1,
            truncated_bytes: 0,
        },
    ));
    state.assign_observation(&progressive_observation(
        4,
        None,
        ConversationProgressiveActivityKind::TurnPlan,
        ConversationProgressiveActivityPayload::TurnPlan {
            explanation: Some(SECRET.to_string()),
            steps: vec![ConversationProgressivePlanStep {
                status: ConversationProgressivePlanStepStatus::Completed,
                text: SECRET.to_string(),
            }],
            omitted_step_count: 4,
            source_bytes: (SECRET.len() * 2) as u64,
            truncated_bytes: 0,
        },
    ));
    state.assign_observation(&progressive_observation(
        5,
        None,
        ConversationProgressiveActivityKind::TokenUsage,
        ConversationProgressiveActivityPayload::TokenUsage {
            usage: ConversationProgressiveTokenUsage {
                last: ConversationProgressiveTokenUsageBreakdown {
                    cached_input_tokens: 0,
                    input_tokens: 7_500,
                    output_tokens: 0,
                    reasoning_output_tokens: 0,
                    total_tokens: 7_500,
                },
                total: ConversationProgressiveTokenUsageBreakdown {
                    cached_input_tokens: 0,
                    input_tokens: 7_500,
                    output_tokens: 0,
                    reasoning_output_tokens: 0,
                    total_tokens: 7_500,
                },
                model_context_window: Some(10_000),
            },
        },
    ));
    state.assign_observation(&progressive_observation(
        6,
        None,
        ConversationProgressiveActivityKind::GuardianWarning,
        ConversationProgressiveActivityPayload::GuardianWarning {
            message: SECRET.to_string(),
            update_count: 1,
            source_bytes: SECRET.len() as u64,
            truncated_bytes: 0,
        },
    ));

    let debug = format!("{state:?}");
    assert!(!debug.contains(SECRET));
    assert!(!debug.contains("command-secret-id"));
    assert!(!debug.contains("patch-secret-id"));
    assert!(!debug.contains("mcp-secret-id"));
    assert_eq!(state.patch_count(), 3);
    assert_eq!(state.mcp_update_count(), 7);
    assert_eq!(state.diff_addition_count(), 3);
    assert_eq!(state.diff_deletion_count(), 2);
    assert_eq!(state.diff_hunk_count(), 1);
    assert_eq!(state.plan_completed_count(), 1);
    assert_eq!(state.plan_total_count(), 5);
    assert_eq!(state.plan_omitted_count(), 4);
    assert_eq!(state.context_pressure_basis_points(), Some(7_500));
}

#[test]
fn concurrent_completion_removes_only_the_matching_active_item() {
    let mut state = ProgressiveActivityState::default();
    start_item(
        &mut state,
        "command-a",
        ConversationItemKind::CommandExecution,
    );
    start_item(
        &mut state,
        "command-b",
        ConversationItemKind::CommandExecution,
    );
    state.assign_observation(&command_observation_for(0, "command-a", "a1\na2"));
    state.assign_observation(&command_observation_for(1, "command-b", "b1"));
    assert_eq!(state.active_items.len(), 2);
    assert_eq!(state.command_line_count(), 3);

    complete_item(
        &mut state,
        "command-b",
        ConversationItemKind::CommandExecution,
    );

    assert_eq!(state.active_items.len(), 1);
    assert_eq!(state.command_line_count(), 2);
    assert_eq!(
        state.active_item_kind(),
        Some(ProgressiveActivityItemKind::CommandExecution)
    );
    assert_eq!(
        state.active_items[0].item_id_digest,
        bounded_item_id_digest("command-a").unwrap()
    );
}

#[test]
fn matching_completion_clears_active_item_after_core_history_loss_or_timestamp_regression() {
    let mut state = ProgressiveActivityState::default();
    start_item(
        &mut state,
        "long-running-command",
        ConversationItemKind::CommandExecution,
    );
    let completed = lifecycle_observation(
        "long-running-command",
        ConversationItemKind::CommandExecution,
        ConversationItemLifecyclePhase::Completed,
    );

    state.observe_item_lifecycle(
        &completed,
        Some(ConversationItemLifecycleConsistency::CompletionWithoutStart),
    );

    assert_eq!(state.active_item_kind(), None);
    assert!(state.bounded_history());

    state.reset();
    start_item(
        &mut state,
        "clock-skew-command",
        ConversationItemKind::CommandExecution,
    );
    let completed = lifecycle_observation(
        "clock-skew-command",
        ConversationItemKind::CommandExecution,
        ConversationItemLifecyclePhase::Completed,
    );
    state.observe_item_lifecycle(
        &completed,
        Some(ConversationItemLifecycleConsistency::TimestampRegression),
    );

    assert_eq!(state.active_item_kind(), None);
    assert!(!state.bounded_history());
}

#[test]
fn anomalous_start_never_creates_active_item() {
    let mut state = ProgressiveActivityState::default();
    let started = lifecycle_observation(
        "clock-skew-command",
        ConversationItemKind::CommandExecution,
        ConversationItemLifecyclePhase::Started,
    );

    state.observe_item_lifecycle(
        &started,
        Some(ConversationItemLifecycleConsistency::TimestampRegression),
    );

    assert_eq!(state.active_item_kind(), None);
}

#[test]
fn item_scoped_update_without_matching_active_identity_is_ignored() {
    let mut state = ProgressiveActivityState::default();
    start_item(
        &mut state,
        "command-a",
        ConversationItemKind::CommandExecution,
    );

    state.assign_observation(&command_observation_for(0, "not-started", "secret output"));
    state.assign_observation(&progressive_observation(
        1,
        Some("command-a"),
        ConversationProgressiveActivityKind::FileChangePatch,
        ConversationProgressiveActivityPayload::FileChangePatch {
            changes: vec![ConversationProgressiveFileChange {
                path: "mismatched".to_string(),
                diff: "mismatched".to_string(),
                kind: ConversationProgressiveFileChangeKind::Add,
            }],
            omitted_change_count: 0,
            source_bytes: 20,
            truncated_bytes: 0,
        },
    ));
    state.assign_observation(&progressive_observation(
        2,
        None,
        ConversationProgressiveActivityKind::CommandOutput,
        ConversationProgressiveActivityPayload::CommandOutput {
            tail: "missing identity".to_string(),
            chunk_count: 1,
            source_bytes: 16,
            newline_count: 0,
            ends_with_newline: false,
            truncated_bytes: 0,
        },
    ));

    assert_eq!(state.command_line_count(), 0);
    assert_eq!(state.patch_count(), 0);
    assert_eq!(state.active_items.len(), 1);
}

#[test]
fn active_item_priority_is_stable_across_concurrent_completion() {
    let mut state = ProgressiveActivityState::default();
    for (item_id, kind) in [
        ("reasoning", ConversationItemKind::Reasoning),
        ("mcp", ConversationItemKind::McpToolCall),
        ("plan", ConversationItemKind::Plan),
        ("patch", ConversationItemKind::FileChange),
        ("command", ConversationItemKind::CommandExecution),
    ] {
        start_item(&mut state, item_id, kind);
    }

    assert_eq!(
        state.active_item_kind(),
        Some(ProgressiveActivityItemKind::CommandExecution)
    );
    complete_item(
        &mut state,
        "command",
        ConversationItemKind::CommandExecution,
    );
    assert_eq!(
        state.active_item_kind(),
        Some(ProgressiveActivityItemKind::FileChange)
    );
    complete_item(&mut state, "patch", ConversationItemKind::FileChange);
    assert_eq!(
        state.active_item_kind(),
        Some(ProgressiveActivityItemKind::Plan)
    );
    complete_item(&mut state, "plan", ConversationItemKind::Plan);
    assert_eq!(
        state.active_item_kind(),
        Some(ProgressiveActivityItemKind::McpToolCall)
    );
    complete_item(&mut state, "mcp", ConversationItemKind::McpToolCall);
    assert_eq!(
        state.active_item_kind(),
        Some(ProgressiveActivityItemKind::Reasoning)
    );
}

#[test]
fn active_item_capacity_fails_closed_without_retaining_raw_ids() {
    const SECRET_PREFIX: &str = "CAPACITY_RAW_SECRET";
    let mut state = ProgressiveActivityState::default();
    for index in 0..MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS {
        start_item(
            &mut state,
            &format!("{SECRET_PREFIX}-{index}"),
            ConversationItemKind::SubAgentActivity,
        );
    }
    assert_eq!(
        state.active_items.len(),
        MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS
    );
    assert!(!state.bounded_history());

    start_item(
        &mut state,
        "CAPACITY_RAW_SECRET-overflow",
        ConversationItemKind::CommandExecution,
    );

    assert_eq!(
        state.active_items.len(),
        MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS
    );
    assert!(state.bounded_history());
    assert!(!format!("{state:?}").contains(SECRET_PREFIX));
}

#[test]
fn primary_fact_excludes_context_and_history_only_state() {
    let mut state = ProgressiveActivityState {
        context_pressure_basis_points: Some(9_500),
        bounded_history: true,
        ..ProgressiveActivityState::default()
    };
    assert!(!state.has_primary_fact());

    start_item(
        &mut state,
        "active-command",
        ConversationItemKind::CommandExecution,
    );
    assert!(state.has_primary_fact());
    complete_item(
        &mut state,
        "active-command",
        ConversationItemKind::CommandExecution,
    );
    assert!(!state.has_primary_fact());

    state.plan_total_count = 1;
    assert!(state.has_primary_fact());
}

#[test]
fn reset_clears_lifecycle_metrics_and_history_flag() {
    let mut state = ProgressiveActivityState::default();
    let started = lifecycle_observation(
        "item-before-reset",
        ConversationItemKind::FileChange,
        ConversationItemLifecyclePhase::Started,
    );
    state.observe_item_lifecycle(
        &started,
        Some(ConversationItemLifecycleConsistency::Accepted),
    );
    state.bounded_history = true;

    state.reset();

    assert_eq!(state, ProgressiveActivityState::default());
}

#[test]
fn view_model_lifecycle_boundaries_reset_transient_progressive_state() {
    use super::super::view_model::ConversationViewModel;

    fn seed(state: &mut ProgressiveActivityState) {
        let started = lifecycle_observation(
            "item-before-boundary",
            ConversationItemKind::McpToolCall,
            ConversationItemLifecyclePhase::Started,
        );
        state.observe_item_lifecycle(
            &started,
            Some(ConversationItemLifecycleConsistency::Accepted),
        );
        state.assign_observation(&progressive_observation(
            0,
            Some("item-before-boundary"),
            ConversationProgressiveActivityKind::McpProgress,
            ConversationProgressiveActivityPayload::McpProgress {
                message: "discarded at boundary".to_string(),
                update_count: 3,
                source_bytes: 21,
                truncated_bytes: 0,
            },
        ));
        state.bounded_history = true;
    }

    let mut view_model = ConversationViewModel::new_draft("/workspace".to_string());
    seed(&mut view_model.progressive_activity);
    view_model.record_thread_prepared(
        "thread-rail".to_string(),
        "Rail".to_string(),
        "/workspace".to_string(),
    );
    assert_eq!(
        view_model.progressive_activity,
        ProgressiveActivityState::default()
    );

    seed(&mut view_model.progressive_activity);
    view_model.record_turn_started("turn-one".to_string());
    assert_eq!(
        view_model.progressive_activity,
        ProgressiveActivityState::default()
    );

    seed(&mut view_model.progressive_activity);
    view_model.finish_turn("turn-one", &[]);
    assert_eq!(
        view_model.progressive_activity,
        ProgressiveActivityState::default()
    );

    view_model.record_turn_started("turn-two".to_string());
    seed(&mut view_model.progressive_activity);
    view_model.fail_turn(Some("turn-two"), "failed".to_string());
    assert_eq!(
        view_model.progressive_activity,
        ProgressiveActivityState::default()
    );
}

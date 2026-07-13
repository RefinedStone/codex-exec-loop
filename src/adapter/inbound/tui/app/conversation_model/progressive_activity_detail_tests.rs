use std::sync::Arc;

use super::super::view_model::ConversationViewModel;
use super::{ProgressiveActivityDetailKind, ProgressiveActivityDetailState};
use crate::domain::conversation_progressive_activity::{
    ConversationProgressiveActivityBatch, ConversationProgressiveActivityKind,
    ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
    ConversationProgressiveActivityProjection, ConversationProgressiveActivityProjectionSnapshot,
};

const THREAD_ID: &str = "thread-detail";
const TURN_ID: &str = "turn-detail";
const SECRET: &str = "RAW_PROGRESSIVE_DETAIL_SECRET";

fn observation(
    sequence: u64,
    item_id: Option<&str>,
    kind: ConversationProgressiveActivityKind,
    payload: ConversationProgressiveActivityPayload,
) -> ConversationProgressiveActivityObservation {
    ConversationProgressiveActivityObservation {
        sequence,
        thread_id: THREAD_ID.to_string(),
        turn_id: Some(TURN_ID.to_string()),
        item_id: item_id.map(str::to_string),
        kind,
        payload,
    }
}

fn command_observation(
    sequence: u64,
    item_id: &str,
    tail: &str,
    truncated_bytes: u64,
) -> ConversationProgressiveActivityObservation {
    observation(
        sequence,
        Some(item_id),
        ConversationProgressiveActivityKind::CommandOutput,
        ConversationProgressiveActivityPayload::CommandOutput {
            tail: tail.to_string(),
            chunk_count: 1,
            source_bytes: u64::try_from(tail.len())
                .unwrap_or(u64::MAX)
                .saturating_add(truncated_bytes),
            newline_count: tail.bytes().filter(|byte| *byte == b'\n').count() as u64,
            ends_with_newline: tail.ends_with('\n'),
            truncated_bytes,
        },
    )
}

fn diff_observation(
    sequence: u64,
    detail: &str,
    truncated_bytes: u64,
) -> ConversationProgressiveActivityObservation {
    observation(
        sequence,
        None,
        ConversationProgressiveActivityKind::TurnDiff,
        ConversationProgressiveActivityPayload::TurnDiff {
            detail: detail.to_string(),
            source_bytes: u64::try_from(detail.len())
                .unwrap_or(u64::MAX)
                .saturating_add(truncated_bytes),
            line_count: 2,
            addition_count: 1,
            deletion_count: 0,
            hunk_count: 1,
            truncated_bytes,
        },
    )
}

fn snapshot(
    observations: impl IntoIterator<Item = ConversationProgressiveActivityObservation>,
) -> Arc<ConversationProgressiveActivityProjectionSnapshot> {
    let mut projection = ConversationProgressiveActivityProjection::default();
    for observation in observations {
        let batch = ConversationProgressiveActivityBatch::single(observation)
            .expect("detail test observation should be valid");
        projection
            .apply_batch_correlated(Some(THREAD_ID), Some(TURN_ID), batch)
            .expect("detail test batch should project");
    }
    projection.snapshot()
}

#[test]
fn replacement_keeps_only_a_weak_core_snapshot_reference() {
    let first = snapshot([command_observation(0, "command-a", "first", 0)]);
    let second = snapshot([command_observation(1, "command-b", "second", 0)]);
    let mut state = ProgressiveActivityDetailState::default();

    state.replace_snapshot(&first);
    assert_eq!(Arc::strong_count(&first), 1);
    state.replace_snapshot(&second);

    assert_eq!(Arc::strong_count(&first), 1);
    assert_eq!(Arc::strong_count(&second), 1);
    let document = state
        .document(ProgressiveActivityDetailKind::Output)
        .expect("replacement output document");
    assert_eq!(Arc::strong_count(&second), 2);
    assert_eq!(document.sequence, 1);
    assert_eq!(document.text(), "second");
    drop(document);
    assert_eq!(Arc::strong_count(&second), 1);
}

#[test]
fn core_mutation_invalidates_weak_detail_without_forcing_snapshot_cow() {
    let mut projection = ConversationProgressiveActivityProjection::default();
    projection
        .apply_batch_correlated(
            Some(THREAD_ID),
            Some(TURN_ID),
            ConversationProgressiveActivityBatch::single(command_observation(
                0,
                "command-a",
                "first",
                0,
            ))
            .expect("first command batch"),
        )
        .expect("first command projection");
    let first = projection.snapshot();
    let mut state = ProgressiveActivityDetailState::default();
    state.replace_snapshot(&first);
    assert_eq!(Arc::strong_count(&first), 2);
    drop(first);

    projection
        .apply_batch_correlated(
            Some(THREAD_ID),
            Some(TURN_ID),
            ConversationProgressiveActivityBatch::single(command_observation(
                1,
                "command-b",
                "second",
                0,
            ))
            .expect("second command batch"),
        )
        .expect("second command projection");

    assert!(
        state
            .document(ProgressiveActivityDetailKind::Output)
            .is_none(),
        "Arc::make_mut should dissociate the old weak snapshot instead of cloning it"
    );
    let second = projection.snapshot();
    state.replace_snapshot(&second);
    assert_eq!(
        state
            .document(ProgressiveActivityDetailKind::Output)
            .expect("rebound output document")
            .text(),
        "second"
    );
}

#[test]
fn document_selects_latest_output_and_diff_with_exact_accounting() {
    let projection = snapshot([
        command_observation(0, "command-a", "old", 0),
        diff_observation(1, "@@\n+x\n", 3),
        command_observation(2, "command-b", "new tail", 4),
    ]);
    let mut state = ProgressiveActivityDetailState::default();
    state.replace_snapshot(&projection);

    let output = state
        .document(ProgressiveActivityDetailKind::Output)
        .expect("latest output document");
    assert_eq!(output.kind, ProgressiveActivityDetailKind::Output);
    assert_eq!(output.sequence, 2);
    assert_eq!(output.text(), "new tail");
    assert_eq!(output.source_bytes, 12);
    assert_eq!(output.retained_bytes, 8);
    assert_eq!(output.truncated_bytes, 4);
    assert!(output.history_incomplete);

    let diff = state
        .document(ProgressiveActivityDetailKind::Diff)
        .expect("latest diff document");
    assert_eq!(diff.kind, ProgressiveActivityDetailKind::Diff);
    assert_eq!(diff.sequence, 1);
    assert_eq!(diff.text(), "@@\n+x\n");
    assert_eq!(diff.source_bytes, 9);
    assert_eq!(diff.retained_bytes, 6);
    assert_eq!(diff.truncated_bytes, 3);
    assert!(diff.history_incomplete);
}

#[test]
fn reasoning_mcp_and_guardian_payloads_expose_no_document_or_secret_debug_text() {
    let projection = snapshot([
        observation(
            0,
            Some("mcp-secret-item"),
            ConversationProgressiveActivityKind::McpProgress,
            ConversationProgressiveActivityPayload::McpProgress {
                message: SECRET.to_string(),
                update_count: 1,
                source_bytes: SECRET.len() as u64,
                truncated_bytes: 0,
            },
        ),
        observation(
            1,
            Some("reasoning-secret-item"),
            ConversationProgressiveActivityKind::ReasoningTextDelta,
            ConversationProgressiveActivityPayload::ReasoningTextDelta {
                content_index: 0,
                chunk_count: 1,
                source_bytes: SECRET.len() as u64,
            },
        ),
        ConversationProgressiveActivityObservation {
            sequence: 2,
            thread_id: THREAD_ID.to_string(),
            turn_id: None,
            item_id: None,
            kind: ConversationProgressiveActivityKind::GuardianWarning,
            payload: ConversationProgressiveActivityPayload::GuardianWarning {
                message: SECRET.to_string(),
                update_count: 1,
                source_bytes: SECRET.len() as u64,
                truncated_bytes: 0,
            },
        },
    ]);
    let mut state = ProgressiveActivityDetailState::default();
    state.replace_snapshot(&projection);

    assert!(
        state
            .document(ProgressiveActivityDetailKind::Diff)
            .is_none()
    );
    assert!(
        state
            .document(ProgressiveActivityDetailKind::Output)
            .is_none()
    );
    assert!(!format!("{state:?}").contains(SECRET));
    assert!(!format!("{state:?}").contains("mcp-secret-item"));
    assert!(!format!("{state:?}").contains("reasoning-secret-item"));
}

#[test]
fn state_and_document_debug_redact_retained_text() {
    let projection = snapshot([command_observation(0, "command-secret", SECRET, 0)]);
    let mut state = ProgressiveActivityDetailState::default();
    state.replace_snapshot(&projection);

    let document = state
        .document(ProgressiveActivityDetailKind::Output)
        .expect("secret output document");
    assert_eq!(document.text(), SECRET);
    assert!(!format!("{document:?}").contains(SECRET));
    assert!(!format!("{state:?}").contains(SECRET));
    assert!(!format!("{state:?}").contains("command-secret"));
}

#[test]
fn reset_releases_snapshot_and_clears_documents() {
    let projection = snapshot([
        command_observation(0, "command-a", "output", 0),
        diff_observation(1, "@@\n+x\n", 0),
    ]);
    let mut state = ProgressiveActivityDetailState::default();
    state.replace_snapshot(&projection);

    state.reset();

    assert_eq!(Arc::strong_count(&projection), 1);
    assert!(
        state
            .document(ProgressiveActivityDetailKind::Diff)
            .is_none()
    );
    assert!(
        state
            .document(ProgressiveActivityDetailKind::Output)
            .is_none()
    );
}

#[test]
fn view_model_boundaries_reset_transient_detail_snapshot() {
    fn seed(
        conversation: &mut ConversationViewModel,
    ) -> Arc<ConversationProgressiveActivityProjectionSnapshot> {
        let snapshot = snapshot([command_observation(
            0,
            "command-boundary",
            "transient output",
            0,
        )]);
        conversation
            .progressive_activity_detail
            .replace_snapshot(&snapshot);
        snapshot
    }

    fn assert_empty(conversation: &ConversationViewModel) {
        assert!(
            conversation
                .progressive_activity_detail
                .document(ProgressiveActivityDetailKind::Output)
                .is_none()
        );
    }

    let mut conversation = ConversationViewModel::new_draft("/workspace".to_string());
    let _snapshot_owner = seed(&mut conversation);
    conversation.record_thread_prepared(
        THREAD_ID.to_string(),
        "Detail thread".to_string(),
        "/workspace".to_string(),
    );
    assert_empty(&conversation);

    let _snapshot_owner = seed(&mut conversation);
    conversation.record_turn_started(TURN_ID.to_string());
    assert_empty(&conversation);

    let _snapshot_owner = seed(&mut conversation);
    conversation.finish_turn(TURN_ID, &[]);
    assert_empty(&conversation);

    conversation.record_turn_started("turn-failure".to_string());
    let _snapshot_owner = seed(&mut conversation);
    conversation.fail_turn("provider failed".to_string());
    assert_empty(&conversation);
}

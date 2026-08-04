use std::time::Instant;

/*
 * This file owns mutable presentation state for one conversation. Persisted
 * transcript facts come from the domain snapshot, while active turn, approval,
 * auto-follow, and post-turn authority arrive as one immutable Core runtime
 * projection. The view model may add composer, viewport, and readable status
 * affordances, but it must not reproduce those semantic transitions.
 */
#[path = "view_model/messages.rs"]
mod messages;
#[path = "view_model/status.rs"]
mod status;

use crate::core::app::conversation::ConversationThreadReviewSnapshot;
use crate::core::app::{
    ActiveTurnPhase, ApprovalAuthorityPhase, AutoFollowAuthoritySnapshot, AutoFollowPhase,
    ConversationRuntimeSnapshot, PostTurnAuthoritySnapshot,
};

use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalRequest, ConversationApprovalReview,
    ConversationMessage, ConversationMessageKind, ConversationRuntimeControlTruth,
    ConversationSnapshot,
};
use crate::domain::conversation_runtime_envelope::ConversationRuntimeEnvelope;
use crate::domain::planning::{
    PlanningQueueMutationReceipt, PlanningRepairRequestSnapshot, TaskHandoff as PlanningTaskHandoff,
};

use super::activity_rail::ActivityRailTerminalState;
use super::auto_follow::{
    AUTO_FOLLOW_MODE_LABEL, AutoFollowSkipReason, AutoFollowSnapshotPresentation,
};
use super::composer_state::ConversationComposerState;
use super::progressive_activity::ProgressiveActivityState;
use super::progressive_activity_detail::ProgressiveActivityDetailState;
use super::turn_activity::TurnActivityState;

const MAX_BASE_WARNINGS: usize = 128;
const MAX_RUNTIME_NOTICES: usize = 256;
const MAX_AUXILIARY_HISTORY_BYTES: usize = 1024 * 1024;
const MAX_AUXILIARY_ITEM_BYTES: usize = 64 * 1024;
const MAX_AUXILIARY_ITEM_LINES: usize = 128;

// Shell rendering keeps this presentation mirror around core conversation
// lifecycle snapshots so loading/failed panels do not fabricate a view model.
#[derive(Debug, Clone)]
pub(crate) enum ConversationState {
    Loading,
    Ready(Box<ConversationViewModel>),
    Failed(String),
}
impl ConversationState {
    pub(crate) fn ready(conversation: ConversationViewModel) -> Self {
        Self::Ready(Box::new(conversation))
    }
}

// Input state is the submit gate used by key handling and runtime callbacks.
// Draft/continue can accept a prompt; submitting/streaming belong to a live turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationInputState {
    DraftReady,
    ReadyToContinue,
    SubmittingTurn,
    StreamingTurn,
}
impl ConversationInputState {
    pub(crate) fn can_submit_now(self) -> bool {
        matches!(self, Self::DraftReady | Self::ReadyToContinue)
    }
}

// Last auto-follow action is kept as copy-ready status history after the phase
// itself has already moved on or been cleared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedAutoFollowActivity {
    pub(crate) summary: String,
    pub(crate) detail: String,
}

// Planning repair lives in the view model because it is driven by TUI retry
// affordances while still carrying application-layer repair requests verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanningRepairState {
    pub(crate) attempts_used: usize,
    pub(crate) max_attempts: usize,
    pub(crate) latest_request: PlanningRepairRequestSnapshot,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct HydratedThreadReviewStatusProjection {
    summary: Option<String>,
    manual_handoff_context: Option<String>,
}

/*
 * Terminal transcript delivery is acknowledged asynchronously from the view
 * model's perspective: a projection is sampled, terminal I/O happens, and only
 * then does the adapter return a receipt. This correlation prevents a receipt
 * for an older transcript frontier from clearing a newer handoff.
 */
#[derive(Debug, Clone)]
pub(crate) struct ConversationViewModel {
    pub(crate) thread_id: String,
    pub(crate) title: String,
    pub(crate) cwd: String,
    /*
     * Before app-server returns a real thread, cwd is speculative draft state.
     * Once a thread exists, cwd is authoritative and draft_workspace_directory is
     * retained only so a blank draft can still follow workspace selector changes.
     */
    pub(crate) draft_workspace_directory: String,
    pub(crate) messages: Vec<ConversationMessage>,
    // Base warnings come from the loaded snapshot; warnings adds view-local status context.
    pub(crate) base_warnings: Vec<String>,
    pub(crate) warnings: Vec<String>,
    // Runtime notices are de-duplicated so repeated recovery probes do not spam the footer.
    pub(crate) runtime_notices: Vec<String>,
    // Headless core owns correlation and applied-state reduction. The TUI retains
    // the resulting envelope for later P0-D presentation without reparsing wire data.
    pub(crate) runtime_envelope: Option<ConversationRuntimeEnvelope>,
    // Prompt editing is isolated from transcript, runtime, planning, approval,
    // and viewport state by the composer reducer boundary.
    pub(crate) composer: ConversationComposerState,
    /*
     * Immutable projection copied from Core AppSnapshot. This is deliberately
     * one opaque value instead of TUI-owned active/approval/auto-follow fields:
     * `apply_runtime_snapshot` replaces it atomically and no TUI reducer may
     * perform a semantic transition inside it.
     */
    runtime_snapshot: ConversationRuntimeSnapshot,
    pub(crate) planning_repair_state: Option<PlanningRepairState>,
    transcript_revision: u64,
    pub(crate) turn_activity: TurnActivityState,
    pub(crate) progressive_activity: ProgressiveActivityState,
    pub(crate) progressive_activity_detail: ProgressiveActivityDetailState,
    pub(crate) activity_rail_terminal_state: Option<ActivityRailTerminalState>,
    pub(crate) approval_detail_scroll_offset: usize,
    pub(crate) turn_control_truth: ConversationRuntimeControlTruth,
    pub(crate) last_auto_follow_activity: Option<RecordedAutoFollowActivity>,
    hydrated_thread_review_status_projection: HydratedThreadReviewStatusProjection,
    pub(crate) latest_queue_mutation_receipt: Option<PlanningQueueMutationReceipt>,
    pub(crate) status_text: String,
}
impl ConversationViewModel {
    #[cfg(test)]
    pub(crate) fn new_draft(cwd: String) -> Self {
        Self::new_draft_with_truth(cwd, ConversationRuntimeControlTruth::default())
    }
    pub(crate) fn new_draft_with_truth(
        cwd: String,
        turn_control_truth: ConversationRuntimeControlTruth,
    ) -> Self {
        let base_status = "new thread draft".to_string();
        let mut view_model = Self {
            thread_id: String::new(),
            title: "New conversation".to_string(),
            cwd: cwd.clone(),
            draft_workspace_directory: cwd,
            messages: Vec::new(),
            base_warnings: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            runtime_envelope: None,
            composer: ConversationComposerState::default(),
            runtime_snapshot: ConversationRuntimeSnapshot::initial(),
            planning_repair_state: None,
            transcript_revision: 0,
            turn_activity: TurnActivityState::default(),
            progressive_activity: ProgressiveActivityState::default(),
            progressive_activity_detail: ProgressiveActivityDetailState::default(),
            activity_rail_terminal_state: None,
            approval_detail_scroll_offset: 0,
            turn_control_truth,
            last_auto_follow_activity: None,
            hydrated_thread_review_status_projection: HydratedThreadReviewStatusProjection::default(
            ),
            latest_queue_mutation_receipt: None,
            status_text: String::new(),
        };
        view_model.enforce_transcript_retention();
        retain_bounded_string_history(&mut view_model.base_warnings, MAX_BASE_WARNINGS);
        view_model.warnings = view_model.base_warnings.clone();
        retain_bounded_string_history(&mut view_model.runtime_notices, MAX_RUNTIME_NOTICES);
        view_model.set_status_with_warnings(base_status);
        view_model
    }
    pub(crate) fn from_snapshot_with_truth(
        snapshot: ConversationSnapshot,
        draft_workspace_directory: String,
        turn_control_truth: ConversationRuntimeControlTruth,
        thread_review: Vec<ConversationThreadReviewSnapshot>,
    ) -> Self {
        // Snapshot warnings are preserved as the immutable baseline for this loaded thread.
        let base_warnings = snapshot.warnings;
        let runtime_notices = snapshot.runtime_notices;
        let warnings = base_warnings.clone();
        let hydrated_thread_review_status_projection =
            hydrate_thread_review_status_projection(&thread_review);
        let base_status = "thread loaded".to_string();
        let conversation_cwd = if snapshot.cwd.trim().is_empty() {
            draft_workspace_directory.clone()
        } else {
            snapshot.cwd
        };
        let mut view_model = Self {
            thread_id: snapshot.thread_id,
            title: snapshot.title,
            cwd: conversation_cwd,
            draft_workspace_directory,
            messages: snapshot.messages,
            base_warnings,
            warnings,
            runtime_notices,
            runtime_envelope: None,
            composer: ConversationComposerState::default(),
            runtime_snapshot: ConversationRuntimeSnapshot::initial(),
            planning_repair_state: None,
            transcript_revision: 0,
            turn_activity: TurnActivityState::default(),
            progressive_activity: ProgressiveActivityState::default(),
            progressive_activity_detail: ProgressiveActivityDetailState::default(),
            activity_rail_terminal_state: None,
            approval_detail_scroll_offset: 0,
            turn_control_truth,
            hydrated_thread_review_status_projection,
            last_auto_follow_activity: None,
            latest_queue_mutation_receipt: None,
            status_text: String::new(),
        };
        view_model.enforce_transcript_retention();
        retain_bounded_string_history(&mut view_model.base_warnings, MAX_BASE_WARNINGS);
        view_model.warnings = view_model.base_warnings.clone();
        retain_bounded_string_history(&mut view_model.runtime_notices, MAX_RUNTIME_NOTICES);
        view_model.set_status_with_warnings(base_status);
        view_model
    }
    pub(crate) fn turn_control_truth(&self) -> ConversationRuntimeControlTruth {
        self.turn_control_truth
    }
    pub(crate) fn resumed_thread_review_summary(&self) -> Option<&str> {
        self.hydrated_thread_review_status_projection
            .summary
            .as_deref()
    }
    pub(crate) fn resumed_thread_review_manual_handoff_context(&self) -> Option<&str> {
        self.hydrated_thread_review_status_projection
            .manual_handoff_context
            .as_deref()
    }
    pub(crate) fn draft_workspace_directory(&self) -> &str {
        self.draft_workspace_directory.as_str()
    }
    pub(crate) fn planning_workspace_directory(&self) -> &str {
        if self.has_active_thread() {
            self.cwd.as_str()
        } else {
            self.draft_workspace_directory()
        }
    }
    pub(crate) fn sync_draft_workspace(&mut self, workspace_directory: String) -> bool {
        // Active threads are pinned to their app-server cwd; only blank drafts track selector moves.
        if self.has_active_thread() || self.draft_workspace_directory == workspace_directory {
            return false;
        }

        self.draft_workspace_directory = workspace_directory.clone();
        self.cwd = workspace_directory;
        self.base_warnings.clear();
        self.warnings.clear();
        self.clear_auto_follow_skip();
        self.set_status_with_warnings("draft workspace synced".to_string());

        true
    }
    pub(crate) fn record_submitted_prompt(
        &mut self,
        transcript_message: ConversationMessage,
        _workspace_directory: String,
        clear_input_buffer: bool,
    ) {
        // Submission writes the user transcript immediately; stream callbacks fill in the reply.
        self.push_message(transcript_message);
        if clear_input_buffer {
            self.composer.clear_input_buffer();
        }
        self.composer.startup_submit_armed = false;
        self.activity_rail_terminal_state = None;
    }
    pub(crate) fn record_manual_preparation_failure(
        &mut self,
        transcript_text: String,
        status_text: String,
    ) {
        self.push_message(ConversationMessage::new(
            ConversationMessageKind::User,
            transcript_text,
            None,
            None,
        ));
        self.composer.clear_input_buffer();
        self.status_text = status_text;
    }
    pub(crate) fn record_thread_prepared(&mut self, thread_id: String, title: String, cwd: String) {
        // Thread preparation upgrades a draft into an app-server backed conversation.
        self.progressive_activity.reset();
        self.progressive_activity_detail.reset();
        self.activity_rail_terminal_state = None;
        let reattached_same_thread = self.thread_id == thread_id && self.has_active_thread();
        self.thread_id = thread_id;
        self.title = title.clone();
        self.cwd = cwd;
        if !reattached_same_thread {
            self.status_text = "thread started".to_string();
            self.append_status_message("thread opened / ".to_string() + &title);
        }
    }
    pub(crate) fn record_turn_started(&mut self, _turn_id: String) {
        self.progressive_activity.reset();
        self.progressive_activity_detail.reset();
        self.activity_rail_terminal_state = None;
        self.turn_activity.start_new_turn();
        self.approval_detail_scroll_offset = 0;
        // Auto-follow has its own phase text, but still shares the transcript status rail.
        if let AutoFollowPhase::Running { turn_index, .. } = self.runtime_snapshot.auto_follow.phase
        {
            let max_auto_turns = self.runtime_snapshot.auto_follow.max_auto_turns_label();
            let status_text = format!(
                "auto-follow running / turn {turn_index}/{max_auto_turns} / mode: {}",
                AUTO_FOLLOW_MODE_LABEL,
            );
            self.status_text = status_text.clone();
            self.append_status_message(status_text);
        } else {
            self.status_text = "turn started".to_string();
            self.append_status_message("turn started");
        }
    }
    pub(crate) fn has_active_thread(&self) -> bool {
        !self.thread_id.trim().is_empty()
    }
    pub(crate) fn is_blank_draft(&self) -> bool {
        !self.has_active_thread()
            && self.messages.is_empty()
            && self.composer.input_buffer.trim().is_empty()
            && !self.runtime_snapshot.has_active_turn()
    }
    pub(crate) fn apply_runtime_snapshot(&mut self, snapshot: ConversationRuntimeSnapshot) {
        self.runtime_snapshot = snapshot;
    }
    pub(crate) fn runtime_snapshot(&self) -> &ConversationRuntimeSnapshot {
        &self.runtime_snapshot
    }
    pub(crate) fn input_state(&self) -> ConversationInputState {
        match self
            .runtime_snapshot
            .active_turn
            .as_ref()
            .map(|turn| turn.phase)
        {
            Some(ActiveTurnPhase::Submitting) => ConversationInputState::SubmittingTurn,
            Some(ActiveTurnPhase::Running) => ConversationInputState::StreamingTurn,
            None if self.has_active_thread() => ConversationInputState::ReadyToContinue,
            None => ConversationInputState::DraftReady,
        }
    }
    pub(crate) fn active_turn_id(&self) -> Option<&str> {
        self.runtime_snapshot
            .active_turn
            .as_ref()
            .and_then(|turn| turn.turn_id.as_deref())
    }
    pub(crate) fn active_turn_workspace_directory(&self) -> Option<&str> {
        self.runtime_snapshot
            .active_turn
            .as_ref()
            .map(|turn| turn.workspace_directory.as_str())
    }
    pub(crate) fn auto_follow_state(&self) -> &AutoFollowAuthoritySnapshot {
        &self.runtime_snapshot.auto_follow
    }
    pub(crate) fn can_accept_runtime_prompt(&self) -> bool {
        self.runtime_snapshot.can_accept_runtime_prompt()
    }
    pub(crate) fn can_accept_manual_prompt(&self) -> bool {
        self.runtime_snapshot.can_accept_manual_prompt()
    }
    pub(crate) fn has_running_turn(&self) -> bool {
        self.runtime_snapshot.has_running_turn()
    }
    pub(crate) fn live_activity_started_at(&self) -> Option<Instant> {
        // Status timers prefer auto-follow evaluation/queue phases over a plain active turn.
        match &self.runtime_snapshot.post_turn {
            PostTurnAuthoritySnapshot::Evaluating { started_at, .. }
            | PostTurnAuthoritySnapshot::AwaitingRoute { started_at, .. } => Some(*started_at),
            PostTurnAuthoritySnapshot::Idle | PostTurnAuthoritySnapshot::Settled { .. } => None,
        }
        .or_else(|| self.runtime_snapshot.auto_follow.active_started_at())
        .or_else(|| {
            self.runtime_snapshot
                .active_turn
                .as_ref()
                .filter(|turn| turn.phase == ActiveTurnPhase::Running)
                .map(|turn| turn.started_at)
        })
    }
    pub(crate) fn pending_approval_request(&self) -> Option<&ConversationApprovalRequest> {
        self.runtime_snapshot
            .approval
            .as_ref()
            .map(|approval| &approval.request)
    }
    pub(crate) fn approval_review(&self) -> Option<&ConversationApprovalReview> {
        self.runtime_snapshot.approval_review.as_ref()
    }
    pub(crate) fn pending_approval_decision(&self) -> Option<ConversationApprovalDecision> {
        self.runtime_snapshot
            .approval
            .as_ref()
            .filter(|approval| {
                matches!(
                    approval.phase,
                    ApprovalAuthorityPhase::Submitting | ApprovalAuthorityPhase::Submitted
                )
            })
            .and_then(|approval| approval.decision.as_ref())
            .map(|correlation| correlation.decision)
    }
    pub(crate) fn move_approval_detail_scroll(&mut self, delta: isize) {
        if self.pending_approval_request().is_none() {
            self.approval_detail_scroll_offset = 0;
            return;
        }
        // Rendered approval rows depend on terminal width, so the reducer cannot
        // clamp against logical detail count. The renderer applies the exact
        // wrapped-row maximum for the current frame.
        self.approval_detail_scroll_offset = self
            .approval_detail_scroll_offset
            .saturating_add_signed(delta)
            .min(u16::MAX as usize);
    }
    pub(crate) fn finish_turn(
        &mut self,
        turn_id: &str,
        changed_planning_file_paths: &[String],
    ) -> String {
        // Return the workspace that produced this turn so post-turn planning uses the same root.
        let workspace_directory = self
            .active_turn_workspace_directory()
            .map(str::to_string)
            .unwrap_or_else(|| self.planning_workspace_directory().to_string());

        self.turn_activity
            .register_changed_planning_file_paths(changed_planning_file_paths);
        self.turn_activity.complete_turn(turn_id);
        self.progressive_activity.reset();
        self.progressive_activity_detail.reset();
        self.activity_rail_terminal_state = None;
        self.approval_detail_scroll_offset = 0;

        workspace_directory
    }
    pub(crate) fn fail_turn(&mut self, failed_turn_id: Option<&str>, message: String) {
        self.fail_turn_with_terminal_state(
            failed_turn_id,
            message,
            Some(ActivityRailTerminalState::RuntimeFailed),
        );
    }
    pub(crate) fn fail_turn_with_terminal_state(
        &mut self,
        _failed_turn_id: Option<&str>,
        message: String,
        terminal_state: Option<ActivityRailTerminalState>,
    ) {
        // Preserve whatever stream content arrived before failure, then reopen the input gate.
        self.progressive_activity.reset();
        self.progressive_activity_detail.reset();
        self.approval_detail_scroll_offset = 0;
        self.activity_rail_terminal_state = terminal_state;
        self.status_text = "turn failed".to_string();
        self.append_status_message(message);
    }
    pub(crate) fn extend_runtime_notices<I>(&mut self, notices: I)
    where
        I: IntoIterator<Item = String>,
    {
        // Runtime recovery can emit the same notice on several ticks; keep the operator copy stable.
        for notice in notices {
            let mut notice = notice;
            messages::truncate_text_to_limits(
                &mut notice,
                MAX_AUXILIARY_ITEM_BYTES,
                MAX_AUXILIARY_ITEM_LINES,
            );
            if !self.runtime_notices.contains(&notice) {
                self.runtime_notices.push(notice);
            }
        }
        retain_bounded_string_history(&mut self.runtime_notices, MAX_RUNTIME_NOTICES);
    }
    pub(crate) fn record_auto_follow_skip(&mut self, reason: AutoFollowSkipReason) {
        let detail = reason.detail(&self.runtime_snapshot.auto_follow, &self.turn_activity);
        // A skip ends post-turn evaluation but keeps a readable activity record for the footer.
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: reason
                .activity_summary(&self.runtime_snapshot.auto_follow)
                .to_string(),
            detail,
        });
    }
    pub(crate) fn clear_auto_follow_skip(&mut self) {
        self.last_auto_follow_activity = None;
    }
    pub(crate) fn record_stale_auto_follow_submission(&mut self) {
        let summary = "auto-follow cancelled".to_string();
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: summary.clone(),
            detail:
                "queued follow-up expired before Core admission; manual input remains available"
                    .to_string(),
        });
        self.status_text = summary.clone();
        self.append_status_message(summary);
    }
    pub(crate) fn record_internal_continuation_paused(&mut self) {
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: "stopped: auto-follow disarmed".to_string(),
            detail: "auto-follow remains disarmed until :turns is explicitly set again".to_string(),
        });
    }
    pub(crate) fn record_auto_follow_submission(&mut self, _completed_turn_id: &str) {
        // Submission stores the handoff so later status copy can explain which planning task moved.
        let turn_index = self
            .runtime_snapshot
            .auto_follow
            .active_turn_index()
            .unwrap_or_else(|| self.runtime_snapshot.auto_follow.next_auto_turn_index());
        let progress = format!(
            "{turn_index}/{}",
            self.runtime_snapshot.auto_follow.max_auto_turns_label()
        );
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: format!("submitted auto turn {progress}"),
            detail: "queued after the previous turn completed; submitted planning auto-follow"
                .to_string(),
        });
    }
    pub(crate) fn record_auto_follow_queue(&mut self, _completed_turn_id: &str) {
        // Queueing records progress before the runtime owns the prompt submission.
        let turn_index = self
            .runtime_snapshot
            .auto_follow
            .active_turn_index()
            .unwrap_or_else(|| self.runtime_snapshot.auto_follow.next_auto_turn_index());
        let next_progress = format!(
            "{turn_index}/{}",
            self.runtime_snapshot.auto_follow.max_auto_turns_label()
        );
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: format!("queued auto turn {next_progress}"),
            detail:
                "queued after the previous turn completed; waiting to submit planning auto-follow"
                    .to_string(),
        });
    }
    pub(crate) fn record_auto_follow_parallel_dispatch(&mut self) {
        /*
         * Parallel mode consumes the post-turn queue signal as a pool dispatch
         * instead of submitting an in-session auto turn. Core has already
         * settled the authority phase before this presentation note is recorded.
         */
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: "delegated: parallel dispatch".to_string(),
            detail: "post-turn queue handoff opened parallel mode dispatch instead of an auto turn"
                .to_string(),
        });
    }
    pub(crate) fn begin_post_turn_settlement(&mut self, _completed_turn_id: &str) {
        /*
         * Every completed turn queues post-turn evaluation, including parallel
         * continuation when the single-session turn budget is off. Keep manual
         * intake closed until that exact evaluation settles; otherwise an
         * operator Enter can race the planning worker and parallel dispatcher.
         */
        self.status_text = "turn completed / evaluating post-turn continuation".to_string();
    }
    pub(crate) fn complete_post_turn_settlement(&mut self, completed_turn_id: &str) -> bool {
        matches!(
            &self.runtime_snapshot.post_turn,
            PostTurnAuthoritySnapshot::Settled { correlation, .. }
                if correlation.completed_turn_id.trim() == completed_turn_id.trim()
        )
    }
    pub(crate) fn has_post_turn_settlement_in_flight(&self) -> bool {
        self.runtime_snapshot.post_turn.is_in_flight()
    }
    pub(crate) fn last_planning_task_handoff(&self) -> Option<&PlanningTaskHandoff> {
        self.runtime_snapshot.planning_handoff.as_ref()
    }
    #[cfg(test)]
    pub(crate) fn replace_planning_handoff_for_test(
        &mut self,
        handoff: Option<PlanningTaskHandoff>,
    ) {
        let mut snapshot = self.runtime_snapshot.clone();
        snapshot.planning_handoff = handoff;
        self.apply_runtime_snapshot(snapshot);
    }
}

fn retain_bounded_string_history(values: &mut Vec<String>, max_items: usize) {
    for value in values.iter_mut() {
        messages::truncate_text_to_limits(
            value,
            MAX_AUXILIARY_ITEM_BYTES,
            MAX_AUXILIARY_ITEM_LINES,
        );
    }
    let mut retained_bytes = values.iter().map(String::len).sum::<usize>();
    let mut remove_count = 0;
    while remove_count < values.len()
        && (values.len().saturating_sub(remove_count) > max_items
            || retained_bytes > MAX_AUXILIARY_HISTORY_BYTES)
    {
        retained_bytes = retained_bytes.saturating_sub(values[remove_count].len());
        remove_count += 1;
    }
    if remove_count > 0 {
        values.drain(0..remove_count);
    }
}

fn hydrate_thread_review_status_projection(
    thread_review: &[ConversationThreadReviewSnapshot],
) -> HydratedThreadReviewStatusProjection {
    HydratedThreadReviewStatusProjection {
        summary: thread_review
            .iter()
            .rev()
            .find_map(build_thread_review_status_summary),
        manual_handoff_context: thread_review
            .iter()
            .rev()
            .find_map(build_thread_review_manual_handoff_context),
    }
}

fn build_thread_review_status_summary(review: &ConversationThreadReviewSnapshot) -> Option<String> {
    let review_label = review.review_label.trim();
    let review_state = review.review_state.trim();
    let review_summary = review.review_summary.trim();
    if review_label.is_empty() && review_state.is_empty() && review_summary.is_empty() {
        return None;
    }

    Some(
        match (
            review_label.is_empty(),
            review_state.is_empty(),
            review_summary.is_empty(),
        ) {
            (false, false, false) => format!("{review_label} ({review_state}): {review_summary}"),
            (false, false, true) => format!("{review_label} ({review_state})"),
            (false, true, false) => format!("{review_label}: {review_summary}"),
            (true, false, false) => format!("{review_state}: {review_summary}"),
            (false, true, true) => review_label.to_string(),
            (true, false, true) => review_state.to_string(),
            (true, true, false) => review_summary.to_string(),
            (true, true, true) => unreachable!(),
        },
    )
}

fn build_thread_review_manual_handoff_context(
    review: &ConversationThreadReviewSnapshot,
) -> Option<String> {
    let handoff_target = review
        .handoff_target
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let handoff_note = review
        .handoff_note
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (handoff_target, handoff_note) {
        (Some(target), Some(note)) => Some(format!("{target}: {note}")),
        (Some(target), None) => Some(target.to_string()),
        (None, Some(note)) => Some(note.to_string()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_hydration_keeps_latest_review_summary_and_manual_handoff_context() {
        let conversation = ConversationViewModel::from_snapshot_with_truth(
            sample_conversation_snapshot(),
            "/tmp/root".to_string(),
            ConversationRuntimeControlTruth::default(),
            vec![
                sample_thread_review(
                    "approval review",
                    "pending",
                    "older repository summary",
                    None,
                    None,
                    "2026-05-01T00:00:00Z",
                    "2026-05-01T00:00:00Z",
                ),
                sample_thread_review(
                    "manual handoff",
                    "waiting",
                    "operator follow-up required",
                    Some("operator"),
                    Some("open review center inbox"),
                    "2026-05-02T00:00:00Z",
                    "2026-05-02T00:00:00Z",
                ),
            ],
        );

        assert_eq!(
            conversation.resumed_thread_review_summary(),
            Some("manual handoff (waiting): operator follow-up required")
        );
        assert_eq!(
            conversation.resumed_thread_review_manual_handoff_context(),
            Some("operator: open review center inbox")
        );
    }

    #[test]
    fn snapshot_hydration_falls_back_to_draft_workspace_when_snapshot_cwd_missing() {
        let conversation = ConversationViewModel::from_snapshot_with_truth(
            ConversationSnapshot {
                thread_id: "thread-1".to_string(),
                title: "thread-1".to_string(),
                cwd: String::new(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
                item_lifecycle: Default::default(),
            },
            "/tmp/fallback-root".to_string(),
            ConversationRuntimeControlTruth::default(),
            Vec::new(),
        );

        assert_eq!(conversation.cwd, "/tmp/fallback-root");
        assert_eq!(
            conversation.planning_workspace_directory(),
            "/tmp/fallback-root"
        );
    }

    fn sample_conversation_snapshot() -> ConversationSnapshot {
        ConversationSnapshot {
            thread_id: "thread-1".to_string(),
            title: "thread-1".to_string(),
            cwd: "/tmp/root".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        }
    }

    fn sample_thread_review(
        review_label: &str,
        review_state: &str,
        review_summary: &str,
        handoff_target: Option<&str>,
        handoff_note: Option<&str>,
        requested_at: &str,
        updated_at: &str,
    ) -> ConversationThreadReviewSnapshot {
        ConversationThreadReviewSnapshot {
            thread_id: "thread-1".to_string(),
            review_id: format!("review-{updated_at}"),
            review_label: review_label.to_string(),
            review_state: review_state.to_string(),
            review_summary: review_summary.to_string(),
            requested_at: requested_at.to_string(),
            updated_at: updated_at.to_string(),
            handoff_target: handoff_target.map(str::to_string),
            handoff_note: handoff_note.map(str::to_string),
        }
    }
}

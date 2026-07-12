use std::time::Instant;

use ratatui::text::Line;

/*
 * This file owns the mutable TUI projection of a conversation. The domain
 * snapshot gives persisted transcript facts; the view model layers on transient
 * rendering caches, input affordances, active-turn bookkeeping, planning runtime
 * state, and auto-follow status that only exist while the operator is in the
 * native client.
 */
#[path = "view_model/messages.rs"]
mod messages;
#[path = "view_model/status.rs"]
mod status;

#[cfg(test)]
use crate::application::service::planning::{
    PlanningAutoFollowBlockReason, PlanningRuntimeAutoFollowDecision,
    PlanningRuntimeAutoFollowRequest, PlanningRuntimeUseCases,
};
use crate::core::app::conversation::ConversationThreadReviewSnapshot;

use crate::application::service::planning::{PlanningRuntimeProjection, PlanningTaskHandoff};
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalRequest, ConversationApprovalReview,
    ConversationMessage, ConversationMessageKind, ConversationRuntimeControlTruth,
    ConversationSnapshot,
};
use crate::domain::conversation_runtime_envelope::ConversationRuntimeEnvelope;
use crate::domain::planning::PlanningRepairRequestSnapshot;

use super::super::inline_shell_commands::{InlineShellCommand, InlineShellCommandPaletteState};
#[cfg(test)]
use super::auto_follow::AutoFollowDecision;
use super::auto_follow::{AutoFollowSkipReason, AutoFollowState};
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingApprovalResolution {
    approval_id: String,
    decision: ConversationApprovalDecision,
}

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
    // Rendered transcript cache is invalidated by message helpers in messages.rs.
    pub(crate) cached_conversation_lines: Vec<Line<'static>>,
    // Streaming assistant text is separate until completion to avoid duplicating partial deltas.
    pub(crate) live_agent_message: Option<ConversationMessage>,
    // Tool output may arrive before the assistant delta it should visually follow.
    pub(crate) buffered_tool_messages: Vec<ConversationMessage>,
    // Base warnings come from the loaded snapshot; warnings adds view-local status context.
    pub(crate) base_warnings: Vec<String>,
    pub(crate) warnings: Vec<String>,
    // Runtime notices are de-duplicated so repeated recovery probes do not spam the footer.
    pub(crate) runtime_notices: Vec<String>,
    // Headless core owns correlation and applied-state reduction. The TUI retains
    // the resulting envelope for later P0-D presentation without reparsing wire data.
    pub(crate) runtime_envelope: Option<ConversationRuntimeEnvelope>,
    pub(crate) input_buffer: String,
    input_cursor_byte_index: Option<usize>,
    pub(crate) inline_shell_command_palette_state: InlineShellCommandPaletteState,
    // Startup submit lets initial CLI text wait until the draft/thread is ready to accept it.
    pub(crate) startup_submit_armed: bool,
    // Active-turn fields bridge submission, app-server turn start, stream reduction, and finish.
    pub(crate) active_turn_id: Option<String>,
    pub(crate) active_turn_workspace_directory: Option<String>,
    pub(crate) active_turn_started_at: Option<Instant>,
    // Set after the shell has forwarded one interrupt for the active turn. It
    // prevents repeated Ctrl-C/:stop input from incrementing the global runtime
    // interrupt generation until this turn starts, finishes, or the request fails.
    pub(crate) interrupt_request_pending: bool,
    pub(crate) planning_repair_state: Option<PlanningRepairState>,
    pub(crate) input_state: ConversationInputState,
    pub(crate) auto_follow_state: AutoFollowState,
    // Transitional service snapshot used only by reducer/event synchronization.
    // Rendering and post-turn worker context must read the core snapshot instead.
    reducer_event_projection_cache: PlanningRuntimeProjection,
    pub(crate) turn_activity: TurnActivityState,
    // Approval review is tied to the currently streaming turn and cleared on a new turn.
    pub(crate) approval_review: Option<ConversationApprovalReview>,
    pub(crate) pending_approval_request: Option<ConversationApprovalRequest>,
    // The first submitted choice is immutable until the runtime resolves or terminates the request.
    pending_approval_resolution: Option<PendingApprovalResolution>,
    pub(crate) approval_detail_scroll_offset: usize,
    pub(crate) turn_control_truth: ConversationRuntimeControlTruth,
    pub(crate) last_auto_follow_activity: Option<RecordedAutoFollowActivity>,
    hydrated_thread_review_status_projection: HydratedThreadReviewStatusProjection,
    pub(crate) last_planning_task_handoff: Option<PlanningTaskHandoff>,
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
            cached_conversation_lines: Vec::new(),
            live_agent_message: None,
            buffered_tool_messages: Vec::new(),
            base_warnings: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            runtime_envelope: None,
            input_buffer: String::new(),
            input_cursor_byte_index: None,
            inline_shell_command_palette_state: InlineShellCommandPaletteState::default(),
            startup_submit_armed: false,
            active_turn_id: None,
            active_turn_workspace_directory: None,
            active_turn_started_at: None,
            interrupt_request_pending: false,
            planning_repair_state: None,
            input_state: ConversationInputState::DraftReady,
            auto_follow_state: AutoFollowState::new(),
            reducer_event_projection_cache: PlanningRuntimeProjection::uninitialized(),
            turn_activity: TurnActivityState::default(),
            approval_review: None,
            pending_approval_request: None,
            pending_approval_resolution: None,
            approval_detail_scroll_offset: 0,
            turn_control_truth,
            last_auto_follow_activity: None,
            hydrated_thread_review_status_projection: HydratedThreadReviewStatusProjection::default(
            ),
            last_planning_task_handoff: None,
            status_text: String::new(),
        };
        view_model.enforce_transcript_retention();
        retain_bounded_string_history(&mut view_model.base_warnings, MAX_BASE_WARNINGS);
        view_model.warnings = view_model.base_warnings.clone();
        retain_bounded_string_history(&mut view_model.runtime_notices, MAX_RUNTIME_NOTICES);
        view_model.set_status_with_warnings(base_status);
        view_model.refresh_conversation_lines();
        view_model
    }
    #[cfg(test)]
    pub(crate) fn from_snapshot(
        snapshot: ConversationSnapshot,
        draft_workspace_directory: String,
    ) -> Self {
        Self::from_snapshot_with_truth(
            snapshot,
            draft_workspace_directory,
            ConversationRuntimeControlTruth::default(),
            Vec::new(),
        )
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
            cached_conversation_lines: Vec::new(),
            live_agent_message: None,
            buffered_tool_messages: Vec::new(),
            base_warnings,
            warnings,
            runtime_notices,
            runtime_envelope: None,
            input_buffer: String::new(),
            input_cursor_byte_index: None,
            inline_shell_command_palette_state: InlineShellCommandPaletteState::default(),
            startup_submit_armed: false,
            active_turn_id: None,
            active_turn_workspace_directory: None,
            active_turn_started_at: None,
            interrupt_request_pending: false,
            planning_repair_state: None,
            input_state: ConversationInputState::ReadyToContinue,
            auto_follow_state: AutoFollowState::new(),
            reducer_event_projection_cache: PlanningRuntimeProjection::uninitialized(),
            turn_activity: TurnActivityState::default(),
            approval_review: None,
            pending_approval_request: None,
            pending_approval_resolution: None,
            approval_detail_scroll_offset: 0,
            turn_control_truth,
            hydrated_thread_review_status_projection,
            last_auto_follow_activity: None,
            last_planning_task_handoff: None,
            status_text: String::new(),
        };
        view_model.enforce_transcript_retention();
        retain_bounded_string_history(&mut view_model.base_warnings, MAX_BASE_WARNINGS);
        view_model.warnings = view_model.base_warnings.clone();
        retain_bounded_string_history(&mut view_model.runtime_notices, MAX_RUNTIME_NOTICES);
        view_model.set_status_with_warnings(base_status);
        view_model.refresh_conversation_lines();
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
    pub(crate) fn reducer_event_projection_cache(&self) -> &PlanningRuntimeProjection {
        &self.reducer_event_projection_cache
    }
    pub(crate) fn replace_reducer_event_projection_cache(
        &mut self,
        reducer_event_projection_cache: PlanningRuntimeProjection,
    ) {
        // The app polls planning state outside the conversation stream; reducers keep this compatibility copy.
        self.reducer_event_projection_cache = reducer_event_projection_cache;
    }
    pub(crate) fn sync_inline_shell_command_palette(&mut self) {
        let preferred_selection = self.inline_shell_command_palette_state.selected_command();
        self.inline_shell_command_palette_state
            .sync_to_input(&self.input_buffer, preferred_selection);
    }
    pub(crate) fn input_cursor_byte_index(&self) -> usize {
        self.input_cursor_byte_index
            .map(|index| clamp_to_char_boundary(&self.input_buffer, index))
            .unwrap_or(self.input_buffer.len())
    }
    pub(crate) fn set_input_cursor_byte_index(&mut self, index: usize) {
        self.input_cursor_byte_index = Some(clamp_to_char_boundary(&self.input_buffer, index));
    }
    pub(crate) fn move_input_cursor_to_end(&mut self) {
        self.input_cursor_byte_index = None;
    }
    pub(crate) fn move_inline_shell_command_palette_selection(&mut self, delta: isize) -> bool {
        self.inline_shell_command_palette_state
            .move_selection(delta)
    }
    pub(crate) fn dismiss_inline_shell_command_palette(&mut self) -> bool {
        self.inline_shell_command_palette_state.dismiss()
    }
    pub(crate) fn insert_inline_shell_command_completion(&mut self, command: InlineShellCommand) {
        self.input_buffer = command.completion_text().to_string();
        self.move_input_cursor_to_end();
        self.sync_inline_shell_command_palette();
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
        self.auto_follow_state = AutoFollowState::new();
        self.base_warnings.clear();
        self.warnings.clear();
        self.clear_auto_follow_skip();
        self.set_status_with_warnings("draft workspace synced".to_string());

        true
    }
    pub(crate) fn record_submitted_prompt(
        &mut self,
        transcript_message: ConversationMessage,
        workspace_directory: String,
        clear_input_buffer: bool,
    ) {
        // Submission writes the user transcript immediately; stream callbacks fill in the reply.
        self.push_message(transcript_message);
        if clear_input_buffer {
            self.input_buffer.clear();
            self.input_cursor_byte_index = None;
            self.inline_shell_command_palette_state = InlineShellCommandPaletteState::default();
        }
        self.mark_turn_submitting(workspace_directory);
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
        self.input_buffer.clear();
        self.input_cursor_byte_index = None;
        self.inline_shell_command_palette_state = InlineShellCommandPaletteState::default();
        self.status_text = status_text;
    }
    pub(crate) fn record_thread_prepared(&mut self, thread_id: String, title: String, cwd: String) {
        // Thread preparation upgrades a draft into an app-server backed conversation.
        let reattached_same_thread = self.thread_id == thread_id && self.has_active_thread();
        self.thread_id = thread_id;
        self.title = title.clone();
        self.cwd = cwd;
        if !reattached_same_thread {
            self.status_text = "thread started".to_string();
            self.append_status_message("thread opened / ".to_string() + &title);
        }
    }
    pub(crate) fn record_turn_started(&mut self, turn_id: String) {
        self.mark_turn_started(turn_id);
        self.live_agent_message = None;
        // Auto-follow has its own phase text, but still shares the transcript status rail.
        if let Some(turn_index) = self.auto_follow_state.mark_auto_turn_started() {
            let max_auto_turns = self.auto_follow_state.max_auto_turns_label();
            let status_text = format!(
                "auto-follow running / turn {turn_index}/{max_auto_turns} / mode: {}",
                self.auto_follow_state.mode_label(),
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
            && self.input_buffer.trim().is_empty()
            && self.active_turn_id.is_none()
    }
    pub(crate) fn ready_input_state(&self) -> ConversationInputState {
        if self.has_active_thread() {
            ConversationInputState::ReadyToContinue
        } else {
            ConversationInputState::DraftReady
        }
    }
    pub(crate) fn can_accept_runtime_prompt(&self) -> bool {
        self.input_state.can_submit_now()
    }
    pub(crate) fn can_accept_manual_prompt(&self) -> bool {
        // Manual prompts wait for auto-follow bookkeeping to settle even if input_state is ready.
        self.can_accept_runtime_prompt() && !self.auto_follow_state.has_live_activity()
    }
    pub(crate) fn has_running_turn(&self) -> bool {
        !self.can_accept_runtime_prompt()
    }
    pub(crate) fn live_activity_started_at(&self) -> Option<Instant> {
        // Status timers prefer auto-follow evaluation/queue phases over a plain active turn.
        self.auto_follow_state.active_started_at().or_else(|| {
            self.active_turn_started_at
                .filter(|_| self.has_running_turn())
        })
    }
    pub(crate) fn arm_startup_submit(&mut self) {
        self.startup_submit_armed = true;
    }
    pub(crate) fn clear_startup_submit(&mut self) -> bool {
        std::mem::replace(&mut self.startup_submit_armed, false)
    }
    pub(crate) fn mark_turn_submitting(&mut self, workspace_directory: String) {
        self.startup_submit_armed = false;
        self.interrupt_request_pending = false;
        self.input_state = ConversationInputState::SubmittingTurn;
        self.active_turn_workspace_directory = Some(workspace_directory);
        self.active_turn_started_at = Some(Instant::now());
    }
    pub(crate) fn replace_active_turn_workspace_directory(&mut self, workspace_directory: String) {
        self.active_turn_workspace_directory = Some(workspace_directory);
    }
    pub(crate) fn mark_turn_started(&mut self, turn_id: String) {
        self.active_turn_id = Some(turn_id);
        // Keep a submitting-phase stop sticky. The runtime reducer resends that
        // interrupt after the concrete turn id arrives, closing the race where
        // app-server samples the first stop generation while starting the turn.
        self.input_state = ConversationInputState::StreamingTurn;
        // A recovered start may arrive without a prior submitting phase, so seed the timer here too.
        self.active_turn_started_at.get_or_insert_with(Instant::now);
        self.turn_activity.start_new_turn();
        self.approval_review = None;
        self.pending_approval_request = None;
        self.pending_approval_resolution = None;
        self.approval_detail_scroll_offset = 0;
        self.buffered_tool_messages.clear();
    }
    pub(crate) fn mark_turn_finished(&mut self) {
        self.active_turn_id = None;
        self.active_turn_workspace_directory = None;
        self.active_turn_started_at = None;
        self.interrupt_request_pending = false;
        self.pending_approval_request = None;
        self.pending_approval_resolution = None;
        self.approval_detail_scroll_offset = 0;
        self.input_state = self.ready_input_state();
    }
    pub(crate) fn set_pending_approval_request(&mut self, request: ConversationApprovalRequest) {
        let is_same_request = self
            .pending_approval_request
            .as_ref()
            .is_some_and(|current| current.approval_id == request.approval_id);
        if !is_same_request {
            self.pending_approval_resolution = None;
        }
        self.pending_approval_request = Some(request);
        self.approval_detail_scroll_offset = 0;
    }
    pub(crate) fn pending_approval_decision(&self) -> Option<ConversationApprovalDecision> {
        let request = self.pending_approval_request.as_ref()?;
        self.pending_approval_resolution
            .as_ref()
            .filter(|resolution| resolution.approval_id == request.approval_id)
            .map(|resolution| resolution.decision)
    }
    pub(crate) fn mark_approval_decision_submitted(
        &mut self,
        approval_id: &str,
        decision: ConversationApprovalDecision,
    ) -> bool {
        let resolves_current_request = self
            .pending_approval_request
            .as_ref()
            .is_some_and(|request| request.approval_id == approval_id);
        if !resolves_current_request || self.pending_approval_resolution.is_some() {
            return false;
        }

        self.pending_approval_resolution = Some(PendingApprovalResolution {
            approval_id: approval_id.to_string(),
            decision,
        });
        true
    }
    pub(crate) fn clear_pending_approval_request(&mut self, approval_id: &str) {
        if self
            .pending_approval_request
            .as_ref()
            .is_some_and(|request| request.approval_id == approval_id)
        {
            self.pending_approval_request = None;
            self.pending_approval_resolution = None;
            self.approval_detail_scroll_offset = 0;
        }
    }
    pub(crate) fn clear_pending_approval_resolution(&mut self, approval_id: &str) -> bool {
        let resolves_current_request = self
            .pending_approval_request
            .as_ref()
            .is_some_and(|request| request.approval_id == approval_id);
        let clears_submitted_decision = self
            .pending_approval_resolution
            .as_ref()
            .is_some_and(|resolution| resolution.approval_id == approval_id);
        if resolves_current_request && clears_submitted_decision {
            self.pending_approval_resolution = None;
            return true;
        }
        false
    }
    pub(crate) fn move_approval_detail_scroll(&mut self, delta: isize) {
        if self.pending_approval_request.is_none() {
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
    pub(crate) fn mark_interrupt_requested_once(&mut self) -> bool {
        if !self.has_running_turn() || self.interrupt_request_pending {
            return false;
        }
        self.interrupt_request_pending = true;
        true
    }
    pub(crate) fn clear_interrupt_request(&mut self) {
        self.interrupt_request_pending = false;
    }
    pub(crate) fn finish_turn(
        &mut self,
        turn_id: &str,
        changed_planning_file_paths: &[String],
    ) -> String {
        // Return the workspace that produced this turn so post-turn planning uses the same root.
        let workspace_directory = self
            .active_turn_workspace_directory
            .clone()
            .unwrap_or_else(|| self.planning_workspace_directory().to_string());

        self.commit_live_agent_message();
        self.flush_buffered_tool_messages();
        self.auto_follow_state.complete_auto_turn_if_running();
        self.turn_activity
            .register_changed_planning_file_paths(changed_planning_file_paths);
        self.turn_activity.complete_turn(turn_id);
        self.mark_turn_finished();

        workspace_directory
    }
    pub(crate) fn fail_turn(&mut self, message: String) {
        // Preserve whatever stream content arrived before failure, then reopen the input gate.
        self.commit_live_agent_message();
        self.flush_buffered_tool_messages();
        self.auto_follow_state.clear_runtime_phase();
        self.mark_turn_finished();
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
        let detail = reason.detail(&self.auto_follow_state, &self.turn_activity);
        // A skip ends post-turn evaluation but keeps a readable activity record for the footer.
        self.auto_follow_state.clear_runtime_phase();
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: reason.activity_summary(&self.auto_follow_state).to_string(),
            detail,
        });
    }
    pub(crate) fn clear_auto_follow_skip(&mut self) {
        self.last_auto_follow_activity = None;
    }
    pub(crate) fn pause_post_turn_continuation(&mut self) {
        self.auto_follow_state.pause_post_turn_continuation();
    }
    pub(crate) fn rearm_parallel_post_turn_continuation(&mut self) {
        self.auto_follow_state
            .rearm_parallel_post_turn_continuation();
    }
    pub(crate) fn disarm_parallel_post_turn_continuation(&mut self) {
        self.auto_follow_state
            .disarm_parallel_post_turn_continuation();
    }
    pub(crate) fn record_internal_continuation_paused(&mut self) {
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: "stopped: auto-follow disarmed".to_string(),
            detail: "auto-follow remains disarmed until :turns is explicitly set again".to_string(),
        });
    }
    pub(crate) fn clear_last_planning_task_handoff(&mut self) {
        self.last_planning_task_handoff = None;
    }
    pub(crate) fn record_manual_intake_handoff(
        &mut self,
        handoff_task: Option<&PlanningTaskHandoff>,
    ) {
        self.last_planning_task_handoff = handoff_task.cloned();
    }
    pub(crate) fn record_auto_follow_submission(
        &mut self,
        _completed_turn_id: &str,
        handoff_task: Option<&PlanningTaskHandoff>,
    ) {
        // Submission stores the handoff so later status copy can explain which planning task moved.
        let turn_index = self.auto_follow_state.mark_auto_turn_submitted();
        let progress = format!(
            "{turn_index}/{}",
            self.auto_follow_state.max_auto_turns_label()
        );
        self.last_planning_task_handoff = handoff_task.cloned();
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: format!("submitted auto turn {progress}"),
            detail: "queued after the previous turn completed; submitted planning auto-follow"
                .to_string(),
        });
    }
    pub(crate) fn record_auto_follow_queue(&mut self, _completed_turn_id: &str) {
        // Queueing records progress before the runtime owns the prompt submission.
        let turn_index = self.auto_follow_state.mark_auto_turn_queued();
        let next_progress = format!(
            "{turn_index}/{}",
            self.auto_follow_state.max_auto_turns_label()
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
         * instead of submitting an in-session auto turn. Clear the queued phase so
         * the footer does not show a forever-pending turn whose done counter can
         * never advance.
         */
        self.auto_follow_state.clear_runtime_phase();
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: "delegated: parallel dispatch".to_string(),
            detail: "post-turn queue handoff opened parallel mode dispatch instead of an auto turn"
                .to_string(),
        });
    }
    pub(crate) fn record_stale_parallel_only_continuation_cancelled(&mut self) {
        /*
         * A parallel-only post-turn decision can reach the UI after `:parallel
         * off` closed its epoch. It must not leave the conversation in Queued,
         * because that phase intentionally blocks manual input until submission.
         */
        self.auto_follow_state.clear_runtime_phase();
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: "cancelled: parallel mode disabled".to_string(),
            detail: "discarded a stale parallel-only continuation after the operator disabled parallel mode"
                .to_string(),
        });
        self.status_text =
            "turn completed / parallel continuation cancelled; auto-follow remains disabled"
                .to_string();
        self.append_status_message(self.status_text.clone());
    }
    pub(crate) fn begin_auto_follow_evaluation(&mut self) {
        /*
         * Every completed turn queues post-turn evaluation, including parallel
         * continuation when the single-session turn budget is off. Keep manual
         * intake closed until that exact evaluation settles; otherwise an
         * operator Enter can race the planning worker and parallel dispatcher.
         */
        self.auto_follow_state.begin_post_turn_evaluation();
        self.status_text = "turn completed / evaluating post-turn continuation".to_string();
    }
    pub(crate) fn last_planning_task_handoff(&self) -> Option<&PlanningTaskHandoff> {
        self.last_planning_task_handoff.as_ref()
    }
    #[cfg(test)]
    pub(crate) fn decide_auto_follow(
        &self,
        planning_runtime: &PlanningRuntimeUseCases,
    ) -> AutoFollowDecision {
        self.decide_auto_follow_with_snapshot(
            planning_runtime,
            &self.reducer_event_projection_cache,
        )
    }
    #[cfg(test)]
    pub(crate) fn decide_auto_follow_with_snapshot(
        &self,
        planning_runtime: &PlanningRuntimeUseCases,
        planning_runtime_projection: &PlanningRuntimeProjection,
    ) -> AutoFollowDecision {
        // Local conversation guards run before asking the planning service to compose a prompt.
        if self.auto_follow_state.post_turn_continuation_paused() {
            return AutoFollowDecision::Skip(AutoFollowSkipReason::PostTurnContinuationPaused);
        }
        if !self.auto_follow_state.can_queue_next() {
            return AutoFollowDecision::Skip(AutoFollowSkipReason::LimitReached);
        }
        let Some(last_message) = self.latest_agent_message_text() else {
            return AutoFollowDecision::Skip(AutoFollowSkipReason::NoAgentReply);
        };
        if self
            .auto_follow_state
            .stop_rules
            .stop_keyword
            .matches(last_message)
        {
            return AutoFollowDecision::Skip(AutoFollowSkipReason::StopKeywordMatched);
        }
        if self
            .auto_follow_state
            .stop_rules
            .should_stop_on_no_file_changes(self.turn_activity.last_completed_file_change_count())
        {
            return AutoFollowDecision::Skip(AutoFollowSkipReason::NoFileChanges);
        }
        // Service block reasons are mapped back to conversation-facing skip copy here.
        match planning_runtime.decide_auto_follow(PlanningRuntimeAutoFollowRequest {
            stop_keyword: self.auto_follow_state.stop_keyword_value(),
            last_message: last_message.trim(),
            projection: planning_runtime_projection,
        }) {
            PlanningRuntimeAutoFollowDecision::QueuePrompt(prompt) => {
                AutoFollowDecision::QueuePrompt(prompt)
            }
            PlanningRuntimeAutoFollowDecision::Blocked(block_reason) => {
                AutoFollowDecision::Skip(match block_reason {
                    PlanningAutoFollowBlockReason::InvalidWorkspace => {
                        AutoFollowSkipReason::PlanningBlocked
                    }
                    PlanningAutoFollowBlockReason::ActionableQueueRequired => {
                        AutoFollowSkipReason::PlanningQueueHeadRequired
                    }
                    PlanningAutoFollowBlockReason::RepeatedQueueHead => {
                        AutoFollowSkipReason::PlanningRepeatedQueueHead
                    }
                })
            }
        }
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

fn clamp_to_char_boundary(buffer: &str, index: usize) -> usize {
    let mut clamped_index = index.min(buffer.len());
    while clamped_index > 0 && !buffer.is_char_boundary(clamped_index) {
        clamped_index -= 1;
    }
    clamped_index
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

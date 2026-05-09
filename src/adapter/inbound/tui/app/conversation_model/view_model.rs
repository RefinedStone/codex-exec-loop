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
use crate::application::service::planning::{
    PlanningRepairRequest, PlanningRuntimeSnapshot, PlanningTaskHandoff,
};
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationMessage, ConversationMessageKind,
    ConversationRuntimeControlTruth, ConversationSnapshot,
};

use super::super::inline_shell_commands::{InlineShellCommand, InlineShellCommandPaletteState};
#[cfg(test)]
use super::auto_follow::AutoFollowDecision;
use super::auto_follow::{AutoFollowSkipReason, AutoFollowState};
use super::turn_activity::TurnActivityState;

// Shell rendering keeps this wrapper around load failures so the outer app can
// render an error panel without fabricating an empty conversation model.
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
    pub(crate) latest_request: PlanningRepairRequest,
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
    pub(crate) input_buffer: String,
    pub(crate) inline_shell_command_palette_state: InlineShellCommandPaletteState,
    // Startup submit lets initial CLI text wait until the draft/thread is ready to accept it.
    pub(crate) startup_submit_armed: bool,
    // Active-turn fields bridge submission, app-server turn start, stream reduction, and finish.
    pub(crate) active_turn_id: Option<String>,
    pub(crate) active_turn_workspace_directory: Option<String>,
    pub(crate) active_turn_started_at: Option<Instant>,
    pub(crate) planning_repair_state: Option<PlanningRepairState>,
    pub(crate) input_state: ConversationInputState,
    pub(crate) auto_follow_state: AutoFollowState,
    // Cached service snapshot used by post-turn auto-follow decisions.
    pub(crate) planning_runtime_snapshot: PlanningRuntimeSnapshot,
    pub(crate) turn_activity: TurnActivityState,
    // Approval review is tied to the currently streaming turn and cleared on a new turn.
    pub(crate) approval_review: Option<ConversationApprovalReview>,
    pub(crate) turn_control_truth: ConversationRuntimeControlTruth,
    pub(crate) last_auto_follow_activity: Option<RecordedAutoFollowActivity>,
    pub(crate) last_planning_task_handoff: Option<PlanningTaskHandoff>,
    // Idempotence guard for async post-turn evaluators racing with newer turns.
    pub(crate) last_applied_post_turn_evaluation_id: Option<String>,
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
            input_buffer: String::new(),
            inline_shell_command_palette_state: InlineShellCommandPaletteState::default(),
            startup_submit_armed: false,
            active_turn_id: None,
            active_turn_workspace_directory: None,
            active_turn_started_at: None,
            planning_repair_state: None,
            input_state: ConversationInputState::DraftReady,
            auto_follow_state: AutoFollowState::new(),
            planning_runtime_snapshot: PlanningRuntimeSnapshot::uninitialized(),
            turn_activity: TurnActivityState::default(),
            approval_review: None,
            turn_control_truth,
            last_auto_follow_activity: None,
            last_planning_task_handoff: None,
            last_applied_post_turn_evaluation_id: None,
            status_text: String::new(),
        };
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
        )
    }
    pub(crate) fn from_snapshot_with_truth(
        snapshot: ConversationSnapshot,
        draft_workspace_directory: String,
        turn_control_truth: ConversationRuntimeControlTruth,
    ) -> Self {
        // Snapshot warnings are preserved as the immutable baseline for this loaded thread.
        let base_warnings = snapshot.warnings;
        let runtime_notices = snapshot.runtime_notices;
        let warnings = base_warnings.clone();
        let base_status = "thread loaded".to_string();
        let mut view_model = Self {
            thread_id: snapshot.thread_id,
            title: snapshot.title,
            cwd: snapshot.cwd,
            draft_workspace_directory,
            messages: snapshot.messages,
            cached_conversation_lines: Vec::new(),
            live_agent_message: None,
            buffered_tool_messages: Vec::new(),
            base_warnings,
            warnings,
            runtime_notices,
            input_buffer: String::new(),
            inline_shell_command_palette_state: InlineShellCommandPaletteState::default(),
            startup_submit_armed: false,
            active_turn_id: None,
            active_turn_workspace_directory: None,
            active_turn_started_at: None,
            planning_repair_state: None,
            input_state: ConversationInputState::ReadyToContinue,
            auto_follow_state: AutoFollowState::new(),
            planning_runtime_snapshot: PlanningRuntimeSnapshot::uninitialized(),
            turn_activity: TurnActivityState::default(),
            approval_review: None,
            turn_control_truth,
            last_auto_follow_activity: None,
            last_planning_task_handoff: None,
            last_applied_post_turn_evaluation_id: None,
            status_text: String::new(),
        };
        view_model.set_status_with_warnings(base_status);
        view_model.refresh_conversation_lines();
        view_model
    }
    pub(crate) fn turn_control_truth(&self) -> ConversationRuntimeControlTruth {
        self.turn_control_truth
    }
    pub(crate) fn replace_planning_runtime_snapshot(
        &mut self,
        planning_runtime_snapshot: PlanningRuntimeSnapshot,
    ) {
        // The app polls planning state outside the conversation stream; this is the latest copy.
        self.planning_runtime_snapshot = planning_runtime_snapshot;
    }
    pub(crate) fn sync_inline_shell_command_palette(&mut self) {
        let preferred_selection = self.inline_shell_command_palette_state.selected_command();
        self.inline_shell_command_palette_state
            .sync_to_input(&self.input_buffer, preferred_selection);
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
        self.inline_shell_command_palette_state = InlineShellCommandPaletteState::default();
        self.status_text = status_text;
    }
    pub(crate) fn record_thread_prepared(&mut self, thread_id: String, title: String, cwd: String) {
        // Thread preparation upgrades a draft into an app-server backed conversation.
        self.thread_id = thread_id;
        self.title = title.clone();
        self.cwd = cwd;
        self.status_text = "thread started".to_string();
        self.append_status_message("thread opened / ".to_string() + &title);
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
        self.input_state = ConversationInputState::SubmittingTurn;
        self.active_turn_workspace_directory = Some(workspace_directory);
        self.active_turn_started_at = Some(Instant::now());
    }
    pub(crate) fn replace_active_turn_workspace_directory(&mut self, workspace_directory: String) {
        self.active_turn_workspace_directory = Some(workspace_directory);
    }
    pub(crate) fn mark_turn_started(&mut self, turn_id: String) {
        self.active_turn_id = Some(turn_id);
        self.input_state = ConversationInputState::StreamingTurn;
        // A recovered start may arrive without a prior submitting phase, so seed the timer here too.
        self.active_turn_started_at.get_or_insert_with(Instant::now);
        self.turn_activity.start_new_turn();
        self.approval_review = None;
        self.buffered_tool_messages.clear();
    }
    pub(crate) fn mark_turn_finished(&mut self) {
        self.active_turn_id = None;
        self.active_turn_workspace_directory = None;
        self.active_turn_started_at = None;
        self.input_state = self.ready_input_state();
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
            if !self.runtime_notices.contains(&notice) {
                self.runtime_notices.push(notice);
            }
        }
    }
    pub(crate) fn record_auto_follow_skip(&mut self, reason: AutoFollowSkipReason) {
        let detail = reason.detail(&self.auto_follow_state, &self.turn_activity);
        // A skip ends post-turn evaluation but keeps a readable activity record for the footer.
        self.auto_follow_state.clear_runtime_phase();
        self.auto_follow_state.clear_post_turn_continuation_pause();
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: reason.activity_summary().to_string(),
            detail,
        });
    }
    pub(crate) fn clear_auto_follow_skip(&mut self) {
        self.last_auto_follow_activity = None;
    }
    pub(crate) fn pause_post_turn_continuation(&mut self) {
        self.auto_follow_state.pause_post_turn_continuation();
    }
    pub(crate) fn record_internal_continuation_paused(&mut self) {
        self.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: "paused: internal continuation".to_string(),
            detail: "post-turn continuation is paused for this internal runtime cycle".to_string(),
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
    pub(crate) fn begin_auto_follow_evaluation(&mut self) {
        // Keep the phase idle when an evaluation would immediately skip; this avoids stale spinners.
        if self.auto_follow_state.post_turn_continuation_paused()
            || !self.auto_follow_state.can_queue_next()
            || self.latest_agent_message_text().is_none()
        {
            self.auto_follow_state.clear_runtime_phase();
            return;
        }

        self.auto_follow_state.begin_post_turn_evaluation();
        self.status_text = "turn completed / auto-follow evaluating next turn".to_string();
    }
    pub(crate) fn last_planning_task_handoff(&self) -> Option<&PlanningTaskHandoff> {
        self.last_planning_task_handoff.as_ref()
    }
    pub(crate) fn accepts_post_turn_evaluation(
        &self,
        thread_id: &str,
        completed_turn_id: &str,
    ) -> bool {
        // Async evaluators are accepted only for the most recently completed turn on this thread.
        self.thread_id == thread_id
            && !self.has_running_turn()
            && self.last_applied_post_turn_evaluation_id.as_deref() != Some(completed_turn_id)
            && self.turn_activity.last_completed_turn_id.as_deref() == Some(completed_turn_id)
    }
    pub(crate) fn record_post_turn_evaluation_applied(&mut self, completed_turn_id: &str) {
        self.last_applied_post_turn_evaluation_id = Some(completed_turn_id.to_string());
    }
    #[cfg(test)]
    pub(crate) fn decide_auto_follow(
        &self,
        planning_runtime: &PlanningRuntimeUseCases,
    ) -> AutoFollowDecision {
        self.decide_auto_follow_with_snapshot(planning_runtime, &self.planning_runtime_snapshot)
    }
    #[cfg(test)]
    pub(crate) fn decide_auto_follow_with_snapshot(
        &self,
        planning_runtime: &PlanningRuntimeUseCases,
        planning_runtime_snapshot: &PlanningRuntimeSnapshot,
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
            snapshot: planning_runtime_snapshot,
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

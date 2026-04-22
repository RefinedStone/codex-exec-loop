use std::time::Instant;

use ratatui::text::Line;

use crate::adapter::inbound::tui::conversation_text::{
    approval_review_status_text, approval_review_summary_text, control_support_label,
    runtime_control_summary_text,
};
use crate::application::service::planning::{
    PlanningAutoFollowBlockReason, PlanningRepairRequest, PlanningRuntimeAutoFollowDecision,
    PlanningRuntimeAutoFollowRequest, PlanningRuntimeSnapshot, PlanningRuntimeUseCases,
    PlanningTaskHandoff,
};
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationMessage, ConversationMessageKind,
    ConversationRuntimeControlTruth, ConversationSnapshot,
};

use super::super::inline_shell_commands::{InlineShellCommand, InlineShellCommandPaletteState};
use super::super::shell_presentation::format_conversation_lines;
use super::auto_follow::{AutoFollowState, AutoFollowupDecision, AutoFollowupSkipReason};
use super::turn_activity::TurnActivityState;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationInputState {
    DraftReady,
    ReadyToContinue,
    SubmittingTurn,
    StreamingTurn,
}

impl ConversationInputState {
    #[cfg(test)]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::DraftReady => "draft ready",
            Self::ReadyToContinue => "ready",
            Self::SubmittingTurn => "submitting",
            Self::StreamingTurn => "streaming",
        }
    }

    pub(crate) fn can_submit_now(self) -> bool {
        matches!(self, Self::DraftReady | Self::ReadyToContinue)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedAutoFollowupActivity {
    pub(crate) summary: String,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanningRepairState {
    pub(crate) root_turn_id: String,
    pub(crate) attempts_used: usize,
    pub(crate) max_attempts: usize,
    pub(crate) latest_request: PlanningRepairRequest,
}

#[derive(Debug, Clone)]
pub(crate) struct ConversationViewModel {
    pub(crate) thread_id: String,
    pub(crate) title: String,
    pub(crate) cwd: String,
    pub(crate) draft_workspace_directory: String,
    pub(crate) messages: Vec<ConversationMessage>,
    pub(crate) cached_conversation_lines: Vec<Line<'static>>,
    pub(crate) live_agent_message: Option<ConversationMessage>,
    pub(crate) buffered_tool_messages: Vec<ConversationMessage>,
    pub(crate) base_warnings: Vec<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) runtime_notices: Vec<String>,
    pub(crate) input_buffer: String,
    pub(crate) inline_shell_command_palette_state: InlineShellCommandPaletteState,
    pub(crate) startup_submit_armed: bool,
    pub(crate) active_turn_id: Option<String>,
    pub(crate) active_turn_workspace_directory: Option<String>,
    pub(crate) active_turn_started_at: Option<Instant>,
    pub(crate) planning_repair_state: Option<PlanningRepairState>,
    pub(crate) input_state: ConversationInputState,
    pub(crate) auto_follow_state: AutoFollowState,
    pub(crate) planning_runtime_snapshot: PlanningRuntimeSnapshot,
    pub(crate) turn_activity: TurnActivityState,
    pub(crate) approval_review: Option<ConversationApprovalReview>,
    pub(crate) turn_control_truth: ConversationRuntimeControlTruth,
    pub(crate) last_auto_followup_activity: Option<RecordedAutoFollowupActivity>,
    pub(crate) last_planning_task_handoff: Option<PlanningTaskHandoff>,
    pub(crate) status_text: String,
}

impl ConversationViewModel {
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
            last_auto_followup_activity: None,
            last_planning_task_handoff: None,
            status_text: String::new(),
        };
        view_model.set_status_with_warnings(base_status);
        view_model.refresh_conversation_lines();
        view_model
    }

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
            last_auto_followup_activity: None,
            last_planning_task_handoff: None,
            status_text: String::new(),
        };
        view_model.set_status_with_warnings(base_status);
        view_model.refresh_conversation_lines();
        view_model
    }

    fn compact_warning_text(warning: &str) -> String {
        let mut compact = String::with_capacity(warning.len());
        for segment in warning.split_whitespace() {
            if !compact.is_empty() {
                compact.push(' ');
            }
            compact.push_str(segment);
        }
        compact
    }

    fn truncate_warning_text(warning: &str, max_detail_len: usize) -> String {
        const TRUNCATION_SUFFIX: &str = "...";

        let compact = Self::compact_warning_text(warning);
        let max_detail_len = max_detail_len.max(TRUNCATION_SUFFIX.len());
        if compact.chars().count() <= max_detail_len {
            return compact;
        }

        let truncated = compact
            .chars()
            .take(max_detail_len - TRUNCATION_SUFFIX.len())
            .collect::<String>();
        format!("{truncated}{TRUNCATION_SUFFIX}")
    }

    fn selected_warning_for_summary(&self) -> Option<&str> {
        self.base_warnings.last().map(String::as_str)
    }

    fn warning_status_label(&self) -> Option<String> {
        let runtime_count = self.base_warnings.len();

        match runtime_count {
            0 => None,
            1 => Some("warning".to_string()),
            warning_count => Some(format!("warnings ({warning_count})")),
        }
    }

    pub(crate) fn warning_summary(&self, max_detail_len: usize) -> String {
        let Some(selected_warning) = self.selected_warning_for_summary() else {
            return "warning: none".to_string();
        };

        let summary = Self::truncate_warning_text(selected_warning, max_detail_len);
        match self.base_warnings.len() {
            0 => "warning: none".to_string(),
            1 => format!("warning: {summary}"),
            warning_count => format!("warnings ({warning_count}): {summary}"),
        }
    }

    pub(crate) fn runtime_notice_summary(&self, max_detail_len: usize) -> Option<String> {
        let selected_notice = self.runtime_notices.last()?;
        let summary = Self::truncate_warning_text(selected_notice, max_detail_len);
        Some(if self.runtime_notices.len() == 1 {
            format!("runtime: {summary}")
        } else {
            format!(
                "runtime notices ({}): {summary}",
                self.runtime_notices.len()
            )
        })
    }

    pub(crate) fn planning_notice_summary(&self, max_detail_len: usize) -> Option<String> {
        let planning_notices = self
            .runtime_notices
            .iter()
            .filter(|notice| notice.starts_with("planning "))
            .collect::<Vec<_>>();
        let selected_notice = planning_notices.last()?;
        let summary = Self::truncate_warning_text(selected_notice, max_detail_len);

        Some(if planning_notices.len() == 1 {
            format!("planning: {summary}")
        } else {
            format!("planning notices ({}): {summary}", planning_notices.len())
        })
    }

    pub(crate) fn approval_summary(&self) -> Option<String> {
        self.approval_review
            .as_ref()
            .map(approval_review_summary_text)
    }

    pub(crate) fn update_approval_review(&mut self, review: ConversationApprovalReview) {
        self.set_status_with_warnings(approval_review_status_text(
            &review,
            self.turn_control_truth.approval,
        ));
        self.approval_review = Some(review);
    }

    pub(crate) fn turn_control_truth(&self) -> ConversationRuntimeControlTruth {
        self.turn_control_truth
    }

    pub(crate) fn turn_control_summary(&self) -> String {
        runtime_control_summary_text(self.turn_control_truth)
    }

    pub(crate) fn interrupt_support_label(&self) -> &'static str {
        control_support_label(self.turn_control_truth.interrupt)
    }

    pub(crate) fn replace_planning_runtime_snapshot(
        &mut self,
        planning_runtime_snapshot: PlanningRuntimeSnapshot,
    ) {
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

    pub(crate) fn set_status_with_warnings(&mut self, base_status: String) {
        self.status_text = match self.warning_status_label() {
            Some(warning_label) => format!("{base_status} / {warning_label}"),
            None => base_status,
        };
    }

    pub(crate) fn refresh_conversation_lines(&mut self) {
        self.cached_conversation_lines = format_conversation_lines(&self.messages);
    }

    fn push_message(&mut self, message: ConversationMessage) {
        self.messages.push(message);
        self.refresh_conversation_lines();
    }

    fn push_messages<I>(&mut self, messages: I)
    where
        I: IntoIterator<Item = ConversationMessage>,
    {
        let mut changed = false;
        for message in messages {
            self.messages.push(message);
            changed = true;
        }

        if changed {
            self.refresh_conversation_lines();
        }
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
        if self.has_active_thread() || self.draft_workspace_directory == workspace_directory {
            return false;
        }

        self.draft_workspace_directory = workspace_directory.clone();
        self.cwd = workspace_directory;
        self.auto_follow_state = AutoFollowState::new();
        self.base_warnings.clear();
        self.warnings.clear();
        self.clear_auto_followup_skip();
        self.set_status_with_warnings("draft workspace synced".to_string());

        true
    }

    pub(crate) fn record_submitted_prompt(
        &mut self,
        transcript_message: ConversationMessage,
        workspace_directory: String,
        clear_input_buffer: bool,
    ) {
        self.push_message(transcript_message);
        if clear_input_buffer {
            self.input_buffer.clear();
            self.inline_shell_command_palette_state = InlineShellCommandPaletteState::default();
        }
        self.mark_turn_submitting(workspace_directory);
    }

    pub(crate) fn record_thread_prepared(&mut self, thread_id: String, title: String, cwd: String) {
        self.thread_id = thread_id;
        self.title = title.clone();
        self.cwd = cwd;
        self.status_text = "thread started".to_string();
        self.append_status_message("thread opened / ".to_string() + &title);
    }

    pub(crate) fn record_turn_started(&mut self, turn_id: String) {
        self.mark_turn_started(turn_id);
        self.live_agent_message = None;
        if let Some(turn_index) = self.auto_follow_state.mark_auto_turn_started() {
            let max_auto_turns = self.auto_follow_state.max_auto_turns_label();
            let status_text = format!(
                "auto follow-up running / turn {turn_index}/{max_auto_turns} / mode: {}",
                self.auto_follow_state.mode_label(),
            );
            self.status_text = status_text.clone();
            self.append_status_message(status_text);
        } else {
            self.status_text = "turn started".to_string();
            self.append_status_message("turn started");
        }
    }

    pub(crate) fn append_status_message(&mut self, text: impl Into<String>) -> bool {
        let text = text.into();
        if text.trim().is_empty() {
            return false;
        }

        if self.messages.last().is_some_and(|message| {
            message.kind == ConversationMessageKind::Status && message.text == text
        }) {
            return false;
        }

        self.push_message(ConversationMessage::new(
            ConversationMessageKind::Status,
            text,
            None,
            None,
        ));
        true
    }

    pub(crate) fn buffer_tool_message(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }

        self.buffered_tool_messages.push(ConversationMessage::new(
            ConversationMessageKind::Tool,
            text,
            None,
            None,
        ));
    }

    pub(crate) fn flush_buffered_tool_messages(&mut self) -> bool {
        if self.buffered_tool_messages.is_empty() {
            return false;
        }

        let buffered_messages = std::mem::take(&mut self.buffered_tool_messages);
        self.push_messages(buffered_messages);
        true
    }

    pub(crate) fn push_live_agent_delta(
        &mut self,
        item_id: String,
        phase: Option<String>,
        delta: String,
    ) {
        if let Some(message) = self.live_agent_message.as_mut()
            && message.item_id.as_deref() == Some(item_id.as_str())
        {
            message.text.push_str(&delta);
            if phase.is_some() {
                message.phase = phase;
            }
            return;
        }

        self.commit_live_agent_message();
        self.live_agent_message = Some(ConversationMessage::new(
            ConversationMessageKind::Agent,
            delta,
            phase,
            Some(item_id),
        ));
    }

    pub(crate) fn complete_live_agent_message(
        &mut self,
        item_id: String,
        phase: Option<String>,
        text: String,
    ) -> bool {
        if let Some(mut message) = self.live_agent_message.take() {
            if message.item_id.as_deref() == Some(item_id.as_str()) {
                message.text = text;
                message.phase = phase;
                self.push_message(message);
                return true;
            }

            self.push_message(message);
        }

        if let Some(message) = self
            .messages
            .iter_mut()
            .rev()
            .find(|message| message.item_id.as_deref() == Some(item_id.as_str()))
        {
            message.text = text;
            message.phase = phase;
            self.refresh_conversation_lines();
            return true;
        }

        self.push_message(ConversationMessage::new(
            ConversationMessageKind::Agent,
            text,
            phase,
            Some(item_id),
        ));
        true
    }

    pub(crate) fn commit_live_agent_message(&mut self) -> bool {
        let Some(message) = self.live_agent_message.take() else {
            return false;
        };

        self.push_message(message);
        true
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

    pub(crate) fn can_submit_prompt(&self) -> bool {
        self.input_state.can_submit_now()
    }

    pub(crate) fn has_running_turn(&self) -> bool {
        !self.can_submit_prompt()
    }

    pub(crate) fn live_activity_started_at(&self) -> Option<Instant> {
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
        for notice in notices {
            if !self.runtime_notices.contains(&notice) {
                self.runtime_notices.push(notice);
            }
        }
    }

    pub(crate) fn record_auto_followup_skip(&mut self, reason: AutoFollowupSkipReason) {
        let detail = reason.detail(&self.auto_follow_state, &self.turn_activity);
        self.auto_follow_state.clear_runtime_phase();
        self.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
            summary: reason.activity_summary().to_string(),
            detail,
        });
    }

    pub(crate) fn clear_auto_followup_skip(&mut self) {
        self.last_auto_followup_activity = None;
    }

    pub(crate) fn record_automation_stopped(&mut self) {
        self.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
            summary: "stopped: automation off".to_string(),
            detail: "post-turn automation is off; toggle Ctrl+a to re-enable it".to_string(),
        });
    }

    pub(crate) fn clear_last_planning_task_handoff(&mut self) {
        self.last_planning_task_handoff = None;
    }

    pub(crate) fn record_auto_followup_submission(
        &mut self,
        _queued_from_turn_id: &str,
        handoff_task: Option<&PlanningTaskHandoff>,
    ) {
        let turn_index = self.auto_follow_state.mark_auto_turn_submitted();
        let progress = format!(
            "{turn_index}/{}",
            self.auto_follow_state.max_auto_turns_label()
        );
        self.last_planning_task_handoff = handoff_task.cloned();
        self.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
            summary: format!("submitted auto turn {progress}"),
            detail: "queued after the previous turn completed; submitted planning auto follow-up"
                .to_string(),
        });
    }

    pub(crate) fn record_auto_followup_queue(&mut self, _queued_from_turn_id: &str) {
        let turn_index = self.auto_follow_state.mark_auto_turn_queued();
        let next_progress = format!(
            "{turn_index}/{}",
            self.auto_follow_state.max_auto_turns_label()
        );
        self.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
            summary: format!("queued auto turn {next_progress}"),
            detail:
                "queued after the previous turn completed; waiting to submit planning auto follow-up"
                    .to_string(),
        });
    }

    pub(crate) fn begin_auto_followup_evaluation(&mut self) {
        if !self.auto_follow_state.enabled
            || !self.auto_follow_state.can_queue_next()
            || self.latest_agent_message_text().is_none()
        {
            self.auto_follow_state.clear_runtime_phase();
            return;
        }

        self.auto_follow_state.begin_post_turn_evaluation();
        self.status_text = "turn completed / auto follow-up evaluating next turn".to_string();
    }

    pub(crate) fn last_planning_task_handoff(&self) -> Option<&PlanningTaskHandoff> {
        self.last_planning_task_handoff.as_ref()
    }

    pub(crate) fn accepts_post_turn_evaluation(
        &self,
        thread_id: &str,
        queued_from_turn_id: &str,
    ) -> bool {
        self.thread_id == thread_id
            && !self.has_running_turn()
            && self.turn_activity.last_completed_turn_id.as_deref() == Some(queued_from_turn_id)
    }

    pub(crate) fn latest_agent_message_text(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|message| {
                message.kind == ConversationMessageKind::Agent && !message.text.trim().is_empty()
            })
            .map(|message| message.text.as_str())
    }

    pub(crate) fn latest_user_message_text(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|message| {
                message.kind == ConversationMessageKind::User && !message.text.trim().is_empty()
            })
            .map(|message| message.text.as_str())
    }

    #[cfg(test)]
    pub(crate) fn decide_auto_followup(
        &self,
        planning_runtime: &PlanningRuntimeUseCases,
    ) -> AutoFollowupDecision {
        self.decide_auto_followup_with_snapshot(planning_runtime, &self.planning_runtime_snapshot)
    }

    pub(crate) fn decide_auto_followup_with_snapshot(
        &self,
        planning_runtime: &PlanningRuntimeUseCases,
        planning_runtime_snapshot: &PlanningRuntimeSnapshot,
    ) -> AutoFollowupDecision {
        if !self.auto_follow_state.enabled {
            return AutoFollowupDecision::Skip(AutoFollowupSkipReason::Disabled);
        }

        if !self.auto_follow_state.can_queue_next() {
            return AutoFollowupDecision::Skip(AutoFollowupSkipReason::LimitReached);
        }

        let Some(last_message) = self.latest_agent_message_text() else {
            return AutoFollowupDecision::Skip(AutoFollowupSkipReason::NoAgentReply);
        };

        if self
            .auto_follow_state
            .stop_rules
            .stop_keyword
            .matches(last_message)
        {
            return AutoFollowupDecision::Skip(AutoFollowupSkipReason::StopKeywordMatched);
        }

        if self
            .auto_follow_state
            .stop_rules
            .should_stop_on_no_file_changes(self.turn_activity.last_completed_file_change_count())
        {
            return AutoFollowupDecision::Skip(AutoFollowupSkipReason::NoFileChanges);
        }

        match planning_runtime.decide_auto_followup(PlanningRuntimeAutoFollowRequest {
            stop_keyword: self.auto_follow_state.stop_keyword_value(),
            last_message: last_message.trim(),
            snapshot: planning_runtime_snapshot,
        }) {
            PlanningRuntimeAutoFollowDecision::QueuePrompt(prompt) => {
                AutoFollowupDecision::QueuePrompt(prompt)
            }
            PlanningRuntimeAutoFollowDecision::Blocked(block_reason) => {
                AutoFollowupDecision::Skip(match block_reason {
                    PlanningAutoFollowBlockReason::PlanningDisabled => {
                        AutoFollowupSkipReason::PlanningDisabled
                    }
                    PlanningAutoFollowBlockReason::InvalidWorkspace => {
                        AutoFollowupSkipReason::PlanningBlocked
                    }
                    PlanningAutoFollowBlockReason::ActionableQueueRequired => {
                        AutoFollowupSkipReason::PlanningQueueHeadRequired
                    }
                    PlanningAutoFollowBlockReason::RepeatedQueueHead => {
                        AutoFollowupSkipReason::PlanningRepeatedQueueHead
                    }
                })
            }
        }
    }
}

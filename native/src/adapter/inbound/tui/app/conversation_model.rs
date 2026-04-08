use ratatui::text::Line;

use crate::domain::conversation::{
    ConversationApprovalReview, ConversationMessage, ConversationMessageKind, ConversationSnapshot,
    ConversationToolActivity, ConversationToolActivityKind,
};
use crate::domain::followup_template::{
    FollowupTemplateCatalog, FollowupTemplateCatalogLoadResult, FollowupTemplateDefinition,
};

use super::{
    DEFAULT_AUTO_FOLLOW_MAX_TURNS, DEFAULT_AUTO_FOLLOW_STOP_KEYWORD, MAX_AUTO_FOLLOW_MAX_TURNS,
    format_conversation_lines,
};

#[derive(Debug, Clone)]
pub(crate) enum ConversationState {
    Loading,
    Ready(ConversationViewModel),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationInputState {
    DraftReady,
    ReadyToContinue,
    SubmittingTurn,
    StreamingTurn,
}

impl ConversationInputState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::DraftReady => "draft ready",
            Self::ReadyToContinue => "ready",
            Self::SubmittingTurn => "submitting",
            Self::StreamingTurn => "streaming",
        }
    }

    pub(crate) fn detail(self) -> &'static str {
        match self {
            Self::DraftReady => "first prompt will create a new thread",
            Self::ReadyToContinue => "session is ready for the next prompt",
            Self::SubmittingTurn => "sending prompt to codex app-server",
            Self::StreamingTurn => "current turn is still running",
        }
    }

    pub(crate) fn can_submit_now(self) -> bool {
        matches!(self, Self::DraftReady | Self::ReadyToContinue)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutoFollowupDecision {
    QueuePrompt(String),
    Skip(AutoFollowupSkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoFollowupSkipReason {
    Disabled,
    ManualInputBuffered,
    LimitReached,
    NoAgentReply,
    StopKeywordMatched,
    NoFileChanges,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedAutoFollowupActivity {
    pub(crate) summary: String,
    pub(crate) detail: String,
}

impl AutoFollowupSkipReason {
    fn detail(
        self,
        auto_follow_state: &AutoFollowState,
        turn_activity: &TurnActivityState,
    ) -> String {
        match self {
            Self::Disabled => "auto follow-up is off; toggle Ctrl+a to re-enable it".to_string(),
            Self::ManualInputBuffered => {
                "the input panel already has a manual prompt buffered".to_string()
            }
            Self::LimitReached => format!(
                "reached the configured auto-turn budget ({})",
                auto_follow_state.progress_label()
            ),
            Self::NoAgentReply => {
                "a non-empty agent reply is required before the next auto turn can be queued"
                    .to_string()
            }
            Self::StopKeywordMatched => format!(
                "the latest agent reply matched the stop keyword {}",
                auto_follow_state.stop_rules.stop_keyword.value()
            ),
            Self::NoFileChanges => format!(
                "the last completed turn changed {} files while the no-file stop rule is on",
                turn_activity.last_completed_file_change_count()
            ),
        }
    }

    pub(crate) fn activity_summary(self) -> &'static str {
        match self {
            Self::Disabled => "stopped: auto follow-up off",
            Self::ManualInputBuffered => "skipped: manual input buffered",
            Self::LimitReached => "stopped: turn limit reached",
            Self::NoAgentReply => "skipped: no agent reply",
            Self::StopKeywordMatched => "stopped: stop keyword matched",
            Self::NoFileChanges => "stopped: no file changes",
        }
    }

    pub(crate) fn runtime_status(
        self,
        turn_id: &str,
        auto_follow_state: &AutoFollowState,
    ) -> String {
        match self {
            Self::Disabled => format!("turn completed: {turn_id} / auto follow-up stopped: off"),
            Self::ManualInputBuffered => {
                format!("turn completed: {turn_id} / auto follow-up skipped: manual input buffered")
            }
            Self::LimitReached => format!(
                "turn completed: {turn_id} / auto follow-up stopped: turn limit reached ({})",
                auto_follow_state.progress_label()
            ),
            Self::NoAgentReply => {
                format!("turn completed: {turn_id} / auto follow-up skipped: no agent reply")
            }
            Self::StopKeywordMatched => format!(
                "turn completed: {turn_id} / auto follow-up stopped: stop keyword matched ({})",
                auto_follow_state.stop_rules.stop_keyword.value()
            ),
            Self::NoFileChanges => {
                format!("turn completed: {turn_id} / auto follow-up stopped: no file changes")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AutoFollowState {
    pub(crate) enabled: bool,
    pub(crate) completed_auto_turns: usize,
    pub(crate) max_auto_turns: usize,
    pub(crate) template_state: AutoFollowTemplateState,
    pub(crate) stop_rules: AutoFollowStopRules,
}

#[derive(Debug, Clone)]
pub(crate) struct AutoFollowStopRules {
    pub(crate) stop_keyword: StopKeywordRule,
    pub(crate) stop_on_no_file_changes: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct StopKeywordRule {
    pub(crate) enabled: bool,
    pub(crate) value: String,
}

#[derive(Debug, Clone)]
pub(crate) struct AutoFollowTemplateState {
    pub(crate) items: Vec<FollowupTemplateDefinition>,
    pub(crate) selected_index: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TurnActivityState {
    pub(crate) current_turn_file_change_count: usize,
    pub(crate) current_turn_command_count: usize,
    pub(crate) current_turn_last_summary: Option<String>,
    pub(crate) last_completed_turn_file_change_count: usize,
    pub(crate) last_completed_turn_command_count: usize,
    pub(crate) last_completed_turn_last_summary: Option<String>,
}

impl AutoFollowState {
    pub(crate) fn new(template_catalog: FollowupTemplateCatalog) -> Self {
        Self {
            enabled: true,
            completed_auto_turns: 0,
            max_auto_turns: DEFAULT_AUTO_FOLLOW_MAX_TURNS,
            template_state: AutoFollowTemplateState::new(template_catalog),
            stop_rules: AutoFollowStopRules::default(),
        }
    }
}

impl Default for AutoFollowStopRules {
    fn default() -> Self {
        Self {
            stop_keyword: StopKeywordRule::default(),
            stop_on_no_file_changes: false,
        }
    }
}

impl Default for StopKeywordRule {
    fn default() -> Self {
        Self {
            enabled: true,
            value: DEFAULT_AUTO_FOLLOW_STOP_KEYWORD.to_string(),
        }
    }
}

impl AutoFollowState {
    pub(crate) fn status_label(&self) -> &'static str {
        if self.enabled { "on" } else { "off" }
    }

    pub(crate) fn progress_label(&self) -> String {
        format!("{}/{}", self.completed_auto_turns, self.max_auto_turns)
    }

    pub(crate) fn max_auto_turns_value(&self) -> usize {
        self.max_auto_turns
    }

    pub(crate) fn template_label(&self) -> &str {
        self.template_state.current().label.as_str()
    }

    pub(crate) fn selected_template(&self) -> &FollowupTemplateDefinition {
        self.template_state.current()
    }

    pub(crate) fn selected_template_index(&self) -> usize {
        self.template_state.selected_index
    }

    pub(crate) fn template_source_label(&self) -> String {
        self.template_state.current().source_label()
    }

    pub(crate) fn template_count(&self) -> usize {
        self.template_state.items.len()
    }

    pub(crate) fn stop_keyword_label(&self) -> String {
        self.stop_rules.stop_keyword.label()
    }

    pub(crate) fn stop_keyword_value(&self) -> &str {
        self.stop_rules.stop_keyword.value()
    }

    pub(crate) fn no_file_change_stop_label(&self) -> &'static str {
        self.stop_rules.no_file_change_label()
    }

    pub(crate) fn next_auto_turn_index(&self) -> usize {
        self.completed_auto_turns + 1
    }

    pub(crate) fn can_queue_next(&self) -> bool {
        self.enabled && self.completed_auto_turns < self.max_auto_turns
    }

    pub(crate) fn reset_for_manual_turn(&mut self) {
        self.completed_auto_turns = 0;
    }

    pub(crate) fn mark_auto_turn_submitted(&mut self) {
        self.completed_auto_turns += 1;
    }

    pub(crate) fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    pub(crate) fn set_max_auto_turns(&mut self, value: usize) {
        self.max_auto_turns = value;
    }

    pub(crate) fn toggle_stop_keyword(&mut self) {
        self.stop_rules.stop_keyword.toggle();
    }

    pub(crate) fn set_stop_keyword_value(&mut self, value: String) {
        self.stop_rules.stop_keyword.set_value(value);
    }

    pub(crate) fn toggle_no_file_change_stop(&mut self) {
        self.stop_rules.stop_on_no_file_changes = !self.stop_rules.stop_on_no_file_changes;
    }

    pub(crate) fn reload_template_catalog(
        &mut self,
        template_catalog: FollowupTemplateCatalog,
    ) -> bool {
        self.template_state.reload_catalog(template_catalog)
    }

    pub(crate) fn cycle_template_kind(&mut self) {
        self.template_state.cycle();
    }

    pub(crate) fn cycle_template_kind_backward(&mut self) {
        self.template_state.cycle_previous();
    }

    pub(crate) fn render_prompt(&self, thread_id: &str, last_message: &str) -> String {
        self.template_state
            .current()
            .body
            .as_str()
            .replace("{auto_turn}", &self.next_auto_turn_index().to_string())
            .replace("{max_auto_turns}", &self.max_auto_turns.to_string())
            .replace("{session_id}", thread_id)
            .replace("{stop_keyword}", self.stop_rules.stop_keyword.value())
            .replace("{last_message}", last_message)
    }

    pub(crate) fn render_prompt_preview(
        &self,
        thread_id: &str,
        last_message: Option<&str>,
    ) -> String {
        let preview_thread_id = if thread_id.trim().is_empty() {
            "draft-thread"
        } else {
            thread_id
        };
        let preview_last_message = last_message
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("(waiting for next agent reply)");
        self.render_prompt(preview_thread_id, preview_last_message)
    }

    pub(crate) fn normalize_max_auto_turns_candidate(candidate: &str) -> Option<usize> {
        let normalized = candidate.trim();
        let value = normalized.parse::<usize>().ok()?;
        if value == 0 || value > MAX_AUTO_FOLLOW_MAX_TURNS {
            None
        } else {
            Some(value)
        }
    }
}

impl AutoFollowStopRules {
    pub(crate) fn should_stop_on_no_file_changes(&self, file_change_count: usize) -> bool {
        self.stop_on_no_file_changes && file_change_count == 0
    }

    pub(crate) fn no_file_change_label(&self) -> &'static str {
        if self.stop_on_no_file_changes {
            "on"
        } else {
            "off"
        }
    }
}

impl StopKeywordRule {
    pub(crate) fn normalize_candidate(candidate: &str) -> Option<String> {
        let normalized = candidate.trim();
        if normalized.is_empty()
            || !normalized
                .chars()
                .all(|character| character.is_alphanumeric() || character == '_')
        {
            None
        } else {
            Some(normalized.to_string())
        }
    }

    pub(crate) fn label(&self) -> String {
        if self.enabled {
            format!("on ({})", self.value)
        } else {
            format!("off ({})", self.value)
        }
    }

    pub(crate) fn matches(&self, text: &str) -> bool {
        self.enabled
            && text.split_whitespace().any(|token| {
                token
                    .trim_matches(|character: char| {
                        !character.is_alphanumeric() && character != '_'
                    })
                    .eq_ignore_ascii_case(&self.value)
            })
    }

    pub(crate) fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    pub(crate) fn set_value(&mut self, value: String) {
        self.value = value;
    }

    pub(crate) fn value(&self) -> &str {
        self.value.as_str()
    }
}

impl AutoFollowTemplateState {
    pub(crate) fn new(template_catalog: FollowupTemplateCatalog) -> Self {
        Self {
            items: template_catalog.items,
            selected_index: 0,
        }
    }

    pub(crate) fn current(&self) -> &FollowupTemplateDefinition {
        self.items
            .get(self.selected_index)
            .expect("follow-up template catalog should not be empty")
    }

    pub(crate) fn cycle(&mut self) {
        if self.items.len() <= 1 {
            return;
        }

        self.selected_index = (self.selected_index + 1) % self.items.len();
    }

    pub(crate) fn cycle_previous(&mut self) {
        if self.items.len() <= 1 {
            return;
        }

        if self.selected_index == 0 {
            self.selected_index = self.items.len() - 1;
        } else {
            self.selected_index -= 1;
        }
    }

    pub(crate) fn reload_catalog(&mut self, template_catalog: FollowupTemplateCatalog) -> bool {
        let selected_template_id = self.current().id.clone();
        self.items = template_catalog.items;
        self.selected_index = self
            .items
            .iter()
            .position(|template| template.id == selected_template_id)
            .unwrap_or(0);

        self.current().id != selected_template_id
    }
}

impl TurnActivityState {
    pub(crate) fn start_new_turn(&mut self) {
        self.current_turn_file_change_count = 0;
        self.current_turn_command_count = 0;
        self.current_turn_last_summary = None;
    }

    pub(crate) fn register_tool_activity(&mut self, activity: &ConversationToolActivity) {
        self.current_turn_last_summary = Some(activity.text.clone());
        match activity.kind {
            ConversationToolActivityKind::FileChange => {
                self.current_turn_file_change_count += activity.file_change_count;
            }
            ConversationToolActivityKind::CommandExecution => {
                self.current_turn_command_count += 1;
            }
        }
    }

    pub(crate) fn complete_turn(&mut self) {
        self.last_completed_turn_file_change_count =
            std::mem::replace(&mut self.current_turn_file_change_count, 0);
        self.last_completed_turn_command_count =
            std::mem::replace(&mut self.current_turn_command_count, 0);
        self.last_completed_turn_last_summary = self.current_turn_last_summary.take();
    }

    pub(crate) fn last_completed_file_change_count(&self) -> usize {
        self.last_completed_turn_file_change_count
    }

    fn has_current_turn_activity(&self) -> bool {
        self.current_turn_file_change_count > 0
            || self.current_turn_command_count > 0
            || self.current_turn_last_summary.is_some()
    }

    pub(crate) fn activity_scope_label(&self, turn_running: bool) -> &'static str {
        if turn_running {
            "current turn"
        } else if self.has_current_turn_activity() {
            "recent turn"
        } else {
            "last turn"
        }
    }

    pub(crate) fn activity_command_count(&self, turn_running: bool) -> usize {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_command_count
        } else {
            self.last_completed_turn_command_count
        }
    }

    pub(crate) fn activity_file_change_count(&self, turn_running: bool) -> usize {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_file_change_count
        } else {
            self.last_completed_turn_file_change_count
        }
    }

    pub(crate) fn activity_summary(&self, turn_running: bool) -> &str {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_last_summary.as_deref().unwrap_or("none")
        } else {
            self.last_completed_turn_last_summary
                .as_deref()
                .unwrap_or("none")
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ConversationViewModel {
    pub(crate) thread_id: String,
    pub(crate) title: String,
    pub(crate) cwd: String,
    pub(crate) messages: Vec<ConversationMessage>,
    pub(crate) cached_conversation_lines: Vec<Line<'static>>,
    pub(crate) base_warnings: Vec<String>,
    pub(crate) template_warnings: Vec<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) runtime_notices: Vec<String>,
    pub(crate) input_buffer: String,
    pub(crate) startup_submit_armed: bool,
    pub(crate) active_turn_id: Option<String>,
    pub(crate) input_state: ConversationInputState,
    pub(crate) auto_follow_state: AutoFollowState,
    pub(crate) turn_activity: TurnActivityState,
    pub(crate) approval_review: Option<ConversationApprovalReview>,
    pub(crate) last_auto_followup_activity: Option<RecordedAutoFollowupActivity>,
    pub(crate) status_text: String,
}

impl ConversationViewModel {
    pub(crate) fn new_draft(
        cwd: String,
        template_load_result: FollowupTemplateCatalogLoadResult,
    ) -> Self {
        let base_status = format!(
            "new thread draft / templates: {}",
            template_load_result.catalog.items.len()
        );
        let mut view_model = Self {
            thread_id: String::new(),
            title: "New conversation".to_string(),
            cwd,
            messages: Vec::new(),
            cached_conversation_lines: Vec::new(),
            base_warnings: Vec::new(),
            template_warnings: template_load_result.warnings.clone(),
            warnings: template_load_result.warnings,
            runtime_notices: Vec::new(),
            input_buffer: String::new(),
            startup_submit_armed: false,
            active_turn_id: None,
            input_state: ConversationInputState::DraftReady,
            auto_follow_state: AutoFollowState::new(template_load_result.catalog),
            turn_activity: TurnActivityState::default(),
            approval_review: None,
            last_auto_followup_activity: None,
            status_text: String::new(),
        };
        view_model.set_status_with_warnings(base_status);
        view_model.refresh_conversation_lines();
        view_model
    }

    pub(crate) fn from_snapshot(
        snapshot: ConversationSnapshot,
        template_load_result: FollowupTemplateCatalogLoadResult,
    ) -> Self {
        let base_warnings = snapshot.warnings;
        let runtime_notices = snapshot.runtime_notices;
        let template_warnings = template_load_result.warnings;
        let warnings = Self::merge_warnings(&base_warnings, &template_warnings);
        let base_status = format!(
            "thread loaded / templates: {}",
            template_load_result.catalog.items.len()
        );

        let mut view_model = Self {
            thread_id: snapshot.thread_id,
            title: snapshot.title,
            cwd: snapshot.cwd,
            messages: snapshot.messages,
            cached_conversation_lines: Vec::new(),
            base_warnings,
            template_warnings,
            warnings,
            runtime_notices,
            input_buffer: String::new(),
            startup_submit_armed: false,
            active_turn_id: None,
            input_state: ConversationInputState::ReadyToContinue,
            auto_follow_state: AutoFollowState::new(template_load_result.catalog),
            turn_activity: TurnActivityState::default(),
            approval_review: None,
            last_auto_followup_activity: None,
            status_text: String::new(),
        };
        view_model.set_status_with_warnings(base_status);
        view_model.refresh_conversation_lines();
        view_model
    }

    fn merge_warnings(base_warnings: &[String], template_warnings: &[String]) -> Vec<String> {
        let mut warnings = base_warnings.to_vec();
        warnings.extend(template_warnings.iter().cloned());
        warnings
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
        self.base_warnings
            .last()
            .map(String::as_str)
            .or_else(|| self.template_warnings.last().map(String::as_str))
    }

    fn warning_status_label(&self) -> Option<String> {
        let runtime_count = self.base_warnings.len();
        let template_count = self.template_warnings.len();

        match (runtime_count, template_count, self.warnings.len()) {
            (_, _, 0) => None,
            (0, 0, 1) => Some("warning".to_string()),
            (0, 0, warning_count) => Some(format!("warnings ({warning_count})")),
            (1, 0, _) => Some("runtime warning".to_string()),
            (runtime_count, 0, _) => Some(format!("runtime warnings ({runtime_count})")),
            (0, 1, _) => Some("template warning".to_string()),
            (0, template_count, _) => Some(format!("template warnings ({template_count})")),
            (runtime_count, template_count, _) => Some(format!(
                "warnings: runtime {runtime_count}, template {template_count}"
            )),
        }
    }

    // Warning order is normalized differently across sources, so this surfaces
    // a compact warning summary without claiming chronology and prefers
    // shared-runtime warnings over template warnings when both exist.
    pub(crate) fn warning_summary(&self, max_detail_len: usize) -> String {
        let Some(selected_warning) = self.selected_warning_for_summary() else {
            return "warning: none".to_string();
        };

        let summary = Self::truncate_warning_text(selected_warning, max_detail_len);
        let runtime_count = self.base_warnings.len();
        let template_count = self.template_warnings.len();

        match (runtime_count, template_count, self.warnings.len()) {
            (0, 0, 1) => format!("warning: {summary}"),
            (0, 0, warning_count) => format!("warnings ({warning_count}): {summary}"),
            (1, 0, _) => format!("runtime warning: {summary}"),
            (runtime_count, 0, _) => format!("runtime warnings ({runtime_count}): {summary}"),
            (0, 1, _) => format!("template warning: {summary}"),
            (0, template_count, _) => {
                format!("template warnings ({template_count}): {summary}")
            }
            (runtime_count, template_count, _) => {
                format!("warnings: runtime {runtime_count}, template {template_count} / {summary}")
            }
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

    pub(crate) fn approval_summary(&self) -> Option<String> {
        self.approval_review
            .as_ref()
            .map(ConversationApprovalReview::summary_text)
    }

    pub(crate) fn update_approval_review(&mut self, review: ConversationApprovalReview) {
        self.set_status_with_warnings(review.status_text());
        self.approval_review = Some(review);
    }

    pub(crate) fn replace_template_warnings(&mut self, template_warnings: Vec<String>) {
        self.template_warnings = template_warnings;
        self.warnings = Self::merge_warnings(&self.base_warnings, &self.template_warnings);
    }

    pub(crate) fn set_status_with_warnings(&mut self, base_status: String) {
        self.status_text = match self.warning_status_label() {
            Some(warning_label) => format!("{base_status} / {warning_label}"),
            None => base_status,
        };
    }

    pub(crate) fn reload_followup_templates(
        &mut self,
        template_load_result: FollowupTemplateCatalogLoadResult,
    ) -> bool {
        let template_count = template_load_result.catalog.items.len();
        let selection_changed = self
            .auto_follow_state
            .reload_template_catalog(template_load_result.catalog);
        self.replace_template_warnings(template_load_result.warnings);
        self.clear_auto_followup_skip();
        let selection_label = self.auto_follow_state.template_label();
        let selection_msg = if selection_changed {
            format!("selected template reset to {selection_label}")
        } else {
            format!("selected: {selection_label}")
        };
        let base_status =
            format!("follow-up templates reloaded / {selection_msg} / templates: {template_count}");
        self.set_status_with_warnings(base_status);

        selection_changed
    }

    pub(crate) fn refresh_conversation_lines(&mut self) {
        self.cached_conversation_lines = format_conversation_lines(&self.messages);
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

    pub(crate) fn arm_startup_submit(&mut self) {
        self.startup_submit_armed = true;
    }

    pub(crate) fn clear_startup_submit(&mut self) -> bool {
        std::mem::replace(&mut self.startup_submit_armed, false)
    }

    pub(crate) fn mark_turn_submitting(&mut self) {
        self.startup_submit_armed = false;
        self.input_state = ConversationInputState::SubmittingTurn;
    }

    pub(crate) fn mark_turn_started(&mut self, turn_id: String) {
        self.active_turn_id = Some(turn_id);
        self.input_state = ConversationInputState::StreamingTurn;
        self.turn_activity.start_new_turn();
        self.approval_review = None;
    }

    pub(crate) fn mark_turn_finished(&mut self) {
        self.active_turn_id = None;
        self.input_state = self.ready_input_state();
    }

    pub(crate) fn record_auto_followup_skip(&mut self, reason: AutoFollowupSkipReason) {
        let detail = reason.detail(&self.auto_follow_state, &self.turn_activity);
        self.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
            summary: reason.activity_summary().to_string(),
            detail,
        });
    }

    pub(crate) fn clear_auto_followup_skip(&mut self) {
        self.last_auto_followup_activity = None;
    }

    pub(crate) fn record_auto_followup_submission(
        &mut self,
        queued_from_turn_id: &str,
        template_label: &str,
    ) {
        let progress = self.auto_follow_state.progress_label();
        self.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
            summary: format!("submitted auto turn {progress}"),
            detail: format!(
                "queued after turn {queued_from_turn_id} completed; submitted with template {template_label}"
            ),
        });
    }

    pub(crate) fn record_auto_followup_queue(
        &mut self,
        queued_from_turn_id: &str,
        template_label: &str,
    ) {
        let next_progress = format!(
            "{}/{}",
            self.auto_follow_state.next_auto_turn_index(),
            self.auto_follow_state.max_auto_turns_value()
        );
        self.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
            summary: format!("queued auto turn {next_progress}"),
            detail: format!(
                "queued after turn {queued_from_turn_id} completed; waiting to submit with template {template_label}"
            ),
        });
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

    pub(crate) fn decide_auto_followup(&self) -> AutoFollowupDecision {
        if !self.auto_follow_state.enabled {
            return AutoFollowupDecision::Skip(AutoFollowupSkipReason::Disabled);
        }

        if !self.input_buffer.trim().is_empty() {
            return AutoFollowupDecision::Skip(AutoFollowupSkipReason::ManualInputBuffered);
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

        AutoFollowupDecision::QueuePrompt(
            self.auto_follow_state
                .render_prompt(&self.thread_id, last_message.trim()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AutoFollowState, AutoFollowupDecision, AutoFollowupSkipReason, ConversationInputState,
        ConversationMessage, ConversationMessageKind, ConversationViewModel, StopKeywordRule,
        TurnActivityState, format_conversation_lines,
    };
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
            messages: Vec::new(),
            cached_conversation_lines: format_conversation_lines(&[]),
            base_warnings: Vec::new(),
            template_warnings: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            input_buffer: String::new(),
            startup_submit_armed: false,
            active_turn_id: None,
            input_state: ConversationInputState::ReadyToContinue,
            auto_follow_state: AutoFollowState::new(sample_template_catalog()),
            turn_activity: TurnActivityState::default(),
            approval_review: None,
            last_auto_followup_activity: None,
            status_text: "thread loaded".to_string(),
        }
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

        let AutoFollowupDecision::QueuePrompt(prompt) = conversation.decide_auto_followup() else {
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
                runtime_notices: vec![
                    "shared runtime reset after startup checks failure".to_string(),
                ],
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
            conversation.decide_auto_followup(),
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

        let AutoFollowupDecision::QueuePrompt(prompt) = conversation.decide_auto_followup() else {
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
    fn render_prompt_preview_uses_placeholders_for_blank_thread_and_reply() {
        let mut state = AutoFollowState::new(sample_template_catalog());
        state.template_state.items[0].body =
            "session={session_id}\nlast={last_message}".to_string();

        let preview = state.render_prompt_preview("", Some("   "));

        assert!(preview.contains("session=draft-thread"));
        assert!(preview.contains("(waiting for next agent reply)"));
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
            conversation.decide_auto_followup(),
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
            conversation.decide_auto_followup(),
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
            conversation.decide_auto_followup(),
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
            conversation.decide_auto_followup(),
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

        let AutoFollowupDecision::QueuePrompt(prompt) = conversation.decide_auto_followup() else {
            panic!("auto follow-up should continue when file changes exist");
        };

        assert!(prompt.contains("latest answer"));
    }
}

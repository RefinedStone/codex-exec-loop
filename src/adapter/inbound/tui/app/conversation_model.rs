use std::time::Instant;

use ratatui::text::Line;

use super::{
    DEFAULT_AUTO_FOLLOW_MAX_TURNS, DEFAULT_AUTO_FOLLOW_STOP_KEYWORD, MAX_AUTO_FOLLOW_MAX_TURNS,
    format_conversation_lines,
};
use crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot;
use crate::application::service::planning_reconciliation_service::PlanningRepairRequest;
use crate::application::service::planning_runtime_facade_service::{
    PlanningRuntimeAutoFollowDecision, PlanningRuntimeAutoFollowRequest,
    PlanningRuntimeFacadeService, PlanningRuntimeQueuedAutoFollowPrompt, PlanningTaskHandoff,
};
use crate::application::service::planning_runtime_policy_service::PlanningAutoFollowBlockReason;
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationMessage, ConversationMessageKind, ConversationSnapshot,
    ConversationToolActivity, ConversationToolActivityKind,
};
use crate::domain::followup_template::{
    FollowupTemplateCatalog, FollowupTemplateCatalogLoadResult, FollowupTemplateDefinition,
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
pub(crate) enum AutoFollowupDecision {
    QueuePrompt(PlanningRuntimeQueuedAutoFollowPrompt),
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
    PlanningBlocked,
    PlanningQueueHeadRequired,
    PlanningRepeatedQueueHead,
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
            Self::PlanningBlocked => {
                "planning files are invalid or incomplete; auto follow-up stays paused until they validate"
                    .to_string()
            }
            Self::PlanningQueueHeadRequired => {
                "the selected auto follow-up template requires an actionable planning queue head"
                    .to_string()
            }
            Self::PlanningRepeatedQueueHead => {
                "the planning queue selected the same task again; auto follow-up stays paused until the queue advances"
                    .to_string()
            }
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
            Self::PlanningBlocked => "paused: planning files invalid",
            Self::PlanningQueueHeadRequired => "paused: planning queue empty",
            Self::PlanningRepeatedQueueHead => "paused: planning queue repeated the same task",
        }
    }

    pub(crate) fn runtime_status(self, auto_follow_state: &AutoFollowState) -> String {
        match self {
            Self::Disabled => "turn completed / auto follow-up stopped: off".to_string(),
            Self::ManualInputBuffered => {
                "turn completed / auto follow-up skipped: manual input buffered".to_string()
            }
            Self::LimitReached => format!(
                "turn completed / auto follow-up stopped: turn limit reached ({})",
                auto_follow_state.progress_label()
            ),
            Self::NoAgentReply => {
                "turn completed / auto follow-up skipped: no agent reply".to_string()
            }
            Self::StopKeywordMatched => format!(
                "turn completed / auto follow-up stopped: stop keyword matched ({})",
                auto_follow_state.stop_rules.stop_keyword.value()
            ),
            Self::NoFileChanges => {
                "turn completed / auto follow-up stopped: no file changes".to_string()
            }
            Self::PlanningBlocked => {
                "turn completed / auto follow-up paused: planning files invalid".to_string()
            }
            Self::PlanningQueueHeadRequired => {
                "turn completed / auto follow-up paused: planning queue has no next task"
                    .to_string()
            }
            Self::PlanningRepeatedQueueHead => {
                "turn completed / auto follow-up paused: planning queue repeated the previous task"
                    .to_string()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AutoFollowState {
    pub(crate) enabled: bool,
    pub(crate) completed_auto_turns: usize,
    pub(crate) max_auto_turns: usize,
    pub(crate) runtime_phase: AutoFollowRuntimePhase,
    pub(crate) template_state: AutoFollowTemplateState,
    pub(crate) stop_rules: AutoFollowStopRules,
}

#[derive(Debug, Clone)]
pub(crate) enum AutoFollowRuntimePhase {
    Idle,
    Evaluating {
        started_at: Instant,
    },
    Queued {
        started_at: Instant,
        turn_index: usize,
        template_label: String,
    },
    Submitting {
        started_at: Instant,
        turn_index: usize,
        template_label: String,
    },
    Running {
        started_at: Instant,
        turn_index: usize,
    },
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
    pub(crate) current_turn_changed_planning_file_paths: Vec<String>,
    pub(crate) last_completed_turn_id: Option<String>,
    pub(crate) last_completed_turn_file_change_count: usize,
    pub(crate) last_completed_turn_command_count: usize,
    pub(crate) last_completed_turn_last_summary: Option<String>,
    pub(crate) last_completed_turn_changed_planning_file_paths: Vec<String>,
}

impl AutoFollowState {
    pub(crate) fn new(template_catalog: FollowupTemplateCatalog) -> Self {
        Self {
            enabled: true,
            completed_auto_turns: 0,
            max_auto_turns: DEFAULT_AUTO_FOLLOW_MAX_TURNS,
            runtime_phase: AutoFollowRuntimePhase::Idle,
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

    pub(crate) fn completed_progress_label(&self) -> String {
        format!(
            "{}/{} completed",
            self.completed_auto_turns, self.max_auto_turns
        )
    }

    #[cfg(test)]
    pub(crate) fn compact_completed_progress_label(&self) -> String {
        format!("{}/{} done", self.completed_auto_turns, self.max_auto_turns)
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

    #[cfg(test)]
    pub(crate) fn template_source_label(&self) -> String {
        self.template_state.current().source_label()
    }

    #[cfg(test)]
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

    pub(crate) fn active_turn_index(&self) -> Option<usize> {
        self.runtime_phase.turn_index()
    }

    pub(crate) fn active_started_at(&self) -> Option<Instant> {
        self.runtime_phase.started_at()
    }

    pub(crate) fn has_live_activity(&self) -> bool {
        !matches!(self.runtime_phase, AutoFollowRuntimePhase::Idle)
    }

    pub(crate) fn activity_label(&self) -> String {
        match &self.runtime_phase {
            AutoFollowRuntimePhase::Idle => "idle".to_string(),
            AutoFollowRuntimePhase::Evaluating { .. } => "evaluating next turn".to_string(),
            AutoFollowRuntimePhase::Queued { turn_index, .. } => {
                format!("queued turn {turn_index}/{}", self.max_auto_turns)
            }
            AutoFollowRuntimePhase::Submitting { turn_index, .. } => {
                format!("submitting turn {turn_index}/{}", self.max_auto_turns)
            }
            AutoFollowRuntimePhase::Running { turn_index, .. } => {
                format!("running turn {turn_index}/{}", self.max_auto_turns)
            }
        }
    }

    pub(crate) fn can_queue_next(&self) -> bool {
        self.enabled && self.completed_auto_turns < self.max_auto_turns
    }

    pub(crate) fn reset_for_manual_turn(&mut self) {
        self.completed_auto_turns = 0;
        self.runtime_phase = AutoFollowRuntimePhase::Idle;
    }

    pub(crate) fn begin_post_turn_evaluation(&mut self) {
        self.runtime_phase = AutoFollowRuntimePhase::Evaluating {
            started_at: Instant::now(),
        };
    }

    pub(crate) fn mark_auto_turn_queued(&mut self, template_label: &str) -> usize {
        let turn_index = self.next_auto_turn_index();
        self.runtime_phase = AutoFollowRuntimePhase::Queued {
            started_at: Instant::now(),
            turn_index,
            template_label: template_label.to_string(),
        };
        turn_index
    }

    pub(crate) fn mark_auto_turn_submitted(&mut self, template_label: &str) -> usize {
        let turn_index = self
            .active_turn_index()
            .unwrap_or_else(|| self.next_auto_turn_index());
        self.runtime_phase = AutoFollowRuntimePhase::Submitting {
            started_at: Instant::now(),
            turn_index,
            template_label: template_label.to_string(),
        };
        turn_index
    }

    pub(crate) fn mark_auto_turn_started(&mut self) -> Option<(usize, String)> {
        let (turn_index, template_label) = match &self.runtime_phase {
            AutoFollowRuntimePhase::Queued {
                turn_index,
                template_label,
                ..
            }
            | AutoFollowRuntimePhase::Submitting {
                turn_index,
                template_label,
                ..
            } => (*turn_index, template_label.clone()),
            AutoFollowRuntimePhase::Idle
            | AutoFollowRuntimePhase::Evaluating { .. }
            | AutoFollowRuntimePhase::Running { .. } => return None,
        };
        self.runtime_phase = AutoFollowRuntimePhase::Running {
            started_at: Instant::now(),
            turn_index,
        };
        Some((turn_index, template_label))
    }

    pub(crate) fn complete_auto_turn_if_running(&mut self) -> bool {
        match self.runtime_phase {
            AutoFollowRuntimePhase::Submitting { .. } | AutoFollowRuntimePhase::Running { .. } => {
                self.completed_auto_turns += 1;
                self.runtime_phase = AutoFollowRuntimePhase::Idle;
                true
            }
            AutoFollowRuntimePhase::Idle
            | AutoFollowRuntimePhase::Evaluating { .. }
            | AutoFollowRuntimePhase::Queued { .. } => {
                self.runtime_phase = AutoFollowRuntimePhase::Idle;
                false
            }
        }
    }

    pub(crate) fn clear_runtime_phase(&mut self) {
        self.runtime_phase = AutoFollowRuntimePhase::Idle;
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

impl AutoFollowRuntimePhase {
    fn turn_index(&self) -> Option<usize> {
        match self {
            Self::Queued { turn_index, .. }
            | Self::Submitting { turn_index, .. }
            | Self::Running { turn_index, .. } => Some(*turn_index),
            Self::Idle | Self::Evaluating { .. } => None,
        }
    }

    fn started_at(&self) -> Option<Instant> {
        match self {
            Self::Evaluating { started_at }
            | Self::Queued { started_at, .. }
            | Self::Submitting { started_at, .. }
            | Self::Running { started_at, .. } => Some(*started_at),
            Self::Idle => None,
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
        self.current_turn_changed_planning_file_paths.clear();
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

    pub(crate) fn complete_turn(&mut self, turn_id: &str) {
        self.last_completed_turn_id = Some(turn_id.to_string());
        self.last_completed_turn_file_change_count =
            std::mem::replace(&mut self.current_turn_file_change_count, 0);
        self.last_completed_turn_command_count =
            std::mem::replace(&mut self.current_turn_command_count, 0);
        self.last_completed_turn_last_summary = self.current_turn_last_summary.take();
        self.last_completed_turn_changed_planning_file_paths =
            std::mem::take(&mut self.current_turn_changed_planning_file_paths);
    }

    pub(crate) fn register_changed_planning_file_paths(&mut self, paths: &[String]) {
        for path in paths {
            if !self
                .current_turn_changed_planning_file_paths
                .iter()
                .any(|existing| existing == path)
            {
                self.current_turn_changed_planning_file_paths
                    .push(path.clone());
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn last_completed_changed_planning_file_paths(&self) -> &[String] {
        &self.last_completed_turn_changed_planning_file_paths
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
    pub(crate) draft_workspace_directory: String,
    pub(crate) messages: Vec<ConversationMessage>,
    pub(crate) cached_conversation_lines: Vec<Line<'static>>,
    pub(crate) live_agent_message: Option<ConversationMessage>,
    pub(crate) buffered_tool_messages: Vec<ConversationMessage>,
    pub(crate) base_warnings: Vec<String>,
    pub(crate) template_warnings: Vec<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) runtime_notices: Vec<String>,
    pub(crate) input_buffer: String,
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
    pub(crate) last_auto_followup_activity: Option<RecordedAutoFollowupActivity>,
    pub(crate) last_planning_task_handoff: Option<PlanningTaskHandoff>,
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
            cwd: cwd.clone(),
            draft_workspace_directory: cwd,
            messages: Vec::new(),
            cached_conversation_lines: Vec::new(),
            live_agent_message: None,
            buffered_tool_messages: Vec::new(),
            base_warnings: Vec::new(),
            template_warnings: template_load_result.warnings.clone(),
            warnings: template_load_result.warnings,
            runtime_notices: Vec::new(),
            input_buffer: String::new(),
            startup_submit_armed: false,
            active_turn_id: None,
            active_turn_workspace_directory: None,
            active_turn_started_at: None,
            planning_repair_state: None,
            input_state: ConversationInputState::DraftReady,
            auto_follow_state: AutoFollowState::new(template_load_result.catalog),
            planning_runtime_snapshot: PlanningRuntimeSnapshot::uninitialized(),
            turn_activity: TurnActivityState::default(),
            approval_review: None,
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
        template_load_result: FollowupTemplateCatalogLoadResult,
        draft_workspace_directory: String,
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
            draft_workspace_directory,
            messages: snapshot.messages,
            cached_conversation_lines: Vec::new(),
            live_agent_message: None,
            buffered_tool_messages: Vec::new(),
            base_warnings,
            template_warnings,
            warnings,
            runtime_notices,
            input_buffer: String::new(),
            startup_submit_armed: false,
            active_turn_id: None,
            active_turn_workspace_directory: None,
            active_turn_started_at: None,
            planning_repair_state: None,
            input_state: ConversationInputState::ReadyToContinue,
            auto_follow_state: AutoFollowState::new(template_load_result.catalog),
            planning_runtime_snapshot: PlanningRuntimeSnapshot::uninitialized(),
            turn_activity: TurnActivityState::default(),
            approval_review: None,
            last_auto_followup_activity: None,
            last_planning_task_handoff: None,
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

    pub(crate) fn replace_planning_runtime_snapshot(
        &mut self,
        planning_runtime_snapshot: PlanningRuntimeSnapshot,
    ) {
        self.planning_runtime_snapshot = planning_runtime_snapshot;
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

    pub(crate) fn sync_draft_workspace(
        &mut self,
        workspace_directory: String,
        template_load_result: FollowupTemplateCatalogLoadResult,
    ) -> bool {
        if self.has_active_thread() || self.draft_workspace_directory == workspace_directory {
            return false;
        }

        let template_count = template_load_result.catalog.items.len();
        let warnings = template_load_result.warnings;
        self.draft_workspace_directory = workspace_directory.clone();
        self.cwd = workspace_directory;
        self.auto_follow_state = AutoFollowState::new(template_load_result.catalog);
        self.base_warnings.clear();
        self.replace_template_warnings(warnings);
        self.clear_auto_followup_skip();
        self.set_status_with_warnings(format!(
            "draft workspace synced / templates: {template_count}"
        ));

        true
    }

    pub(crate) fn record_submitted_prompt(
        &mut self,
        transcript_message: ConversationMessage,
        workspace_directory: String,
    ) {
        self.push_message(transcript_message);
        self.input_buffer.clear();
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
        if let Some((turn_index, template_label)) = self.auto_follow_state.mark_auto_turn_started()
        {
            let status_text = format!(
                "auto follow-up running / turn {turn_index}/{} / template: {template_label}",
                self.auto_follow_state.max_auto_turns_value()
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

    pub(crate) fn clear_last_planning_task_handoff(&mut self) {
        self.last_planning_task_handoff = None;
    }

    pub(crate) fn record_planning_repair_submission(
        &mut self,
        attempt: usize,
        max_attempts: usize,
    ) {
        self.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
            summary: format!("submitted planning repair {attempt}/{max_attempts}"),
            detail: format!(
                "queued after task-ledger validation failed; submitted retry {attempt}/{max_attempts}"
            ),
        });
    }

    pub(crate) fn record_planning_repair_queue(&mut self, attempt: usize, max_attempts: usize) {
        self.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
            summary: format!("queued planning repair {attempt}/{max_attempts}"),
            detail: format!(
                "queued after task-ledger validation failed; waiting to submit retry {attempt}/{max_attempts}"
            ),
        });
    }

    pub(crate) fn record_auto_followup_submission(
        &mut self,
        _queued_from_turn_id: &str,
        template_label: &str,
        handoff_task: Option<&PlanningTaskHandoff>,
    ) {
        let turn_index = self
            .auto_follow_state
            .mark_auto_turn_submitted(template_label);
        let progress = format!(
            "{turn_index}/{}",
            self.auto_follow_state.max_auto_turns_value()
        );
        self.last_planning_task_handoff = handoff_task.cloned();
        self.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
            summary: format!("submitted auto turn {progress}"),
            detail: format!(
                "queued after the previous turn completed; submitted with template {template_label}"
            ),
        });
    }

    pub(crate) fn record_auto_followup_queue(
        &mut self,
        _queued_from_turn_id: &str,
        template_label: &str,
    ) {
        let turn_index = self.auto_follow_state.mark_auto_turn_queued(template_label);
        let next_progress = format!(
            "{turn_index}/{}",
            self.auto_follow_state.max_auto_turns_value()
        );
        self.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
            summary: format!("queued auto turn {next_progress}"),
            detail: format!(
                "queued after the previous turn completed; waiting to submit with template {template_label}"
            ),
        });
    }

    pub(crate) fn begin_auto_followup_evaluation(&mut self) {
        if !self.auto_follow_state.enabled
            || !self.input_buffer.trim().is_empty()
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

    #[cfg(test)]
    pub(crate) fn decide_auto_followup(
        &self,
        planning_runtime_facade_service: &PlanningRuntimeFacadeService,
    ) -> AutoFollowupDecision {
        self.decide_auto_followup_with_snapshot(
            planning_runtime_facade_service,
            &self.planning_runtime_snapshot,
        )
    }

    pub(crate) fn decide_auto_followup_with_snapshot(
        &self,
        planning_runtime_facade_service: &PlanningRuntimeFacadeService,
        planning_runtime_snapshot: &PlanningRuntimeSnapshot,
    ) -> AutoFollowupDecision {
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

        match planning_runtime_facade_service.decide_auto_followup(
            PlanningRuntimeAutoFollowRequest {
                template: self.auto_follow_state.selected_template(),
                auto_turn: self.auto_follow_state.next_auto_turn_index(),
                max_auto_turns: self.auto_follow_state.max_auto_turns_value(),
                session_id: &self.thread_id,
                stop_keyword: self.auto_follow_state.stop_keyword_value(),
                last_message: last_message.trim(),
                snapshot: planning_runtime_snapshot,
            },
        ) {
            PlanningRuntimeAutoFollowDecision::QueuePrompt(prompt) => {
                AutoFollowupDecision::QueuePrompt(prompt)
            }
            PlanningRuntimeAutoFollowDecision::Blocked(block_reason) => {
                AutoFollowupDecision::Skip(match block_reason {
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

#[cfg(test)]
#[path = "conversation_model_tests.rs"]
mod tests;

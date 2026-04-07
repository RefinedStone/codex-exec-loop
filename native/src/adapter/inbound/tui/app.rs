use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::MoveToNextLine;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::adapter::inbound::tui::shell_chrome::{
    ExitConfirmationState, SessionState, ShellChromeEffect, ShellChromeEvent, ShellChromeState,
    ShellOverlay, StartupState, reduce_shell_chrome,
};
use crate::adapter::outbound::codex_app_server_adapter::CodexAppServerAdapter;
use crate::adapter::outbound::filesystem_followup_template_adapter::FilesystemFollowupTemplateAdapter;
use crate::application::port::outbound::codex_app_server_port::CodexAppServerPort;
use crate::application::port::outbound::followup_template_port::FollowupTemplatePort;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::followup_template_service::FollowupTemplateService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::{
    ConversationMessage, ConversationMessageKind, ConversationSnapshot, ConversationStreamEvent,
};
use crate::domain::followup_template::{
    FollowupTemplateCatalog, FollowupTemplateCatalogLoadResult, FollowupTemplateDefinition,
};
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;
use crate::domain::startup_diagnostics::StartupDiagnostics;

const SESSION_PAGE_SIZE: usize = 10;
const MAX_CONVERSATION_HISTORY_LINES: usize = 160;
const DEFAULT_AUTO_FOLLOW_MAX_TURNS: usize = 3;
const DEFAULT_AUTO_FOLLOW_STOP_KEYWORD: &str = "AUTO_STOP";
const FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP: u16 = 6;
const MIN_SHELL_STATUS_HEIGHT: u16 = 5;
const MAX_SHELL_STATUS_HEIGHT: u16 = 8;
const MIN_COMPOSER_HEIGHT: u16 = 4;
const MAX_COMPOSER_HEIGHT: u16 = 8;
const DEFAULT_TRANSCRIPT_PAGE_STEP: u16 = 6;
const ALT_SCREEN_ENV_VAR: &str = "CODEX_EXEC_LOOP_ALT_SCREEN";

#[path = "app/conversation_input.rs"]
mod conversation_input;
#[path = "app/conversation_intents.rs"]
mod conversation_intents;
#[path = "app/conversation_lifecycle.rs"]
mod conversation_lifecycle;
#[path = "app/conversation_runtime.rs"]
mod conversation_runtime;
#[path = "app/followup_controls.rs"]
mod followup_controls;
#[path = "app/followup_overlay_ui.rs"]
mod followup_overlay_ui;
#[path = "app/inline_shell_commands.rs"]
mod inline_shell_commands;
#[path = "app/session_overlay_ui.rs"]
mod session_overlay_ui;
#[path = "app/shell_layout.rs"]
mod shell_layout;
#[path = "app/shell_presentation.rs"]
mod shell_presentation;
#[path = "app/shell_rendering.rs"]
mod shell_rendering;
#[path = "app/transcript_viewport.rs"]
mod transcript_viewport;

use conversation_input::{ConversationInputEvent, reduce_conversation_input};
use conversation_intents::{
    ConversationIntentEffect, ConversationIntentEvent, ConversationIntentMode,
    ConversationIntentState, reduce_conversation_intents,
};
use conversation_lifecycle::{
    ConversationLifecycleEffect, ConversationLifecycleEvent, ConversationLifecycleState,
    reduce_conversation_lifecycle,
};
use conversation_runtime::{
    ConversationRuntimeEffect, ConversationRuntimeEvent, reduce_conversation_runtime,
};
use followup_controls::{FollowupControlEffect, FollowupControlEvent, reduce_followup_controls};
use followup_overlay_ui::{
    FollowupOverlayUiEvent, FollowupOverlayUiState, reduce_followup_overlay_ui,
};
use inline_shell_commands::InlineShellCommand;
use session_overlay_ui::SessionOverlayUiState;
use shell_layout::{
    block_height_for_lines, build_conversation_scroll_offset, build_input_block_height,
    build_shell_footer_height,
};
#[cfg(test)]
use shell_presentation::build_ready_input_lines;
use shell_presentation::{
    build_conversation_lines, build_input_lines, build_input_title, build_shell_footer_lines,
    build_shell_title, build_status_title, build_transcript_title, format_conversation_lines,
    input_state_style, shell_action_availability_label, startup_state_style,
};
use shell_rendering::draw;
use transcript_viewport::TranscriptViewportState;

pub fn run() -> Result<()> {
    let codex_app_server_port: Arc<dyn CodexAppServerPort> = Arc::new(CodexAppServerAdapter::new(
        "codex-exec-loop-native",
        env!("CARGO_PKG_VERSION"),
    ));
    let followup_template_port: Arc<dyn FollowupTemplatePort> =
        Arc::new(FilesystemFollowupTemplateAdapter::new());
    let startup_service = StartupService::new(codex_app_server_port.clone());
    let session_service = SessionService::new(codex_app_server_port.clone());
    let conversation_service = ConversationService::new(codex_app_server_port);
    let followup_template_service = FollowupTemplateService::new(followup_template_port);

    let mut app = NativeTuiApp::new(
        startup_service,
        session_service,
        conversation_service,
        followup_template_service,
    );
    app.dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested);
    run_tui(app)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiPresentationMode {
    MainScreen,
    AlternateScreen,
}

impl TuiPresentationMode {
    fn from_environment() -> Self {
        Self::from_env_value(std::env::var(ALT_SCREEN_ENV_VAR).ok().as_deref())
    }

    fn from_env_value(value: Option<&str>) -> Self {
        if value.is_some_and(env_flag_is_truthy) {
            Self::AlternateScreen
        } else {
            Self::MainScreen
        }
    }

    fn uses_alternate_screen(self) -> bool {
        self == Self::AlternateScreen
    }
}

fn env_flag_is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[derive(Debug, Clone)]
enum ConversationState {
    Loading,
    Ready(ConversationViewModel),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConversationInputState {
    DraftReady,
    ReadyToContinue,
    SubmittingTurn,
    StreamingTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellActionAvailability {
    Ready,
    Pending,
    Blocked,
}

impl ConversationInputState {
    fn label(self) -> &'static str {
        match self {
            Self::DraftReady => "draft ready",
            Self::ReadyToContinue => "ready",
            Self::SubmittingTurn => "submitting",
            Self::StreamingTurn => "streaming",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::DraftReady => "first prompt will create a new thread",
            Self::ReadyToContinue => "session is ready for the next prompt",
            Self::SubmittingTurn => "sending prompt to codex app-server",
            Self::StreamingTurn => "current turn is still running",
        }
    }

    fn can_submit_now(self) -> bool {
        matches!(self, Self::DraftReady | Self::ReadyToContinue)
    }
}

impl ShellActionAvailability {
    fn allows_actions(self) -> bool {
        self == Self::Ready
    }

    fn status_text(self) -> &'static str {
        match self {
            Self::Ready => "startup ready",
            Self::Pending => "startup checks still running",
            Self::Blocked => "startup diagnostics need attention",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptOrigin {
    Manual,
    AutoFollow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AutoFollowupDecision {
    QueuePrompt(String),
    Skip(AutoFollowupSkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoFollowupSkipReason {
    Disabled,
    ManualInputBuffered,
    LimitReached,
    NoAgentReply,
    StopKeywordMatched,
    NoFileChanges,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedAutoFollowupSkip {
    reason: AutoFollowupSkipReason,
    detail: String,
}

impl AutoFollowupSkipReason {
    fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::ManualInputBuffered => "manual input buffered",
            Self::LimitReached => "turn limit reached",
            Self::NoAgentReply => "no agent reply",
            Self::StopKeywordMatched => "stop keyword matched",
            Self::NoFileChanges => "no file changes",
        }
    }

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
}

#[derive(Debug, Clone)]
struct AutoFollowState {
    enabled: bool,
    completed_auto_turns: usize,
    max_auto_turns: usize,
    template_state: AutoFollowTemplateState,
    stop_rules: AutoFollowStopRules,
}

#[derive(Debug, Clone)]
struct AutoFollowStopRules {
    stop_keyword: StopKeywordRule,
    stop_on_no_file_changes: bool,
}

#[derive(Debug, Clone)]
struct StopKeywordRule {
    enabled: bool,
    value: String,
}

#[derive(Debug, Clone)]
struct AutoFollowTemplateState {
    items: Vec<FollowupTemplateDefinition>,
    selected_index: usize,
}

#[derive(Debug, Clone, Default)]
struct TurnActivityState {
    current_turn_file_change_count: usize,
    last_completed_turn_file_change_count: usize,
}

impl AutoFollowState {
    fn new(template_catalog: FollowupTemplateCatalog) -> Self {
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
    fn status_label(&self) -> &'static str {
        if self.enabled { "on" } else { "off" }
    }

    fn progress_label(&self) -> String {
        format!("{}/{}", self.completed_auto_turns, self.max_auto_turns)
    }

    fn template_label(&self) -> &str {
        self.template_state.current().label.as_str()
    }

    fn selected_template(&self) -> &FollowupTemplateDefinition {
        self.template_state.current()
    }

    fn selected_template_index(&self) -> usize {
        self.template_state.selected_index
    }

    fn template_source_label(&self) -> String {
        self.template_state.current().source_label()
    }

    fn template_count(&self) -> usize {
        self.template_state.items.len()
    }

    fn stop_keyword_label(&self) -> String {
        self.stop_rules.stop_keyword.label()
    }

    fn stop_keyword_value(&self) -> &str {
        self.stop_rules.stop_keyword.value()
    }

    fn no_file_change_stop_label(&self) -> &'static str {
        self.stop_rules.no_file_change_label()
    }

    fn next_auto_turn_index(&self) -> usize {
        self.completed_auto_turns + 1
    }

    fn can_queue_next(&self) -> bool {
        self.enabled && self.completed_auto_turns < self.max_auto_turns
    }

    fn reset_for_manual_turn(&mut self) {
        self.completed_auto_turns = 0;
    }

    fn mark_auto_turn_submitted(&mut self) {
        self.completed_auto_turns += 1;
    }

    fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    fn toggle_stop_keyword(&mut self) {
        self.stop_rules.stop_keyword.toggle();
    }

    fn set_stop_keyword_value(&mut self, value: String) {
        self.stop_rules.stop_keyword.set_value(value);
    }

    fn toggle_no_file_change_stop(&mut self) {
        self.stop_rules.stop_on_no_file_changes = !self.stop_rules.stop_on_no_file_changes;
    }

    fn cycle_template_kind(&mut self) {
        self.template_state.cycle();
    }

    fn cycle_template_kind_backward(&mut self) {
        self.template_state.cycle_previous();
    }

    fn render_prompt(&self, thread_id: &str, last_message: &str) -> String {
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

    fn render_prompt_preview(&self, thread_id: &str, last_message: Option<&str>) -> String {
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
}

impl AutoFollowStopRules {
    fn should_stop_on_no_file_changes(&self, file_change_count: usize) -> bool {
        self.stop_on_no_file_changes && file_change_count == 0
    }

    fn no_file_change_label(&self) -> &'static str {
        if self.stop_on_no_file_changes {
            "on"
        } else {
            "off"
        }
    }
}

impl StopKeywordRule {
    fn normalize_candidate(candidate: &str) -> Option<String> {
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

    fn label(&self) -> String {
        if self.enabled {
            format!("on ({})", self.value)
        } else {
            format!("off ({})", self.value)
        }
    }

    fn matches(&self, text: &str) -> bool {
        self.enabled
            && text.split_whitespace().any(|token| {
                token
                    .trim_matches(|character: char| {
                        !character.is_alphanumeric() && character != '_'
                    })
                    .eq_ignore_ascii_case(&self.value)
            })
    }

    fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    fn set_value(&mut self, value: String) {
        self.value = value;
    }

    fn value(&self) -> &str {
        self.value.as_str()
    }
}

impl AutoFollowTemplateState {
    fn new(template_catalog: FollowupTemplateCatalog) -> Self {
        Self {
            items: template_catalog.items,
            selected_index: 0,
        }
    }

    fn current(&self) -> &FollowupTemplateDefinition {
        self.items
            .get(self.selected_index)
            .expect("follow-up template catalog should not be empty")
    }

    fn cycle(&mut self) {
        if self.items.len() <= 1 {
            return;
        }

        self.selected_index = (self.selected_index + 1) % self.items.len();
    }

    fn cycle_previous(&mut self) {
        if self.items.len() <= 1 {
            return;
        }

        if self.selected_index == 0 {
            self.selected_index = self.items.len() - 1;
        } else {
            self.selected_index -= 1;
        }
    }
}

impl TurnActivityState {
    fn start_new_turn(&mut self) {
        self.current_turn_file_change_count = 0;
    }

    fn register_file_change(&mut self, file_change_count: usize) {
        self.current_turn_file_change_count += file_change_count;
    }

    fn complete_turn(&mut self) {
        self.last_completed_turn_file_change_count = self.current_turn_file_change_count;
    }

    fn last_completed_file_change_count(&self) -> usize {
        self.last_completed_turn_file_change_count
    }
}

enum BackgroundMessage {
    StartupLoaded(Result<StartupDiagnostics, String>),
    SessionsLoaded(Result<RecentSessions, String>),
    ConversationLoaded(Result<ConversationSnapshot, String>),
    ConversationStream(ConversationStreamEvent),
}

#[derive(Debug, Clone)]
struct ConversationViewModel {
    thread_id: String,
    title: String,
    cwd: String,
    messages: Vec<ConversationMessage>,
    cached_conversation_lines: Vec<Line<'static>>,
    warnings: Vec<String>,
    input_buffer: String,
    active_turn_id: Option<String>,
    input_state: ConversationInputState,
    auto_follow_state: AutoFollowState,
    turn_activity: TurnActivityState,
    last_auto_followup_skip: Option<RecordedAutoFollowupSkip>,
    status_text: String,
}

impl ConversationViewModel {
    fn new_draft(cwd: String, template_load_result: FollowupTemplateCatalogLoadResult) -> Self {
        let status_text = format!(
            "new thread draft / templates: {}",
            template_load_result.catalog.items.len()
        );
        let mut view_model = Self {
            thread_id: String::new(),
            title: "New conversation".to_string(),
            cwd,
            messages: Vec::new(),
            cached_conversation_lines: Vec::new(),
            warnings: template_load_result.warnings,
            input_buffer: String::new(),
            active_turn_id: None,
            input_state: ConversationInputState::DraftReady,
            auto_follow_state: AutoFollowState::new(template_load_result.catalog),
            turn_activity: TurnActivityState::default(),
            last_auto_followup_skip: None,
            status_text,
        };
        view_model.refresh_conversation_lines();
        view_model
    }

    fn from_snapshot(
        snapshot: ConversationSnapshot,
        template_load_result: FollowupTemplateCatalogLoadResult,
    ) -> Self {
        let mut warnings = snapshot.warnings;
        warnings.extend(template_load_result.warnings);
        let status_text = if warnings.is_empty() {
            format!(
                "thread loaded / templates: {}",
                template_load_result.catalog.items.len()
            )
        } else {
            warnings.join(" | ")
        };

        let mut view_model = Self {
            thread_id: snapshot.thread_id,
            title: snapshot.title,
            cwd: snapshot.cwd,
            messages: snapshot.messages,
            cached_conversation_lines: Vec::new(),
            warnings,
            input_buffer: String::new(),
            active_turn_id: None,
            input_state: ConversationInputState::ReadyToContinue,
            auto_follow_state: AutoFollowState::new(template_load_result.catalog),
            turn_activity: TurnActivityState::default(),
            last_auto_followup_skip: None,
            status_text,
        };
        view_model.refresh_conversation_lines();
        view_model
    }

    fn refresh_conversation_lines(&mut self) {
        self.cached_conversation_lines = format_conversation_lines(&self.messages);
    }

    fn has_active_thread(&self) -> bool {
        !self.thread_id.trim().is_empty()
    }

    fn is_blank_draft(&self) -> bool {
        !self.has_active_thread()
            && self.messages.is_empty()
            && self.input_buffer.trim().is_empty()
            && self.active_turn_id.is_none()
    }

    fn ready_input_state(&self) -> ConversationInputState {
        if self.has_active_thread() {
            ConversationInputState::ReadyToContinue
        } else {
            ConversationInputState::DraftReady
        }
    }

    fn can_submit_prompt(&self) -> bool {
        self.input_state.can_submit_now()
    }

    fn has_running_turn(&self) -> bool {
        !self.can_submit_prompt()
    }

    fn mark_turn_submitting(&mut self) {
        self.input_state = ConversationInputState::SubmittingTurn;
    }

    fn mark_turn_started(&mut self, turn_id: String) {
        self.active_turn_id = Some(turn_id);
        self.input_state = ConversationInputState::StreamingTurn;
        self.turn_activity.start_new_turn();
    }

    fn mark_turn_finished(&mut self) {
        self.active_turn_id = None;
        self.input_state = self.ready_input_state();
    }

    fn record_auto_followup_skip(&mut self, reason: AutoFollowupSkipReason) {
        self.last_auto_followup_skip = Some(RecordedAutoFollowupSkip {
            reason,
            detail: reason.detail(&self.auto_follow_state, &self.turn_activity),
        });
    }

    fn clear_auto_followup_skip(&mut self) {
        self.last_auto_followup_skip = None;
    }

    fn latest_agent_message_text(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|message| {
                message.kind == ConversationMessageKind::Agent && !message.text.trim().is_empty()
            })
            .map(|message| message.text.as_str())
    }

    fn decide_auto_followup(&self) -> AutoFollowupDecision {
        match (
            self.auto_follow_state.enabled,
            self.input_buffer.trim().is_empty(),
            self.auto_follow_state.can_queue_next(),
            self.latest_agent_message_text(),
        ) {
            (false, _, _, _) => AutoFollowupDecision::Skip(AutoFollowupSkipReason::Disabled),
            (true, false, _, _) => {
                AutoFollowupDecision::Skip(AutoFollowupSkipReason::ManualInputBuffered)
            }
            (true, true, false, _) => {
                AutoFollowupDecision::Skip(AutoFollowupSkipReason::LimitReached)
            }
            (true, true, true, None) => {
                AutoFollowupDecision::Skip(AutoFollowupSkipReason::NoAgentReply)
            }
            (true, true, true, Some(last_message))
                if self
                    .auto_follow_state
                    .stop_rules
                    .stop_keyword
                    .matches(last_message.trim()) =>
            {
                AutoFollowupDecision::Skip(AutoFollowupSkipReason::StopKeywordMatched)
            }
            (true, true, true, Some(_))
                if self
                    .auto_follow_state
                    .stop_rules
                    .should_stop_on_no_file_changes(
                        self.turn_activity.last_completed_file_change_count(),
                    ) =>
            {
                AutoFollowupDecision::Skip(AutoFollowupSkipReason::NoFileChanges)
            }
            (true, true, true, Some(last_message)) => AutoFollowupDecision::QueuePrompt(
                self.auto_follow_state
                    .render_prompt(&self.thread_id, last_message.trim()),
            ),
        }
    }
}

struct NativeTuiApp {
    shell_overlay: ShellOverlay,
    exit_confirmation_state: ExitConfirmationState,
    startup_state: StartupState,
    session_state: SessionState,
    conversation_state: ConversationState,
    selected_session_index: usize,
    session_overlay_ui_state: SessionOverlayUiState,
    followup_overlay_ui_state: FollowupOverlayUiState,
    transcript_viewport_state: TranscriptViewportState,
    active_session: Option<SessionSummary>,
    startup_service: StartupService,
    session_service: SessionService,
    conversation_service: ConversationService,
    followup_template_service: FollowupTemplateService,
    tx: Sender<BackgroundMessage>,
    rx: Receiver<BackgroundMessage>,
}

impl NativeTuiApp {
    fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        followup_template_service: FollowupTemplateService,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let workspace_directory = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let initial_conversation = ConversationState::Ready(ConversationViewModel::new_draft(
            workspace_directory.clone(),
            followup_template_service.load_catalog(&workspace_directory),
        ));
        let app = Self {
            shell_overlay: ShellOverlay::Hidden,
            exit_confirmation_state: ExitConfirmationState::Hidden,
            startup_state: StartupState::Idle,
            session_state: SessionState::Idle,
            conversation_state: initial_conversation,
            selected_session_index: 0,
            session_overlay_ui_state: SessionOverlayUiState::default(),
            followup_overlay_ui_state: FollowupOverlayUiState::default(),
            transcript_viewport_state: TranscriptViewportState::default(),
            active_session: None,
            startup_service,
            session_service,
            conversation_service,
            followup_template_service,
            tx,
            rx,
        };
        app
    }

    fn take_shell_chrome_state(&mut self) -> ShellChromeState {
        ShellChromeState {
            shell_overlay: self.shell_overlay,
            exit_confirmation_state: self.exit_confirmation_state,
            startup_state: std::mem::replace(&mut self.startup_state, StartupState::Idle),
            session_state: std::mem::replace(&mut self.session_state, SessionState::Idle),
            selected_session_index: self.selected_session_index,
        }
    }

    fn apply_shell_chrome_state(&mut self, state: ShellChromeState) {
        self.shell_overlay = state.shell_overlay;
        self.exit_confirmation_state = state.exit_confirmation_state;
        self.startup_state = state.startup_state;
        self.session_state = state.session_state;
        self.selected_session_index = state.selected_session_index;
    }

    fn dispatch_shell_chrome(&mut self, event: ShellChromeEvent) {
        let reduction = reduce_shell_chrome(self.take_shell_chrome_state(), event);
        self.apply_shell_chrome_state(reduction.state);
        for effect in reduction.effects {
            self.execute_shell_chrome_effect(effect);
        }
    }

    fn execute_shell_chrome_effect(&mut self, effect: ShellChromeEffect) {
        match effect {
            ShellChromeEffect::RunStartupChecks => {
                let tx = self.tx.clone();
                let service = self.startup_service.clone();
                thread::spawn(move || {
                    let result = service.run_checks().map_err(|error| error.to_string());
                    let _ = tx.send(BackgroundMessage::StartupLoaded(result));
                });
            }
            ShellChromeEffect::LoadRecentSessions { limit } => {
                let tx = self.tx.clone();
                let service = self.session_service.clone();
                thread::spawn(move || {
                    let result = service
                        .load_recent_sessions(limit)
                        .map_err(|error| error.to_string());
                    let _ = tx.send(BackgroundMessage::SessionsLoaded(result));
                });
            }
        }
    }

    fn take_conversation_lifecycle_state(&mut self) -> ConversationLifecycleState {
        ConversationLifecycleState {
            conversation_state: std::mem::replace(
                &mut self.conversation_state,
                ConversationState::Loading,
            ),
            active_session: self.active_session.take(),
        }
    }

    fn apply_conversation_lifecycle_state(&mut self, state: ConversationLifecycleState) {
        self.conversation_state = state.conversation_state;
        self.active_session = state.active_session;
    }

    fn dispatch_conversation_lifecycle(&mut self, event: ConversationLifecycleEvent) {
        let reduction =
            reduce_conversation_lifecycle(self.take_conversation_lifecycle_state(), event);
        self.apply_conversation_lifecycle_state(reduction.state);
        self.reset_transcript_viewport();
        for effect in reduction.effects {
            self.execute_conversation_lifecycle_effect(effect);
        }
    }

    fn execute_conversation_lifecycle_effect(&mut self, effect: ConversationLifecycleEffect) {
        match effect {
            ConversationLifecycleEffect::LoadConversation { thread_id } => {
                let tx = self.tx.clone();
                let service = self.conversation_service.clone();
                thread::spawn(move || {
                    let result = service
                        .load_snapshot(&thread_id)
                        .map_err(|error| error.to_string());
                    let _ = tx.send(BackgroundMessage::ConversationLoaded(result));
                });
            }
        }
    }

    fn start_turn_submission(&mut self) {
        let inline_command = match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                InlineShellCommand::parse(&conversation.input_buffer)
            }
            _ => None,
        };
        if let Some(command) = inline_command {
            self.execute_inline_shell_command(command);
            return;
        }

        let prompt = match &self.conversation_state {
            ConversationState::Ready(conversation) if conversation.can_submit_prompt() => {
                conversation.input_buffer.trim().to_string()
            }
            _ => return,
        };
        self.submit_prompt(prompt, PromptOrigin::Manual);
    }

    fn take_ready_conversation_state(&mut self) -> Option<ConversationViewModel> {
        let state = std::mem::replace(&mut self.conversation_state, ConversationState::Loading);
        match state {
            ConversationState::Ready(conversation) => Some(conversation),
            other => {
                self.conversation_state = other;
                None
            }
        }
    }

    fn dispatch_conversation_runtime(&mut self, event: ConversationRuntimeEvent) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let reduction = reduce_conversation_runtime(conversation, event);
        self.conversation_state = ConversationState::Ready(reduction.state);
        for effect in reduction.effects {
            self.execute_conversation_runtime_effect(effect);
        }
    }

    fn dispatch_conversation_input(&mut self, event: ConversationInputEvent) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let reduction = reduce_conversation_input(conversation, event);
        self.conversation_state = ConversationState::Ready(reduction.state);
    }

    fn clear_input_buffer(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::InputCleared);
    }

    fn conversation_intent_state(&self) -> ConversationIntentState {
        let mode = match &self.conversation_state {
            ConversationState::Loading => ConversationIntentMode::Loading,
            ConversationState::Failed(_) => ConversationIntentMode::Failed,
            ConversationState::Ready(conversation) if conversation.is_blank_draft() => {
                ConversationIntentMode::BlankDraft
            }
            ConversationState::Ready(_) => ConversationIntentMode::Ready,
        };

        ConversationIntentState {
            has_running_turn: self.conversation_has_running_turn(),
            mode,
        }
    }

    fn dispatch_conversation_intent(&mut self, event: ConversationIntentEvent) {
        let reduction = reduce_conversation_intents(self.conversation_intent_state(), event);
        for effect in reduction.effects {
            self.execute_conversation_intent_effect(effect);
        }
    }

    fn execute_conversation_intent_effect(&mut self, effect: ConversationIntentEffect) {
        match effect {
            ConversationIntentEffect::ShowStatus { status_text } => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
            }
            ConversationIntentEffect::OpenNewDraft => {
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                let workspace_directory = self.current_workspace_directory();
                let template_load_result =
                    self.load_followup_template_catalog(&workspace_directory);
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
                    workspace_directory: workspace_directory.clone(),
                    template_load_result,
                });
                self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::ContentReset {
                    stop_keyword: self.current_stop_keyword_value(),
                });
            }
            ConversationIntentEffect::OpenSession { session } => {
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
                    session,
                });
            }
            ConversationIntentEffect::ShowExitConfirmation => {
                self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationShown);
            }
        }
    }

    fn execute_conversation_runtime_effect(&mut self, effect: ConversationRuntimeEffect) {
        match effect {
            ConversationRuntimeEffect::StartStream {
                cwd,
                thread_id,
                prompt,
            } => {
                let outer_tx = self.tx.clone();
                let service = self.conversation_service.clone();
                thread::spawn(move || {
                    let (event_tx, event_rx) = mpsc::channel();

                    let service_thread = thread::spawn(move || {
                        let result = match thread_id {
                            Some(thread_id) => {
                                service.run_turn_stream(&thread_id, &prompt, event_tx)
                            }
                            None => service.run_new_thread_stream(&cwd, &prompt, event_tx),
                        };
                        let _ = result;
                    });

                    while let Ok(event) = event_rx.recv() {
                        let should_stop = matches!(
                            event,
                            ConversationStreamEvent::TurnCompleted { .. }
                                | ConversationStreamEvent::Failed { .. }
                        );
                        let _ = outer_tx.send(BackgroundMessage::ConversationStream(event));
                        if should_stop {
                            break;
                        }
                    }

                    let _ = service_thread.join();
                });
            }
            ConversationRuntimeEffect::QueueAutoPrompt { prompt } => {
                self.submit_prompt(prompt, PromptOrigin::AutoFollow);
            }
        }
    }

    fn dispatch_followup_controls(&mut self, event: FollowupControlEvent) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let reduction = reduce_followup_controls(conversation, event);
        self.conversation_state = ConversationState::Ready(reduction.state);
        if !self.is_stop_keyword_editing() {
            self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordValueSynced {
                value: self.current_stop_keyword_value(),
            });
        }
        for effect in reduction.effects {
            self.execute_followup_control_effect(effect);
        }
    }

    fn execute_followup_control_effect(&mut self, effect: FollowupControlEffect) {
        match effect {
            FollowupControlEffect::SyncTemplateOverlayUi => {
                self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::TemplateChanged);
            }
            FollowupControlEffect::SyncStopKeywordEditor { value } => {
                self.dispatch_followup_overlay_ui(
                    FollowupOverlayUiEvent::StopKeywordEditCommitted {
                        current_value: value,
                    },
                );
            }
        }
    }

    fn dispatch_followup_overlay_ui(&mut self, event: FollowupOverlayUiEvent) {
        let state = std::mem::take(&mut self.followup_overlay_ui_state);
        self.followup_overlay_ui_state = reduce_followup_overlay_ui(state, event);
    }

    fn submit_prompt(&mut self, prompt: String, prompt_origin: PromptOrigin) {
        if !self.shell_action_availability().allows_actions() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: self.submission_blocked_status(prompt_origin),
            });
            return;
        }

        self.dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            origin: prompt_origin,
        });
    }

    fn poll_background_messages(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                BackgroundMessage::StartupLoaded(result) => {
                    let workspace_directory = match &result {
                        Ok(diagnostics) => Some(diagnostics.workspace_path.clone()),
                        Err(_) => None,
                    };
                    self.dispatch_shell_chrome(ShellChromeEvent::StartupLoaded {
                        result,
                        session_page_size: SESSION_PAGE_SIZE,
                    });
                    if let Some(workspace_directory) = workspace_directory {
                        self.sync_draft_shell_workspace(&workspace_directory);
                    }
                }
                BackgroundMessage::SessionsLoaded(result) => {
                    self.dispatch_shell_chrome(ShellChromeEvent::SessionsLoaded(result));
                    self.session_overlay_ui_state.reset();
                }
                BackgroundMessage::ConversationLoaded(result) => {
                    let template_load_result = match &result {
                        Ok(snapshot) => Some(self.load_followup_template_catalog(&snapshot.cwd)),
                        Err(_) => None,
                    };
                    self.dispatch_conversation_lifecycle(
                        ConversationLifecycleEvent::ConversationLoaded {
                            result,
                            template_load_result,
                        },
                    );
                    self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::ContentReset {
                        stop_keyword: self.current_stop_keyword_value(),
                    });
                }
                BackgroundMessage::ConversationStream(event) => {
                    self.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
                        event,
                    ));
                }
            }
        }
    }

    fn can_open_session_list(&self) -> bool {
        matches!(
            &self.startup_state,
            StartupState::Ready(diagnostics) if diagnostics.can_continue()
        )
    }

    fn shell_action_availability(&self) -> ShellActionAvailability {
        match &self.startup_state {
            StartupState::Ready(diagnostics) if diagnostics.can_continue() => {
                ShellActionAvailability::Ready
            }
            StartupState::Idle | StartupState::Loading => ShellActionAvailability::Pending,
            StartupState::Ready(_) | StartupState::Failed(_) => ShellActionAvailability::Blocked,
        }
    }

    fn submission_blocked_status(&self, prompt_origin: PromptOrigin) -> String {
        match (prompt_origin, self.shell_action_availability()) {
            (_, ShellActionAvailability::Ready) => "ready".to_string(),
            (PromptOrigin::Manual, state) => {
                format!("{}; open diagnostics with Ctrl+d", state.status_text())
            }
            (PromptOrigin::AutoFollow, ShellActionAvailability::Pending) => {
                "auto follow-up paused while startup checks are still running".to_string()
            }
            (PromptOrigin::AutoFollow, ShellActionAvailability::Blocked) => {
                "auto follow-up paused because startup diagnostics need attention".to_string()
            }
        }
    }

    fn conversation_has_running_turn(&self) -> bool {
        matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation) if conversation.has_running_turn()
        )
    }

    fn sync_draft_shell_workspace(&mut self, workspace_directory: &str) {
        let should_refresh_draft = matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation)
                if !conversation.has_active_thread() && conversation.cwd != workspace_directory
        );
        if !should_refresh_draft {
            return;
        }

        self.dispatch_followup_controls(FollowupControlEvent::DraftWorkspaceSynced {
            workspace_directory: workspace_directory.to_string(),
            template_load_result: self.load_followup_template_catalog(workspace_directory),
        });
    }

    fn show_startup_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::StartupOverlayShown);
    }

    fn show_session_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::SessionsOverlayShown {
            limit: SESSION_PAGE_SIZE,
        });
    }

    fn show_followup_template_overlay(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::OverlayShown {
            stop_keyword: self.current_stop_keyword_value(),
        });
        self.dispatch_shell_chrome(ShellChromeEvent::FollowupTemplatesOverlayShown);
    }

    fn toggle_startup_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::StartupOverlayToggled);
    }

    fn toggle_session_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::SessionsOverlayToggled {
            limit: SESSION_PAGE_SIZE,
        });
    }

    fn toggle_followup_template_overlay(&mut self) {
        if self.shell_overlay != ShellOverlay::FollowupTemplates {
            self.show_followup_template_overlay();
            return;
        }
        self.dispatch_shell_chrome(ShellChromeEvent::FollowupTemplatesOverlayToggled);
    }

    fn close_shell_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::OverlayClosed);
    }

    fn open_new_conversation_shell(&mut self) {
        self.dispatch_conversation_intent(ConversationIntentEvent::NewDraftRequested);
    }

    fn execute_inline_shell_command(&mut self, command: InlineShellCommand) {
        match command {
            InlineShellCommand::Diagnostics => self.show_startup_overlay(),
            InlineShellCommand::Sessions => self.show_session_overlay(),
            InlineShellCommand::Templates => self.show_followup_template_overlay(),
            InlineShellCommand::NewDraft => self.open_new_conversation_shell(),
            InlineShellCommand::Help => {}
        }

        if let Some(status_text) = command.execution_status() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: status_text.to_string(),
            });
        }
        self.clear_input_buffer();
    }

    fn current_session(&self) -> Option<&SessionSummary> {
        match &self.session_state {
            SessionState::Ready(recent_sessions) => {
                recent_sessions.items.get(self.selected_session_index)
            }
            _ => None,
        }
    }

    fn open_conversation_shell(&mut self) {
        self.dispatch_conversation_intent(ConversationIntentEvent::SessionOpenRequested {
            session: self.current_session().cloned(),
        });
    }

    fn move_selection(&mut self, delta: isize) {
        self.dispatch_shell_chrome(ShellChromeEvent::SessionSelectionMoved { delta });
    }

    fn conversation_can_accept_input(&self) -> bool {
        matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation) if conversation.can_submit_prompt()
        )
    }

    fn push_input_character(&mut self, character: char) {
        self.dispatch_conversation_input(ConversationInputEvent::CharacterTyped { character });
    }

    fn insert_input_newline(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::NewlineInserted);
    }

    fn pop_input_character(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::BackspacePressed);
    }

    fn toggle_auto_followup(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::AutoFollowToggled);
    }

    fn current_stop_keyword_value(&self) -> String {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => conversation
                .auto_follow_state
                .stop_keyword_value()
                .to_string(),
            ConversationState::Loading | ConversationState::Failed(_) => {
                DEFAULT_AUTO_FOLLOW_STOP_KEYWORD.to_string()
            }
        }
    }

    fn is_stop_keyword_editing(&self) -> bool {
        self.followup_overlay_ui_state
            .stop_keyword_editor
            .is_editing
    }

    fn start_stop_keyword_edit(&mut self) {
        if !matches!(self.conversation_state, ConversationState::Ready(_)) {
            return;
        }

        if self.shell_overlay != ShellOverlay::FollowupTemplates {
            self.show_followup_template_overlay();
        }

        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordEditStarted {
            current_value: self.current_stop_keyword_value(),
        });
    }

    fn save_stop_keyword_edit(&mut self) {
        if !self.is_stop_keyword_editing() {
            return;
        }

        self.dispatch_followup_controls(FollowupControlEvent::StopKeywordValueUpdated {
            value: self
                .followup_overlay_ui_state
                .stop_keyword_editor
                .buffer
                .clone(),
        });
    }

    fn cancel_stop_keyword_edit(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordEditCanceled {
            current_value: self.current_stop_keyword_value(),
        });
    }

    fn push_stop_keyword_character(&mut self, character: char) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordCharacterTyped {
            character,
        });
    }

    fn pop_stop_keyword_character(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordBackspacePressed);
    }

    fn toggle_stop_keyword(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::StopKeywordToggled);
    }

    fn toggle_no_file_change_stop(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::NoFileChangeStopToggled);
    }

    fn reset_transcript_viewport(&mut self) {
        self.transcript_viewport_state = TranscriptViewportState::default();
    }

    fn sync_transcript_viewport_metrics(
        &mut self,
        max_scroll_offset: u16,
        visible_height: u16,
    ) -> u16 {
        self.transcript_viewport_state
            .sync_metrics(max_scroll_offset, visible_height);
        self.transcript_viewport_state.current_scroll_offset()
    }

    fn transcript_viewport_status_label(&self) -> String {
        self.transcript_viewport_state.status_label()
    }

    fn scroll_transcript_page_up(&mut self) {
        self.transcript_viewport_state.scroll_page_up();
    }

    fn scroll_transcript_page_down(&mut self) {
        self.transcript_viewport_state.scroll_page_down();
    }

    fn scroll_transcript_to_top(&mut self) {
        self.transcript_viewport_state.scroll_to_top();
    }

    fn scroll_transcript_to_tail(&mut self) {
        self.transcript_viewport_state.scroll_to_tail();
    }

    fn cycle_auto_followup_template(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::TemplateCycledForward);
    }

    fn cycle_auto_followup_template_backward(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::TemplateCycledBackward);
    }

    #[cfg(test)]
    fn followup_template_selection(&self) -> Option<usize> {
        match &self.conversation_state {
            ConversationState::Ready(conversation)
                if !conversation
                    .auto_follow_state
                    .template_state
                    .items
                    .is_empty() =>
            {
                Some(conversation.auto_follow_state.selected_template_index())
            }
            _ => None,
        }
    }

    fn scroll_followup_template_preview(&mut self, delta: i32) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::PreviewScrolled { delta });
    }

    fn current_workspace_directory(&self) -> String {
        match &self.startup_state {
            StartupState::Ready(diagnostics) => diagnostics.workspace_path.clone(),
            _ => std::env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| ".".to_string()),
        }
    }

    fn load_followup_template_catalog(
        &self,
        workspace_directory: &str,
    ) -> FollowupTemplateCatalogLoadResult {
        self.followup_template_service
            .load_catalog(workspace_directory)
    }

    fn is_shell_overlay_visible(&self) -> bool {
        self.shell_overlay != ShellOverlay::Hidden
    }

    fn is_exit_confirmation_visible(&self) -> bool {
        self.exit_confirmation_state == ExitConfirmationState::Visible
    }

    fn handle_exit_confirmation_key(&mut self, key: event::KeyEvent) -> Option<bool> {
        if !self.is_exit_confirmation_visible() {
            return None;
        }

        if !key.modifiers.is_empty() && key.modifiers != KeyModifiers::SHIFT {
            return None;
        }

        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationHidden);
                Some(false)
            }
            _ => Some(false),
        }
    }

    fn handle_stop_keyword_editor_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::FollowupTemplates || !self.is_stop_keyword_editing()
        {
            return false;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => self.save_stop_keyword_edit(),
            KeyCode::Esc => self.cancel_stop_keyword_edit(),
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.cancel_stop_keyword_edit()
            }
            KeyCode::Backspace => self.pop_stop_keyword_character(),
            KeyCode::Char(character)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.push_stop_keyword_character(character);
            }
            _ => {}
        }

        true
    }

    fn handle_shell_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay == ShellOverlay::Hidden {
            return false;
        }
        let is_startup_overlay = self.shell_overlay == ShellOverlay::Startup;

        if self.handle_stop_keyword_editor_key(key) {
            return true;
        }

        if key.code == KeyCode::Esc
            || (key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c'))
        {
            self.close_shell_overlay();
            return true;
        }

        if is_startup_overlay {
            match key.code {
                KeyCode::Char('r') if key.modifiers.is_empty() => {
                    self.dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested)
                }
                KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                    self.show_session_overlay()
                }
                _ => {}
            }
            return true;
        }

        if self.shell_overlay == ShellOverlay::FollowupTemplates {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                    self.cycle_auto_followup_template_backward()
                }
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                    self.cycle_auto_followup_template()
                }
                KeyCode::Char('f') if key.modifiers == KeyModifiers::CONTROL => {
                    self.cycle_auto_followup_template()
                }
                KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                    self.toggle_auto_followup()
                }
                KeyCode::Char('g') if key.modifiers == KeyModifiers::CONTROL => {
                    self.start_stop_keyword_edit()
                }
                KeyCode::Char('k') if key.modifiers == KeyModifiers::CONTROL => {
                    self.toggle_stop_keyword()
                }
                KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => {
                    self.toggle_no_file_change_stop()
                }
                KeyCode::PageUp if key.modifiers.is_empty() => self
                    .scroll_followup_template_preview(
                        -(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32),
                    ),
                KeyCode::PageDown if key.modifiers.is_empty() => self
                    .scroll_followup_template_preview(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32),
                KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => self
                    .scroll_followup_template_preview(
                        -(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32),
                    ),
                KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => self
                    .scroll_followup_template_preview(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32),
                KeyCode::Enter if key.modifiers.is_empty() => self.close_shell_overlay(),
                _ => {}
            }
            return true;
        }

        match key.code {
            KeyCode::Char('r') if key.modifiers.is_empty() => {
                if self.can_open_session_list() {
                    self.dispatch_shell_chrome(ShellChromeEvent::SessionsRequested {
                        limit: SESSION_PAGE_SIZE,
                    });
                }
            }
            KeyCode::Char('n') if key.modifiers.is_empty() => {
                self.open_new_conversation_shell();
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.move_selection(1)
            }
            KeyCode::Enter if key.modifiers.is_empty() => self.open_conversation_shell(),
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                self.show_startup_overlay()
            }
            _ => {}
        }
        true
    }

    fn handle_ctrl_c(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationHidden);

        if self.is_shell_overlay_visible() {
            self.close_shell_overlay();
            return;
        }

        self.dispatch_conversation_intent(ConversationIntentEvent::CtrlCPressed);
    }
}

fn run_tui(mut app: NativeTuiApp) -> Result<()> {
    let presentation_mode = TuiPresentationMode::from_environment();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if presentation_mode.uses_alternate_screen() {
        execute!(stdout, EnterAlternateScreen)?;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    if presentation_mode.uses_alternate_screen() {
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    } else {
        execute!(terminal.backend_mut(), MoveToNextLine(1))?;
    }
    terminal.show_cursor()?;
    result
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut NativeTuiApp,
) -> Result<()> {
    let mut should_quit = false;

    while !should_quit {
        app.poll_background_messages();
        terminal.draw(|frame| draw(frame, app))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if let Some(confirmed_exit) = app.handle_exit_confirmation_key(key) {
            if confirmed_exit {
                should_quit = true;
            }
            continue;
        }

        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('q') {
            should_quit = true;
            continue;
        }

        if app.handle_shell_overlay_key(key) {
            continue;
        }

        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
            app.handle_ctrl_c();
            continue;
        }

        match key.code {
            KeyCode::PageUp if key.modifiers.is_empty() => app.scroll_transcript_page_up(),
            KeyCode::PageDown if key.modifiers.is_empty() => app.scroll_transcript_page_down(),
            KeyCode::Home if key.modifiers.is_empty() => app.scroll_transcript_to_top(),
            KeyCode::End if key.modifiers.is_empty() => app.scroll_transcript_to_tail(),
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                app.toggle_auto_followup()
            }
            KeyCode::Char('g') if key.modifiers == KeyModifiers::CONTROL => {
                app.start_stop_keyword_edit()
            }
            KeyCode::Char('f') if key.modifiers == KeyModifiers::CONTROL => {
                app.cycle_auto_followup_template()
            }
            KeyCode::Char('k') if key.modifiers == KeyModifiers::CONTROL => {
                app.toggle_stop_keyword()
            }
            KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => {
                app.toggle_no_file_change_stop()
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                app.toggle_startup_overlay()
            }
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                app.toggle_session_overlay()
            }
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                app.toggle_followup_template_overlay()
            }
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => {
                app.dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested)
            }
            KeyCode::Char('t') if key.modifiers == KeyModifiers::CONTROL => {
                app.open_new_conversation_shell()
            }
            KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => {
                app.insert_input_newline()
            }
            KeyCode::Backspace => app.pop_input_character(),
            KeyCode::Enter if app.conversation_can_accept_input() => app.start_turn_submission(),
            KeyCode::Enter => app.start_turn_submission(),
            KeyCode::Char(character)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                app.push_input_character(character);
            }
            _ => {}
        }
    }

    Ok(())
}

fn build_followup_template_preview_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Loading => vec![Line::from("conversation is still loading")],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
        ConversationState::Ready(conversation) => {
            let template = conversation.auto_follow_state.selected_template();
            let preview_thread_id = if conversation.thread_id.trim().is_empty() {
                "draft-thread"
            } else {
                conversation.thread_id.as_str()
            };
            let latest_agent_message = conversation.latest_agent_message_text();
            let rendered_preview = conversation
                .auto_follow_state
                .render_prompt_preview(&conversation.thread_id, latest_agent_message);

            let mut lines = vec![
                Line::from(format!("selected: {}", template.label)),
                Line::from(format!("source: {}", template.source_label())),
                Line::from(format!("preview thread id: {preview_thread_id}")),
            ];

            if latest_agent_message.is_some() {
                lines.push(Line::from(
                    "preview last_message: using the latest non-empty agent reply",
                ));
            } else {
                lines.push(Line::from(
                    "preview last_message: placeholder until an agent reply exists",
                ));
            }

            lines.push(Line::from(""));
            lines.push(Line::from("Raw Template"));
            for body_line in template.body.lines() {
                lines.push(Line::from(body_line.to_string()));
            }

            lines.push(Line::from(""));
            lines.push(Line::from("Rendered Preview"));
            for preview_line in rendered_preview.lines() {
                lines.push(Line::from(preview_line.to_string()));
            }

            lines
        }
    }
}

fn build_followup_template_status_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Loading => vec![Line::from("conversation is still loading")],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
        ConversationState::Ready(conversation) => {
            let mut lines = vec![
                Line::from(format!(
                    "auto follow-up: {}",
                    conversation.auto_follow_state.status_label()
                )),
                Line::from(format!(
                    "progress: {}",
                    conversation.auto_follow_state.progress_label()
                )),
                Line::from(format!(
                    "stop keyword: {}",
                    conversation.auto_follow_state.stop_keyword_label()
                )),
                Line::from(format!(
                    "stop on no-file-change: {}",
                    conversation.auto_follow_state.no_file_change_stop_label()
                )),
                Line::from(format!(
                    "last turn file changes: {}",
                    conversation
                        .turn_activity
                        .last_completed_file_change_count()
                )),
            ];

            if app.is_stop_keyword_editing() {
                lines.push(Line::from(format!(
                    "editing stop keyword: {}",
                    app.followup_overlay_ui_state.stop_keyword_editor.buffer
                )));
                lines.push(Line::from("save with Enter or cancel with Esc/Ctrl+C"));
            } else {
                lines.push(Line::from("stop keyword edit: press Ctrl+g"));
            }
            lines.push(Line::from(Span::styled(
                format!("status: {}", conversation.status_text),
                Style::default().fg(Color::Yellow),
            )));

            lines
        }
    }
}

fn build_followup_template_key_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    if app.is_stop_keyword_editing() {
        return vec![
            Line::from("Type the new stop keyword directly. Backspace deletes."),
            Line::from("Enter: save stop keyword    Esc/Ctrl+C: cancel edit"),
            Line::from("Use letters, numbers, or underscores only."),
        ];
    }

    vec![
        Line::from("Up/Down or j/k: change template    Ctrl+f: next template"),
        Line::from("PageUp/PageDown or Ctrl+u/Ctrl+d: scroll preview"),
        Line::from("Ctrl+a: auto on/off    Ctrl+g: edit stop keyword"),
        Line::from("Ctrl+k: stop rule on/off    Ctrl+n: no-file stop    Enter/Esc/Ctrl+C: close"),
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use anyhow::Result;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{
        AutoFollowState, AutoFollowupDecision, AutoFollowupSkipReason, ConversationInputState,
        ConversationMessage, ConversationMessageKind, ConversationRuntimeEvent, ConversationState,
        ConversationViewModel, DEFAULT_AUTO_FOLLOW_STOP_KEYWORD,
        FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP, InlineShellCommand, MAX_COMPOSER_HEIGHT,
        NativeTuiApp, PromptOrigin, RecordedAutoFollowupSkip, ShellActionAvailability,
        ShellOverlay, StartupState, TuiPresentationMode, TurnActivityState,
        build_followup_template_preview_lines, build_followup_template_status_lines,
        build_input_title, build_ready_input_lines, build_shell_footer_lines, build_status_title,
        build_transcript_title, format_conversation_lines, shell_layout,
    };
    use crate::application::port::outbound::codex_app_server_port::{
        AppServerStartupContext, CodexAppServerPort,
    };
    use crate::application::port::outbound::followup_template_port::{
        FollowupTemplatePort, WorkspaceFollowupTemplateRecord,
    };
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::followup_template_service::FollowupTemplateService;
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};
    use crate::domain::followup_template::{
        FollowupTemplateCatalog, FollowupTemplateDefinition, FollowupTemplateSource,
    };
    use crate::domain::recent_sessions::RecentSessions;

    #[derive(Default)]
    struct FakeCodexAppServerPort {
        new_thread_calls: Mutex<Vec<(String, String)>>,
        turn_calls: Mutex<Vec<(String, String)>>,
    }

    impl CodexAppServerPort for FakeCodexAppServerPort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            Ok(AppServerStartupContext {
                initialize_detail: "ok".to_string(),
                account_detail: "ok".to_string(),
                account_ok: true,
                warnings: Vec::new(),
            })
        }

        fn load_recent_sessions(&self, _limit: usize) -> Result<RecentSessions> {
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            })
        }

        fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
            Ok(ConversationSnapshot {
                thread_id: thread_id.to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/root".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
            })
        }

        fn run_new_thread_stream(
            &self,
            cwd: &str,
            prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            self.new_thread_calls
                .lock()
                .expect("new-thread call mutex poisoned")
                .push((cwd.to_string(), prompt.to_string()));
            Ok(())
        }

        fn run_turn_stream(
            &self,
            thread_id: &str,
            prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            self.turn_calls
                .lock()
                .expect("turn call mutex poisoned")
                .push((thread_id.to_string(), prompt.to_string()));
            Ok(())
        }
    }

    struct FakeFollowupTemplatePort;

    impl FollowupTemplatePort for FakeFollowupTemplatePort {
        fn load_workspace_templates(
            &self,
            workspace_dir: &str,
        ) -> Result<Vec<WorkspaceFollowupTemplateRecord>> {
            if workspace_dir == "/tmp/root" {
                return Ok(vec![WorkspaceFollowupTemplateRecord {
                    name: "root-template".to_string(),
                    path: "/tmp/root/.codex-exec-loop/followups/root-template.md".to_string(),
                    body: "workspace template body".to_string(),
                }]);
            }

            Ok(Vec::new())
        }
    }

    fn make_test_app() -> (NativeTuiApp, Arc<FakeCodexAppServerPort>) {
        let codex_port = Arc::new(FakeCodexAppServerPort::default());
        let followup_port = Arc::new(FakeFollowupTemplatePort);
        let app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port.clone()),
            FollowupTemplateService::new(followup_port),
        );

        (app, codex_port)
    }

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
            warnings: Vec::new(),
            input_buffer: String::new(),
            active_turn_id: None,
            input_state: ConversationInputState::ReadyToContinue,
            auto_follow_state: AutoFollowState::new(sample_template_catalog()),
            turn_activity: TurnActivityState::default(),
            last_auto_followup_skip: None,
            status_text: "thread loaded".to_string(),
        }
    }

    #[test]
    fn running_turn_still_shows_buffered_prompt() {
        let mut conversation = ready_conversation();
        conversation.input_state = ConversationInputState::StreamingTurn;
        conversation.input_buffer = "Continue from the last change.".to_string();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Continue from the last change."));
        assert!(rendered.contains("Ctrl+j inserts a new line"));
    }

    #[test]
    fn empty_existing_session_prompts_for_next_message() {
        let conversation = ready_conversation();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Ready to continue this session."));
        assert!(rendered.contains("Ctrl+j for newline"));
        assert!(rendered.contains("Shell commands: :diag"));
    }

    #[test]
    fn empty_draft_prompts_for_first_message() {
        let mut conversation = ready_conversation();
        conversation.thread_id.clear();
        conversation.input_state = ConversationInputState::DraftReady;

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Ready to start a new thread."));
        assert!(rendered.contains("Ctrl+j for newline"));
    }

    #[test]
    fn multiline_buffer_renders_as_multiple_input_lines() {
        let mut conversation = ready_conversation();
        conversation.input_buffer = "first line\nsecond line".to_string();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line == "first line"));
        assert!(rendered.iter().any(|line| line == "second line"));
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("Ctrl+j inserts a new line"))
        );
    }

    #[test]
    fn inline_shell_command_buffer_shows_command_hint() {
        let mut conversation = ready_conversation();
        conversation.input_buffer = ":templates".to_string();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(":templates"));
        assert!(rendered.contains("Press Enter to open the template overlay."));
    }

    #[test]
    fn tui_presentation_mode_defaults_to_main_screen() {
        assert_eq!(
            TuiPresentationMode::from_env_value(None),
            TuiPresentationMode::MainScreen
        );
        assert_eq!(
            TuiPresentationMode::from_env_value(Some("0")),
            TuiPresentationMode::MainScreen
        );
        assert_eq!(
            TuiPresentationMode::from_env_value(Some("no")),
            TuiPresentationMode::MainScreen
        );
    }

    #[test]
    fn tui_presentation_mode_accepts_truthy_alt_screen_flag() {
        assert_eq!(
            TuiPresentationMode::from_env_value(Some("1")),
            TuiPresentationMode::AlternateScreen
        );
        assert_eq!(
            TuiPresentationMode::from_env_value(Some(" true ")),
            TuiPresentationMode::AlternateScreen
        );
        assert_eq!(
            TuiPresentationMode::from_env_value(Some("ON")),
            TuiPresentationMode::AlternateScreen
        );
    }

    #[test]
    fn tui_presentation_mode_ignores_unrecognized_flag_values() {
        assert_eq!(
            TuiPresentationMode::from_env_value(Some("maybe")),
            TuiPresentationMode::MainScreen
        );
    }

    #[test]
    fn multiline_buffer_expands_composer_height() {
        let mut conversation = ready_conversation();
        conversation.input_buffer = "one\ntwo\nthree\nfour\nfive\nsix".to_string();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready);

        assert_eq!(
            shell_layout::build_input_block_height(&rendered),
            MAX_COMPOSER_HEIGHT
        );
    }

    #[test]
    fn status_footer_height_expands_for_ready_shell_summary() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());

        let rendered = build_shell_footer_lines(&app);

        assert_eq!(shell_layout::build_shell_footer_height(&rendered), 7);
    }

    #[test]
    fn startup_pending_prompts_wait_before_send() {
        let conversation = ready_conversation();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Pending)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Startup checks are still running."));
        assert!(rendered.contains("send once diagnostics turn ready"));
    }

    #[test]
    fn startup_blocked_prompt_guides_user_to_diagnostics_overlay() {
        let conversation = ready_conversation();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Blocked)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Startup diagnostics need attention."));
        assert!(rendered.contains("Open Ctrl+d"));
    }

    #[test]
    fn draft_workspace_sync_preserves_buffered_input() {
        let (mut app, _) = make_test_app();

        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should start with a draft conversation");
        };
        conversation.cwd = "/tmp/subdir".to_string();
        conversation.input_buffer = "buffered prompt".to_string();

        app.sync_draft_shell_workspace("/tmp/root");

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("draft conversation should still be ready");
        };
        assert_eq!(conversation.cwd, "/tmp/root");
        assert_eq!(conversation.input_buffer, "buffered prompt");
        assert_eq!(conversation.auto_follow_state.template_count(), 5);
        assert!(conversation.status_text.contains("draft workspace synced"));
    }

    #[test]
    fn opening_new_draft_is_blocked_while_turn_is_streaming() {
        let (mut app, _) = make_test_app();

        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should start with a draft conversation");
        };
        conversation.thread_id = "thread-123".to_string();
        conversation.title = "Streaming thread".to_string();
        conversation.input_state = ConversationInputState::StreamingTurn;

        app.open_new_conversation_shell();

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert_eq!(conversation.thread_id, "thread-123");
        assert_eq!(conversation.title, "Streaming thread");
        assert_eq!(
            conversation.input_state,
            ConversationInputState::StreamingTurn
        );
        assert!(conversation.status_text.contains("turn still running"));
    }

    #[test]
    fn auto_follow_submission_respects_startup_gate() {
        let (mut app, codex_port) = make_test_app();
        app.startup_state = StartupState::Loading;

        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should start with a draft conversation");
        };
        conversation.thread_id = "thread-123".to_string();
        conversation.input_state = ConversationInputState::ReadyToContinue;

        app.submit_prompt("continue working".to_string(), PromptOrigin::AutoFollow);

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert!(
            codex_port
                .turn_calls
                .lock()
                .expect("turn call mutex poisoned")
                .is_empty()
        );
        assert!(conversation.status_text.contains("auto follow-up paused"));
    }

    #[test]
    fn inline_diag_command_opens_overlay_and_clears_input() {
        let (mut app, codex_port) = make_test_app();
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should start with a ready conversation");
        };
        conversation.input_buffer = ":diag".to_string();

        app.start_turn_submission();

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert_eq!(app.shell_overlay, ShellOverlay::Startup);
        assert!(conversation.input_buffer.is_empty());
        assert!(
            conversation
                .status_text
                .contains("opened diagnostics overlay")
        );
        assert!(
            codex_port
                .new_thread_calls
                .lock()
                .expect("new-thread call mutex poisoned")
                .is_empty()
        );
        assert!(
            codex_port
                .turn_calls
                .lock()
                .expect("turn call mutex poisoned")
                .is_empty()
        );
    }

    #[test]
    fn inline_templates_command_opens_overlay_while_turn_is_streaming() {
        let (mut app, codex_port) = make_test_app();
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should start with a ready conversation");
        };
        conversation.input_state = ConversationInputState::StreamingTurn;
        conversation.input_buffer = ":templates".to_string();

        app.start_turn_submission();

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert_eq!(app.shell_overlay, ShellOverlay::FollowupTemplates);
        assert_eq!(
            conversation.input_state,
            ConversationInputState::StreamingTurn
        );
        assert!(conversation.input_buffer.is_empty());
        assert!(
            codex_port
                .turn_calls
                .lock()
                .expect("turn call mutex poisoned")
                .is_empty()
        );
    }

    #[test]
    fn inline_help_command_updates_status_and_clears_input() {
        let (mut app, _) = make_test_app();
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should start with a ready conversation");
        };
        conversation.input_buffer = ":help".to_string();

        app.start_turn_submission();

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert!(conversation.input_buffer.is_empty());
        assert!(
            conversation
                .status_text
                .contains(InlineShellCommand::command_list_line())
        );
    }

    #[test]
    fn transcript_title_includes_transcript_viewport_status() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.sync_transcript_viewport_metrics(18, 6);
        app.scroll_transcript_page_up();

        assert_eq!(
            build_transcript_title(&app).to_string(),
            "Transcript / manual 13/18 / PageUp PageDown / Home End"
        );
    }

    #[test]
    fn composer_title_includes_submit_and_newline_hints() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());

        let rendered = build_input_title(&app).to_string();

        assert!(rendered.contains("Composer / ready"));
        assert!(rendered.contains("Enter send"));
        assert!(rendered.contains("Ctrl+j newline"));
    }

    #[test]
    fn composer_title_shows_readiness_gated_submit_hint() {
        let (mut app, _) = make_test_app();
        app.startup_state = StartupState::Loading;
        app.conversation_state = ConversationState::Ready(ready_conversation());

        let rendered = build_input_title(&app).to_string();

        assert!(rendered.contains("Enter send when ready"));
    }

    #[test]
    fn status_title_surfaces_overlay_and_followup_controls() {
        let rendered = build_status_title().to_string();

        assert!(rendered.contains("Ctrl+o sessions"));
        assert!(rendered.contains("Ctrl+d diag"));
        assert!(rendered.contains("Ctrl+p templ"));
        assert!(rendered.contains("Ctrl+a auto"));
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
    fn followup_template_preview_renders_selected_template_and_runtime_values() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));
        app.conversation_state = ConversationState::Ready(conversation);

        let rendered = build_followup_template_preview_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("selected: builtin next-task"));
        assert!(rendered.contains("preview thread id: thread-1"));
        assert!(rendered.contains("latest answer"));
        assert!(rendered.contains("AUTO_STOP"));
        assert!(rendered.contains("Rendered Preview"));
    }

    #[test]
    fn followup_template_preview_uses_placeholder_without_agent_reply() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());

        let rendered = build_followup_template_preview_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("preview last_message: placeholder"));
        assert!(rendered.contains("(waiting for next agent reply)"));
    }

    #[test]
    fn followup_template_overlay_navigation_updates_selection() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.show_followup_template_overlay();

        assert_eq!(app.shell_overlay, ShellOverlay::FollowupTemplates);
        assert_eq!(app.followup_template_selection(), Some(0));

        assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE,)));

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should stay ready");
        };
        assert_eq!(
            conversation.auto_follow_state.template_label(),
            "builtin plan-queue"
        );
        assert!(conversation.status_text.contains("auto follow-up template"));
        assert_eq!(app.followup_template_selection(), Some(1));
        assert_eq!(app.followup_overlay_ui_state.preview_scroll, 0);
    }

    #[test]
    fn followup_template_overlay_enter_closes_overlay() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.show_followup_template_overlay();

        assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
    }

    #[test]
    fn followup_template_overlay_scroll_keys_update_preview_offset() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.show_followup_template_overlay();

        assert!(
            app.handle_shell_overlay_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,))
        );
        assert_eq!(
            app.followup_overlay_ui_state.preview_scroll,
            FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP
        );

        assert!(
            app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL,))
        );
        assert_eq!(
            app.followup_overlay_ui_state.preview_scroll,
            FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP.saturating_mul(2)
        );

        assert!(
            app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL,))
        );
        assert_eq!(
            app.followup_overlay_ui_state.preview_scroll,
            FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP
        );
    }

    #[test]
    fn ctrl_g_starts_stop_keyword_edit_in_followup_overlay() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());

        app.start_stop_keyword_edit();

        assert_eq!(app.shell_overlay, ShellOverlay::FollowupTemplates);
        assert!(app.is_stop_keyword_editing());
        assert_eq!(
            app.followup_overlay_ui_state.stop_keyword_editor.buffer,
            DEFAULT_AUTO_FOLLOW_STOP_KEYWORD
        );
    }

    #[test]
    fn stop_keyword_edit_commit_updates_saved_value_and_preview() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));
        app.conversation_state = ConversationState::Ready(conversation);
        app.start_stop_keyword_edit();
        app.followup_overlay_ui_state.stop_keyword_editor.buffer = "DONE".to_string();

        assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should stay ready");
        };
        assert_eq!(conversation.auto_follow_state.stop_keyword_value(), "DONE");
        assert!(!app.is_stop_keyword_editing());

        let rendered = build_followup_template_preview_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("DONE"));
    }

    #[test]
    fn invalid_stop_keyword_edit_keeps_editor_open() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.start_stop_keyword_edit();
        app.followup_overlay_ui_state.stop_keyword_editor.buffer = "two words".to_string();

        assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should stay ready");
        };
        assert_eq!(
            conversation.auto_follow_state.stop_keyword_value(),
            DEFAULT_AUTO_FOLLOW_STOP_KEYWORD
        );
        assert!(app.is_stop_keyword_editing());
        assert!(
            conversation
                .status_text
                .contains("letters, numbers, or underscores")
        );
    }

    #[test]
    fn followup_template_status_lines_include_latest_status_text() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation.status_text =
            "auto stop keyword must use only letters, numbers, or underscores".to_string();
        app.conversation_state = ConversationState::Ready(conversation);

        let rendered = build_followup_template_status_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            rendered.contains(
                "status: auto stop keyword must use only letters, numbers, or underscores"
            )
        );
    }

    #[test]
    fn stop_keyword_edit_cancel_restores_saved_value() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.start_stop_keyword_edit();
        app.followup_overlay_ui_state.stop_keyword_editor.buffer = "DONE".to_string();

        assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)));

        assert!(!app.is_stop_keyword_editing());
        assert_eq!(
            app.followup_overlay_ui_state.stop_keyword_editor.buffer,
            DEFAULT_AUTO_FOLLOW_STOP_KEYWORD
        );
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

    #[test]
    fn auto_followup_skip_reason_is_visible_in_status_footer() {
        let (mut app, _) = make_test_app();
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
        app.conversation_state = ConversationState::Ready(conversation);

        app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
            ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-1".to_string(),
            },
        ));

        let rendered = build_shell_footer_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("last skip: no file changes"));
        assert!(rendered.contains("detail: the last completed turn changed 0 files"));
    }

    #[test]
    fn auto_followup_queue_clears_previous_skip_reason_from_status_footer() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation.last_auto_followup_skip = Some(RecordedAutoFollowupSkip {
            reason: AutoFollowupSkipReason::Disabled,
            detail: "auto follow-up is off; toggle Ctrl+a to re-enable it".to_string(),
        });
        conversation
            .turn_activity
            .last_completed_turn_file_change_count = 2;
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));
        app.conversation_state = ConversationState::Ready(conversation);

        app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
            ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-2".to_string(),
            },
        ));

        let rendered = build_shell_footer_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("last skip: none"));
        assert!(rendered.contains("detail: none"));
    }

    #[test]
    fn recorded_limit_skip_detail_stays_stable_after_progress_resets() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation.auto_follow_state.completed_auto_turns =
            conversation.auto_follow_state.max_auto_turns;
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));
        app.conversation_state = ConversationState::Ready(conversation);

        app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
            ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-limit".to_string(),
            },
        ));

        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("conversation should remain ready");
        };
        conversation.auto_follow_state.completed_auto_turns = 0;

        let rendered = build_shell_footer_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("last skip: turn limit reached"));
        assert!(rendered.contains("detail: reached the configured auto-turn budget (3/3)"));
        assert!(!rendered.contains("detail: reached the configured auto-turn budget (0/3)"));
    }
}

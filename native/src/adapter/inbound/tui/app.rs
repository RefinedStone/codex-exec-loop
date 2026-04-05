use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::adapter::outbound::codex_app_server_adapter::CodexAppServerAdapter;
use crate::application::port::outbound::codex_app_server_port::CodexAppServerPort;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::{
    ConversationMessage, ConversationMessageKind, ConversationSnapshot, ConversationStreamEvent,
};
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;
use crate::domain::startup_diagnostics::StartupDiagnostics;

const SESSION_PAGE_SIZE: usize = 10;
const MAX_CONVERSATION_HISTORY_LINES: usize = 160;
const DEFAULT_AUTO_FOLLOW_MAX_TURNS: usize = 3;
const AUTO_FOLLOW_TEMPLATE_NEXT_TASK: &str = r#"대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

방금 결과를 기준으로 다음 작업 1개만 이어서 진행하세요.

직전 답변:
{last_message}"#;
const AUTO_FOLLOW_TEMPLATE_PLAN_QUEUE: &str = r#"대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

방금 결과를 바탕으로 개선점과 다음 작업 후보를 `plan_priority_queue.md` 에 정리하고,
가장 우선순위가 높은 항목 1개를 바로 진행하세요.

직전 답변:
{last_message}"#;
const AUTO_FOLLOW_TEMPLATE_BUGFIX: &str = r#"대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

직전 결과 기준으로 아직 남아 있는 버그나 리스크 1개만 골라 수정하세요.
수정이 끝나면 무엇을 고쳤는지 짧게 요약하세요.

직전 답변:
{last_message}"#;
const AUTO_FOLLOW_TEMPLATE_DOCS: &str = r#"대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

방금 작업을 기준으로 README 또는 사용자 문서에 빠진 내용 1개만 보강하세요.

직전 답변:
{last_message}"#;

pub fn run() -> Result<()> {
    let codex_app_server_port: Arc<dyn CodexAppServerPort> = Arc::new(CodexAppServerAdapter::new(
        "codex-exec-loop-native",
        env!("CARGO_PKG_VERSION"),
    ));
    let startup_service = StartupService::new(codex_app_server_port.clone());
    let session_service = SessionService::new(codex_app_server_port.clone());
    let conversation_service = ConversationService::new(codex_app_server_port);

    let mut app = NativeTuiApp::new(startup_service, session_service, conversation_service);
    app.start_startup_check();
    run_tui(app)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    SessionList,
    ConversationShell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitConfirmationState {
    Hidden,
    Visible,
}

#[derive(Debug, Clone)]
enum StartupState {
    Idle,
    Loading,
    Ready(StartupDiagnostics),
    Failed(String),
}

#[derive(Debug, Clone)]
enum SessionState {
    Idle,
    Loading,
    Ready(RecentSessions),
    Failed(String),
}

#[derive(Debug, Clone)]
enum ConversationState {
    Idle,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptOrigin {
    Manual,
    AutoFollow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AutoFollowupDecision {
    Disabled,
    ManualInputBuffered,
    QueuePrompt(String),
    LimitReached,
    NoAgentReply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoFollowTemplateKind {
    NextTask,
    PlanQueue,
    Bugfix,
    Docs,
}

impl AutoFollowTemplateKind {
    fn label(self) -> &'static str {
        match self {
            Self::NextTask => "builtin next-task",
            Self::PlanQueue => "builtin plan-queue",
            Self::Bugfix => "builtin bugfix",
            Self::Docs => "builtin docs",
        }
    }

    fn template(self) -> &'static str {
        match self {
            Self::NextTask => AUTO_FOLLOW_TEMPLATE_NEXT_TASK,
            Self::PlanQueue => AUTO_FOLLOW_TEMPLATE_PLAN_QUEUE,
            Self::Bugfix => AUTO_FOLLOW_TEMPLATE_BUGFIX,
            Self::Docs => AUTO_FOLLOW_TEMPLATE_DOCS,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::NextTask => Self::PlanQueue,
            Self::PlanQueue => Self::Bugfix,
            Self::Bugfix => Self::Docs,
            Self::Docs => Self::NextTask,
        }
    }
}

#[derive(Debug, Clone)]
struct AutoFollowState {
    enabled: bool,
    completed_auto_turns: usize,
    max_auto_turns: usize,
    template_kind: AutoFollowTemplateKind,
}

impl Default for AutoFollowState {
    fn default() -> Self {
        Self {
            enabled: true,
            completed_auto_turns: 0,
            max_auto_turns: DEFAULT_AUTO_FOLLOW_MAX_TURNS,
            template_kind: AutoFollowTemplateKind::NextTask,
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

    fn template_label(&self) -> &'static str {
        self.template_kind.label()
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

    fn cycle_template_kind(&mut self) {
        self.template_kind = self.template_kind.next();
    }

    fn render_prompt(&self, thread_id: &str, last_message: &str) -> String {
        self.template_kind
            .template()
            .replace("{auto_turn}", &self.next_auto_turn_index().to_string())
            .replace("{max_auto_turns}", &self.max_auto_turns.to_string())
            .replace("{session_id}", thread_id)
            .replace("{last_message}", last_message)
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
    status_text: String,
}

impl ConversationViewModel {
    fn new_draft(cwd: String) -> Self {
        let mut view_model = Self {
            thread_id: String::new(),
            title: "New conversation".to_string(),
            cwd,
            messages: Vec::new(),
            cached_conversation_lines: Vec::new(),
            warnings: Vec::new(),
            input_buffer: String::new(),
            active_turn_id: None,
            input_state: ConversationInputState::DraftReady,
            auto_follow_state: AutoFollowState::default(),
            status_text: "new thread draft".to_string(),
        };
        view_model.refresh_conversation_lines();
        view_model
    }

    fn from_snapshot(snapshot: ConversationSnapshot) -> Self {
        let status_text = if snapshot.warnings.is_empty() {
            "thread loaded".to_string()
        } else {
            snapshot.warnings.join(" | ")
        };

        let mut view_model = Self {
            thread_id: snapshot.thread_id,
            title: snapshot.title,
            cwd: snapshot.cwd,
            messages: snapshot.messages,
            cached_conversation_lines: Vec::new(),
            warnings: snapshot.warnings,
            input_buffer: String::new(),
            active_turn_id: None,
            input_state: ConversationInputState::ReadyToContinue,
            auto_follow_state: AutoFollowState::default(),
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
    }

    fn mark_turn_finished(&mut self) {
        self.active_turn_id = None;
        self.input_state = self.ready_input_state();
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
            (false, _, _, _) => AutoFollowupDecision::Disabled,
            (true, false, _, _) => AutoFollowupDecision::ManualInputBuffered,
            (true, true, false, _) => AutoFollowupDecision::LimitReached,
            (true, true, true, Some(last_message)) => AutoFollowupDecision::QueuePrompt(
                self.auto_follow_state
                    .render_prompt(&self.thread_id, last_message.trim()),
            ),
            (true, true, true, None) => AutoFollowupDecision::NoAgentReply,
        }
    }
}

struct NativeTuiApp {
    current_screen: Screen,
    conversation_return_screen: Screen,
    exit_confirmation_state: ExitConfirmationState,
    startup_state: StartupState,
    session_state: SessionState,
    conversation_state: ConversationState,
    selected_session_index: usize,
    active_session: Option<SessionSummary>,
    startup_service: StartupService,
    session_service: SessionService,
    conversation_service: ConversationService,
    tx: Sender<BackgroundMessage>,
    rx: Receiver<BackgroundMessage>,
}

impl NativeTuiApp {
    fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            current_screen: Screen::Home,
            conversation_return_screen: Screen::Home,
            exit_confirmation_state: ExitConfirmationState::Hidden,
            startup_state: StartupState::Idle,
            session_state: SessionState::Idle,
            conversation_state: ConversationState::Idle,
            selected_session_index: 0,
            active_session: None,
            startup_service,
            session_service,
            conversation_service,
            tx,
            rx,
        }
    }

    fn start_startup_check(&mut self) {
        self.startup_state = StartupState::Loading;
        let tx = self.tx.clone();
        let service = self.startup_service.clone();
        thread::spawn(move || {
            let result = service.run_checks().map_err(|error| error.to_string());
            let _ = tx.send(BackgroundMessage::StartupLoaded(result));
        });
    }

    fn start_session_load(&mut self) {
        self.session_state = SessionState::Loading;
        let tx = self.tx.clone();
        let service = self.session_service.clone();
        thread::spawn(move || {
            let result = service
                .load_recent_sessions(SESSION_PAGE_SIZE)
                .map_err(|error| error.to_string());
            let _ = tx.send(BackgroundMessage::SessionsLoaded(result));
        });
    }

    fn start_conversation_load(&mut self, thread_id: String) {
        self.conversation_state = ConversationState::Loading;
        let tx = self.tx.clone();
        let service = self.conversation_service.clone();
        thread::spawn(move || {
            let result = service
                .load_snapshot(&thread_id)
                .map_err(|error| error.to_string());
            let _ = tx.send(BackgroundMessage::ConversationLoaded(result));
        });
    }

    fn start_turn_submission(&mut self) {
        let prompt = match &self.conversation_state {
            ConversationState::Ready(conversation) => conversation.input_buffer.trim().to_string(),
            _ => return,
        };
        self.submit_prompt(prompt, PromptOrigin::Manual);
    }

    fn submit_prompt(&mut self, prompt: String, prompt_origin: PromptOrigin) {
        let ConversationState::Ready(conversation) = &mut self.conversation_state else {
            return;
        };
        if conversation.has_running_turn() {
            return;
        }

        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return;
        }

        let thread_id = conversation.thread_id.clone();
        let cwd = conversation.cwd.clone();
        let is_new_thread = !conversation.has_active_thread();
        match prompt_origin {
            PromptOrigin::Manual => conversation.auto_follow_state.reset_for_manual_turn(),
            PromptOrigin::AutoFollow => conversation.auto_follow_state.mark_auto_turn_submitted(),
        }
        let auto_follow_progress = conversation.auto_follow_state.progress_label();
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::User,
            prompt.clone(),
            None,
            None,
        ));
        conversation.refresh_conversation_lines();
        conversation.input_buffer.clear();
        conversation.mark_turn_submitting();
        conversation.status_text = match prompt_origin {
            PromptOrigin::Manual => "starting turn".to_string(),
            PromptOrigin::AutoFollow => {
                format!("auto follow-up submitted ({auto_follow_progress})")
            }
        };

        let outer_tx = self.tx.clone();
        let service = self.conversation_service.clone();
        thread::spawn(move || {
            let (event_tx, event_rx) = mpsc::channel();

            let service_thread = thread::spawn(move || {
                let result = if is_new_thread {
                    service.run_new_thread_stream(&cwd, &prompt, event_tx)
                } else {
                    service.run_turn_stream(&thread_id, &prompt, event_tx)
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

    fn poll_background_messages(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                BackgroundMessage::StartupLoaded(result) => {
                    self.startup_state = match result {
                        Ok(diagnostics) => StartupState::Ready(diagnostics),
                        Err(message) => StartupState::Failed(message),
                    };
                }
                BackgroundMessage::SessionsLoaded(result) => {
                    self.session_state = match result {
                        Ok(recent_sessions) => {
                            self.selected_session_index = 0;
                            SessionState::Ready(recent_sessions)
                        }
                        Err(message) => SessionState::Failed(message),
                    };
                }
                BackgroundMessage::ConversationLoaded(result) => {
                    self.conversation_state = match result {
                        Ok(snapshot) => {
                            ConversationState::Ready(ConversationViewModel::from_snapshot(snapshot))
                        }
                        Err(message) => ConversationState::Failed(message),
                    };
                }
                BackgroundMessage::ConversationStream(event) => {
                    self.apply_conversation_event(event);
                }
            }
        }
    }

    fn apply_conversation_event(&mut self, event: ConversationStreamEvent) {
        let mut queued_auto_prompt: Option<String> = None;

        let ConversationState::Ready(conversation) = &mut self.conversation_state else {
            return;
        };
        let mut should_refresh_lines = false;

        match event {
            ConversationStreamEvent::ThreadPrepared {
                thread_id,
                title,
                cwd,
            } => {
                conversation.thread_id = thread_id;
                conversation.title = title;
                conversation.cwd = cwd;
                conversation.status_text = "thread started".to_string();
            }
            ConversationStreamEvent::TurnStarted { turn_id } => {
                conversation.mark_turn_started(turn_id);
                conversation.status_text = "turn started".to_string();
            }
            ConversationStreamEvent::StatusUpdated { text } => {
                conversation.status_text = text;
            }
            ConversationStreamEvent::AgentMessageDelta {
                item_id,
                phase,
                delta,
            } => {
                if let Some(message) = conversation
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|message| message.item_id.as_deref() == Some(item_id.as_str()))
                {
                    message.text.push_str(&delta);
                    if phase.is_some() {
                        message.phase = phase;
                    }
                } else {
                    conversation.messages.push(ConversationMessage::new(
                        ConversationMessageKind::Agent,
                        delta,
                        phase,
                        Some(item_id),
                    ));
                }
                should_refresh_lines = true;
            }
            ConversationStreamEvent::AgentMessageCompleted {
                item_id,
                phase,
                text,
            } => {
                if let Some(message) = conversation
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|message| message.item_id.as_deref() == Some(item_id.as_str()))
                {
                    message.text = text;
                    message.phase = phase;
                } else {
                    conversation.messages.push(ConversationMessage::new(
                        ConversationMessageKind::Agent,
                        text,
                        phase,
                        Some(item_id),
                    ));
                }
                should_refresh_lines = true;
            }
            ConversationStreamEvent::ToolMessage { text } => {
                conversation.messages.push(ConversationMessage::new(
                    ConversationMessageKind::Tool,
                    text,
                    None,
                    None,
                ));
                should_refresh_lines = true;
            }
            ConversationStreamEvent::TurnCompleted { turn_id } => {
                conversation.mark_turn_finished();
                match conversation.decide_auto_followup() {
                    AutoFollowupDecision::Disabled => {
                        conversation.status_text =
                            format!("turn completed: {turn_id} / auto follow-up off");
                    }
                    AutoFollowupDecision::ManualInputBuffered => {
                        conversation.status_text =
                            "manual prompt buffered; auto follow-up skipped".to_string();
                    }
                    AutoFollowupDecision::QueuePrompt(prompt) => {
                        queued_auto_prompt = Some(prompt);
                    }
                    AutoFollowupDecision::LimitReached => {
                        conversation.status_text = format!(
                            "turn completed: {turn_id} / auto follow-up limit reached ({})",
                            conversation.auto_follow_state.progress_label()
                        );
                    }
                    AutoFollowupDecision::NoAgentReply => {
                        conversation.status_text =
                            format!("turn completed: {turn_id} / no agent reply to continue from");
                    }
                }
            }
            ConversationStreamEvent::Failed { message } => {
                conversation.mark_turn_finished();
                conversation.status_text = "turn failed".to_string();
                conversation.messages.push(ConversationMessage::new(
                    ConversationMessageKind::Status,
                    message,
                    None,
                    None,
                ));
                should_refresh_lines = true;
            }
        }

        if should_refresh_lines {
            conversation.refresh_conversation_lines();
        }

        if let Some(prompt) = queued_auto_prompt {
            self.submit_prompt(prompt, PromptOrigin::AutoFollow);
        }
    }

    fn can_open_session_list(&self) -> bool {
        matches!(
            &self.startup_state,
            StartupState::Ready(diagnostics) if diagnostics.can_continue()
        )
    }

    fn open_session_list(&mut self) {
        self.exit_confirmation_state = ExitConfirmationState::Hidden;
        self.current_screen = Screen::SessionList;
        self.active_session = None;
        self.conversation_state = ConversationState::Idle;
        self.start_session_load();
    }

    fn open_new_conversation_shell(&mut self) {
        self.active_session = None;
        self.conversation_return_screen = self.current_screen;
        self.conversation_state = ConversationState::Ready(ConversationViewModel::new_draft(
            self.current_workspace_directory(),
        ));
        self.current_screen = Screen::ConversationShell;
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
        if let Some(session) = self.current_session().cloned() {
            let thread_id = session.id.clone();
            self.active_session = Some(session);
            self.conversation_return_screen = Screen::SessionList;
            self.exit_confirmation_state = ExitConfirmationState::Hidden;
            self.current_screen = Screen::ConversationShell;
            self.start_conversation_load(thread_id);
        }
    }

    fn leave_conversation_shell(&mut self) {
        let return_screen = self.conversation_return_screen;
        self.current_screen = return_screen;
        self.conversation_state = ConversationState::Idle;

        if return_screen == Screen::Home {
            self.active_session = None;
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let SessionState::Ready(recent_sessions) = &self.session_state else {
            return;
        };
        if recent_sessions.items.is_empty() {
            self.selected_session_index = 0;
            return;
        }

        let max_index = recent_sessions.items.len().saturating_sub(1) as isize;
        let current_index = self.selected_session_index as isize;
        let next_index = (current_index + delta).clamp(0, max_index);
        self.selected_session_index = next_index as usize;
    }

    fn conversation_can_accept_input(&self) -> bool {
        matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation) if conversation.can_submit_prompt()
        )
    }

    fn push_input_character(&mut self, character: char) {
        if let ConversationState::Ready(conversation) = &mut self.conversation_state {
            conversation.input_buffer.push(character);
        }
    }

    fn pop_input_character(&mut self) {
        if let ConversationState::Ready(conversation) = &mut self.conversation_state {
            conversation.input_buffer.pop();
        }
    }

    fn toggle_auto_followup(&mut self) {
        if let ConversationState::Ready(conversation) = &mut self.conversation_state {
            conversation.auto_follow_state.toggle();
            conversation.status_text = format!(
                "auto follow-up {}",
                conversation.auto_follow_state.status_label()
            );
        }
    }

    fn cycle_auto_followup_template(&mut self) {
        if let ConversationState::Ready(conversation) = &mut self.conversation_state {
            conversation.auto_follow_state.cycle_template_kind();
            conversation.status_text = format!(
                "auto follow-up template: {}",
                conversation.auto_follow_state.template_label()
            );
        }
    }

    fn current_workspace_directory(&self) -> String {
        match &self.startup_state {
            StartupState::Ready(diagnostics) => diagnostics.workspace_path.clone(),
            _ => std::env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| ".".to_string()),
        }
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
                self.exit_confirmation_state = ExitConfirmationState::Hidden;
                Some(false)
            }
            _ => Some(false),
        }
    }

    fn handle_ctrl_c(&mut self) {
        self.exit_confirmation_state = ExitConfirmationState::Hidden;

        match self.current_screen {
            Screen::ConversationShell => {
                self.leave_conversation_shell();
            }
            Screen::SessionList => {
                self.current_screen = Screen::Home;
                self.active_session = None;
                self.conversation_state = ConversationState::Idle;
            }
            Screen::Home => {
                self.exit_confirmation_state = ExitConfirmationState::Visible;
            }
        }
    }
}

fn run_tui(mut app: NativeTuiApp) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
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

        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
            app.handle_ctrl_c();
            continue;
        }

        match app.current_screen {
            Screen::Home => match key.code {
                KeyCode::Char('q') => should_quit = true,
                KeyCode::Char('n') if key.modifiers.is_empty() && app.can_open_session_list() => {
                    app.open_new_conversation_shell()
                }
                KeyCode::Char('r') => app.start_startup_check(),
                KeyCode::Enter if app.can_open_session_list() => app.open_session_list(),
                _ => {}
            },
            Screen::SessionList => match key.code {
                KeyCode::Char('q') => should_quit = true,
                KeyCode::Char('b') => app.current_screen = Screen::Home,
                KeyCode::Char('n') if key.modifiers.is_empty() => app.open_new_conversation_shell(),
                KeyCode::Char('r') => app.start_session_load(),
                KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
                KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
                KeyCode::Enter => app.open_conversation_shell(),
                _ => {}
            },
            Screen::ConversationShell => match key.code {
                KeyCode::Char('q') if key.modifiers.is_empty() => should_quit = true,
                KeyCode::Char('a') if key.modifiers.is_empty() => app.toggle_auto_followup(),
                KeyCode::Char('f') if key.modifiers == KeyModifiers::CONTROL => {
                    app.cycle_auto_followup_template()
                }
                KeyCode::Char('b') if key.modifiers.is_empty() => {
                    app.leave_conversation_shell();
                }
                KeyCode::Backspace => app.pop_input_character(),
                KeyCode::Enter if app.conversation_can_accept_input() => {
                    app.start_turn_submission()
                }
                KeyCode::Char(character)
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT =>
                {
                    app.push_input_character(character);
                }
                _ => {}
            },
        }
    }

    Ok(())
}

fn draw(frame: &mut Frame<'_>, app: &NativeTuiApp) {
    match app.current_screen {
        Screen::Home => draw_home(frame, app),
        Screen::SessionList => draw_session_list(frame, app),
        Screen::ConversationShell => draw_conversation_shell(frame, app),
    }

    if app.is_exit_confirmation_visible() {
        draw_exit_confirmation(frame);
    }
}

fn draw_home(frame: &mut Frame<'_>, app: &NativeTuiApp) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(7),
            Constraint::Min(10),
            Constraint::Length(7),
            Constraint::Length(3),
        ])
        .split(area);

    let title = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                "codex-exec-loop",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" native"),
        ]),
        Line::from("Codex app-server client prototype"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Welcome"));
    frame.render_widget(title, layout[0]);

    let summary = match &app.startup_state {
        StartupState::Idle => vec![
            Line::from("status: idle"),
            Line::from("startup check has not started"),
        ],
        StartupState::Loading => vec![
            Line::from(vec![
                Span::styled("status: ", Style::default().fg(Color::Gray)),
                Span::styled("running checks", Style::default().fg(Color::Yellow)),
            ]),
            Line::from("probing codex binary, app-server handshake, account state, and cwd"),
        ],
        StartupState::Ready(diagnostics) => vec![
            Line::from(vec![
                Span::styled("status: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    if diagnostics.can_continue() {
                        "ready"
                    } else {
                        "needs attention"
                    },
                    Style::default().fg(if diagnostics.can_continue() {
                        Color::Green
                    } else {
                        Color::Yellow
                    }),
                ),
            ]),
            Line::from(format!("cwd: {}", diagnostics.cwd)),
        ],
        StartupState::Failed(message) => vec![
            Line::from(vec![
                Span::styled("status: ", Style::default().fg(Color::Gray)),
                Span::styled("failed", Style::default().fg(Color::Red)),
            ]),
            Line::from(message.clone()),
        ],
    };

    let summary_widget = Paragraph::new(summary)
        .block(Block::default().borders(Borders::ALL).title("Startup"))
        .wrap(Wrap { trim: true });
    frame.render_widget(summary_widget, layout[1]);

    let checklist = build_check_items(app);
    let check_list =
        List::new(checklist).block(Block::default().borders(Borders::ALL).title("Checks"));
    frame.render_widget(check_list, layout[2]);

    let warnings = build_startup_warning_lines(app);
    let warning_widget = Paragraph::new(warnings)
        .block(Block::default().borders(Borders::ALL).title("Warnings"))
        .wrap(Wrap { trim: true });
    frame.render_widget(warning_widget, layout[3]);

    let help = Paragraph::new(vec![
        Line::from("Enter: open recent sessions    n: new conversation"),
        Line::from("r: rerun checks    Ctrl+C: exit confirm    q: quit"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(help, layout[4]);
}

fn draw_session_list(frame: &mut Frame<'_>, app: &NativeTuiApp) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(4),
            Constraint::Length(3),
        ])
        .split(area);

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                "Recent Sessions",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" from codex app-server"),
        ]),
        Line::from("Browse Codex conversation threads and pick one to resume."),
    ])
    .block(Block::default().borders(Borders::ALL).title("Sessions"));
    frame.render_widget(header, layout[0]);

    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(layout[1]);

    draw_session_list_panel(frame, content_layout[0], app);
    draw_session_detail_panel(frame, content_layout[1], app);

    let warnings = build_session_warning_lines(app);
    let warning_widget = Paragraph::new(warnings)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Session Warnings"),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(warning_widget, layout[2]);

    let help = Paragraph::new(vec![
        Line::from("Up/Down or j/k: move    Enter: open live shell"),
        Line::from("n: new conversation    r: reload    b/Ctrl+C: back    q: quit"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(help, layout[3]);
}

fn draw_session_list_panel(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
    match &app.session_state {
        SessionState::Idle => {
            let widget = Paragraph::new("session list has not loaded yet")
                .block(Block::default().borders(Borders::ALL).title("Threads"))
                .wrap(Wrap { trim: true });
            frame.render_widget(widget, area);
        }
        SessionState::Loading => {
            let widget = Paragraph::new("loading recent sessions from codex app-server")
                .block(Block::default().borders(Borders::ALL).title("Threads"))
                .wrap(Wrap { trim: true });
            frame.render_widget(widget, area);
        }
        SessionState::Failed(message) => {
            let widget = Paragraph::new(message.as_str())
                .block(Block::default().borders(Borders::ALL).title("Threads"))
                .wrap(Wrap { trim: true });
            frame.render_widget(widget, area);
        }
        SessionState::Ready(recent_sessions) => {
            let items = if recent_sessions.items.is_empty() {
                vec![ListItem::new("(no sessions found)")]
            } else {
                recent_sessions
                    .items
                    .iter()
                    .map(build_session_list_item)
                    .collect::<Vec<_>>()
            };

            let mut list_state = ListState::default();
            if recent_sessions.items.is_empty() {
                list_state.select(None);
            } else {
                list_state.select(Some(app.selected_session_index));
            }

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title("Threads"))
                .highlight_style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(">> ");

            frame.render_stateful_widget(list, area, &mut list_state);
        }
    }
}

fn draw_session_detail_panel(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
    let lines = match &app.session_state {
        SessionState::Idle => vec![Line::from("session details are not available yet")],
        SessionState::Loading => vec![Line::from("waiting for session list response")],
        SessionState::Failed(message) => vec![Line::from(message.clone())],
        SessionState::Ready(recent_sessions) if recent_sessions.items.is_empty() => {
            vec![Line::from("no session detail to show")]
        }
        SessionState::Ready(recent_sessions) => {
            let selected_session = recent_sessions
                .items
                .get(app.selected_session_index)
                .unwrap_or(&recent_sessions.items[0]);

            let mut lines = vec![
                Line::from(format!("id: {}", selected_session.id)),
                Line::from(format!("updated: {}", selected_session.updated_at_label())),
                Line::from(format!("workspace: {}", selected_session.cwd)),
                Line::from(format!("source: {}", selected_session.source)),
                Line::from(format!(
                    "model provider: {}",
                    selected_session.model_provider
                )),
                Line::from(format!("status: {}", selected_session.status_type)),
            ];

            if let Some(branch) = &selected_session.git_branch {
                lines.push(Line::from(format!("git branch: {branch}")));
            }

            if recent_sessions.next_cursor.is_some() {
                lines.push(Line::from("more threads are available in the next cursor"));
            }

            lines.push(Line::from(""));
            lines.push(Line::from("preview"));
            lines.push(Line::from(selected_session.preview_block()));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("path: {}", selected_session.path)));
            lines
        }
    };

    let detail = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Selected Session"),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, area);
}

fn draw_conversation_shell(frame: &mut Frame<'_>, app: &NativeTuiApp) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(4),
            Constraint::Length(3),
        ])
        .split(area);

    let header_lines = match &app.conversation_state {
        ConversationState::Idle => vec![
            Line::from("Conversation shell"),
            Line::from("Open a session first."),
        ],
        ConversationState::Loading => vec![
            Line::from(vec![
                Span::styled("Conversation Shell", Style::default().fg(Color::Cyan)),
                Span::raw(" / loading thread"),
            ]),
            Line::from("Reading thread history from codex app-server."),
        ],
        ConversationState::Ready(conversation) => vec![
            Line::from(vec![
                Span::styled("Conversation Shell", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" / {}", conversation.title)),
            ]),
            Line::from(vec![
                Span::raw(format!(
                    "thread: {}  |  input: ",
                    if conversation.has_active_thread() {
                        conversation.thread_id.as_str()
                    } else {
                        "not started yet"
                    }
                )),
                Span::styled(
                    conversation.input_state.label(),
                    input_state_style(conversation.input_state),
                ),
            ]),
        ],
        ConversationState::Failed(message) => vec![
            Line::from(vec![
                Span::styled("Conversation Shell", Style::default().fg(Color::Red)),
                Span::raw(" / failed"),
            ]),
            Line::from(message.clone()),
        ],
    };
    let header = Paragraph::new(header_lines)
        .block(Block::default().borders(Borders::ALL).title("Shell"))
        .wrap(Wrap { trim: true });
    frame.render_widget(header, layout[0]);

    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(layout[1]);

    let conversation_lines = build_conversation_lines(app);
    let conversation_scroll = build_conversation_scroll_offset(
        &conversation_lines,
        content_layout[0].width.saturating_sub(2),
        content_layout[0].height.saturating_sub(2),
    );
    let conversation = Paragraph::new(conversation_lines)
        .block(Block::default().borders(Borders::ALL).title("Conversation"))
        .scroll((conversation_scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(conversation, content_layout[0]);

    let activity = Paragraph::new(build_conversation_activity_lines(app))
        .block(Block::default().borders(Borders::ALL).title("Activity"))
        .wrap(Wrap { trim: false });
    frame.render_widget(activity, content_layout[1]);

    let input = Paragraph::new(build_input_lines(app))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(build_input_title(app)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(input, layout[2]);

    let help = Paragraph::new(vec![
        Line::from("Type your prompt and press Enter to send"),
        Line::from(
            "a: auto on/off    Ctrl+f: next template    Backspace: delete    b/Ctrl+C: back    q: quit",
        ),
    ])
    .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(help, layout[3]);
}

fn draw_exit_confirmation(frame: &mut Frame<'_>) {
    let popup_area = centered_rect(42, 22, frame.area());
    frame.render_widget(Clear, popup_area);

    let popup = Paragraph::new(vec![
        Line::from("You are already at home."),
        Line::from("Exit codex-exec-loop?"),
        Line::from(""),
        Line::from("y: exit    n: stay"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Confirm Exit"))
    .wrap(Wrap { trim: true });

    frame.render_widget(popup, popup_area);
}

fn build_check_items(app: &NativeTuiApp) -> Vec<ListItem<'static>> {
    match &app.startup_state {
        StartupState::Idle => vec![ListItem::new("startup check has not started")],
        StartupState::Loading => vec![
            ListItem::new("checking codex binary"),
            ListItem::new("opening codex app-server"),
            ListItem::new("reading account state"),
        ],
        StartupState::Ready(diagnostics) => vec![
            diagnostic_item(
                "codex binary",
                diagnostics.codex_binary_ok,
                &diagnostics.codex_binary_detail,
            ),
            diagnostic_item(
                "workspace",
                diagnostics.workspace_ok,
                &diagnostics.workspace_detail,
            ),
            diagnostic_item(
                "app-server initialize",
                diagnostics.initialize_ok,
                &diagnostics.initialize_detail,
            ),
            diagnostic_item(
                "account/read",
                diagnostics.account_ok,
                &diagnostics.account_detail,
            ),
            ListItem::new(format!("schema snapshot: {}", diagnostics.schema_snapshot)),
        ],
        StartupState::Failed(message) => vec![ListItem::new(format!("startup error: {message}"))],
    }
}

fn build_startup_warning_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.startup_state {
        StartupState::Ready(diagnostics) if !diagnostics.warnings.is_empty() => diagnostics
            .warnings
            .iter()
            .cloned()
            .map(Line::from)
            .collect::<Vec<_>>(),
        StartupState::Failed(message) => vec![Line::from(message.clone())],
        _ => vec![Line::from("no warnings")],
    }
}

fn build_session_warning_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.session_state {
        SessionState::Ready(recent_sessions) if !recent_sessions.warnings.is_empty() => {
            recent_sessions
                .warnings
                .iter()
                .cloned()
                .map(Line::from)
                .collect::<Vec<_>>()
        }
        SessionState::Failed(message) => vec![Line::from(message.clone())],
        SessionState::Loading => vec![Line::from("waiting for app-server response")],
        _ => vec![Line::from("no warnings")],
    }
}

fn build_session_list_item(session: &SessionSummary) -> ListItem<'static> {
    ListItem::new(vec![
        Line::from(format!(
            "{}  {}  {}",
            session.short_id(),
            session.updated_at_label(),
            session.workspace_label(),
        )),
        Line::from(format!(
            "{} [{} / {}]",
            session.title(),
            session.source,
            session.model_provider,
        )),
    ])
}

fn build_conversation_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Idle => vec![Line::from("No conversation selected.")],
        ConversationState::Loading => vec![Line::from("Loading thread history...")],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
        ConversationState::Ready(conversation) => conversation.cached_conversation_lines.clone(),
    }
}

fn build_conversation_scroll_offset(
    lines: &[Line<'static>],
    content_width: u16,
    visible_height: u16,
) -> u16 {
    if content_width == 0 || visible_height == 0 {
        return 0;
    }

    let rendered_line_count = count_rendered_conversation_lines(lines, content_width);
    let visible_height = visible_height as usize;
    rendered_line_count
        .saturating_sub(visible_height)
        .min(u16::MAX as usize) as u16
}

fn count_rendered_conversation_lines(lines: &[Line<'static>], content_width: u16) -> usize {
    if content_width == 0 {
        return 0;
    }

    lines
        .iter()
        .map(|line| count_wrapped_rows(line, content_width))
        .sum()
}

fn count_wrapped_rows(line: &Line<'static>, content_width: u16) -> usize {
    if content_width == 0 {
        return 0;
    }

    let line_width = line.width();
    if line_width == 0 {
        return 1;
    }

    line_width.div_ceil(content_width as usize)
}

fn format_conversation_lines(messages: &[ConversationMessage]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for message in messages {
        let label = message.kind.label(message.phase.as_deref());
        lines.push(Line::from(Span::styled(
            format!("{label}:"),
            label_style(message.kind),
        )));
        for text_line in message.text.lines() {
            lines.push(Line::from(format!("  {text_line}")));
        }
        lines.push(Line::from(""));
    }

    if lines.is_empty() {
        lines.push(Line::from("No messages in this thread yet."));
    }

    if lines.len() > MAX_CONVERSATION_HISTORY_LINES {
        lines = lines.split_off(lines.len() - MAX_CONVERSATION_HISTORY_LINES);
    }

    lines
}

fn build_conversation_activity_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Idle => vec![Line::from("No active conversation")],
        ConversationState::Loading => vec![Line::from("Loading conversation metadata")],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
        ConversationState::Ready(conversation) => {
            let mut lines = vec![
                Line::from(format!("title: {}", conversation.title)),
                Line::from(format!(
                    "thread id: {}",
                    if conversation.has_active_thread() {
                        conversation.thread_id.as_str()
                    } else {
                        "(new thread will be created on first send)"
                    }
                )),
                Line::from(format!("cwd: {}", conversation.cwd)),
                Line::from(format!("messages: {}", conversation.messages.len())),
                Line::from(format!(
                    "turn running: {}",
                    if conversation.has_running_turn() {
                        "yes"
                    } else {
                        "no"
                    }
                )),
                Line::from(format!("input state: {}", conversation.input_state.label())),
                Line::from(format!(
                    "input detail: {}",
                    conversation.input_state.detail()
                )),
                Line::from(format!(
                    "send action: {}",
                    if conversation.can_submit_prompt() {
                        "enabled"
                    } else {
                        "waiting for current turn"
                    }
                )),
                Line::from(format!(
                    "auto follow-up: {}",
                    conversation.auto_follow_state.status_label()
                )),
                Line::from(format!(
                    "auto progress: {}",
                    conversation.auto_follow_state.progress_label()
                )),
                Line::from(format!(
                    "auto template: {}",
                    conversation.auto_follow_state.template_label()
                )),
                Line::from(format!("status: {}", conversation.status_text)),
            ];

            if let Some(turn_id) = &conversation.active_turn_id {
                lines.push(Line::from(format!("active turn: {turn_id}")));
            }

            if !conversation.warnings.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from("warnings"));
                for warning in &conversation.warnings {
                    lines.push(Line::from(warning.clone()));
                }
            }

            lines
        }
    }
}

fn build_input_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Idle => vec![Line::from("Select a session first.")],
        ConversationState::Loading => vec![
            Line::from("Thread is still loading."),
            Line::from("Input becomes available when the shell reaches ready state."),
        ],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
        ConversationState::Ready(conversation) => build_ready_input_lines(conversation),
    }
}

fn build_ready_input_lines(conversation: &ConversationViewModel) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if conversation.input_buffer.is_empty() {
        match conversation.input_state {
            ConversationInputState::DraftReady => {
                lines.push(Line::from("Ready to start a new thread."));
                lines.push(Line::from("Type the first prompt and press Enter."));
            }
            ConversationInputState::ReadyToContinue => {
                lines.push(Line::from("Ready to continue this session."));
                lines.push(Line::from("Type the next prompt and press Enter."));
            }
            ConversationInputState::SubmittingTurn => {
                lines.push(Line::from("Sending prompt to Codex..."));
                lines.push(Line::from(
                    "Wait for the turn to open before sending again.",
                ));
            }
            ConversationInputState::StreamingTurn => {
                lines.push(Line::from("Codex is still working on the current turn."));
                lines.push(Line::from(
                    "Type now; press Enter after the turn completes.",
                ));
            }
        }

        return lines;
    }

    lines.push(Line::from(conversation.input_buffer.clone()));

    match conversation.input_state {
        ConversationInputState::DraftReady => {
            lines.push(Line::from("Press Enter to create thread and send."));
        }
        ConversationInputState::ReadyToContinue => {
            lines.push(Line::from("Press Enter to send this prompt."));
        }
        ConversationInputState::SubmittingTurn | ConversationInputState::StreamingTurn => {
            lines.push(Line::from("Prompt buffered. Press Enter when turn ends."));
        }
    }

    lines
}

fn build_input_title(app: &NativeTuiApp) -> String {
    match &app.conversation_state {
        ConversationState::Idle => "Input / idle".to_string(),
        ConversationState::Loading => "Input / loading".to_string(),
        ConversationState::Failed(_) => "Input / unavailable".to_string(),
        ConversationState::Ready(conversation) => {
            format!("Input / {}", conversation.input_state.label())
        }
    }
}

fn input_state_style(input_state: ConversationInputState) -> Style {
    match input_state {
        ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue => {
            Style::default().fg(Color::Green)
        }
        ConversationInputState::SubmittingTurn => Style::default().fg(Color::Yellow),
        ConversationInputState::StreamingTurn => Style::default().fg(Color::Cyan),
    }
}

fn label_style(kind: ConversationMessageKind) -> Style {
    match kind {
        ConversationMessageKind::User => Style::default().fg(Color::Yellow),
        ConversationMessageKind::Agent => Style::default().fg(Color::Cyan),
        ConversationMessageKind::Tool => Style::default().fg(Color::Magenta),
        ConversationMessageKind::Status => Style::default().fg(Color::Red),
    }
}

fn diagnostic_item(title: &str, ok: bool, detail: &str) -> ListItem<'static> {
    let marker = if ok { "[ok]" } else { "[warn]" };
    ListItem::new(format!("{marker} {title}: {detail}"))
}

fn centered_rect(horizontal_percent: u16, vertical_percent: u16, area: Rect) -> Rect {
    let vertical_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - vertical_percent) / 2),
            Constraint::Percentage(vertical_percent),
            Constraint::Percentage((100 - vertical_percent) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - horizontal_percent) / 2),
            Constraint::Percentage(horizontal_percent),
            Constraint::Percentage((100 - horizontal_percent) / 2),
        ])
        .split(vertical_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::{
        AutoFollowState, AutoFollowTemplateKind, AutoFollowupDecision, ConversationInputState,
        ConversationMessage, ConversationMessageKind, ConversationViewModel,
        build_conversation_scroll_offset, build_ready_input_lines,
        count_rendered_conversation_lines, format_conversation_lines,
    };

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
            auto_follow_state: AutoFollowState::default(),
            status_text: "thread loaded".to_string(),
        }
    }

    #[test]
    fn running_turn_still_shows_buffered_prompt() {
        let mut conversation = ready_conversation();
        conversation.input_state = ConversationInputState::StreamingTurn;
        conversation.input_buffer = "Continue from the last change.".to_string();

        let rendered = build_ready_input_lines(&conversation)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Continue from the last change."));
        assert!(rendered.contains("Prompt buffered. Press Enter when turn ends."));
    }

    #[test]
    fn empty_existing_session_prompts_for_next_message() {
        let conversation = ready_conversation();

        let rendered = build_ready_input_lines(&conversation)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Ready to continue this session."));
        assert!(rendered.contains("Type the next prompt and press Enter."));
    }

    #[test]
    fn empty_draft_prompts_for_first_message() {
        let mut conversation = ready_conversation();
        conversation.thread_id.clear();
        conversation.input_state = ConversationInputState::DraftReady;

        let rendered = build_ready_input_lines(&conversation)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Ready to start a new thread."));
        assert!(rendered.contains("Type the first prompt and press Enter."));
    }

    #[test]
    fn conversation_scroll_offset_moves_to_latest_rows() {
        let lines = vec![
            ratatui::text::Line::from("line-1"),
            ratatui::text::Line::from("line-2"),
            ratatui::text::Line::from("line-3"),
            ratatui::text::Line::from("line-4"),
        ];

        let scroll_offset = build_conversation_scroll_offset(&lines, 20, 2);

        assert_eq!(scroll_offset, 2);
    }

    #[test]
    fn conversation_scroll_offset_counts_wrapped_rows() {
        let lines = vec![
            ratatui::text::Line::from("1234567890"),
            ratatui::text::Line::from("tail"),
        ];

        let rendered_line_count = count_rendered_conversation_lines(&lines, 4);
        let scroll_offset = build_conversation_scroll_offset(&lines, 4, 2);

        assert_eq!(rendered_line_count, 4);
        assert_eq!(scroll_offset, 2);
    }

    #[test]
    fn conversation_scroll_offset_handles_zero_visible_height() {
        let lines = vec![ratatui::text::Line::from("line-1")];

        let scroll_offset = build_conversation_scroll_offset(&lines, 10, 0);

        assert_eq!(scroll_offset, 0);
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
            AutoFollowupDecision::ManualInputBuffered
        );
    }

    #[test]
    fn auto_followup_template_kind_cycles_in_expected_order() {
        let mut state = AutoFollowState::default();

        assert_eq!(state.template_kind, AutoFollowTemplateKind::NextTask);
        state.cycle_template_kind();
        assert_eq!(state.template_kind, AutoFollowTemplateKind::PlanQueue);
        state.cycle_template_kind();
        assert_eq!(state.template_kind, AutoFollowTemplateKind::Bugfix);
        state.cycle_template_kind();
        assert_eq!(state.template_kind, AutoFollowTemplateKind::Docs);
        state.cycle_template_kind();
        assert_eq!(state.template_kind, AutoFollowTemplateKind::NextTask);
    }

    #[test]
    fn auto_followup_prompt_uses_selected_template_kind() {
        let mut conversation = ready_conversation();
        conversation.auto_follow_state.template_kind = AutoFollowTemplateKind::PlanQueue;
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
}

use std::sync::mpsc::{self, Sender};
use std::thread;

use crate::application::service::conversation_service::ConversationService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::{
    ConversationMessage, ConversationMessageKind, ConversationSnapshot, ConversationStreamEvent,
    ConversationToolActivity,
};
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::startup_diagnostics::StartupDiagnostics;

pub const POC_SESSION_PAGE_SIZE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamShellOverlay {
    None,
    Startup,
    Sessions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamTurnPhase {
    Idle,
    Submitting,
    Streaming,
}

#[derive(Debug, Clone)]
pub enum StreamShellStartupState {
    Idle,
    Loading,
    Ready(StartupDiagnostics),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum StreamShellSessionsState {
    Idle,
    Loading,
    Ready(RecentSessions),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct StreamShellThreadState {
    pub thread_id: Option<String>,
    pub title: String,
    pub cwd: String,
    pub transcript: Vec<ConversationMessage>,
    pub turn_phase: StreamTurnPhase,
}

#[derive(Debug, Clone)]
pub struct StreamShellState {
    pub overlay: StreamShellOverlay,
    pub startup: StreamShellStartupState,
    pub sessions: StreamShellSessionsState,
    pub thread: StreamShellThreadState,
    pub composer: String,
    pub status_line: String,
}

impl StreamShellState {
    pub fn new(initial_cwd: impl Into<String>) -> Self {
        Self {
            overlay: StreamShellOverlay::None,
            startup: StreamShellStartupState::Idle,
            sessions: StreamShellSessionsState::Idle,
            thread: StreamShellThreadState {
                thread_id: None,
                title: "New conversation".to_string(),
                cwd: initial_cwd.into(),
                transcript: Vec::new(),
                turn_phase: StreamTurnPhase::Idle,
            },
            composer: String::new(),
            status_line: "idle".to_string(),
        }
    }

    fn startup_allows_sessions(&self) -> bool {
        matches!(
            &self.startup,
            StreamShellStartupState::Ready(diagnostics) if diagnostics.can_continue()
        )
    }
}

#[derive(Debug, Clone)]
pub enum StreamShellEvent {
    AppStarted,
    OpenStartupOverlay,
    OpenSessionsOverlay,
    CloseOverlay,
    SessionsRequested,
    SessionChosen { thread_id: String },
    ComposerChanged(String),
    SubmitPressed,
    StartupLoaded(Result<StartupDiagnostics, String>),
    SessionsLoaded(Result<RecentSessions, String>),
    ConversationLoaded(Result<ConversationSnapshot, String>),
    StreamUpdate(ConversationStreamEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamShellEffect {
    RunStartupChecks,
    LoadRecentSessions {
        limit: usize,
    },
    LoadConversation {
        thread_id: String,
    },
    SubmitPrompt {
        cwd: String,
        thread_id: Option<String>,
        prompt: String,
    },
}

#[derive(Debug, Clone)]
pub struct StreamShellReduction {
    pub state: StreamShellState,
    pub effects: Vec<StreamShellEffect>,
}

pub fn reduce_stream_shell(
    mut state: StreamShellState,
    event: StreamShellEvent,
) -> StreamShellReduction {
    let mut effects = Vec::new();

    match event {
        StreamShellEvent::AppStarted => {
            state.startup = StreamShellStartupState::Loading;
            state.status_line = "running startup checks".to_string();
            effects.push(StreamShellEffect::RunStartupChecks);
        }
        StreamShellEvent::OpenStartupOverlay => {
            state.overlay = StreamShellOverlay::Startup;
        }
        StreamShellEvent::OpenSessionsOverlay => {
            state.overlay = StreamShellOverlay::Sessions;
        }
        StreamShellEvent::CloseOverlay => {
            state.overlay = StreamShellOverlay::None;
        }
        StreamShellEvent::SessionsRequested => {
            if state.startup_allows_sessions() {
                state.sessions = StreamShellSessionsState::Loading;
                state.status_line = "loading recent sessions".to_string();
                effects.push(StreamShellEffect::LoadRecentSessions {
                    limit: POC_SESSION_PAGE_SIZE,
                });
            }
        }
        StreamShellEvent::SessionChosen { thread_id } => {
            state.overlay = StreamShellOverlay::None;
            state.status_line = format!("loading thread {thread_id}");
            effects.push(StreamShellEffect::LoadConversation { thread_id });
        }
        StreamShellEvent::ComposerChanged(value) => {
            state.composer = value;
        }
        StreamShellEvent::SubmitPressed => {
            let prompt = state.composer.trim().to_string();
            if prompt.is_empty() || state.thread.turn_phase != StreamTurnPhase::Idle {
                return StreamShellReduction { state, effects };
            }

            state.thread.turn_phase = StreamTurnPhase::Submitting;
            state.thread.transcript.push(ConversationMessage::new(
                ConversationMessageKind::User,
                prompt.clone(),
                None,
                None,
            ));
            state.composer.clear();
            state.status_line = "submitting prompt".to_string();
            effects.push(StreamShellEffect::SubmitPrompt {
                cwd: state.thread.cwd.clone(),
                thread_id: state.thread.thread_id.clone(),
                prompt,
            });
        }
        StreamShellEvent::StartupLoaded(result) => match result {
            Ok(diagnostics) => {
                let can_continue = diagnostics.can_continue();
                state.thread.cwd = diagnostics.workspace_path.clone();
                state.startup = StreamShellStartupState::Ready(diagnostics);
                state.status_line = if can_continue {
                    "startup ready".to_string()
                } else {
                    "startup diagnostics need attention".to_string()
                };
                if can_continue {
                    state.sessions = StreamShellSessionsState::Loading;
                    effects.push(StreamShellEffect::LoadRecentSessions {
                        limit: POC_SESSION_PAGE_SIZE,
                    });
                }
            }
            Err(message) => {
                state.startup = StreamShellStartupState::Failed(message.clone());
                state.status_line = format!("startup failed: {message}");
            }
        },
        StreamShellEvent::SessionsLoaded(result) => match result {
            Ok(sessions) => {
                state.status_line = format!("{} recent sessions loaded", sessions.items.len());
                state.sessions = StreamShellSessionsState::Ready(sessions);
            }
            Err(message) => {
                state.sessions = StreamShellSessionsState::Failed(message.clone());
                state.status_line = format!("session load failed: {message}");
            }
        },
        StreamShellEvent::ConversationLoaded(result) => match result {
            Ok(snapshot) => {
                state.thread = thread_state_from_snapshot(snapshot);
                state.status_line = "thread loaded".to_string();
            }
            Err(message) => {
                state.status_line = format!("thread load failed: {message}");
            }
        },
        StreamShellEvent::StreamUpdate(event) => {
            apply_stream_event(&mut state, event);
        }
    }

    StreamShellReduction { state, effects }
}

#[derive(Clone)]
pub struct StreamShellEffectHandler {
    startup_service: StartupService,
    session_service: SessionService,
    conversation_service: ConversationService,
}

impl StreamShellEffectHandler {
    pub fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
    ) -> Self {
        Self {
            startup_service,
            session_service,
            conversation_service,
        }
    }

    pub fn execute(&self, effect: StreamShellEffect, event_sender: Sender<StreamShellEvent>) {
        match effect {
            StreamShellEffect::RunStartupChecks => {
                let service = self.startup_service.clone();
                thread::spawn(move || {
                    let result = service.run_checks().map_err(|error| error.to_string());
                    let _ = event_sender.send(StreamShellEvent::StartupLoaded(result));
                });
            }
            StreamShellEffect::LoadRecentSessions { limit } => {
                let service = self.session_service.clone();
                thread::spawn(move || {
                    let result = service
                        .load_recent_sessions(limit)
                        .map_err(|error| error.to_string());
                    let _ = event_sender.send(StreamShellEvent::SessionsLoaded(result));
                });
            }
            StreamShellEffect::LoadConversation { thread_id } => {
                let service = self.conversation_service.clone();
                thread::spawn(move || {
                    let result = service
                        .load_snapshot(&thread_id)
                        .map_err(|error| error.to_string());
                    let _ = event_sender.send(StreamShellEvent::ConversationLoaded(result));
                });
            }
            StreamShellEffect::SubmitPrompt {
                cwd,
                thread_id,
                prompt,
            } => {
                let service = self.conversation_service.clone();
                thread::spawn(move || {
                    let (stream_tx, stream_rx) = mpsc::channel();
                    let runner = thread::spawn(move || match thread_id {
                        Some(thread_id) => service.run_turn_stream(&thread_id, &prompt, stream_tx),
                        None => service.run_new_thread_stream(&cwd, &prompt, stream_tx),
                    });

                    let mut saw_terminal_event = false;
                    let mut ui_connected = true;
                    while let Ok(event) = stream_rx.recv() {
                        let terminal = matches!(
                            event,
                            ConversationStreamEvent::TurnCompleted { .. }
                                | ConversationStreamEvent::Failed { .. }
                        );
                        if event_sender
                            .send(StreamShellEvent::StreamUpdate(event))
                            .is_err()
                        {
                            ui_connected = false;
                            break;
                        }
                        if terminal {
                            saw_terminal_event = true;
                            break;
                        }
                    }
                    drop(stream_rx);

                    match runner.join() {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            if ui_connected && !saw_terminal_event {
                                let _ = event_sender.send(StreamShellEvent::StreamUpdate(
                                    ConversationStreamEvent::Failed {
                                        message: error.to_string(),
                                    },
                                ));
                            }
                        }
                        Err(_) => {
                            if ui_connected && !saw_terminal_event {
                                let _ = event_sender.send(StreamShellEvent::StreamUpdate(
                                    ConversationStreamEvent::Failed {
                                        message: "stream worker panicked".to_string(),
                                    },
                                ));
                            }
                        }
                    }
                });
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamShellRenderModel {
    pub transcript_lines: Vec<String>,
    pub composer_text: String,
    pub footer_status: String,
    pub overlay_label: Option<&'static str>,
}

pub fn present_stream_shell(state: &StreamShellState) -> StreamShellRenderModel {
    StreamShellRenderModel {
        transcript_lines: state
            .thread
            .transcript
            .iter()
            .flat_map(render_message)
            .collect(),
        composer_text: state.composer.clone(),
        footer_status: state.status_line.clone(),
        overlay_label: match state.overlay {
            StreamShellOverlay::None => None,
            StreamShellOverlay::Startup => Some("startup"),
            StreamShellOverlay::Sessions => Some("sessions"),
        },
    }
}

fn thread_state_from_snapshot(snapshot: ConversationSnapshot) -> StreamShellThreadState {
    StreamShellThreadState {
        thread_id: Some(snapshot.thread_id),
        title: snapshot.title,
        cwd: snapshot.cwd,
        transcript: snapshot.messages,
        turn_phase: StreamTurnPhase::Idle,
    }
}

fn apply_stream_event(state: &mut StreamShellState, event: ConversationStreamEvent) {
    match event {
        ConversationStreamEvent::ThreadPrepared {
            thread_id,
            title,
            cwd,
        } => {
            state.thread.thread_id = Some(thread_id);
            state.thread.title = title;
            state.thread.cwd = cwd;
            state.status_line = "thread prepared".to_string();
        }
        ConversationStreamEvent::TurnStarted { turn_id } => {
            state.thread.turn_phase = StreamTurnPhase::Streaming;
            state.status_line = format!("turn started: {turn_id}");
        }
        ConversationStreamEvent::StatusUpdated { text } => {
            state.status_line = text;
        }
        ConversationStreamEvent::AgentMessageDelta {
            item_id,
            phase,
            delta,
        } => {
            push_agent_delta(&mut state.thread.transcript, item_id, phase, delta);
        }
        ConversationStreamEvent::AgentMessageCompleted {
            item_id,
            phase,
            text,
        } => {
            complete_agent_message(&mut state.thread.transcript, item_id, phase, text);
        }
        ConversationStreamEvent::ToolActivity { activity } => {
            push_tool_activity(&mut state.thread.transcript, activity);
        }
        ConversationStreamEvent::ApprovalReviewUpdated { review } => {
            let mut segments = vec![match review.status {
                crate::domain::conversation::ConversationApprovalReviewStatus::InProgress => {
                    "approval review in progress".to_string()
                }
                crate::domain::conversation::ConversationApprovalReviewStatus::Approved => {
                    "approval review approved".to_string()
                }
                crate::domain::conversation::ConversationApprovalReviewStatus::Denied => {
                    "approval review denied".to_string()
                }
                crate::domain::conversation::ConversationApprovalReviewStatus::Aborted => {
                    "approval review aborted".to_string()
                }
            }];
            if !review.target_item_id.trim().is_empty() {
                segments.push(format!("target: {}", review.target_item_id));
            }
            if let Some(risk_level) = review
                .risk_level
                .as_deref()
                .filter(|risk| !risk.trim().is_empty())
            {
                segments.push(format!("risk: {risk_level}"));
            }
            state.status_line = segments.join(" / ");
        }
        ConversationStreamEvent::TurnCompleted { turn_id } => {
            state.thread.turn_phase = StreamTurnPhase::Idle;
            state.status_line = format!("turn completed: {turn_id}");
        }
        ConversationStreamEvent::Failed { message } => {
            state.thread.turn_phase = StreamTurnPhase::Idle;
            state.status_line = format!("turn failed: {message}");
            state.thread.transcript.push(ConversationMessage::new(
                ConversationMessageKind::Status,
                message,
                None,
                None,
            ));
        }
    }
}

fn push_agent_delta(
    transcript: &mut Vec<ConversationMessage>,
    item_id: String,
    phase: Option<String>,
    delta: String,
) {
    if let Some(message) = find_message_by_item_id_mut(transcript, &item_id) {
        message.text.push_str(&delta);
        if phase.is_some() {
            message.phase = phase;
        }
        return;
    }

    transcript.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        delta,
        phase,
        Some(item_id),
    ));
}

fn complete_agent_message(
    transcript: &mut Vec<ConversationMessage>,
    item_id: String,
    phase: Option<String>,
    text: String,
) {
    if let Some(message) = find_message_by_item_id_mut(transcript, &item_id) {
        message.text = text;
        message.phase = phase;
        return;
    }

    transcript.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        text,
        phase,
        Some(item_id),
    ));
}

fn find_message_by_item_id_mut<'a>(
    transcript: &'a mut [ConversationMessage],
    item_id: &str,
) -> Option<&'a mut ConversationMessage> {
    transcript
        .iter_mut()
        .rev()
        .find(|message| message.item_id.as_deref() == Some(item_id))
}

fn push_tool_activity(
    transcript: &mut Vec<ConversationMessage>,
    activity: ConversationToolActivity,
) {
    transcript.push(ConversationMessage::new(
        ConversationMessageKind::Tool,
        activity.text,
        None,
        None,
    ));
}

fn render_message(message: &ConversationMessage) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("{}:", message.kind.label(message.phase.as_deref())));
    for text_line in message.text.lines() {
        lines.push(format!("  {text_line}"));
    }
    if message.text.is_empty() {
        lines.push("  ".to_string());
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    use anyhow::{Result, anyhow};

    use super::{
        POC_SESSION_PAGE_SIZE, StreamShellEffect, StreamShellEffectHandler, StreamShellEvent,
        StreamShellOverlay, StreamShellSessionsState, StreamShellStartupState, StreamTurnPhase,
        present_stream_shell, reduce_stream_shell,
    };
    use crate::application::port::outbound::codex_app_server_port::{
        AppServerStartupContext, CodexAppServerPort,
    };
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::{
        ConversationMessage, ConversationMessageKind, ConversationSnapshot, ConversationStreamEvent,
    };
    use crate::domain::recent_sessions::RecentSessions;
    use crate::domain::session_summary::SessionSummary;
    use crate::domain::startup_diagnostics::StartupDiagnostics;

    #[derive(Clone, Copy)]
    enum FakeStreamMode {
        Succeed,
        FailAfterTerminalEvent,
    }

    struct FakeCodexAppServerPort {
        stream_mode: Mutex<FakeStreamMode>,
    }

    impl FakeCodexAppServerPort {
        fn with_stream_mode(stream_mode: FakeStreamMode) -> Self {
            Self {
                stream_mode: Mutex::new(stream_mode),
            }
        }
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
            _cwd: &str,
            _prompt: &str,
            event_sender: mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            self.emit_stream(event_sender)
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            event_sender: mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            self.emit_stream(event_sender)
        }
    }

    impl FakeCodexAppServerPort {
        fn emit_stream(&self, event_sender: mpsc::Sender<ConversationStreamEvent>) -> Result<()> {
            match *self.stream_mode.lock().expect("stream mode mutex poisoned") {
                FakeStreamMode::Succeed => {
                    event_sender.send(ConversationStreamEvent::TurnCompleted {
                        turn_id: "turn-1".to_string(),
                    })?;
                    Ok(())
                }
                FakeStreamMode::FailAfterTerminalEvent => {
                    let message = "adapter failed".to_string();
                    event_sender.send(ConversationStreamEvent::Failed {
                        message: message.clone(),
                    })?;
                    Err(anyhow!(message))
                }
            }
        }
    }

    #[test]
    fn app_started_requests_startup_checks() {
        let initial = super::StreamShellState::new("/tmp/workspace");

        let reduced = reduce_stream_shell(initial, StreamShellEvent::AppStarted);

        assert!(matches!(
            reduced.state.startup,
            StreamShellStartupState::Loading
        ));
        assert_eq!(reduced.state.status_line, "running startup checks");
        assert_eq!(reduced.effects, vec![StreamShellEffect::RunStartupChecks]);
    }

    #[test]
    fn successful_startup_syncs_workspace_and_requests_sessions() {
        let initial = super::StreamShellState::new("/tmp/old");

        let reduced = reduce_stream_shell(
            initial,
            StreamShellEvent::StartupLoaded(Ok(sample_startup_diagnostics())),
        );

        assert!(matches!(
            reduced.state.startup,
            StreamShellStartupState::Ready(_)
        ));
        assert_eq!(reduced.state.thread.cwd, "/tmp/root");
        assert_eq!(reduced.state.status_line, "startup ready");
        assert_eq!(
            reduced.effects,
            vec![StreamShellEffect::LoadRecentSessions {
                limit: POC_SESSION_PAGE_SIZE
            }]
        );
    }

    #[test]
    fn submit_pressed_emits_prompt_effect_and_clears_composer() {
        let initial = super::StreamShellState::new("/tmp/root");
        let reduced = reduce_stream_shell(
            initial,
            StreamShellEvent::ComposerChanged("ship it".to_string()),
        );

        let reduced = reduce_stream_shell(reduced.state, StreamShellEvent::SubmitPressed);

        assert_eq!(reduced.state.thread.turn_phase, StreamTurnPhase::Submitting);
        assert!(reduced.state.composer.is_empty());
        assert_eq!(reduced.state.status_line, "submitting prompt");
        assert_eq!(reduced.state.thread.transcript.len(), 1);
        assert_eq!(reduced.state.thread.transcript[0].text, "ship it");
        assert_eq!(
            reduced.effects,
            vec![StreamShellEffect::SubmitPrompt {
                cwd: "/tmp/root".to_string(),
                thread_id: None,
                prompt: "ship it".to_string(),
            }]
        );
    }

    #[test]
    fn conversation_load_replaces_thread_state() {
        let initial = super::StreamShellState::new("/tmp/root");

        let reduced = reduce_stream_shell(
            initial,
            StreamShellEvent::ConversationLoaded(Ok(ConversationSnapshot {
                thread_id: "thread-1".to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/project".to_string(),
                messages: vec![ConversationMessage::new(
                    ConversationMessageKind::Agent,
                    "loaded answer",
                    None,
                    Some("msg-1".to_string()),
                )],
                warnings: Vec::new(),
            })),
        );

        assert_eq!(reduced.state.thread.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(reduced.state.thread.title, "Loaded thread");
        assert_eq!(reduced.state.thread.cwd, "/tmp/project");
        assert_eq!(reduced.state.thread.transcript.len(), 1);
        assert_eq!(reduced.state.status_line, "thread loaded");
    }

    #[test]
    fn stream_updates_are_presented_without_ui_logic_in_the_reducer() {
        let initial = super::StreamShellState::new("/tmp/root");
        let reduced = reduce_stream_shell(
            initial,
            StreamShellEvent::StreamUpdate(ConversationStreamEvent::AgentMessageCompleted {
                item_id: "msg-1".to_string(),
                phase: Some("final_answer".to_string()),
                text: "done".to_string(),
            }),
        );

        let render_model = present_stream_shell(&reduced.state);

        assert!(
            render_model
                .transcript_lines
                .iter()
                .any(|line| line == "Codex:")
        );
        assert!(
            render_model
                .transcript_lines
                .iter()
                .any(|line| line == "  done")
        );
        assert_eq!(render_model.footer_status, "idle");
    }

    #[test]
    fn session_overlay_and_loaded_sessions_live_in_state_not_view_code() {
        let initial = super::StreamShellState::new("/tmp/root");
        let reduced = reduce_stream_shell(initial, StreamShellEvent::OpenSessionsOverlay);
        let reduced = reduce_stream_shell(
            reduced.state,
            StreamShellEvent::SessionsLoaded(Ok(RecentSessions {
                items: vec![sample_session()],
                warnings: Vec::new(),
                next_cursor: None,
            })),
        );

        assert_eq!(reduced.state.overlay, StreamShellOverlay::Sessions);
        assert!(matches!(
            reduced.state.sessions,
            StreamShellSessionsState::Ready(_)
        ));
        assert_eq!(reduced.state.status_line, "1 recent sessions loaded");
    }

    #[test]
    fn submit_prompt_effect_does_not_emit_duplicate_failed_events() {
        let port = Arc::new(FakeCodexAppServerPort::with_stream_mode(
            FakeStreamMode::FailAfterTerminalEvent,
        ));
        let handler = StreamShellEffectHandler::new(
            StartupService::new(port.clone()),
            SessionService::new(port.clone()),
            ConversationService::new(port),
        );
        let (event_tx, event_rx) = mpsc::channel();

        handler.execute(
            StreamShellEffect::SubmitPrompt {
                cwd: "/tmp/root".to_string(),
                thread_id: None,
                prompt: "ship it".to_string(),
            },
            event_tx,
        );

        let events = collect_stream_events(&event_rx);
        let failed_count = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    StreamShellEvent::StreamUpdate(ConversationStreamEvent::Failed { .. })
                )
            })
            .count();

        assert_eq!(failed_count, 1);
    }

    #[test]
    fn submit_prompt_effect_emits_completion_when_stream_succeeds() {
        let port = Arc::new(FakeCodexAppServerPort::with_stream_mode(
            FakeStreamMode::Succeed,
        ));
        let handler = StreamShellEffectHandler::new(
            StartupService::new(port.clone()),
            SessionService::new(port.clone()),
            ConversationService::new(port),
        );
        let (event_tx, event_rx) = mpsc::channel();

        handler.execute(
            StreamShellEffect::SubmitPrompt {
                cwd: "/tmp/root".to_string(),
                thread_id: None,
                prompt: "ship it".to_string(),
            },
            event_tx,
        );

        let events = collect_stream_events(&event_rx);

        assert_eq!(events.len(), 1);
        assert!(matches!(
            events.first(),
            Some(StreamShellEvent::StreamUpdate(
                ConversationStreamEvent::TurnCompleted { turn_id }
            )) if turn_id == "turn-1"
        ));
    }

    fn collect_stream_events(event_rx: &mpsc::Receiver<StreamShellEvent>) -> Vec<StreamShellEvent> {
        let mut events = Vec::new();

        while let Ok(event) = event_rx.recv_timeout(Duration::from_millis(200)) {
            let terminal = matches!(
                &event,
                StreamShellEvent::StreamUpdate(
                    ConversationStreamEvent::TurnCompleted { .. }
                        | ConversationStreamEvent::Failed { .. }
                )
            );
            events.push(event);
            if terminal {
                break;
            }
        }

        events
    }

    fn sample_startup_diagnostics() -> StartupDiagnostics {
        StartupDiagnostics {
            cwd: "/tmp/root".to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "/opt/homebrew/bin/codex".to_string(),
            workspace_ok: true,
            workspace_path: "/tmp/root".to_string(),
            workspace_detail: "git repo: /tmp/root".to_string(),
            initialize_ok: true,
            initialize_detail: "darwin / unix / codex".to_string(),
            account_ok: true,
            account_detail: "logged in".to_string(),
            warnings: Vec::new(),
            schema_snapshot: "native/schema/codex_app_server_protocol.v2.schemas.json".to_string(),
        }
    }

    fn sample_session() -> SessionSummary {
        SessionSummary {
            id: "thread-1".to_string(),
            name: Some("Loaded thread".to_string()),
            preview: "preview".to_string(),
            cwd: "/tmp/root".to_string(),
            source: "codex".to_string(),
            model_provider: "openai".to_string(),
            updated_at_epoch: 1_700_000_000,
            status_type: "ready".to_string(),
            path: "/tmp/root/thread.json".to_string(),
            git_branch: Some("main".to_string()),
        }
    }
}

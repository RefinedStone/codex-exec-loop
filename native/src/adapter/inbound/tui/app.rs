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
    is_turn_running: bool,
    active_turn_id: Option<String>,
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
            is_turn_running: false,
            active_turn_id: None,
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
            is_turn_running: false,
            active_turn_id: None,
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
        let ConversationState::Ready(conversation) = &mut self.conversation_state else {
            return;
        };
        if conversation.is_turn_running {
            return;
        }

        let prompt = conversation.input_buffer.trim().to_string();
        if prompt.is_empty() {
            return;
        }

        let thread_id = conversation.thread_id.clone();
        let cwd = conversation.cwd.clone();
        let is_new_thread = !conversation.has_active_thread();
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::User,
            prompt.clone(),
            None,
            None,
        ));
        conversation.refresh_conversation_lines();
        conversation.input_buffer.clear();
        conversation.is_turn_running = true;
        conversation.status_text = "starting turn".to_string();

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
                conversation.active_turn_id = Some(turn_id);
                conversation.is_turn_running = true;
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
                conversation.active_turn_id = None;
                conversation.is_turn_running = false;
                conversation.status_text = format!("turn completed: {turn_id}");
            }
            ConversationStreamEvent::Failed { message } => {
                conversation.active_turn_id = None;
                conversation.is_turn_running = false;
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
            ConversationState::Ready(conversation) if !conversation.is_turn_running
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
            Line::from(format!(
                "thread: {}",
                if conversation.has_active_thread() {
                    conversation.thread_id.as_str()
                } else {
                    "not started yet"
                }
            )),
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

    let conversation = Paragraph::new(build_conversation_lines(app))
        .block(Block::default().borders(Borders::ALL).title("Conversation"))
        .wrap(Wrap { trim: false });
    frame.render_widget(conversation, content_layout[0]);

    let activity = Paragraph::new(build_conversation_activity_lines(app))
        .block(Block::default().borders(Borders::ALL).title("Activity"))
        .wrap(Wrap { trim: false });
    frame.render_widget(activity, content_layout[1]);

    let input = Paragraph::new(build_input_lines(app))
        .block(Block::default().borders(Borders::ALL).title("Input"))
        .wrap(Wrap { trim: false });
    frame.render_widget(input, layout[2]);

    let help = Paragraph::new(vec![
        Line::from("Type your prompt and press Enter to send"),
        Line::from("Backspace: delete    b/Ctrl+C: back    q: quit"),
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
                    if conversation.is_turn_running {
                        "yes"
                    } else {
                        "no"
                    }
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
        ConversationState::Loading => vec![Line::from("Thread is still loading.")],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
        ConversationState::Ready(conversation) => {
            if conversation.is_turn_running {
                vec![
                    Line::from("Codex is still working on the current turn."),
                    Line::from("Wait for completion before sending another prompt."),
                ]
            } else if !conversation.has_active_thread() && conversation.input_buffer.is_empty() {
                vec![
                    Line::from("Type the first prompt for a new thread."),
                    Line::from("Press Enter to create the thread and send it."),
                ]
            } else if conversation.input_buffer.is_empty() {
                vec![
                    Line::from("Type a prompt here."),
                    Line::from("Press Enter to send."),
                ]
            } else {
                vec![Line::from(conversation.input_buffer.clone())]
            }
        }
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

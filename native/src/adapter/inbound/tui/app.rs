use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
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
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;
use crate::domain::startup_diagnostics::StartupDiagnostics;

const SESSION_PAGE_SIZE: usize = 10;

pub fn run() -> Result<()> {
    let codex_app_server_port: Arc<dyn CodexAppServerPort> = Arc::new(CodexAppServerAdapter::new(
        "codex-exec-loop-native",
        env!("CARGO_PKG_VERSION"),
    ));
    let startup_service = StartupService::new(codex_app_server_port.clone());
    let session_service = SessionService::new(codex_app_server_port);

    let mut app = NativeTuiApp::new(startup_service, session_service);
    app.start_startup_check();
    run_tui(app)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    SessionList,
    ConversationShell,
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

enum BackgroundMessage {
    StartupLoaded(Result<StartupDiagnostics, String>),
    SessionsLoaded(Result<RecentSessions, String>),
}

struct NativeTuiApp {
    current_screen: Screen,
    startup_state: StartupState,
    session_state: SessionState,
    selected_session_index: usize,
    active_session: Option<SessionSummary>,
    startup_service: StartupService,
    session_service: SessionService,
    tx: Sender<BackgroundMessage>,
    rx: Receiver<BackgroundMessage>,
}

impl NativeTuiApp {
    fn new(startup_service: StartupService, session_service: SessionService) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            current_screen: Screen::Home,
            startup_state: StartupState::Idle,
            session_state: SessionState::Idle,
            selected_session_index: 0,
            active_session: None,
            startup_service,
            session_service,
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
            }
        }
    }

    fn can_open_session_list(&self) -> bool {
        matches!(
            &self.startup_state,
            StartupState::Ready(diagnostics) if diagnostics.can_continue()
        )
    }

    fn open_session_list(&mut self) {
        self.current_screen = Screen::SessionList;
        self.active_session = None;
        self.start_session_load();
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
            self.active_session = Some(session);
            self.current_screen = Screen::ConversationShell;
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

        match app.current_screen {
            Screen::Home => match key.code {
                KeyCode::Char('q') => should_quit = true,
                KeyCode::Char('r') => app.start_startup_check(),
                KeyCode::Enter if app.can_open_session_list() => app.open_session_list(),
                _ => {}
            },
            Screen::SessionList => match key.code {
                KeyCode::Char('q') => should_quit = true,
                KeyCode::Char('b') => app.current_screen = Screen::Home,
                KeyCode::Char('r') => app.start_session_load(),
                KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
                KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
                KeyCode::Enter => app.open_conversation_shell(),
                _ => {}
            },
            Screen::ConversationShell => match key.code {
                KeyCode::Char('q') => should_quit = true,
                KeyCode::Char('b') => app.current_screen = Screen::SessionList,
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
        Line::from("Enter: open recent sessions"),
        Line::from("r: rerun checks    q: quit"),
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
        Line::from("Up/Down or j/k: move    Enter: open shell preview"),
        Line::from("r: reload    b: back    q: quit"),
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

    let selected_session = app.active_session.as_ref();

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Conversation Shell", Style::default().fg(Color::Cyan)),
            Span::raw(" / resume preview"),
        ]),
        Line::from("The next milestone will stream thread and turn events here."),
    ])
    .block(Block::default().borders(Borders::ALL).title("Shell"));
    frame.render_widget(header, layout[0]);

    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(layout[1]);

    let conversation_lines = match selected_session {
        Some(session) => vec![
            Line::from("Selected session preview"),
            Line::from(""),
            Line::from(session.preview_block()),
        ],
        None => vec![Line::from("No session selected.")],
    };
    let conversation = Paragraph::new(conversation_lines)
        .block(Block::default().borders(Borders::ALL).title("Conversation"))
        .wrap(Wrap { trim: false });
    frame.render_widget(conversation, content_layout[0]);

    let activity_lines = match selected_session {
        Some(session) => vec![
            Line::from(format!("session id: {}", session.id)),
            Line::from(format!("workspace: {}", session.workspace_label())),
            Line::from(format!("updated: {}", session.updated_at_label())),
            Line::from(format!("source: {}", session.source)),
            Line::from(format!("status: {}", session.status_type)),
        ],
        None => vec![Line::from("session metadata is not available")],
    };
    let activity = Paragraph::new(activity_lines)
        .block(Block::default().borders(Borders::ALL).title("Activity"))
        .wrap(Wrap { trim: true });
    frame.render_widget(activity, content_layout[1]);

    let roadmap = Paragraph::new(vec![
        Line::from("next step"),
        Line::from("- thread/start or existing-thread resume action"),
        Line::from("- turn/start input box"),
        Line::from("- streamed notifications rendered in place"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Roadmap"))
    .wrap(Wrap { trim: true });
    frame.render_widget(roadmap, layout[2]);

    let help = Paragraph::new(vec![
        Line::from("b: back to recent sessions"),
        Line::from("q: quit"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(help, layout[3]);
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

fn diagnostic_item(title: &str, ok: bool, detail: &str) -> ListItem<'static> {
    let marker = if ok { "[ok]" } else { "[warn]" };
    ListItem::new(format!("{marker} {title}: {detail}"))
}

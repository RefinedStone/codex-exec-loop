mod probe;

use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::probe::{StartupDiagnostics, run_startup_probe};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    ShellPreview,
}

#[derive(Debug, Clone)]
enum ProbeState {
    Idle,
    Running,
    Ready(StartupDiagnostics),
    Failed(String),
}

struct App {
    screen: Screen,
    probe_state: ProbeState,
    tx: Sender<Result<StartupDiagnostics, String>>,
    rx: Receiver<Result<StartupDiagnostics, String>>,
}

impl App {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            screen: Screen::Home,
            probe_state: ProbeState::Idle,
            tx,
            rx,
        }
    }

    fn start_probe(&mut self) {
        self.probe_state = ProbeState::Running;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = run_startup_probe().map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
    }

    fn poll_probe_result(&mut self) {
        while let Ok(result) = self.rx.try_recv() {
            self.probe_state = match result {
                Ok(diagnostics) => ProbeState::Ready(diagnostics),
                Err(message) => ProbeState::Failed(message),
            };
        }
    }

    fn can_continue(&self) -> bool {
        match &self.probe_state {
            ProbeState::Ready(diagnostics) => diagnostics.can_continue(),
            _ => false,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let mut app = App::new();
    app.start_probe();
    run_tui(app)
}

fn run_tui(mut app: App) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut should_quit = false;
    while !should_quit {
        app.poll_probe_result();
        terminal.draw(|frame| draw(frame, &app))?;

        if event::poll(Duration::from_millis(100))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Char('q') => should_quit = true,
                KeyCode::Char('r') => app.start_probe(),
                KeyCode::Enter if app.screen == Screen::Home && app.can_continue() => {
                    app.screen = Screen::ShellPreview;
                }
                KeyCode::Char('b') if app.screen == Screen::ShellPreview => {
                    app.screen = Screen::Home;
                }
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn draw(frame: &mut Frame<'_>, app: &App) {
    match app.screen {
        Screen::Home => draw_home(frame, app),
        Screen::ShellPreview => draw_shell_preview(frame, app),
    }
}

fn draw_home(frame: &mut Frame<'_>, app: &App) {
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
        Line::from("Rust TUI prototype for Codex app-server"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Welcome"));
    frame.render_widget(title, layout[0]);

    let summary = match &app.probe_state {
        ProbeState::Idle => vec![
            Line::from("status: idle"),
            Line::from("first probe has not started"),
        ],
        ProbeState::Running => vec![
            Line::from(vec![
                Span::styled("status: ", Style::default().fg(Color::Gray)),
                Span::styled("running checks", Style::default().fg(Color::Yellow)),
            ]),
            Line::from("probing codex binary, app-server handshake, account state, and cwd"),
        ],
        ProbeState::Ready(diag) => vec![
            Line::from(vec![
                Span::styled("status: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    if diag.can_continue() {
                        "ready"
                    } else {
                        "needs attention"
                    },
                    Style::default().fg(if diag.can_continue() {
                        Color::Green
                    } else {
                        Color::Yellow
                    }),
                ),
            ]),
            Line::from(format!("cwd: {}", diag.cwd)),
        ],
        ProbeState::Failed(message) => vec![
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

    let warnings = build_warning_lines(app);
    let warning_widget = Paragraph::new(warnings)
        .block(Block::default().borders(Borders::ALL).title("Warnings"))
        .wrap(Wrap { trim: true });
    frame.render_widget(warning_widget, layout[3]);

    let help = Paragraph::new(vec![
        Line::from("Enter: continue to shell preview"),
        Line::from("r: rerun checks    q: quit"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(help, layout[4]);
}

fn draw_shell_preview(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(4),
            Constraint::Length(3),
        ])
        .split(area);

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("codex-exec-loop", Style::default().fg(Color::Cyan)),
            Span::raw(" / shell preview"),
        ]),
        Line::from("Phase 1 placeholder: startup checks succeeded, conversation shell is next."),
    ])
    .block(Block::default().borders(Borders::ALL).title("Main"));
    frame.render_widget(header, layout[0]);

    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(layout[1]);

    let conversation = Paragraph::new(vec![
        Line::from("Conversation pane is not wired yet."),
        Line::from("Next step: thread list, thread start, turn start, streamed notifications."),
    ])
    .block(Block::default().borders(Borders::ALL).title("Conversation"))
    .wrap(Wrap { trim: true });
    frame.render_widget(conversation, middle[0]);

    let activity = Paragraph::new(shell_preview_lines(app))
        .block(Block::default().borders(Borders::ALL).title("Activity"))
        .wrap(Wrap { trim: true });
    frame.render_widget(activity, middle[1]);

    let input = Paragraph::new("Input bar placeholder")
        .block(Block::default().borders(Borders::ALL).title("Input"));
    frame.render_widget(input, layout[2]);

    let footer = Paragraph::new("b: back    q: quit")
        .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(footer, layout[3]);
}

fn build_check_items(app: &App) -> Vec<ListItem<'static>> {
    match &app.probe_state {
        ProbeState::Idle => vec![ListItem::new("waiting to start probe")],
        ProbeState::Running => vec![
            ListItem::new("codex binary: running"),
            ListItem::new("app-server initialize: running"),
            ListItem::new("account/read: running"),
            ListItem::new("workspace probe: running"),
        ],
        ProbeState::Failed(message) => vec![
            styled_item("codex binary: failed", Color::Red),
            ListItem::new(message.clone()),
        ],
        ProbeState::Ready(diag) => vec![
            check_item(
                "codex binary",
                diag.codex_binary_ok,
                Some(diag.codex_binary_detail.as_str()),
            ),
            check_item(
                "workspace",
                diag.workspace_ok,
                Some(diag.workspace_detail.as_str()),
            ),
            check_item(
                "app-server initialize",
                diag.initialize_ok,
                Some(diag.initialize_detail.as_str()),
            ),
            check_item(
                "account/read",
                diag.account_ok,
                Some(diag.account_detail.as_str()),
            ),
        ],
    }
}

fn build_warning_lines(app: &App) -> Vec<Line<'static>> {
    match &app.probe_state {
        ProbeState::Ready(diag) => {
            if diag.warnings.is_empty() {
                vec![Line::from("no warnings")]
            } else {
                diag.warnings
                    .iter()
                    .map(|warning| Line::from(warning.clone()))
                    .collect()
            }
        }
        ProbeState::Failed(message) => vec![Line::from(message.clone())],
        ProbeState::Running => vec![Line::from("collecting warnings from app-server")],
        ProbeState::Idle => vec![Line::from("no probe run yet")],
    }
}

fn shell_preview_lines(app: &App) -> Vec<Line<'static>> {
    match &app.probe_state {
        ProbeState::Ready(diag) => vec![
            Line::from(format!("codex: {}", diag.codex_binary_detail)),
            Line::from(format!("account: {}", diag.account_detail)),
            Line::from(format!("platform: {}", diag.initialize_detail)),
            Line::from(format!("schema: {}", diag.schema_snapshot)),
        ],
        ProbeState::Failed(message) => vec![Line::from(message.clone())],
        _ => vec![Line::from("startup checks incomplete")],
    }
}

fn styled_item(text: impl Into<String>, color: Color) -> ListItem<'static> {
    let owned = text.into();
    ListItem::new(Line::from(Span::styled(
        owned,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )))
}

fn check_item(label: &str, ok: bool, detail: Option<&str>) -> ListItem<'static> {
    let status = if ok { "ok" } else { "fail" };
    let color = if ok { Color::Green } else { Color::Red };
    let text = match detail {
        Some(detail) if !detail.is_empty() => format!("{label}: {status} ({detail})"),
        _ => format!("{label}: {status}"),
    };
    styled_item(text, color)
}

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::{MoveToNextLine, Show};
use crossterm::event;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::TerminalOptions;
use ratatui::Viewport;
use ratatui::backend::CrosstermBackend;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget, Wrap};

use super::shell_frontend::ShellFrontend;
use super::shell_rendering::draw;
use super::shell_runtime::ShellRuntime;
use super::{ConversationState, INLINE_VIEWPORT_HEIGHT, NativeTuiApp, ShellFrontendMode};

pub(super) fn run(mut runtime: ShellRuntime, frontend: ShellFrontend) -> Result<()> {
    let _restore_guard = TerminalRestoreGuard::activate(frontend)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = build_terminal(backend, frontend.mode())?;
    let mut inline_history = InlineHistoryState::default();
    run_event_loop(&mut terminal, &mut runtime, frontend, &mut inline_history)
}

fn build_terminal(
    backend: CrosstermBackend<io::Stdout>,
    mode: ShellFrontendMode,
) -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    match mode {
        ShellFrontendMode::InlineMainBuffer => Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(INLINE_VIEWPORT_HEIGHT),
            },
        ),
        ShellFrontendMode::AlternateScreen => Terminal::new(backend),
    }
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    runtime: &mut ShellRuntime,
    frontend: ShellFrontend,
    inline_history: &mut InlineHistoryState,
) -> Result<()> {
    while !runtime.should_quit() {
        runtime.poll_background_messages();
        if runtime.take_redraw_request() {
            sync_inline_history(terminal, runtime, frontend.mode(), inline_history)?;
            terminal.draw(|frame| draw(frame, runtime.app_mut(), frontend.mode()))?;
        }

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        runtime.handle_terminal_event(event::read()?);
    }

    Ok(())
}

fn sync_inline_history(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    runtime: &mut ShellRuntime,
    mode: ShellFrontendMode,
    inline_history: &mut InlineHistoryState,
) -> io::Result<()> {
    if mode != ShellFrontendMode::InlineMainBuffer {
        return Ok(());
    }

    let current_lines = current_inline_history_lines(runtime.app_mut());
    inline_history.sync(terminal, &current_lines)
}

fn current_inline_history_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Ready(conversation) => conversation.cached_conversation_lines.clone(),
        ConversationState::Loading | ConversationState::Failed(_) => Vec::new(),
    }
}

#[derive(Default)]
struct InlineHistoryState {
    rendered_lines: Vec<Line<'static>>,
}

impl InlineHistoryState {
    fn sync(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        current_lines: &[Line<'static>],
    ) -> io::Result<()> {
        let pending_lines = self.pending_lines(current_lines);
        if !pending_lines.is_empty() {
            insert_inline_history_lines(terminal, &pending_lines)?;
        }
        self.rendered_lines = current_lines.to_vec();
        Ok(())
    }

    fn pending_lines(&self, current_lines: &[Line<'static>]) -> Vec<Line<'static>> {
        if current_lines.is_empty() {
            return Vec::new();
        }

        if current_lines.starts_with(self.rendered_lines.as_slice()) {
            return current_lines[self.rendered_lines.len()..].to_vec();
        }

        current_lines.to_vec()
    }
}

fn insert_inline_history_lines(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    lines: &[Line<'static>],
) -> io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }

    let width = terminal.size()?.width;
    if width == 0 {
        return Ok(());
    }

    let height = count_rendered_history_rows(lines, width).min(u16::MAX as usize) as u16;
    if height == 0 {
        return Ok(());
    }

    terminal.insert_before(height, |buffer| {
        Paragraph::new(lines.to_vec())
            .wrap(Wrap { trim: false })
            .render(buffer.area, buffer);
    })
}

fn count_rendered_history_rows(lines: &[Line<'static>], width: u16) -> usize {
    if width == 0 {
        return 0;
    }

    lines
        .iter()
        .map(|line| {
            let line_width = line.width();
            if line_width == 0 {
                1
            } else {
                line_width.div_ceil(width as usize)
            }
        })
        .sum()
}

struct TerminalRestoreGuard {
    use_alternate_screen: bool,
}

impl TerminalRestoreGuard {
    fn activate(frontend: ShellFrontend) -> Result<Self> {
        let use_alternate_screen = frontend.mode().uses_alternate_screen();
        enable_raw_mode()?;
        let guard = Self {
            use_alternate_screen,
        };
        let mut stdout = io::stdout();
        if use_alternate_screen {
            execute!(stdout, EnterAlternateScreen)?;
        }
        Ok(guard)
    }
}

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        if self.use_alternate_screen {
            let _ = execute!(stdout, LeaveAlternateScreen);
        } else {
            let _ = execute!(stdout, MoveToNextLine(1));
        }
        let _ = execute!(stdout, Show);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::text::Line;

    use super::InlineHistoryState;

    #[test]
    fn pending_lines_returns_only_new_suffix_for_appended_history() {
        let state = InlineHistoryState {
            rendered_lines: vec![
                Line::from("User:"),
                Line::from("  first prompt"),
                Line::from(""),
            ],
        };
        let current_lines = vec![
            Line::from("User:"),
            Line::from("  first prompt"),
            Line::from(""),
            Line::from("Status:"),
            Line::from("  turn started"),
            Line::from(""),
        ];

        let pending = state.pending_lines(&current_lines);

        assert_eq!(
            pending,
            vec![
                Line::from("Status:"),
                Line::from("  turn started"),
                Line::from(""),
            ]
        );
    }

    #[test]
    fn pending_lines_replays_full_history_after_reset() {
        let state = InlineHistoryState {
            rendered_lines: vec![
                Line::from("User:"),
                Line::from("  old thread"),
                Line::from(""),
            ],
        };
        let current_lines = vec![
            Line::from("Status:"),
            Line::from("  thread opened: thread-2 / Loaded thread"),
            Line::from(""),
        ];

        let pending = state.pending_lines(&current_lines);

        assert_eq!(pending, current_lines);
    }
}

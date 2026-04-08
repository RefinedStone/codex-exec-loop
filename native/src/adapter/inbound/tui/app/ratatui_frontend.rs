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
use ratatui::backend::CrosstermBackend;

use super::shell_frontend::ShellFrontend;
use super::shell_rendering::draw;
use super::shell_runtime::ShellRuntime;

pub(super) fn run(mut runtime: ShellRuntime, frontend: ShellFrontend) -> Result<()> {
    let _restore_guard = TerminalRestoreGuard::activate(frontend)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    run_event_loop(&mut terminal, &mut runtime, frontend)
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    runtime: &mut ShellRuntime,
    frontend: ShellFrontend,
) -> Result<()> {
    while !runtime.should_quit() {
        runtime.poll_background_messages();
        if runtime.take_redraw_request() {
            terminal.draw(|frame| draw(frame, runtime.app_mut(), frontend.mode()))?;
        }

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        runtime.handle_terminal_event(event::read()?);
    }

    Ok(())
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

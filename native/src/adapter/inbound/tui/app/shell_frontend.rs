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

use super::ALT_SCREEN_ENV_VAR;
use super::shell_rendering::draw;
use super::shell_runtime::ShellRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShellFrontendMode {
    InlineMainBuffer,
    AlternateScreen,
}

impl ShellFrontendMode {
    pub(super) fn from_environment() -> Self {
        Self::from_env_value(std::env::var(ALT_SCREEN_ENV_VAR).ok().as_deref())
    }

    fn from_env_value(value: Option<&str>) -> Self {
        if value.is_some_and(env_flag_is_truthy) {
            Self::AlternateScreen
        } else {
            Self::InlineMainBuffer
        }
    }
}

pub(super) fn run(runtime: ShellRuntime, mode: ShellFrontendMode) -> Result<()> {
    run_ratatui_frontend(runtime, matches!(mode, ShellFrontendMode::AlternateScreen))
}

fn run_ratatui_frontend(mut runtime: ShellRuntime, use_alternate_screen: bool) -> Result<()> {
    let _restore_guard = TerminalRestoreGuard::activate(use_alternate_screen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    run_event_loop(&mut terminal, &mut runtime)
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    runtime: &mut ShellRuntime,
) -> Result<()> {
    while !runtime.should_quit() {
        runtime.poll_background_messages();
        terminal.draw(|frame| draw(frame, runtime.app_mut()))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        runtime.handle_terminal_event(event::read()?);
    }

    Ok(())
}

fn env_flag_is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

struct TerminalRestoreGuard {
    use_alternate_screen: bool,
}

impl TerminalRestoreGuard {
    fn activate(use_alternate_screen: bool) -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if use_alternate_screen {
            execute!(stdout, EnterAlternateScreen)?;
        }
        Ok(Self {
            use_alternate_screen,
        })
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
    use super::ShellFrontendMode;

    #[test]
    fn shell_frontend_mode_defaults_to_inline_main_buffer() {
        assert_eq!(
            ShellFrontendMode::from_env_value(None),
            ShellFrontendMode::InlineMainBuffer
        );
        assert_eq!(
            ShellFrontendMode::from_env_value(Some("0")),
            ShellFrontendMode::InlineMainBuffer
        );
        assert_eq!(
            ShellFrontendMode::from_env_value(Some("no")),
            ShellFrontendMode::InlineMainBuffer
        );
    }

    #[test]
    fn shell_frontend_mode_accepts_truthy_alt_screen_flag() {
        assert_eq!(
            ShellFrontendMode::from_env_value(Some("1")),
            ShellFrontendMode::AlternateScreen
        );
        assert_eq!(
            ShellFrontendMode::from_env_value(Some(" true ")),
            ShellFrontendMode::AlternateScreen
        );
        assert_eq!(
            ShellFrontendMode::from_env_value(Some("ON")),
            ShellFrontendMode::AlternateScreen
        );
    }

    #[test]
    fn shell_frontend_mode_ignores_unrecognized_flag_values() {
        assert_eq!(
            ShellFrontendMode::from_env_value(Some("maybe")),
            ShellFrontendMode::InlineMainBuffer
        );
    }
}

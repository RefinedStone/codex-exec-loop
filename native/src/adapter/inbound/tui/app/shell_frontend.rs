use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::MoveToNextLine;
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
    match mode {
        ShellFrontendMode::InlineMainBuffer => run_inline_main_buffer_frontend(runtime),
        ShellFrontendMode::AlternateScreen => run_alternate_screen_frontend(runtime),
    }
}

fn run_inline_main_buffer_frontend(runtime: ShellRuntime) -> Result<()> {
    run_ratatui_frontend(runtime, false)
}

fn run_alternate_screen_frontend(runtime: ShellRuntime) -> Result<()> {
    run_ratatui_frontend(runtime, true)
}

fn run_ratatui_frontend(mut runtime: ShellRuntime, use_alternate_screen: bool) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if use_alternate_screen {
        execute!(stdout, EnterAlternateScreen)?;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_event_loop(&mut terminal, &mut runtime);

    disable_raw_mode()?;
    if use_alternate_screen {
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    } else {
        execute!(terminal.backend_mut(), MoveToNextLine(1))?;
    }
    terminal.show_cursor()?;
    result
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

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::{MoveToNextLine, Show};
use crossterm::event;
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use super::inline_terminal_adapter::{
    InlineTerminalAdapter, InlineTerminalBackend, terminal_options_for_render_mode,
};
use super::shell_runtime::ShellRuntime;

pub(super) fn run(mut runtime: ShellRuntime) -> Result<()> {
    let _restore_guard = TerminalRestoreGuard::activate()?;
    let backend = CrosstermBackend::new(io::stdout());
    let render_mode = runtime.app_mut().inline_history_render_mode;
    let terminal = build_terminal(backend, render_mode)?;
    let mut adapter = InlineTerminalAdapter::new(terminal);
    run_event_loop(&mut adapter, &mut runtime)
}

fn build_terminal(
    backend: CrosstermBackend<io::Stdout>,
    render_mode: super::InlineHistoryRenderMode,
) -> io::Result<Terminal<InlineTerminalBackend<CrosstermBackend<io::Stdout>>>> {
    Terminal::with_options(
        InlineTerminalBackend::new(backend),
        terminal_options_for_render_mode(render_mode),
    )
}

fn run_event_loop(
    adapter: &mut InlineTerminalAdapter<InlineTerminalBackend<CrosstermBackend<io::Stdout>>>,
    runtime: &mut ShellRuntime,
) -> Result<()> {
    while !runtime.should_quit() {
        runtime.poll_background_messages();
        if runtime.take_due_draw_request(std::time::Instant::now()) {
            adapter.draw_inline_transaction(runtime)?;
        }

        let poll_timeout =
            runtime.next_event_poll_timeout(std::time::Instant::now(), Duration::from_millis(100));
        if !event::poll(poll_timeout)? {
            continue;
        }

        runtime.handle_terminal_event(event::read()?);
    }

    Ok(())
}

struct TerminalRestoreGuard;

impl TerminalRestoreGuard {
    fn activate() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, event::EnableFocusChange) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        Ok(Self)
    }
}

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, event::DisableFocusChange);
        let _ = disable_raw_mode();
        let _ = execute!(stdout, MoveToNextLine(1));
        let _ = execute!(stdout, Show);
    }
}

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

use crate::adapter::inbound::tui::shell_chrome::ShellOverlay;

use super::shell_frontend::ShellFrontend;
use super::shell_presentation::build_inline_tail_lines;
use super::shell_rendering::draw;
use super::shell_runtime::ShellRuntime;
use super::{
    ConversationState, INLINE_VIEWPORT_HEIGHT, MAX_CONVERSATION_HISTORY_LINES, NativeTuiApp,
    ShellFrontendMode,
};

pub(super) fn run(mut runtime: ShellRuntime, frontend: ShellFrontend) -> Result<()> {
    let _restore_guard = TerminalRestoreGuard::activate(frontend)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut inline_viewport = InlineViewportState::default();
    let mut terminal = build_terminal(backend, frontend.mode())?;
    run_event_loop(&mut terminal, &mut runtime, frontend, &mut inline_viewport)
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
    inline_viewport: &mut InlineViewportState,
) -> Result<()> {
    while !runtime.should_quit() {
        runtime.poll_background_messages();
        if runtime.take_redraw_request() {
            let should_draw =
                sync_inline_viewport(terminal, runtime, frontend.mode(), inline_viewport)?;
            if should_draw {
                terminal.draw(|frame| draw(frame, runtime.app_mut(), frontend.mode()))?;
            }
        }

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        runtime.handle_terminal_event(event::read()?);
    }

    Ok(())
}

fn sync_inline_viewport(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    runtime: &mut ShellRuntime,
    mode: ShellFrontendMode,
    inline_viewport: &mut InlineViewportState,
) -> io::Result<bool> {
    if mode != ShellFrontendMode::InlineMainBuffer {
        return Ok(true);
    }

    let current_lines = current_inline_history_lines(runtime.app_mut());
    inline_viewport.history.sync(terminal, &current_lines)?;

    let terminal_size = terminal.size()?;
    Ok(inline_viewport.should_draw_inline_frame(
        runtime.app_mut(),
        terminal_size.width,
        terminal_size.height,
    ))
}

fn current_inline_history_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Ready(conversation) => conversation.cached_conversation_lines.clone(),
        ConversationState::Loading | ConversationState::Failed(_) => Vec::new(),
    }
}

struct InlineViewportState {
    history: InlineHistoryState,
    last_tail_frame: Option<InlineTailFrameSignature>,
}

impl Default for InlineViewportState {
    fn default() -> Self {
        Self {
            history: InlineHistoryState::default(),
            last_tail_frame: None,
        }
    }
}

impl InlineViewportState {
    fn should_draw_inline_frame(
        &mut self,
        app: &NativeTuiApp,
        terminal_width: u16,
        terminal_height: u16,
    ) -> bool {
        if app.shell_overlay != ShellOverlay::Hidden || app.is_exit_confirmation_visible() {
            self.last_tail_frame = None;
            return true;
        }

        let next_signature = InlineTailFrameSignature {
            terminal_width,
            terminal_height,
            lines: build_inline_tail_lines(app),
        };
        let should_draw = self.last_tail_frame.as_ref() != Some(&next_signature);
        self.last_tail_frame = Some(next_signature);
        should_draw
    }
}

#[derive(Clone, PartialEq, Eq)]
struct InlineTailFrameSignature {
    terminal_width: u16,
    terminal_height: u16,
    lines: Vec<Line<'static>>,
}

#[derive(Default)]
struct InlineHistoryState {
    rendered_lines: Vec<Line<'static>>,
}

const MIN_SHIFTED_HISTORY_OVERLAP: usize = 8;

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

        if let Some(overlap_len) = self.shifted_window_overlap_len(current_lines) {
            return current_lines[overlap_len..].to_vec();
        }

        current_lines.to_vec()
    }

    fn shifted_window_overlap_len(&self, current_lines: &[Line<'static>]) -> Option<usize> {
        if self.rendered_lines.len() != MAX_CONVERSATION_HISTORY_LINES
            || current_lines.len() != MAX_CONVERSATION_HISTORY_LINES
        {
            return None;
        }

        let max_overlap = self.rendered_lines.len().min(current_lines.len());
        if max_overlap < MIN_SHIFTED_HISTORY_OVERLAP {
            return None;
        }

        (MIN_SHIFTED_HISTORY_OVERLAP..=max_overlap)
            .rev()
            .find(|overlap_len| {
                self.rendered_lines[self.rendered_lines.len() - overlap_len..]
                    == current_lines[..*overlap_len]
            })
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
    use std::sync::Arc;

    use anyhow::Result;
    use ratatui::text::Line;

    use super::{InlineHistoryState, InlineViewportState};
    use crate::adapter::inbound::tui::app::{MAX_CONVERSATION_HISTORY_LINES, NativeTuiApp};
    use crate::adapter::inbound::tui::shell_chrome::ShellOverlay;
    use crate::application::port::outbound::codex_app_server_port::{
        AppServerStartupContext, CodexAppServerPort,
    };
    use crate::application::port::outbound::followup_template_port::{
        FollowupTemplatePort, WorkspaceFollowupTemplateRecord,
    };
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::followup_template_service::FollowupTemplateService;
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};
    use crate::domain::recent_sessions::RecentSessions;

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

    #[test]
    fn pending_lines_only_inserts_new_suffix_for_shifted_history_window() {
        let state = InlineHistoryState {
            rendered_lines: (0..MAX_CONVERSATION_HISTORY_LINES)
                .map(|idx| Line::from(format!("line {idx}")))
                .collect(),
        };
        let current_lines = (3..MAX_CONVERSATION_HISTORY_LINES + 3)
            .map(|idx| Line::from(format!("line {idx}")))
            .collect::<Vec<_>>();

        let pending = state.pending_lines(&current_lines);

        assert_eq!(
            pending,
            vec![
                Line::from(format!("line {}", MAX_CONVERSATION_HISTORY_LINES)),
                Line::from(format!("line {}", MAX_CONVERSATION_HISTORY_LINES + 1)),
                Line::from(format!("line {}", MAX_CONVERSATION_HISTORY_LINES + 2)),
            ]
        );
    }

    #[test]
    fn pending_lines_does_not_treat_small_overlap_as_shifted_history() {
        let state = InlineHistoryState {
            rendered_lines: vec![
                Line::from("User:"),
                Line::from("  old prompt"),
                Line::from(""),
                Line::from("Agent:"),
                Line::from("  old answer"),
                Line::from(""),
                Line::from("Status:"),
                Line::from("  completed"),
            ],
        };
        let current_lines = vec![
            Line::from("Status:"),
            Line::from("  completed"),
            Line::from("User:"),
            Line::from("  brand new thread"),
            Line::from(""),
        ];

        let pending = state.pending_lines(&current_lines);

        assert_eq!(pending, current_lines);
    }

    #[test]
    fn pending_lines_does_not_shift_uncapped_history_window_even_with_large_overlap() {
        let state = InlineHistoryState {
            rendered_lines: vec![
                Line::from("Status:"),
                Line::from("  queued"),
                Line::from(""),
                Line::from("Agent:"),
                Line::from("  first answer"),
                Line::from(""),
                Line::from("Status:"),
                Line::from("  completed"),
                Line::from("User:"),
                Line::from("  old tail"),
                Line::from(""),
            ],
        };
        let current_lines = vec![
            Line::from("Status:"),
            Line::from("  queued"),
            Line::from(""),
            Line::from("Agent:"),
            Line::from("  first answer"),
            Line::from(""),
            Line::from("Status:"),
            Line::from("  completed"),
            Line::from("User:"),
            Line::from("  replacement thread"),
            Line::from(""),
        ];

        let pending = state.pending_lines(&current_lines);

        assert_eq!(pending, current_lines);
    }

    #[test]
    fn hidden_inline_tail_skips_redundant_frame_draws() {
        let app = make_test_app();
        let mut inline_viewport = InlineViewportState::default();

        assert!(inline_viewport.should_draw_inline_frame(&app, 80, 24));
        assert!(!inline_viewport.should_draw_inline_frame(&app, 80, 24));
        assert!(inline_viewport.should_draw_inline_frame(&app, 96, 24));
    }

    #[test]
    fn overlay_cycle_resets_hidden_tail_redraw_cache() {
        let mut app = make_test_app();
        let mut inline_viewport = InlineViewportState::default();

        assert!(inline_viewport.should_draw_inline_frame(&app, 80, 24));
        assert!(!inline_viewport.should_draw_inline_frame(&app, 80, 24));

        app.shell_overlay = ShellOverlay::Startup;
        assert!(inline_viewport.should_draw_inline_frame(&app, 80, 24));

        app.shell_overlay = ShellOverlay::Hidden;
        assert!(inline_viewport.should_draw_inline_frame(&app, 80, 24));
    }

    struct FakeCodexAppServerPort;

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
                runtime_notices: Vec::new(),
            })
        }

        fn run_new_thread_stream(
            &self,
            _cwd: &str,
            _prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            Ok(())
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            Ok(())
        }
    }

    struct FakeFollowupTemplatePort;

    impl FollowupTemplatePort for FakeFollowupTemplatePort {
        fn load_workspace_templates(
            &self,
            _workspace_dir: &str,
        ) -> Result<Vec<WorkspaceFollowupTemplateRecord>> {
            Ok(Vec::new())
        }
    }

    fn make_test_app() -> NativeTuiApp {
        let codex_port = Arc::new(FakeCodexAppServerPort);
        let followup_port = Arc::new(FakeFollowupTemplatePort);
        NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            FollowupTemplateService::new(followup_port),
        )
    }
}

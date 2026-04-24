use std::io;
use std::ops::Range;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::{MoveToNextLine, Show};
use crossterm::event;
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::TerminalOptions;
use ratatui::Viewport;
use ratatui::backend::ClearType;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Rect, Size};
use ratatui::text::Line;

use crate::adapter::inbound::tui::shell_chrome::ShellOverlay;

use super::history_insertion::{
    HistoryInsertionAdapter, HistoryInsertionMode, count_rendered_history_rows,
};
use super::shell_presentation::{
    build_inline_tail_view, build_startup_banner_lines, format_conversation_lines_with_debug,
};
use super::shell_rendering::{draw, prepare_render_state};
use super::shell_runtime::ShellRuntime;
use super::{
    ConversationState, INLINE_VIEWPORT_HEIGHT, InlineHistoryRenderMode,
    MAX_CONVERSATION_HISTORY_LINES, NativeTuiApp, ShellFrontendMode,
};

pub(super) fn run(mut runtime: ShellRuntime) -> Result<()> {
    let _restore_guard = TerminalRestoreGuard::activate()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut inline_terminal = InlineTerminalState::default();
    let render_mode = runtime.app_mut().inline_history_render_mode;
    let mut terminal = build_terminal(backend, render_mode)?;
    run_event_loop(&mut terminal, &mut runtime, &mut inline_terminal)
}

fn build_terminal(
    backend: CrosstermBackend<io::Stdout>,
    render_mode: InlineHistoryRenderMode,
) -> io::Result<Terminal<InlineTerminalBackend<CrosstermBackend<io::Stdout>>>> {
    Terminal::with_options(
        InlineTerminalBackend::new(backend),
        terminal_options_for_render_mode(render_mode),
    )
}

pub(super) fn terminal_options_for_render_mode(
    render_mode: InlineHistoryRenderMode,
) -> TerminalOptions {
    let viewport = match render_mode {
        InlineHistoryRenderMode::HostScrollback => Viewport::Inline(INLINE_VIEWPORT_HEIGHT),
        InlineHistoryRenderMode::ViewportReplay => Viewport::Inline(INLINE_VIEWPORT_HEIGHT),
    };
    TerminalOptions { viewport }
}

fn run_event_loop(
    terminal: &mut Terminal<InlineTerminalBackend<CrosstermBackend<io::Stdout>>>,
    runtime: &mut ShellRuntime,
    inline_terminal: &mut InlineTerminalState,
) -> Result<()> {
    while !runtime.should_quit() {
        runtime.poll_background_messages();
        if runtime.take_due_draw_request(std::time::Instant::now()) {
            draw_inline_transaction(terminal, runtime, inline_terminal)?;
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

fn draw_inline_transaction<B: InlineResizeBackend>(
    terminal: &mut Terminal<B>,
    runtime: &mut ShellRuntime,
    inline_terminal: &mut InlineTerminalState,
) -> Result<(), B::Error> {
    if sync_inline_viewport(terminal, runtime, inline_terminal)? {
        draw_inline_frame(terminal, runtime, inline_terminal)?;
    }
    Ok(())
}

fn draw_inline_frame<B: InlineResizeBackend>(
    terminal: &mut Terminal<B>,
    runtime: &mut ShellRuntime,
    inline_terminal: &mut InlineTerminalState,
) -> Result<(), B::Error> {
    terminal
        .backend_mut()
        .set_resize_append_lines_suppressed(true);
    let result = terminal
        .draw(|frame| {
            let frame_area = frame.area();
            let app = runtime.app_mut();
            prepare_render_state(app, ShellFrontendMode::InlineMainBuffer, frame_area);
            draw(frame, app, ShellFrontendMode::InlineMainBuffer);
        })
        .map(|_| ());
    terminal
        .backend_mut()
        .set_resize_append_lines_suppressed(false);
    result?;
    let terminal_size = terminal.size()?;
    inline_terminal.mark_frame_drawn(terminal_size);
    Ok(())
}

fn sync_inline_viewport<B: InlineResizeBackend>(
    terminal: &mut Terminal<B>,
    runtime: &mut ShellRuntime,
    inline_terminal: &mut InlineTerminalState,
) -> Result<bool, B::Error> {
    let app = runtime.app_mut();
    let render_mode = app.inline_history_render_mode;
    let insert_mode = app.history_insert_mode;
    autoresize_inline_viewport(terminal)?;
    let terminal_size = terminal.size()?;
    inline_terminal.record_terminal_size(terminal_size);
    inline_terminal.insert_mode = insert_mode;
    let current_lines = current_inline_history_lines(app);
    let writes_host_scrollback = render_mode.writes_host_scrollback();
    let history_sync = if writes_host_scrollback {
        inline_terminal
            .history
            .sync(terminal, &current_lines, insert_mode)?
    } else {
        inline_terminal.history.remember(&current_lines);
        InlineHistorySyncResult::default()
    };
    inline_terminal.record_history_sync(&current_lines, history_sync);
    if history_sync.inserted() {
        inline_terminal.invalidate_back_buffer(terminal);
    }

    let terminal_size = terminal.size()?;
    inline_terminal.record_terminal_size(terminal_size);
    let tail_frame_changed = inline_terminal.should_draw_inline_frame(
        runtime.app_mut(),
        terminal_size.width,
        terminal_size.height,
    );
    Ok(history_sync.inserted() || tail_frame_changed)
}

fn autoresize_inline_viewport<B: InlineResizeBackend>(
    terminal: &mut Terminal<B>,
) -> Result<(), B::Error> {
    terminal
        .backend_mut()
        .set_resize_append_lines_suppressed(true);
    let result = terminal.autoresize();
    terminal
        .backend_mut()
        .set_resize_append_lines_suppressed(false);
    result
}

fn current_inline_history_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    if let Some(startup_banner_lines) = build_startup_banner_lines(app, None) {
        return startup_banner_lines;
    }

    match &app.conversation_state {
        ConversationState::Ready(conversation) => {
            if app.planner_shows_debug_details() {
                format_conversation_lines_with_debug(&conversation.messages, true)
            } else {
                conversation.cached_conversation_lines.clone()
            }
        }
        ConversationState::Loading | ConversationState::Failed(_) => Vec::new(),
    }
}

trait InlineResizeBackend: Backend {
    fn set_resize_append_lines_suppressed(&mut self, suppressed: bool);
}

pub(super) struct InlineTerminalBackend<B> {
    inner: B,
    suppress_resize_append_lines: bool,
}

impl<B> InlineTerminalBackend<B> {
    pub(super) fn new(inner: B) -> Self {
        Self {
            inner,
            suppress_resize_append_lines: false,
        }
    }

    #[cfg(test)]
    pub(super) fn inner(&self) -> &B {
        &self.inner
    }

    #[cfg(test)]
    pub(super) fn inner_mut(&mut self) -> &mut B {
        &mut self.inner
    }
}

impl<B: Backend> InlineResizeBackend for InlineTerminalBackend<B> {
    fn set_resize_append_lines_suppressed(&mut self, suppressed: bool) {
        self.suppress_resize_append_lines = suppressed;
    }
}

impl<B: Backend> Backend for InlineTerminalBackend<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.inner.draw(content)
    }

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        if self.suppress_resize_append_lines {
            return Ok(());
        }
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<ratatui::backend::WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }

    fn scroll_region_up(&mut self, region: Range<u16>, line_count: u16) -> Result<(), Self::Error> {
        self.inner.scroll_region_up(region, line_count)
    }

    fn scroll_region_down(
        &mut self,
        region: Range<u16>,
        line_count: u16,
    ) -> Result<(), Self::Error> {
        self.inner.scroll_region_down(region, line_count)
    }
}

impl<B: std::fmt::Display> std::fmt::Display for InlineTerminalBackend<B> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(formatter)
    }
}

#[derive(Default)]
struct InlineTerminalState {
    history: InlineHistoryState,
    last_tail_frame: Option<InlineTailFrameSignature>,
    last_known_screen_size: Option<Size>,
    viewport_area: Option<Rect>,
    visible_history_rows: u16,
    back_buffer_trustworthy: bool,
    insert_mode: HistoryInsertionMode,
}

impl InlineTerminalState {
    fn record_terminal_size(&mut self, terminal_size: Size) {
        self.last_known_screen_size = Some(terminal_size);
        self.viewport_area = Some(inline_viewport_area(terminal_size));
    }

    fn record_history_sync(
        &mut self,
        current_lines: &[Line<'static>],
        sync_result: InlineHistorySyncResult,
    ) {
        if current_lines.is_empty() {
            self.visible_history_rows = 0;
            return;
        }

        self.visible_history_rows = self
            .visible_history_rows
            .saturating_add(sync_result.inserted_rows);
    }

    fn invalidate_back_buffer<B: Backend>(&mut self, terminal: &mut Terminal<B>) {
        self.back_buffer_trustworthy = false;
        terminal.swap_buffers();
        terminal.swap_buffers();
    }

    fn mark_frame_drawn(&mut self, terminal_size: Size) {
        self.record_terminal_size(terminal_size);
        self.back_buffer_trustworthy = true;
    }

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
            lines: build_inline_tail_view(app, terminal_width).lines,
        };
        let should_draw = self.last_tail_frame.as_ref() != Some(&next_signature);
        self.last_tail_frame = Some(next_signature);
        should_draw
    }
}

fn inline_viewport_area(terminal_size: Size) -> Rect {
    let height = INLINE_VIEWPORT_HEIGHT.min(terminal_size.height);
    Rect {
        x: 0,
        y: terminal_size.height.saturating_sub(height),
        width: terminal_size.width,
        height,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct InlineHistorySyncResult {
    inserted_rows: u16,
}

impl InlineHistorySyncResult {
    fn inserted(self) -> bool {
        self.inserted_rows > 0
    }
}

const MIN_SHIFTED_HISTORY_OVERLAP: usize = 8;

impl InlineHistoryState {
    fn sync<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        current_lines: &[Line<'static>],
        insert_mode: HistoryInsertionMode,
    ) -> Result<InlineHistorySyncResult, B::Error> {
        let pending_lines = self.pending_lines(current_lines);
        let width = terminal.size()?.width;
        let inserted_rows =
            count_rendered_history_rows(&pending_lines, width).min(u16::MAX as usize) as u16;
        if !pending_lines.is_empty() {
            HistoryInsertionAdapter::new(insert_mode).insert(terminal, &pending_lines)?;
        }
        self.remember(current_lines);
        Ok(InlineHistorySyncResult { inserted_rows })
    }

    fn remember(&mut self, current_lines: &[Line<'static>]) {
        self.rendered_lines = current_lines.to_vec();
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
        if current_lines.len() != MAX_CONVERSATION_HISTORY_LINES {
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

struct TerminalRestoreGuard;

impl TerminalRestoreGuard {
    fn activate() -> Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, MoveToNextLine(1));
        let _ = execute!(stdout, Show);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;
    use ratatui::backend::TestBackend;
    use ratatui::layout::{Rect, Size};
    use ratatui::text::Line;
    use ratatui::{Terminal, Viewport};

    use super::super::tui_testkit;
    use super::{
        HistoryInsertionMode, InlineHistoryState, InlineTerminalBackend, InlineTerminalState,
        ShellRuntime, current_inline_history_lines, draw_inline_frame, draw_inline_transaction,
        sync_inline_viewport, terminal_options_for_render_mode,
    };
    use crate::adapter::inbound::tui::app::{
        ConversationMessage, ConversationMessageKind, ConversationState, INLINE_VIEWPORT_HEIGHT,
        InlineHistoryRenderMode, MAX_CONVERSATION_HISTORY_LINES, NativeTuiApp, PlannerVisibility,
    };
    use crate::adapter::inbound::tui::shell_chrome::ShellOverlay;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::codex_app_server_port::{
        AppServerStartupContext, CodexAppServerPort,
    };
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::planning::PlanningServices;
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::ConversationSnapshot;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog};

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
    fn pending_lines_only_inserts_new_suffix_when_history_first_hits_cap() {
        let state = InlineHistoryState {
            rendered_lines: (0..MAX_CONVERSATION_HISTORY_LINES - 10)
                .map(|idx| Line::from(format!("line {idx}")))
                .collect(),
        };
        let current_lines = (10..MAX_CONVERSATION_HISTORY_LINES + 10)
            .map(|idx| Line::from(format!("line {idx}")))
            .collect::<Vec<_>>();

        let pending = state.pending_lines(&current_lines);

        assert_eq!(
            pending,
            (MAX_CONVERSATION_HISTORY_LINES - 10..MAX_CONVERSATION_HISTORY_LINES + 10)
                .map(|idx| Line::from(format!("line {idx}")))
                .collect::<Vec<_>>()
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
    fn history_sync_reports_insertions_that_need_viewport_redraw() {
        let mut terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
        let mut state = InlineHistoryState::default();
        let current_lines = vec![
            Line::from("User:"),
            Line::from("  first prompt"),
            Line::from(""),
        ];

        assert!(
            state
                .sync(
                    &mut terminal,
                    &current_lines,
                    HistoryInsertionMode::StandardScrollRegion,
                )
                .unwrap()
                .inserted()
        );
        assert!(
            !state
                .sync(
                    &mut terminal,
                    &current_lines,
                    HistoryInsertionMode::StandardScrollRegion,
                )
                .unwrap()
                .inserted()
        );

        let appended_lines = vec![
            Line::from("User:"),
            Line::from("  first prompt"),
            Line::from(""),
            Line::from("Agent:"),
            Line::from("  first answer"),
            Line::from(""),
        ];
        assert!(
            state
                .sync(
                    &mut terminal,
                    &appended_lines,
                    HistoryInsertionMode::StandardScrollRegion,
                )
                .unwrap()
                .inserted()
        );
    }

    #[test]
    fn history_sync_for_empty_thread_clears_remembered_history_without_insert() {
        let mut terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
        let mut state = InlineHistoryState {
            rendered_lines: vec![
                Line::from("User:"),
                Line::from("  old prompt"),
                Line::from(""),
                Line::from("Agent:"),
                Line::from("  old answer"),
            ],
        };

        assert!(
            !state
                .sync(
                    &mut terminal,
                    &[],
                    HistoryInsertionMode::StandardScrollRegion,
                )
                .unwrap()
                .inserted()
        );
        assert!(state.rendered_lines.is_empty());

        let next_thread_lines = vec![Line::from("Status:"), Line::from("  new thread loaded")];
        assert_eq!(state.pending_lines(&next_thread_lines), next_thread_lines);
    }

    #[test]
    fn host_history_sync_keeps_live_agent_delta_out_of_inserted_history() {
        let mut terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
        let mut app = make_test_app();
        app.show_startup_ascii_art = false;
        app.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
        append_history_message(&mut app, "committed answer belongs in host history");
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should start in a ready conversation state");
        };
        conversation.record_turn_started("turn-1".to_string());
        conversation.push_live_agent_delta(
            "agent-live".to_string(),
            Some("final_answer".to_string()),
            "live answer stays in tail".to_string(),
        );
        let mut runtime = ShellRuntime::new(app);
        let mut inline_viewport = InlineTerminalState::default();

        assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
        let inserted_history = tui_testkit::screen_text(&terminal);
        assert!(inserted_history.contains("committed answer belongs in host history"));
        assert!(!inserted_history.contains("live answer stays in tail"));

        draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);
        let live_frame = tui_testkit::screen_text(&terminal);
        assert!(live_frame.contains("live answer stays in tail"));
    }

    #[test]
    fn history_insert_invalidates_back_buffer_until_frame_draw() {
        let mut terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
        let mut app = make_test_app();
        app.show_startup_ascii_art = false;
        append_history_message(&mut app, "history insert should invalidate diff state");
        let mut runtime = ShellRuntime::new(app);
        let mut inline_terminal = InlineTerminalState::default();

        assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());

        assert_eq!(
            inline_terminal.last_known_screen_size,
            Some(Size {
                width: 80,
                height: 24
            })
        );
        assert_eq!(
            inline_terminal.viewport_area,
            Some(Rect {
                x: 0,
                y: 8,
                width: 80,
                height: 16
            })
        );
        assert_eq!(
            inline_terminal.insert_mode,
            HistoryInsertionMode::StandardScrollRegion
        );
        assert!(inline_terminal.visible_history_rows > 0);
        assert!(!inline_terminal.back_buffer_trustworthy);

        draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);

        assert!(inline_terminal.back_buffer_trustworthy);
    }

    #[test]
    fn newline_fallback_history_insert_invalidates_back_buffer() {
        let mut terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
        let mut app = make_test_app();
        app.show_startup_ascii_art = false;
        app.history_insert_mode = HistoryInsertionMode::NewlineFallback;
        append_history_message(&mut app, "newline fallback committed history");
        let mut runtime = ShellRuntime::new(app);
        let mut inline_terminal = InlineTerminalState::default();

        assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());

        assert_eq!(
            inline_terminal.insert_mode,
            HistoryInsertionMode::NewlineFallback
        );
        assert!(!inline_terminal.back_buffer_trustworthy);

        draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);

        assert!(inline_terminal.back_buffer_trustworthy);
        assert!(tui_testkit::screen_text(&terminal).contains("newline fallback committed history"));
    }

    #[test]
    fn draw_transaction_flushes_history_and_live_tail_together() {
        let mut terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
        let mut app = make_test_app();
        app.show_startup_ascii_art = false;
        append_history_message(&mut app, "committed history in transaction");
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should start in a ready conversation state");
        };
        conversation.record_turn_started("turn-1".to_string());
        conversation.push_live_agent_delta(
            "agent-live".to_string(),
            Some("final_answer".to_string()),
            "live tail in same transaction".to_string(),
        );
        let mut runtime = ShellRuntime::new(app);
        let mut inline_terminal = InlineTerminalState::default();

        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("draw transaction");

        let screen_text = tui_testkit::screen_text(&terminal);
        assert!(screen_text.contains("committed history in transaction"));
        assert!(screen_text.contains("live tail in same transaction"));
        assert!(inline_terminal.back_buffer_trustworthy);
        assert!(
            !tui_testkit::inline_scrollback_text(&terminal)
                .contains("live tail in same transaction")
        );
    }

    #[test]
    fn viewport_replay_sync_skips_host_scrollback_insertions() {
        let mut replay_terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
        let mut replay_app = make_test_app();
        replay_app.show_startup_ascii_art = false;
        replay_app.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
        append_history_message(
            &mut replay_app,
            "history should not be inserted in replay mode",
        );
        let mut replay_runtime = ShellRuntime::new(replay_app);
        let mut replay_viewport = InlineTerminalState::default();

        assert!(
            sync_inline_viewport(
                &mut replay_terminal,
                &mut replay_runtime,
                &mut replay_viewport
            )
            .unwrap()
        );
        assert!(
            !tui_testkit::screen_text(&replay_terminal)
                .contains("history should not be inserted in replay mode")
        );

        let mut host_terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
        let mut host_app = make_test_app();
        host_app.show_startup_ascii_art = false;
        host_app.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
        append_history_message(&mut host_app, "history should be inserted in host mode");
        let mut host_runtime = ShellRuntime::new(host_app);
        let mut host_viewport = InlineTerminalState::default();

        assert!(
            sync_inline_viewport(&mut host_terminal, &mut host_runtime, &mut host_viewport)
                .unwrap()
        );
        assert!(tui_testkit::screen_text(&host_terminal).contains("history should be inserted"));
    }

    #[test]
    fn viewport_replay_keeps_inline_viewport_for_shell_positioning() {
        assert_eq!(
            terminal_options_for_render_mode(InlineHistoryRenderMode::ViewportReplay).viewport,
            Viewport::Inline(INLINE_VIEWPORT_HEIGHT)
        );
        assert_eq!(
            terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback).viewport,
            Viewport::Inline(INLINE_VIEWPORT_HEIGHT)
        );
    }

    #[test]
    fn viewport_replay_resize_does_not_push_tail_into_scrollback() {
        assert_resize_sequence_does_not_leak_live_tail(
            InlineHistoryRenderMode::ViewportReplay,
            "resize replay should stay in the active viewport",
        );
    }

    #[test]
    fn host_scrollback_resize_does_not_push_tail_into_scrollback() {
        assert_resize_sequence_does_not_leak_live_tail(
            InlineHistoryRenderMode::HostScrollback,
            "resize host history stays committed",
        );
    }

    #[test]
    fn draw_internal_resize_does_not_push_tail_into_scrollback() {
        assert_draw_internal_resize_does_not_leak_live_tail(
            InlineHistoryRenderMode::HostScrollback,
            "host history before draw-time resize",
        );
        assert_draw_internal_resize_does_not_leak_live_tail(
            InlineHistoryRenderMode::ViewportReplay,
            "replay history before draw-time resize",
        );
    }

    fn assert_resize_sequence_does_not_leak_live_tail(
        render_mode: InlineHistoryRenderMode,
        history_message: &str,
    ) {
        let mut terminal = tui_testkit::inline_history_terminal(render_mode, 80, 24);
        let mut app = make_test_app();
        app.show_startup_ascii_art = false;
        app.inline_history_render_mode = render_mode;
        if let ConversationState::Ready(conversation) = &mut app.conversation_state {
            conversation.input_buffer = "live prompt must not move to scrollback".to_string();
        }
        append_history_message(&mut app, history_message);
        let mut runtime = ShellRuntime::new(app);
        let mut inline_viewport = InlineTerminalState::default();

        assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
        draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);

        tui_testkit::resize_inline_history_terminal(&mut terminal, 80, 8);
        assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
        draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);

        tui_testkit::resize_inline_history_terminal(&mut terminal, 80, 24);
        assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
        draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);

        assert_no_live_tail_leak(&terminal, render_mode);
    }

    fn assert_draw_internal_resize_does_not_leak_live_tail(
        render_mode: InlineHistoryRenderMode,
        history_message: &str,
    ) {
        let mut terminal = tui_testkit::inline_history_terminal(render_mode, 80, 24);
        let mut app = make_test_app();
        app.show_startup_ascii_art = false;
        app.inline_history_render_mode = render_mode;
        if let ConversationState::Ready(conversation) = &mut app.conversation_state {
            conversation.input_buffer = "live prompt must not move to scrollback".to_string();
        }
        append_history_message(&mut app, history_message);
        let mut runtime = ShellRuntime::new(app);
        let mut inline_viewport = InlineTerminalState::default();

        assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
        draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);

        tui_testkit::resize_inline_history_terminal(&mut terminal, 80, 8);
        assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
        tui_testkit::resize_inline_history_terminal(&mut terminal, 80, 12);
        draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);

        assert_no_live_tail_leak(&terminal, render_mode);
    }

    fn assert_no_live_tail_leak(
        terminal: &Terminal<InlineTerminalBackend<TestBackend>>,
        render_mode: InlineHistoryRenderMode,
    ) {
        let scrollback_text = tui_testkit::inline_scrollback_text(terminal);
        assert!(
            !scrollback_text.contains("live prompt must not move to scrollback"),
            "{render_mode:?} should not leak live prompt rows into scrollback after resize: {scrollback_text:?}"
        );
        assert!(
            !scrollback_text.contains("thread: new draft"),
            "{render_mode:?} should not leak live status rows into scrollback after resize: {scrollback_text:?}"
        );
        let screen_text = tui_testkit::screen_text(terminal);
        assert!(
            screen_text.contains("> live prompt must not move to scrollback"),
            "{render_mode:?} should keep the active prompt visible after shrink/restore: {screen_text:?}"
        );
        assert_eq!(
            screen_text
                .matches("> live prompt must not move to scrollback")
                .count(),
            1,
            "{render_mode:?} should not duplicate the active prompt after shrink/restore: {screen_text:?}"
        );
    }

    #[test]
    fn hidden_inline_tail_skips_redundant_frame_draws() {
        let app = make_test_app();
        let mut inline_viewport = InlineTerminalState::default();

        assert!(inline_viewport.should_draw_inline_frame(&app, 80, 24));
        assert!(!inline_viewport.should_draw_inline_frame(&app, 80, 24));
        assert!(inline_viewport.should_draw_inline_frame(&app, 96, 24));
    }

    #[test]
    fn overlay_cycle_resets_hidden_tail_redraw_cache() {
        let mut app = make_test_app();
        let mut inline_viewport = InlineTerminalState::default();

        assert!(inline_viewport.should_draw_inline_frame(&app, 80, 24));
        assert!(!inline_viewport.should_draw_inline_frame(&app, 80, 24));

        app.shell_overlay = ShellOverlay::Startup;
        assert!(inline_viewport.should_draw_inline_frame(&app, 80, 24));

        app.shell_overlay = ShellOverlay::Hidden;
        assert!(inline_viewport.should_draw_inline_frame(&app, 80, 24));
    }

    #[test]
    fn inline_history_uses_startup_banner_while_typing_in_new_draft() {
        let mut app = make_test_app();
        app.show_startup_ascii_art = true;
        if let crate::adapter::inbound::tui::app::ConversationState::Ready(conversation) =
            &mut app.conversation_state
        {
            conversation.input_buffer = "hello banner".to_string();
        }

        let lines = current_inline_history_lines(&app)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        let rendered = lines.join("\n");

        assert!(rendered.contains(".::::::.::::::.::::::.::::::"));
        assert!(rendered.contains(".::       .::.::  .::   .::"));
        assert!(!rendered.contains("No messages in this thread yet."));
    }

    #[test]
    fn inline_history_shows_planner_debug_detail_when_visibility_is_debug() {
        let mut app = make_test_app();
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should start in a ready conversation state");
        };
        conversation.messages.push(
            ConversationMessage::new(
                ConversationMessageKind::User,
                "다음 queued task 1개를 이어서 진행합니다.",
                None,
                None,
            )
            .with_display_label("Auto Follow-up")
            .with_debug_detail("planner temp session: refresh / refresh ok"),
        );
        conversation.refresh_conversation_lines();

        let normal_lines = current_inline_history_lines(&app)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!normal_lines.contains("planner temp session"));

        app.planner_visibility = PlannerVisibility::Debug;
        let debug_lines = current_inline_history_lines(&app)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(debug_lines.contains("planner temp session: refresh / refresh ok"));
    }

    fn draw_test_frame(
        terminal: &mut Terminal<InlineTerminalBackend<TestBackend>>,
        runtime: &mut ShellRuntime,
        inline_terminal: &mut InlineTerminalState,
    ) {
        draw_inline_frame(terminal, runtime, inline_terminal).expect("draw test frame");
    }

    fn append_history_message(app: &mut NativeTuiApp, text: &str) {
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should start in a ready conversation state");
        };
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            text.to_string(),
            None,
            None,
        ));
        conversation.refresh_conversation_lines();
    }

    struct FakeCodexAppServerPort;

    impl CodexAppServerPort for FakeCodexAppServerPort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            Ok(AppServerStartupContext {
                attachment_profile:
                    crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile::codex_app_server(),
                initialize_detail: "ok".to_string(),
                account_detail: "ok".to_string(),
                account_ok: true,
                warnings: Vec::new(),
            })
        }

        fn load_recent_sessions(&self, _limit: usize) -> Result<SessionCatalog> {
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            }
            .into())
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

    fn make_test_app() -> NativeTuiApp {
        let codex_port = Arc::new(FakeCodexAppServerPort);
        NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
            PlanningServices::from_workspace_port(Arc::new(
                FilesystemPlanningWorkspaceAdapter::new(),
            )),
        )
    }
}

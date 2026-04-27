use ratatui::Terminal;
use ratatui::TerminalOptions;
use ratatui::Viewport;
use ratatui::backend::Backend;
use ratatui::backend::ClearType;
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Rect, Size};
use ratatui::text::Line;
use std::ops::Range;

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

pub(super) fn terminal_options_for_render_mode(
    render_mode: InlineHistoryRenderMode,
) -> TerminalOptions {
    let viewport = match render_mode {
        InlineHistoryRenderMode::HostScrollback => Viewport::Inline(INLINE_VIEWPORT_HEIGHT),
        InlineHistoryRenderMode::ViewportReplay => Viewport::Inline(INLINE_VIEWPORT_HEIGHT),
    };
    TerminalOptions { viewport }
}

pub(super) struct InlineTerminalAdapter<B: InlineResizeBackend> {
    terminal: Terminal<B>,
    state: InlineTerminalState,
}

impl<B: InlineResizeBackend> InlineTerminalAdapter<B> {
    pub(super) fn new(terminal: Terminal<B>) -> Self {
        Self {
            terminal,
            state: InlineTerminalState::default(),
        }
    }

    pub(super) fn draw_inline_transaction(
        &mut self,
        runtime: &mut ShellRuntime,
    ) -> Result<(), B::Error> {
        draw_inline_transaction(&mut self.terminal, runtime, &mut self.state)
    }
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
    if !inline_terminal.viewport.back_buffer_trustworthy {
        clear_inline_viewport(terminal)?;
    }
    terminal
        .backend_mut()
        .set_resize_append_lines_suppressed(true);
    let mut drawn_viewport_area = Rect::default();
    let result = terminal
        .draw(|frame| {
            let frame_area = frame.area();
            drawn_viewport_area = frame_area;
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
    let cursor_position = terminal.get_cursor_position()?;
    inline_terminal.mark_frame_drawn(terminal_size, drawn_viewport_area, cursor_position);
    Ok(())
}

fn clear_inline_viewport<B: Backend>(terminal: &mut Terminal<B>) -> Result<(), B::Error> {
    terminal.clear()
}

fn sync_inline_viewport<B: InlineResizeBackend>(
    terminal: &mut Terminal<B>,
    runtime: &mut ShellRuntime,
    inline_terminal: &mut InlineTerminalState,
) -> Result<bool, B::Error> {
    let (render_mode, insert_mode) = {
        let app = runtime.app_mut();
        (app.inline_history_render_mode, app.history_insert_mode)
    };
    autoresize_inline_viewport(terminal)?;
    let terminal_size = terminal.size()?;
    let viewport_area = current_viewport_area(terminal);
    let cursor_position = terminal.get_cursor_position()?;
    inline_terminal.record_terminal_viewport(terminal_size, viewport_area, cursor_position);
    inline_terminal.viewport.insert_mode = insert_mode;
    let visible_history_adjusted = inline_terminal.history_flush.fit_visible_rows_to_viewport(
        terminal,
        terminal_size,
        viewport_area,
    )?;
    if visible_history_adjusted {
        inline_terminal.invalidate_back_buffer();
    }
    let current_lines = current_inline_history_lines(runtime.app_mut());
    let writes_host_scrollback = render_mode.writes_host_scrollback();
    let history_sync = if writes_host_scrollback {
        inline_terminal
            .history_flush
            .sync(terminal, &current_lines, insert_mode)?
    } else {
        inline_terminal
            .history_flush
            .remember_without_flush(&current_lines);
        HistoryFlushResult::default()
    };
    if history_sync.inserted() {
        inline_terminal.invalidate_back_buffer();
    }
    let viewport_area = current_viewport_area(terminal);
    let cursor_position = terminal.get_cursor_position()?;
    inline_terminal.record_terminal_viewport(terminal_size, viewport_area, cursor_position);

    let tail_frame_changed = inline_terminal.should_draw_inline_frame(
        runtime.app_mut(),
        viewport_area.width,
        viewport_area.height,
    );
    Ok(visible_history_adjusted || history_sync.inserted() || tail_frame_changed)
}

fn current_viewport_area<B: Backend>(terminal: &mut Terminal<B>) -> Rect {
    terminal.get_frame().area()
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

pub(super) trait InlineResizeBackend: Backend {
    fn set_resize_append_lines_suppressed(&mut self, suppressed: bool);
}

pub(super) struct InlineTerminalBackend<B> {
    inner: B,
    suppress_resize_append_lines: bool,
    tracked_cursor_position: Option<Position>,
}

impl<B> InlineTerminalBackend<B> {
    pub(super) fn new(inner: B) -> Self {
        Self {
            inner,
            suppress_resize_append_lines: false,
            tracked_cursor_position: None,
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
        let size = self.inner.size()?;
        self.inner
            .draw(content.filter(move |(x, y, _)| *x < size.width && *y < size.height))
    }

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        if self.suppress_resize_append_lines {
            return Ok(());
        }
        self.inner.append_lines(n)?;
        self.track_cursor_after_append_lines(n);
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        if let Some(position) = self.tracked_cursor_position {
            return Ok(position);
        }
        let position = self.inner.get_cursor_position()?;
        self.tracked_cursor_position = Some(position);
        Ok(position)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        let position = position.into();
        self.inner.set_cursor_position(position)?;
        self.tracked_cursor_position = Some(position);
        Ok(())
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

impl<B: Backend> InlineTerminalBackend<B> {
    fn track_cursor_after_append_lines(&mut self, line_count: u16) {
        if line_count == 0 {
            return;
        }
        let Some(mut position) = self.tracked_cursor_position else {
            return;
        };
        if let Ok(size) = self.inner.size() {
            if size.height == 0 {
                return;
            }
            position.x = 0;
            position.y = position
                .y
                .saturating_add(line_count)
                .min(size.height.saturating_sub(1));
            self.tracked_cursor_position = Some(position);
        }
    }
}

impl<B: std::fmt::Display> std::fmt::Display for InlineTerminalBackend<B> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(formatter)
    }
}

#[derive(Default)]
struct InlineTerminalState {
    viewport: TerminalViewportState,
    history_flush: HistoryFlushState,
    frame_cache: FrameCacheState,
}

impl InlineTerminalState {
    fn record_terminal_viewport(
        &mut self,
        terminal_size: Size,
        viewport_area: Rect,
        cursor_position: Position,
    ) {
        let terminal_resized = self
            .viewport
            .last_known_screen_size
            .is_some_and(|last_known_screen_size| last_known_screen_size != terminal_size);
        self.viewport
            .record_terminal_viewport(terminal_size, viewport_area, cursor_position);
        if terminal_resized {
            self.invalidate_back_buffer();
        }
    }

    fn invalidate_back_buffer(&mut self) {
        self.viewport.invalidate_back_buffer();
    }

    fn mark_frame_drawn(
        &mut self,
        terminal_size: Size,
        viewport_area: Rect,
        cursor_position: Position,
    ) {
        self.viewport
            .mark_frame_drawn(terminal_size, viewport_area, cursor_position);
    }

    fn should_draw_inline_frame(
        &mut self,
        app: &NativeTuiApp,
        terminal_width: u16,
        terminal_height: u16,
    ) -> bool {
        self.frame_cache.should_draw_inline_frame(
            app,
            &self.viewport,
            terminal_width,
            terminal_height,
        )
    }

    #[cfg(test)]
    fn last_known_screen_size(&self) -> Option<Size> {
        self.viewport.last_known_screen_size
    }

    #[cfg(test)]
    fn last_known_cursor_pos(&self) -> Option<Position> {
        self.viewport.last_known_cursor_pos
    }

    #[cfg(test)]
    fn viewport_area(&self) -> Option<Rect> {
        self.viewport.viewport_area
    }

    #[cfg(test)]
    fn back_buffer_trustworthy(&self) -> bool {
        self.viewport.back_buffer_trustworthy
    }

    #[cfg(test)]
    fn insert_mode(&self) -> HistoryInsertionMode {
        self.viewport.insert_mode
    }
}

struct TerminalViewportState {
    viewport_area: Option<Rect>,
    last_known_screen_size: Option<Size>,
    last_known_cursor_pos: Option<Position>,
    back_buffer_trustworthy: bool,
    insert_mode: HistoryInsertionMode,
}

impl Default for TerminalViewportState {
    fn default() -> Self {
        Self {
            viewport_area: None,
            last_known_screen_size: None,
            last_known_cursor_pos: None,
            back_buffer_trustworthy: true,
            insert_mode: HistoryInsertionMode::default(),
        }
    }
}

impl TerminalViewportState {
    fn record_terminal_viewport(
        &mut self,
        terminal_size: Size,
        viewport_area: Rect,
        cursor_position: Position,
    ) {
        self.viewport_area = Some(viewport_area);
        self.last_known_screen_size = Some(terminal_size);
        self.last_known_cursor_pos = Some(cursor_position);
    }

    fn invalidate_back_buffer(&mut self) {
        self.back_buffer_trustworthy = false;
    }

    fn mark_frame_drawn(
        &mut self,
        terminal_size: Size,
        viewport_area: Rect,
        cursor_position: Position,
    ) {
        self.record_terminal_viewport(terminal_size, viewport_area, cursor_position);
        self.back_buffer_trustworthy = true;
    }
}

#[derive(Default)]
struct FrameCacheState {
    last_tail_frame: Option<InlineTailFrameSignature>,
}

impl FrameCacheState {
    fn should_draw_inline_frame(
        &mut self,
        app: &NativeTuiApp,
        viewport: &TerminalViewportState,
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
        let should_draw = !viewport.back_buffer_trustworthy
            || self.last_tail_frame.as_ref() != Some(&next_signature);
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
struct HistoryFlushState {
    rendered_lines: Vec<Line<'static>>,
    pending_history_lines: Vec<Line<'static>>,
    visible_history_rows: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct HistoryFlushResult {
    inserted_rows: u16,
}

impl HistoryFlushResult {
    fn inserted(self) -> bool {
        self.inserted_rows > 0
    }
}

const MIN_SHIFTED_HISTORY_OVERLAP: usize = 8;

impl HistoryFlushState {
    fn fit_visible_rows_to_viewport<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        terminal_size: Size,
        viewport_area: Rect,
    ) -> Result<bool, B::Error> {
        let viewport_top = viewport_area.top();
        if self.visible_history_rows <= viewport_top {
            return Ok(false);
        }

        let overflow_rows = self.visible_history_rows - viewport_top;
        terminal.backend_mut().set_cursor_position(Position {
            x: 0,
            y: terminal_size.height.saturating_sub(1),
        })?;
        terminal.backend_mut().append_lines(overflow_rows)?;
        self.visible_history_rows = viewport_top;
        Ok(true)
    }

    fn sync<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        current_lines: &[Line<'static>],
        insert_mode: HistoryInsertionMode,
    ) -> Result<HistoryFlushResult, B::Error> {
        self.pending_history_lines = self.pending_lines(current_lines);
        let terminal_size = terminal.size()?;
        let width = terminal_size.width;
        let inserted_rows = if self.pending_history_lines.is_empty() {
            0
        } else {
            count_rendered_history_rows(&self.pending_history_lines, width).min(u16::MAX as usize)
                as u16
        };
        if inserted_rows > 0 {
            HistoryInsertionAdapter::new(insert_mode).insert_with_rendered_rows(
                terminal,
                &self.pending_history_lines,
                inserted_rows,
            )?;
        }
        let viewport_top_after_insert = terminal.get_frame().area().top();
        if current_lines.is_empty() {
            self.visible_history_rows = 0;
        } else if inserted_rows > 0 {
            self.visible_history_rows = if self.pending_history_lines.len() == current_lines.len() {
                inserted_rows.min(viewport_top_after_insert)
            } else {
                self.visible_history_rows
                    .saturating_add(inserted_rows)
                    .min(viewport_top_after_insert)
            };
        }
        self.remember(current_lines);
        self.pending_history_lines.clear();
        Ok(HistoryFlushResult { inserted_rows })
    }

    fn remember_without_flush(&mut self, current_lines: &[Line<'static>]) {
        if current_lines.is_empty() {
            self.visible_history_rows = 0;
        }
        self.pending_history_lines.clear();
        self.remember(current_lines);
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

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::ops::Range;
    use std::sync::Arc;

    use anyhow::Result;
    use ratatui::backend::{Backend, ClearType, TestBackend, WindowSize};
    use ratatui::buffer::Cell;
    use ratatui::layout::{Position, Size};
    use ratatui::text::Line;
    use ratatui::{Terminal, Viewport};

    use super::super::tui_testkit;
    use super::{
        HistoryFlushState, HistoryInsertionMode, InlineResizeBackend, InlineTerminalBackend,
        InlineTerminalState, ShellRuntime, current_inline_history_lines, draw_inline_frame,
        draw_inline_transaction, sync_inline_viewport, terminal_options_for_render_mode,
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
    use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningServices};
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::ConversationSnapshot;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog};

    #[test]
    fn pending_lines_returns_only_new_suffix_for_appended_history() {
        let state = HistoryFlushState {
            rendered_lines: vec![
                Line::from("User:"),
                Line::from("  first prompt"),
                Line::from(""),
            ],
            pending_history_lines: Vec::new(),
            visible_history_rows: 0,
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
        let state = HistoryFlushState {
            rendered_lines: vec![
                Line::from("User:"),
                Line::from("  old thread"),
                Line::from(""),
            ],
            pending_history_lines: Vec::new(),
            visible_history_rows: 0,
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
        let state = HistoryFlushState {
            rendered_lines: (0..MAX_CONVERSATION_HISTORY_LINES)
                .map(|idx| Line::from(format!("line {idx}")))
                .collect(),
            pending_history_lines: Vec::new(),
            visible_history_rows: 0,
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
        let state = HistoryFlushState {
            rendered_lines: (0..MAX_CONVERSATION_HISTORY_LINES - 10)
                .map(|idx| Line::from(format!("line {idx}")))
                .collect(),
            pending_history_lines: Vec::new(),
            visible_history_rows: 0,
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
        let state = HistoryFlushState {
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
            pending_history_lines: Vec::new(),
            visible_history_rows: 0,
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
        let state = HistoryFlushState {
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
            pending_history_lines: Vec::new(),
            visible_history_rows: 0,
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
        let mut state = HistoryFlushState::default();
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
        let mut state = HistoryFlushState {
            rendered_lines: vec![
                Line::from("User:"),
                Line::from("  old prompt"),
                Line::from(""),
                Line::from("Agent:"),
                Line::from("  old answer"),
                Line::from(""),
            ],
            pending_history_lines: Vec::new(),
            visible_history_rows: 6,
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
        let inserted_history = tui_testkit::inline_terminal_history_text(&terminal);
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
        app.history_insert_mode = HistoryInsertionMode::StandardScrollRegion;
        append_history_message(&mut app, "history insert should invalidate diff state");
        let mut runtime = ShellRuntime::new(app);
        let mut inline_terminal = InlineTerminalState::default();

        assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());

        assert_eq!(
            inline_terminal.last_known_screen_size(),
            Some(Size {
                width: 80,
                height: 24
            })
        );
        assert_eq!(
            inline_terminal.viewport_area(),
            Some(terminal.get_frame().area())
        );
        assert_eq!(
            inline_terminal.insert_mode(),
            HistoryInsertionMode::StandardScrollRegion
        );
        assert_eq!(
            inline_terminal.last_known_cursor_pos(),
            Some(terminal.get_cursor_position().unwrap())
        );
        assert!(!inline_terminal.back_buffer_trustworthy());

        draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);

        assert!(inline_terminal.back_buffer_trustworthy());
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
            inline_terminal.insert_mode(),
            HistoryInsertionMode::NewlineFallback
        );
        assert_eq!(
            inline_terminal.last_known_cursor_pos(),
            Some(terminal.get_cursor_position().unwrap())
        );
        assert!(!inline_terminal.back_buffer_trustworthy());

        draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);

        assert!(inline_terminal.back_buffer_trustworthy());
        assert!(
            tui_testkit::inline_terminal_history_text(&terminal)
                .contains("newline fallback committed history")
        );
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

        let terminal_history = tui_testkit::inline_terminal_history_text(&terminal);
        assert!(terminal_history.contains("committed history in transaction"));
        let screen_text = tui_testkit::screen_text(&terminal);
        assert!(screen_text.contains("live tail in same transaction"));
        assert!(inline_terminal.back_buffer_trustworthy());
        assert!(
            !tui_testkit::inline_scrollback_text(&terminal)
                .contains("live tail in same transaction")
        );
    }

    #[test]
    fn vt100_terminal_app_preserves_newline_fallback_history_after_live_resize() {
        let mut terminal = tui_testkit::inline_history_vt100_terminal(
            InlineHistoryRenderMode::HostScrollback,
            80,
            24,
        );
        let mut app = make_test_app();
        app.show_startup_ascii_art = false;
        app.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
        app.history_insert_mode = HistoryInsertionMode::NewlineFallback;
        append_user_history_message(&mut app, "HISTORY_MARKER_ONE user prompt");
        append_history_message(&mut app, "HISTORY_MARKER_TWO committed answer");
        append_user_history_message(&mut app, "HISTORY_MARKER_THREE follow-up prompt");
        append_history_message(&mut app, "HISTORY_MARKER_FOUR committed answer");
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should start in a ready conversation state");
        };
        conversation.record_turn_started("turn-1".to_string());
        conversation.push_live_agent_delta(
            "agent-live".to_string(),
            Some("final_answer".to_string()),
            "LIVE_MARKER_FIVE streaming answer".to_string(),
        );
        let mut runtime = ShellRuntime::new(app);
        let mut inline_terminal = InlineTerminalState::default();

        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("initial vt100 draw transaction");
        let terminal_history_before_completion =
            tui_testkit::inline_vt100_scrollback_text(&mut terminal);
        assert!(
            terminal_history_before_completion.contains("HISTORY_MARKER_TWO"),
            "committed history should be in terminal app history before live completion: {terminal_history_before_completion:?}"
        );
        let host_scrollback_before_completion =
            tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
        assert!(
            !host_scrollback_before_completion.contains("LIVE_MARKER_FIVE"),
            "live streaming rows must not be flushed into host scrollback before completion: {host_scrollback_before_completion:?}"
        );

        let app = runtime.app_mut();
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should stay in a ready conversation state");
        };
        assert!(conversation.complete_live_agent_message(
            "agent-live".to_string(),
            Some("final_answer".to_string()),
            "LIVE_MARKER_FIVE final answer".to_string(),
        ));

        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("completion vt100 draw transaction");
        tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 80, 8);
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("short vt100 draw transaction");
        tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 80, 24);
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("restored vt100 draw transaction");

        let terminal_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
        for marker in [
            "HISTORY_MARKER_ONE",
            "HISTORY_MARKER_TWO",
            "HISTORY_MARKER_THREE",
            "HISTORY_MARKER_FOUR",
            "LIVE_MARKER_FIVE",
        ] {
            assert!(
                terminal_history.contains(marker),
                "terminal app history lost {marker} after live commit and resize:\n{terminal_history}"
            );
            assert_eq!(
                terminal_history.matches(marker).count(),
                1,
                "terminal app history duplicated {marker} after live commit and resize:\n{terminal_history}"
            );
        }
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
        assert!(
            tui_testkit::inline_terminal_history_text(&host_terminal)
                .contains("history should be inserted")
        );
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
    fn inline_backend_reuses_tracked_cursor_after_initial_query() {
        let backend =
            InlineTerminalBackend::new(CursorQueryCountingBackend::new(TestBackend::new(80, 24)));
        let mut terminal = Terminal::with_options(
            backend,
            terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback),
        )
        .expect("inline terminal should initialize");
        let initial_query_count = terminal.backend_mut().inner().cursor_query_count();

        terminal
            .set_cursor_position(Position::new(3, 4))
            .expect("cursor should move");
        assert_eq!(
            terminal.get_cursor_position().expect("cursor should read"),
            Position::new(3, 4)
        );
        assert_eq!(
            terminal.backend_mut().inner().cursor_query_count(),
            initial_query_count,
            "tracked cursor reads should not call the inner crossterm position query again"
        );

        terminal
            .backend_mut()
            .append_lines(2)
            .expect("append should scroll from tracked cursor");
        assert_eq!(
            terminal.get_cursor_position().expect("cursor should read"),
            Position::new(0, 6)
        );
        assert_eq!(
            terminal.backend_mut().inner().cursor_query_count(),
            initial_query_count,
            "append-line cursor tracking should still avoid another terminal query"
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
        app.history_insert_mode = HistoryInsertionMode::StandardScrollRegion;
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
        app.history_insert_mode = HistoryInsertionMode::StandardScrollRegion;
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

        assert!(rendered.contains(" █████╗ ██╗  ██╗██████╗  █████╗"));
        assert!(rendered.contains("╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝"));
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

    fn draw_test_frame<B>(
        terminal: &mut Terminal<InlineTerminalBackend<B>>,
        runtime: &mut ShellRuntime,
        inline_terminal: &mut InlineTerminalState,
    ) where
        InlineTerminalBackend<B>: InlineResizeBackend,
        <InlineTerminalBackend<B> as Backend>::Error: std::fmt::Debug,
    {
        draw_inline_frame(terminal, runtime, inline_terminal).expect("draw test frame");
    }

    fn append_history_message(app: &mut NativeTuiApp, text: &str) {
        append_message(app, ConversationMessageKind::Agent, text);
    }

    fn append_user_history_message(app: &mut NativeTuiApp, text: &str) {
        append_message(app, ConversationMessageKind::User, text);
    }

    fn append_message(app: &mut NativeTuiApp, kind: ConversationMessageKind, text: &str) {
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should start in a ready conversation state");
        };
        conversation
            .messages
            .push(ConversationMessage::new(kind, text.to_string(), None, None));
        conversation.refresh_conversation_lines();
    }

    struct CursorQueryCountingBackend {
        inner: TestBackend,
        cursor_query_count: usize,
    }

    impl CursorQueryCountingBackend {
        fn new(inner: TestBackend) -> Self {
            Self {
                inner,
                cursor_query_count: 0,
            }
        }

        fn cursor_query_count(&self) -> usize {
            self.cursor_query_count
        }
    }

    impl Backend for CursorQueryCountingBackend {
        type Error = Infallible;

        fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
        where
            I: Iterator<Item = (u16, u16, &'a Cell)>,
        {
            self.inner.draw(content)
        }

        fn hide_cursor(&mut self) -> Result<(), Self::Error> {
            self.inner.hide_cursor()
        }

        fn show_cursor(&mut self) -> Result<(), Self::Error> {
            self.inner.show_cursor()
        }

        fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
            self.cursor_query_count += 1;
            self.inner.get_cursor_position()
        }

        fn set_cursor_position<P: Into<Position>>(
            &mut self,
            position: P,
        ) -> Result<(), Self::Error> {
            self.inner.set_cursor_position(position)
        }

        fn clear(&mut self) -> Result<(), Self::Error> {
            self.inner.clear()
        }

        fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
            self.inner.clear_region(clear_type)
        }

        fn append_lines(&mut self, line_count: u16) -> Result<(), Self::Error> {
            self.inner.append_lines(line_count)
        }

        fn size(&self) -> Result<Size, Self::Error> {
            self.inner.size()
        }

        fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
            self.inner.window_size()
        }

        fn flush(&mut self) -> Result<(), Self::Error> {
            self.inner.flush()
        }

        fn scroll_region_up(
            &mut self,
            region: Range<u16>,
            line_count: u16,
        ) -> Result<(), Self::Error> {
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
        let mut app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
            PlanningServices::from_workspace_port(Arc::new(
                FilesystemPlanningWorkspaceAdapter::new(),
            )),
        );
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should start with a ready draft conversation");
        };
        conversation.cwd = "/tmp/root".to_string();
        conversation.draft_workspace_directory = "/tmp/root".to_string();
        conversation.replace_planning_runtime_snapshot(PlanningRuntimeSnapshot::uninitialized());
        app
    }
}

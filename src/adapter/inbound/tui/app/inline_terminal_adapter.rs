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
#[path = "inline_terminal_adapter/tests.rs"]
mod tests;

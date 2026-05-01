use ratatui::Terminal;
use ratatui::TerminalOptions;
use ratatui::Viewport;
use ratatui::backend::Backend;
use ratatui::layout::{Position, Rect, Size};
use ratatui::text::Line;

use crate::adapter::inbound::tui::shell_chrome::ShellOverlay;

use super::history_insertion::HistoryInsertionMode;
use super::shell_presentation::{
    build_inline_tail_view, build_startup_banner_lines, format_conversation_lines_with_debug,
};
use super::shell_rendering::{draw, prepare_render_state};
use super::shell_runtime::ShellRuntime;
use super::{
    ConversationState, INLINE_VIEWPORT_HEIGHT, InlineHistoryRenderMode, NativeTuiApp,
    ShellFrontendMode,
};

#[path = "inline_terminal_adapter/backend.rs"]
mod backend;
#[path = "inline_terminal_adapter/history_flush.rs"]
mod history_flush;

pub(super) use self::backend::{InlineResizeBackend, InlineTerminalBackend};
use self::history_flush::{HistoryFlushResult, HistoryFlushState};

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

#[cfg(test)]
#[path = "inline_terminal_adapter/tests.rs"]
mod tests;

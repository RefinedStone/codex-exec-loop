use super::super::tui_testkit;
use super::{
    HistoryInsertionMode, InlineResizeBackend, InlineTerminalBackend, InlineTerminalState,
    ShellRuntime, current_inline_history_lines, draw_inline_frame, draw_inline_transaction,
    sync_inline_viewport, terminal_options_for_render_mode,
};
use crate::adapter::inbound::tui::app::{
    ConversationMessage, ConversationMessageKind, ConversationState, INLINE_VIEWPORT_HEIGHT,
    InlineHistoryRenderMode, NativeTuiApp, PlannerVisibility,
};
use crate::adapter::inbound::tui::shell_chrome::ShellOverlay;
use ratatui::backend::{Backend, ClearType, TestBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::{Terminal, Viewport};
use std::convert::Infallible;
use std::ops::Range;

// These tests pin the terminal-adapter contract between committed host
// history and the live inline tail. They intentionally exercise both
// TestBackend and VT100-backed paths because resize/scrollback behavior
// differs by backend.
#[path = "tests/fixtures.rs"]
mod fixtures;
#[path = "tests/history_flush.rs"]
mod history_flush;
use self::fixtures::make_test_app;

// Host history sync must insert only committed transcript rows; live agent
// deltas stay in the active tail until the turn is completed.
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

// Any direct history insertion invalidates ratatui's back buffer. The next
// frame draw must rebuild the buffer before incremental diffs are trusted.
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

// Newline fallback uses a different insertion path, but it must leave the same
// redraw contract as scroll-region insertion.
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

// A draw transaction flushes committed history and live tail together while
// preserving the split between host scrollback and active viewport content.
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
        !tui_testkit::inline_scrollback_text(&terminal).contains("live tail in same transaction")
    );
}

// VT100 coverage catches terminal-app behavior that TestBackend cannot:
// newline fallback history must survive live completion and viewport resize
// without duplicating markers.
#[test]
fn vt100_terminal_app_preserves_newline_fallback_history_after_live_resize() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
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

// Viewport replay mode repaints the active viewport instead of inserting
// history into host scrollback; host-scrollback mode should do the opposite.
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
        sync_inline_viewport(&mut host_terminal, &mut host_runtime, &mut host_viewport).unwrap()
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

// The adapter tracks cursor position after the initial query so append_lines
// and cursor reads do not repeatedly call through to the inner backend.
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

// Resize paths are regression-prone because committed history and live tail
// are both redrawn; this helper asserts the live prompt remains in viewport
// and never leaks into scrollback.
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

// Counting backend is a probe for adapter behavior: it exposes accidental
// cursor-position queries without depending on a real terminal.
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
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
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

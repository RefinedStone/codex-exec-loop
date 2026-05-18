use super::super::tui_testkit;
use super::{
    HistoryInsertionMode, InlineResizeBackend, InlineTerminalBackend, InlineTerminalState,
    ShellRuntime, current_inline_history_lines, draw_inline_frame, draw_inline_transaction,
    sync_inline_viewport, terminal_options_for_render_mode,
};
use crate::adapter::inbound::tui::app::{
    ConversationMessage, ConversationMessageKind, ConversationState, ConversationViewMode,
    INLINE_VIEWPORT_HEIGHT, InlineHistoryRenderMode, NativeTuiApp, PlanningWorkerVisibility,
};
use crate::adapter::inbound::tui::shell_chrome::ShellOverlay;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeCompletionFeedEntry,
    ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot,
    ParallelModeRuntimeEventFeedEntry, ParallelModeSupervisorDetailSnapshot,
    ParallelModeSupervisorSnapshot, ParallelModeSupervisorState,
};
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

#[derive(Debug)]
struct RecordedInlineFrame {
    label: &'static str,
    screen_text: String,
    host_scrollback_text: String,
    terminal_history_text: String,
    app_event_stream_text: String,
}

#[derive(Default)]
struct InlineFrameRecorder {
    frames: Vec<RecordedInlineFrame>,
}

impl InlineFrameRecorder {
    fn draw_and_record(
        &mut self,
        label: &'static str,
        terminal: &mut Terminal<InlineTerminalBackend<TestBackend>>,
        runtime: &mut ShellRuntime,
        inline_terminal: &mut InlineTerminalState,
    ) {
        draw_inline_transaction(terminal, runtime, inline_terminal)
            .expect("recorded inline draw transaction");
        self.record(label, terminal, runtime);
    }

    fn record(
        &mut self,
        label: &'static str,
        terminal: &Terminal<InlineTerminalBackend<TestBackend>>,
        runtime: &ShellRuntime,
    ) {
        self.frames.push(RecordedInlineFrame {
            label,
            screen_text: tui_testkit::screen_text(terminal),
            host_scrollback_text: tui_testkit::inline_scrollback_text(terminal),
            terminal_history_text: tui_testkit::inline_terminal_history_text(terminal),
            app_event_stream_text: runtime
                .app()
                .parallel_supervisor_event_lines()
                .into_iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        });
    }

    fn frame(&self, label: &str) -> &RecordedInlineFrame {
        self.frames
            .iter()
            .find(|frame| frame.label == label)
            .unwrap_or_else(|| panic!("recorded frame {label} should exist"))
    }
}

// Host history sync must insert only committed transcript rows; live agent
// deltas stay in the active tail until the turn is completed.
#[test]
fn host_history_sync_keeps_live_agent_delta_out_of_inserted_history() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 160, 24);
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

#[test]
fn parallel_event_stream_flushes_rows_without_live_panel_chrome() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.show_startup_ascii_art = false;
    app.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    append_history_message(
        &mut app,
        "single mode history must not own parallel scrollback",
    );
    for index in 0..40 {
        app.push_parallel_supervisor_event_for_test(
            "11:45:02",
            "Task Intake",
            format!("parallel-event-{index:02}"),
        );
    }
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("parallel event draw transaction");

    let terminal_scrollback = tui_testkit::inline_scrollback_text(&terminal);
    assert!(
        !terminal_scrollback.contains("Parallel Event Stream"),
        "the stream title belongs to the live inline section, not durable host scrollback:\n{terminal_scrollback}"
    );
    assert!(
        terminal_scrollback.contains("parallel-event-00"),
        "parallel board events should be written into host scrollback for operator scrollback:\n{terminal_scrollback}"
    );
    assert!(
        !terminal_scrollback.contains("single mode history must not own parallel scrollback"),
        "parallel mode should suppress the hidden single-mode transcript:\n{terminal_scrollback}"
    );
    let screen_text = tui_testkit::screen_text(&terminal);
    assert!(screen_text.contains("Parallel Event Stream"));
    assert!(
        [
            "parallel-event-37",
            "parallel-event-38",
            "parallel-event-39"
        ]
        .iter()
        .any(|event| screen_text.contains(event)),
        "live stream should show the recent event tail:\n{screen_text}"
    );
    assert!(
        !screen_text.contains("parallel-event-00"),
        "old parallel events should not force the live viewport to show every row:\n{screen_text}"
    );

    for index in 40..60 {
        runtime.app_mut().push_parallel_supervisor_event_for_test(
            "11:45:03",
            "Task Intake",
            format!("parallel-event-{index:02}"),
        );
    }
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("parallel event delta draw transaction");

    let terminal_scrollback = tui_testkit::inline_scrollback_text(&terminal);
    assert!(
        !terminal_scrollback.contains("Parallel Event Stream"),
        "redraws must not replay the live section title into host scrollback:\n{terminal_scrollback}"
    );
    assert_eq!(
        terminal_scrollback.matches("parallel-event-00").count(),
        1,
        "existing parallel events should not duplicate on later redraws:\n{terminal_scrollback}"
    );
    assert_eq!(
        terminal_scrollback.matches("parallel-event-40").count(),
        1,
        "new parallel events should append into host scrollback exactly once:\n{terminal_scrollback}"
    );
}

#[test]
fn parallel_runtime_feed_primes_baseline_without_scrollback_duplication() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.show_startup_ascii_art = false;
    app.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(runtime_feed_supervisor_snapshot(
        vec![
            inline_runtime_feed_entry(2, "seed runtime event two"),
            inline_runtime_feed_entry(1, "seed runtime event one"),
        ],
    )));
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("initial runtime feed draw transaction");

    let terminal_scrollback = tui_testkit::inline_scrollback_text(&terminal);
    assert!(
        !terminal_scrollback.contains("seed runtime event one"),
        "old runtime feed rows should not be replayed above the live parallel board:\n{terminal_scrollback}"
    );
    let screen_text = tui_testkit::screen_text(&terminal);
    assert!(
        !screen_text.contains("seed runtime event one"),
        "old runtime feed rows should not be replayed into the live event stream:\n{screen_text}"
    );
    assert_eq!(
        screen_text.matches("seed runtime event one").count(),
        0,
        "initial runtime feed should stay hidden after the baseline is primed:\n{screen_text}"
    );

    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(runtime_feed_supervisor_snapshot(
            vec![
                inline_runtime_feed_entry(3, "new runtime event three"),
                inline_runtime_feed_entry(2, "seed runtime event two"),
                inline_runtime_feed_entry(1, "seed runtime event one"),
            ],
        )));
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("runtime feed delta draw transaction");

    let app_scrollback = runtime
        .app_mut()
        .parallel_supervisor_event_scrollback_lines()
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        app_scrollback.contains("new runtime event three"),
        "new runtime events should be retained in the durable stream history:\n{app_scrollback}"
    );
    assert!(
        !app_scrollback.contains("seed runtime event one"),
        "primed runtime events must not be backfilled on later redraws:\n{app_scrollback}"
    );
    let terminal_scrollback = tui_testkit::inline_scrollback_text(&terminal);
    assert!(
        !terminal_scrollback.contains("new runtime event three"),
        "new runtime events should stay out of durable host scrollback:\n{terminal_scrollback}"
    );
    let screen_text = tui_testkit::screen_text(&terminal);
    assert!(
        screen_text.contains("new runtime event three"),
        "new runtime events should append to the live event stream after the primed baseline:\n{screen_text}"
    );
    assert!(
        !screen_text.contains("seed runtime event one"),
        "primed runtime events must not be backfilled into the live event stream:\n{screen_text}"
    );
}

#[test]
fn parallel_stream_preserves_initial_status_rows_as_runtime_events_advance() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.show_startup_ascii_art = false;
    app.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(
        status_runtime_feed_supervisor_snapshot(vec![inline_runtime_feed_entry(
            1,
            "seed runtime event one",
        )]),
    ));
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("initial status stream draw transaction");

    let initial_stream = runtime
        .app_mut()
        .parallel_supervisor_event_lines()
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        initial_stream.contains("parallel board 상태를 갱신했습니다"),
        "initial board status should be recorded as stream history:\n{initial_stream}"
    );
    assert!(
        initial_stream.contains("reported 단계 기록: no agent results reported yet"),
        "initial ledger status should be recorded as stream history:\n{initial_stream}"
    );

    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(
            status_runtime_feed_supervisor_snapshot(
                (1..=40)
                    .map(|sequence| {
                        inline_runtime_feed_entry(
                            sequence,
                            format!("runtime stream marker {sequence:02}"),
                        )
                    })
                    .collect(),
            ),
        ));
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("runtime stream tail draw transaction");

    let screen_text = tui_testkit::screen_text(&terminal);
    assert!(
        screen_text.contains("runtime stream marker 40"),
        "live stream should keep following new runtime events:\n{screen_text}"
    );
    let terminal_scrollback = tui_testkit::inline_scrollback_text(&terminal);
    assert!(
        terminal_scrollback.contains("control tower is live")
            && terminal_scrollback.contains("in read-only supervisor mode"),
        "initial board status should move into durable terminal history instead of disappearing:\n{terminal_scrollback}"
    );
    assert!(
        terminal_scrollback.contains("no agent results reported yet"),
        "initial ledger status should move into durable terminal history instead of being replaced:\n{terminal_scrollback}"
    );
    assert_eq!(
        terminal_scrollback
            .matches("no agent results reported yet")
            .count(),
        1,
        "snapshot status rows should not be replayed on every redraw:\n{terminal_scrollback}"
    );
    assert!(
        !terminal_scrollback.contains("Parallel Event Stream"),
        "durable stream history must not include live panel chrome:\n{terminal_scrollback}"
    );
}

#[test]
fn direct_frame_recorder_keeps_parallel_status_rows_across_runtime_redraw() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.show_startup_ascii_art = false;
    app.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(
        status_runtime_feed_supervisor_snapshot(vec![inline_runtime_feed_entry(
            1,
            "seed runtime event one",
        )]),
    ));
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let mut frame_recorder = InlineFrameRecorder::default();

    frame_recorder.draw_and_record(
        "initial-status",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let initial_frame = frame_recorder.frame("initial-status");
    assert!(
        initial_frame.screen_text.contains("Parallel Event Stream"),
        "initial frame should render the live parallel stream panel:\n{}",
        initial_frame.screen_text
    );
    assert!(
        initial_frame
            .app_event_stream_text
            .contains("control tower is live"),
        "initial status rows should be recorded in the app stream immediately:\n{}",
        initial_frame.app_event_stream_text
    );
    assert!(
        initial_frame
            .app_event_stream_text
            .contains("no agent results reported yet"),
        "initial ledger rows should be recorded in the app stream immediately:\n{}",
        initial_frame.app_event_stream_text
    );

    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(
            status_runtime_feed_supervisor_snapshot(
                (1..=40)
                    .map(|sequence| {
                        inline_runtime_feed_entry(
                            sequence,
                            format!("runtime stream marker {sequence:02}"),
                        )
                    })
                    .collect(),
            ),
        ));
    frame_recorder.draw_and_record(
        "runtime-tail",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let runtime_tail_frame = frame_recorder.frame("runtime-tail");
    assert!(
        runtime_tail_frame
            .screen_text
            .contains("runtime stream marker 40"),
        "redraw should keep following the newest runtime event:\n{}",
        runtime_tail_frame.screen_text
    );
    assert!(
        runtime_tail_frame
            .terminal_history_text
            .contains("control tower is live")
            && runtime_tail_frame
                .terminal_history_text
                .contains("in read-only supervisor mode"),
        "initial board status should remain in terminal history after redraw:\n{}",
        runtime_tail_frame.terminal_history_text
    );
    assert!(
        runtime_tail_frame
            .terminal_history_text
            .contains("no agent results reported yet"),
        "initial ledger status should remain in terminal history after redraw:\n{}",
        runtime_tail_frame.terminal_history_text
    );
    assert_eq!(
        runtime_tail_frame
            .terminal_history_text
            .matches("no agent results reported yet")
            .count(),
        1,
        "redraw should not duplicate snapshot status rows:\n{}",
        runtime_tail_frame.terminal_history_text
    );
    assert!(
        !runtime_tail_frame
            .host_scrollback_text
            .contains("Parallel Event Stream"),
        "host scrollback should persist stream rows but not live panel chrome:\n{}",
        runtime_tail_frame.host_scrollback_text
    );
    assert!(
        runtime_tail_frame
            .app_event_stream_text
            .contains("control tower is live")
            && runtime_tail_frame
                .app_event_stream_text
                .contains("runtime stream marker 40"),
        "app-side stream should contain both the initial status and latest runtime event:\n{}",
        runtime_tail_frame.app_event_stream_text
    );
}

#[test]
fn parallel_history_fit_clears_live_panel_before_scrollback_adjustment() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.show_startup_ascii_art = false;
    app.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    for index in 0..12 {
        app.push_parallel_supervisor_event_for_test(
            "11:45:02",
            "Task Intake",
            format!("fit-event-{index:02}"),
        );
    }
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("initial parallel event draw transaction");
    assert!(tui_testkit::screen_text(&terminal).contains("Parallel Event Stream"));

    let viewport_top = terminal.get_frame().area().top();
    inline_terminal.history_flush.visible_history_rows = viewport_top.saturating_add(2);
    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());

    let terminal_scrollback = tui_testkit::inline_scrollback_text(&terminal);
    assert!(
        !terminal_scrollback.contains("Parallel Event Stream"),
        "viewport fitting must not push the live stream title into host scrollback:\n{terminal_scrollback}"
    );
    assert!(
        !terminal_scrollback.contains("Command Hints"),
        "viewport fitting must not push live panel footer chrome into host scrollback:\n{terminal_scrollback}"
    );
}

fn runtime_feed_supervisor_snapshot(
    runtime_event_feed: Vec<ParallelModeRuntimeEventFeedEntry>,
) -> ParallelModeSupervisorSnapshot {
    ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle")
            .with_runtime_event_feed(runtime_event_feed),
        None,
    )
}

fn status_runtime_feed_supervisor_snapshot(
    runtime_event_feed: Vec<ParallelModeRuntimeEventFeedEntry>,
) -> ParallelModeSupervisorSnapshot {
    ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(
            Vec::new(),
            vec![
                ParallelModeCompletionFeedEntry::new("reported", "no agent results reported yet"),
                ParallelModeCompletionFeedEntry::new(
                    "ledger refreshing",
                    "no official refresh workers are active",
                ),
                ParallelModeCompletionFeedEntry::new("official", "nothing is queued for merge"),
                ParallelModeCompletionFeedEntry::new(
                    "merge queued",
                    "no distributor queue items are waiting",
                ),
                ParallelModeCompletionFeedEntry::new(
                    "merged",
                    "nothing has been integrated into prerelease yet",
                ),
            ],
            "idle",
            "queue idle",
        )
        .with_runtime_event_feed(runtime_event_feed),
        Some("control tower is live in read-only supervisor mode".to_string()),
    )
}

fn inline_runtime_feed_entry(
    sequence: i64,
    summary: impl Into<String>,
) -> ParallelModeRuntimeEventFeedEntry {
    ParallelModeRuntimeEventFeedEntry::new(
        sequence,
        "parallel_runtime_reset",
        "parallel_runtime",
        "pool",
        60,
        summary,
        format!("2026-05-13T11:45:{sequence:02}+00:00"),
    )
}

#[test]
fn vt100_parallel_history_fit_does_not_push_live_panel_chrome() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.show_startup_ascii_art = false;
    app.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    for index in 0..12 {
        app.push_parallel_supervisor_event_for_test(
            "11:45:02",
            "Task Intake",
            format!("vt100-fit-event-{index:02}"),
        );
    }
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("initial vt100 parallel event draw transaction");

    let viewport_top = terminal.get_frame().area().top();
    inline_terminal.history_flush.visible_history_rows = viewport_top.saturating_add(2);
    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());

    let host_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert!(
        !host_scrollback.contains("Parallel Event Stream"),
        "vt100 viewport fitting must not push the live stream title into host scrollback:\n{host_scrollback}"
    );
    assert!(
        !host_scrollback.contains("Command Hints"),
        "vt100 viewport fitting must not push live panel footer chrome into host scrollback:\n{host_scrollback}"
    );
}

#[test]
fn vt100_newline_fallback_parallel_delta_keeps_chrome_out_of_host_scrollback() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.show_startup_ascii_art = false;
    app.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.history_insert_mode = HistoryInsertionMode::NewlineFallback;
    app.set_parallel_mode_enabled_for_test(true);
    for index in 0..20 {
        app.push_parallel_supervisor_event_for_test(
            "11:45:02",
            "Task Intake",
            format!("fallback-event-{index:02}"),
        );
    }
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("initial vt100 fallback parallel event draw transaction");
    for index in 20..40 {
        runtime.app_mut().push_parallel_supervisor_event_for_test(
            "11:45:03",
            "Task Intake",
            format!("fallback-event-{index:02}"),
        );
    }
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("delta vt100 fallback parallel event draw transaction");

    let host_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert!(
        host_scrollback.contains("fallback-event-00"),
        "fallback insertion should preserve historical event rows:\n{host_scrollback}"
    );
    assert!(
        host_scrollback.contains("fallback-event-20"),
        "fallback insertion should append delta event rows:\n{host_scrollback}"
    );
    assert!(
        !host_scrollback.contains("Parallel Event Stream"),
        "fallback insertion must not push live stream title into host scrollback:\n{host_scrollback}"
    );
    assert!(
        !host_scrollback.contains("Command Hints"),
        "fallback insertion must not push live footer chrome into host scrollback:\n{host_scrollback}"
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
fn parallel_runtime_live_event_invalidates_hidden_frame_cache() {
    let mut app = make_test_app();
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(runtime_feed_supervisor_snapshot(
        vec![inline_runtime_feed_entry(1, "seed runtime event one")],
    )));
    let mut inline_viewport = InlineTerminalState::default();

    assert!(inline_viewport.should_draw_inline_frame(&app, 80, 24));
    assert!(!inline_viewport.should_draw_inline_frame(&app, 80, 24));

    app.set_parallel_mode_supervisor_snapshot_for_test(Some(runtime_feed_supervisor_snapshot(
        vec![
            inline_runtime_feed_entry(2, "new runtime event two"),
            inline_runtime_feed_entry(1, "seed runtime event one"),
        ],
    )));
    let live_events = app
        .parallel_supervisor_event_lines()
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(live_events.contains("new runtime event two"));
    assert!(inline_viewport.should_draw_inline_frame(&app, 80, 24));
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
fn inline_history_does_not_flush_startup_banner_while_parallel_home_is_active() {
    let mut app = make_test_app();
    app.show_startup_ascii_art = true;
    app.set_parallel_mode_enabled_for_test(true);

    let rendered = current_inline_history_lines(&app)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(!rendered.contains(" █████╗ ██╗  ██╗██████╗  █████╗"));
    assert!(!rendered.contains("╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝"));
    assert!(!rendered.contains("No messages in this thread yet."));
    assert!(rendered.is_empty());
}
#[test]
fn inline_history_shows_planning_worker_debug_detail_when_visibility_is_debug() {
    let mut app = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.messages.push(
        ConversationMessage::new(
            ConversationMessageKind::User,
            "다음 queued-task 1개를 이어서 진행합니다.",
            None,
            None,
        )
        .with_display_label("Auto Follow-up")
        .with_debug_detail("planning worker temporary session: refresh / refresh ok"),
    );
    conversation.refresh_conversation_lines();
    let normal_lines = current_inline_history_lines(&app)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!normal_lines.contains("planning worker temporary session"));

    app.planning_worker_visibility = PlanningWorkerVisibility::Debug;
    let debug_lines = current_inline_history_lines(&app)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(debug_lines.contains("planning worker temporary session: refresh / refresh ok"));
}

#[test]
fn inline_history_view_mode_controls_tool_and_status_rows() {
    let mut app = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "visible codex reply",
        Some("commentary".to_string()),
        None,
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Tool,
        "command: cargo test [completed]",
        None,
        None,
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Status,
        "thread status: running",
        None,
        None,
    ));

    let simple_lines = current_inline_history_lines(&app)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(simple_lines.contains("Codex Commentary:"));
    assert!(!simple_lines.contains("Tool:"));
    assert!(!simple_lines.contains("Status:"));

    app.conversation_view_mode = ConversationViewMode::Medium;
    let medium_lines = current_inline_history_lines(&app)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(medium_lines.contains("Tool:"));
    assert!(medium_lines.contains("Status:"));
}

#[test]
fn host_scrollback_preserves_long_single_completion_beyond_screen_cap() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 100, 30);
    let mut app = make_test_app();
    app.show_startup_ascii_art = false;
    app.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    let body = (0..180)
        .map(|index| match index {
            0 => "LONG_SINGLE_MARKER_FIRST".to_string(),
            90 => "LONG_SINGLE_MARKER_MIDDLE".to_string(),
            179 => "LONG_SINGLE_MARKER_LAST".to_string(),
            _ => format!("long completion filler {index}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    append_history_message(&mut app, &body);
    let capped_lines = {
        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("test app should start in a ready conversation state");
        };
        conversation.cached_conversation_lines.clone()
    };
    assert!(
        !capped_lines
            .iter()
            .any(|line| line.to_string().contains("LONG_SINGLE_MARKER_FIRST")),
        "fixture must prove the live screen projection is capped"
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("draw transaction");
    let terminal_history = tui_testkit::inline_terminal_history_text(&terminal);
    for marker in [
        "LONG_SINGLE_MARKER_FIRST",
        "LONG_SINGLE_MARKER_MIDDLE",
        "LONG_SINGLE_MARKER_LAST",
    ] {
        assert!(
            terminal_history.contains(marker),
            "host scrollback lost {marker} from uncapped completion:\n{terminal_history}"
        );
        assert_eq!(
            terminal_history.matches(marker).count(),
            1,
            "host scrollback duplicated {marker} from uncapped completion:\n{terminal_history}"
        );
    }
}

#[test]
fn host_scrollback_preserves_multiturn_history_beyond_screen_cap_without_duplicates() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 100, 30);
    let mut app = make_test_app();
    app.show_startup_ascii_art = false;
    app.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let markers = (0..75)
        .map(|index| format!("MULTITURN_MARKER_{index:02}"))
        .collect::<Vec<_>>();

    for (index, marker) in markers.iter().enumerate() {
        append_user_history_message(runtime.app_mut(), &format!("prompt {marker}"));
        append_history_message(runtime.app_mut(), &format!("answer {marker}"));
        if index % 5 == 0 {
            draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
                .expect("incremental draw transaction");
        }
    }

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("final draw transaction");
    let terminal_history = tui_testkit::inline_terminal_history_text(&terminal);
    for marker in markers {
        assert!(
            terminal_history.contains(&marker),
            "host scrollback lost {marker} from multiturn history:\n{terminal_history}"
        );
        assert_eq!(
            terminal_history.matches(&marker).count(),
            2,
            "host scrollback should contain prompt and answer once for {marker}:\n{terminal_history}"
        );
    }
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

use ratatui::Terminal;
use ratatui::backend::{Backend, TestBackend};
use ratatui::layout::{Position, Rect};
use ratatui::text::Line;

use crate::adapter::inbound::tui::app::{InlineHistoryRenderMode, MAX_CONVERSATION_HISTORY_LINES};

use super::super::backend::InlineResizeBackend;
use super::super::{
    HistoryFlushState, HistoryInsertionMode, InlineTerminalBackend,
    terminal_options_for_render_mode,
};
use super::tui_testkit;

/*
 * These tests pin the diff layer between the conversation transcript and host
 * terminal scrollback. Inline rendering repeatedly rebuilds the full transcript
 * as ratatui Lines, but the terminal must receive only durable new rows; replaying
 * the whole model on every frame would duplicate history above the live viewport.
 */
#[test]
fn pending_lines_returns_only_new_suffix_for_appended_history() {
    /*
     * The normal path is append-only: a submitted prompt is already in scrollback
     * and a later status block is added behind it. The cache must return only the
     * suffix so draw_inline_transaction can invalidate the frame without reprinting
     * the old prompt.
     */
    let state = HistoryFlushState {
        rendered_lines: vec![
            Line::from("User:"),
            Line::from("  first prompt"),
            Line::from(""),
        ],
        parallel_rendered_lines: Vec::new(),
        pending_history_lines: Vec::new(),
        visible_history_rows: 0,
        visible_history_rows_dirty: false,
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
    /*
     * Session switches deliberately break the old baseline. A small status-only
     * transcript for a newly opened thread should not be interpreted as a suffix of
     * the previous thread; the host scrollback needs a complete marker for the new
     * conversation boundary.
     */
    let state = HistoryFlushState {
        rendered_lines: vec![
            Line::from("User:"),
            Line::from("  old thread"),
            Line::from(""),
        ],
        parallel_rendered_lines: Vec::new(),
        pending_history_lines: Vec::new(),
        visible_history_rows: 0,
        visible_history_rows_dirty: false,
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
    /*
     * Once the conversation reaches MAX_CONVERSATION_HISTORY_LINES, the model is a
     * rolling window. Here the first three lines fell off and three new lines
     * appeared at the tail, so the overlap detector should preserve scrollback
     * continuity and emit only those three tail rows.
     */
    let state = HistoryFlushState {
        rendered_lines: (0..MAX_CONVERSATION_HISTORY_LINES)
            .map(|idx| Line::from(format!("line {idx}")))
            .collect(),
        parallel_rendered_lines: Vec::new(),
        pending_history_lines: Vec::new(),
        visible_history_rows: 0,
        visible_history_rows_dirty: false,
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
    /*
     * This is the transition into capped history rather than a steady capped
     * window. The old baseline is shorter than the cap, so only the overlapping
     * tail of the old baseline can be trusted; the missing old prefix plus the new
     * tail must be inserted to keep host scrollback aligned with what the user saw.
     */
    let state = HistoryFlushState {
        rendered_lines: (0..MAX_CONVERSATION_HISTORY_LINES - 10)
            .map(|idx| Line::from(format!("line {idx}")))
            .collect(),
        parallel_rendered_lines: Vec::new(),
        pending_history_lines: Vec::new(),
        visible_history_rows: 0,
        visible_history_rows_dirty: false,
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
    /*
     * Prompt/status fragments are intentionally repetitive. A tiny overlap at the
     * front of a new transcript is not proof of a capped rolling window; treating it
     * that way would hide the beginning of a newly loaded thread from scrollback.
     */
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
        parallel_rendered_lines: Vec::new(),
        pending_history_lines: Vec::new(),
        visible_history_rows: 0,
        visible_history_rows_dirty: false,
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
    /*
     * A large textual overlap is still unsafe until the current transcript is
     * exactly at the shared cap. This protects ordinary uncapped session changes
     * where two threads begin with the same setup/status rows but diverge near the
     * tail.
     */
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
        parallel_rendered_lines: Vec::new(),
        pending_history_lines: Vec::new(),
        visible_history_rows: 0,
        visible_history_rows_dirty: false,
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
    /*
     * sync is the production write barrier: it computes pending lines, inserts them
     * through HistoryInsertionAdapter, refreshes the baseline, and reports whether
     * the host scrollback moved. InlineTerminalAdapter uses inserted() to decide
     * whether the ratatui back buffer is now untrustworthy.
     */
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut state = HistoryFlushState::default();
    let current_lines = vec![
        Line::from("User:"),
        Line::from("  first prompt"),
        Line::from(""),
    ];

    let snapshot = terminal.backend().resize_snapshot().unwrap();
    assert!(
        state
            .sync(
                &mut terminal,
                &current_lines,
                snapshot,
                HistoryInsertionMode::StandardScrollRegion,
            )
            .unwrap()
            .inserted()
    );

    // Repeating the same model must be a no-op after the baseline is refreshed.
    let snapshot = terminal.backend().resize_snapshot().unwrap();
    assert!(
        !state
            .sync(
                &mut terminal,
                &current_lines,
                snapshot,
                HistoryInsertionMode::StandardScrollRegion,
            )
            .unwrap()
            .inserted()
    );

    // Appending an agent block moves scrollback again and should force a redraw.
    let appended_lines = vec![
        Line::from("User:"),
        Line::from("  first prompt"),
        Line::from(""),
        Line::from("Agent:"),
        Line::from("  first answer"),
        Line::from(""),
    ];
    let snapshot = terminal.backend().resize_snapshot().unwrap();
    assert!(
        state
            .sync(
                &mut terminal,
                &appended_lines,
                snapshot,
                HistoryInsertionMode::StandardScrollRegion,
            )
            .unwrap()
            .inserted()
    );
}

#[test]
fn first_parallel_event_preserves_one_shot_handoff_rows() {
    for insert_mode in [
        HistoryInsertionMode::StandardScrollRegion,
        HistoryInsertionMode::NewlineFallback,
    ] {
        let mut terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
        let mut state = HistoryFlushState::default();
        let handoff_lines = vec![
            Line::from("User: one-shot prompt"),
            Line::from("Agent: one-shot answer"),
            Line::from(""),
        ];

        let snapshot = terminal.backend().resize_snapshot().unwrap();
        let handoff = state
            .append_durable_lines_preserving_baseline(
                &mut terminal,
                &handoff_lines,
                snapshot,
                insert_mode,
            )
            .unwrap();
        assert!(handoff.inserted());
        let handoff_visible_rows = state.visible_history_rows;

        let parallel_lines = vec![Line::from("parallel event")];
        let snapshot = terminal.backend().resize_snapshot().unwrap();
        let first_event = state
            .sync_parallel(&mut terminal, &parallel_lines, snapshot, insert_mode)
            .unwrap();
        assert!(first_event.inserted());
        assert_eq!(
            state.visible_history_rows,
            handoff_visible_rows
                .saturating_add(1)
                .min(terminal.get_frame().area().top())
        );

        let visible_rows_after_first_event = state.visible_history_rows;
        let snapshot = terminal.backend().resize_snapshot().unwrap();
        assert!(
            !state
                .sync_parallel(&mut terminal, &[], snapshot, insert_mode)
                .unwrap()
                .inserted()
        );
        assert_eq!(state.parallel_rendered_lines, parallel_lines);
        assert_eq!(state.visible_history_rows, visible_rows_after_first_event);

        let snapshot = terminal.backend().resize_snapshot().unwrap();
        assert!(
            !state
                .sync_parallel(&mut terminal, &parallel_lines, snapshot, insert_mode)
                .unwrap()
                .inserted()
        );
        assert_eq!(state.visible_history_rows, visible_rows_after_first_event);
    }
}

#[test]
fn shifted_parallel_window_inserts_only_its_new_tail() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut state = HistoryFlushState {
        parallel_rendered_lines: (0..10)
            .map(|index| Line::from(format!("parallel event {index}")))
            .collect(),
        ..HistoryFlushState::default()
    };
    let shifted_lines = (1..11)
        .map(|index| Line::from(format!("parallel event {index}")))
        .collect::<Vec<_>>();

    let snapshot = terminal.backend().resize_snapshot().unwrap();
    let result = state
        .sync_parallel(
            &mut terminal,
            &shifted_lines,
            snapshot,
            HistoryInsertionMode::StandardScrollRegion,
        )
        .unwrap();

    assert!(result.inserted());
    assert_eq!(state.visible_history_rows, 1);
    assert_eq!(state.parallel_rendered_lines, shifted_lines);
}

#[test]
fn history_sync_for_empty_thread_clears_baseline_without_losing_geometry() {
    /*
     * Empty transcripts appear while a thread is being replaced or before history
     * has loaded. They should clear the old semantic baseline without pretending
     * that already-rendered terminal rows disappeared; the next non-empty thread
     * then replays in full and adds to the physical row count.
     */
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
        parallel_rendered_lines: Vec::new(),
        pending_history_lines: Vec::new(),
        visible_history_rows: 6,
        visible_history_rows_dirty: false,
    };

    let snapshot = terminal.backend().resize_snapshot().unwrap();
    assert!(
        !state
            .sync(
                &mut terminal,
                &[],
                snapshot,
                HistoryInsertionMode::StandardScrollRegion,
            )
            .unwrap()
            .inserted()
    );
    assert!(state.rendered_lines.is_empty());
    assert_eq!(state.visible_history_rows, 6);

    let next_thread_lines = vec![Line::from("Status:"), Line::from("  new thread loaded")];
    assert_eq!(state.pending_lines(&next_thread_lines), next_thread_lines);
}

#[test]
fn conversation_projection_reset_preserves_parallel_baseline_and_geometry() {
    let mut state = HistoryFlushState {
        rendered_lines: vec![Line::from("old conversation")],
        parallel_rendered_lines: vec![Line::from("parallel event")],
        pending_history_lines: vec![Line::from("pending conversation")],
        visible_history_rows: 7,
        visible_history_rows_dirty: true,
    };

    state.reset_conversation_projection();

    assert!(state.rendered_lines.is_empty());
    assert!(state.pending_history_lines.is_empty());
    assert_eq!(
        state.parallel_rendered_lines,
        vec![Line::from("parallel event")]
    );
    assert_eq!(state.visible_history_rows, 7);
    assert!(state.visible_history_rows_dirty);
}

#[test]
fn conversation_reset_full_insert_accumulates_rows_after_viewport_fit() {
    for insert_mode in [
        HistoryInsertionMode::StandardScrollRegion,
        HistoryInsertionMode::NewlineFallback,
    ] {
        let mut backend = TestBackend::new(80, 24);
        backend
            .set_cursor_position(Position::new(0, 23))
            .expect("fixture cursor should start at the physical bottom");
        let mut terminal = Terminal::with_options(
            InlineTerminalBackend::new(backend),
            terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback),
        )
        .expect("bottom-anchored inline fixture");
        let mut state = HistoryFlushState::default();
        let old_conversation = (0..8)
            .map(|index| Line::from(format!("old conversation row {index}")))
            .collect::<Vec<_>>();
        let snapshot = terminal.backend().resize_snapshot().unwrap();
        assert!(
            state
                .sync(&mut terminal, &old_conversation, snapshot, insert_mode)
                .unwrap()
                .inserted()
        );
        let frame_area = terminal.get_frame().area();
        assert!(frame_area.top() >= 3, "inline fixture needs history space");
        state.reset_conversation_projection();

        let fit_top = frame_area.top() - 2;
        let fit_area = Rect::new(
            frame_area.x,
            fit_top,
            frame_area.width,
            frame_area.bottom().saturating_sub(fit_top),
        );
        let snapshot = terminal.backend().resize_snapshot().unwrap();
        assert_eq!(
            state
                .fit_visible_rows_to_viewport(&mut terminal, snapshot, fit_area)
                .unwrap(),
            Some(true)
        );
        assert_eq!(state.visible_history_rows, fit_top);

        let replacement = vec![Line::from("new conversation"), Line::from("second row")];
        let snapshot = terminal.backend().resize_snapshot().unwrap();
        assert!(
            state
                .sync(&mut terminal, &replacement, snapshot, insert_mode)
                .unwrap()
                .inserted()
        );
        let expected_rows = fit_top
            .saturating_add(2)
            .min(terminal.get_frame().area().top());
        assert_eq!(state.visible_history_rows, expected_rows);

        let snapshot = terminal.backend().resize_snapshot().unwrap();
        assert!(
            !state
                .sync(&mut terminal, &replacement, snapshot, insert_mode)
                .unwrap()
                .inserted()
        );
        assert_eq!(state.visible_history_rows, expected_rows);
    }
}

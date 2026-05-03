use ratatui::text::Line;

use crate::adapter::inbound::tui::app::{InlineHistoryRenderMode, MAX_CONVERSATION_HISTORY_LINES};

use super::super::{HistoryFlushState, HistoryInsertionMode};
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

    // Repeating the same model must be a no-op after the baseline is refreshed.
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

    // Appending an agent block moves scrollback again and should force a redraw.
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
    /*
     * Empty transcripts appear while a thread is being replaced or before history
     * has loaded. They should clear the old baseline and visible row count without
     * emitting blank scrollback rows; the next non-empty thread then replays in
     * full instead of diffing against stale history.
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

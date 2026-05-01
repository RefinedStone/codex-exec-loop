use ratatui::text::Line;

use crate::adapter::inbound::tui::app::{InlineHistoryRenderMode, MAX_CONVERSATION_HISTORY_LINES};

use super::super::{HistoryFlushState, HistoryInsertionMode};
use super::tui_testkit;

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

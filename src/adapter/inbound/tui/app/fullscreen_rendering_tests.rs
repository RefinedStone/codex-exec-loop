use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;

use super::*;
use crate::adapter::inbound::tui::app::shell_runtime::ShellRuntime;
use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;
use crate::domain::conversation::ConversationMessageKind;

fn ready_conversation_mut(app: &mut NativeTuiApp) -> &mut ConversationViewModel {
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should contain a ready conversation");
    };
    conversation
}

fn render(app: &mut NativeTuiApp, width: u16, height: u16) -> String {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("fullscreen test terminal");
    terminal
        .draw(|frame| draw(frame, app, ShellFrontendMode::Fullscreen))
        .expect("fullscreen frame should render");
    buffer_text(terminal.backend().buffer())
}

fn render_buffer(app: &mut NativeTuiApp, width: u16, height: u16) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("fullscreen test terminal");
    terminal
        .draw(|frame| draw(frame, app, ShellFrontendMode::Fullscreen))
        .expect("fullscreen frame should render");
    terminal.backend().buffer().clone()
}

fn buffer_text(buffer: &Buffer) -> String {
    if buffer.area.width == 0 {
        return String::new();
    }
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|cells| {
            cells
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn seed_long_transcript(app: &mut NativeTuiApp, rows: usize) {
    let conversation = ready_conversation_mut(app);
    conversation.messages.clear();
    for index in 0..rows {
        assert!(conversation.finalize_agent_message(
            format!("agent-{index}"),
            Some("commentary".to_string()),
            format!("stable transcript row {index:02}"),
        ));
    }
}

fn click_first_visible_tool_card(app: &mut NativeTuiApp, _width: u16, _height: u16) -> bool {
    let Some(hit_area) = app
        .shell
        .transcript_viewport_ui_state
        .card_hit_areas()
        .first()
        .copied()
    else {
        return false;
    };
    let column = hit_area.area.x;
    let row = hit_area.area.y;
    let down = app.handle_transcript_mouse_event(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
    let up = app.handle_transcript_mouse_event(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
    down && up
}

#[test]
fn fullscreen_transcript_owns_history_and_keeps_the_composer_visible() {
    let mut app = test_native_tui_app();
    seed_long_transcript(&mut app, 40);

    let screen = render(&mut app, 80, 24);

    assert!(screen.contains("stable transcript row 39"));
    assert!(screen.contains("Describe a task") || screen.contains("Task"));
    assert!(!screen.contains("host scrollback"));
}

#[test]
fn startup_screen_renders_the_akra_logo_above_the_composer() {
    let mut app = test_native_tui_app();
    app.shell.show_startup_ascii_art = true;

    let screen = render(&mut app, 80, 24);

    assert!(screen.contains("██████╗"));
    assert!(screen.contains("Describe a task"));
    let logo_position = screen.find("██████╗").expect("startup logo should render");
    let composer_position = screen
        .find("Describe a task")
        .expect("composer should remain visible");
    assert!(
        logo_position < composer_position,
        "startup logo should stay above the composer:\n{screen}"
    );
}

#[test]
fn short_startup_screen_centers_the_logo_without_transcript_scrolling() {
    let mut app = test_native_tui_app();
    app.shell.show_startup_ascii_art = true;
    let projection = FullscreenConversationFrameProjection::from_app(&app, 80);
    let available_logo_height = 8_u16.saturating_sub(projection.tail_view.rendered_height(80, 8));
    let expected_logo_lines = startup_ascii_art_lines(Some(available_logo_height))
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    let screen = render(&mut app, 80, 8);

    for line in expected_logo_lines {
        assert!(
            screen.contains(&line),
            "missing centered startup logo row: {line}"
        );
    }
    assert!(screen.contains("Describe a task"));
}

#[test]
fn transcript_window_rebases_scroll_offsets_beyond_u16_without_losing_the_target_row() {
    let lines = (0..1_000)
        .map(|index| Line::from(format!("{index:04}:{}", "x".repeat(75))))
        .collect::<Vec<_>>();

    let (line_index, local_scroll) = transcript_window_for_scroll(&lines, 1, 70_000);

    assert_eq!(line_index, 875);
    assert_eq!(local_scroll, 0);
}

#[test]
fn transcript_wrap_layout_preserves_word_separators_without_inventing_hard_wrap_spaces() {
    let interactions = [ConversationTranscriptLineInteraction {
        selection_range_id: Some(1),
        selectable_from_column: 0,
    }];
    let word_wrap = transcript_wrapped_row_layout(&[Line::from("hello world")], &interactions, 5);
    assert_eq!(word_wrap.len(), 2);
    assert_eq!(word_wrap[0].soft_wrap_separator, " ");
    assert!(word_wrap[1].soft_wrap_separator.is_empty());

    let hard_wrap = transcript_wrapped_row_layout(&[Line::from("helloworld")], &interactions, 5);
    assert_eq!(hard_wrap.len(), 2);
    assert!(hard_wrap[0].soft_wrap_separator.is_empty());
}

#[test]
fn transcript_wrap_layout_matches_ratatui_row_counts() {
    for (text, width) in [
        ("", 5),
        ("hello world", 5),
        ("hello  world", 5),
        ("helloworld", 5),
        ("한글 문장 줄바꿈", 6),
        (" leading and trailing ", 8),
    ] {
        let lines = [Line::from(text)];
        assert_eq!(
            transcript_wrapped_row_layout(
                &lines,
                &vec![
                    ConversationTranscriptLineInteraction {
                        selection_range_id: Some(1),
                        selectable_from_column: 0,
                    };
                    lines.len()
                ],
                width,
            )
            .len(),
            count_wrapped_rows(&lines, width),
            "row metadata must track Ratatui for {text:?} at width {width}"
        );
    }
}

#[test]
fn streaming_append_does_not_move_a_reader_and_exposes_a_new_output_badge() {
    let mut app = test_native_tui_app();
    seed_long_transcript(&mut app, 48);
    let _ = render(&mut app, 80, 24);
    assert!(app.jump_transcript_to_top());
    let before = render(&mut app, 80, 24);
    assert!(before.contains("stable transcript row 00"));

    assert!(ready_conversation_mut(&mut app).finalize_agent_message(
        "agent-late".to_string(),
        Some("commentary".to_string()),
        "late streaming output".to_string(),
    ));
    let during_stream = render(&mut app, 80, 24);

    assert!(during_stream.contains("stable transcript row 00"));
    assert!(!during_stream.contains("late streaming output"));
    assert!(during_stream.contains("new output"));

    assert!(app.follow_latest_transcript());
    let latest = render(&mut app, 80, 24);
    assert!(latest.contains("late streaming output"));
    assert!(!latest.contains("new output"));
}

#[test]
fn new_output_badge_remains_non_selectable_ui_chrome() {
    let mut app = test_native_tui_app();
    let conversation = ready_conversation_mut(&mut app);
    conversation.messages.clear();
    let initial_output = (0..40)
        .map(|index| format!("selectable transcript row {index:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(conversation.finalize_agent_message(
        "agent-initial".to_string(),
        Some("commentary".to_string()),
        initial_output,
    ));
    let _ = render(&mut app, 80, 24);
    assert!(app.jump_transcript_to_top());
    assert!(ready_conversation_mut(&mut app).finalize_agent_message(
        "agent-late".to_string(),
        Some("commentary".to_string()),
        "late streaming output".to_string(),
    ));
    let _ = render(&mut app, 80, 24);
    let snapshot = app
        .shell
        .transcript_viewport_ui_state
        .frame_snapshot()
        .cloned()
        .expect("badge draw should bind the committed transcript cells");
    let (visible_row, start_column) = snapshot
        .rows
        .iter()
        .enumerate()
        .find_map(|(visible_row, row)| {
            row.cells
                .windows("new output".len())
                .position(|window| {
                    window.iter().map(String::as_str).collect::<String>() == "new output"
                })
                .map(|column| (visible_row as u16, column as u16))
        })
        .expect("new output badge must exist in the selection snapshot");
    let row = snapshot.area.y.saturating_add(visible_row);
    let start = snapshot.area.x.saturating_add(start_column);
    let badge_row = &snapshot.rows[usize::from(visible_row)];
    assert!(
        badge_row.selection_range_id.is_some(),
        "the badge must overlap a real selectable transcript row for this regression"
    );
    assert!(
        badge_row
            .selection_excluded_columns
            .iter()
            .any(|(range_start, range_end)| {
                start_column >= *range_start && start_column <= *range_end
            }),
        "only the badge columns should be marked as selection chrome"
    );
    assert!(!app.handle_transcript_mouse_event(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: start,
        row,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(app.take_terminal_ui_effects().is_empty());

    let response_start_column = badge_row.selectable_from_column;
    let response_end_column = start_column.saturating_sub(1);
    assert!(response_end_column > response_start_column);
    let viewport = &mut app.shell.transcript_viewport_ui_state;
    assert!(viewport.begin_selection(snapshot.area.x.saturating_add(response_start_column), row,));
    assert!(
        viewport.update_selection(
            snapshot
                .area
                .x
                .saturating_add(response_start_column.saturating_add(3)),
            row,
        )
    );
    let highlighted = render_buffer(&mut app, 80, 24);
    for column in response_start_column..=response_start_column.saturating_add(3) {
        assert_eq!(
            highlighted
                .cell(Position::new(snapshot.area.x.saturating_add(column), row))
                .expect("selected response cell")
                .style()
                .bg,
            AkraTheme::transcript_selection().bg
        );
    }
    for column in start_column..snapshot.area.width {
        assert_ne!(
            highlighted
                .cell(Position::new(snapshot.area.x.saturating_add(column), row))
                .expect("badge cell")
                .style()
                .bg,
            AkraTheme::transcript_selection().bg,
            "badge columns must not inherit transcript selection styling"
        );
    }
    let viewport = &mut app.shell.transcript_viewport_ui_state;
    assert!(matches!(
        viewport.finish_selection(
            snapshot
                .area
                .x
                .saturating_add(response_start_column.saturating_add(3)),
            row,
        ),
        TranscriptSelectionFinish::Copy(_)
    ));
}

#[test]
fn read_card_is_quiet_by_default_and_expands_in_the_conversation() {
    let mut app = test_native_tui_app();
    let conversation = ready_conversation_mut(&mut app);
    conversation.messages.clear();
    conversation.append_tool_message_with_detail(
        "Read src/lib.rs",
        Some("read".to_string()),
        Some("read-1".to_string()),
        Some("path: C:/dev/akra/src/lib.rs\nlines: 1-40".to_string()),
    );

    let collapsed = render(&mut app, 100, 24);
    assert!(collapsed.contains("Read src/lib.rs"));
    assert!(!collapsed.contains("path: C:/dev/akra/src/lib.rs"));

    assert!(click_first_visible_tool_card(&mut app, 100, 24));
    let expanded = render(&mut app, 100, 24);
    assert!(expanded.contains("path: C:/dev/akra/src/lib.rs"));
    assert!(expanded.contains("lines: 1-40"));
}

#[test]
fn patch_card_expands_to_semantic_diff_lines_inside_the_conversation() {
    let mut app = test_native_tui_app();
    let conversation = ready_conversation_mut(&mut app);
    conversation.messages.clear();
    conversation.append_tool_message_with_detail(
        "file change: update src/lib.rs",
        Some("patch".to_string()),
        Some("patch-1".to_string()),
        Some(
            "[update] src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -7 +7 @@\n-old\n+new"
                .to_string(),
        ),
    );

    let collapsed = render(&mut app, 100, 24);
    assert!(!collapsed.contains("+new"));
    assert!(app.toggle_latest_transcript_tool_card());
    let expanded = render(&mut app, 100, 24);

    assert!(expanded.contains("src/lib.rs"));
    assert!(expanded.contains("-old"));
    assert!(expanded.contains("+new"));
    assert!(
        ready_conversation_mut(&mut app)
            .messages
            .iter()
            .any(|message| message.kind == ConversationMessageKind::Tool)
    );
}

#[test]
fn ctrl_e_expands_the_latest_visible_tool_card() {
    let mut app = test_native_tui_app();
    let conversation = ready_conversation_mut(&mut app);
    conversation.messages.clear();
    conversation.append_tool_message_with_detail(
        "Explored 2 targets",
        Some("explore".to_string()),
        Some("explore-1".to_string()),
        Some("1. Read src/core/app.rs\n2. Read src/domain/conversation.rs".to_string()),
    );
    let mut runtime = ShellRuntime::new(app);
    let collapsed = render(runtime.app_mut(), 100, 24);
    assert!(!collapsed.contains("src/domain/conversation.rs"));

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('e'),
        KeyModifiers::CONTROL,
    )));
    let expanded = render(runtime.app_mut(), 100, 24);

    assert!(expanded.contains("src/core/app.rs"));
    assert!(expanded.contains("src/domain/conversation.rs"));
}

#[test]
fn transcript_drag_highlights_exact_cells_and_queues_clipboard_copy() {
    let mut app = test_native_tui_app();
    let conversation = ready_conversation_mut(&mut app);
    conversation.messages.clear();
    assert!(conversation.finalize_agent_message(
        "agent-selection".to_string(),
        Some("final".to_string()),
        "selectable words".to_string(),
    ));
    let _ = render(&mut app, 80, 24);
    let snapshot = app
        .shell
        .transcript_viewport_ui_state
        .frame_snapshot()
        .cloned()
        .expect("stable draw should bind transcript cells");
    let (visible_row, start_column) = snapshot
        .rows
        .iter()
        .enumerate()
        .find_map(|(visible_row, row)| {
            row.cells
                .windows("selectable".len())
                .position(|window| {
                    window.iter().map(String::as_str).collect::<String>() == "selectable"
                })
                .map(|column| (visible_row as u16, column as u16))
        })
        .expect("selection fixture should be visible");
    let row = snapshot.area.y.saturating_add(visible_row);
    let start = snapshot.area.x.saturating_add(start_column);
    let end = start.saturating_add("selectable".len() as u16 - 1);

    assert!(app.handle_transcript_mouse_event(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: start,
        row,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(app.handle_transcript_mouse_event(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: end,
        row,
        modifiers: KeyModifiers::NONE,
    }));
    let highlighted = render_buffer(&mut app, 80, 24);
    for column in start..=end {
        assert_eq!(
            highlighted
                .cell(Position::new(column, row))
                .expect("selected cell")
                .style()
                .bg,
            AkraTheme::transcript_selection().bg
        );
    }

    assert!(app.handle_transcript_mouse_event(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: end,
        row,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(
        app.take_terminal_ui_effects(),
        vec![TerminalUiEffect::CopyToClipboard("selectable".to_string())]
    );
}

#[test]
fn submitted_prompt_is_visible_but_does_not_capture_a_drag() {
    let mut app = test_native_tui_app();
    let conversation = ready_conversation_mut(&mut app);
    conversation.messages.clear();
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::User,
        "prompt remains application chrome",
        None,
        Some("user-selection-zone".to_string()),
    ));
    assert!(conversation.finalize_agent_message(
        "agent-selection-zone".to_string(),
        Some("final".to_string()),
        "response remains selectable".to_string(),
    ));
    let _ = render(&mut app, 80, 24);
    let snapshot = app
        .shell
        .transcript_viewport_ui_state
        .frame_snapshot()
        .cloned()
        .expect("stable draw should bind transcript cells");
    let (visible_row, start_column) = snapshot
        .rows
        .iter()
        .enumerate()
        .find_map(|(visible_row, row)| {
            row.cells
                .windows("prompt remains".len())
                .position(|window| {
                    window.iter().map(String::as_str).collect::<String>() == "prompt remains"
                })
                .map(|column| (visible_row, column as u16))
        })
        .expect("prompt fixture should be visible");
    assert_eq!(snapshot.rows[visible_row].selection_range_id, None);
    assert!(
        !app.handle_transcript_mouse_event(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: snapshot.area.x.saturating_add(start_column),
            row: snapshot
                .area
                .y
                .saturating_add(u16::try_from(visible_row).unwrap_or(u16::MAX)),
            modifiers: KeyModifiers::NONE,
        })
    );
}

#[test]
fn reference_capture_is_rendered_by_the_real_fullscreen_testbackend() {
    let mut app = test_native_tui_app();
    let conversation = ready_conversation_mut(&mut app);
    conversation.messages.clear();
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::User,
        "Refactor the transcript into one app-owned fullscreen viewport.",
        None,
        Some("user-1".to_string()),
    ));
    assert!(conversation.finalize_agent_message(
        "agent-1".to_string(),
        Some("commentary".to_string()),
        "I’ll inspect the stream reducer and terminal transaction.".to_string(),
    ));
    conversation.append_tool_message_with_detail(
        "Explored 3 targets",
        Some("explore".to_string()),
        Some("explore-1".to_string()),
        Some(
            "1. src/adapter/inbound/tui/app/conversation_runtime.rs\n2. src/adapter/inbound/tui/app/fullscreen_frame_model.rs\n3. src/adapter/inbound/tui/app/shell_rendering.rs"
                .to_string(),
        ),
    );
    assert!(conversation.finalize_agent_message(
        "agent-2".to_string(),
        Some("commentary".to_string()),
        "The canonical row can now be updated in place without transcript replay.".to_string(),
    ));
    conversation.append_tool_message_with_detail(
        "file change: update transcript viewport",
        Some("patch".to_string()),
        Some("patch-1".to_string()),
        Some(
            "[update] src/adapter/inbound/tui/app/transcript_viewport_ui.rs\n--- a/transcript_viewport_ui.rs\n+++ b/transcript_viewport_ui.rs\n@@ -42 +42 @@\n-host_scrollback\n+app_owned_viewport"
                .to_string(),
        ),
    );

    let _ = render(&mut app, 100, 30);
    assert!(app.toggle_latest_transcript_tool_card());
    let screen = render(&mut app, 100, 30);
    assert!(screen.contains("app_owned_viewport"));
    assert!(screen.contains("Describe a task"));

    if std::env::var_os("AKRA_CAPTURE_FULLSCREEN_FRAME").is_some() {
        println!("\n--- AKRA FULLSCREEN 100x30 ---\n{screen}\n--- END FRAME ---");
    }
}

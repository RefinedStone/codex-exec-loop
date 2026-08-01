use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

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

fn click_first_visible_tool_card(app: &mut NativeTuiApp, width: u16, height: u16) -> bool {
    (0..height).any(|row| {
        app.handle_transcript_mouse_event(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: width.saturating_sub(1),
            row,
            modifiers: KeyModifiers::NONE,
        })
    })
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
fn transcript_window_rebases_scroll_offsets_beyond_u16_without_losing_the_target_row() {
    let lines = (0..1_000)
        .map(|index| Line::from(format!("{index:04}:{}", "x".repeat(75))))
        .collect::<Vec<_>>();

    let (line_index, local_scroll) = transcript_window_for_scroll(&lines, 1, 70_000);

    assert_eq!(line_index, 875);
    assert_eq!(local_scroll, 0);
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

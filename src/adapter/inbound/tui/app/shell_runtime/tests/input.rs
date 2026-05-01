use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use super::{
    ConversationState, InlineShellCommand, ShellOverlay, StartupState, make_test_runtime,
    sample_startup_diagnostics,
};

#[test]
fn plain_character_input_uses_empty_modifier_check() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::empty(),
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.input_buffer, "a");
    assert!(runtime.take_redraw_request());
}

#[test]
fn supersession_overlay_allows_prompt_input() {
    let mut runtime = make_test_runtime();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::empty(),
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.input_buffer, "a");
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn supersession_overlay_allows_r_prompt_input() {
    let mut runtime = make_test_runtime();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('r'),
        KeyModifiers::empty(),
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.input_buffer, "r");
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn supersession_overlay_ctrl_r_refreshes_readiness() {
    let mut runtime = make_test_runtime();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('r'),
        KeyModifiers::CONTROL,
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(conversation.input_buffer.is_empty());
    assert!(
        conversation
            .status_text
            .starts_with("parallel readiness refreshed / state:")
    );
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn supersession_overlay_allows_enter_to_submit_prompt() {
    let mut runtime = make_test_runtime();
    runtime.app_mut().startup_state = StartupState::Ready(sample_startup_diagnostics(
        &runtime.app().current_workspace_directory(),
    ));
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    for character in "run next".chars() {
        runtime.app_mut().push_input_character(character);
    }
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(conversation.input_buffer.is_empty());
    assert!(conversation.has_running_turn());
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn enter_executes_selected_inline_command_palette_item() {
    let mut runtime = make_test_runtime();
    runtime.app_mut().push_input_character(':');
    runtime.app_mut().push_input_character('d');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Startup);
    assert!(conversation.input_buffer.is_empty());
    assert!(
        conversation
            .status_text
            .contains("opened diagnostics inspection")
    );
}

#[test]
fn down_then_enter_on_palette_item_with_argument_inserts_completion() {
    let mut runtime = make_test_runtime();
    runtime.app_mut().push_input_character(':');
    runtime.app_mut().push_input_character('r');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.input_buffer, ":reset ");
    assert!(!conversation.inline_shell_command_palette_state.is_active());
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Hidden);
}

#[test]
fn up_wraps_inline_command_palette_selection() {
    let mut runtime = make_test_runtime();
    runtime.app_mut().push_input_character(':');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(
        conversation
            .inline_shell_command_palette_state
            .selected_command(),
        Some(InlineShellCommand::Help)
    );
}

#[test]
fn escape_dismisses_inline_command_palette_without_clearing_buffer() {
    let mut runtime = make_test_runtime();
    runtime.app_mut().push_input_character(':');
    runtime.app_mut().push_input_character('p');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.input_buffer, ":p");
    assert!(!conversation.inline_shell_command_palette_state.is_active());
}

#[test]
fn page_navigation_keys_do_not_trigger_transcript_navigation() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::PageUp,
        KeyModifiers::NONE,
    )));

    assert!(!runtime.take_redraw_request());
}

#[test]
fn ctrl_u_clears_buffered_input() {
    let mut runtime = make_test_runtime();
    runtime.app_mut().push_input_character('s');
    runtime.app_mut().push_input_character('h');
    runtime.app_mut().push_input_character('i');
    runtime.app_mut().push_input_character('p');

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(conversation.input_buffer.is_empty());
}

#[test]
fn ctrl_w_deletes_previous_buffered_word() {
    let mut runtime = make_test_runtime();
    for character in "ship this next".chars() {
        runtime.app_mut().push_input_character(character);
    }

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('w'),
        KeyModifiers::CONTROL,
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.input_buffer, "ship this ");
}

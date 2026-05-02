use super::{
    ConversationState, InlineShellCommand, ShellOverlay, StartupState, make_test_runtime,
    sample_startup_diagnostics,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

/*
학습 주석: 이 파일은 production terminal event loop의 key routing contract를 고정합니다.
`ShellRuntime::handle_terminal_event`는 crossterm `Event`를 받아 overlay, inline command palette,
conversation input reducer, startup 상태를 한 번에 조정하므로, 작은 키 조합 하나가 엉뚱한
surface로 새지 않는지 테스트에서 직접 검증합니다.
*/

#[test]
fn plain_character_input_uses_empty_modifier_check() {
    // 학습 주석: plain character는 modifier가 완전히 비어 있을 때만 prompt buffer로 들어가야
    // 합니다. 이 테스트는 Ctrl/Alt 조합이 일반 입력으로 누수되지 않는 기준선을 잡습니다.
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
    // 학습 주석: supersession overlay는 상태를 보여 주는 비차단 overlay입니다. 사용자가 새 prompt를
    // 계속 입력할 수 있어야 오래된 PR/turn 상태 확인이 작성 흐름을 막지 않습니다.
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
    // 학습 주석: `r`은 Ctrl-R refresh shortcut의 문자라 regression이 나기 쉽습니다. modifier가
    // 없으면 overlay shortcut이 아니라 prompt text로 남아야 합니다.
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
    // 학습 주석: 같은 `r`이라도 Ctrl modifier가 붙으면 parallel readiness refresh로 라우팅됩니다.
    // buffer가 비어 있고 overlay가 유지되는지 함께 확인해 refresh가 작성 중 입력을 망가뜨리지 않게 합니다.
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
    // 학습 주석: supersession overlay가 떠 있어도 Enter는 prompt submit 흐름으로 들어가야 합니다.
    // startup diagnostics를 Ready로 만든 이유는 submit guard가 startup readiness에 막히지 않게 하기 위해서입니다.
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
    // 학습 주석: colon command palette에서 실행형 항목을 고르면 prompt submit이 아니라 shell command
    // executor로 들어가야 합니다. `:d`는 diagnostics overlay를 여는 대표적인 side effect입니다.
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
    // 학습 주석: argument가 필요한 palette item은 즉시 실행하지 않고 buffer completion만 삽입합니다.
    // `:reset `처럼 공백까지 포함한 입력을 남겨 사용자가 대상 argument를 이어서 칠 수 있게 합니다.
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
    // 학습 주석: palette selection은 위쪽 이동에서 끝 항목으로 wrap됩니다. keyboard-only 사용자가
    // 짧은 prefix 상태에서도 모든 command에 접근할 수 있게 하는 navigation contract입니다.
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
    // 학습 주석: Escape는 palette chrome만 닫고 사용자가 입력한 raw command prefix는 보존합니다.
    // 그래야 palette suggestion을 숨긴 뒤에도 일반 prompt text로 계속 편집할 수 있습니다.
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
    // 학습 주석: PageUp/PageDown은 예전 transcript navigation과 충돌하던 키입니다. 현재 input
    // runtime에서는 별도 redraw를 요구하지 않는 no-op로 고정해 terminal scrollback과 충돌을 피합니다.
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
    // 학습 주석: Ctrl-U는 shell-style line kill shortcut입니다. conversation reducer를 거쳐
    // prompt buffer만 비우고 session/overlay 상태는 건드리지 않는지 확인합니다.
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
    // 학습 주석: Ctrl-W는 직전 단어만 제거하는 편집 shortcut입니다. 공백을 보존한 결과를 확인해
    // 다음 단어 입력이 자연스럽게 이어지는 shell-like editing contract를 고정합니다.
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

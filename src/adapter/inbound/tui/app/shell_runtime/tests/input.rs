use super::{
    ConversationState, InlineShellCommand, ShellOverlay, StartupState, make_test_runtime,
    sample_startup_diagnostics,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot, ParallelModeDistributorSnapshot,
    ParallelModePoolBoardSnapshot, ParallelModeSupervisorDetailSnapshot,
    ParallelModeSupervisorSnapshot, ParallelModeSupervisorState,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Instant;

/*
이 테스트 모듈은 production terminal event loop의 key routing contract를 고정한다.
`ratatui_frontend`는 crossterm `Event`를 그대로 `ShellRuntime::handle_terminal_event`에 넘기고,
runtime은 overlay, inline command palette, conversation input reducer, startup submit guard로
분기한다. 작은 modifier 차이 하나가 prompt text, shell command, refresh shortcut, submit flow 사이를
바꿀 수 있으므로 이 파일은 "어느 surface가 키를 소비하는가"를 직접 검증한다.
*/

#[test]
fn plain_character_input_uses_empty_modifier_check() {
    /*
     * plain character는 modifier가 완전히 비어 있을 때만 prompt buffer로 들어가야 한다.
     * Ctrl/Alt 조합이 일반 입력으로 누수되면 shortcut과 prompt text가 동시에 반응하므로,
     * 이 테스트가 character input route의 기준선을 잡는다.
     */
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
fn supersession_overlay_blocks_prompt_input_while_loading() {
    /*
     * Supersession overlay는 loading 중에만 일반 prompt 입력을 막는다. 이 시점에는
     * pool reset/reconcile/dispatch가 진행 중이라 새 prompt 작성과 섞이면 상태를 읽기 어렵다.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().parallel_mode_enabled = true;
    runtime.app_mut().parallel_mode_supervisor_snapshot =
        Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(0, "loading: pool", "loading", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "loading agent roster"),
            ParallelModeSupervisorDetailSnapshot::new(None, "loading detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "loading", "loading"),
            Some("loading 2/4: pool reconcile".to_string()),
        ));
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::empty(),
    )));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(conversation.input_buffer.is_empty());
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn supersession_overlay_allows_prompt_input_after_loading_finishes() {
    /*
     * Loading이 끝나 concrete supervisor snapshot이 들어오면 Supersession board를 열어 둔 채로도
     * prompt editing은 다시 가능해야 한다. Ctrl+R/Ctrl+P 같은 board shortcut만 overlay가 계속 소유한다.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().parallel_mode_enabled = true;
    runtime.app_mut().parallel_mode_supervisor_snapshot =
        Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        ));
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
fn supervisor_invalidation_keeps_cached_board_visible() {
    /*
     * Worker updates invalidate supervisor data after dispatch. The visible board
     * must not fall back to the loading placeholder while the replacement snapshot
     * is being refreshed in the background.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().parallel_mode_enabled = true;
    runtime.app_mut().parallel_mode_supervisor_snapshot =
        Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        ));

    runtime
        .app_mut()
        .invalidate_parallel_mode_supervisor_snapshot();

    assert_eq!(
        runtime
            .app()
            .parallel_mode_supervisor_snapshot()
            .pool
            .configured_size,
        3
    );
}

#[test]
fn supersession_active_worker_requests_live_pulse() {
    /*
     * Active parallel workers need periodic redraws even when no stream event arrives,
     * otherwise the Supersession board looks frozen while a worker is running.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().parallel_mode_enabled = true;
    runtime.app_mut().parallel_mode_supervisor_snapshot =
        Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "running", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(
                vec![ParallelModeAgentRosterEntry::new(
                    "agent-1",
                    "Task One",
                    "slot-1",
                    "akra-agent/slot-1/task-one",
                    "running",
                    "12s",
                    "working",
                )],
                "no active agents",
            ),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        ));

    assert!(runtime.app().live_activity_pulse(Instant::now()).is_some());
}

#[test]
fn supersession_overlay_blocks_plain_r_prompt_input_while_loading() {
    /*
     * `r`은 Ctrl-R refresh shortcut과 같은 문자다. modifier가 없으면 overlay control도 아니지만,
     * Supersession loading 중에는 prompt text로도 내려가면 안 된다.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().parallel_mode_enabled = true;
    runtime.app_mut().parallel_mode_supervisor_snapshot =
        Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(0, "loading: pool", "loading", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "loading agent roster"),
            ParallelModeSupervisorDetailSnapshot::new(None, "loading detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "loading", "loading"),
            Some("loading 2/4: pool reconcile".to_string()),
        ));
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('r'),
        KeyModifiers::empty(),
    )));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(conversation.input_buffer.is_empty());
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn supersession_overlay_ctrl_r_refreshes_readiness() {
    /*
     * 같은 `r`이라도 Ctrl modifier가 붙으면 supersession overlay의 parallel readiness refresh로 간다.
     * refresh는 status만 갱신해야 하므로 prompt buffer를 비우거나 overlay를 닫는 부작용이 없는지 함께 확인한다.
     */
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
fn supersession_overlay_blocks_enter_submit_prompt_while_loading() {
    /*
     * Supersession overlay가 loading 중이면 Enter도 prompt submit으로 내려가지 않는다.
     * startup diagnostics를 Ready로 만든 이유는 startup guard가 아니라 overlay routing을
     * 직접 검증하기 위해서다.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().startup_state = StartupState::Ready(sample_startup_diagnostics(
        &runtime.app().current_workspace_directory(),
    ));
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().parallel_mode_enabled = true;
    runtime.app_mut().parallel_mode_supervisor_snapshot =
        Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(0, "loading: pool", "loading", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "loading agent roster"),
            ParallelModeSupervisorDetailSnapshot::new(None, "loading detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "loading", "loading"),
            Some("loading 2/4: pool reconcile".to_string()),
        ));
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
    assert_eq!(conversation.input_buffer, "run next");
    assert!(!conversation.has_running_turn());
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn enter_executes_selected_inline_command_palette_item() {
    /*
     * colon command palette에서 실행형 항목을 고르면 prompt submit이 아니라 shell command executor로 간다.
     * `:d`는 diagnostics overlay를 여는 대표 side effect라, command execution route가 실제 overlay
     * 상태까지 바꾸는지 확인하기 좋다.
     */
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
    /*
     * argument가 필요한 palette item은 즉시 실행하지 않고 buffer completion만 삽입한다.
     * `:reset `처럼 공백까지 포함한 입력을 남겨 사용자가 대상 argument를 이어서 칠 수 있게 한다.
     */
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
    /*
     * Palette selection은 위쪽 이동에서 끝 항목으로 wrap된다. keyboard-only 사용자가 짧은 prefix
     * 상태에서도 모든 command에 접근할 수 있게 하는 navigation contract다.
     */
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
    /*
     * Escape는 palette chrome만 닫고 사용자가 입력한 raw command prefix는 보존한다.
     * 그래야 suggestion을 숨긴 뒤에도 같은 buffer를 일반 prompt text처럼 계속 편집할 수 있다.
     */
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
    /*
     * PageUp/PageDown은 예전 transcript navigation과 host terminal scrollback이 충돌하던 키다.
     * 현재 input runtime에서는 redraw도 요구하지 않는 no-op로 고정해 terminal이 가진 scrollback
     * behavior와 앱 내부 navigation이 경쟁하지 않게 한다.
     */
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
    /*
     * Ctrl-U는 shell-style line kill shortcut이다. conversation reducer를 거쳐 prompt buffer만 비우고
     * session/overlay 상태는 건드리지 않는지 확인한다.
     */
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
    /*
     * Ctrl-W는 직전 단어만 제거하는 shell-style editing shortcut이다. 공백을 보존한 결과를 확인해
     * 다음 단어 입력이 자연스럽게 이어지는 prompt editing contract를 고정한다.
     */
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

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use insta::assert_snapshot;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::super::tui_testkit;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::contract_tests::{
    make_test_app, sample_planning_editor_session, sample_startup_diagnostics,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::*;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::adapter::inbound::tui::app::test_helpers::sample_planning_runtime_snapshot;

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn inline_main_buffer_ready_shell_matches_snapshot() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = tui_testkit::render_inline_snapshot(&mut app, 80, 24);

    assert!(rendered.contains("prompt: new thread ready"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("inline_main_buffer_ready_shell", rendered);
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn queue_overlay_matches_snapshot() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context\nQueue Summary",
        "Queue Summary",
    ));
    app.shell_overlay = ShellOverlay::Queue;

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = tui_testkit::render_shell_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("Ready Queue"));
    assert!(rendered.contains("Queue Summary"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("queue_overlay", rendered);
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn planning_manual_editor_matches_snapshot() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell_overlay = ShellOverlay::PlanningInit;
    app.planning_init_overlay_ui_state.open_manual_editor();
    app.planning_draft_editor_ui_state
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .open_session(sample_planning_editor_session());

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = tui_testkit::render_shell_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("Planning Draft"));
    assert!(rendered.contains("result-output.md"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("planning_manual_editor", rendered);
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn inline_main_buffer_viewport_replay_keeps_recent_transcript_while_streaming() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    tui_testkit::append_agent_history_message(
        &mut app,
        "previous transcript should remain visible in viewport replay mode",
    );
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: rank 1 / terminal-bridge plan",
    ));
    conversation.record_turn_started("turn-1".to_string());
    conversation.push_live_agent_delta(
        "agent-1".to_string(),
        Some("final_answer".to_string()),
        "streaming reply still visible".to_string(),
    );

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = tui_testkit::render_inline_snapshot(&mut app, 80, 24);

    assert_eq!(
        rendered
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .matches("previous transcript should remain vis")
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .count(),
        1
    );
    assert_eq!(rendered.matches("streaming reply still visible").count(), 1);
    assert_snapshot!("inline_main_buffer_viewport_replay_streaming", rendered);
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn vt100_ready_shell_matches_snapshot() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 96, 32);

    assert!(rendered.contains("prompt: new thread ready"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("vt100_ready_shell", rendered);
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn vt100_streaming_shell_matches_snapshot() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    tui_testkit::set_live_agent_message(
        &mut app,
        "streaming delta should stay in the transcript until completion",
    );

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 80, 24);

    assert_eq!(
        rendered
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .matches("streaming delta should stay in the transcript until completion")
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .count(),
        1
    );
    assert!(rendered.contains("Codex:"));
    assert!(!rendered.contains("live: Codex"));
    assert!(!rendered.contains("ghost"));
    assert_snapshot!("vt100_streaming_shell", rendered);
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn vt100_viewport_replay_streaming_matches_snapshot() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    tui_testkit::append_agent_history_message(
        &mut app,
        "viewport replay transcript remains anchored",
    );
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    tui_testkit::set_live_agent_message(&mut app, "viewport replay stream remains separate");

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 80, 24);

    assert_eq!(rendered.matches("viewport replay transcript").count(), 1);
    assert_eq!(
        rendered
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .matches("viewport replay stream remains separate")
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .count(),
        1
    );
    assert!(rendered.contains("Codex:"));
    assert!(!rendered.contains("live: Codex"));
    assert_snapshot!("vt100_viewport_replay_streaming", rendered);
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn vt100_markdown_code_block_shell_matches_snapshot() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    tui_testkit::set_live_agent_message(&mut app, "```rust\nlet ok = true;\n```");

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 96, 32);

    assert_snapshot!("vt100_markdown_code_block_shell", rendered);
    assert!(rendered.contains("let ok = true;"));
    assert_eq!(rendered.matches("```").count(), 2);
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn vt100_queue_overlay_matches_snapshot() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context\nQueue Summary",
        "Queue Summary",
    ));
    app.shell_overlay = ShellOverlay::Queue;

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = tui_testkit::render_shell_vt100_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("Ready Queue"));
    assert!(rendered.contains("Proposals"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("vt100_queue_overlay", rendered);
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn vt100_planning_manual_editor_matches_snapshot() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell_overlay = ShellOverlay::PlanningInit;
    app.planning_init_overlay_ui_state.open_manual_editor();
    app.planning_draft_editor_ui_state
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .open_session(sample_planning_editor_session());

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = tui_testkit::render_shell_vt100_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("Planning Draft"));
    assert!(rendered.contains("controls: Ctrl+S saves and validates"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("vt100_planning_manual_editor", rendered);
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn vt100_narrow_shell_matches_snapshot() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    tui_testkit::set_live_agent_message(&mut app, "narrow resize keeps the live tail visible");

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 48, 10);

    assert_snapshot!("vt100_narrow_shell", rendered);
    assert!(rendered.contains("turn running"));
    assert!(rendered.lines().all(|line| line.chars().count() <= 48));
}

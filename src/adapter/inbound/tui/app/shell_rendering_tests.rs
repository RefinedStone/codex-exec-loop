use insta::assert_snapshot;

use super::super::tui_testkit;
use super::contract_tests::{
    make_test_app, sample_planning_editor_session, sample_startup_diagnostics,
};
use super::*;
use crate::adapter::inbound::tui::app::test_helpers::sample_planning_runtime_snapshot;

#[test]
fn inline_main_buffer_ready_shell_matches_snapshot() {
    // 학습 주석: 기본 inline shell은 startup diagnostics가 준비된 상태에서 frame border 없이 transcript/prompt만 보여야 합니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    let rendered = tui_testkit::render_inline_snapshot(&mut app, 80, 24);

    assert!(rendered.contains("prompt: new thread ready"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("inline_main_buffer_ready_shell", rendered);
}

#[test]
fn queue_overlay_matches_snapshot() {
    // 학습 주석: queue overlay snapshot은 planning runtime summary가 popup section으로 압축되는 계약을 잠급니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context\nQueue Summary",
        "Queue Summary",
    ));
    app.shell_overlay = ShellOverlay::Queue;

    let rendered = tui_testkit::render_shell_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("Ready Queue"));
    assert!(rendered.contains("Queue Summary"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("queue_overlay", rendered);
}

#[test]
fn planning_manual_editor_matches_snapshot() {
    // 학습 주석: manual editor overlay는 planning init shell 안에서 draft files와 editor controls를 함께 렌더링해야 합니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell_overlay = ShellOverlay::PlanningInit;
    app.planning_init_overlay_ui_state.open_manual_editor();
    app.planning_draft_editor_ui_state
        .open_session(sample_planning_editor_session());

    let rendered = tui_testkit::render_shell_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("Planning Draft"));
    assert!(rendered.contains("result-output.md"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("planning_manual_editor", rendered);
}

#[test]
fn inline_main_buffer_viewport_replay_keeps_recent_transcript_while_streaming() {
    // 학습 주석: viewport replay mode는 과거 transcript와 live stream tail을 같은 frame에서 중복 없이 보존해야 합니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
    tui_testkit::append_agent_history_message(
        &mut app,
        "previous transcript should remain visible in viewport replay mode",
    );
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

    let rendered = tui_testkit::render_inline_snapshot(&mut app, 80, 24);

    assert_eq!(
        rendered
            .matches("previous transcript should remain vis")
            .count(),
        1
    );
    assert_eq!(rendered.matches("streaming reply still visible").count(), 1);
    assert_snapshot!("inline_main_buffer_viewport_replay_streaming", rendered);
}

#[test]
fn vt100_ready_shell_matches_snapshot() {
    // 학습 주석: vt100 backend도 inline renderer와 같은 ready shell copy를 ANSI terminal output으로 보존해야 합니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 96, 32);

    assert!(rendered.contains("prompt: new thread ready"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("vt100_ready_shell", rendered);
}

#[test]
fn vt100_streaming_shell_matches_snapshot() {
    // 학습 주석: streaming vt100 snapshot은 live delta가 transcript tail에 한 번만 들어가고 legacy live label이 사라졌는지 검증합니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    tui_testkit::set_live_agent_message(
        &mut app,
        "streaming delta should stay in the transcript until completion",
    );

    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 80, 24);

    assert_eq!(
        rendered
            .matches("streaming delta should stay in the transcript until completion")
            .count(),
        1
    );
    assert!(rendered.contains("Codex:"));
    assert!(!rendered.contains("live: Codex"));
    assert!(!rendered.contains("ghost"));
    assert_snapshot!("vt100_streaming_shell", rendered);
}

#[test]
fn vt100_viewport_replay_streaming_matches_snapshot() {
    // 학습 주석: vt100 viewport replay는 persisted transcript와 live stream을 분리해 scroll replay regression을 잡습니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
    tui_testkit::append_agent_history_message(
        &mut app,
        "viewport replay transcript remains anchored",
    );
    tui_testkit::set_live_agent_message(&mut app, "viewport replay stream remains separate");

    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 80, 24);

    assert_eq!(rendered.matches("viewport replay transcript").count(), 1);
    assert_eq!(
        rendered
            .matches("viewport replay stream remains separate")
            .count(),
        1
    );
    assert!(rendered.contains("Codex:"));
    assert!(!rendered.contains("live: Codex"));
    assert_snapshot!("vt100_viewport_replay_streaming", rendered);
}

#[test]
fn vt100_markdown_code_block_shell_matches_snapshot() {
    // 학습 주석: markdown code fence는 terminal renderer를 지나도 fence와 code line이 같이 남아야 합니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    tui_testkit::set_live_agent_message(&mut app, "```rust\nlet ok = true;\n```");

    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 96, 32);

    assert_snapshot!("vt100_markdown_code_block_shell", rendered);
    assert!(rendered.contains("let ok = true;"));
    assert_eq!(rendered.matches("```").count(), 2);
}

#[test]
fn vt100_queue_overlay_matches_snapshot() {
    // 학습 주석: queue overlay의 vt100 path는 inline snapshot과 같은 planning sections를 terminal backend에서 검증합니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context\nQueue Summary",
        "Queue Summary",
    ));
    app.shell_overlay = ShellOverlay::Queue;

    let rendered = tui_testkit::render_shell_vt100_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("Ready Queue"));
    assert!(rendered.contains("Proposals"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("vt100_queue_overlay", rendered);
}

#[test]
fn vt100_planning_manual_editor_matches_snapshot() {
    // 학습 주석: manual editor의 vt100 snapshot은 controls/help copy가 terminal backend에서도 보존되는지 확인합니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell_overlay = ShellOverlay::PlanningInit;
    app.planning_init_overlay_ui_state.open_manual_editor();
    app.planning_draft_editor_ui_state
        .open_session(sample_planning_editor_session());

    let rendered = tui_testkit::render_shell_vt100_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("Planning Draft"));
    assert!(rendered.contains("controls: Ctrl+S saves and validates"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("vt100_planning_manual_editor", rendered);
}

#[test]
fn vt100_narrow_shell_matches_snapshot() {
    // 학습 주석: narrow shell snapshot은 terminal width를 넘는 줄이 생기지 않도록 resize contract를 잠급니다.
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    tui_testkit::set_live_agent_message(&mut app, "narrow resize keeps the live tail visible");

    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 48, 10);

    assert_snapshot!("vt100_narrow_shell", rendered);
    assert!(rendered.contains("turn running"));
    assert!(rendered.lines().all(|line| line.chars().count() <= 48));
}

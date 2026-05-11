use insta::assert_snapshot;

use super::super::tui_testkit;
use super::contract_tests::{
    make_test_app, sample_planning_editor_session, sample_startup_diagnostics,
};
use super::*;
use crate::adapter::inbound::tui::app::test_helpers::sample_planning_runtime_projection;

#[test]
fn inline_main_buffer_ready_shell_matches_snapshot() {
    /*
     * InlineMainBuffer는 host terminal scrollback 안에 직접 그리는 primary frontend다.
     * ready shell snapshot은 popup frame border 없이 transcript/prompt chrome만 남는지 확인해,
     * inline renderer가 modal layout을 잘못 끌고 오지 않도록 막는다.
     */
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    let rendered = tui_testkit::render_inline_snapshot(&mut app, 80, 24);

    assert!(rendered.contains("prompt: new thread ready"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("inline_main_buffer_ready_shell", rendered);
}

#[test]
fn queue_overlay_matches_snapshot() {
    /*
     * Queue overlay snapshot은 planning runtime projection을 popup summary/queue/proposal/note section으로
     * 압축되는 presentation contract를 잠근다. domain queue ranking 자체는 다른 테스트가 맡고,
     * 여기서는 shell frame이 그 read model을 좁은 overlay에 어떻게 배치하는지 본다.
     */
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.replace_planning_runtime_projection(sample_planning_runtime_projection(
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
    /*
     * Manual editor overlay는 planning init flow에서 staged draft buffers와 editor control copy를 함께 보여 준다.
     * 이 snapshot은 editor 상태가 popup chrome, file list, footer key guide로 끝까지 전달되는지 확인한다.
     */
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
    /*
     * ViewportReplay는 host scrollback을 믿지 않고 최근 transcript를 viewport에 다시 그린다.
     * streaming 중에는 persisted history와 live tail이 동시에 보이되 중복되면 안 되므로,
     * 이 테스트는 replay buffer와 live delta lane의 병합 경계를 고정한다.
     */
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
    conversation.replace_planning_runtime_projection(sample_planning_runtime_projection(
        "Planning Context",
        "queue head: rank 1 / terminal-bridge plan",
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
    /*
     * vt100 backend snapshot은 real terminal escape output을 통과한 결과를 본다.
     * TestBackend의 cell buffer와 달리 ANSI backend path에서 ready shell copy와 border-free inline layout이
     * 같은지 확인해 backend별 rendering drift를 잡는다.
     */
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 96, 32);

    assert!(rendered.contains("prompt: new thread ready"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("vt100_ready_shell", rendered);
}

#[test]
fn vt100_streaming_shell_matches_snapshot() {
    /*
     * Streaming vt100 snapshot은 live delta가 transcript tail에 한 번만 들어가는지 확인한다.
     * 과거 live label path가 남아 있으면 `live: Codex`나 ghost text가 같이 보일 수 있어,
     * terminal output 기준으로 legacy lane 제거를 검증한다.
     */
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
    /*
     * vt100 + ViewportReplay 조합은 persisted transcript replay와 live stream tail을 모두 ANSI backend로 통과시킨다.
     * scroll replay regression은 보통 이 조합에서 중복/누락으로 나타나므로 두 문자열이 각각 한 번만 남는지 본다.
     */
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
    /*
     * Markdown code fence는 syntax-ish text이지만 terminal renderer는 내용을 잃지 말아야 한다.
     * fence 두 개와 code line이 ANSI output 뒤에도 남는지 확인해 markdown line projection과 wrapping이
     * code block structure를 지우지 않게 한다.
     */
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
    /*
     * Queue overlay의 vt100 path는 popup planning sections가 terminal backend에서도 보존되는지 확인한다.
     * TestBackend snapshot만 통과하면 ANSI write/clear/resize path의 section loss를 놓칠 수 있어
     * queue/proposal headings를 실제 terminal output에서도 고정한다.
     */
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.replace_planning_runtime_projection(sample_planning_runtime_projection(
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
    /*
     * Planning manual editor의 vt100 snapshot은 draft editor controls/help copy가 terminal backend에서도
     * 보존되는지 확인한다. 특히 footer key guide는 좁은 terminal layout에서 잘리기 쉬워 snapshot으로
     * editor affordance를 고정한다.
     */
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
    /*
     * Narrow shell snapshot은 resize contract다. Inline renderer는 terminal width를 넘는 줄을 만들면
     * host terminal scrollback과 cursor tracking이 흔들리므로, vt100 output의 모든 visible line이
     * requested width 안에 들어오는지 확인한다.
     */
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    tui_testkit::set_live_agent_message(&mut app, "narrow resize keeps the live tail visible");

    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 48, 10);

    assert_snapshot!("vt100_narrow_shell", rendered);
    assert!(rendered.contains("turn running"));
    assert!(rendered.lines().all(|line| line.chars().count() <= 48));
}

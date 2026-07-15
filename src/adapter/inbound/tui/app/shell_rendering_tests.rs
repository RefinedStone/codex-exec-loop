use insta::assert_snapshot;

use super::super::tui_testkit;
use super::contract_tests::{
    make_test_app, sample_planning_editor_session, sample_startup_diagnostics,
};
use super::*;
use crate::adapter::inbound::tui::app::test_helpers::sample_planning_runtime_projection;
use crate::domain::conversation::{ConversationApprovalRequest, ConversationApprovalRequestKind};
use crate::domain::conversation_runtime_envelope::{
    ConversationRuntimeConfigurationObservation, ConversationRuntimeEnvelope,
    ConversationRuntimeLaunchEnvironment, ConversationRuntimeObservedValue,
};
use crate::domain::planning::{
    PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskStatus,
};

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
    app.sync_ready_conversation_planning_runtime_projection(sample_planning_runtime_projection(
        "Planning Context\nQueue Summary",
        "Queue Summary",
    ));
    app.shell_overlay = ShellOverlay::Queue;
    app.sync_queue_overlay_selection();

    let rendered = tui_testkit::render_shell_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("Ready Queue"));
    assert!(rendered.contains("Queue Summary"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("queue_overlay", rendered);
}

#[test]
fn compact_queue_overlay_keeps_hidden_proposal_and_skipped_selection_visible() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let queue_task = |rank: usize| PriorityQueueTask {
        rank,
        task_id: format!("task-{rank}"),
        direction_id: "direction-1".to_string(),
        direction_title: "Direction 1".to_string(),
        task_title: format!("Deep queue task {rank}"),
        status: TaskStatus::Ready,
        combined_priority: 100 - rank as i32,
        updated_at: format!("2026-07-15T00:00:0{rank}Z"),
        rank_reasons: vec!["ready".to_string()],
    };
    let active_tasks = (1..=4).map(queue_task).collect::<Vec<_>>();
    let proposal = PriorityQueueTask {
        rank: 1,
        task_id: "proposal-deep".to_string(),
        direction_id: "direction-1".to_string(),
        direction_title: "Direction 1".to_string(),
        task_title: "Deep proposal selection".to_string(),
        status: TaskStatus::Proposed,
        combined_priority: 40,
        updated_at: "2026-07-15T00:00:05Z".to_string(),
        rank_reasons: vec!["proposed".to_string()],
    };
    app.sync_ready_conversation_planning_runtime_projection(
        crate::application::service::planning::PlanningRuntimeProjection::ready_with_queue_projection(
            "context".to_string(),
            "queue ready".to_string(),
            Some("proposal ready".to_string()),
            active_tasks.first().cloned(),
            PriorityQueueProjection {
                next_task: active_tasks.first().cloned(),
                active_tasks,
                proposed_tasks: vec![proposal],
                skipped_tasks: vec![PriorityQueueSkippedTask {
                    task_id: "skipped-deep".to_string(),
                    task_title: "Deep skipped selection".to_string(),
                    direction_id: "direction-1".to_string(),
                    status: TaskStatus::Ready,
                    reason: "waiting on dependency".to_string(),
                }],
            },
        )
        .with_planning_revision(Some(9)),
    );
    app.shell_overlay = ShellOverlay::Queue;
    app.sync_queue_overlay_selection();
    let task_ids = app
        .queue_action_tasks()
        .into_iter()
        .map(|task| task.task_id)
        .collect::<Vec<_>>();

    app.queue_overlay_ui_state.move_selection(&task_ids, 3);
    let hidden = tui_testkit::render_shell_snapshot(&mut app, 80, 16);
    assert!(hidden.contains("> #4"));
    assert!(hidden.contains("Deep queue task 4"));

    app.queue_overlay_ui_state.move_selection(&task_ids, 1);
    let proposal = tui_testkit::render_shell_snapshot(&mut app, 80, 16);
    assert!(proposal.contains("> #1 [proposed"));
    assert!(proposal.contains("Deep proposal selection"));

    app.queue_overlay_ui_state.move_selection(&task_ids, 1);
    let skipped = tui_testkit::render_shell_snapshot(&mut app, 80, 16);
    assert!(skipped.contains("> [ready / skipped]"));
    assert!(skipped.contains("Deep skipped selection"));

    let narrow_skipped = tui_testkit::render_shell_snapshot(&mut app, 48, 16);
    assert!(narrow_skipped.contains("> [ready / skipped]"));
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
fn approval_overlay_matches_snapshot() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.pending_approval_request = Some(ConversationApprovalRequest {
        approval_id: "approval-render".to_string(),
        server_request_id: "server-42".to_string(),
        method: "item/commandExecution/requestApproval".to_string(),
        kind: ConversationApprovalRequestKind::CommandExecution,
        summary: "Command execution requested; values are bounded and normalized for display."
            .to_string(),
        details: vec![
            "Command: cargo test --lib".to_string(),
            "Working directory: /workspace".to_string(),
            "Reason: verify approval flow".to_string(),
        ],
    });
    app.shell_overlay = ShellOverlay::Approval;

    let rendered = tui_testkit::render_shell_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("Approval Required"));
    assert!(rendered.contains("Command: cargo test --lib"));
    assert!(rendered.contains("Working directory: /workspace"));
    assert!(rendered.contains("Y: approve once"));
    assert_snapshot!("approval_overlay", rendered);

    let narrow = tui_testkit::render_shell_snapshot(&mut app, 48, 18);
    assert!(narrow.contains("Y: approve once"));
    assert!(narrow.contains("N / Esc: decline"));
    assert!(narrow.contains("prompt: paused while an approval decision"));
    assert!(
        narrow.lines().all(|line| {
            line.strip_prefix('"')
                .and_then(|line| line.strip_suffix('"'))
                .unwrap_or(line)
                .chars()
                .count()
                <= 48
        }),
        "narrow approval overlay exceeded its viewport:\n{narrow}"
    );

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should keep a ready conversation state");
    };
    assert!(conversation.mark_approval_decision_submitted(
        "approval-render",
        crate::domain::conversation::ConversationApprovalDecision::Accept,
    ));
    let awaiting_resolution = tui_testkit::render_shell_snapshot(&mut app, 96, 28);
    assert!(awaiting_resolution.contains("Decision submitted: accept"));
    assert!(awaiting_resolution.contains("Decision locked: accept"));
    assert!(awaiting_resolution.contains("Waiting for runtime resolution"));
    assert!(!awaiting_resolution.contains("N / Esc: decline"));
}

#[test]
fn approval_overlay_scrolls_long_permission_details_without_hiding_decision_keys() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.pending_approval_request = Some(ConversationApprovalRequest {
        approval_id: "approval-scroll".to_string(),
        server_request_id: "server-scroll".to_string(),
        method: "item/permissions/requestApproval".to_string(),
        kind: ConversationApprovalRequestKind::Permissions,
        summary: "Additional turn-scoped permissions requested.".to_string(),
        details: (1..=20)
            .map(|index| format!("Rule {index:02}: /workspace/path-{index:02}"))
            .collect(),
    });
    conversation.approval_detail_scroll_offset = usize::MAX;
    app.shell_overlay = ShellOverlay::Approval;

    let rendered = tui_testkit::render_shell_snapshot(&mut app, 80, 20);

    assert!(
        rendered.contains("Requested Details / 14-20 of 20"),
        "unexpected last approval detail page:\n{rendered}"
    );
    assert!(rendered.contains("Rule 14: /workspace/path-14"));
    assert!(rendered.contains("Rule 20: /workspace/path-20"));
    assert!(!rendered.contains("Rule 13: /workspace/path-13"));
    assert!(rendered.contains("Y: approve once"));
    assert!(rendered.contains("Ctrl-C: decline + stop"));
}

#[test]
fn approval_overlay_scrolls_wrapped_command_rows_to_the_exact_suffix() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.pending_approval_request = Some(ConversationApprovalRequest {
        approval_id: "approval-wrapped".to_string(),
        server_request_id: "server-wrapped".to_string(),
        method: "item/commandExecution/requestApproval".to_string(),
        kind: ConversationApprovalRequestKind::CommandExecution,
        summary: "Review the exact bounded command.".to_string(),
        details: vec![
            format!("Command: {}", "safe-prefix ".repeat(30)),
            "Exact suffix: rm -rf protected-output".to_string(),
        ],
    });
    conversation.approval_detail_scroll_offset = usize::MAX;
    app.shell_overlay = ShellOverlay::Approval;

    let rendered = tui_testkit::render_shell_snapshot(&mut app, 48, 20);

    assert!(rendered.contains("Exact suffix: rm -rf"), "{rendered}");
    assert!(rendered.contains("protected-output"), "{rendered}");
    assert!(rendered.contains("Y: approve once"));
    assert!(rendered.contains("Ctrl-C: decline + stop"));
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
    let runtime_projection = sample_planning_runtime_projection(
        "Planning Context",
        "queue head: rank 1 / terminal-bridge plan",
    );
    app.sync_ready_conversation_planning_runtime_projection(runtime_projection);
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
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
fn progressive_activity_rail_matches_wide_and_narrow_snapshots() {
    let secret = "AKRA_RENDER_PROGRESSIVE_SECRET";
    let mut wide_app = make_test_app();
    wide_app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    wide_app.show_startup_ascii_art = false;
    tui_testkit::set_progressive_command_activity(
        &mut wide_app,
        &format!("{secret}\nsecond line"),
        false,
    );

    let wide = tui_testkit::render_inline_snapshot(&mut wide_app, 160, 24);
    assert!(
        wide.contains(
            "notice: activity: cmd:2 lines | active:command | diff:+1 -1 h1 | ctx:75.00% | model:gpt-5.5 | task:P0-D3 rail | lane:cmd1/files2"
        )
    );
    assert!(!wide.contains("requested-model-hidden"));
    assert!(!wide.contains(secret));
    assert_snapshot!("inline_progressive_activity_rail_wide", wide);

    let mut narrow_app = make_test_app();
    narrow_app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    narrow_app.show_startup_ascii_art = false;
    tui_testkit::set_progressive_command_activity(
        &mut narrow_app,
        &format!("{secret}\nsecond line"),
        false,
    );

    let narrow = tui_testkit::render_inline_snapshot(&mut narrow_app, 48, 10);
    assert!(narrow.contains("notice: activity: cmd:2 lines | active:command"));
    assert!(!narrow.contains("diff:"));
    assert!(!narrow.contains("ctx:"));
    assert!(!narrow.contains(secret));
    assert!(
        narrow
            .lines()
            .all(|line| line.trim_matches('"').chars().count() <= 48),
        "{narrow:?}"
    );
    assert_snapshot!("inline_progressive_activity_rail_narrow", narrow);
}

#[test]
fn vt100_progressive_activity_rail_is_transient_and_payload_free() {
    let secret = "AKRA_VT100_PROGRESSIVE_SECRET";
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.show_startup_ascii_art = false;
    tui_testkit::set_progressive_command_activity(
        &mut app,
        &format!("{secret}\nsecond line"),
        false,
    );

    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 160, 24);

    assert!(
        rendered.contains(
            "notice: activity: cmd:2 lines | active:command | diff:+1 -1 h1 | ctx:75.00% | model:gpt-5.5 | task:P0-D3 rail | lane:cmd1/files2"
        )
    );
    assert!(!rendered.contains("requested-model-hidden"));
    assert!(!rendered.contains(secret));
    assert_snapshot!("vt100_progressive_activity_rail", rendered);
}

#[test]
fn narrow_running_rail_never_falls_back_to_raw_coarse_summary() {
    let secret = "AKRA_RAW_COARSE_FALLBACK_SECRET";
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.show_startup_ascii_art = false;
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should keep a ready conversation state");
    };
    conversation.record_thread_prepared(
        "thread-rail".to_string(),
        "Typed rail".to_string(),
        "/tmp/root".to_string(),
    );
    conversation.record_turn_started("turn-rail".to_string());
    conversation.runtime_envelope = Some(ConversationRuntimeEnvelope::prepared(
        Default::default(),
        ConversationRuntimeConfigurationObservation {
            model: ConversationRuntimeObservedValue::Observed("m".repeat(256)),
            ..ConversationRuntimeConfigurationObservation::default()
        },
        ConversationRuntimeLaunchEnvironment::unknown(),
        ConversationRuntimeObservedValue::Missing,
    ));
    conversation.turn_activity.current_turn_command_count = 1;
    conversation.turn_activity.current_turn_last_summary = Some(format!("{secret}\u{1b}[31m"));

    let rendered = tui_testkit::render_inline_snapshot(&mut app, 32, 10);

    assert!(!rendered.contains(secret));
    assert!(!rendered.contains('\u{1b}'));
    assert!(!rendered.contains("tool activity:"));
}

#[test]
fn progressive_activity_inspector_matches_wide_narrow_and_vt100_snapshots() {
    let secret = "AKRA_ACTIVITY_DETAIL_SECRET";
    let detail = format!(
        "한글 wide detail\n@@ -1 +1 @@\n-old value\n+{secret}\u{1b}[31m\n{}",
        (0..48)
            .map(|index| format!("detail line {index:02}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.show_startup_ascii_art = false;
    let core_snapshot = tui_testkit::set_progressive_command_activity(&mut app, &detail, true);
    assert_eq!(std::sync::Arc::strong_count(&core_snapshot), 1);
    assert!(app.show_progressive_activity_overlay(ProgressiveActivityDetailKind::Diff));

    let wide = tui_testkit::render_inline_snapshot(&mut app, 120, 30);
    assert!(wide.contains("Activity / inline inspection"));
    assert!(wide.contains("> Diff"));
    assert!(wide.contains("Retained Diff"));
    assert!(wide.contains("history:incomplete"));
    assert!(wide.contains(&format!("{secret}\\x1b[31m")));
    assert!(!wide.contains('\u{1b}'));
    assert!(wide.contains("prompt:"));
    assert_eq!(std::sync::Arc::strong_count(&core_snapshot), 1);
    assert_snapshot!("inline_progressive_activity_inspector_wide", wide);

    let narrow = tui_testkit::render_inline_snapshot(&mut app, 48, 10);
    assert!(narrow.contains("Activity / inline inspection"));
    assert!(narrow.contains("> Diff"));
    assert!(narrow.contains("Retained Diff"));
    assert!(narrow.contains("status:"), "{narrow}");
    assert!(narrow.contains("prompt:"), "{narrow}");
    assert!(!narrow.contains('\u{1b}'));
    assert!(
        narrow.lines().all(|line| {
            let line = line.strip_prefix('"').unwrap_or(line);
            let line = line.split("\" Hidden by").next().unwrap_or(line);
            let line = line.strip_suffix('"').unwrap_or(line);
            ratatui::text::Line::from(line.to_string()).width() <= 48
        }),
        "narrow activity inspector exceeded its viewport:\n{narrow}"
    );
    assert_snapshot!("inline_progressive_activity_inspector_narrow", narrow);
    assert_eq!(std::sync::Arc::strong_count(&core_snapshot), 1);

    assert!(app.show_progressive_activity_overlay(ProgressiveActivityDetailKind::Output));
    let vt100 = tui_testkit::render_inline_vt100_snapshot(&mut app, 80, 24);
    assert!(vt100.contains("> Output"));
    assert!(vt100.contains("Retained Output Tail"));
    assert!(!vt100.contains("Full Output"));
    assert!(!vt100.contains('\u{1b}'));
    assert_eq!(std::sync::Arc::strong_count(&core_snapshot), 1);
    assert_snapshot!("vt100_progressive_activity_inspector_output", vt100);
}

#[test]
fn activity_inspector_resets_page_when_new_turn_reuses_document_sequence() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.show_startup_ascii_art = false;
    let first_detail = (0..80)
        .map(|index| format!("first turn row {index:03}"))
        .collect::<Vec<_>>()
        .join("\n");
    let _first_core_snapshot =
        tui_testkit::set_progressive_command_activity(&mut app, &first_detail, false);
    assert!(app.show_progressive_activity_overlay(ProgressiveActivityDetailKind::Diff));
    let _ = tui_testkit::render_inline_snapshot(&mut app, 80, 24);
    assert!(
        app.progressive_activity_overlay_ui_state
            .move_to_next_page()
    );
    assert!(
        app.progressive_activity_overlay_ui_state
            .current_page_start()
            > 0
    );

    let _second_core_snapshot = tui_testkit::set_progressive_command_activity(
        &mut app,
        "SECOND_TURN_SAME_SEQUENCE_CANARY\nnext row",
        false,
    );
    let rendered = tui_testkit::render_inline_snapshot(&mut app, 80, 24);

    assert!(rendered.contains("SECOND_TURN_SAME_SEQUENCE_CANARY"));
    assert!(rendered.contains("Retained Diff / bytes 0.."));
    assert_eq!(
        app.progressive_activity_overlay_ui_state
            .current_page_start(),
        0
    );
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
    app.sync_ready_conversation_planning_runtime_projection(sample_planning_runtime_projection(
        "Planning Context\nQueue Summary",
        "Queue Summary",
    ));
    app.shell_overlay = ShellOverlay::Queue;
    app.sync_queue_overlay_selection();

    let rendered = tui_testkit::render_shell_vt100_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("Ready Queue"));
    assert!(rendered.contains("Proposals"));
    assert!(rendered.contains("x/Delete: remove"));
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

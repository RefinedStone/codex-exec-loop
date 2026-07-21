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
    PlanningQueueMutationKind, PlanningQueueMutationReceipt, PlanningQueueMutationReceiptEntry,
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
fn dense_hidden_tail_preserves_prompt_suffix_snapshot() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
    tui_testkit::append_agent_history_message(&mut app, "previous operator-visible response");
    app.sync_ready_conversation_planning_runtime_projection(sample_planning_runtime_projection(
        "Planning Context",
        "queue head: rank 1 / task-1 / Implement shell planning status",
    ));
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.thread_id = "thread-dense-tail".to_string();
    conversation.title = "Dense tail".to_string();
    conversation.warnings = vec!["runtime recovery needs operator attention".to_string()];
    conversation.runtime_notices =
        vec!["attachment recovered with a long operational notice".to_string()];
    conversation.composer.input_buffer = "queued follow-up must remain visible".to_string();
    conversation.latest_queue_mutation_receipt = Some(PlanningQueueMutationReceipt {
        completed_turn_id: "turn-dense-tail".to_string(),
        planning_revision: 9,
        entries: vec![PlanningQueueMutationReceiptEntry {
            task_id: "queued-task".to_string(),
            task_title: "Keep the prompt visible".to_string(),
            mutation_kind: PlanningQueueMutationKind::Created,
            before_status: None,
            after_status: TaskStatus::Ready,
            after_updated_at: "2026-07-15T00:00:00Z".to_string(),
            unchanged_since_mutation: true,
        }],
    });

    let rendered = tui_testkit::render_shell_snapshot(&mut app, 48, 18);

    assert!(
        rendered.contains("> queued follow-up must remain visible"),
        "{rendered}"
    );
    assert!(
        rendered.contains("buffered prompt  |  Enter send"),
        "{rendered}"
    );
    let prompt_row = rendered
        .lines()
        .position(|line| line.contains("> queued follow-up must remain visible"))
        .expect("prompt row should be visible");
    assert!(
        prompt_row >= 8,
        "dense tail should reserve upper viewport rows:\n{rendered}"
    );
    let (action_row, action_line) = rendered
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("[ Undo queue ]"))
        .expect("visible undo action should have a rendered row");
    let action_column = action_line
        .trim_matches('"')
        .find("[ Undo queue ]")
        .expect("visible undo action should have a rendered column");
    let hit_area = app
        .queue_overlay_ui_state
        .receipt_undo_hit_area()
        .expect("visible undo action should retain its mouse target");
    assert_eq!(hit_area.x, action_column as u16);
    assert_eq!(hit_area.y, action_row as u16);
    assert_snapshot!("dense_hidden_tail_prompt_suffix", rendered);
}

#[test]
fn narrow_exit_confirmation_keeps_decision_keys_snapshot() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationShown);

    let rendered = tui_testkit::render_shell_snapshot(&mut app, 48, 18);

    assert!(rendered.contains("Akra / Confirm Exit"), "{rendered}");
    assert!(rendered.contains("Exit codex-exec-loop?"), "{rendered}");
    assert!(rendered.contains("y: exit    n: stay"), "{rendered}");
    assert_snapshot!("narrow_exit_confirmation", rendered);
}

#[test]
fn narrow_turn_steer_confirmation_keeps_exact_identity_prompt_and_keys() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.thread_id = "thread-steer-123456789".to_string();
    conversation.record_turn_started("turn-steer-123456789".to_string());
    conversation.composer.input_buffer =
        "Prioritize the exact queue cancellation regression before continuing.\n    cargo test --lib"
            .to_string();
    app.turn_steer_confirmation = Some(TurnSteerUiIntent {
        input_revision: 0,
        source_input_buffer: conversation.composer.input_buffer.clone(),
        request: ConversationTurnSteerRequest {
            thread_id: conversation.thread_id.clone(),
            expected_turn_id: conversation
                .active_turn_id
                .clone()
                .expect("running turn should have identity"),
            prompt: conversation.composer.input_buffer.clone(),
        },
    });

    let rendered = tui_testkit::render_shell_snapshot(&mut app, 48, 18);

    assert!(rendered.contains("Akra / Steer Active Turn"), "{rendered}");
    assert!(rendered.contains("thread: thread-steer..."), "{rendered}");
    assert!(rendered.contains("turn:"), "{rendered}");
    assert!(rendered.contains("turn-steer-1..."), "{rendered}");
    assert!(
        rendered.contains("Prioritize the exact queue"),
        "{rendered}"
    );
    assert!(rendered.contains("    cargo test --lib"), "{rendered}");
    assert!(rendered.contains("Enter/Tab: steer"), "{rendered}");
    assert_snapshot!("narrow_turn_steer_confirmation", rendered);
}

#[test]
fn captured_turn_steer_confirmation_keeps_language_and_exact_identity_after_app_mutation() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.tui_language = TuiLanguage::Korean;
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.thread_id = "thread-A".to_string();
    conversation.record_turn_started("turn-A".to_string());
    conversation.composer.input_buffer = "CAPTURED_STEER_PROMPT 한글".to_string();
    assert!(app.show_turn_steer_confirmation());

    let projection = InlineConversationFrameProjection::from_app(&app, 80);

    app.tui_language = TuiLanguage::English;
    let intent = app
        .turn_steer_confirmation
        .as_mut()
        .expect("captured turn-steer intent should remain visible");
    intent.request = ConversationTurnSteerRequest {
        thread_id: "thread-B".to_string(),
        expected_turn_id: "turn-B".to_string(),
        prompt: "MUTATED_STEER_PROMPT".to_string(),
    };

    let mut terminal = tui_testkit::shell_terminal(80, 24);
    terminal
        .draw(|frame| {
            draw_projected(
                frame,
                &mut app,
                ShellFrontendMode::InlineMainBuffer,
                projection,
            )
        })
        .expect("captured turn-steer projection should render");
    let rendered = tui_testkit::screen_text(&terminal);

    for expected in [
        "현재 턴에 전달",
        "이 초안을 현재 실행 중인 턴에 정확히 전달할까요?",
        "thread: thread-A  |  turn: turn-A",
        "CAPTURED_STEER_PROMPT 한글",
        "Enter/Tab: 전달",
    ] {
        assert!(rendered.contains(expected), "{rendered}");
    }
    for stale in [
        "Steer Active Turn",
        "Send this exact draft",
        "thread-B",
        "turn-B",
        "MUTATED_STEER_PROMPT",
    ] {
        assert!(!rendered.contains(stale), "{rendered}");
    }
}

#[test]
fn vt100_turn_steer_confirmation_hides_prompt_cursor_and_escape_restores_it() {
    let mut terminal =
        ratatui::Terminal::new(tui_testkit::Vt100Backend::new(48, 18)).expect("vt100 terminal");
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let draft = "keep this exact draft for the active turn".to_string();
    let (thread_id, turn_id) = {
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should start in a ready conversation state");
        };
        conversation.thread_id = "thread-cursor-steer".to_string();
        conversation.record_turn_started("turn-cursor-steer".to_string());
        conversation.composer.input_buffer = draft.clone();
        conversation
            .composer
            .set_input_cursor_byte_index("keep this exact ".len());
        (
            conversation.thread_id.clone(),
            conversation
                .active_turn_id
                .clone()
                .expect("running turn should have identity"),
        )
    };

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("baseline shell render succeeds");
    assert!(!terminal.backend().parser_cursor_hidden());
    let expected_cursor = terminal.backend().parser_cursor_position();

    app.turn_steer_confirmation = Some(TurnSteerUiIntent {
        input_revision: 0,
        source_input_buffer: draft.clone(),
        request: ConversationTurnSteerRequest {
            thread_id,
            expected_turn_id: turn_id,
            prompt: draft.clone(),
        },
    });
    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("turn steer confirmation render succeeds");

    assert!(terminal.backend().parser_cursor_hidden());
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("test app should keep a ready conversation state");
    };
    assert_eq!(conversation.composer.input_buffer, draft);

    assert!(app.handle_turn_steer_confirmation_key(event::KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("restored shell render succeeds");

    assert!(!terminal.backend().parser_cursor_hidden());
    assert_eq!(terminal.backend().parser_cursor_position(), expected_cursor);
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("test app should keep a ready conversation state");
    };
    assert_eq!(conversation.composer.input_buffer, draft);
}

#[test]
fn vt100_exit_confirmation_hides_prompt_cursor_and_cancel_restores_it() {
    let mut terminal =
        ratatui::Terminal::new(tui_testkit::Vt100Backend::new(48, 18)).expect("vt100 terminal");
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let draft = "cancel exit and keep this draft".to_string();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.composer.input_buffer = draft.clone();
    conversation
        .composer
        .set_input_cursor_byte_index("cancel exit ".len());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("baseline shell render succeeds");
    assert!(!terminal.backend().parser_cursor_hidden());
    let expected_cursor = terminal.backend().parser_cursor_position();

    app.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationShown);
    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("exit confirmation render succeeds");

    assert!(terminal.backend().parser_cursor_hidden());
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("test app should keep a ready conversation state");
    };
    assert_eq!(conversation.composer.input_buffer, draft);

    assert_eq!(
        app.handle_exit_confirmation_key(event::KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
        )),
        Some(false)
    );
    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("restored shell render succeeds");

    assert!(!terminal.backend().parser_cursor_hidden());
    assert_eq!(terminal.backend().parser_cursor_position(), expected_cursor);
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("test app should keep a ready conversation state");
    };
    assert_eq!(conversation.composer.input_buffer, draft);
}

#[test]
fn steer_prompt_preview_preserves_lines_and_marks_bounded_omissions() {
    let (preview, truncated) = steer_prompt_preview("first\n    indented\nlast", 64, 6);
    assert_eq!(preview, vec!["first", "    indented", "last"]);
    assert!(!truncated);

    let (preview, truncated) = steer_prompt_preview("first\nsecond\nthird", 64, 2);
    assert_eq!(preview, vec!["first", "second"]);
    assert!(truncated);
}

#[test]
fn queue_receipt_renders_clickable_undo_action_in_conversation_tail() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.thread_id = "thread-mouse-undo".to_string();
    conversation.title = "Mouse undo".to_string();
    conversation.record_turn_started("turn-active-mouse-undo".to_string());
    conversation.latest_queue_mutation_receipt = Some(PlanningQueueMutationReceipt {
        completed_turn_id: "turn-mouse-undo".to_string(),
        planning_revision: 7,
        entries: vec![PlanningQueueMutationReceiptEntry {
            task_id: "queued-task".to_string(),
            task_title: "Undo from the conversation tail".to_string(),
            mutation_kind: PlanningQueueMutationKind::Created,
            before_status: None,
            after_status: TaskStatus::Ready,
            after_updated_at: "2026-07-15T00:00:00Z".to_string(),
            unchanged_since_mutation: true,
        }],
    });

    let rendered = tui_testkit::render_inline_snapshot(&mut app, 96, 24);

    assert!(rendered.contains("[ Undo queue ]"), "{rendered}");
    assert!(rendered.contains("click to cancel 1 queued task"));
    let hit_area = app
        .queue_overlay_ui_state
        .receipt_undo_hit_area()
        .expect("visible queue undo action should own a mouse target");
    assert_eq!(hit_area.width, "[ Undo queue ]".len() as u16);
    assert_eq!(hit_area.height, 1);
    assert!(app.queue_receipt_undo_mouse_capture_requested());

    app.shell_overlay = ShellOverlay::Queue;
    let overlay = tui_testkit::render_shell_snapshot(&mut app, 96, 24);

    assert!(!overlay.contains("[ Undo queue ]"), "{overlay}");
    assert!(app.queue_overlay_ui_state.receipt_undo_hit_area().is_none());
    assert!(!app.queue_receipt_undo_mouse_capture_requested());
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
    app.sync_ready_conversation_planning_runtime_projection(
        sample_planning_runtime_projection("Planning Context\nQueue Summary", "Queue Summary")
            .with_planning_revision(Some(1)),
    );
    app.shell_overlay = ShellOverlay::Queue;
    app.bind_queue_overlay_authority_for_test(
        1,
        std::collections::BTreeMap::from([
            (
                "task-1".to_string(),
                queue_overlay_ui::QueueOverlayAuthorityToken {
                    status: TaskStatus::Ready,
                    updated_at: "2026-04-10T00:00:00Z".to_string(),
                },
            ),
            (
                "task-2".to_string(),
                queue_overlay_ui::QueueOverlayAuthorityToken {
                    status: TaskStatus::Ready,
                    updated_at: "2026-04-10T01:00:00Z".to_string(),
                },
            ),
        ]),
    );

    let rendered = tui_testkit::render_shell_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("queued: 2"));
    assert!(!rendered.contains("revision:"));
    assert!(!rendered.contains("next:"));
    assert!(!rendered.contains("No promotable proposals"));
    assert!(!rendered.contains("┌"));
    assert_snapshot!("queue_overlay", rendered);

    let narrow = tui_testkit::render_shell_snapshot(&mut app, 48, 18);
    assert!(narrow.contains("> #1 [ready]"), "{narrow}");
    assert!(
        narrow.contains("Implement shell planning status"),
        "{narrow}"
    );
    assert!(narrow.contains("Up/Down, j/k: select"), "{narrow}");
    assert!(narrow.contains("x/Delete: remove"), "{narrow}");
    assert!(narrow.contains("Esc/Ctrl+C: close"), "{narrow}");
    assert!(!narrow.contains("Proposals"), "{narrow}");
    let narrow_lines = narrow.lines().collect::<Vec<_>>();
    let title_line = narrow_lines
        .iter()
        .position(|line| line.contains("Planning Queue"))
        .expect("queue title should be visible");
    let summary_line = narrow_lines
        .iter()
        .position(|line| line.contains("Summary"))
        .expect("queue summary should be visible");
    assert_eq!(summary_line, title_line + 1, "{narrow}");

    assert!(
        app.handle_queue_overlay_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('x'),
            crossterm::event::KeyModifiers::NONE,
        ))
    );
    assert_eq!(app.pending_queue_mutation_operation_id(), None);
    let armed = tui_testkit::render_shell_snapshot(&mut app, 48, 18);
    assert!(armed.contains("> #1 [ready]"), "{armed}");
    assert!(armed.contains("remove task-1?"), "{armed}");
    assert!(armed.contains("Enter/x/Delete: confirm remove"), "{armed}");
    assert!(!armed.contains("x/Delete: remove |"), "{armed}");

    app.tui_language = TuiLanguage::Korean;
    let korean = tui_testkit::render_shell_snapshot(&mut app, 48, 18);
    assert!(korean.contains("> #1 [ready]"), "{korean}");
    assert!(korean.contains("task-1 제거할까요?"), "{korean}");
    assert!(korean.contains("Enter/x/Delete: 제거 확인"), "{korean}");
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
    let authority_tokens = app
        .queue_action_tasks()
        .into_iter()
        .map(|task| {
            (
                task.task_id,
                queue_overlay_ui::QueueOverlayAuthorityToken {
                    status: task.status,
                    updated_at: "2026-07-15T00:00:09Z".to_string(),
                },
            )
        })
        .collect();
    app.bind_queue_overlay_authority_for_test(9, authority_tokens);
    let task_ids = app
        .queue_action_tasks()
        .into_iter()
        .map(|task| task.task_id)
        .collect::<Vec<_>>();

    app.queue_overlay_ui_state.move_selection(&task_ids, 3);
    let hidden = tui_testkit::render_shell_snapshot(&mut app, 80, 16);
    assert!(hidden.contains("> #4 [ready]"));
    assert!(hidden.contains("Deep queue task 4"));
    assert!(
        app.handle_queue_overlay_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('x'),
            crossterm::event::KeyModifiers::NONE,
        ))
    );
    let armed_active = tui_testkit::render_shell_snapshot(&mut app, 48, 16);
    assert!(armed_active.contains("> #4 [ready]"), "{armed_active}");
    assert!(armed_active.contains("remove task-4?"), "{armed_active}");
    assert!(
        armed_active.contains("Enter/x/Delete: confirm remove"),
        "{armed_active}"
    );

    app.queue_overlay_ui_state.move_selection(&task_ids, 1);
    let proposal = tui_testkit::render_shell_snapshot(&mut app, 80, 16);
    assert!(proposal.contains("> #1 [proposed]"));
    assert!(proposal.contains("Deep proposal selection"));
    assert!(
        app.handle_queue_overlay_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('x'),
            crossterm::event::KeyModifiers::NONE,
        ))
    );
    let armed_proposal = tui_testkit::render_shell_snapshot(&mut app, 48, 16);
    assert!(
        armed_proposal.contains("> #1 [proposed]"),
        "{armed_proposal}"
    );
    assert!(
        armed_proposal.contains("remove proposal-deep?"),
        "{armed_proposal}"
    );
    assert!(
        armed_proposal.contains("Enter/x/Delete: confirm remove"),
        "{armed_proposal}"
    );

    app.queue_overlay_ui_state.move_selection(&task_ids, 1);
    let skipped = tui_testkit::render_shell_snapshot(&mut app, 80, 16);
    assert!(skipped.contains("> [ready / skipped]"));
    assert!(skipped.contains("Deep skipped selection"));

    let narrow_skipped = tui_testkit::render_shell_snapshot(&mut app, 48, 16);
    assert!(narrow_skipped.contains("> [ready / skipped]"));

    assert!(
        app.handle_queue_overlay_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('x'),
            crossterm::event::KeyModifiers::NONE,
        ))
    );
    let armed_skipped = tui_testkit::render_shell_snapshot(&mut app, 48, 16);
    assert!(
        armed_skipped.contains("remove skipped-deep?"),
        "{armed_skipped}"
    );
    assert!(
        armed_skipped.contains("> [ready / skipped]"),
        "{armed_skipped}"
    );
    assert!(
        armed_skipped.contains("Enter/x/Delete: confirm remove"),
        "{armed_skipped}"
    );
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
            "notice: activity: active:command | diff:+1 -1 h1 | cmd:2 lines | ctx:75.00% | model:gpt-5.5 | task:P0-D3 rail"
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
    assert!(
        narrow.contains("notice: activity: active:command | diff:+1 -1 h1"),
        "{narrow}"
    );
    assert!(!narrow.contains("cmd:"));
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
fn compact_operator_tail_prioritizes_approval_terminal_and_live_activity() {
    let mut activity_app = make_test_app();
    activity_app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    activity_app.show_startup_ascii_art = false;
    tui_testkit::set_progressive_command_activity(&mut activity_app, "first\nsecond", false);
    assert!(activity_app.show_progressive_activity_overlay(ProgressiveActivityDetailKind::Diff));

    let activity = tui_testkit::render_inline_snapshot(&mut activity_app, 48, 10);
    assert!(
        activity.contains("notice: activity: active:command"),
        "{activity}"
    );
    assert!(!activity.contains("status: turn started"), "{activity}");
    assert!(!activity.contains("prompt: turn running"), "{activity}");

    let mut terminal_app = make_test_app();
    terminal_app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    terminal_app.show_startup_ascii_art = false;
    let ConversationState::Ready(conversation) = &mut terminal_app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.fail_turn("runtime failed".to_string());
    terminal_app.shell_overlay = ShellOverlay::Activity;

    let terminal = tui_testkit::render_inline_snapshot(&mut terminal_app, 48, 10);
    assert!(
        terminal.contains("notice: activity: terminal:runtime-failed"),
        "{terminal}"
    );

    let mut approval_app = make_test_app();
    approval_app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut approval_app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.pending_approval_request = Some(ConversationApprovalRequest {
        approval_id: "approval-compact".to_string(),
        server_request_id: "server-compact".to_string(),
        method: "item/commandExecution/requestApproval".to_string(),
        kind: ConversationApprovalRequestKind::CommandExecution,
        summary: "Review compact approval".to_string(),
        details: vec!["Command: cargo test".to_string()],
    });
    approval_app.shell_overlay = ShellOverlay::Approval;

    let approval = tui_testkit::render_inline_snapshot(&mut approval_app, 48, 10);
    assert!(approval.contains("Approval Required"), "{approval}");
    assert!(approval.contains("Y: approve once"), "{approval}");
    assert!(approval.contains("N / Esc: decline"), "{approval}");
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
            "notice: activity: active:command | diff:+1 -1 h1 | cmd:2 lines | ctx:75.00% | model:gpt-5.5 | task:P0-D3 rail"
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
    assert!(wide.contains("filter:"));
    assert!(wide.contains("> Diff") || wide.contains("diff"));
    assert!(wide.contains("◆"));
    assert!(wide.contains("history:incomplete"));
    assert!(wide.contains("1 -old value"), "{wide}");
    assert!(wide.contains(&format!("{secret}\\x1b[31m")), "{wide}");
    assert!(!wide.contains("@@ -1 +1 @@"), "{wide}");
    assert!(!wide.contains('\u{1b}'));
    assert!(wide.contains("prompt:"));
    assert_eq!(std::sync::Arc::strong_count(&core_snapshot), 1);
    assert_snapshot!("inline_progressive_activity_inspector_wide", wide);

    let narrow = tui_testkit::render_inline_snapshot(&mut app, 48, 10);
    assert!(narrow.contains("Activity / inline inspection"));
    assert!(narrow.contains("filter:") || narrow.contains("diff"));
    assert!(
        narrow.contains("Up/Down: card")
            || narrow.contains("PgUp/PgDn: page")
            || narrow.contains("Tab: filter"),
        "{narrow}"
    );
    assert!(narrow.contains("notice: activity:"), "{narrow}");
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
    assert!(vt100.contains("filter:") || vt100.contains("command") || vt100.contains("> Output"));
    assert!(vt100.contains("◆") || vt100.contains("command"));
    assert!(!vt100.contains("Full Output"));
    assert!(!vt100.contains('\u{1b}'));
    assert_eq!(std::sync::Arc::strong_count(&core_snapshot), 1);
    assert_snapshot!("vt100_progressive_activity_inspector_output", vt100);
}

#[test]
fn activity_inspector_clamps_selection_before_first_frame_document_projection() {
    const SURVIVING_DETAIL: &str = "SURVIVING_CARD_DOCUMENT_BODY";
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.show_startup_ascii_art = false;
    let previous_snapshot = tui_testkit::set_progressive_command_activity(
        &mut app,
        &format!("surviving card title\n{SURVIVING_DETAIL}"),
        false,
    );
    assert!(app.show_progressive_activity_overlay_all());
    assert!(
        app.progressive_activity_overlay_ui_state
            .move_card_selection(2, 3)
    );
    assert_eq!(
        app.progressive_activity_overlay_ui_state
            .selected_card_index(),
        2
    );

    let mut next_snapshot = previous_snapshot.as_ref().clone();
    next_snapshot.records.truncate(1);
    next_snapshot.last_sequence = next_snapshot
        .records
        .last()
        .map(|record| record.last_sequence());
    next_snapshot.source_observation_count = next_snapshot
        .records
        .iter()
        .map(|record| record.observation_count())
        .sum();
    let next_snapshot = std::sync::Arc::new(next_snapshot);
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should keep a ready conversation state");
    };
    conversation.progressive_activity_detail.reset();
    conversation
        .progressive_activity_detail
        .replace_snapshot(&next_snapshot);

    let first_frame = tui_testkit::render_inline_snapshot(&mut app, 80, 24);

    assert_eq!(
        app.progressive_activity_overlay_ui_state
            .selected_card_index(),
        0
    );
    assert!(first_frame.contains(SURVIVING_DETAIL), "{first_frame}");
}

#[test]
fn narrow_activity_inspector_omits_normal_metadata_and_keeps_exact_keys() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.show_startup_ascii_art = false;
    let _core_snapshot =
        tui_testkit::set_progressive_command_activity(&mut app, "complete detail", false);
    assert!(app.show_progressive_activity_overlay(ProgressiveActivityDetailKind::Diff));

    let rendered = tui_testkit::render_inline_snapshot(&mut app, 48, 18);

    assert!(rendered.contains("complete detail"), "{rendered}");
    assert!(
        rendered.contains("Tab: filter")
            || rendered.contains("Up/Down: card")
            || rendered.contains("PgUp/PgDn: page"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Home: first") || rendered.contains("Esc: close"),
        "{rendered}"
    );
    assert!(!rendered.contains("truncated:0"), "{rendered}");
    assert!(!rendered.contains("history:complete"), "{rendered}");
    assert!(!rendered.contains("sequence:"), "{rendered}");
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
    assert!(
        rendered.contains("| 0-") || rendered.contains("diff ·"),
        "{rendered}"
    );
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
     * Markdown code fence와 info string은 렌더링 문법이므로 숨기고 code line만 보여야 한다.
     * VT100 output에서도 fence가 다시 노출되지 않는지 확인해 transcript projection 계약을 고정한다.
     */
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    tui_testkit::set_live_agent_message(&mut app, "```rust\nlet ok = true;\n```");

    let rendered = tui_testkit::render_inline_vt100_snapshot(&mut app, 96, 32);

    assert_snapshot!("vt100_markdown_code_block_shell", rendered);
    assert!(rendered.contains("let ok = true;"));
    assert!(!rendered.contains("```"));
    assert!(!rendered.contains("rust"));
}

#[test]
fn vt100_queue_overlay_matches_snapshot() {
    /*
     * Queue overlay의 vt100 path는 compact planning rows와 action keys가 terminal backend에서도
     * 보존되는지 확인한다. TestBackend snapshot만 통과하면 ANSI write/clear/resize path의 section
     * loss를 놓칠 수 있어 selected task와 key guide를 실제 terminal output에서도 고정한다.
     */
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.sync_ready_conversation_planning_runtime_projection(
        sample_planning_runtime_projection("Planning Context\nQueue Summary", "Queue Summary")
            .with_planning_revision(Some(1)),
    );
    app.shell_overlay = ShellOverlay::Queue;
    app.bind_queue_overlay_authority_for_test(
        1,
        std::collections::BTreeMap::from([
            (
                "task-1".to_string(),
                queue_overlay_ui::QueueOverlayAuthorityToken {
                    status: TaskStatus::Ready,
                    updated_at: "2026-04-10T00:00:00Z".to_string(),
                },
            ),
            (
                "task-2".to_string(),
                queue_overlay_ui::QueueOverlayAuthorityToken {
                    status: TaskStatus::Ready,
                    updated_at: "2026-04-10T01:00:00Z".to_string(),
                },
            ),
        ]),
    );

    let rendered = tui_testkit::render_shell_vt100_snapshot(&mut app, 96, 28);

    assert!(rendered.contains("> #1 [ready] Implement shell planning status"));
    assert!(!rendered.contains("Proposals"));
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
    assert!(rendered.contains("Enter queue"));
    assert!(rendered.contains("Tab steer"));
    assert!(rendered.contains("Ctrl+j nl"));
    assert!(!rendered.contains("prompt: turn running"));
    assert!(rendered.lines().all(|line| line.chars().count() <= 48));
}

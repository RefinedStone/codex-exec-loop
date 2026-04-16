use std::thread;

use crossterm::event::Event;

use super::super::shell_runtime;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning_contract::DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH;

use super::{
    ConversationInputState, ConversationMessage, ConversationMessageKind,
    ConversationRuntimeEvent, ConversationState, KeyCode, KeyEvent, KeyModifiers,
    PlanningExecutionSnapshot, PlanningRuntimeSnapshot, ShellActionAvailability, ShellOverlay,
    StartupState, TASK_LEDGER_FILE_PATH, build_automation_overlay_view,
    build_automation_preview_lines, build_automation_status_lines, build_ready_input_lines,
    create_temp_workspace, make_test_app, ready_conversation, ready_turn_planning_capture,
    sample_planning_runtime_snapshot, sample_startup_diagnostics,
};

#[test]
fn stale_planning_capture_blocks_reconciliation_for_other_workspace() {
    let (mut app, codex_port) = make_test_app();
    let current_workspace = create_temp_workspace("planning-capture-current");
    let stale_workspace = create_temp_workspace("planning-capture-stale");
    let mut conversation = ready_conversation();
    conversation.cwd = current_workspace.clone();
    conversation.active_turn_workspace_directory = Some(current_workspace.clone());
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-mismatch".to_string());
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::ready(conversation);
    app.active_turn_planning_capture = Some(ready_turn_planning_capture(
        &stale_workspace,
        PlanningExecutionSnapshot {
            directions_toml: Some("version = 1".to_string()),
            task_ledger_json: Some("{\"version\":1,\"tasks\":[]}".to_string()),
            task_ledger_schema_json: None,
            result_output_markdown: None,
            queue_snapshot_json: None,
        },
    ));

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-mismatch".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ));

    assert!(
        codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .is_empty()
    );
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(conversation.planning_runtime_snapshot.preview_status_label(), "blocked");
    assert!(
        conversation
            .planning_runtime_snapshot
            .preview_detail()
            .is_some_and(|detail| detail.contains("stale planning snapshot"))
    );
    assert!(conversation.runtime_notices.iter().any(|notice| {
        notice.contains(&stale_workspace) && notice.contains(&current_workspace)
    }));

    std::fs::remove_dir_all(current_workspace).expect("current workspace should be removed");
    std::fs::remove_dir_all(stale_workspace).expect("stale workspace should be removed");
}

#[test]
fn stream_worker_forces_failure_when_service_exits_without_terminal_event() {
    let (app, codex_port) = make_test_app();
    let mut runtime = shell_runtime::ShellRuntime::new(app);
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("conversation should start ready");
    };
    conversation.thread_id = "thread-123".to_string();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context\nQueue Summary",
        "next task: task-1",
    ));
    codex_port
        .turn_stream_behavior
        .lock()
        .expect("turn stream behavior mutex poisoned")
        .error = Some("transport closed".to_string());

    runtime
        .app_mut()
        .submit_prompt("ship it".to_string(), super::PromptOrigin::Manual);

    for _ in 0..20 {
        thread::sleep(super::Duration::from_millis(5));
        runtime.poll_background_messages();
        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            continue;
        };
        if conversation.input_state == ConversationInputState::ReadyToContinue
            && conversation.status_text == "turn failed"
        {
            break;
        }
    }

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(conversation.input_state, ConversationInputState::ReadyToContinue);
    assert_eq!(conversation.status_text, "turn failed");
    assert!(conversation.messages.iter().any(|message| {
        message.kind == ConversationMessageKind::Status
            && message
                .text
                .contains("turn stream failed before a terminal event: transport closed")
    }));
}

#[test]
fn inline_shell_command_buffer_shows_automation_hint() {
    let mut conversation = ready_conversation();
    conversation.input_buffer = ":auto".to_string();
    conversation.sync_inline_shell_command_palette();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("> :auto"));
    assert!(rendered.contains("automation controls"));
}

#[test]
fn automation_inline_command_opens_overlay() {
    let (app, _) = make_test_app();
    let mut runtime = shell_runtime::ShellRuntime::new(app);
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    runtime.app_mut().push_input_character(':');
    runtime.app_mut().push_input_character('a');
    runtime.app_mut().push_input_character('u');
    runtime.app_mut().push_input_character('t');
    runtime.app_mut().push_input_character('o');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Automation);
    assert!(conversation.input_buffer.is_empty());
    assert!(conversation.status_text.contains("opened automation controls"));
}

#[test]
fn automation_overlay_view_surfaces_preview_status_and_keys() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context\nQueue Summary",
        "next task: task-1",
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.show_automation_overlay();

    let view = build_automation_overlay_view(&app);
    let header = view
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let list = view
        .list_view
        .message_lines
        .expect("automation overlay should expose static list messaging")
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let preview = view
        .preview_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let status = view
        .status_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let keys = view
        .key_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(header.contains("Automation Controls"));
    assert!(list.contains("automation follows the planning queue only"));
    assert!(preview.contains("mode: planning queue"));
    assert!(preview.contains("Rendered Preview"));
    assert!(status.contains("automation: on"));
    assert!(keys.contains("Ctrl+a: automation on/off"));
}

#[test]
fn automation_preview_uses_placeholder_without_agent_reply() {
    let (app, _) = make_test_app();

    let rendered = build_automation_preview_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("preview last_message: placeholder until an agent reply exists"));
    assert!(rendered.contains("preview thread id: draft-thread"));
}

#[test]
fn automation_preview_surfaces_queue_refresh_copy_when_queue_is_idle() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.replace_planning_runtime_snapshot(
        PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            None,
        )
        .with_queue_idle_policy(
            crate::domain::planning::QueueIdlePolicy::ReviewAndEnqueue,
            Some(DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string()),
        ),
    );

    let rendered = build_automation_preview_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("planning priority queue를 갱신하세요."));
    assert!(rendered.contains("planning: ready"));
}

#[test]
fn automation_status_lines_include_runtime_and_warning_summary() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.status_text = "turn completed / queued auto follow-up with mode planning queue"
        .to_string();
    conversation.base_warnings =
        vec!["planner queue changed shape after reconciliation".to_string()];
    conversation.warnings = conversation.base_warnings.clone();
    conversation.runtime_notices = vec!["planning reconciliation completed".to_string()];

    let rendered = build_automation_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("automation: on"));
    assert!(rendered.contains("status: turn completed / queued auto follow-up with mode planning queue"));
    assert!(rendered.contains("warning: planner queue changed shape"));
    assert!(rendered.contains("planning reconciliation completed"));
}

#[test]
fn ctrl_l_starts_max_auto_turns_editing_inside_automation_overlay() {
    let (app, _) = make_test_app();
    let mut runtime = shell_runtime::ShellRuntime::new(app);
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('l'),
        KeyModifiers::CONTROL,
    )));

    assert!(runtime.app().is_max_auto_turns_editing());
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Automation);
}

#[test]
fn automation_overlay_scroll_keys_update_preview_offset() {
    let (mut app, _) = make_test_app();
    app.show_automation_overlay();
    assert_eq!(app.followup_overlay_ui_state.preview_scroll, 0);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(
        KeyCode::PageDown,
        KeyModifiers::NONE,
    )));
    assert!(app.followup_overlay_ui_state.preview_scroll > 0);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
}

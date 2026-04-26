use std::thread;
use std::time::Duration;

use crossterm::event::Event;

use super::super::shell_runtime;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModeReadinessSnapshot, ParallelModeReadinessState,
};

use super::{
    ConversationInputState, ConversationMessage, ConversationMessageKind, ConversationRuntimeEvent,
    ConversationState, KeyCode, KeyEvent, KeyModifiers, PlanningExecutionSnapshot, ShellOverlay,
    StartupState, TASK_LEDGER_FILE_PATH, create_temp_workspace, make_test_app, ready_conversation,
    ready_turn_planning_capture, sample_planning_runtime_snapshot, sample_startup_diagnostics,
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
    assert_eq!(
        conversation
            .planning_runtime_snapshot
            .preview_status_label(),
        "blocked"
    );
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
        thread::sleep(Duration::from_millis(5));
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
    assert_eq!(
        conversation.input_state,
        ConversationInputState::ReadyToContinue
    );
    assert_eq!(conversation.status_text, "turn failed");
    assert!(conversation.messages.iter().any(|message| {
        message.kind == ConversationMessageKind::Status
            && message
                .text
                .contains("turn stream failed before a terminal event: transport closed")
    }));
}

#[test]
fn queue_inline_command_opens_overlay_from_palette_selection() {
    let (app, _) = make_test_app();
    let mut runtime = shell_runtime::ShellRuntime::new(app);
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    runtime.app_mut().push_input_character(':');
    runtime.app_mut().push_input_character('q');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Queue);
    assert!(conversation.input_buffer.is_empty());
    assert!(
        conversation
            .status_text
            .contains("opened planning queue inspection")
    );
}

#[test]
fn help_inline_command_opens_command_help_overlay() {
    let (app, _) = make_test_app();
    let mut runtime = shell_runtime::ShellRuntime::new(app);
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    for character in ":help".chars() {
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
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Help);
    assert!(conversation.input_buffer.is_empty());
    assert!(
        conversation
            .status_text
            .contains("opened shell command help")
    );
}

#[test]
fn parallel_inline_command_opens_supersession_overlay() {
    let (app, _) = make_test_app();
    let mut runtime = shell_runtime::ShellRuntime::new(app);
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    runtime.app_mut().push_input_character(':');
    runtime.app_mut().push_input_character('p');
    runtime.app_mut().push_input_character('a');
    runtime.app_mut().push_input_character('r');
    runtime.app_mut().push_input_character('a');
    runtime.app_mut().push_input_character('l');
    runtime.app_mut().push_input_character('l');
    runtime.app_mut().push_input_character('e');
    runtime.app_mut().push_input_character('l');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(conversation.input_buffer.is_empty());
    assert!(conversation.status_text.contains("parallel mode:"));
}

#[test]
fn sessions_command_routes_to_supersession_when_parallel_mode_is_enabled() {
    let (app, _) = make_test_app();
    let mut runtime = shell_runtime::ShellRuntime::new(app);
    runtime.app_mut().parallel_mode_enabled = true;
    runtime.app_mut().parallel_mode_readiness_snapshot = Some(sample_parallel_mode_snapshot(
        ParallelModeReadinessState::Ready,
    ));
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    runtime.app_mut().push_input_character(':');
    runtime.app_mut().push_input_character('s');
    runtime.app_mut().push_input_character('e');
    runtime.app_mut().push_input_character('s');
    runtime.app_mut().push_input_character('s');
    runtime.app_mut().push_input_character('i');
    runtime.app_mut().push_input_character('o');
    runtime.app_mut().push_input_character('n');
    runtime.app_mut().push_input_character('s');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(
        conversation
            .status_text
            .contains("opened supersession control tower")
    );
}

fn sample_parallel_mode_snapshot(
    readiness: ParallelModeReadinessState,
) -> ParallelModeReadinessSnapshot {
    ParallelModeReadinessSnapshot::new(
        "/tmp/root",
        readiness,
        vec![
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GitRepository,
                ParallelModeCapabilityState::Ready,
                "git repo detected at /tmp/root",
                None,
            ),
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::Planning,
                ParallelModeCapabilityState::Ready,
                "planning workspace is healthy",
                None,
            ),
        ],
        None,
    )
}

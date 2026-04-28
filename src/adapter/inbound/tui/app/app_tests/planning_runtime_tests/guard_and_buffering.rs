use std::thread;
use std::time::Duration;

use super::super::{
    ConversationInputState, ConversationMessage, ConversationMessageKind, ConversationRuntimeEvent,
    ConversationState, ConversationStreamEvent, PlanningBootstrapMode, PlanningBootstrapService,
    PlanningExecutionSnapshot, PlanningRepairRequest, PlanningRepairState, StartupState,
    TASK_LEDGER_FILE_PATH, bootstrap_active_planning_workspace, create_temp_workspace,
    failed_turn_planning_capture, make_test_app, ready_conversation, ready_turn_planning_capture,
    sample_startup_diagnostics,
};

#[test]
fn stale_planning_repair_state_does_not_queue_visible_retry() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let mut conversation = ready_conversation();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-repair-1".to_string());
    conversation.planning_repair_state = Some(PlanningRepairState {
        root_turn_id: "turn-root".to_string(),
        attempts_used: 1,
        max_attempts: 2,
        latest_request: PlanningRepairRequest {
            failure_summary: "failed to parse task-ledger.json".to_string(),
            validation_errors: vec!["failed to parse task-ledger.json".to_string()],
            directions_toml: "version = 1".to_string(),
            task_ledger_schema_json: "{\"type\":\"object\"}".to_string(),
            accepted_task_ledger_json: "{\"version\":1,\"tasks\":[]}".to_string(),
            rejected_task_ledger_json: Some("{ invalid json".to_string()),
            rejected_archive_path: Some(
                "/tmp/workspace/.codex-exec-loop/planning/rejected/turn-root/task-ledger.json"
                    .to_string(),
            ),
        },
    });
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-repair-1".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    let mut turn_prompts = Vec::new();
    for _ in 0..20 {
        turn_prompts = codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .collect::<Vec<_>>();
        if !turn_prompts.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };

    assert!(
        turn_prompts
            .iter()
            .all(|prompt| !prompt.contains("planning repair"))
    );
    assert!(
        codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .is_empty()
    );
    assert!(conversation.planning_repair_state.is_none());
}

#[test]
fn stale_repair_state_is_cleared_after_read_only_task_ledger_restore() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-repair-still-invalid");
    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    bootstrap_active_planning_workspace(&workspace_dir);
    let bootstrap_artifacts =
        PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);
    std::fs::write(planning_dir.join("task-ledger.json"), "{ invalid json")
        .expect("invalid task ledger should write");

    let mut conversation = ready_conversation();
    conversation.cwd = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-repair-2".to_string());
    conversation.planning_repair_state = Some(PlanningRepairState {
        root_turn_id: "turn-root".to_string(),
        attempts_used: 1,
        max_attempts: 2,
        latest_request: PlanningRepairRequest {
            failure_summary: "failed to parse task-ledger.json".to_string(),
            validation_errors: vec!["failed to parse task-ledger.json".to_string()],
            directions_toml: bootstrap_artifacts.directions_toml.clone(),
            task_ledger_schema_json: bootstrap_artifacts.task_ledger_schema_json.clone(),
            accepted_task_ledger_json: bootstrap_artifacts.task_ledger_json.clone(),
            rejected_task_ledger_json: Some("{ invalid json".to_string()),
            rejected_archive_path: None,
        },
    });
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::ready(conversation);
    app.active_turn_planning_capture = Some(ready_turn_planning_capture(
        &workspace_dir,
        PlanningExecutionSnapshot {
            directions_toml: Some(bootstrap_artifacts.directions_toml.clone()),
            task_ledger_json: Some(bootstrap_artifacts.task_ledger_json.clone()),
            task_ledger_schema_json: Some(bootstrap_artifacts.task_ledger_schema_json.clone()),
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown.clone()),
            queue_snapshot_json: None,
        },
    ));

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-repair-2".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ));

    let mut repair_prompt = None;
    for _ in 0..20 {
        repair_prompt = codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .find(|prompt| prompt.contains("planning repair 1/2"));
        if repair_prompt.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert!(repair_prompt.is_none());

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn buffered_manual_input_survives_read_only_task_ledger_restore() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-repair-manual-buffer");
    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    bootstrap_active_planning_workspace(&workspace_dir);
    let bootstrap_artifacts =
        PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);
    std::fs::write(planning_dir.join("task-ledger.json"), "{ invalid json")
        .expect("invalid task ledger should write");

    let mut conversation = ready_conversation();
    conversation.cwd = workspace_dir.clone();
    conversation.input_buffer = "operator override draft".to_string();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-repair-3".to_string());
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::ready(conversation);
    app.active_turn_planning_capture = Some(ready_turn_planning_capture(
        &workspace_dir,
        PlanningExecutionSnapshot {
            directions_toml: Some(bootstrap_artifacts.directions_toml.clone()),
            task_ledger_json: Some(bootstrap_artifacts.task_ledger_json.clone()),
            task_ledger_schema_json: Some(bootstrap_artifacts.task_ledger_schema_json.clone()),
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown.clone()),
            queue_snapshot_json: None,
        },
    ));

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-repair-3".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ));

    let hidden_prompts = codex_port
        .new_thread_calls
        .lock()
        .expect("new-thread call mutex poisoned")
        .iter()
        .map(|(_, prompt)| prompt.clone())
        .collect::<Vec<_>>();
    assert!(
        hidden_prompts
            .iter()
            .all(|prompt| !prompt.contains("planning repair"))
    );
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(conversation.input_buffer, "operator override draft");
    assert!(conversation.planning_repair_state.is_none());
    assert!(!conversation.status_text.contains("manual input buffered"));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn internal_continuation_pause_stops_hidden_planning_repair_and_followup() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("internal-pause-no-hidden-repair");
    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    std::fs::create_dir_all(&planning_dir).expect("planning directory should be created");
    let bootstrap_artifacts =
        PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
    std::fs::write(
        planning_dir.join("directions.toml"),
        &bootstrap_artifacts.directions_toml,
    )
    .expect("directions should write");
    std::fs::write(planning_dir.join("task-ledger.json"), "{ invalid json")
        .expect("invalid task ledger should write");
    std::fs::write(
        planning_dir.join("task-ledger.schema.json"),
        &bootstrap_artifacts.task_ledger_schema_json,
    )
    .expect("schema should write");
    std::fs::write(
        planning_dir.join("result-output.md"),
        &bootstrap_artifacts.result_output_markdown,
    )
    .expect("result output should write");

    let mut conversation = ready_conversation();
    conversation
        .auto_follow_state
        .pause_post_turn_continuation();
    conversation.cwd = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-repair-4".to_string());
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::ready(conversation);
    app.active_turn_planning_capture = Some(ready_turn_planning_capture(
        &workspace_dir,
        PlanningExecutionSnapshot {
            directions_toml: Some(bootstrap_artifacts.directions_toml.clone()),
            task_ledger_json: Some(bootstrap_artifacts.task_ledger_json.clone()),
            task_ledger_schema_json: Some(bootstrap_artifacts.task_ledger_schema_json.clone()),
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown.clone()),
            queue_snapshot_json: None,
        },
    ));

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-repair-4".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ));

    assert!(
        codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .is_empty()
    );
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
        conversation.status_text,
        "turn completed / internal continuation paused"
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn buffered_queue_command_stays_available_while_auto_followup_submits() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("queue-command-followup");
    bootstrap_active_planning_workspace(&workspace_dir);
    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    std::fs::write(
        planning_dir.join("task-ledger.json"),
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-queue-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Current next task for the queue command regression.",
      "title": "Convert kimchi lecture notes into table format",
      "description": "Turn the list into a teaching slide table.",
      "status": "ready",
      "base_priority": 80,
      "dynamic_priority_delta": 0,
      "priority_reason": "Current top executable task.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": "turn-prev",
      "updated_at": "2026-04-13T00:00:00Z"
    }
  ]
}"#,
    )
    .expect("task ledger should write");

    let mut conversation = ready_conversation();
    conversation.cwd = workspace_dir.clone();
    conversation.draft_workspace_directory = workspace_dir.clone();
    conversation.input_buffer = ":q".to_string();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-main".to_string());
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.replace_planning_runtime_snapshot(
        app.planning
            .runtime
            .load_runtime_snapshot_or_invalid(&workspace_dir),
    );
    app.conversation_state = ConversationState::ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-main".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    let mut turn_calls = Vec::new();
    for _ in 0..20 {
        turn_calls = codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .collect::<Vec<_>>();
        if !turn_calls.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };

    assert_eq!(turn_calls.len(), 1);
    assert_eq!(conversation.input_buffer, ":q");
    assert_eq!(
        conversation.status_text,
        "auto follow-up submitted / turn 1/20 / mode: planning queue"
    );
    assert_eq!(
        conversation
            .last_auto_followup_activity
            .as_ref()
            .map(|activity| activity.summary.as_str()),
        Some("submitted auto turn 1/20")
    );

    app.start_turn_submission();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(app.shell_overlay, super::super::ShellOverlay::Queue);
    assert!(conversation.input_buffer.is_empty());

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn buffered_manual_text_is_preserved_while_auto_followup_submits() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("manual-buffer-followup");
    bootstrap_active_planning_workspace(&workspace_dir);
    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    std::fs::write(
        planning_dir.join("task-ledger.json"),
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-buffer-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Current next task for the manual buffer regression.",
      "title": "Convert kimchi lecture notes into table format",
      "description": "Turn the list into a teaching slide table.",
      "status": "ready",
      "base_priority": 80,
      "dynamic_priority_delta": 0,
      "priority_reason": "Current top executable task.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": "turn-prev",
      "updated_at": "2026-04-13T00:00:00Z"
    }
  ]
}"#,
    )
    .expect("task ledger should write");

    let mut conversation = ready_conversation();
    conversation.cwd = workspace_dir.clone();
    conversation.draft_workspace_directory = workspace_dir.clone();
    conversation.input_buffer = "operator draft stays here".to_string();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-main".to_string());
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.replace_planning_runtime_snapshot(
        app.planning
            .runtime
            .load_runtime_snapshot_or_invalid(&workspace_dir),
    );
    app.conversation_state = ConversationState::ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-main".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    let mut turn_calls = Vec::new();
    for _ in 0..20 {
        turn_calls = codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .collect::<Vec<_>>();
        if !turn_calls.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };

    assert_eq!(turn_calls.len(), 1);
    assert_eq!(conversation.input_buffer, "operator draft stays here");
    assert_eq!(
        conversation.status_text,
        "auto follow-up submitted / turn 1/20 / mode: planning queue"
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn stale_exhausted_repair_state_is_cleared_after_read_only_task_ledger_restore() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-repair-exhausted");
    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    bootstrap_active_planning_workspace(&workspace_dir);
    let bootstrap_artifacts =
        PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);
    std::fs::write(planning_dir.join("task-ledger.json"), "{ invalid json")
        .expect("invalid task ledger should write");

    let mut conversation = ready_conversation();
    conversation.cwd = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-repair-2".to_string());
    conversation.planning_repair_state = Some(PlanningRepairState {
        root_turn_id: "turn-root".to_string(),
        attempts_used: 2,
        max_attempts: 2,
        latest_request: PlanningRepairRequest {
            failure_summary: "failed to parse task-ledger.json".to_string(),
            validation_errors: vec!["failed to parse task-ledger.json".to_string()],
            directions_toml: bootstrap_artifacts.directions_toml.clone(),
            task_ledger_schema_json: bootstrap_artifacts.task_ledger_schema_json.clone(),
            accepted_task_ledger_json: bootstrap_artifacts.task_ledger_json.clone(),
            rejected_task_ledger_json: Some("{ invalid json".to_string()),
            rejected_archive_path: None,
        },
    });
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::ready(conversation);
    app.active_turn_planning_capture = Some(ready_turn_planning_capture(
        &workspace_dir,
        PlanningExecutionSnapshot {
            directions_toml: Some(bootstrap_artifacts.directions_toml.clone()),
            task_ledger_json: Some(bootstrap_artifacts.task_ledger_json.clone()),
            task_ledger_schema_json: Some(bootstrap_artifacts.task_ledger_schema_json.clone()),
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown.clone()),
            queue_snapshot_json: None,
        },
    ));

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-repair-2".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ));

    assert!(
        codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .iter()
            .all(|(_, prompt)| !prompt.contains("planning repair"))
    );
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert!(conversation.planning_repair_state.is_none());
    assert_eq!(
        conversation
            .planning_runtime_snapshot
            .preview_status_label(),
        "ready"
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn snapshot_capture_failure_blocks_followup_without_claiming_reconciliation() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-reconcile-snapshot-failure");
    let mut conversation = ready_conversation();
    conversation.cwd = workspace_dir.clone();
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::ready(conversation);
    app.active_turn_planning_capture = Some(failed_turn_planning_capture(
        &workspace_dir,
        "planning reconciliation could not capture the accepted planning snapshot before the turn started: failed to read task-ledger.json",
    ));

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-snapshot-failure".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ));

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
            .is_some_and(
                |detail| detail.contains("could not capture the accepted planning snapshot")
            )
    );
    assert!(
        conversation
            .runtime_notices
            .iter()
            .any(|notice| notice.contains("could not capture the accepted planning snapshot"))
    );
    assert!(
        !conversation
            .runtime_notices
            .iter()
            .any(|notice| notice.contains("restored the last accepted ledger"))
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

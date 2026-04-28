use std::thread;
use std::time::Duration;

use super::super::{
    ConversationInputState, ConversationMessage, ConversationMessageKind, ConversationRuntimeEvent,
    ConversationState, ConversationStreamEvent, PlanningBootstrapMode, PlanningBootstrapService,
    PlanningExecutionSnapshot, StartupState, TASK_LEDGER_FILE_PATH,
    bootstrap_active_planning_workspace, create_temp_workspace, make_test_app, ready_conversation,
    ready_turn_planning_capture, sample_startup_diagnostics,
};

#[test]
fn invalid_task_ledger_file_change_restores_read_only_export_without_hidden_repair() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-reconcile-app");
    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    bootstrap_active_planning_workspace(&workspace_dir);

    let bootstrap_artifacts =
        PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);

    let mut conversation = ready_conversation();
    conversation.cwd = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-invalid".to_string());
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

    std::fs::write(planning_dir.join("task-ledger.json"), "{ invalid json")
        .expect("invalid task ledger should write");

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-invalid".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ));

    let restored_task_ledger = std::fs::read_to_string(planning_dir.join("task-ledger.json"))
        .expect("restored task ledger should read");
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
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };

    assert_eq!(restored_task_ledger, bootstrap_artifacts.task_ledger_json);
    assert_eq!(
        conversation
            .planning_runtime_snapshot
            .preview_status_label(),
        "ready"
    );
    assert!(repair_prompt.is_none());
    assert!(
        conversation
            .runtime_notices
            .iter()
            .any(|notice| notice.contains("read-only task-ledger.json export"))
    );
    assert!(conversation.planning_repair_state.is_none());

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn queue_idle_active_derivation_creates_next_task_and_submits_auto_followup() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-active-derive-followup-app");
    bootstrap_active_planning_workspace(&workspace_dir);

    codex_port
        .new_thread_stream_behavior
        .lock()
        .expect("new-thread stream behavior mutex poisoned")
        .events = vec![
        ConversationStreamEvent::ThreadPrepared {
            thread_id: "planner-thread-1".to_string(),
            title: "Planner".to_string(),
            cwd: workspace_dir.clone(),
        },
        ConversationStreamEvent::AgentMessageCompleted {
            item_id: "planner-item-1".to_string(),
            phase: None,
            text: "planner derived the next lecture-authoring task".to_string(),
        },
        ConversationStreamEvent::TurnCompleted {
            turn_id: "planner-turn-1".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ];
    codex_port
        .new_thread_stream_behavior
        .lock()
        .expect("new-thread stream behavior mutex poisoned")
        .planning_file_writes = vec![(
        TASK_LEDGER_FILE_PATH.to_string(),
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-chef-outline-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "The latest answer already proposed the next lecture-building sequence.",
      "title": "중식 분류 체계 강의 자료 초안 작성",
      "description": "중국 8대 요리 계열과 입문 분류 체계를 강의 자료용 목차로 정리한다.",
      "status": "ready",
      "base_priority": 85,
      "dynamic_priority_delta": 0,
      "priority_reason": "The latest reply explicitly listed this as the first next step.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": "turn-main",
      "updated_at": "2026-04-14T00:00:00Z"
    },
    {
      "id": "task-chef-outline-2",
      "direction_id": "general-workstream",
      "direction_relation_note": "Second follow-up step from the latest answer.",
      "title": "대표 메뉴 20선 강의 섹션 구성",
      "description": "대표 메뉴 20선을 강의 흐름에 맞게 선정하고 섹션 순서를 잡는다.",
      "status": "proposed",
      "base_priority": 70,
      "dynamic_priority_delta": 0,
      "priority_reason": "The latest reply listed this as the next follow-up after the classification section.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": "turn-main",
      "updated_at": "2026-04-14T00:00:00Z"
    }
  ]
}"#
        .to_string(),
    )];

    let latest_user_request =
        "중식 요리사가 되기 위해, 강의 자료를 만들어줘 우선 중국요리 목록부터 보여줘";
    let latest_reply = [
        "좋습니다. 강의 자료용으로 먼저 중국요리 목록을 보기 좋게 정리해드리겠습니다.",
        "",
        "강의 자료를 이어서 만들려면 다음 순서가 좋습니다.",
        "1. 중식 분류 체계",
        "2. 꼭 알아야 할 대표 메뉴 20선",
        "3. 기초 칼질, 웍 사용법, 불 조절",
    ]
    .join("\n");

    let mut conversation = ready_conversation();
    conversation.cwd = workspace_dir.clone();
    conversation.draft_workspace_directory = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-main".to_string());
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::User,
        latest_user_request,
        None,
        None,
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        latest_reply,
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
    let mut hidden_prompts = Vec::new();
    for _ in 0..20 {
        turn_calls = codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .collect::<Vec<_>>();
        hidden_prompts = codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .collect::<Vec<_>>();
        if !turn_calls.is_empty() && !hidden_prompts.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };

    assert_eq!(turn_calls.len(), 1);
    assert!(
        turn_calls[0].contains("중식 분류 체계 강의 자료 초안 작성"),
        "auto follow-up prompt should target the derived queue head: {}",
        turn_calls[0]
    );
    assert_eq!(
        conversation
            .planning_runtime_snapshot
            .queue_head()
            .map(|task| task.task_id.as_str()),
        Some("task-chef-outline-1")
    );
    assert_eq!(
        conversation.status_text,
        "auto follow-up submitted / turn 1/20 / mode: planning queue"
    );
    assert_eq!(
        app.planner_worker_panel_state
            .last_operation_label
            .as_deref(),
        Some("active-derive")
    );
    assert_eq!(hidden_prompts.len(), 1);
    assert!(hidden_prompts[0].contains("latest operator request:"));
    assert!(hidden_prompts[0].contains("중식 요리사가 되기 위해"));
    assert!(hidden_prompts[0].contains("1. 중식 분류 체계"));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

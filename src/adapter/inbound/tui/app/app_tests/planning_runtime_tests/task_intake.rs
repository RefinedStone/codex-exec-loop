use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::{
    ActiveTurnPlanningCapture, ConversationInputState, ConversationRuntimeEvent, ConversationState,
    InlineShellCommandInput, PlanningExecutionSnapshot, ShellOverlay, StartupState,
    TaskIntakeOverlayStep, TempGitWorkspace, bootstrap_active_planning_workspace,
    create_temp_workspace, make_test_app, replace_candidate_planning_workspace_file,
    sample_startup_diagnostics, sync_draft_conversation_to_startup_workspace,
};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskAuthorityCommit;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::{
    PlanningBootstrapMode, PlanningBootstrapService, TASK_LEDGER_FILE_PATH,
};
use crate::domain::planning::{PriorityQueueProjection, TaskLedgerDocument};

#[test]
fn task_command_with_prompt_previews_and_commits_ready_task() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("task-intake-commit");
    bootstrap_active_planning_workspace(&workspace_dir);
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":task Add a release checklist")
            .expect("task command should parse"),
    );

    assert_eq!(app.shell_overlay, ShellOverlay::TaskIntake);
    assert!(app.task_intake_overlay_ui_state.proposal().is_some());
    assert!(
        app.task_intake_overlay_ui_state
            .proposal()
            .expect("proposal")
            .draft
            .task
            .title
            .contains("Add a release checklist")
    );

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE,)));

    assert_eq!(app.shell_overlay, ShellOverlay::Queue);
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should be ready");
    };
    assert!(
        conversation
            .status_text
            .contains("task accepted into planning queue")
    );
    assert!(
        conversation
            .planning_runtime_snapshot
            .queue_head()
            .is_some_and(|task| task.task_title.contains("Add a release checklist"))
    );

    let workspace = FilesystemPlanningWorkspaceAdapter::new()
        .load_planning_workspace_files(&workspace_dir)
        .expect("workspace should load");
    assert!(
        workspace
            .task_ledger_json
            .as_deref()
            .expect("task ledger should exist")
            .contains("Add a release checklist")
    );
    let exported_ledger =
        std::fs::read_to_string(std::path::Path::new(&workspace_dir).join(TASK_LEDGER_FILE_PATH))
            .expect("task ledger export should be refreshed");
    assert!(exported_ledger.contains("Add a release checklist"));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn task_preview_edit_and_cancel_keys_keep_overlay_state_coherent() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("task-intake-keys");
    bootstrap_active_planning_workspace(&workspace_dir);
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":task Add release notes")
            .expect("task command should parse"),
    );

    assert_eq!(
        app.task_intake_overlay_ui_state.step(),
        TaskIntakeOverlayStep::Preview
    );
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE,)));
    assert_eq!(
        app.task_intake_overlay_ui_state.step(),
        TaskIntakeOverlayStep::Prompt
    );
    assert!(app.task_intake_overlay_ui_state.proposal().is_none());

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));
    assert_eq!(
        app.task_intake_overlay_ui_state.step(),
        TaskIntakeOverlayStep::Preview
    );
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE,)));
    assert_eq!(app.shell_overlay, ShellOverlay::Hidden);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":task Add release notes")
            .expect("task command should parse"),
    );
    assert_eq!(app.shell_overlay, ShellOverlay::TaskIntake);
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)));
    assert_eq!(app.shell_overlay, ShellOverlay::Hidden);

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn task_command_without_prompt_keeps_editor_open_on_blank_preview() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("task-intake-blank");
    bootstrap_active_planning_workspace(&workspace_dir);
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":task").expect("task command should parse"),
    );
    assert_eq!(app.shell_overlay, ShellOverlay::TaskIntake);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    assert_eq!(app.shell_overlay, ShellOverlay::TaskIntake);
    assert!(app.task_intake_overlay_ui_state.proposal().is_none());
    assert!(
        app.task_intake_overlay_ui_state
            .error()
            .expect("blank prompt should show an error")
            .contains("task prompt")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn task_command_bootstraps_missing_planning_workspace_and_commits_ready_task() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("task-intake-default-bootstrap");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":task Add work").expect("task command should parse"),
    );

    assert_eq!(app.shell_overlay, ShellOverlay::TaskIntake);
    assert!(
        app.task_intake_overlay_ui_state
            .proposal()
            .expect("fresh workspace task command should preview")
            .draft
            .task
            .title
            .contains("Add work")
    );
    assert!(
        std::path::Path::new(&workspace_dir)
            .join(".codex-exec-loop")
            .exists(),
        "task intake should initialize the default planning workspace"
    );

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE,)));

    assert_eq!(app.shell_overlay, ShellOverlay::Queue);
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should be ready");
    };
    assert!(
        conversation
            .planning_runtime_snapshot
            .queue_head()
            .is_some_and(|task| task.task_title.contains("Add work"))
    );
    let exported_ledger = std::fs::read_to_string(
        std::path::Path::new(&workspace_dir)
            .join(".codex-exec-loop/runtime/exports/task-ledger.json"),
    )
    .expect("task authority export should be refreshed");
    assert!(
        exported_ledger.contains("Add work"),
        "task intake should commit into the ready task authority"
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn task_command_blocks_default_bootstrap_when_candidate_workspace_exists() {
    let (mut app, _) = make_test_app();
    let workspace = TempGitWorkspace::new("task-intake-candidate-without-authority");
    let workspace_dir = workspace.workspace_dir().to_string();
    let bootstrap =
        PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
    replace_candidate_planning_workspace_file(
        &workspace_dir,
        bootstrap.directions_path.as_str(),
        bootstrap.directions_toml.as_str(),
    );
    replace_candidate_planning_workspace_file(
        &workspace_dir,
        bootstrap.task_ledger_path.as_str(),
        bootstrap.task_ledger_json.as_str(),
    );
    replace_candidate_planning_workspace_file(
        &workspace_dir,
        bootstrap.task_ledger_schema_path.as_str(),
        bootstrap.task_ledger_schema_json.as_str(),
    );
    replace_candidate_planning_workspace_file(
        &workspace_dir,
        bootstrap.result_output_path.as_str(),
        bootstrap.result_output_markdown.as_str(),
    );
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":task Add work").expect("task command should parse"),
    );

    assert_eq!(app.shell_overlay, ShellOverlay::TaskIntake);
    assert!(app.task_intake_overlay_ui_state.proposal().is_none());
    let error = app
        .task_intake_overlay_ui_state
        .error()
        .expect("candidate workspace should block default bootstrap");
    assert!(
        error.contains("tracked planning candidates exist without active authority"),
        "candidate workspace should be preserved: {error}"
    );
    assert!(
        error.contains(":directions apply") && error.contains(":queue apply"),
        "blocked bootstrap should guide tracked candidate apply commands: {error}"
    );
    let workspace = FilesystemPlanningWorkspaceAdapter::new()
        .load_planning_workspace_files(&workspace_dir)
        .expect("active workspace should load");
    assert!(
        !workspace.has_any_files(),
        "task intake must not overwrite active authority from default bootstrap"
    );
}

#[test]
fn task_command_preserves_unknown_direction_failure_and_guides_directions_apply() {
    let (mut app, _) = make_test_app();
    let workspace = TempGitWorkspace::new("task-intake-authority-drift");
    let workspace_dir = workspace.workspace_dir().to_string();
    bootstrap_active_planning_workspace(&workspace_dir);
    write_tracked_directions_with_claude_runner(&workspace_dir);
    commit_task_authority_with_claude_runner_task(&workspace_dir);
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":task Add follow-up work")
            .expect("task command should parse"),
    );

    assert_eq!(app.shell_overlay, ShellOverlay::TaskIntake);
    let error = app
        .task_intake_overlay_ui_state
        .error()
        .expect("authority drift should block task preview");
    assert!(
        error.contains(
            "task claude-runner-task references unknown direction_id claude-first-headless-cli-runner"
        ),
        "task intake should preserve the validator's first concrete failure: {error}"
    );
    assert!(
        error.contains(":directions apply"),
        "task intake should point to tracked directions repair: {error}"
    );
}

fn write_tracked_directions_with_claude_runner(workspace_dir: &str) {
    let directions_path = std::path::Path::new(workspace_dir)
        .join(".codex-exec-loop")
        .join("planning")
        .join("directions.toml");
    std::fs::create_dir_all(
        directions_path
            .parent()
            .expect("directions path should have a parent"),
    )
    .expect("tracked planning directory should be created");
    std::fs::write(
        directions_path,
        r#"version = 1

[queue_idle]
policy = "review_and_enqueue"
prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md"

[[directions]]
id = "general-workstream"
title = "General workstream"
summary = "Default planning direction."
success_criteria = [
    "Keep work represented in the task ledger.",
]
scope_hints = [
    "Use for generic runtime intake work.",
]
detail_doc_path = ""
state = "active"

[[directions]]
id = "claude-first-headless-cli-runner"
title = "Claude-first headless CLI runner"
summary = "Tracked direction that has not been applied to the authority store yet."
success_criteria = [
    "Task intake can see this direction after :directions apply.",
]
scope_hints = [
    "Use for authority drift regression coverage.",
]
detail_doc_path = ""
state = "active"
"#,
    )
    .expect("tracked directions should write");
}

fn commit_task_authority_with_claude_runner_task(workspace_dir: &str) {
    let task_ledger: TaskLedgerDocument = serde_json::from_str(
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "claude-runner-task",
      "direction_id": "claude-first-headless-cli-runner",
      "direction_relation_note": "authority drift regression",
      "title": "Run the claude-first headless CLI",
      "description": "Keep this task in the authority store while active directions are stale.",
      "status": "ready",
      "base_priority": 80,
      "dynamic_priority_delta": 0,
      "priority_reason": "Regression fixture",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-23T12:00:00Z"
    }
  ]
}"#,
    )
    .expect("task authority fixture should parse");
    let queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };
    SqlitePlanningAuthorityAdapter::commit_task_authority_snapshot(
        workspace_dir,
        PlanningTaskAuthorityCommit {
            observed_planning_revision: None,
            task_ledger: &task_ledger,
            queue_projection: &queue_projection,
        },
    )
    .expect("task authority fixture should commit");
}

#[test]
fn task_command_enter_during_streaming_defers_until_post_turn_safe_point() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("task-intake-streaming-safe-point");
    bootstrap_active_planning_workspace(&workspace_dir);
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    let execution_snapshot = PlanningExecutionSnapshot {
        directions_toml: Some(
            std::fs::read_to_string(planning_dir.join("directions.toml"))
                .expect("directions should read"),
        ),
        task_ledger_json: Some(
            std::fs::read_to_string(planning_dir.join("task-ledger.json"))
                .expect("task ledger should read"),
        ),
        task_ledger_schema_json: Some(
            std::fs::read_to_string(planning_dir.join("task-ledger.schema.json"))
                .expect("task ledger schema should read"),
        ),
        result_output_markdown: Some(
            std::fs::read_to_string(planning_dir.join("result-output.md"))
                .expect("result output should read"),
        ),
        queue_snapshot_json: None,
    };

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.input_buffer = ":task Add a streamed-turn follow-up".to_string();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-streaming".to_string());
    app.active_turn_planning_capture = Some(ActiveTurnPlanningCapture::ready(
        &workspace_dir,
        execution_snapshot,
    ));

    app.start_turn_submission();

    assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(
        conversation.input_buffer,
        ":task Add a streamed-turn follow-up"
    );
    assert!(conversation.status_text.contains("planning-safe point"));

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-streaming".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ));

    assert_eq!(app.shell_overlay, ShellOverlay::TaskIntake);
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert!(
        !conversation
            .auto_follow_state
            .post_turn_continuation_paused()
    );
    assert!(!conversation.auto_follow_state.has_live_activity());
    assert!(
        app.task_intake_overlay_ui_state
            .proposal()
            .expect("queued task command should preview")
            .draft
            .task
            .title
            .contains("Add a streamed-turn follow-up")
    );

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)));

    let exported_ledger = std::fs::read_to_string(
        std::path::Path::new(&workspace_dir)
            .join(".codex-exec-loop/runtime/exports/task-ledger.json"),
    )
    .expect("task ledger export should be refreshed");
    assert!(exported_ledger.contains("Add a streamed-turn follow-up"));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

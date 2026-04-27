use std::collections::HashSet;
use std::path::Path;

use super::super::{
    ConversationState, InlineShellCommandInput, PlanningInitOverlayStep, ShellOverlay,
    StartupState, bootstrap_active_planning_workspace, create_temp_workspace, make_test_app,
    rewrite_active_directions_toml, sample_startup_diagnostics,
    sync_draft_conversation_to_startup_workspace,
};
use crate::application::service::planning::TASK_LEDGER_FILE_PATH;

#[test]
fn planning_command_opens_first_run_simple_review() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-init-selector");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":planning").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::SimpleReview
    );
    assert!(
        conversation
            .status_text
            .contains("planning simple review ready")
    );
    assert!(
        conversation
            .status_text
            .contains("simple behavior: no next task yet")
    );
    assert!(
        conversation
            .status_text
            .contains("queue-idle review stays enabled")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_command_opens_existing_workspace_controls_when_workspace_is_present() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-existing-controls");
    bootstrap_active_planning_workspace(&workspace_dir);
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":planning").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::ExistingWorkspace
    );
    assert!(
        conversation
            .status_text
            .contains("operator surface: planning setup / existing workspace")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn directions_apply_command_refreshes_overlay_on_success() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("directions-apply-command");
    bootstrap_active_planning_workspace(&workspace_dir);
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":directions apply").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
    assert!(
        conversation
            .status_text
            .contains("tracked directions applied")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn directions_apply_command_surfaces_validation_reason() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("directions-apply-invalid-command");
    bootstrap_active_planning_workspace(&workspace_dir);
    rewrite_active_directions_toml(&workspace_dir, |directions| {
        directions.replace(
            r#"prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md""#,
            r#"prompt_path = ".codex-exec-loop/planning/prompts/missing.md""#,
        )
    });
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":directions apply").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(
        conversation
            .status_text
            .contains("tracked directions apply blocked")
    );
    assert!(
        conversation
            .status_text
            .contains("queue_idle.prompt_path does not exist")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn queue_apply_command_refreshes_queue_overlay_on_success() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("queue-apply-command");
    bootstrap_active_planning_workspace(&workspace_dir);
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":queue apply").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(app.shell_overlay, ShellOverlay::Queue);
    assert!(
        conversation
            .status_text
            .contains("tracked task catalog applied")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn queue_apply_command_surfaces_validation_reason() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("queue-apply-invalid-command");
    bootstrap_active_planning_workspace(&workspace_dir);
    std::fs::write(
        Path::new(&workspace_dir).join(TASK_LEDGER_FILE_PATH),
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "missing-direction",
      "direction_relation_note": "queue apply test",
      "title": "Apply tracked queue head",
      "description": "Sync the tracked queue head into active planning.",
      "status": "ready",
      "base_priority": 50,
      "dynamic_priority_delta": 0,
      "priority_reason": "Current top executable task.",
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
    .expect("invalid task ledger should write");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":queue apply").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(
        conversation
            .status_text
            .contains("tracked task catalog apply blocked")
    );
    assert!(
        conversation
            .status_text
            .contains("references unknown direction_id")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn doctor_command_reports_absent_workspace_and_points_to_init() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-doctor-absent");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":doctor").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::ModeSelection
    );
    assert!(conversation.status_text.contains("planning state: absent"));
    assert!(conversation.status_text.contains("next action: run :init"));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn doctor_command_is_read_only_for_invalid_supporting_paths() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-doctor-command");
    bootstrap_active_planning_workspace(&workspace_dir);
    std::fs::write(
        Path::new(&workspace_dir).join("README.md"),
        "# invalid planning supporting path\n",
    )
    .expect("workspace readme should be writable");
    rewrite_active_directions_toml(&workspace_dir, |directions| {
        directions
            .replace(
                r#"detail_doc_path = """#,
                r#"detail_doc_path = "README.md""#,
            )
            .replace(
                r#"prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md""#,
                r#"prompt_path = "README.md""#,
            )
    });
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":doctor").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(conversation.status_text.contains("planning state: invalid"));
    assert!(conversation.status_text.contains("issue:"));
    assert!(
        std::fs::read_to_string(
            Path::new(&workspace_dir).join(".codex-exec-loop/planning/directions.toml")
        )
        .expect("directions should stay readable")
        .contains(r#"detail_doc_path = "README.md""#)
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn init_command_stages_simple_review_immediately() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-init-fast-path");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":init").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::SimpleReview
    );
    assert!(
        conversation
            .status_text
            .contains("planning simple review ready")
    );
    assert!(
        conversation
            .status_text
            .contains("simple behavior: no next task yet")
    );
    assert!(
        conversation
            .status_text
            .contains("queue-idle review stays enabled")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn init_command_reuses_existing_workspace_controls_when_workspace_exists() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-init-existing");
    bootstrap_active_planning_workspace(&workspace_dir);
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":init").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::ExistingWorkspace
    );
    assert!(
        conversation
            .status_text
            .contains("planning workspace already exists")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn reset_queue_command_rewrites_task_ledger_immediately() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-reset-queue-command");
    bootstrap_active_planning_workspace(&workspace_dir);
    std::fs::write(
        Path::new(&workspace_dir).join(TASK_LEDGER_FILE_PATH),
        r#"{"version":1,"tasks":[{"id":"task-1","direction_id":"general-workstream","direction_relation_note":"keep moving","title":"Do work","description":"Reset queue state","status":"ready","base_priority":10,"created_by":"user","last_updated_by":"user","updated_at":"2026-04-16T00:00:00Z"}]}"#,
    )
    .expect("task ledger should be writable");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":reset queue").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(
        conversation
            .status_text
            .contains("planning reset applied / target: queue")
    );
    assert_eq!(
        std::fs::read_to_string(Path::new(&workspace_dir).join(TASK_LEDGER_FILE_PATH))
            .expect("task ledger should be readable"),
        "{\n  \"version\": 1,\n  \"tasks\": []\n}"
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn reset_directions_command_requires_confirm_before_rewriting_active_files() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-reset-directions-preview");
    bootstrap_active_planning_workspace(&workspace_dir);
    rewrite_active_directions_toml(&workspace_dir, |directions| {
        directions
            .replace(
                r#"detail_doc_path = """#,
                r#"detail_doc_path = ".codex-exec-loop/planning/directions/custom.md""#,
            )
            .replace(
                r#"prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md""#,
                r#"prompt_path = ".codex-exec-loop/planning/prompts/custom.md""#,
            )
    });
    std::fs::create_dir_all(Path::new(&workspace_dir).join(".codex-exec-loop/planning/directions"))
        .expect("directions directory should be created");
    std::fs::write(
        Path::new(&workspace_dir).join(".codex-exec-loop/planning/directions/custom.md"),
        "# custom detail doc\n",
    )
    .expect("detail doc should be writable");
    std::fs::write(
        Path::new(&workspace_dir).join(".codex-exec-loop/planning/prompts/custom.md"),
        "# custom prompt\n",
    )
    .expect("prompt should be writable");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":reset directions").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(
        conversation
            .status_text
            .contains("reset directions preview")
    );
    assert!(
        conversation
            .status_text
            .contains(":reset directions confirm")
    );
    assert!(
        Path::new(&workspace_dir)
            .join(".codex-exec-loop/planning/directions/custom.md")
            .exists()
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn reset_directions_confirm_rewrites_default_direction_artifacts() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-reset-directions-confirm");
    bootstrap_active_planning_workspace(&workspace_dir);
    std::fs::write(
        Path::new(&workspace_dir).join(TASK_LEDGER_FILE_PATH),
        r#"{"version":1,"tasks":[{"id":"task-1","direction_id":"general-workstream","direction_relation_note":"done work","title":"Done work","description":"The current work is already complete","status":"done","base_priority":10,"created_by":"user","last_updated_by":"user","updated_at":"2026-04-16T00:00:00Z"}]}"#,
    )
    .expect("task ledger should be writable");
    rewrite_active_directions_toml(&workspace_dir, |directions| {
        directions.replace(
            r#"detail_doc_path = """#,
            r#"detail_doc_path = ".codex-exec-loop/planning/directions/custom.md""#,
        )
    });
    std::fs::create_dir_all(Path::new(&workspace_dir).join(".codex-exec-loop/planning/directions"))
        .expect("directions directory should be created");
    std::fs::write(
        Path::new(&workspace_dir).join(".codex-exec-loop/planning/directions/custom.md"),
        "# custom detail doc\n",
    )
    .expect("detail doc should be writable");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":reset directions confirm").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(
        conversation
            .status_text
            .contains("planning reset applied / target: directions")
    );
    let directions = std::fs::read_to_string(
        Path::new(&workspace_dir).join(".codex-exec-loop/planning/directions.toml"),
    )
    .expect("directions should be readable");
    assert!(directions.contains("general-workstream"));
    assert!(
        !Path::new(&workspace_dir)
            .join(".codex-exec-loop/planning/directions/custom.md")
            .exists()
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_simple_mode_selection_stages_bootstrap_files_in_current_workspace() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-init-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);
    let staged_drafts_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning")
        .join("drafts");

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":planning").expect("command should parse"),
    );
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(
        conversation
            .status_text
            .contains("planning simple review ready")
    );
    assert!(conversation.status_text.contains("staged draft:"));
    assert!(conversation.status_text.contains("validation state: ok"));
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::SimpleReview
    );
    assert!(!app.planning_draft_editor_ui_state.is_open());
    assert!(staged_drafts_dir.exists());
    let draft_directories = std::fs::read_dir(&staged_drafts_dir)
        .expect("drafts directory should be readable")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    assert_eq!(draft_directories.len(), 1);
    let staged_files = std::fs::read_dir(&draft_directories[0])
        .expect("staged draft directory should be readable")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect::<HashSet<_>>();
    let expected_files = [
        "directions.toml".to_string(),
        "task-ledger.json".to_string(),
        "task-ledger.schema.json".to_string(),
        "result-output.md".to_string(),
        "prompts".to_string(),
    ]
    .into_iter()
    .collect::<HashSet<_>>();
    assert_eq!(staged_files, expected_files);
    let directions = std::fs::read_to_string(draft_directories[0].join("directions.toml"))
        .expect("staged directions should be readable");
    assert!(directions.contains("general-workstream"));
    assert!(
        draft_directories[0]
            .join("prompts")
            .join("queue-idle-review.md")
            .exists()
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

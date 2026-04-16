use std::collections::HashSet;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::{
    ConversationState, InlineShellCommandInput, PLAN_OFF_FILE_PATH, PlanningInitOverlayStep,
    ShellOverlay, StartupState, bootstrap_active_planning_workspace, create_temp_workspace,
    make_test_app, rewrite_active_directions_toml, sample_startup_diagnostics,
    sync_draft_conversation_to_startup_workspace,
};
use crate::application::service::planning::PlanningRuntimeWorkspaceStatus;
use crate::application::service::planning_contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, default_direction_detail_doc_path,
};

#[test]
fn planning_init_command_opens_selector_overlay() {
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
        PlanningInitOverlayStep::ModeSelection
    );
    assert!(
        conversation
            .status_text
            .contains("operator surface: planning setup / workspace: not initialized")
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
            .contains("operator surface: planning setup / planning mode: on")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_off_command_turns_plan_off_and_blocks_directions() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-off-command");
    bootstrap_active_planning_workspace(&workspace_dir);
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":planning off").expect("command should parse"),
    );

    let plan_off_path = Path::new(&workspace_dir).join(PLAN_OFF_FILE_PATH);
    assert!(plan_off_path.exists(), "Plan off marker should be written");

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(!conversation.planning_runtime_snapshot.plan_enabled());
    assert!(conversation.status_text.contains("planning mode: off"));

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":directions").expect("command should parse"),
    );
    assert_ne!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(
        conversation.status_text,
        "planning mode: off / next action: open :planning to initialize the workspace"
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_on_command_requires_existing_workspace() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-on-command-no-workspace");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":planning on").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(
        conversation.status_text,
        "planning workspace: missing / next action: open :planning to initialize it"
    );
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::ModeSelection
    );
    let plan_off_path = Path::new(&workspace_dir).join(PLAN_OFF_FILE_PATH);
    assert!(
        !plan_off_path.exists(),
        "Plan off marker should stay absent"
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_doctor_command_repairs_safe_supporting_path_errors() {
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
        InlineShellCommandInput::parse(":planning doctor").expect("command should parse"),
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(conversation.status_text.contains("planning doctor applied"));
    assert!(conversation.status_text.contains("validation: ok"));
    assert_ne!(
        conversation.planning_runtime_snapshot.workspace_status(),
        PlanningRuntimeWorkspaceStatus::Invalid
    );
    assert!(
        Path::new(&workspace_dir)
            .join(default_direction_detail_doc_path("general-workstream"))
            .is_file()
    );
    assert!(
        Path::new(&workspace_dir)
            .join(DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH)
            .is_file()
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_existing_workspace_overlay_prompts_to_turn_plan_on_before_directions() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-existing-workspace-turn-on");
    bootstrap_active_planning_workspace(&workspace_dir);
    std::fs::write(
        Path::new(&workspace_dir).join(PLAN_OFF_FILE_PATH),
        "plan off\n",
    )
    .expect("plan off marker should be writable");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":planning").expect("command should parse"),
    );
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::ExistingWorkspace
    );

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE,)));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(
        conversation.status_text,
        "planning mode: off / next action: turn Plan on in this menu first"
    );
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);

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
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(
        conversation
            .status_text
            .contains("planning simple draft staged")
    );
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

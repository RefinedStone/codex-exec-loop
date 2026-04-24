use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::{
    ConversationState, InlineShellCommandInput, ShellOverlay, StartupState, TaskIntakeOverlayStep,
    bootstrap_active_planning_workspace, create_temp_workspace, make_test_app,
    sample_startup_diagnostics, sync_draft_conversation_to_startup_workspace,
};
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning::shared::contract::TASK_LEDGER_FILE_PATH;

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
fn task_command_rejects_missing_planning_workspace_without_bootstrap() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("task-intake-missing-workspace");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":task Add planning task")
            .expect("task command should parse"),
    );

    assert_eq!(app.shell_overlay, ShellOverlay::TaskIntake);
    assert!(
        app.task_intake_overlay_ui_state
            .error()
            .expect("missing workspace should show an error")
            .contains(":planning")
    );
    assert!(
        !std::path::Path::new(&workspace_dir)
            .join(".codex-exec-loop")
            .exists(),
        "task intake should not bootstrap planning files implicitly"
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

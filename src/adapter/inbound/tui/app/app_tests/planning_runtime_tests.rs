use super::{
    InlineShellCommandInput, KeyCode, KeyEvent, KeyModifiers, NativeTuiApp,
    PlanningInitOverlayStep, sync_draft_conversation_to_startup_workspace,
};

#[path = "planning_runtime_tests/workspace_controls.rs"]
mod workspace_controls;

#[path = "planning_runtime_tests/draft_review.rs"]
mod draft_review;

#[path = "planning_runtime_tests/draft_editor.rs"]
mod draft_editor;

#[path = "planning_runtime_tests/repair_and_queue.rs"]
mod repair_and_queue;

#[path = "planning_runtime_tests/queue_refresh.rs"]
mod queue_refresh;

#[path = "planning_runtime_tests/guard_and_buffering.rs"]
mod guard_and_buffering;

fn open_planning_simple_review(app: &mut NativeTuiApp) {
    sync_draft_conversation_to_startup_workspace(app);
    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":planning").expect("command should parse"),
    );
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::SimpleReview
    );
}

fn open_planning_manual_editor(app: &mut NativeTuiApp) {
    sync_draft_conversation_to_startup_workspace(app);
    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":planning").expect("command should parse"),
    );
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::ManualEditor
    );
}

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::{
    ConversationState, DirectionsMaintenanceOverlayStep, InlineShellCommandInput,
    PlanningInitOverlayStep, ShellOverlay, StartupState, bootstrap_active_planning_workspace,
    create_temp_workspace, make_test_app, rewrite_active_directions_toml,
    sample_startup_diagnostics, sync_draft_conversation_to_startup_workspace,
};
use super::open_planning_simple_review;

#[test]
fn planning_simple_mode_promote_copies_active_files_and_refreshes_prompt_context() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-simple-promote-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should stay in ready state");
    };
    conversation.cwd = workspace_dir.clone();

    open_planning_simple_review(&mut app);
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::SimpleReview
    );

    assert!(
        app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL,))
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
    assert!(conversation.status_text.contains("planning draft promoted"));
    assert_eq!(
        conversation
            .planning_runtime_snapshot
            .preview_status_label(),
        "ready"
    );

    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    assert!(planning_dir.join("directions.toml").exists());
    assert!(planning_dir.join("task-ledger.json").exists());
    assert!(planning_dir.join("task-ledger.schema.json").exists());
    assert!(planning_dir.join("result-output.md").exists());

    let directions = std::fs::read_to_string(planning_dir.join("directions.toml"))
        .expect("promoted directions should be readable");
    assert!(directions.contains("general-workstream"));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn directions_editor_still_opens_when_supporting_paths_are_invalid() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("directions-invalid-supporting-paths-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);
    bootstrap_active_planning_workspace(&workspace_dir);
    rewrite_active_directions_toml(&workspace_dir, |directions| {
        directions.replace(
            r#"prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md""#,
            r#"prompt_path = "../escape.md""#,
        )
    });

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":directions").expect("command should parse"),
    );
    assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));
    assert!(app.planning_draft_editor_ui_state.is_open());

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(conversation.status_text.contains("directions editor ready"));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn directions_overlay_counts_broken_mappings_and_allows_repair_flow() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("directions-broken-mappings-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);
    bootstrap_active_planning_workspace(&workspace_dir);
    rewrite_active_directions_toml(&workspace_dir, |directions| {
        directions
            .replace(
                r#"prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md""#,
                r#"prompt_path = "../escape.md""#,
            )
            .replace(
                r#"detail_doc_path = """#,
                r#"detail_doc_path = ".codex-exec-loop/planning/directions/general-workstream.md""#,
            )
    });

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":directions").expect("command should parse"),
    );

    let summary = app
        .directions_maintenance_overlay_ui_state
        .summary()
        .expect("directions summary should exist");
    assert_eq!(summary.missing_detail_doc_count, 0);
    assert_eq!(summary.broken_detail_doc_count, 1);
    assert_eq!(summary.queue_idle_prompt_status.label(), "broken");

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE,)));
    assert_eq!(
        app.directions_maintenance_overlay_ui_state.step(),
        DirectionsMaintenanceOverlayStep::DetailDocSelection
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn directions_manual_editor_close_returns_to_overview() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("directions-close-overview-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);
    bootstrap_active_planning_workspace(&workspace_dir);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":directions").expect("command should parse"),
    );
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));
    assert_eq!(
        app.directions_maintenance_overlay_ui_state.step(),
        DirectionsMaintenanceOverlayStep::ManualEditor
    );

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)));
    assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
    assert_eq!(
        app.directions_maintenance_overlay_ui_state.step(),
        DirectionsMaintenanceOverlayStep::Overview
    );
    assert!(!app.planning_draft_editor_ui_state.is_open());

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(
        conversation
            .status_text
            .contains("directions editor closed")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_simple_mode_review_can_open_embedded_editor() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-simple-editor-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));

    open_planning_simple_review(&mut app);

    assert!(
        app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL,))
    );
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::ManualEditor
    );
    assert!(app.planning_draft_editor_ui_state.is_open());

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(
        conversation
            .status_text
            .contains("planning simple draft editor ready")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_simple_mode_review_can_edit_max_auto_turns_without_leaving_overlay() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-simple-max-auto-turns-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));

    open_planning_simple_review(&mut app);

    assert!(
        app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL,))
    );
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::SimpleReview
    );
    assert!(app.is_max_auto_turns_editing());
    app.followup_overlay_ui_state.max_auto_turns_editor.buffer = "7".to_string();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(conversation.auto_follow_state.max_auto_turns_value(), 7);
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::SimpleReview
    );
    assert!(!app.is_max_auto_turns_editing());

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_detail_manual_selection_opens_embedded_editor() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-init-detail-app");
    let staged_drafts_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning")
        .join("drafts");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":planning").expect("command should parse"),
    );
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::DetailSelection
    );
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(
        conversation
            .status_text
            .contains("planning draft editor ready")
    );
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::ManualEditor
    );
    assert!(app.planning_draft_editor_ui_state.is_open());
    assert!(staged_drafts_dir.exists());
    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

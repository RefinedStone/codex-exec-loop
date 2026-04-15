use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::super::{PlanningInitDetailSelection, PlanningInitModeSelection};
use super::super::{
    ConversationState, PlanningInitOverlayStep, ShellOverlay, StartupState,
    bootstrap_active_planning_workspace, build_planning_init_overlay_view,
    count_staged_planning_drafts, create_temp_workspace, make_test_app, ready_conversation,
    sample_planning_runtime_snapshot, sample_startup_diagnostics,
    sync_draft_conversation_to_startup_workspace,
};
use super::open_planning_manual_editor;

#[test]
fn planning_manual_editor_save_writes_staged_draft_file_and_clears_dirty_state() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-editor-save-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));

    open_planning_manual_editor(&mut app);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('#'), KeyModifiers::NONE,)));
    assert!(app.planning_draft_editor_ui_state.has_dirty_buffers());

    assert!(
        app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL,))
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(conversation.status_text.contains("planning draft saved"));
    assert!(conversation.status_text.contains("Ctrl+P"));
    assert!(!app.planning_draft_editor_ui_state.has_dirty_buffers());

    let draft_directory = std::fs::read_dir(
        std::path::Path::new(&workspace_dir)
            .join(".codex-exec-loop")
            .join("planning")
            .join("drafts"),
    )
    .expect("drafts directory should be readable")
    .filter_map(|entry| entry.ok())
    .map(|entry| entry.path())
    .next()
    .expect("draft directory should exist");
    let result_output = std::fs::read_to_string(draft_directory.join("result-output.md"))
        .expect("staged result output should be readable");
    assert!(result_output.starts_with('#'));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_manual_editor_dirty_close_requires_confirmation() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-editor-close-dirty-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    open_planning_manual_editor(&mut app);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('#'), KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)));
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert!(app.planning_draft_editor_ui_state.is_open());
    assert!(
        app.planning_draft_editor_ui_state
            .is_close_confirmation_pending()
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(conversation.status_text.contains("close pending"));
    assert!(conversation.status_text.contains("discard unsaved edits"));

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));
    assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
    assert!(!app.planning_draft_editor_ui_state.is_open());

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(
        conversation
            .status_text
            .contains("unsaved in-memory edits were discarded")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_manual_editor_close_warning_can_be_canceled() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-editor-close-cancel-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    open_planning_manual_editor(&mut app);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('#'), KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)));
    assert!(
        app.planning_draft_editor_ui_state
            .is_close_confirmation_pending()
    );

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE,)));
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert!(
        !app.planning_draft_editor_ui_state
            .is_close_confirmation_pending()
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(conversation.status_text.contains("keep editing"));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_manual_editor_invalid_saved_draft_requires_confirmation_before_close() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-editor-close-invalid-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    open_planning_manual_editor(&mut app);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('#'), KeyModifiers::NONE,)));
    assert!(
        app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL,))
    );
    assert!(!app.planning_draft_editor_ui_state.has_dirty_buffers());
    assert!(
        !app.planning_draft_editor_ui_state
            .validation_report()
            .expect("validation report")
            .is_valid()
    );

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)));
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert!(
        app.planning_draft_editor_ui_state
            .is_close_confirmation_pending()
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(conversation.status_text.contains("invalid staged draft"));

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)));
    assert_eq!(app.shell_overlay, ShellOverlay::Hidden);

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(
        conversation
            .status_text
            .contains("invalid staged draft remains in drafts")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_manual_editor_clean_valid_close_remains_immediate() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-editor-close-clean-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    open_planning_manual_editor(&mut app);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)));

    assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
    assert!(!app.planning_draft_editor_ui_state.is_open());

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert!(!conversation.status_text.contains("close pending"));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_manual_editor_promote_copies_active_files_and_refreshes_prompt_context() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-editor-promote-app");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should stay in ready state");
    };
    conversation.cwd = workspace_dir.clone();

    open_planning_manual_editor(&mut app);

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

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_manual_editor_promote_stays_open_when_validation_fails() {
    let (mut app, _) = make_test_app();
    let startup_workspace_dir =
        create_temp_workspace("planning-editor-promote-invalid-startup-app");
    let workspace_dir = create_temp_workspace("planning-editor-promote-invalid-app");
    bootstrap_active_planning_workspace(&startup_workspace_dir);
    let startup_draft_count = count_staged_planning_drafts(&startup_workspace_dir);
    app.startup_state =
        StartupState::Ready(sample_startup_diagnostics(&startup_workspace_dir, true));
    app.conversation_state = ConversationState::ready(ready_conversation());
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should stay in ready state");
    };
    conversation.cwd = workspace_dir.clone();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "planning context",
        "queue ready",
    ));

    open_planning_manual_editor(&mut app);
    assert_eq!(
        count_staged_planning_drafts(&startup_workspace_dir),
        startup_draft_count
    );
    assert_eq!(count_staged_planning_drafts(&workspace_dir), 1);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('#'), KeyModifiers::NONE,)));
    assert!(
        app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL,))
    );

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
    assert_eq!(
        app.planning_init_overlay_ui_state.step(),
        PlanningInitOverlayStep::ManualEditor
    );
    assert!(
        conversation
            .status_text
            .contains("planning draft promote blocked")
    );
    assert_eq!(
        conversation
            .planning_runtime_snapshot
            .preview_status_label(),
        "inactive"
    );
    assert!(
        !std::path::Path::new(&workspace_dir)
            .join(".codex-exec-loop")
            .join("planning")
            .join("directions.toml")
            .exists()
    );
    assert!(
        std::path::Path::new(&startup_workspace_dir)
            .join(".codex-exec-loop")
            .join("planning")
            .join("directions.toml")
            .exists()
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    std::fs::remove_dir_all(startup_workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_detail_overlay_surfaces_llm_assisted_as_disabled() {
    let (mut app, _) = make_test_app();
    app.show_planning_init_overlay();
    app.planning_init_overlay_ui_state.open_detail_selection();
    app.planning_init_overlay_ui_state
        .select_detail(PlanningInitDetailSelection::LlmAssisted);

    let view = build_planning_init_overlay_view(&app);
    let rendered = view
        .option_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("llm-assisted"));
    assert!(rendered.contains("not supported yet"));
}

#[test]
fn planning_mode_selection_uses_vertical_navigation_keys() {
    let (mut app, _) = make_test_app();
    let workspace_dir = create_temp_workspace("planning-mode-selection");
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir, true));
    sync_draft_conversation_to_startup_workspace(&mut app);
    app.show_planning_init_overlay();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE,)));
    assert_eq!(
        app.planning_init_overlay_ui_state.selected_mode(),
        PlanningInitModeSelection::Detail
    );

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE,)));
    assert_eq!(
        app.planning_init_overlay_ui_state.selected_mode(),
        PlanningInitModeSelection::Simple
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

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
            .contains("opened planning initialization selector")
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
            .contains("opened planning workspace controls")
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
    assert!(conversation.status_text.contains("Plan off"));

    app.execute_inline_shell_command_input(
        InlineShellCommandInput::parse(":directions").expect("command should parse"),
    );
    assert_ne!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("app should stay in ready state");
    };
    assert_eq!(
        conversation.status_text,
        "Plan off - initialize with :planning first"
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
        "planning workspace missing; open :planning to initialize it"
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
        "Plan off - turn Plan on in this menu first"
    );
    assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn planning_simple_mode_selection_stages_bootstrap_files_in_current_workspace() {
    use std::collections::HashSet;

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
        .select_detail(super::PlanningInitDetailSelection::LlmAssisted);

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
        super::PlanningInitModeSelection::Detail
    );

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE,)));
    assert_eq!(
        app.planning_init_overlay_ui_state.selected_mode(),
        super::PlanningInitModeSelection::Simple
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn invalid_task_ledger_change_restores_snapshot_and_runs_hidden_planning_repair() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-reconcile-app");
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
    std::fs::write(
        planning_dir.join("task-ledger.json"),
        &bootstrap_artifacts.task_ledger_json,
    )
    .expect("task ledger should write");
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
    conversation.auto_follow_state.template_state.selected_index = 0;
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
    assert!(
        repair_prompt
            .as_deref()
            .is_some_and(|prompt| prompt.contains("planning repair 1/2"))
    );
    assert!(
        repair_prompt
            .as_deref()
            .is_some_and(|prompt| prompt.contains("Validation errors:"))
    );
    assert!(
        repair_prompt
            .as_deref()
            .is_some_and(|prompt| prompt.contains("Rejected candidate excerpt"))
    );
    assert!(
        conversation
            .runtime_notices
            .iter()
            .any(|notice| notice.contains("archived rejected task-ledger"))
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
    conversation.auto_follow_state.template_state.selected_index = 0;
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
        "auto follow-up submitted / turn 1/3 / template: builtin next-task"
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

#[test]
fn proposed_only_refresh_promotes_top_proposal_and_queues_auto_followup() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-proposal-followup-app");
    bootstrap_active_planning_workspace(&workspace_dir);
    enable_queue_idle_review_and_enqueue(&workspace_dir);
    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    std::fs::write(
        planning_dir.join("task-ledger.json"),
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-proposal-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Follow-up option offered in the latest answer.",
      "title": "Draft a Korea-specific Chinese-chef job entry guide",
      "description": "Expand the answer into a Korea-specific hiring guide.",
      "status": "proposed",
      "base_priority": 70,
      "dynamic_priority_delta": 0,
      "priority_reason": "First follow-up branch from the latest answer.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": null,
      "updated_at": "2026-04-13T00:00:00Z"
    },
    {
      "id": "task-proposal-2",
      "direction_id": "general-workstream",
      "direction_relation_note": "Alternate follow-up option offered in the latest answer.",
      "title": "Create a beginner 3-month Chinese-cooking training plan",
      "description": "Turn the answer into a 3-month training plan.",
      "status": "proposed",
      "base_priority": 65,
      "dynamic_priority_delta": 0,
      "priority_reason": "Second follow-up branch from the latest answer.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": null,
      "updated_at": "2026-04-13T00:00:00Z"
    }
  ]
}"#,
    )
    .expect("task ledger should write");

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
            text: "planner refreshed the queue".to_string(),
        },
        ConversationStreamEvent::TurnCompleted {
            turn_id: "planner-turn-1".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ];

    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 0;
    conversation.cwd = workspace_dir.clone();
    conversation.draft_workspace_directory = workspace_dir.clone();
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
    assert!(
        turn_calls[0].contains("Draft a Korea-specific Chinese-chef job entry guide"),
        "auto follow-up prompt should target the promoted proposal: {}",
        turn_calls[0]
    );
    assert_eq!(
        conversation
            .planning_runtime_snapshot
            .queue_head()
            .map(|task| task.task_id.as_str()),
        Some("task-proposal-1"),
        "status={}, notices={:?}",
        conversation.status_text,
        conversation.runtime_notices
    );
    assert!(
        app.planner_worker_panel_state
            .last_host_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("host promoted top follow-up proposal"))
    );
    assert_eq!(
        conversation.status_text,
        "auto follow-up submitted / turn 1/3 / template: builtin next-task"
    );
    assert_eq!(
        app.planner_worker_panel_state.last_queue_summary.as_deref(),
        Some("next task: Draft a Korea-specific Chinese-chef job entry guide")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn repeated_builtin_next_task_refresh_pauses_auto_followup_until_queue_advances() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-repeated-next-task");
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
      "id": "task-repeat-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Current next task.",
      "title": "Rust 입문 8주 커리큘럼 구체화",
      "description": "Expand the roadmap into a week-by-week curriculum.",
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
            text: "planner refreshed the queue".to_string(),
        },
        ConversationStreamEvent::TurnCompleted {
            turn_id: "planner-turn-1".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ];

    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 0;
    conversation.cwd = workspace_dir.clone();
    conversation.draft_workspace_directory = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-main".to_string());
    conversation.last_planning_task_handoff = Some(PlanningTaskHandoff {
        task_id: "task-repeat-1".to_string(),
        task_title: "Rust 입문 8주 커리큘럼 구체화".to_string(),
        direction_id: "general-workstream".to_string(),
        combined_priority: 80,
        updated_at: "2026-04-13T00:00:00Z".to_string(),
        status_label: "ready".to_string(),
    });
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

    thread::sleep(Duration::from_millis(50));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };

    assert!(
        codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .is_empty()
    );
    assert_eq!(
        conversation.status_text,
        "turn completed / auto follow-up paused: planning queue repeated the previous task"
    );
    assert!(
        conversation
            .planning_runtime_snapshot
            .auto_followup_pause_reason()
            .is_some_and(|reason| reason.contains("previously handed-off task"))
    );
    assert!(
        app.planner_worker_panel_state
            .last_host_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("previously handed-off task"))
    );
    assert!(
        app.planner_worker_panel_state
            .last_prompt
            .as_deref()
            .is_some_and(|prompt| prompt.contains("planning worker refresh 입니다."))
    );
    assert_eq!(
        app.planner_worker_panel_state
            .last_operation_label
            .as_deref(),
        Some("refresh")
    );
    assert_eq!(
        app.planner_worker_panel_state.last_response.as_deref(),
        Some("planner refreshed the queue")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn refreshed_queue_head_with_same_task_id_but_new_timestamp_still_submits_auto_followup() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-repeated-next-task-updated");
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
      "id": "task-repeat-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Current next task.",
      "title": "Rust 입문 8주 커리큘럼 구체화",
      "description": "Expand the roadmap into a week-by-week curriculum.",
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
            text: "planner refreshed the queue with an updated task".to_string(),
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
      "id": "task-repeat-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Current next task was updated after the latest reply.",
      "title": "Rust 입문 8주 커리큘럼 구체화",
      "description": "Expand the roadmap into a week-by-week curriculum.",
      "status": "ready",
      "base_priority": 80,
      "dynamic_priority_delta": 0,
      "priority_reason": "Current top executable task.",
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

    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 0;
    conversation.cwd = workspace_dir.clone();
    conversation.draft_workspace_directory = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-main".to_string());
    conversation.last_planning_task_handoff = Some(PlanningTaskHandoff {
        task_id: "task-repeat-1".to_string(),
        task_title: "Rust 입문 8주 커리큘럼 구체화".to_string(),
        direction_id: "general-workstream".to_string(),
        combined_priority: 80,
        updated_at: "2026-04-13T00:00:00Z".to_string(),
        status_label: "ready".to_string(),
    });
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
    assert_eq!(
        conversation.status_text,
        "auto follow-up submitted / turn 1/3 / template: builtin next-task"
    );
    assert!(
        conversation
            .planning_runtime_snapshot
            .auto_followup_pause_reason()
            .is_none()
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn builtin_next_task_refresh_passes_full_latest_agent_reply_to_hidden_planner_prompt() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-refresh-full-latest-reply");
    bootstrap_active_planning_workspace(&workspace_dir);
    enable_queue_idle_review_and_enqueue(&workspace_dir);

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
            text: "planner refreshed the queue".to_string(),
        },
        ConversationStreamEvent::TurnCompleted {
            turn_id: "planner-turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ];

    let latest_reply = [
        "시험 최소 범위에 맞추면 아래 목차가 깔끔합니다.",
        "",
        "**강의명**",
        "`CKA 합격을 위한 쿠버네티스 네트워크 최소 핵심`",
        "",
        "1. 강의 소개: CKA에서 네트워크가 왜 중요한가, 어디까지 알면 충분한가",
        "2. 네트워크 기초 15분 압축: IP, Port, TCP/UDP, CIDR, DNS만 빠르게 정리",
        "3. 쿠버네티스 네트워크의 3가지 기본 원칙: Pod IP, Pod 간 통신, Node 간 통신 관점 이해",
        "4. Pod 네트워크 이해: Pod IP가 붙는 방식, 같은 노드와 다른 노드 간 통신 흐름, CNI는 무엇인가",
        "5. Service 핵심: ClusterIP, NodePort, LoadBalancer 차이와 시험에서 보는 포인트",
        "6. Service가 실제로 연결되는 방식: selector, endpoints, kube-proxy를 아주 얕고 실전적으로 이해",
        "7. 클러스터 DNS: CoreDNS, Service 이름으로 통신하는 방식, FQDN과 네임스페이스 개념",
        "8. Ingress 기초: Ingress가 필요한 이유, Service와의 관계, 시험에서 알아야 할 정도만",
        "9. NetworkPolicy 핵심: ingress/egress, allow 기준 사고방식, 자주 나오는 정책 해석법",
        "10. 트러블슈팅 패턴: Pod to Pod, Pod to Service, DNS 문제를 어떤 순서로 확인할지",
        "11. 시험용 필수 명령어: kubectl get svc, kubectl get endpoints, kubectl describe, nslookup, dig, curl, ping 활용",
        "12. 실습 1: Pod 간 통신 확인",
        "13. 실습 2: Service 연결 확인과 endpoint 문제 찾기",
        "14. 실습 3: DNS 조회 실패 문제 해결",
        "15. 실습 4: NetworkPolicy 적용 전후 통신 비교",
        "16. 시험 직전 암기 포인트 정리: 꼭 기억할 개념, 자주 헷갈리는 차이점, 문제 풀이 순서",
        "",
        "빼도 되는 내용도 정해두면 강의가 더 선명합니다.",
        "",
        "- OSI 7계층 상세 설명",
        "- 라우팅 프로토콜 심화",
        "- iptables/IPVS 내부 동작 심화",
        "- CNI 플러그인 구현 디테일",
        "- BGP, VXLAN 심화",
    ]
    .join("\n");
    let latest_user_request =
        "CKA 네트워크 강의를 만들 건데 시험 최소 범위에 맞는 목차부터 정리해줘";

    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 0;
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
        latest_reply.clone(),
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

    let mut hidden_prompts = Vec::new();
    for _ in 0..20 {
        hidden_prompts = codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .collect::<Vec<_>>();
        if !hidden_prompts.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(hidden_prompts.len(), 1);
    assert!(hidden_prompts[0].contains("latest operator request:"));
    assert!(hidden_prompts[0].contains(latest_user_request));
    assert!(hidden_prompts[0].contains("main session latest reply:"));
    assert!(hidden_prompts[0].contains("5. Service 핵심"));
    assert!(hidden_prompts[0].contains("- BGP, VXLAN 심화"));
    assert!(!hidden_prompts[0].contains("worker received full text"));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn stale_planning_repair_state_does_not_queue_visible_retry() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 1;
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
fn stale_repair_state_does_not_change_hidden_repair_prompt_shape() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-repair-still-invalid");
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
    conversation.auto_follow_state.template_state.selected_index = 1;
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

    assert!(repair_prompt.is_some());
    assert!(repair_prompt.as_deref().is_some_and(|prompt| {
        !prompt.contains(
            "직전 repair 시도에서 `task-ledger.json` 을 수정했지만 여전히 유효하지 않습니다",
        )
    }));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn buffered_manual_input_does_not_pause_hidden_planning_repair() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-repair-manual-buffer");
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
    conversation.auto_follow_state.template_state.selected_index = 1;
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
            .any(|prompt| prompt.contains("planning repair 1/2"))
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
fn automation_off_stops_hidden_planning_repair_and_auto_followup() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("automation-off-no-hidden-repair");
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
    conversation.auto_follow_state.enabled = false;
    conversation.auto_follow_state.template_state.selected_index = 1;
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
        "turn completed / automation stopped: off"
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
    conversation.auto_follow_state.template_state.selected_index = 0;
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
        "auto follow-up submitted / turn 1/3 / template: builtin next-task"
    );
    assert_eq!(
        conversation
            .last_auto_followup_activity
            .as_ref()
            .map(|activity| activity.summary.as_str()),
        Some("submitted auto turn 1/3")
    );

    app.start_turn_submission();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(app.shell_overlay, ShellOverlay::Queue);
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
    conversation.auto_follow_state.template_state.selected_index = 0;
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
        "auto follow-up submitted / turn 1/3 / template: builtin next-task"
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn stale_exhausted_repair_state_does_not_block_hidden_repair() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-repair-exhausted");
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
    conversation.auto_follow_state.template_state.selected_index = 1;
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
            .any(|(_, prompt)| prompt.contains("planning repair 1/2"))
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

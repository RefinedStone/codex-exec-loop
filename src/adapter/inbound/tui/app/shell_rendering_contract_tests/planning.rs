use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::application::service::planning::{
    PlanningBootstrapMode, PlanningDraftEditorFile, PlanningDraftEditorSession,
    PlanningInitStageResult,
};
use crate::domain::planning::PlanningValidationReport;
use crate::domain::startup_diagnostics::StartupDiagnostics;
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

use super::*;

#[test]
fn inline_planning_init_inspection_renders_existing_auto_seeded_workspace_inside_shell_frame() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    let workspace_dir = std::env::temp_dir().join(format!(
        "codex-exec-loop-render-planning-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace_dir).expect("temp workspace should be created");
    let workspace_dir = workspace_dir.to_string_lossy().to_string();
    app.startup_state = StartupState::Ready(StartupDiagnostics {
        cwd: workspace_dir.clone(),
        codex_binary_ok: true,
        codex_binary_detail: "codex".to_string(),
        workspace_ok: true,
        workspace_path: workspace_dir.clone(),
        workspace_detail: "workspace found".to_string(),
        attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
        initialize_ok: true,
        initialize_detail: "app-server initialize ok".to_string(),
        account_ok: true,
        account_detail: "account ok".to_string(),
        warnings: Vec::new(),
        schema_snapshot: "snapshot.json".to_string(),
    });
    app.sync_draft_shell_workspace(&workspace_dir);
    app.show_planning_init_overlay();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline planning inspection render succeeds");

    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Planning / inline inspection"));
    assert!(rendered.contains("planning state:"));
    assert!(rendered.contains("queue idle policy:"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn inline_planning_manual_editor_renders_files_and_editor_panels() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell_overlay = ShellOverlay::PlanningInit;
    app.planning_init_overlay_ui_state.open_manual_editor();
    app.planning_draft_editor_ui_state
        .open_session(sample_planning_editor_session());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline planning editor render succeeds");

    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Planning Draft"));
    assert!(rendered.contains("Files"));
    assert!(rendered.contains("result-output.md"));
    assert!(rendered.contains("controls: Ctrl+S saves and validates"));
    assert!(rendered.contains("Ctrl+P saves and promotes active planning"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn prepare_render_state_syncs_inline_planning_editor_scroll_before_render() {
    let mut app = make_test_app();
    app.shell_overlay = ShellOverlay::PlanningInit;
    app.planning_init_overlay_ui_state.open_manual_editor();
    app.planning_draft_editor_ui_state
        .open_session(sample_long_planning_editor_session());
    for _ in 0..10 {
        app.planning_draft_editor_ui_state.move_cursor_down();
    }

    assert_eq!(
        app.planning_draft_editor_ui_state
            .selected_buffer()
            .expect("buffer")
            .editor_scroll(),
        0
    );

    let area = Rect::new(0, 0, 96, 28);
    prepare_render_state(&mut app, ShellFrontendMode::InlineMainBuffer, area);

    let tail_lines = build_inline_tail_lines(&app);
    let inspection_area = build_inline_terminal_flow_layout(&app, area, &tail_lines)[0];
    let editor_content_height = inspection_area
        .height
        .saturating_sub(14)
        .max(6)
        .saturating_sub(1)
        .max(1);
    let view = build_planning_draft_editor_overlay_view(&app, editor_content_height)
        .expect("planning draft editor overlay view should be available");

    assert!(view.editor_scroll > 0);
    assert!(view.editor_cursor_offset.expect("cursor").1 < editor_content_height);
}

#[test]
fn inline_planning_simple_review_renders_promote_and_edit_actions() {
    let mut app = make_test_app();
    app.shell_overlay = ShellOverlay::PlanningInit;
    app.planning_init_overlay_ui_state
        .open_simple_review(PlanningInitStageResult {
            mode: PlanningBootstrapMode::Simple,
            draft_name: "bootstrap-1".to_string(),
            draft_directory: "/tmp/bootstrap-1".to_string(),
            staged_files: Vec::new(),
            staged_file_count: 4,
            validation_report: PlanningValidationReport::default(),
        });

    let view = build_planning_init_overlay_view(&app);
    let header = view
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let options = view
        .option_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let status = view
        .status_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let keys = view
        .key_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(header.contains("Planning Setup / operator inspection"));
    assert!(header.contains("Simple mode review"));
    assert!(options.contains("bootstrap-1"));
    assert!(options.contains("advanced path"));
    assert!(status.contains("turn budget: 20"));
    assert!(status.contains("advanced action: D opens detail-mode authoring"));
    assert!(keys.contains("Enter or Ctrl+P promotes the staged scaffold."));
    assert!(keys.contains("Ctrl+L edits turn budget."));
    assert!(keys.contains("Ctrl+E inspects or edits the draft."));
}

#[test]
fn inline_planning_simple_review_renders_editing_specific_key_guidance() {
    let mut app = make_test_app();
    app.shell_overlay = ShellOverlay::PlanningInit;
    app.planning_init_overlay_ui_state
        .open_simple_review(PlanningInitStageResult {
            mode: PlanningBootstrapMode::Simple,
            draft_name: "bootstrap-1".to_string(),
            draft_directory: "/tmp/bootstrap-1".to_string(),
            staged_files: Vec::new(),
            staged_file_count: 4,
            validation_report: PlanningValidationReport::default(),
        });
    app.start_max_auto_turns_edit();
    app.followup_overlay_ui_state.max_auto_turns_editor.buffer = "12".to_string();

    let view = build_planning_init_overlay_view(&app);
    let keys = view
        .key_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(keys.contains("next action: type the new turn budget directly."));
    assert!(keys.contains("controls: Enter saves"));
    assert!(keys.contains("validation: use a whole number greater than 0, or type infinite."));
    assert!(!keys.contains("promote staged scaffold"));
}

#[test]
fn inline_planning_manual_editor_renders_close_confirmation_guidance() {
    let mut app = make_test_app();
    app.shell_overlay = ShellOverlay::PlanningInit;
    app.planning_init_overlay_ui_state.open_manual_editor();
    app.planning_draft_editor_ui_state
        .open_session(sample_planning_editor_session());
    app.planning_draft_editor_ui_state.insert_character('#');
    let _ = app.planning_draft_editor_ui_state.request_close();

    let view = build_planning_draft_editor_overlay_view(&app, 8)
        .expect("planning draft editor overlay view should be available");
    let status = view
        .status_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let keys = view
        .key_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(status.contains("close pending"));
    assert!(keys.contains("controls: Enter, Esc, or Ctrl+C confirms close"));
    assert!(keys.contains("n keeps editing"));
}

fn sample_long_planning_editor_session() -> PlanningDraftEditorSession {
    PlanningDraftEditorSession {
        draft_name: "bootstrap-test".to_string(),
        draft_directory: "/tmp/root/.codex-exec-loop/planning/drafts/bootstrap-test".to_string(),
        editable_files: vec![PlanningDraftEditorFile {
            active_path: ".codex-exec-loop/planning/result-output.md".to_string(),
            staged_path:
                "/tmp/root/.codex-exec-loop/planning/drafts/bootstrap-test/result-output.md"
                    .to_string(),
            body: (1..=12)
                .map(|index| format!("line {index}"))
                .collect::<Vec<_>>()
                .join("\n"),
        }],
        validation_report: Default::default(),
    }
}

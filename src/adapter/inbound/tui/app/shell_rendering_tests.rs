use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use ratatui::Terminal;
use ratatui::TerminalOptions;
use ratatui::Viewport;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use ratatui::style::Color;

use super::*;
use crate::adapter::inbound::tui::app::shell_presentation::format_conversation_lines_with_debug;
use crate::adapter::outbound::filesystem_planning_workspace_adapter::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::codex_app_server_port::{
    AppServerStartupContext, CodexAppServerPort,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::planning::PlanningBootstrapMode;
use crate::application::service::planning::PlanningServices;
use crate::application::service::planning::{
    PlanningDraftEditorFile, PlanningDraftEditorSession, PlanningInitStageResult,
};
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::planning::PlanningValidationReport;
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;
use crate::domain::startup_diagnostics::StartupDiagnostics;

#[test]
fn centered_rect_clamps_percentages_above_hundred() {
    let area = Rect::new(4, 2, 80, 24);

    assert_eq!(centered_rect(140, 120, area), area);
}

#[test]
fn transcript_debug_detail_is_rendered_in_gray_only_when_enabled() {
    let message = ConversationMessage::new(
        ConversationMessageKind::User,
        "다음 queued task 1개를 이어서 진행합니다.",
        None,
        None,
    )
    .with_display_label("Auto Follow-up")
    .with_debug_detail("planner session: refresh  |  state: refresh ok");

    let without_debug = format_conversation_lines(std::slice::from_ref(&message));
    assert!(
        !without_debug
            .iter()
            .any(|line| line.to_string().contains("planner session"))
    );

    let with_debug = format_conversation_lines_with_debug(&[message], true);
    let detail_line = with_debug
        .iter()
        .find(|line: &&Line<'static>| line.to_string().contains("planner session"))
        .expect("debug transcript should include the planner detail line");

    assert_eq!(
        detail_line.to_string(),
        "  planner session: refresh  |  state: refresh ok"
    );
    assert_eq!(detail_line.spans[0].style.fg, Some(Color::Gray));
}

#[test]
fn inline_main_buffer_rendering_avoids_box_borders() {
    let mut terminal = inline_terminal(80, 24);
    let mut app = make_test_app();
    append_stable_history_message(&mut app, "stable history should stay above the live region");

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");

    let rendered = format!("{}", terminal.backend());

    assert!(!rendered.contains("Shell / Ctrl+t new draft"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("Controls / shell shortcuts and live status"));
    assert!(!rendered.contains("Prompt / ready"));
    assert!(rendered.contains(
        "thread: new draft  |  turn: idle  |  auto: on/idle  |  done: 0/3  |  in: draft"
    ));
    assert!(!rendered.contains("stable history should stay above the live region"));
    assert!(!rendered.contains("No messages in this thread yet."));
    assert!(!rendered.contains("┌"));
    assert!(!rendered.contains("│"));
}

#[test]
fn inline_main_buffer_tail_starts_at_top_of_viewport_after_history() {
    let mut terminal = inline_terminal(80, 24);
    let mut app = make_test_app();
    append_stable_history_message(&mut app, "latest reply should stay in scrollback");

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");

    let rendered = format!("{}", terminal.backend());
    let first_non_empty_line = rendered
        .lines()
        .find(|line| !line.trim().is_empty())
        .expect("inline viewport should contain visible tail text");
    let first_non_empty_line = first_non_empty_line.trim_matches('"');

    assert!(first_non_empty_line.starts_with("thread: new draft  |  turn: idle"));
}

#[test]
fn inline_main_buffer_tail_frame_does_not_render_startup_ascii_art_transiently() {
    let mut terminal = inline_terminal(80, 24);
    let mut app = make_test_app();
    app.show_startup_ascii_art = true;
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");

    let rendered = format!("{}", terminal.backend());

    assert!(!rendered.contains(".:  .::    .::  .::.: .:::   .::"));
    assert!(!rendered.contains(".::.::  .::   .::    .::  .::   .::"));
    assert!(rendered.contains("startup: startup checks ready"));
    assert!(rendered.contains("workspace: /tmp/root"));
    assert!(rendered.contains("current state: ready"));
    assert!(rendered.contains("startup checks: codex ready"));
    assert!(rendered.contains("workspace ready"));
    assert!(rendered.contains("app-server ready"));
    assert!(rendered.contains("conversation"));
    assert!(rendered.contains("first reply appears here after you send the opening prompt"));
    assert!(rendered.contains("operator prompt: ready for the opening prompt"));
}

#[test]
fn inline_main_buffer_clears_stale_live_tail_rows_after_turn_finishes() {
    let mut terminal = inline_terminal(80, 24);
    let mut app = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());
    conversation.active_turn_started_at = Some(std::time::Instant::now() - Duration::from_secs(5));
    conversation.live_agent_message = Some(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "ghost line should disappear".to_string(),
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("first inline render succeeds");

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should stay in a ready conversation state");
    };
    conversation.live_agent_message = None;
    conversation.active_turn_id = None;
    conversation.active_turn_started_at = None;
    conversation.input_state = ConversationInputState::ReadyToContinue;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("second inline render succeeds");

    let rendered = format!("{}", terminal.backend());

    assert!(!rendered.contains("ghost line should disappear"));
}

#[test]
fn inline_main_buffer_clears_stale_tail_rows_when_overlay_opens() {
    let mut terminal = inline_terminal(80, 24);
    let mut app = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());
    conversation.active_turn_started_at = Some(std::time::Instant::now() - Duration::from_secs(5));
    conversation.live_agent_message = Some(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "overlay ghost line should disappear".to_string(),
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("first inline render succeeds");

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should stay in a ready conversation state");
    };
    conversation.live_agent_message = None;
    conversation.active_turn_id = None;
    conversation.active_turn_started_at = None;
    conversation.input_state = ConversationInputState::ReadyToContinue;
    app.shell_overlay = ShellOverlay::Startup;
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("overlay inline render succeeds");

    let rendered = format!("{}", terminal.backend());

    assert!(!rendered.contains("overlay ghost line should disappear"));
}

#[test]
fn inline_render_positions_cursor_on_empty_prompt_line() {
    let mut terminal = inline_terminal(80, 24);
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");

    terminal
        .backend_mut()
        .assert_cursor_position(Position::new(2, 11));
}

#[test]
fn inline_queue_overlay_rendering_shows_compact_sections() {
    let mut terminal = inline_terminal(80, 24);
    let mut app = make_test_app();
    append_stable_history_message(&mut app, "stable history stays visible above the queue");
    app.shell_overlay = ShellOverlay::Queue;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("queue render succeeds");

    let rendered = format!("{}", terminal.backend());

    assert!(rendered.contains("Ready Queue"));
    assert!(rendered.contains("Proposals"));
}

fn inline_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    Terminal::with_options(
        TestBackend::new(width, height),
        TerminalOptions {
            viewport: Viewport::Inline(INLINE_VIEWPORT_HEIGHT),
        },
    )
    .expect("inline test terminal")
}

#[test]
fn inline_startup_inspection_replaces_transcript_panel() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell_overlay = ShellOverlay::Startup;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline inspection render succeeds");

    let rendered = format!("{}", terminal.backend());

    assert!(rendered.contains("Startup Checks / operator inspection"));
    assert!(rendered.contains("Startup Checks"));
    assert!(rendered.contains("Current State, Cause, and Next Action"));
    assert!(rendered.contains("Warnings Requiring Review"));
    assert!(rendered.contains("Operator Actions"));
    assert!(rendered.contains("current state: ready"));
    assert!(rendered.contains("cause: codex, workspace, app-server"));
    assert!(rendered.contains("next action: continue in the shell"));
    assert!(rendered.contains("schema snapshot: snapshot.json"));
    assert!(!rendered.contains("shell inspection"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_sessions_inspection_renders_browser_panels_without_popup_frame() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.session_state = SessionState::Ready(RecentSessions {
        items: vec![sample_session("thread-1"), sample_session("thread-2")],
        warnings: vec!["cache is stale".to_string()],
        next_cursor: None,
    });
    app.shell_overlay = ShellOverlay::Sessions;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline session inspection render succeeds");

    let rendered = format!("{}", terminal.backend());

    assert!(rendered.contains("Recent Sessions / operator inspection"));
    assert!(rendered.contains("Session List"));
    assert!(rendered.contains("Selected Session Detail"));
    assert!(rendered.contains("Warnings Requiring Review"));
    assert!(rendered.contains("Operator Actions"));
    assert!(!rendered.contains("shell inspection"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_followup_inspection_renders_preview_inside_shell_frame() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.show_automation_overlay();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline followup inspection render succeeds");

    let rendered = format!("{}", terminal.backend());

    assert!(rendered.contains("Automation Controls / operator inspection"));
    assert!(rendered.contains("Automation Controls"));
    assert!(rendered.contains("Rendered Next-Turn Prompt"));
    assert!(rendered.contains("Operator Status"));
    assert!(rendered.contains("Operator Actions"));
    assert!(rendered.contains("automation state: on"));
    assert!(!rendered.contains("shell inspection"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_planning_init_inspection_renders_selector_inside_shell_frame() {
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

    let rendered = format!("{}", terminal.backend());

    assert!(rendered.contains("Planning Setup / operator inspection"));
    assert!(rendered.contains("Current State, Cause, and Next Action"));
    assert!(rendered.contains("Available Options"));
    assert!(rendered.contains("Operator Status"));
    assert!(rendered.contains("Operator Actions"));
    assert!(rendered.contains("simple mode"));
    assert!(rendered.contains("detail mode"));
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

    let rendered = format!("{}", terminal.backend());

    assert!(rendered.contains("Planning Draft / operator inspection"));
    assert!(rendered.contains("Draft Files"));
    assert!(rendered.contains("Operator Status"));
    assert!(rendered.contains("Operator Actions"));
    assert!(rendered.contains("staged draft:"));
    assert!(rendered.contains("current file:"));
    assert!(rendered.contains("validation state: ok"));
    assert!(rendered.contains("directions.toml"));
    assert!(rendered.contains("Ctrl+S saves and validates"));
    assert!(rendered.contains("Ctrl+P saves and promotes active planning"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn popup_helper_titles_use_operator_facing_copy() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    terminal
        .draw(|frame| draw_startup_overlay(frame, &app))
        .expect("startup popup render succeeds");
    let startup_rendered = format!("{}", terminal.backend());
    assert!(startup_rendered.contains("Startup Checks / operator inspection"));
    assert!(startup_rendered.contains("Current State, Cause, and Next Action"));
    assert!(startup_rendered.contains("Warnings Requiring Review"));
    assert!(startup_rendered.contains("Operator Actions"));

    app.session_state = SessionState::Ready(RecentSessions {
        items: vec![sample_session("thread-1"), sample_session("thread-2")],
        warnings: vec!["cache is stale".to_string()],
        next_cursor: None,
    });

    terminal
        .draw(|frame| draw_session_overlay(frame, &mut app))
        .expect("session popup render succeeds");
    let session_rendered = format!("{}", terminal.backend());
    assert!(session_rendered.contains("Recent Sessions / operator inspection"));
    assert!(session_rendered.contains("Session List"));
    assert!(session_rendered.contains("Selected Session Detail"));
    assert!(session_rendered.contains("Warnings Requiring Review"));
    assert!(session_rendered.contains("Operator Actions"));

    app.show_automation_overlay();
    terminal
        .draw(|frame| draw_automation_overlay(frame, &mut app))
        .expect("automation popup render succeeds");
    let automation_rendered = format!("{}", terminal.backend());
    assert!(automation_rendered.contains("Automation Controls / operator inspection"));
    assert!(automation_rendered.contains("Automation Controls"));
    assert!(automation_rendered.contains("Rendered Next-Turn Prompt"));
    assert!(automation_rendered.contains("Operator Status"));
    assert!(automation_rendered.contains("Operator Actions"));
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
    assert!(status.contains("turn budget: 3"));
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
    assert!(keys.contains("validation: use a whole number between 1 and 50."));
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

struct FakeCodexAppServerPort;

impl CodexAppServerPort for FakeCodexAppServerPort {
    fn load_startup_context(&self) -> Result<AppServerStartupContext> {
        Ok(AppServerStartupContext {
            initialize_detail: "ok".to_string(),
            account_detail: "ok".to_string(),
            account_ok: true,
            warnings: Vec::new(),
        })
    }

    fn load_recent_sessions(&self, _limit: usize) -> Result<RecentSessions> {
        Ok(RecentSessions {
            items: Vec::new(),
            warnings: Vec::new(),
            next_cursor: None,
        })
    }

    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        Ok(ConversationSnapshot {
            thread_id: thread_id.to_string(),
            title: "Loaded thread".to_string(),
            cwd: "/tmp/root".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
        })
    }

    fn run_new_thread_stream(
        &self,
        _cwd: &str,
        _prompt: &str,
        _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        Ok(())
    }

    fn run_turn_stream(
        &self,
        _thread_id: &str,
        _prompt: &str,
        _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        Ok(())
    }
}

fn make_test_app() -> NativeTuiApp {
    let codex_port = Arc::new(FakeCodexAppServerPort);
    let mut app = NativeTuiApp::new(
        StartupService::new(codex_port.clone()),
        SessionService::new(codex_port.clone()),
        ConversationService::new(codex_port),
        PlanningServices::from_workspace_port(Arc::new(FilesystemPlanningWorkspaceAdapter::new())),
    );
    app.show_startup_ascii_art = false;
    app
}

fn sample_startup_diagnostics() -> StartupDiagnostics {
    StartupDiagnostics {
        cwd: "/tmp/root".to_string(),
        codex_binary_ok: true,
        codex_binary_detail: "codex".to_string(),
        workspace_ok: true,
        workspace_path: "/tmp/root".to_string(),
        workspace_detail: "workspace found".to_string(),
        initialize_ok: true,
        initialize_detail: "app-server initialize ok".to_string(),
        account_ok: true,
        account_detail: "account ok".to_string(),
        warnings: Vec::new(),
        schema_snapshot: "snapshot.json".to_string(),
    }
}

fn sample_session(id: &str) -> SessionSummary {
    SessionSummary {
        id: id.to_string(),
        name: Some(format!("Session {id}")),
        preview: "Preview line".to_string(),
        cwd: "/tmp/root".to_string(),
        source: "native".to_string(),
        model_provider: "openai".to_string(),
        updated_at_epoch: 1_700_000_000,
        status_type: "ready".to_string(),
        path: format!("/tmp/root/{id}.json"),
        git_branch: Some("feature/demo".to_string()),
    }
}

fn append_stable_history_message(app: &mut NativeTuiApp, text: &str) {
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        text.to_string(),
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.refresh_conversation_lines();
}

fn sample_planning_editor_session() -> PlanningDraftEditorSession {
    PlanningDraftEditorSession {
        draft_name: "bootstrap-test".to_string(),
        draft_directory: "/tmp/root/.codex-exec-loop/planning/drafts/bootstrap-test".to_string(),
        editable_files: vec![
            PlanningDraftEditorFile {
                active_path: ".codex-exec-loop/planning/directions.toml".to_string(),
                staged_path:
                    "/tmp/root/.codex-exec-loop/planning/drafts/bootstrap-test/directions.toml"
                        .to_string(),
                body: "version = 1\n".to_string(),
            },
            PlanningDraftEditorFile {
                active_path: ".codex-exec-loop/planning/task-ledger.json".to_string(),
                staged_path:
                    "/tmp/root/.codex-exec-loop/planning/drafts/bootstrap-test/task-ledger.json"
                        .to_string(),
                body: "{\n  \"version\": 1,\n  \"tasks\": []\n}".to_string(),
            },
            PlanningDraftEditorFile {
                active_path: ".codex-exec-loop/planning/result-output.md".to_string(),
                staged_path:
                    "/tmp/root/.codex-exec-loop/planning/drafts/bootstrap-test/result-output.md"
                        .to_string(),
                body: "# result\n".to_string(),
            },
        ],
        validation_report: Default::default(),
    }
}

fn sample_long_planning_editor_session() -> PlanningDraftEditorSession {
    PlanningDraftEditorSession {
        draft_name: "bootstrap-test".to_string(),
        draft_directory: "/tmp/root/.codex-exec-loop/planning/drafts/bootstrap-test".to_string(),
        editable_files: vec![PlanningDraftEditorFile {
            active_path: ".codex-exec-loop/planning/directions.toml".to_string(),
            staged_path:
                "/tmp/root/.codex-exec-loop/planning/drafts/bootstrap-test/directions.toml"
                    .to_string(),
            body: (1..=12)
                .map(|index| format!("line {index}"))
                .collect::<Vec<_>>()
                .join("\n"),
        }],
        validation_report: Default::default(),
    }
}

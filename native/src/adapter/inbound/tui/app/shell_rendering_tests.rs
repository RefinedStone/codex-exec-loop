use std::sync::Arc;

use anyhow::Result;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;

use super::*;
use crate::application::port::outbound::codex_app_server_port::{
    AppServerStartupContext, CodexAppServerPort,
};
use crate::application::port::outbound::followup_template_port::{
    FollowupTemplatePort, WorkspaceFollowupTemplateRecord,
};
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::followup_template_service::FollowupTemplateService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;
use crate::domain::startup_diagnostics::StartupDiagnostics;

#[test]
fn centered_rect_clamps_percentages_above_hundred() {
    let area = Rect::new(4, 2, 80, 24);

    assert_eq!(centered_rect(140, 120, area), area);
}

#[test]
fn inline_main_buffer_rendering_avoids_box_borders() {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
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
    assert!(rendered.contains("thread: new draft  |  turn: idle  |  auto: on (0/3)"));
    assert!(!rendered.contains("stable history should stay above the live region"));
    assert!(!rendered.contains("No messages in this thread yet."));
    assert!(!rendered.contains("┌"));
    assert!(!rendered.contains("│"));
}

#[test]
fn inline_render_positions_cursor_on_empty_prompt_line() {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");

    terminal
        .backend_mut()
        .assert_cursor_position(Position::new(2, 3));
}

#[test]
fn alternate_screen_rendering_keeps_bordered_frame() {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
    let mut app = make_test_app();
    append_stable_history_message(&mut app, "stable history stays inside the framed renderer");

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::AlternateScreen))
        .expect("alternate render succeeds");

    let rendered = format!("{}", terminal.backend());

    assert!(rendered.contains("Shell / Ctrl+t new draft"));
    assert!(rendered.contains("Transcript"));
    assert!(rendered.contains("stable history stays inside the framed renderer"));
    assert!(rendered.contains("┌"));
    assert!(rendered.contains("│"));
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

    assert!(rendered.contains("Diagnostics / inline inspection"));
    assert!(rendered.contains("Checks"));
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

    assert!(rendered.contains("Recent Sessions / inline inspection"));
    assert!(rendered.contains("Threads"));
    assert!(rendered.contains("Selected Session"));
    assert!(rendered.contains("Session Warnings"));
    assert!(!rendered.contains("shell inspection"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_followup_inspection_renders_preview_inside_shell_frame() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.show_followup_template_overlay();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline followup inspection render succeeds");

    let rendered = format!("{}", terminal.backend());

    assert!(rendered.contains("Follow-Up Templates / inline inspection"));
    assert!(rendered.contains("Template List"));
    assert!(rendered.contains("Preview"));
    assert!(rendered.contains("auto follow-up: on"));
    assert!(!rendered.contains("shell inspection"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
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

struct FakeFollowupTemplatePort;

impl FollowupTemplatePort for FakeFollowupTemplatePort {
    fn load_workspace_templates(
        &self,
        _workspace_dir: &str,
    ) -> Result<Vec<WorkspaceFollowupTemplateRecord>> {
        Ok(Vec::new())
    }
}

fn make_test_app() -> NativeTuiApp {
    let codex_port = Arc::new(FakeCodexAppServerPort);
    let followup_port = Arc::new(FakeFollowupTemplatePort);
    NativeTuiApp::new(
        StartupService::new(codex_port.clone()),
        SessionService::new(codex_port.clone()),
        ConversationService::new(codex_port),
        FollowupTemplateService::new(followup_port),
    )
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

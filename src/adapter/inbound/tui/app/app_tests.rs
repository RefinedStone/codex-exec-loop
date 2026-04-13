use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;

use super::conversation_model::PlanningRepairState;
use super::shell_presentation::{
    build_inline_prompt_cursor_offset, build_input_prompt_cursor_offset,
};
use super::{
    ActiveTurnPlanningCapture, AutoFollowRuntimePhase, AutoFollowState, AutoFollowupSubmitContext,
    BackgroundMessage, ConversationInputState, ConversationMessage, ConversationMessageKind,
    ConversationRuntimeEffect, ConversationRuntimeEvent, ConversationState, ConversationViewModel,
    DEFAULT_AUTO_FOLLOW_MAX_TURNS, DEFAULT_AUTO_FOLLOW_STOP_KEYWORD, ExitConfirmationState,
    FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP, GithubReviewPollingState, InlineShellCommand,
    InlineShellCommandInput, MAX_COMPOSER_HEIGHT, NativeTuiApp, PlannerVisibility,
    PlannerWorkerPanelState, PlannerWorkerStatus, PlanningInitOverlayStep, PromptOrigin,
    RecordedAutoFollowupActivity, SessionOverlayUiState, SessionState, ShellActionAvailability,
    ShellFrontendMode, ShellOverlay, StartupState, TurnActivityState,
    build_conversation_shell_frame_view, build_conversation_shell_view,
    build_followup_template_overlay_view, build_followup_template_preview_lines,
    build_followup_template_status_lines, build_inline_tail_lines, build_input_title,
    build_planning_init_overlay_view, build_queue_overlay_view, build_ready_input_lines,
    build_session_overlay_view, build_shell_footer_lines, build_startup_overlay_view,
    build_status_title, build_transcript_panel_view, build_transcript_title,
    format_conversation_lines, shell_layout, startup_ascii_art_enabled_from_value,
};
use crate::adapter::inbound::tui::app::test_helpers::{
    sample_planning_runtime_snapshot, sample_proposal_only_planning_runtime_snapshot,
};
use crate::adapter::outbound::app_server_planning_worker_adapter::{
    AppServerPlanningWorkerAdapter, PlanningThreadLauncher,
};
use crate::adapter::outbound::filesystem_planning_workspace_adapter::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::codex_app_server_port::{
    AppServerStartupContext, CodexAppServerPort,
};
use crate::application::port::outbound::followup_template_port::{
    FollowupTemplatePort, WorkspaceFollowupTemplateRecord,
};
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::followup_template_service::FollowupTemplateService;
use crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot;
use crate::application::service::planning_reconciliation_service::{
    PlanningExecutionSnapshot, PlanningRepairRequest,
};
use crate::application::service::planning_runtime_facade_service::PlanningTaskHandoff;
use crate::application::service::planning_services::PlanningServices;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationSnapshot,
    ConversationStreamEvent,
};
use crate::domain::followup_template::{
    FollowupTemplateCatalog, FollowupTemplateDefinition, FollowupTemplateSource,
};
use crate::domain::github_review::{
    GithubPullRequestActivityEvent, GithubPullRequestActivityKind,
    GithubPullRequestActivitySnapshot, GithubPullRequestPollResult, GithubPullRequestTarget,
};
use crate::domain::planning::{DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, TASK_LEDGER_FILE_PATH};
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;
use crate::domain::startup_diagnostics::StartupDiagnostics;

#[derive(Default)]
struct FakeCodexAppServerPort {
    new_thread_calls: Mutex<Vec<(String, String)>>,
    turn_calls: Mutex<Vec<(String, String)>>,
    new_thread_stream_behavior: Mutex<FakeStreamBehavior>,
    turn_stream_behavior: Mutex<FakeStreamBehavior>,
}

#[derive(Debug, Clone, Default)]
struct FakeStreamBehavior {
    events: Vec<ConversationStreamEvent>,
    error: Option<String>,
}

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
        cwd: &str,
        prompt: &str,
        event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .push((cwd.to_string(), prompt.to_string()));
        run_fake_stream(
            event_sender,
            self.new_thread_stream_behavior
                .lock()
                .expect("new-thread stream behavior mutex poisoned")
                .clone(),
        )
    }

    fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .push((thread_id.to_string(), prompt.to_string()));
        run_fake_stream(
            event_sender,
            self.turn_stream_behavior
                .lock()
                .expect("turn stream behavior mutex poisoned")
                .clone(),
        )
    }
}

impl PlanningThreadLauncher for FakeCodexAppServerPort {
    fn run_hidden_planning_thread(
        &self,
        workspace_directory: &str,
        prompt: &str,
        event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.run_new_thread_stream(workspace_directory, prompt, event_sender)
    }
}

fn run_fake_stream(
    event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    behavior: FakeStreamBehavior,
) -> Result<()> {
    for event in behavior.events {
        let _ = event_sender.send(event);
    }

    if let Some(error) = behavior.error {
        Err(anyhow::anyhow!(error))
    } else {
        Ok(())
    }
}

struct FakeFollowupTemplatePort;

impl FollowupTemplatePort for FakeFollowupTemplatePort {
    fn load_workspace_templates(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<WorkspaceFollowupTemplateRecord>> {
        if workspace_dir == "/tmp/failing" {
            return Err(anyhow::anyhow!("permission denied"));
        }
        if workspace_dir == "/tmp/root" {
            return Ok(vec![WorkspaceFollowupTemplateRecord {
                name: "root-template".to_string(),
                path: "/tmp/root/.codex-exec-loop/followups/root-template.md".to_string(),
                body: "workspace template body".to_string(),
            }]);
        }

        Ok(Vec::new())
    }
}

fn make_test_app() -> (NativeTuiApp, Arc<FakeCodexAppServerPort>) {
    let codex_port = Arc::new(FakeCodexAppServerPort::default());
    let followup_port = Arc::new(FakeFollowupTemplatePort);
    let mut app = NativeTuiApp::new(
        StartupService::new(codex_port.clone()),
        SessionService::new(codex_port.clone()),
        ConversationService::new(codex_port.clone()),
        FollowupTemplateService::new(followup_port),
        PlanningServices::from_ports(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            Arc::new(AppServerPlanningWorkerAdapter::new(codex_port.clone())),
        ),
    );
    app.show_startup_ascii_art = false;

    (app, codex_port)
}

fn sample_template_catalog() -> FollowupTemplateCatalog {
    FollowupTemplateCatalog {
        items: vec![
            FollowupTemplateDefinition {
                id: "builtin-next-task".to_string(),
                label: "builtin next-task".to_string(),
                body: "대리인입니다.\n자동 후속 {auto_turn}/{max_auto_turns} 입니다.\n\n직전 답변:\n{last_message}\n{stop_keyword}".to_string(),
                source: FollowupTemplateSource::Builtin,
            },
            FollowupTemplateDefinition {
                id: "builtin-plan-queue".to_string(),
                label: "builtin plan-queue".to_string(),
                body: "plan_priority_queue.md\n{last_message}\n{stop_keyword}".to_string(),
                source: FollowupTemplateSource::Builtin,
            },
            FollowupTemplateDefinition {
                id: "workspace-custom-review".to_string(),
                label: "workspace custom-review".to_string(),
                body: "workspace custom body\n{last_message}".to_string(),
                source: FollowupTemplateSource::WorkspaceFile {
                    path: "/tmp/workspace/.codex-exec-loop/followups/custom-review.md"
                        .to_string(),
                },
            },
        ],
    }
}

fn sample_startup_diagnostics(workspace_path: &str, can_continue: bool) -> StartupDiagnostics {
    StartupDiagnostics {
        cwd: workspace_path.to_string(),
        codex_binary_ok: true,
        codex_binary_detail: "ok".to_string(),
        workspace_ok: true,
        workspace_path: workspace_path.to_string(),
        workspace_detail: "ok".to_string(),
        initialize_ok: true,
        initialize_detail: "ok".to_string(),
        account_ok: can_continue,
        account_detail: if can_continue {
            "ok".to_string()
        } else {
            "needs login".to_string()
        },
        warnings: Vec::new(),
        schema_snapshot: "schema".to_string(),
    }
}

fn sample_session(id: &str) -> SessionSummary {
    sample_session_with_workspace(id, "/tmp/root", "preview")
}

fn sample_session_with_workspace(id: &str, cwd: &str, preview: &str) -> SessionSummary {
    sample_session_with_workspace_at(id, cwd, preview, 1_700_000_000)
}

fn sample_session_with_workspace_at(
    id: &str,
    cwd: &str,
    preview: &str,
    updated_at_epoch: i64,
) -> SessionSummary {
    SessionSummary {
        id: id.to_string(),
        name: Some(id.to_string()),
        preview: preview.to_string(),
        cwd: cwd.to_string(),
        source: "codex".to_string(),
        model_provider: "openai".to_string(),
        updated_at_epoch,
        status_type: "ready".to_string(),
        path: format!("{cwd}/{id}.json"),
        git_branch: Some("main".to_string()),
    }
}

fn create_temp_workspace(prefix: &str) -> String {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be valid")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
    std::fs::create_dir_all(&path).expect("temp workspace should be created");
    path.display().to_string()
}

fn bootstrap_active_planning_workspace(workspace_dir: &str) {
    let planning_services =
        PlanningServices::from_workspace_port(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    let stage_result = planning_services
        .init_service
        .stage_simple_mode_draft(workspace_dir)
        .expect("planning workspace should stage");
    let promote_result = planning_services
        .init_service
        .promote_staged_draft(workspace_dir, &stage_result.draft_name)
        .expect("planning workspace should promote");
    assert!(
        promote_result.promoted_file_count > 0,
        "bootstrap planning workspace should become ready"
    );
}

fn enable_queue_idle_review_and_enqueue(workspace_dir: &str) {
    let planning_dir = std::path::Path::new(workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    let directions_path = planning_dir.join("directions.toml");
    let directions = std::fs::read_to_string(&directions_path)
        .expect("directions.toml should exist before enabling queue-idle review");
    let directions = directions
        .replace(r#"policy = "stop""#, r#"policy = "review_and_enqueue""#)
        .replace(
            r#"prompt_path = """#,
            &format!(r#"prompt_path = "{DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH}""#),
        );
    std::fs::write(&directions_path, directions).expect("updated directions.toml should write");

    let prompt_path = std::path::Path::new(workspace_dir).join(DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH);
    std::fs::create_dir_all(
        prompt_path
            .parent()
            .expect("queue-idle prompt path should have a parent"),
    )
    .expect("queue-idle prompt directory should be created");
    std::fs::write(
        prompt_path,
        "# Queue Idle Review\n\n- Re-open the directions and enqueue only justified follow-up work.\n",
    )
    .expect("queue-idle prompt should write");
}

fn count_staged_planning_drafts(workspace_dir: &str) -> usize {
    let drafts_dir = std::path::Path::new(workspace_dir)
        .join(".codex-exec-loop")
        .join("planning")
        .join("drafts");
    std::fs::read_dir(drafts_dir)
        .map(|entries| entries.filter_map(|entry| entry.ok()).count())
        .unwrap_or(0)
}

fn sync_draft_conversation_to_startup_workspace(app: &mut NativeTuiApp) {
    let workspace_dir = app.current_workspace_directory();
    app.sync_draft_shell_workspace(&workspace_dir);
}

fn ready_turn_planning_capture(
    workspace_directory: &str,
    snapshot: PlanningExecutionSnapshot,
) -> ActiveTurnPlanningCapture {
    ActiveTurnPlanningCapture::ready(workspace_directory.to_string(), snapshot)
}

fn failed_turn_planning_capture(
    workspace_directory: &str,
    message: impl Into<String>,
) -> ActiveTurnPlanningCapture {
    ActiveTurnPlanningCapture::capture_failed(workspace_directory.to_string(), message.into())
}

fn ready_conversation() -> ConversationViewModel {
    ConversationViewModel {
        thread_id: "thread-1".to_string(),
        title: "Existing session".to_string(),
        cwd: "/tmp/workspace".to_string(),
        draft_workspace_directory: "/tmp/workspace".to_string(),
        messages: Vec::new(),
        cached_conversation_lines: format_conversation_lines(&[]),
        live_agent_message: None,
        buffered_tool_messages: Vec::new(),
        base_warnings: Vec::new(),
        template_warnings: Vec::new(),
        warnings: Vec::new(),
        runtime_notices: Vec::new(),
        input_buffer: String::new(),
        startup_submit_armed: false,
        active_turn_id: None,
        active_turn_workspace_directory: None,
        active_turn_started_at: None,
        planning_repair_state: None,
        input_state: ConversationInputState::ReadyToContinue,
        auto_follow_state: AutoFollowState::new(sample_template_catalog()),
        planning_runtime_snapshot: PlanningRuntimeSnapshot::uninitialized(),
        turn_activity: TurnActivityState::default(),
        approval_review: None,
        last_auto_followup_activity: None,
        last_planning_task_handoff: None,
        status_text: "thread loaded".to_string(),
    }
}

#[test]
fn running_turn_still_shows_buffered_prompt() {
    let mut conversation = ready_conversation();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.input_buffer = "Continue from the last change.".to_string();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Continue from the last change."));
    assert!(rendered.contains("Ctrl+j inserts a new line"));
}

#[test]
fn empty_existing_session_prompts_for_next_message() {
    let conversation = ready_conversation();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line == "> "));
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Ready to continue this session."))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Ctrl+j for newline"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Shell commands: :diag"))
    );
}

#[test]
fn inline_tail_compacts_empty_session_prompt_copy() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.conversation_state = ConversationState::Ready(ready_conversation());

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("> "));
    assert!(rendered.contains("prompt: session ready"));
    assert!(rendered.contains("Ctrl+j nl"));
    assert!(rendered.contains(":help"));
    assert!(!rendered.contains(":help commands"));
    assert!(!rendered.contains("Ready to continue this session."));
    assert!(!rendered.contains("Shell commands: :diag"));
}

#[test]
fn inline_tail_compacts_empty_draft_prompt_copy() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let mut conversation = ready_conversation();
    conversation.thread_id.clear();
    conversation.input_state = ConversationInputState::DraftReady;
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("startup: startup ready"));
    assert!(rendered.contains("workspace: /tmp/root"));
    assert!(rendered.contains("diagnostics: codex ok  |  app-server ok  |  account ok"));
    assert!(rendered.contains("conversation"));
    assert!(rendered.contains("first reply appears here after you send the opening prompt"));
    assert!(rendered.contains("starter: start with a task, file path, or bug summary"));
    assert!(rendered.contains("> "));
    assert!(rendered.contains("prompt: new thread ready"));
    assert!(rendered.contains("Ctrl+j nl"));
    assert!(rendered.contains(":help"));
    assert!(!rendered.contains(":help commands"));
    assert!(!rendered.contains("thread: new draft  |  turn: idle"));
}

#[test]
fn inline_tail_uses_compact_thread_title_instead_of_full_thread_id() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let mut conversation = ready_conversation();
    conversation.thread_id = "019d6e93-818a-7661-9e0d-7dec23c4b84d".to_string();
    conversation.title = "Untitled thread".to_string();
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("thread: Untitled thread"));
    assert!(!rendered.contains("019d6e93-818a-7661-9e0d-7dec23c4b84d"));
}

#[test]
fn empty_draft_prompts_for_first_message() {
    let mut conversation = ready_conversation();
    conversation.thread_id.clear();
    conversation.input_state = ConversationInputState::DraftReady;

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line == "> "));
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Ready to start a new thread."))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Ctrl+j for newline"))
    );
}

#[test]
fn planning_init_command_opens_selector_overlay() {
    let (mut app, _) = make_test_app();

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
    ]
    .into_iter()
    .collect::<HashSet<_>>();
    assert_eq!(staged_files, expected_files);
    let directions = std::fs::read_to_string(draft_directories[0].join("directions.toml"))
        .expect("staged directions should be readable");
    assert!(directions.contains("general-workstream"));

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
    app.conversation_state = ConversationState::Ready(ready_conversation());
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
        crate::application::service::planning_bootstrap_service::PlanningBootstrapService::new()
            .build_artifacts();
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
    app.conversation_state = ConversationState::Ready(conversation);
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
        app.planning_services
            .runtime_facade
            .load_runtime_snapshot_or_invalid(&workspace_dir),
    );
    app.conversation_state = ConversationState::Ready(conversation);

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
    });
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.replace_planning_runtime_snapshot(
        app.planning_services
            .runtime_facade
            .load_runtime_snapshot_or_invalid(&workspace_dir),
    );
    app.conversation_state = ConversationState::Ready(conversation);

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

    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 0;
    conversation.cwd = workspace_dir.clone();
    conversation.draft_workspace_directory = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-main".to_string());
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        latest_reply.clone(),
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.replace_planning_runtime_snapshot(
        app.planning_services
            .runtime_facade
            .load_runtime_snapshot_or_invalid(&workspace_dir),
    );
    app.conversation_state = ConversationState::Ready(conversation);

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
    app.conversation_state = ConversationState::Ready(conversation);

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
        crate::application::service::planning_bootstrap_service::PlanningBootstrapService::new()
            .build_artifacts();
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
    app.conversation_state = ConversationState::Ready(conversation);
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
        crate::application::service::planning_bootstrap_service::PlanningBootstrapService::new()
            .build_artifacts();
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
    app.conversation_state = ConversationState::Ready(conversation);
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
        crate::application::service::planning_bootstrap_service::PlanningBootstrapService::new()
            .build_artifacts();
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
    app.conversation_state = ConversationState::Ready(conversation);
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
        app.planning_services
            .runtime_facade
            .load_runtime_snapshot_or_invalid(&workspace_dir),
    );
    app.conversation_state = ConversationState::Ready(conversation);

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
        app.planning_services
            .runtime_facade
            .load_runtime_snapshot_or_invalid(&workspace_dir),
    );
    app.conversation_state = ConversationState::Ready(conversation);

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
        crate::application::service::planning_bootstrap_service::PlanningBootstrapService::new()
            .build_artifacts();
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
    app.conversation_state = ConversationState::Ready(conversation);
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
    app.conversation_state = ConversationState::Ready(conversation);
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

#[test]
fn stale_planning_capture_blocks_reconciliation_for_other_workspace() {
    let (mut app, codex_port) = make_test_app();
    let current_workspace = create_temp_workspace("planning-capture-current");
    let stale_workspace = create_temp_workspace("planning-capture-stale");
    let mut conversation = ready_conversation();
    conversation.cwd = current_workspace.clone();
    conversation.active_turn_workspace_directory = Some(current_workspace.clone());
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-mismatch".to_string());
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::Ready(conversation);
    app.active_turn_planning_capture = Some(ready_turn_planning_capture(
        &stale_workspace,
        PlanningExecutionSnapshot {
            directions_toml: Some("version = 1".to_string()),
            task_ledger_json: Some("{\"version\":1,\"tasks\":[]}".to_string()),
            task_ledger_schema_json: None,
            result_output_markdown: None,
            queue_snapshot_json: None,
        },
    ));

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-mismatch".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ));

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
        conversation
            .planning_runtime_snapshot
            .preview_status_label(),
        "blocked"
    );
    assert!(
        conversation
            .planning_runtime_snapshot
            .preview_detail()
            .is_some_and(|detail| detail.contains("stale planning snapshot"))
    );
    assert!(conversation.runtime_notices.iter().any(|notice| {
        notice.contains(&stale_workspace) && notice.contains(&current_workspace)
    }));

    std::fs::remove_dir_all(current_workspace).expect("current workspace should be removed");
    std::fs::remove_dir_all(stale_workspace).expect("stale workspace should be removed");
}

#[test]
fn stream_worker_forces_failure_when_service_exits_without_terminal_event() {
    let (app, codex_port) = make_test_app();
    let mut runtime = super::shell_runtime::ShellRuntime::new(app);
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("conversation should start ready");
    };
    conversation.thread_id = "thread-123".to_string();
    codex_port
        .turn_stream_behavior
        .lock()
        .expect("turn stream behavior mutex poisoned")
        .error = Some("transport closed".to_string());

    runtime
        .app_mut()
        .submit_prompt("ship it".to_string(), PromptOrigin::Manual);

    for _ in 0..20 {
        thread::sleep(Duration::from_millis(5));
        runtime.poll_background_messages();
        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            continue;
        };
        if conversation.input_state == ConversationInputState::ReadyToContinue
            && conversation.status_text == "turn failed"
        {
            break;
        }
    }

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(
        conversation.input_state,
        ConversationInputState::ReadyToContinue
    );
    assert_eq!(conversation.status_text, "turn failed");
    assert!(conversation.messages.iter().any(|message| {
        message.kind == ConversationMessageKind::Status
            && message
                .text
                .contains("turn stream failed before a terminal event: transport closed")
    }));
    assert!(conversation.runtime_notices.iter().any(|notice| {
        notice.contains("turn stream returned an error before a terminal event: transport closed")
    }));
}

#[test]
fn multiline_buffer_renders_as_multiple_input_lines() {
    let mut conversation = ready_conversation();
    conversation.input_buffer = "first line\nsecond line".to_string();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line == "> first line"));
    assert!(rendered.iter().any(|line| line == "  second line"));
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Ctrl+j inserts a new line"))
    );
}

#[test]
fn trailing_newline_keeps_blank_prompt_line_visible() {
    let mut conversation = ready_conversation();
    conversation.input_buffer = "first line\n".to_string();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line == "> first line"));
    assert!(rendered.iter().any(|line| line == "  "));
}

#[test]
fn inline_shell_command_buffer_shows_command_hint() {
    let mut conversation = ready_conversation();
    conversation.input_buffer = ":templates".to_string();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("> :templates"));
    assert!(rendered.contains("Press Enter to open the template inspection."));
    assert!(rendered.contains(":templates template inspection"));
}

#[test]
fn max_auto_turns_command_buffer_shows_argument_aware_hint() {
    let mut conversation = ready_conversation();
    conversation.input_buffer = ":turns 8".to_string();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("> :turns 8"));
    assert!(rendered.contains("Press Enter to set max auto turns to 8."));
    assert!(rendered.contains(":turns set max auto turns"));
}

#[test]
fn colon_only_buffer_shows_available_shell_commands() {
    let mut conversation = ready_conversation();
    conversation.input_buffer = ":".to_string();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("> :"));
    assert!(rendered.contains("Shell commands: type a name, then press Enter."));
    assert!(rendered.contains(":diag diagnostics"));
    assert!(rendered.contains(":planning planning mode"));
    assert!(rendered.contains(":turns set max auto turns"));
    assert!(rendered.contains(":help command help"));
}

#[test]
fn command_prefix_buffer_filters_matching_shell_commands() {
    let mut conversation = ready_conversation();
    conversation.input_buffer = ":p".to_string();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("> :p"));
    assert!(rendered.contains("Matching shell commands for `:p`:"));
    assert!(rendered.contains(":planning planning mode"));
    assert!(!rendered.contains(":diag diagnostics"));
}

#[test]
fn inline_tail_command_prefix_shows_filtered_matches_while_typing() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let mut conversation = ready_conversation();
    conversation.input_buffer = ":p".to_string();
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("> :p"));
    assert!(rendered.contains("matches :p: :planning"));
    assert!(!rendered.contains(":diag"));
}

#[test]
fn input_prompt_cursor_offset_starts_after_prompt_prefix() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());

    assert_eq!(build_input_prompt_cursor_offset(&app, 80), Some((2, 0)));
}

#[test]
fn input_prompt_cursor_offset_tracks_trailing_blank_line() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.input_buffer = "first line\n".to_string();
    app.conversation_state = ConversationState::Ready(conversation);

    assert_eq!(build_input_prompt_cursor_offset(&app, 80), Some((2, 1)));
}

#[test]
fn inline_prompt_cursor_offset_accounts_for_status_lines() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.conversation_state = ConversationState::Ready(ready_conversation());

    assert_eq!(build_inline_prompt_cursor_offset(&app, 80), Some((2, 3)));
}

#[test]
fn multiline_buffer_expands_composer_height() {
    let mut conversation = ready_conversation();
    conversation.input_buffer = "one\ntwo\nthree\nfour\nfive\nsix".to_string();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready);

    assert_eq!(
        shell_layout::build_input_block_height(&rendered),
        MAX_COMPOSER_HEIGHT
    );
}

#[test]
fn status_footer_height_expands_for_ready_shell_summary() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());

    let rendered = build_shell_footer_lines(&app);

    assert_eq!(shell_layout::build_shell_footer_height(&rendered), 5);
}

#[test]
fn shell_footer_shows_github_polling_state_summary() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.github_review_polling_state = GithubReviewPollingState::SetupError {
        target: Some(GithubPullRequestTarget::new("acme/widgets", 42)),
        message: "missing RefinedStone credential".to_string(),
    };

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("gh: setup failed acme/widgets#42"));
}

#[test]
fn shell_footer_surfaces_recent_github_review_change_summary() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.github_review_polling_state = GithubReviewPollingState::active(
        super::github_polling::GithubReviewPollingConfig {
            target: GithubPullRequestTarget::new("acme/widgets", 42),
            interval: std::time::Duration::from_secs(30),
        },
        std::time::Instant::now(),
    );
    app.record_github_review_poll_result(
        std::time::Instant::now(),
        Ok(sample_github_review_poll_result()),
    );

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("notice: gh update: review commented by reviewer: Looks good"));
}

#[test]
fn inline_shell_view_surfaces_live_agent_output_in_footer() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a ready conversation");
    };
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());
    conversation.live_agent_message = Some(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "working through the next streaming answer chunk",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    let view = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer);
    let rendered = view
        .footer_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("live: Codex"));
    assert!(rendered.contains("  working through the next streaming answer chunk"));
}

#[test]
fn inline_tail_shows_latest_live_agent_lines_instead_of_activity_summary() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a ready conversation");
    };
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());
    conversation.live_agent_message = Some(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "line one\nline two\nline three",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("live: Codex"));
    assert!(!rendered.contains("tool: idle"));
    assert!(!rendered.contains("line one"));
    assert!(rendered.contains("  line two"));
    assert!(rendered.contains("  line three"));
}

#[test]
fn tool_activity_stays_out_of_inline_transcript_until_turn_completion() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a ready conversation");
    };
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::ToolActivity {
            activity: crate::domain::conversation::ConversationToolActivity {
                kind: crate::domain::conversation::ConversationToolActivityKind::CommandExecution,
                text: "command: cargo test [running]".to_string(),
                file_change_count: 0,
            },
        },
    ));

    let view = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer);
    let transcript_rendered = view
        .conversation_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let footer_rendered = view
        .footer_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(!transcript_rendered.contains("command: cargo test [running]"));
    assert!(footer_rendered.contains("notice: tool activity: command: cargo test [running]"));

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    let view = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer);
    let transcript_rendered = view
        .conversation_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(transcript_rendered.contains("Tool:"));
    assert!(transcript_rendered.contains("command: cargo test [running]"));
}

#[test]
fn startup_pending_prompts_wait_before_send() {
    let conversation = ready_conversation();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Pending)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Startup checks are still running."));
    assert!(rendered.contains("send once diagnostics turn ready"));
}

#[test]
fn armed_startup_submit_surfaces_queue_hint() {
    let mut conversation = ready_conversation();
    conversation.input_buffer = "ship it".to_string();
    conversation.startup_submit_armed = true;

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Pending)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Prompt queued until startup checks finish."));
    assert!(rendered.contains("Editing cancels the queued send."));
}

#[test]
fn startup_blocked_prompt_guides_user_to_diagnostics_overlay() {
    let conversation = ready_conversation();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Blocked)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Startup diagnostics need attention."));
    assert!(rendered.contains("Open Ctrl+d"));
}

#[test]
fn draft_workspace_sync_preserves_buffered_input() {
    let (mut app, _) = make_test_app();

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.cwd = "/tmp/subdir".to_string();
    conversation.draft_workspace_directory = "/tmp/subdir".to_string();
    conversation.input_buffer = "buffered prompt".to_string();

    app.sync_draft_shell_workspace("/tmp/root");

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("draft conversation should still be ready");
    };
    assert_eq!(conversation.cwd, "/tmp/root");
    assert_eq!(conversation.input_buffer, "buffered prompt");
    assert_eq!(conversation.auto_follow_state.template_count(), 5);
    assert!(conversation.status_text.contains("draft workspace synced"));
}

#[test]
fn background_startup_message_updates_startup_state_and_syncs_draft_workspace() {
    let (app, _) = make_test_app();
    let mut runtime = super::shell_runtime::ShellRuntime::new(app);
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.cwd = "/tmp/subdir".to_string();
    conversation.draft_workspace_directory = "/tmp/subdir".to_string();

    runtime
        .app()
        .tx
        .send(BackgroundMessage::StartupLoaded(Ok(
            sample_startup_diagnostics("/tmp/root", false),
        )))
        .expect("background message should enqueue");

    runtime.poll_background_messages();

    let app = runtime.app();
    assert!(matches!(app.startup_state, StartupState::Ready(_)));
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(conversation.cwd, "/tmp/root");
    assert_eq!(conversation.auto_follow_state.template_count(), 5);
}

#[test]
fn background_conversation_loaded_resets_followup_overlay_state() {
    let (app, _) = make_test_app();
    let mut runtime = super::shell_runtime::ShellRuntime::new(app);
    runtime.app_mut().followup_overlay_ui_state.preview_scroll = 12;
    runtime
        .app_mut()
        .followup_overlay_ui_state
        .list_state
        .select(Some(2));
    runtime
        .app_mut()
        .followup_overlay_ui_state
        .stop_keyword_editor
        .buffer = "STALE".to_string();
    runtime
        .app_mut()
        .followup_overlay_ui_state
        .max_auto_turns_editor
        .buffer = "99".to_string();
    runtime.app_mut().planner_worker_panel_state = PlannerWorkerPanelState {
        status: PlannerWorkerStatus::RefreshSucceeded,
        last_operation_label: Some("refresh".to_string()),
        last_summary: Some("stale planner state".to_string()),
        last_rejected_summary: None,
        last_queue_summary: None,
        last_notice_detail: None,
        last_prompt: Some("stale prompt".to_string()),
        last_response: Some("stale response".to_string()),
        last_host_detail: None,
    };

    runtime
        .app()
        .tx
        .send(BackgroundMessage::ConversationLoaded(Ok(
            ConversationSnapshot {
                thread_id: "thread-123".to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/root".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
            },
        )))
        .expect("background message should enqueue");

    runtime.poll_background_messages();

    let app = runtime.app();
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should become ready");
    };
    assert_eq!(conversation.thread_id, "thread-123");
    assert_eq!(conversation.cwd, "/tmp/root");
    assert_eq!(app.followup_overlay_ui_state.preview_scroll, 0);
    assert_eq!(app.followup_overlay_ui_state.list_state.selected(), None);
    assert_eq!(
        app.followup_overlay_ui_state.max_auto_turns_editor.buffer,
        "3"
    );
    assert_eq!(
        app.followup_overlay_ui_state.stop_keyword_editor.buffer,
        DEFAULT_AUTO_FOLLOW_STOP_KEYWORD
    );
    assert_eq!(
        app.planner_worker_panel_state,
        PlannerWorkerPanelState::default()
    );
}

#[test]
fn opening_new_draft_is_blocked_while_turn_is_streaming() {
    let (mut app, _) = make_test_app();

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.thread_id = "thread-123".to_string();
    conversation.title = "Streaming thread".to_string();
    conversation.input_state = ConversationInputState::StreamingTurn;

    app.open_new_conversation_shell();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(conversation.thread_id, "thread-123");
    assert_eq!(conversation.title, "Streaming thread");
    assert_eq!(
        conversation.input_state,
        ConversationInputState::StreamingTurn
    );
    assert!(conversation.status_text.contains("turn still running"));
}

#[test]
fn auto_follow_submission_respects_startup_gate() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Loading;

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.thread_id = "thread-123".to_string();
    conversation.input_state = ConversationInputState::ReadyToContinue;

    app.submit_prompt(
        "continue working".to_string(),
        PromptOrigin::AutoFollow(AutoFollowupSubmitContext {
            queued_from_turn_id: "turn-0".to_string(),
            template_label: "builtin next-task".to_string(),
            transcript_text: "다음 queued task 1개를 이어서 진행합니다.".to_string(),
            debug_detail: None,
            handoff_task: None,
        }),
    );

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
    assert!(conversation.status_text.contains("auto follow-up paused"));
}

#[test]
fn queue_auto_prompt_records_planner_debug_detail_when_debug_is_enabled() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.planner_visibility = PlannerVisibility::Debug;
    app.planner_worker_panel_state = PlannerWorkerPanelState {
        status: PlannerWorkerStatus::RefreshSucceeded,
        last_operation_label: Some("refresh".to_string()),
        last_summary: Some("planner refreshed the queue".to_string()),
        last_rejected_summary: None,
        last_queue_summary: None,
        last_notice_detail: None,
        last_prompt: Some("planning worker prompt body".to_string()),
        last_response: Some("planner worker response body".to_string()),
        last_host_detail: None,
    };

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.thread_id = "thread-123".to_string();
    conversation.input_state = ConversationInputState::ReadyToContinue;

    app.execute_conversation_runtime_effect(ConversationRuntimeEffect::QueueAutoPrompt {
        prompt: "continue working".to_string(),
        queued_from_turn_id: "turn-0".to_string(),
        template_label: "builtin next-task".to_string(),
        transcript_text: "다음 queued task 1개를 이어서 진행합니다.".to_string(),
        handoff_task: None,
    });

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    let message = conversation
        .messages
        .first()
        .expect("auto follow-up message should be recorded");
    let debug_detail = message
        .debug_detail
        .as_deref()
        .expect("debug mode should attach transcript detail");
    assert!(debug_detail.contains("planner temp session: refresh / refresh ok"));
    assert!(debug_detail.contains("planner prompt:"));
    assert!(debug_detail.contains("planning worker prompt body"));
    assert!(debug_detail.contains("planner response:"));
    assert!(debug_detail.contains("planner worker response body"));
}

#[test]
fn queue_auto_prompt_omits_planner_debug_detail_for_non_builtin_transcript() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.planner_visibility = PlannerVisibility::Debug;
    app.planner_worker_panel_state = PlannerWorkerPanelState {
        status: PlannerWorkerStatus::RefreshSucceeded,
        last_operation_label: Some("refresh".to_string()),
        last_summary: Some("planner refreshed the queue".to_string()),
        last_rejected_summary: None,
        last_queue_summary: None,
        last_notice_detail: None,
        last_prompt: Some("planning worker prompt body".to_string()),
        last_response: Some("planner worker response body".to_string()),
        last_host_detail: None,
    };

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.thread_id = "thread-123".to_string();
    conversation.input_state = ConversationInputState::ReadyToContinue;

    app.execute_conversation_runtime_effect(ConversationRuntimeEffect::QueueAutoPrompt {
        prompt: "continue working".to_string(),
        queued_from_turn_id: "turn-0".to_string(),
        template_label: "workspace custom-review".to_string(),
        transcript_text: "workspace custom review를 이어서 진행합니다.".to_string(),
        handoff_task: None,
    });

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert!(
        conversation
            .messages
            .first()
            .expect("auto follow-up message should be recorded")
            .debug_detail
            .is_none()
    );
}

#[test]
fn queue_auto_prompt_omits_planner_debug_detail_outside_debug_mode() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.planner_worker_panel_state = PlannerWorkerPanelState {
        status: PlannerWorkerStatus::RefreshSucceeded,
        last_operation_label: Some("refresh".to_string()),
        last_summary: Some("planner refreshed the queue".to_string()),
        last_rejected_summary: None,
        last_queue_summary: None,
        last_notice_detail: None,
        last_prompt: Some("planning worker prompt body".to_string()),
        last_response: Some("planner worker response body".to_string()),
        last_host_detail: None,
    };

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.thread_id = "thread-123".to_string();
    conversation.input_state = ConversationInputState::ReadyToContinue;

    app.execute_conversation_runtime_effect(ConversationRuntimeEffect::QueueAutoPrompt {
        prompt: "continue working".to_string(),
        queued_from_turn_id: "turn-0".to_string(),
        template_label: "builtin next-task".to_string(),
        transcript_text: "다음 queued task 1개를 이어서 진행합니다.".to_string(),
        handoff_task: None,
    });

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert!(
        conversation
            .messages
            .first()
            .expect("auto follow-up message should be recorded")
            .debug_detail
            .is_none()
    );
}

#[test]
fn queue_auto_prompt_debug_detail_keeps_tail_lines_when_preview_is_truncated() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.planner_visibility = PlannerVisibility::Debug;
    let long_prompt = (0..40)
        .map(|index| format!("prompt line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let long_response = (0..40)
        .map(|index| format!("response line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.planner_worker_panel_state = PlannerWorkerPanelState {
        status: PlannerWorkerStatus::RefreshSucceeded,
        last_operation_label: Some("refresh".to_string()),
        last_summary: Some("planner refreshed the queue".to_string()),
        last_rejected_summary: None,
        last_queue_summary: None,
        last_notice_detail: None,
        last_prompt: Some(long_prompt),
        last_response: Some(long_response),
        last_host_detail: None,
    };

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.thread_id = "thread-123".to_string();
    conversation.input_state = ConversationInputState::ReadyToContinue;

    app.execute_conversation_runtime_effect(ConversationRuntimeEffect::QueueAutoPrompt {
        prompt: "continue working".to_string(),
        queued_from_turn_id: "turn-0".to_string(),
        template_label: "builtin next-task".to_string(),
        transcript_text: "다음 queued task 1개를 이어서 진행합니다.".to_string(),
        handoff_task: None,
    });

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    let debug_detail = conversation
        .messages
        .first()
        .expect("auto follow-up message should be recorded")
        .debug_detail
        .as_deref()
        .expect("debug mode should attach transcript detail");
    assert!(debug_detail.contains("prompt line 15"));
    assert!(!debug_detail.contains("prompt line 20"));
    assert!(debug_detail.contains("prompt line 39"));
    assert!(debug_detail.contains("response line 39"));
    assert!(debug_detail.contains("middle lines omitted in debug preview"));
    assert!(debug_detail.contains("worker received full text"));
}

#[test]
fn shell_view_toggles_auto_follow_debug_detail_with_planner_visibility() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.messages.push(
        ConversationMessage::new(
            ConversationMessageKind::User,
            "다음 queued task 1개를 이어서 진행합니다.",
            None,
            None,
        )
        .with_display_label("Auto Follow-up")
        .with_debug_detail("planner temp session: refresh / refresh ok"),
    );
    conversation.refresh_conversation_lines();

    let normal_rendered = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer)
        .conversation_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!normal_rendered.contains("planner temp session"));

    app.planner_visibility = PlannerVisibility::Debug;
    let debug_rendered = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer)
        .conversation_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(debug_rendered.contains("planner temp session: refresh / refresh ok"));
}

#[test]
fn manual_submit_while_startup_pending_arms_queue() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Loading;

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.input_buffer = "ship it".to_string();

    app.start_turn_submission();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert!(conversation.startup_submit_armed);
    assert_eq!(conversation.input_buffer, "ship it");
    assert!(
        conversation
            .status_text
            .contains("prompt queued until startup checks finish")
    );
    assert!(
        codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .is_empty()
    );
}

#[test]
fn startup_ready_submits_armed_prompt() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Loading;

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.input_buffer = "ship it".to_string();

    app.start_turn_submission();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.resolve_startup_submit_queue();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert!(!conversation.startup_submit_armed);
    assert!(matches!(
        conversation.input_state,
        ConversationInputState::SubmittingTurn
    ));
    let mut submitted_prompt = None;
    for _ in 0..20 {
        submitted_prompt = codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .iter()
            .find(|(_, prompt)| prompt.starts_with("ship it"))
            .map(|(_, prompt)| prompt.clone());
        if submitted_prompt.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        submitted_prompt.is_some(),
        "queued startup submit should reach the codex app-server port"
    );
}

#[test]
fn manual_submit_appends_planning_context_when_ready() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.input_buffer = "ship it".to_string();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context\nQueue Summary",
        "next task: task-1",
    ));

    app.start_turn_submission();

    let mut submitted_prompt = None;
    for _ in 0..20 {
        submitted_prompt = codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .next();
        if submitted_prompt.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let submitted_prompt =
        submitted_prompt.expect("manual submit should reach the codex app-server port");
    assert!(submitted_prompt.starts_with("ship it"));
    assert!(!submitted_prompt.contains("Planning Context"));
    assert!(!submitted_prompt.contains("Queue Summary"));
}

#[test]
fn editing_buffer_cancels_armed_startup_submit() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Loading;

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a draft conversation");
    };
    conversation.input_buffer = "ship".to_string();

    app.start_turn_submission();
    app.push_input_character('!');
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.resolve_startup_submit_queue();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert!(!conversation.startup_submit_armed);
    assert_eq!(conversation.input_buffer, "ship!");
    assert!(
        conversation
            .status_text
            .contains("queued startup send canceled")
    );
    assert!(
        codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .is_empty()
    );
}

#[test]
fn inline_diag_command_opens_overlay_and_clears_input() {
    let (mut app, codex_port) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a ready conversation");
    };
    conversation.input_buffer = ":diag".to_string();

    app.start_turn_submission();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(app.shell_overlay, ShellOverlay::Startup);
    assert!(conversation.input_buffer.is_empty());
    assert!(
        conversation
            .status_text
            .contains("opened diagnostics inspection")
    );
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
}

#[test]
fn inline_templates_command_opens_overlay_while_turn_is_streaming() {
    let (mut app, codex_port) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a ready conversation");
    };
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.input_buffer = ":templates".to_string();

    app.start_turn_submission();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(app.shell_overlay, ShellOverlay::FollowupTemplates);
    assert_eq!(
        conversation.input_state,
        ConversationInputState::StreamingTurn
    );
    assert!(conversation.input_buffer.is_empty());
    assert!(
        codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .is_empty()
    );
}

#[test]
fn inline_help_command_updates_status_and_clears_input() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a ready conversation");
    };
    conversation.input_buffer = ":help".to_string();

    app.start_turn_submission();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert!(conversation.input_buffer.is_empty());
    assert!(
        conversation
            .status_text
            .contains(InlineShellCommand::command_list_line())
    );
}

#[test]
fn inline_turns_command_updates_max_auto_turns_and_clears_input() {
    let (mut app, codex_port) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a ready conversation");
    };
    conversation.input_buffer = ":turns 8".to_string();

    app.start_turn_submission();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert!(conversation.input_buffer.is_empty());
    assert_eq!(conversation.auto_follow_state.max_auto_turns_value(), 8);
    assert!(
        codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .is_empty()
    );
}

#[test]
fn inline_turns_command_without_argument_shows_usage_and_clears_input() {
    let (mut app, codex_port) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a ready conversation");
    };
    conversation.input_buffer = ":turns".to_string();

    app.start_turn_submission();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert!(conversation.input_buffer.is_empty());
    assert_eq!(
        conversation.auto_follow_state.max_auto_turns_value(),
        DEFAULT_AUTO_FOLLOW_MAX_TURNS
    );
    assert!(
        conversation
            .status_text
            .contains("usage: :turns <1-50>  |  alias: :auto-turns <1-50>")
    );
    assert!(
        codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .is_empty()
    );
}

#[test]
fn inline_turns_command_with_invalid_argument_keeps_validation_message() {
    let (mut app, codex_port) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a ready conversation");
    };
    conversation.input_buffer = ":turns 70".to_string();

    app.start_turn_submission();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert!(conversation.input_buffer.is_empty());
    assert_eq!(
        conversation.auto_follow_state.max_auto_turns_value(),
        DEFAULT_AUTO_FOLLOW_MAX_TURNS
    );
    assert!(
        conversation
            .status_text
            .contains("whole number between 1 and 50")
    );
    assert!(
        codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .is_empty()
    );
}

#[test]
fn queue_command_opens_queue_overlay_and_clears_input() {
    for command in [":q", ":queue"] {
        let (mut app, codex_port) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should stay ready");
        };
        conversation.input_buffer = command.to_string();

        app.start_turn_submission();

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert_eq!(app.shell_overlay, ShellOverlay::Queue, "{command}");
        assert!(conversation.input_buffer.is_empty(), "{command}");
        assert!(
            conversation
                .status_text
                .contains("opened planning queue inspection"),
            "{command}"
        );
        assert!(
            codex_port
                .new_thread_calls
                .lock()
                .expect("new-thread call mutex poisoned")
                .is_empty(),
            "{command}"
        );
        assert!(
            codex_port
                .turn_calls
                .lock()
                .expect("turn call mutex poisoned")
                .is_empty(),
            "{command}"
        );
    }
}

#[test]
fn stop_command_turns_off_automation_and_clears_input() {
    let (mut app, codex_port) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("app should start with a ready conversation");
    };
    conversation.input_buffer = ":stop".to_string();

    app.start_turn_submission();

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert!(!conversation.auto_follow_state.enabled);
    assert!(conversation.input_buffer.is_empty());
    assert_eq!(conversation.status_text, "automation off");
    assert!(
        conversation
            .last_auto_followup_activity
            .as_ref()
            .is_some_and(|activity| activity.summary == "stopped: automation off")
    );
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
}

#[test]
fn queue_overlay_view_summarizes_ready_queue_without_raw_dump() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "focus on queue",
        "2 ready tasks",
    ));
    app.conversation_state = ConversationState::Ready(conversation);

    let view = build_queue_overlay_view(&app);
    let summary = view
        .summary_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let queue = view
        .queue_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let notes = view
        .note_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(summary.contains("next: Implement shell planning status"));
    assert!(summary.contains("queue: 2 ready tasks"));
    assert!(!summary.contains("workspace:"));
    assert!(!summary.contains("planner worker:"));
    assert!(queue.contains("#1 [ready / p10] Implement shell planning status"));
    assert!(notes.contains("skipped tasks: 1"));
    assert!(!queue.contains("\"task_id\""));
    assert!(!queue.contains("task-1"));
}

#[test]
fn queue_overlay_view_shows_proposals_in_compact_form() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_proposal_only_planning_runtime_snapshot(
        "focus on queue proposals",
        "0 ready tasks",
        "1 proposal available",
    ));
    app.conversation_state = ConversationState::Ready(conversation);

    let view = build_queue_overlay_view(&app);
    let proposals = view
        .proposal_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(proposals.contains("#1 [proposed / p7] Draft a queue inspection overlay"));
    assert!(!proposals.contains("\"direction_id\""));
}

#[test]
fn transcript_title_describes_live_scrollback() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());

    assert_eq!(
        build_transcript_title(&app, ShellFrontendMode::InlineMainBuffer).to_string(),
        "Transcript / live scrollback"
    );
}

#[test]
fn transcript_panel_view_collects_title_and_tail_scroll_offset() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    let transcript_lines = (1..=24)
        .map(|index| Line::from(format!("line {index}")))
        .collect::<Vec<_>>();

    let view = build_transcript_panel_view(&mut app, transcript_lines, 20, 6);

    assert_eq!(view.scroll_offset, 18);
    assert_eq!(view.title.to_string(), "Transcript / live scrollback");
    assert_eq!(view.lines.len(), 24);
}

#[test]
fn input_title_includes_submit_and_newline_hints() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());

    let rendered = build_input_title(&app, ShellFrontendMode::InlineMainBuffer).to_string();

    assert!(rendered.contains("Prompt / ready"));
    assert!(rendered.contains("Enter send"));
    assert!(rendered.contains("Ctrl+j newline"));
}

#[test]
fn input_title_shows_readiness_gated_submit_hint() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Loading;
    app.conversation_state = ConversationState::Ready(ready_conversation());

    let rendered = build_input_title(&app, ShellFrontendMode::InlineMainBuffer).to_string();

    assert!(rendered.contains("Enter send when ready"));
}

#[test]
fn composer_title_shows_queued_submit_hint_when_startup_queue_is_armed() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Loading;
    let mut conversation = ready_conversation();
    conversation.startup_submit_armed = true;
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_input_title(&app, ShellFrontendMode::InlineMainBuffer).to_string();

    assert!(rendered.contains("queued until ready"));
}

#[test]
fn status_title_surfaces_overlay_and_followup_controls() {
    let rendered = build_status_title().to_string();

    assert_eq!(rendered, "Controls / shell shortcuts and live status");
}

#[test]
fn conversation_shell_view_collects_inline_snapshot_content() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.conversation_state = ConversationState::Ready(ready_conversation());

    let view = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer);
    let header = view
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let transcript_title = build_transcript_title(&app, ShellFrontendMode::InlineMainBuffer);

    assert!(view.shell_title.to_string().contains("Shell /"));
    assert_eq!(transcript_title.to_string(), "Transcript / live scrollback");
    assert!(view.status_title.to_string().contains("Controls /"));
    assert!(view.input_title.to_string().contains("Prompt / ready"));
    assert!(header.contains("thread: thread-1"));
    assert!(header.contains("frontend: inline main buffer"));
    assert!(header.contains("history: host terminal scrollback"));
    assert!(header.contains("startup: "));
    assert!(!view.conversation_lines.is_empty());
    assert!(!view.footer_lines.is_empty());
    assert!(!view.input_lines.is_empty());
}

#[test]
fn startup_ascii_art_defaults_to_enabled_unless_explicitly_disabled() {
    assert!(startup_ascii_art_enabled_from_value(None));
    assert!(startup_ascii_art_enabled_from_value(Some("true")));
    assert!(!startup_ascii_art_enabled_from_value(Some("false")));
    assert!(!startup_ascii_art_enabled_from_value(Some("0")));
}

#[test]
fn blank_draft_uses_startup_ascii_art_when_enabled() {
    let (mut app, _) = make_test_app();
    app.show_startup_ascii_art = true;

    let view = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer);
    let rendered = view
        .conversation_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains(".::::::.::::::.::::::.::::::"));
    assert!(rendered.contains(".::       .::.::  .::   .::"));
    assert!(!rendered.contains("No messages in this thread yet."));
}

#[test]
fn typing_in_blank_draft_keeps_startup_ascii_art_visible() {
    let (mut app, _) = make_test_app();
    app.show_startup_ascii_art = true;
    if let ConversationState::Ready(conversation) = &mut app.conversation_state {
        conversation.input_buffer = "hello".to_string();
    }

    let view = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer);
    let rendered = view
        .conversation_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains(".::::::.::::::.::::::.::::::"));
    assert!(rendered.contains(".::       .::.::  .::   .::"));
    assert!(!rendered.contains("No messages in this thread yet."));
}

#[test]
fn inline_tail_keeps_startup_context_above_buffered_prompt_in_new_draft() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    if let ConversationState::Ready(conversation) = &mut app.conversation_state {
        conversation.input_buffer = "hello".to_string();
    }

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("startup: startup ready"));
    assert!(rendered.contains("workspace: /tmp/root"));
    assert!(rendered.contains("diagnostics: codex ok  |  app-server ok  |  account ok"));
    assert!(rendered.contains("conversation"));
    assert!(rendered.contains("first reply appears here after you send the opening prompt"));
    assert!(rendered.contains("starter: opening prompt buffered below"));
    assert!(rendered.contains("> hello"));
    assert!(rendered.contains("buffered prompt"));
}

#[test]
fn inline_transcript_panel_stays_pinned_to_tail_even_after_manual_viewport_state() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.conversation_state = ConversationState::Ready(ready_conversation());
    let transcript_lines = (1..=24)
        .map(|index| Line::from(format!("line {index}")))
        .collect::<Vec<_>>();

    let view = build_transcript_panel_view(&mut app, transcript_lines, 20, 6);

    assert_eq!(view.scroll_offset, 18);
}

#[test]
fn conversation_shell_frame_view_collects_layout_and_transcript_panel() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.conversation_state = ConversationState::Ready(ready_conversation());

    let view = build_conversation_shell_frame_view(
        &mut app,
        ShellFrontendMode::InlineMainBuffer,
        Rect::new(0, 0, 100, 36),
    );

    assert!(view.shell_title.to_string().contains("Shell /"));
    assert_eq!(view.header_area.y, 1);
    assert!(view.transcript_area.height >= 12);
    assert!(view.footer_area.y > view.transcript_area.y);
    assert!(view.input_area.y > view.footer_area.y);
    assert!(
        view.transcript_view
            .title
            .to_string()
            .contains("Transcript /")
    );
    assert!(
        view.header_lines
            .iter()
            .any(|line| line.to_string().contains("frontend: inline main buffer"))
    );
    assert!(!view.transcript_view.lines.is_empty());
}

#[test]
fn startup_overlay_view_collects_summary_checks_and_keys() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));

    let view = build_startup_overlay_view(&app);
    let summary = view
        .summary_lines
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

    assert!(
        view.header_lines[0]
            .to_string()
            .contains("Startup Diagnostics")
    );
    assert!(summary.contains("status: ready"));
    assert!(summary.contains("/tmp/root"));
    assert!(!view.check_lines.is_empty());
    assert!(keys.contains("rerun checks"));
}

#[test]
fn session_overlay_view_collects_selected_session_detail_and_keys() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.session_state = SessionState::Ready(RecentSessions {
        items: vec![sample_session("thread-1"), sample_session("thread-2")],
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.selected_session_index = 1;

    let view = build_session_overlay_view(&app);
    let list = view
        .list_view
        .items
        .iter()
        .map(|item| {
            item.lines
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n---\n");
    let detail = view
        .detail_lines
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

    assert!(view.header_lines[0].to_string().contains("Recent Sessions"));
    assert!(view.list_view.message_lines.is_none());
    assert_eq!(view.list_view.selected_index, Some(1));
    assert!(list.contains("thread-2"));
    assert!(detail.contains("id: thread-2"));
    assert!(detail.contains("/tmp/root/thread-2.json"));
    assert!(keys.contains("/: query"));
    assert!(keys.contains("c: clear"));
    assert!(keys.contains("Home/End"));
    assert!(keys.contains("Enter: open"));
}

#[test]
fn session_overlay_view_clamps_selection_inside_filtered_browser_page() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.session_state = SessionState::Ready(RecentSessions {
        items: vec![
            sample_session("thread-1"),
            sample_session_with_workspace("thread-2", "/tmp/docs", "docs refresh"),
            sample_session_with_workspace("thread-3", "/tmp/docs", "docs release"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.selected_session_index = 7;
    app.session_overlay_ui_state.set_project_filter(
        crate::application::service::session_service::SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/docs".to_string(),
        },
    );
    app.session_overlay_ui_state.set_search_query("release");

    let view = build_session_overlay_view(&app);
    let detail = view
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(view.list_view.selected_index, Some(0));
    assert_eq!(view.list_view.items.len(), 1);
    assert!(detail.contains("id: thread-3"));
    assert!(detail.contains("browser: page 1 of 1 | showing 1-1 of 1 matches"));
}

#[test]
fn sessions_overlay_clear_key_resets_browser_state() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.session_state = SessionState::Ready(RecentSessions {
        items: vec![
            sample_session("thread-1"),
            sample_session_with_workspace("thread-2", "/tmp/docs", "docs refresh"),
            sample_session("thread-3"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.selected_session_index = 1;
    app.shell_overlay = ShellOverlay::Sessions;
    app.session_overlay_ui_state.set_search_query("docs");
    app.session_overlay_ui_state.set_project_filter(
        crate::application::service::session_service::SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/docs".to_string(),
        },
    );
    app.session_overlay_ui_state.move_page(2, 4);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE,)));

    assert_eq!(
        app.session_overlay_ui_state.browser_state().search_query,
        ""
    );
    assert_eq!(app.session_overlay_ui_state.browser_state().page_index, 0);
    assert_eq!(
        app.session_overlay_ui_state.browser_state().project_filter,
        crate::application::service::session_service::SessionProjectFilter::AllProjects
    );
    assert_eq!(
        app.session_overlay_ui_state.selected_session_id(),
        Some("thread-1")
    );
    assert!(!app.session_overlay_ui_state.is_search_query_editing());

    let view = build_session_overlay_view(&app);
    let detail = view
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(view.list_view.selected_index, Some(0));
    assert!(detail.contains("id: thread-1"));
}

#[test]
fn sessions_overlay_home_and_end_keys_jump_to_browser_edges() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.session_state = SessionState::Ready(RecentSessions {
        items: (1..=12)
            .map(|index| sample_session(&format!("thread-{index}")))
            .collect(),
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.shell_overlay = ShellOverlay::Sessions;
    app.session_overlay_ui_state.move_page(1, 2);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE,)));
    assert_eq!(app.session_overlay_ui_state.browser_state().page_index, 1);
    assert_eq!(
        app.session_overlay_ui_state.selected_session_id(),
        Some("thread-12")
    );

    let end_view = build_session_overlay_view(&app);
    let end_detail = end_view
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(end_view.list_view.selected_index, Some(1));
    assert!(end_detail.contains("id: thread-12"));

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE,)));
    assert_eq!(app.session_overlay_ui_state.browser_state().page_index, 0);
    assert_eq!(
        app.session_overlay_ui_state.selected_session_id(),
        Some("thread-1")
    );

    let home_view = build_session_overlay_view(&app);
    let home_detail = home_view
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(home_view.list_view.selected_index, Some(0));
    assert!(home_detail.contains("id: thread-1"));
}

#[test]
fn sessions_overlay_g_shortcuts_jump_to_browser_edges() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.session_state = SessionState::Ready(RecentSessions {
        items: (1..=12)
            .map(|index| sample_session(&format!("thread-{index}")))
            .collect(),
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.shell_overlay = ShellOverlay::Sessions;

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT,)));
    assert_eq!(app.session_overlay_ui_state.browser_state().page_index, 1);
    assert_eq!(
        app.session_overlay_ui_state.selected_session_id(),
        Some("thread-12")
    );

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE,)));
    assert_eq!(app.session_overlay_ui_state.browser_state().page_index, 0);
    assert_eq!(
        app.session_overlay_ui_state.selected_session_id(),
        Some("thread-1")
    );
}

#[test]
fn session_query_edit_commit_filters_results_and_surfaces_browser_state() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.session_state = SessionState::Ready(RecentSessions {
        items: vec![
            sample_session("thread-1"),
            sample_session_with_workspace("thread-2", "/tmp/docs", "docs refresh"),
            sample_session_with_workspace("thread-3", "/tmp/docs", "docs release"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.shell_overlay = ShellOverlay::Sessions;

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE,)));
    assert!(app.is_session_search_query_editing());
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    let view = build_session_overlay_view(&app);
    let detail = view
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        app.session_overlay_ui_state.browser_state().search_query,
        "docs"
    );
    assert!(!app.is_session_search_query_editing());
    assert_eq!(view.list_view.items.len(), 2);
    assert!(detail.contains("query: docs"));
    assert!(detail.contains("filter: all projects (3 recent sessions across 2 workspaces)"));
    assert!(detail.contains("context: current workspace (/tmp/root) has 1 recent session"));
    assert!(detail.contains("browser: page 1 of 1 | showing 1-2 of 2 matches"));
}

#[test]
fn session_query_edit_cancel_restores_saved_query() {
    let (mut app, _) = make_test_app();
    app.session_overlay_ui_state.set_search_query("release");
    app.shell_overlay = ShellOverlay::Sessions;

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)));

    assert_eq!(
        app.session_overlay_ui_state.browser_state().search_query,
        "release"
    );
    assert!(!app.is_session_search_query_editing());
    assert_eq!(
        app.session_overlay_ui_state.search_query_editor_buffer(),
        "release"
    );
}

#[test]
fn session_overlay_tab_cycles_recent_project_filters() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.session_state = SessionState::Ready(RecentSessions {
        items: vec![
            sample_session_with_workspace("thread-1", "/tmp/docs", "docs refresh"),
            sample_session_with_workspace("thread-2", "/tmp/docs", "docs release"),
            sample_session("thread-3"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.shell_overlay = ShellOverlay::Sessions;

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE,)));

    let view = build_session_overlay_view(&app);
    let detail = view
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        app.session_overlay_ui_state.browser_state().project_filter,
        crate::application::service::session_service::SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/docs".to_string(),
        }
    );
    assert_eq!(view.list_view.items.len(), 2);
    assert!(detail.contains("filter: /tmp/docs (2 recent sessions)"));
    assert!(detail.contains("context: current workspace (/tmp/root) has 1 recent session"));

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT,)));
    assert_eq!(
        app.session_overlay_ui_state.browser_state().project_filter,
        crate::application::service::session_service::SessionProjectFilter::AllProjects
    );
}

#[test]
fn followup_template_overlay_view_collects_preview_status_and_keys() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::Ready(conversation);

    let view = build_followup_template_overlay_view(&app);
    let list = view
        .list_view
        .items
        .iter()
        .map(|item| {
            item.lines
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n---\n");
    let preview = view
        .preview_lines
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

    assert!(
        view.header_lines[0]
            .to_string()
            .contains("Follow-Up Templates")
    );
    assert!(view.list_view.message_lines.is_none());
    assert_eq!(view.list_view.selected_index, Some(0));
    assert!(list.contains("builtin next-task"));
    assert!(preview.contains("Rendered Preview"));
    assert!(status.contains("automation:"));
    assert!(keys.contains("change template"));
    assert!(keys.contains("r: reload"));
}

#[test]
fn followup_template_preview_renders_selected_template_and_runtime_values() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 1;
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_followup_template_preview_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("selected: builtin plan-queue"));
    assert!(rendered.contains("preview thread id: thread-1"));
    assert!(rendered.contains("latest answer"));
    assert!(rendered.contains("AUTO_STOP"));
    assert!(rendered.contains("Rendered Preview"));
}

#[test]
fn followup_template_preview_uses_placeholder_without_agent_reply() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 1;
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_followup_template_preview_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("preview last_message: placeholder"));
    assert!(rendered.contains("(waiting for next agent reply)"));
}

#[test]
fn followup_template_preview_surfaces_planning_refresh_for_builtin_next_task() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(
        PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "next_task: none".to_string(),
            None,
        )
        .with_queue_idle_policy(
            crate::domain::planning::QueueIdlePolicy::ReviewAndEnqueue,
            Some(DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string()),
        ),
    );
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_followup_template_preview_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("planning: ready"));
    assert!(rendered.contains("planning detail: next_task: none"));
    assert!(rendered.contains(
        "A queue-manager planning worker reviews the direction goals after the current turn"
    ));
}

#[test]
fn followup_template_preview_hides_planner_session_debug_outside_debug_mode() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.planner_worker_panel_state = PlannerWorkerPanelState {
        status: PlannerWorkerStatus::RefreshSucceeded,
        last_operation_label: Some("refresh".to_string()),
        last_summary: Some("planner refreshed the queue".to_string()),
        last_rejected_summary: None,
        last_queue_summary: Some("next task: Implement shell planning status".to_string()),
        last_notice_detail: None,
        last_prompt: Some("planning worker prompt body".to_string()),
        last_response: Some("planner worker response body".to_string()),
        last_host_detail: None,
    };

    let rendered = build_followup_template_preview_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(!rendered.contains("Planner Session Debug"));
    assert!(!rendered.contains("planning worker prompt body"));
    assert!(!rendered.contains("planner worker response body"));
}

#[test]
fn followup_template_preview_shows_planner_session_debug_in_debug_mode() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.planner_visibility = PlannerVisibility::Debug;
    app.planner_worker_panel_state = PlannerWorkerPanelState {
        status: PlannerWorkerStatus::RefreshSucceeded,
        last_operation_label: Some("refresh".to_string()),
        last_summary: Some("planner refreshed the queue".to_string()),
        last_rejected_summary: None,
        last_queue_summary: Some("next task: Implement shell planning status".to_string()),
        last_notice_detail: None,
        last_prompt: Some("planning worker prompt body".to_string()),
        last_response: Some("planner worker response body".to_string()),
        last_host_detail: None,
    };

    let rendered = build_followup_template_preview_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Planner Session Debug"));
    assert!(rendered.contains("last planner session: refresh / refresh ok"));
    assert!(rendered.contains("planning worker prompt body"));
    assert!(rendered.contains("planner worker response body"));
}

#[test]
fn followup_template_preview_styles_planner_debug_headers_in_debug_mode() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.planner_visibility = PlannerVisibility::Debug;
    app.planner_worker_panel_state = PlannerWorkerPanelState {
        status: PlannerWorkerStatus::RefreshSucceeded,
        last_operation_label: Some("refresh".to_string()),
        last_summary: Some("planner refreshed the queue".to_string()),
        last_rejected_summary: None,
        last_queue_summary: None,
        last_notice_detail: None,
        last_prompt: Some("planning worker prompt body".to_string()),
        last_response: Some("planner worker response body".to_string()),
        last_host_detail: None,
    };

    let lines = build_followup_template_preview_lines(&app);
    let header_line = lines
        .iter()
        .find(|line| line.to_string() == "Planner Session Debug")
        .expect("debug header should be present");
    assert_eq!(header_line.spans[0].style.fg, Some(Color::Cyan));
    assert!(
        header_line.spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD)
    );

    let prompt_line = lines
        .iter()
        .find(|line| line.to_string() == "Prompt")
        .expect("prompt section header should be present");
    assert_eq!(prompt_line.spans[0].style.fg, Some(Color::Gray));
    assert!(
        prompt_line.spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD)
    );
}

#[test]
fn followup_template_preview_keeps_tail_lines_when_debug_block_is_truncated() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.planner_visibility = PlannerVisibility::Debug;
    let long_prompt = (0..300)
        .map(|index| format!("prompt line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.planner_worker_panel_state = PlannerWorkerPanelState {
        status: PlannerWorkerStatus::RefreshSucceeded,
        last_operation_label: Some("refresh".to_string()),
        last_summary: Some("planner refreshed the queue".to_string()),
        last_rejected_summary: None,
        last_queue_summary: None,
        last_notice_detail: None,
        last_prompt: Some(long_prompt),
        last_response: Some("planner worker response body".to_string()),
        last_host_detail: None,
    };

    let rendered = build_followup_template_preview_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("prompt line 127"));
    assert!(!rendered.contains("prompt line 150"));
    assert!(rendered.contains("prompt line 299"));
    assert!(rendered.contains("middle lines omitted in debug preview"));
    assert!(rendered.contains("worker received full text"));
}

#[test]
fn followup_template_overlay_navigation_updates_selection() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.show_followup_template_overlay();

    assert_eq!(app.shell_overlay, ShellOverlay::FollowupTemplates);
    assert_eq!(app.followup_template_selection(), Some(0));

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE,)));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should stay ready");
    };
    assert_eq!(
        conversation.auto_follow_state.template_label(),
        "builtin plan-queue"
    );
    assert!(conversation.status_text.contains("auto follow-up template"));
    assert_eq!(app.followup_template_selection(), Some(1));
    assert_eq!(app.followup_overlay_ui_state.preview_scroll, 0);
}

#[test]
fn followup_template_overlay_reload_refreshes_catalog_for_active_thread() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.cwd = "/tmp/root".to_string();
    conversation.auto_follow_state.enabled = false;
    conversation.auto_follow_state.set_max_auto_turns(7);
    conversation
        .auto_follow_state
        .set_stop_keyword_value("DONE".to_string());
    conversation
        .auto_follow_state
        .stop_rules
        .stop_on_no_file_changes = true;
    conversation.auto_follow_state.template_state.selected_index = 1;
    app.conversation_state = ConversationState::Ready(conversation);
    app.show_followup_template_overlay();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE,)));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should stay ready");
    };
    assert_eq!(conversation.auto_follow_state.template_count(), 5);
    assert_eq!(
        conversation.auto_follow_state.template_label(),
        "builtin plan-queue"
    );
    assert!(!conversation.auto_follow_state.enabled);
    assert_eq!(conversation.auto_follow_state.max_auto_turns_value(), 7);
    assert_eq!(conversation.auto_follow_state.stop_keyword_value(), "DONE");
    assert_eq!(
        conversation.auto_follow_state.no_file_change_stop_label(),
        "on"
    );
    assert!(
        conversation
            .status_text
            .contains("follow-up templates reloaded")
    );
}

#[test]
fn followup_template_overlay_reload_reports_noop_when_catalog_is_current() {
    let (mut app, _) = make_test_app();
    let conversation = ConversationViewModel::new_draft(
        "/tmp/root".to_string(),
        app.load_followup_template_catalog("/tmp/root"),
    );
    app.conversation_state = ConversationState::Ready(conversation);
    app.show_followup_template_overlay();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE,)));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should stay ready");
    };
    assert_eq!(conversation.auto_follow_state.template_count(), 5);
    assert!(
        conversation
            .status_text
            .contains("follow-up templates already up to date")
    );
}

#[test]
fn followup_template_overlay_reload_failure_keeps_existing_catalog() {
    let (mut app, _) = make_test_app();
    let mut conversation = ConversationViewModel::new_draft(
        "/tmp/failing".to_string(),
        app.load_followup_template_catalog("/tmp/root"),
    );
    conversation.auto_follow_state.template_state.selected_index = 4;
    app.conversation_state = ConversationState::Ready(conversation);
    app.show_followup_template_overlay();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE,)));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should stay ready");
    };
    assert_eq!(conversation.auto_follow_state.template_count(), 5);
    assert_eq!(
        conversation.auto_follow_state.template_label(),
        "workspace root-template"
    );
    assert!(
        conversation
            .status_text
            .contains("failed to reload workspace follow-up templates / keeping current catalog")
    );
}

#[test]
fn startup_overlay_ctrl_o_opens_sessions_overlay_and_starts_loading_when_ready() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.show_startup_overlay();

    assert!(
        app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL,))
    );

    assert_eq!(app.shell_overlay, ShellOverlay::Sessions);
    assert!(matches!(app.session_state, SessionState::Loading));
}

#[test]
fn sessions_overlay_reload_is_gated_until_startup_ready() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", false));
    app.session_state = SessionState::Failed("load failed".to_string());
    app.show_session_overlay();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE,)));

    assert_eq!(app.shell_overlay, ShellOverlay::Sessions);
    assert!(matches!(
        &app.session_state,
        SessionState::Failed(message) if message == "load failed"
    ));
}

#[test]
fn sessions_overlay_enter_opens_selected_session_and_dismisses_chrome() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.exit_confirmation_state = ExitConfirmationState::Visible;
    app.session_state = SessionState::Ready(RecentSessions {
        items: vec![sample_session("thread-1"), sample_session("thread-2")],
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.selected_session_index = 1;
    app.shell_overlay = ShellOverlay::Sessions;

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
    assert_eq!(app.exit_confirmation_state, ExitConfirmationState::Hidden);
    assert!(matches!(app.conversation_state, ConversationState::Loading));
    assert_eq!(
        app.active_session
            .as_ref()
            .map(|session| session.id.as_str()),
        Some("thread-2")
    );
}

#[test]
fn sessions_overlay_enter_while_turn_is_streaming_keeps_overlay_visible() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.thread_id = "thread-current".to_string();
    conversation.title = "Streaming thread".to_string();
    conversation.input_state = ConversationInputState::StreamingTurn;
    app.conversation_state = ConversationState::Ready(conversation);
    app.session_state = SessionState::Ready(RecentSessions {
        items: vec![sample_session("thread-2")],
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.shell_overlay = ShellOverlay::Sessions;

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    assert_eq!(app.shell_overlay, ShellOverlay::Sessions);
    assert!(app.active_session.is_none());
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(conversation.thread_id, "thread-current");
    assert!(
        conversation
            .status_text
            .contains("wait for completion before switching sessions")
    );
}

#[test]
fn sessions_overlay_page_controls_open_selected_filtered_page_session() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.session_overlay_ui_state = SessionOverlayUiState::new(1);
    app.session_state = SessionState::Ready(RecentSessions {
        items: vec![
            sample_session("thread-1"),
            sample_session_with_workspace("thread-2", "/tmp/docs", "docs refresh"),
            sample_session_with_workspace("thread-3", "/tmp/docs", "docs release"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.session_overlay_ui_state.set_search_query("docs");
    app.shell_overlay = ShellOverlay::Sessions;

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,)));

    let view = build_session_overlay_view(&app);
    let detail = view
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(view.list_view.selected_index, Some(0));
    assert!(detail.contains("id: thread-3"));
    assert!(detail.contains("browser: page 2 of 2 | showing 2-2 of 2 matches"));

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
    assert_eq!(
        app.active_session
            .as_ref()
            .map(|session| session.id.as_str()),
        Some("thread-3")
    );
}

#[test]
fn session_overlay_view_surfaces_ranked_query_results() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.session_state = SessionState::Ready(RecentSessions {
        items: vec![
            sample_session_with_workspace_at(
                "thread-1",
                "/tmp/root",
                "docs checklist",
                1_700_000_000,
            ),
            sample_session_with_workspace_at(
                "docs-thread-2",
                "/tmp/root",
                "release prep",
                1_699_999_900,
            ),
            sample_session_with_workspace_at(
                "thread-3",
                "/tmp/root",
                "docs rollout",
                1_700_000_100,
            ),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.session_overlay_ui_state.set_search_query("docs");

    let view = build_session_overlay_view(&app);
    let list = view
        .list_view
        .items
        .iter()
        .map(|item| {
            item.lines
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>();

    assert!(list[0].contains("docs-thread-2"));
    assert!(list[1].contains("thread-3"));
}

#[test]
fn session_overlay_view_describes_query_miss_inside_project_filter() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.session_state = SessionState::Ready(RecentSessions {
        items: vec![
            sample_session_with_workspace("thread-1", "/tmp/docs", "docs refresh"),
            sample_session_with_workspace("thread-2", "/tmp/docs", "docs release"),
            sample_session("thread-3"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.session_overlay_ui_state.set_project_filter(
        crate::application::service::session_service::SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/docs".to_string(),
        },
    );
    app.session_overlay_ui_state.set_search_query("missing");

    let view = build_session_overlay_view(&app);
    let list_message = view
        .list_view
        .message_lines
        .as_ref()
        .expect("query miss should show a list message")
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let detail = view
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(list_message.contains("no sessions in /tmp/docs match query \"missing\""));
    assert!(detail.contains("filter: /tmp/docs (2 recent sessions)"));
    assert!(detail.contains("browser: no matches in /tmp/docs across 2 recent sessions"));
    assert!(
        detail.contains(
            "Press c to clear the browser, Tab/BackTab to cycle filters, or r to reload."
        )
    );
}

#[test]
fn followup_template_overlay_enter_closes_overlay() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.show_followup_template_overlay();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
}

#[test]
fn followup_template_overlay_scroll_keys_update_preview_offset() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.show_followup_template_overlay();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,)));
    assert_eq!(
        app.followup_overlay_ui_state.preview_scroll,
        FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP
    );

    assert!(
        app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL,))
    );
    assert_eq!(
        app.followup_overlay_ui_state.preview_scroll,
        FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP.saturating_mul(2)
    );

    assert!(
        app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL,))
    );
    assert_eq!(
        app.followup_overlay_ui_state.preview_scroll,
        FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP
    );
}

#[test]
fn ctrl_g_starts_stop_keyword_edit_in_followup_overlay() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());

    app.start_stop_keyword_edit();

    assert_eq!(app.shell_overlay, ShellOverlay::FollowupTemplates);
    assert!(app.is_stop_keyword_editing());
    assert_eq!(
        app.followup_overlay_ui_state.stop_keyword_editor.buffer,
        DEFAULT_AUTO_FOLLOW_STOP_KEYWORD
    );
}

#[test]
fn ctrl_l_starts_max_auto_turns_edit_in_followup_overlay() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());

    app.start_max_auto_turns_edit();

    assert_eq!(app.shell_overlay, ShellOverlay::FollowupTemplates);
    assert!(app.is_max_auto_turns_editing());
    assert_eq!(
        app.followup_overlay_ui_state.max_auto_turns_editor.buffer,
        DEFAULT_AUTO_FOLLOW_MAX_TURNS.to_string()
    );
}

#[test]
fn max_auto_turns_edit_commit_updates_saved_value_and_preview() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::Ready(conversation);
    app.start_max_auto_turns_edit();
    app.followup_overlay_ui_state.max_auto_turns_editor.buffer = "5".to_string();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should stay ready");
    };
    assert_eq!(conversation.auto_follow_state.max_auto_turns_value(), 5);
    assert!(!app.is_max_auto_turns_editing());

    let rendered = build_followup_template_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("max auto turns: 5"));
}

#[test]
fn invalid_max_auto_turns_edit_keeps_editor_open() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.start_max_auto_turns_edit();
    app.followup_overlay_ui_state.max_auto_turns_editor.buffer = "51".to_string();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should stay ready");
    };
    assert_eq!(
        conversation.auto_follow_state.max_auto_turns_value(),
        DEFAULT_AUTO_FOLLOW_MAX_TURNS
    );
    assert!(app.is_max_auto_turns_editing());
    assert!(
        conversation
            .status_text
            .contains("whole number between 1 and 50")
    );
}

#[test]
fn max_auto_turns_edit_ignores_non_digit_input() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.start_max_auto_turns_edit();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE,)));
    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE,)));

    assert_eq!(
        app.followup_overlay_ui_state.max_auto_turns_editor.buffer,
        "34"
    );
}

#[test]
fn stop_keyword_edit_commit_updates_saved_value_and_preview() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 1;
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::Ready(conversation);
    app.start_stop_keyword_edit();
    app.followup_overlay_ui_state.stop_keyword_editor.buffer = "DONE".to_string();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should stay ready");
    };
    assert_eq!(conversation.auto_follow_state.stop_keyword_value(), "DONE");
    assert!(!app.is_stop_keyword_editing());

    let rendered = build_followup_template_preview_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("DONE"));
}

#[test]
fn invalid_stop_keyword_edit_keeps_editor_open() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.start_stop_keyword_edit();
    app.followup_overlay_ui_state.stop_keyword_editor.buffer = "two words".to_string();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should stay ready");
    };
    assert_eq!(
        conversation.auto_follow_state.stop_keyword_value(),
        DEFAULT_AUTO_FOLLOW_STOP_KEYWORD
    );
    assert!(app.is_stop_keyword_editing());
    assert!(
        conversation
            .status_text
            .contains("letters, numbers, or underscores")
    );
}

#[test]
fn followup_template_status_lines_include_latest_status_text() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.status_text =
        "auto stop keyword must use only letters, numbers, or underscores".to_string();
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_followup_template_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        rendered
            .contains("status: auto stop keyword must use only letters, numbers, or underscores")
    );
}

#[test]
fn followup_template_status_lines_include_warning_summary_detail() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.template_warnings = vec![
        "template catalog reloaded with fallback".to_string(),
        "workspace template missing".to_string(),
    ];
    conversation.warnings = conversation.template_warnings.clone();
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_followup_template_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("template warnings (2): workspace template missing"));
}

#[test]
fn followup_template_status_lines_include_runtime_notice_summary() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.template_warnings = vec!["workspace template missing".to_string()];
    conversation.warnings = conversation.template_warnings.clone();
    conversation.runtime_notices =
        vec!["shared runtime reconnected after the previous app-server process exited".to_string()];
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_followup_template_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("template warning: workspace template missing"));
    assert!(rendered.contains("runtime: shared runtime reconnected"));
}

#[test]
fn followup_template_status_lines_surface_planning_queue_failure_and_notice() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: rank 1 / task-1 / Implement shell planning status",
    ));
    conversation.planning_repair_state = Some(PlanningRepairState {
        root_turn_id: "turn-root".to_string(),
        attempts_used: 1,
        max_attempts: 2,
        latest_request: PlanningRepairRequest {
            failure_summary: "task-ledger.json is missing direction_id".to_string(),
            validation_errors: vec!["task-ledger.json is missing direction_id".to_string()],
            directions_toml: "version = 1".to_string(),
            task_ledger_schema_json: "{\"type\":\"object\"}".to_string(),
            accepted_task_ledger_json: "{\"version\":1,\"tasks\":[]}".to_string(),
            rejected_task_ledger_json: None,
            rejected_archive_path: None,
        },
    });
    conversation.runtime_notices =
        vec!["planning reconciliation restored protected directions.toml".to_string()];
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_followup_template_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("planning status: repairing"));
    assert!(rendered.contains("planning repair attempt: 1/2"));
    assert!(rendered.contains("planning queue head: next task: rank 1 / task-1"));
    assert!(rendered.contains("last planning failure: task-ledger.json is missing direction_id"));
    assert!(rendered.contains("planning: planning reconciliation restored protected"));
}

#[test]
fn followup_template_status_lines_hide_planner_panel_outside_debug_mode() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: rank 1 / task-1 / Implement shell planning status",
    ));
    app.conversation_state = ConversationState::Ready(conversation);
    app.planner_worker_panel_state = PlannerWorkerPanelState {
        status: PlannerWorkerStatus::RefreshSucceeded,
        last_operation_label: Some("refresh".to_string()),
        last_summary: Some("planner refreshed the queue".to_string()),
        last_rejected_summary: None,
        last_queue_summary: Some("next task: Implement shell planning status".to_string()),
        last_notice_detail: Some(
            "planning reconciliation restored protected queue.snapshot.json".to_string(),
        ),
        last_prompt: Some("refresh prompt body".to_string()),
        last_response: Some("refresh response body".to_string()),
        last_host_detail: Some("host promoted top follow-up proposal".to_string()),
    };

    let rendered = build_followup_template_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("planner detail: normal"));
    assert!(!rendered.contains("planner status:"));
    assert!(!rendered.contains("planner notice:"));
    assert!(!rendered.contains("planner host detail:"));
}

#[test]
fn followup_template_status_lines_show_planner_panel_in_debug_mode() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: rank 1 / task-1 / Implement shell planning status",
    ));
    app.conversation_state = ConversationState::Ready(conversation);
    app.planner_visibility = PlannerVisibility::Debug;
    app.planner_worker_panel_state = PlannerWorkerPanelState {
        status: PlannerWorkerStatus::RefreshSucceeded,
        last_operation_label: Some("refresh".to_string()),
        last_summary: Some("planner refreshed the queue".to_string()),
        last_rejected_summary: None,
        last_queue_summary: Some("next task: Implement shell planning status".to_string()),
        last_notice_detail: Some(
            "planning reconciliation restored protected queue.snapshot.json".to_string(),
        ),
        last_prompt: Some("refresh prompt body".to_string()),
        last_response: Some("refresh response body".to_string()),
        last_host_detail: Some("host promoted top follow-up proposal".to_string()),
    };

    let rendered = build_followup_template_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("planner detail: debug"));
    assert!(rendered.contains("planner status: refresh ok"));
    assert!(rendered.contains("planner notice: planning reconciliation restored protected"));
    assert!(rendered.contains("planner host detail: host promoted top follow-up proposal"));
}

#[test]
fn followup_template_status_lines_surface_proposed_followups_when_queue_is_idle() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(PlanningRuntimeSnapshot::ready_with_details(
        "Planning Context".to_string(),
        "queue idle: no executable planning task".to_string(),
        Some(
            "2 promotable follow-up proposals available: Draft roadmap | Draft checklist"
                .to_string(),
        ),
        None,
    ));
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_followup_template_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("planning status: valid"));
    assert!(rendered.contains("planning queue: queue idle: no executable planning task"));
    assert!(rendered.contains("planning proposals: 2 promotable follow-up proposals"));
}

#[test]
fn followup_template_status_lines_include_max_auto_turns_value() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.auto_follow_state.set_max_auto_turns(5);
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_followup_template_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("max auto turns: 5"));
}

#[test]
fn followup_template_status_lines_include_recent_tool_activity() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.turn_activity.last_completed_turn_command_count = 2;
    conversation.turn_activity.last_completed_turn_last_summary =
        Some("command: cargo test [completed]".to_string());
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_followup_template_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("last turn commands: 2"));
    assert!(rendered.contains("last turn tool activity: command: cargo test [completed]"));
}

#[test]
fn followup_template_status_lines_include_approval_review_summary() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.approval_review = Some(ConversationApprovalReview {
        target_item_id: "command-1".to_string(),
        status: ConversationApprovalReviewStatus::InProgress,
        risk_level: Some("high".to_string()),
        rationale: None,
    });
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_followup_template_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("approval: reviewing high"));
}

#[test]
fn followup_template_status_lines_include_github_review_change_summary() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.github_review_polling_state = GithubReviewPollingState::active(
        super::github_polling::GithubReviewPollingConfig {
            target: GithubPullRequestTarget::new("acme/widgets", 42),
            interval: std::time::Duration::from_secs(30),
        },
        std::time::Instant::now(),
    );
    app.record_github_review_poll_result(
        std::time::Instant::now(),
        Ok(sample_github_review_poll_result()),
    );

    let rendered = build_followup_template_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("github: review commented by r..."));
}

#[test]
fn followup_template_status_lines_fit_default_overlay_budget() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());

    let lines = build_followup_template_status_lines(&app);

    assert_eq!(lines.len(), 11);
}

#[test]
fn followup_template_status_lines_fit_edit_overlay_budget() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.start_max_auto_turns_edit();

    let lines = build_followup_template_status_lines(&app);

    assert_eq!(lines.len(), 11);
}

#[test]
fn stop_keyword_edit_cancel_restores_saved_value() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.start_stop_keyword_edit();
    app.followup_overlay_ui_state.stop_keyword_editor.buffer = "DONE".to_string();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)));

    assert!(!app.is_stop_keyword_editing());
    assert_eq!(
        app.followup_overlay_ui_state.stop_keyword_editor.buffer,
        DEFAULT_AUTO_FOLLOW_STOP_KEYWORD
    );
}

#[test]
fn max_auto_turns_edit_cancel_restores_saved_value() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Ready(ready_conversation());
    app.start_max_auto_turns_edit();
    app.followup_overlay_ui_state.max_auto_turns_editor.buffer = "9".to_string();

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)));

    assert!(!app.is_max_auto_turns_editing());
    assert_eq!(
        app.followup_overlay_ui_state.max_auto_turns_editor.buffer,
        DEFAULT_AUTO_FOLLOW_MAX_TURNS.to_string()
    );
}

#[test]
fn auto_followup_skip_reason_is_visible_in_status_footer() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation
        .auto_follow_state
        .stop_rules
        .stop_on_no_file_changes = true;
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::Ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("notice: auto: stopped: no file changes"));
    assert!(rendered.contains("detail: the last completed turn changed 0 files"));
    assert!(!rendered.contains("turn-1"));
}

#[test]
fn auto_followup_queue_clears_previous_skip_reason_from_status_footer() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 1;
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: task-1",
    ));
    conversation.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
        summary: "stopped: automation off".to_string(),
        detail: "post-turn automation is off; toggle Ctrl+a to re-enable it".to_string(),
    });
    conversation
        .turn_activity
        .last_completed_turn_file_change_count = 2;
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::Ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-2".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("notice: auto: queued auto turn 1/3"));
    assert!(rendered.contains("detail: queued after the previous turn completed"));
    assert!(!rendered.contains("turn-2"));
}

#[test]
fn inline_tail_hides_raw_turn_ids_after_auto_followup_status_updates() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 1;
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: task-1",
    ));
    conversation
        .turn_activity
        .last_completed_turn_file_change_count = 1;
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::Ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "019d7032-fa43-7a62-a7b4-5328f373bb90".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("notice: auto: queued auto turn 1/3"));
    assert!(!rendered.contains("019d7032-fa43-7a62-a7b4-5328f373bb90"));
}

#[test]
fn inline_tail_surfaces_stale_planning_status_while_turn_is_running() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: rank 1 / task-1 / Implement shell planning status",
    ));
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("planning: stale  |  queue: next task: rank 1 / task-1"));
}

#[test]
fn inline_tail_surfaces_manual_turn_working_line() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());
    conversation.active_turn_started_at = Some(std::time::Instant::now() - Duration::from_secs(45));
    conversation.live_agent_message = Some(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "still streaming".to_string(),
        Some("commentary".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("◦ Working (45s • turn running)"));
}

#[test]
fn shell_footer_surfaces_recent_tool_activity_summary() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.turn_activity.last_completed_turn_command_count = 1;
    conversation
        .turn_activity
        .last_completed_turn_file_change_count = 3;
    conversation.turn_activity.last_completed_turn_last_summary =
        Some("file change: update src/app.rs".to_string());
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("notice: tool activity: file change: update src/app.rs"));
    assert!(rendered.contains("last turn commands: 1"));
    assert!(rendered.contains("last turn file changes: 3"));
}

#[test]
fn shell_footer_surfaces_warning_summary_detail() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.template_warnings = vec!["workspace template missing".to_string()];
    conversation.warnings = conversation.template_warnings.clone();
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("template warning: workspace template missing"));
}

#[test]
fn shell_footer_prioritizes_runtime_warning_summary_when_runtime_and_template_mix() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.base_warnings =
        vec!["shared runtime reconnected after the previous app-server process exited".to_string()];
    conversation.template_warnings = vec!["workspace template missing".to_string()];
    conversation.warnings = conversation
        .base_warnings
        .iter()
        .chain(conversation.template_warnings.iter())
        .cloned()
        .collect();
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("warnings: runtime 1, template 1"));
}

#[test]
fn shell_footer_surfaces_runtime_notice_summary() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.template_warnings = vec!["workspace template missing".to_string()];
    conversation.warnings = conversation.template_warnings.clone();
    conversation.runtime_notices =
        vec!["shared runtime reconnected after the previous app-server process exited".to_string()];
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("template warning: workspace template missing"));
    assert!(rendered.contains("runtime: shared runtime reconnected"));
}

#[test]
fn shell_footer_surfaces_planning_summary_and_notice() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: rank 1 / task-1 / Implement shell planning status",
    ));
    conversation.runtime_notices =
        vec!["planning reconciliation restored protected directions.toml".to_string()];
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("planning: valid  |  queue: next task: rank 1 / task-1"));
    assert!(
        rendered.contains("planning notice: planning: planning reconciliation restored protected")
    );
}

#[test]
fn shell_footer_surfaces_approval_review_summary() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.turn_activity.last_completed_turn_command_count = 1;
    conversation
        .turn_activity
        .last_completed_turn_file_change_count = 2;
    conversation.approval_review = Some(ConversationApprovalReview {
        target_item_id: "command-1".to_string(),
        status: ConversationApprovalReviewStatus::Approved,
        risk_level: Some("medium".to_string()),
        rationale: None,
    });
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("notice: tool activity: none"));
    assert!(rendered.contains("approval: approved medium"));
    assert!(rendered.contains("last turn commands: 1"));
    assert!(rendered.contains("last turn file changes: 2"));
}

#[test]
fn shell_footer_shows_current_turn_activity_while_streaming() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());
    conversation.turn_activity.current_turn_command_count = 1;
    conversation.turn_activity.current_turn_file_change_count = 2;
    conversation.turn_activity.current_turn_last_summary =
        Some("command: cargo test [running]".to_string());
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("notice: tool activity: command: cargo test [running]"));
    assert!(rendered.contains("current turn commands: 1"));
    assert!(rendered.contains("current turn file changes: 2"));
}

#[test]
fn recorded_limit_skip_detail_stays_stable_after_progress_resets() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.auto_follow_state.completed_auto_turns =
        conversation.auto_follow_state.max_auto_turns;
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::Ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-limit".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should remain ready");
    };
    conversation.auto_follow_state.completed_auto_turns = 0;

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("notice: auto: stopped: turn limit reached"));
    assert!(rendered.contains("detail: reached the configured auto-turn budget (3/3)"));
    assert!(!rendered.contains("detail: reached the configured auto-turn budget (0/3)"));
}

#[test]
fn shell_footer_surfaces_auto_follow_working_line_and_completed_progress() {
    let (mut app, _) = make_test_app();
    let mut conversation = ready_conversation();
    conversation.auto_follow_state.runtime_phase = AutoFollowRuntimePhase::Evaluating {
        started_at: std::time::Instant::now() - Duration::from_secs(130),
    };
    app.conversation_state = ConversationState::Ready(conversation);

    let rendered = build_shell_footer_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("auto: on / evaluating next turn"));
    assert!(rendered.contains("progress: 0/3 done"));
    assert!(rendered.contains("◦ Working (2m 10s • evaluating next auto follow-up)"));
}

#[test]
fn github_review_poll_result_updates_snapshot_and_recent_changes() {
    let (mut app, _) = make_test_app();
    app.github_review_polling_state = GithubReviewPollingState::active(
        super::github_polling::GithubReviewPollingConfig {
            target: GithubPullRequestTarget::new("acme/widgets", 42),
            interval: std::time::Duration::from_secs(30),
        },
        std::time::Instant::now(),
    );

    app.record_github_review_poll_result(
        std::time::Instant::now(),
        Ok(sample_github_review_poll_result()),
    );

    let GithubReviewPollingState::Active(polling_state) = &app.github_review_polling_state else {
        panic!("expected active github review polling state");
    };
    assert_eq!(polling_state.recent_changes.len(), 1);
    assert_eq!(
        polling_state
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.events.len()),
        Some(1)
    );
    assert!(polling_state.last_error.is_none());
}

fn sample_github_review_poll_result() -> GithubPullRequestPollResult {
    let target = GithubPullRequestTarget::new("acme/widgets", 42);
    let snapshot = GithubPullRequestActivitySnapshot {
        target,
        title: "Track review state".to_string(),
        url: "https://example.invalid/pr/42".to_string(),
        head_branch: "feature/native-github-poll-scheduling".to_string(),
        base_branch: "prerelease".to_string(),
        events: vec![GithubPullRequestActivityEvent {
            id: 100,
            kind: GithubPullRequestActivityKind::Review,
            submitted_at: "2026-04-08T09:00:00Z".to_string(),
            author_login: "reviewer".to_string(),
            body: "Looks good".to_string(),
            state: Some("COMMENTED".to_string()),
            url: "https://example.invalid/pr/42#review-100".to_string(),
            path: None,
        }],
    };

    GithubPullRequestPollResult {
        next_state: snapshot.poll_state(),
        changes: snapshot.events.clone(),
        snapshot,
    }
}

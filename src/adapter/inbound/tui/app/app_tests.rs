use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::Line;

use super::conversation_model::PlanningRepairState;
use super::shell_presentation::{
    build_inline_prompt_cursor_offset, build_input_prompt_cursor_offset,
};
use super::{
    ActiveTurnPlanningCapture, AutoFollowRuntimePhase, AutoFollowState, AutoFollowupSubmitContext,
    BackgroundMessage, ConversationInputState, ConversationMessage, ConversationMessageKind,
    ConversationRuntimeEffect, ConversationRuntimeEvent, ConversationState, ConversationViewModel,
    DEFAULT_AUTO_FOLLOW_MAX_TURNS, DEFAULT_AUTO_FOLLOW_STOP_KEYWORD,
    DirectionsMaintenanceOverlayStep, ExitConfirmationState, FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP,
    GithubReviewPollingState, InlineShellCommand, InlineShellCommandInput, MAX_COMPOSER_HEIGHT,
    NativeTuiApp, PlannerVisibility, PlannerWorkerPanelState, PlannerWorkerStatus,
    PlanningInitOverlayStep, PromptOrigin, RecordedAutoFollowupActivity, SessionOverlayUiState,
    SessionState, ShellActionAvailability, ShellFrontendMode, ShellOverlay, StartupState,
    TurnActivityState, build_conversation_shell_frame_view, build_conversation_shell_view,
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
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::followup_template_service::FollowupTemplateService;
use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::application::service::planning::PlanningServices;
use crate::application::service::planning::PlanningTaskHandoff;
use crate::application::service::planning::{PlanningExecutionSnapshot, PlanningRepairRequest};
use crate::application::service::planning_bootstrap_service::{
    PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning_contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLAN_OFF_FILE_PATH, TASK_LEDGER_FILE_PATH,
};
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::followup_template::{
    FollowupTemplateCatalog, FollowupTemplateDefinition, FollowupTemplateSource,
};
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
    planning_file_writes: Vec<(String, String)>,
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
            Some(cwd),
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
            None,
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
    workspace_directory: Option<&str>,
    event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    behavior: FakeStreamBehavior,
) -> Result<()> {
    if let Some(workspace_directory) = workspace_directory {
        for (relative_path, body) in &behavior.planning_file_writes {
            let file_path = Path::new(workspace_directory).join(relative_path);
            fs::create_dir_all(
                file_path
                    .parent()
                    .expect("fake planning file should have a parent directory"),
            )
            .expect("fake planning directory should be created");
            fs::write(&file_path, body).expect("fake planning file should write");
        }
    }

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

struct TempWorkspace {
    path: String,
}

impl TempWorkspace {
    fn new(prefix: &str) -> Self {
        Self {
            path: create_temp_workspace(prefix),
        }
    }

    fn path(&self) -> &str {
        self.path.as_str()
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

fn wait_for_new_thread_prompt(
    codex_port: &Arc<FakeCodexAppServerPort>,
    predicate: impl Fn(&str) -> bool,
) -> String {
    let timeout = Duration::from_millis(500);
    let poll_interval = Duration::from_millis(5);
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(prompt) = codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .find(|prompt| predicate(prompt))
        {
            return prompt;
        }
        assert!(
            Instant::now() < deadline,
            "manual submit should reach the codex app-server port within {timeout:?}"
        );
        thread::sleep(poll_interval);
    }
}

fn bootstrap_active_planning_workspace(workspace_dir: &str) {
    let planning =
        PlanningServices::from_workspace_port(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    let stage_result = planning
        .workspace
        .stage_simple_mode_draft(workspace_dir)
        .expect("planning workspace should stage");
    let promote_result = planning
        .workspace
        .promote_staged_draft(workspace_dir, &stage_result.draft_name)
        .expect("planning workspace should promote");
    assert!(
        promote_result.promoted_file_count > 0,
        "bootstrap planning workspace should become ready"
    );
}

fn rewrite_active_directions_toml(workspace_dir: &str, f: impl FnOnce(String) -> String) {
    let directions_path = std::path::Path::new(workspace_dir)
        .join(".codex-exec-loop")
        .join("planning")
        .join("directions.toml");
    let directions =
        std::fs::read_to_string(&directions_path).expect("directions.toml should be readable");
    std::fs::write(&directions_path, f(directions)).expect("updated directions.toml should write");
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
        inline_shell_command_palette_state: Default::default(),
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
        repeated_planning_queue_head_count: 0,
        status_text: "thread loaded".to_string(),
    }
}

#[path = "app_tests/input_copy_tests.rs"]
mod input_copy_tests;

#[path = "app_tests/planning_runtime_tests.rs"]
mod planning_runtime_tests;

#[path = "app_tests/shell_surface_tests.rs"]
mod shell_surface_tests;

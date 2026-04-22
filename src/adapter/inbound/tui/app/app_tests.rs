use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::conversation_model::PlanningRepairState;
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
    TurnActivityState, build_automation_overlay_view, build_automation_preview_lines,
    build_automation_status_lines, build_conversation_shell_frame_view,
    build_conversation_shell_view, build_inline_tail_lines, build_planning_init_overlay_view,
    build_queue_overlay_view, build_ready_input_lines, build_session_overlay_view,
    build_startup_overlay_view, build_status_title, build_transcript_panel_view,
    format_conversation_lines, shell_layout, startup_ascii_art_enabled_from_value,
};
use crate::adapter::inbound::tui::app::test_helpers::{
    sample_planning_runtime_snapshot, sample_proposal_only_planning_runtime_snapshot,
};
use crate::adapter::outbound::app_server::{
    AppServerPlanningWorkerAdapter, PlanningThreadLauncher,
};
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::codex_app_server_port::{
    AppServerStartupContext, CodexAppServerPort,
};
use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::application::service::planning::PlanningServices;
use crate::application::service::planning::PlanningTaskHandoff;
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLAN_OFF_FILE_PATH, TASK_LEDGER_FILE_PATH,
};
use crate::application::service::planning::{PlanningExecutionSnapshot, PlanningRepairRequest};
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
};
use crate::domain::recent_sessions::{RecentSessions, SessionCatalog};
use crate::domain::session_summary::SessionSummary;
use crate::domain::startup_diagnostics::StartupDiagnostics;

#[derive(Default)]
struct FakeCodexAppServerPort {
    new_thread_calls: Mutex<Vec<(String, String)>>,
    hidden_planning_calls: Mutex<Vec<(String, String)>>,
    turn_calls: Mutex<Vec<(String, String)>>,
    new_thread_stream_behavior: Mutex<FakeStreamBehavior>,
    hidden_planning_stream_behavior: Mutex<FakeStreamBehavior>,
    turn_stream_behavior: Mutex<FakeStreamBehavior>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct FakeStreamBehavior {
    events: Vec<ConversationStreamEvent>,
    error: Option<String>,
    planning_file_writes: Vec<(String, String)>,
    merge_active_branch_into_akra_repo: Option<String>,
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

    fn load_recent_sessions(&self, _limit: usize) -> Result<SessionCatalog> {
        Ok(RecentSessions {
            items: Vec::new(),
            warnings: Vec::new(),
            next_cursor: None,
        }
        .into())
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
        self.hidden_planning_calls
            .lock()
            .expect("hidden planning call mutex poisoned")
            .push((workspace_directory.to_string(), prompt.to_string()));
        self.new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .push((workspace_directory.to_string(), prompt.to_string()));
        let hidden_behavior = self
            .hidden_planning_stream_behavior
            .lock()
            .expect("hidden planning stream behavior mutex poisoned")
            .clone();
        let behavior = if hidden_behavior == FakeStreamBehavior::default() {
            self.new_thread_stream_behavior
                .lock()
                .expect("new-thread stream behavior mutex poisoned")
                .clone()
        } else {
            hidden_behavior
        };
        run_fake_stream(Some(workspace_directory), event_sender, behavior)
    }
}

fn run_fake_stream(
    workspace_directory: Option<&str>,
    event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    behavior: FakeStreamBehavior,
) -> Result<()> {
    if let Some(workspace_directory) = workspace_directory {
        for (relative_path, body) in &behavior.planning_file_writes {
            replace_candidate_planning_workspace_file(workspace_directory, relative_path, body);
        }
    }

    for event in behavior.events {
        let _ = event_sender.send(event);
    }

    if let (Some(workspace_directory), Some(repo_root)) = (
        workspace_directory,
        behavior.merge_active_branch_into_akra_repo.as_deref(),
    ) {
        merge_active_branch_into_akra(repo_root, workspace_directory);
    }

    if let Some(error) = behavior.error {
        Err(anyhow::anyhow!(error))
    } else {
        Ok(())
    }
}

fn make_test_app() -> (NativeTuiApp, Arc<FakeCodexAppServerPort>) {
    let codex_port = Arc::new(FakeCodexAppServerPort::default());
    let mut app = NativeTuiApp::new(
        StartupService::new(codex_port.clone()),
        SessionService::new(codex_port.clone()),
        ConversationService::new(codex_port.clone()),
        PlanningServices::from_ports(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            Arc::new(AppServerPlanningWorkerAdapter::new(codex_port.clone())),
        ),
    );
    app.show_startup_ascii_art = false;

    (app, codex_port)
}

#[derive(Debug, Clone, Default)]
struct ReadyGithubAutomationPort {
    head_branch: Arc<Mutex<Option<String>>>,
    base_branch: Arc<Mutex<Option<String>>>,
}

impl GithubAutomationPort for ReadyGithubAutomationPort {
    fn inspect_capabilities(&self, _repo_root: &str) -> GithubAutomationCapabilities {
        GithubAutomationCapabilities::new(
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Ready,
                "test push remote is ready",
                None,
            ),
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GhBinary,
                ParallelModeCapabilityState::Ready,
                "test gh binary is ready",
                None,
            ),
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GhAuth,
                ParallelModeCapabilityState::Ready,
                "test gh auth is ready",
                None,
            ),
        )
    }

    fn push_branch(
        &self,
        _repo_root: &str,
        _branch_name: &str,
        _force_with_lease: bool,
    ) -> Result<()> {
        Ok(())
    }

    fn ensure_pull_request(
        &self,
        _repo_root: &str,
        base_branch: &str,
        head_branch: &str,
        _title: &str,
        _body: &str,
    ) -> Result<GithubAutomationPullRequest> {
        *self
            .head_branch
            .lock()
            .expect("fake github head branch mutex poisoned") = Some(head_branch.to_string());
        *self
            .base_branch
            .lock()
            .expect("fake github base branch mutex poisoned") = Some(base_branch.to_string());
        Ok(GithubAutomationPullRequest::new(
            42,
            "https://github.com/RefinedStone/codex-exec-loop/pull/42",
            "OPEN",
            base_branch,
            head_branch,
            false,
        ))
    }

    fn inspect_pull_request(
        &self,
        _repo_root: &str,
        pr_number: u64,
    ) -> Result<GithubAutomationPullRequest> {
        let base_branch = self
            .base_branch
            .lock()
            .expect("fake github base branch mutex poisoned")
            .clone()
            .unwrap_or_else(|| "akra".to_string());
        let head_branch = self
            .head_branch
            .lock()
            .expect("fake github head branch mutex poisoned")
            .clone()
            .unwrap_or_else(|| "akra-agent/slot-1/task".to_string());
        Ok(GithubAutomationPullRequest::new(
            pr_number,
            format!("https://github.com/RefinedStone/codex-exec-loop/pull/{pr_number}"),
            "OPEN",
            base_branch,
            head_branch,
            false,
        ))
    }

    fn push_integration_branch(&self, _repo_root: &str, _branch_name: &str) -> Result<()> {
        Ok(())
    }

    fn close_pull_request(&self, _repo_root: &str, _pr_number: u64) -> Result<()> {
        Ok(())
    }
}

fn install_ready_github_automation(app: &mut NativeTuiApp) {
    app.parallel_mode_service =
        ParallelModeService::with_github_automation(Arc::new(ReadyGithubAutomationPort::default()));
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

fn create_temp_workspace(prefix: &str) -> String {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be valid")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
    std::fs::create_dir_all(&path).expect("temp workspace should be created");
    path.display().to_string()
}

struct TempGitWorkspace {
    root: String,
}

impl TempGitWorkspace {
    fn new(prefix: &str) -> Self {
        let root = create_temp_workspace(prefix);
        run_git(&root, &["init"]);
        run_git(&root, &["config", "user.name", "Codex Tests"]);
        run_git(&root, &["config", "user.email", "codex-tests@example.com"]);
        fs::write(Path::new(&root).join("README.md"), "workspace\n")
            .expect("git workspace readme should write");
        run_git(&root, &["add", "README.md"]);
        run_git(&root, &["commit", "-m", "initial"]);
        run_git(&root, &["branch", "akra"]);

        Self { root }
    }

    fn workspace_dir(&self) -> &str {
        self.root.as_str()
    }
}

impl Drop for TempGitWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run_git(repo_root: &str, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git command should spawn");
    assert!(
        output.status.success(),
        "git command should succeed: git {:?}\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn merge_active_branch_into_akra(repo_root: &str, slot_workspace_directory: &str) {
    let branch_name = current_git_branch(slot_workspace_directory);
    let original_branch = current_git_branch(repo_root);
    run_git(repo_root, &["checkout", "akra"]);
    run_git(repo_root, &["merge", "--ff-only", branch_name.as_str()]);
    run_git(repo_root, &["checkout", original_branch.as_str()]);
}

fn git_branch_exists(repo_root: &str, branch_name: &str) -> bool {
    Command::new("git")
        .current_dir(repo_root)
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch_name}"),
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .is_ok_and(|status| status.success())
}

fn current_git_branch(workspace_directory: &str) -> String {
    let output = Command::new("git")
        .current_dir(workspace_directory)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git rev-parse should spawn");
    assert!(
        output.status.success(),
        "git rev-parse should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("branch name should be utf-8")
        .trim()
        .to_string()
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

fn commit_active_planning_workspace_into_akra(workspace_dir: &str) {
    seed_ready_active_planning_workspace(workspace_dir);
    seed_ready_candidate_planning_workspace(workspace_dir);
    run_git(workspace_dir, &["add", ".codex-exec-loop"]);
    run_git(
        workspace_dir,
        &["commit", "-m", "Bootstrap planning workspace"],
    );
    merge_active_branch_into_akra(workspace_dir, workspace_dir);
}

fn seed_ready_active_planning_workspace(workspace_dir: &str) {
    let workspace_adapter = FilesystemPlanningWorkspaceAdapter::new();
    let bootstrap =
        PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);
    workspace_adapter
        .commit_planning_workspace_files(
            workspace_dir,
            &PlanningWorkspaceLoadRecord {
                directions_toml: Some(bootstrap.directions_toml),
                task_ledger_json: Some(bootstrap.task_ledger_json),
                task_ledger_schema_json: Some(bootstrap.task_ledger_schema_json),
                queue_snapshot_json: None,
                result_output_markdown: Some(bootstrap.result_output_markdown),
            },
        )
        .expect("bootstrap planning workspace should commit");
    for supplemental_file in bootstrap.supplemental_files {
        workspace_adapter
            .replace_planning_workspace_file(
                workspace_dir,
                &supplemental_file.active_path,
                Some(&supplemental_file.body),
            )
            .expect("bootstrap planning supplemental file should write");
    }
    let seeded_workspace = workspace_adapter
        .load_planning_workspace_files(workspace_dir)
        .expect("seeded planning workspace should load");
    assert!(
        seeded_workspace.directions_toml.is_some()
            && seeded_workspace.task_ledger_json.is_some()
            && seeded_workspace.task_ledger_schema_json.is_some()
            && seeded_workspace.result_output_markdown.is_some(),
        "seeded planning workspace should contain the active contract files"
    );
}

fn seed_ready_candidate_planning_workspace(workspace_dir: &str) {
    let bootstrap =
        PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);
    replace_candidate_planning_workspace_file(
        workspace_dir,
        &bootstrap.directions_path,
        &bootstrap.directions_toml,
    );
    replace_candidate_planning_workspace_file(
        workspace_dir,
        &bootstrap.task_ledger_path,
        &bootstrap.task_ledger_json,
    );
    replace_candidate_planning_workspace_file(
        workspace_dir,
        &bootstrap.task_ledger_schema_path,
        &bootstrap.task_ledger_schema_json,
    );
    replace_candidate_planning_workspace_file(
        workspace_dir,
        &bootstrap.result_output_path,
        &bootstrap.result_output_markdown,
    );
    for supplemental_file in bootstrap.supplemental_files {
        replace_candidate_planning_workspace_file(
            workspace_dir,
            &supplemental_file.active_path,
            &supplemental_file.body,
        );
    }
}

fn replace_active_planning_workspace_file(workspace_dir: &str, relative_path: &str, body: &str) {
    FilesystemPlanningWorkspaceAdapter::new()
        .replace_planning_workspace_file(workspace_dir, relative_path, Some(body))
        .expect("active planning workspace file should write");
    replace_candidate_planning_workspace_file(workspace_dir, relative_path, body);
}

fn replace_candidate_planning_workspace_file(workspace_dir: &str, relative_path: &str, body: &str) {
    let path = Path::new(workspace_dir).join(relative_path);
    fs::create_dir_all(
        path.parent()
            .expect("candidate planning workspace file should have a parent"),
    )
    .expect("candidate planning workspace directory should exist");
    fs::write(&path, body).expect("candidate planning workspace file should write");
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
        auto_follow_state: AutoFollowState::new(),
        planning_runtime_snapshot: PlanningRuntimeSnapshot::uninitialized(),
        turn_activity: TurnActivityState::default(),
        approval_review: None,
        last_auto_followup_activity: None,
        last_planning_task_handoff: None,
        status_text: "thread loaded".to_string(),
    }
}

#[path = "app_tests/input_copy_tests.rs"]
mod input_copy_tests;

#[path = "app_tests/planning_runtime_tests.rs"]
mod planning_runtime_tests;

#[path = "app_tests/parallel_mode_runtime_tests.rs"]
mod parallel_mode_runtime_tests;

#[path = "app_tests/shell_surface_tests.rs"]
mod shell_surface_tests;

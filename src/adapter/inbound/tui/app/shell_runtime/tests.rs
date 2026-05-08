use super::*;
use crate::adapter::inbound::tui::app::conversation_model::AutoFollowSkipReason;
use crate::adapter::inbound::tui::app::conversation_runtime::{
    ConversationPostTurnAction, ConversationPostTurnEvaluation,
};
use crate::adapter::inbound::tui::app::{
    ConversationInputState, ConversationState, InlineShellCommand, PlanningWorkerPanelState,
    PlanningWorkerStatus, test_helpers,
};
use crate::adapter::inbound::tui::shell_chrome::{ShellChromeEvent, ShellOverlay, StartupState};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
use crate::application::port::outbound::parallel_agent_worker_port::{
    ParallelAgentWorkerPort, ParallelAgentWorkerStreamRequest,
};
use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
use crate::application::port::outbound::startup_probe_port::{
    AppServerStartupContext, StartupProbePort,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningServices, PlanningTaskIntakeRequest,
};
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::{
    ConversationMessage, ConversationMessageKind, ConversationSnapshot,
};
use crate::domain::github_review::{GithubPullRequestActivitySnapshot, GithubPullRequestTarget};
use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogRequest};
use crate::domain::startup_diagnostics::StartupDiagnostics;
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
use anyhow::Result;
use crossterm::event::KeyEventState;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
#[path = "tests/flows.rs"]
mod flows;
#[path = "tests/input.rs"]
mod input;
#[path = "tests/scheduler.rs"]
mod scheduler;

// Shell runtime tests exercise the adapter boundary where terminal events,
// background workers, and TUI state meet. The fakes below keep outbound ports
// deterministic while still driving the same message paths as the real app.
#[derive(Default)]
struct FakeAppServerPort;
impl StartupProbePort for FakeAppServerPort {
    fn load_startup_context(&self) -> Result<AppServerStartupContext> {
        Ok(AppServerStartupContext {
            attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
            initialize_detail: "ok".to_string(),
            account_detail: "ok".to_string(),
            account_ok: true,
            warnings: Vec::new(),
        })
    }
}
impl SessionCatalogPort for FakeAppServerPort {
    fn load_session_catalog(&self, _request: SessionCatalogRequest) -> Result<SessionCatalog> {
        Ok(RecentSessions {
            items: Vec::new(),
            warnings: Vec::new(),
            next_cursor: None,
        }
        .into())
    }
}
impl InteractiveTurnRuntimePort for FakeAppServerPort {
    fn runtime_control_truth(
        &self,
    ) -> crate::domain::conversation::ConversationRuntimeControlTruth {
        crate::domain::conversation::ConversationRuntimeControlTruth::codex_app_server()
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
    fn request_stop_all_sessions(&self) -> Result<()> {
        Ok(())
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
struct FakeGithubReviewPollerPort;
impl GithubReviewPollerPort for FakeGithubReviewPollerPort {
    fn load_pull_request_activity(
        &self,
        target: &GithubPullRequestTarget,
    ) -> Result<GithubPullRequestActivitySnapshot> {
        Ok(GithubPullRequestActivitySnapshot {
            target: target.clone(),
            title: "Review status".to_string(),
            url: "https://github.com/acme/widgets/pull/42".to_string(),
            head_branch: "feature/native-github-poll-scheduling".to_string(),
            base_branch: "prerelease".to_string(),
            events: Vec::new(),
        })
    }
}
#[derive(Default)]
struct FakeSessionCatalogPort {
    requests: Mutex<Vec<SessionCatalogRequest>>,
}
impl SessionCatalogPort for FakeSessionCatalogPort {
    fn load_session_catalog(&self, request: SessionCatalogRequest) -> Result<SessionCatalog> {
        self.requests
            .lock()
            .expect("session catalog request mutex poisoned")
            .push(request);
        Ok(RecentSessions {
            items: Vec::new(),
            warnings: Vec::new(),
            next_cursor: None,
        }
        .into())
    }
}
#[derive(Debug)]
struct CountingParallelAgentWorkerPort {
    launch_count: Arc<AtomicUsize>,
}

impl ParallelAgentWorkerPort for CountingParallelAgentWorkerPort {
    fn run_isolated_new_thread_stream(
        &self,
        _request: ParallelAgentWorkerStreamRequest<'_>,
        _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.launch_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct ShellRuntimeParallelFixture {
    runtime: ShellRuntime,
    launch_count: Arc<AtomicUsize>,
}

// Runtime fixtures use real services around fake ports so tests cover
// ShellRuntime orchestration instead of isolated state mutations.
fn make_test_runtime() -> ShellRuntime {
    let codex_port = Arc::new(FakeAppServerPort);
    let app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            Arc::new(
                crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort,
            ),
            crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
            test_helpers::test_planning_services(Arc::new(
                FilesystemPlanningWorkspaceAdapter::new(),
            )),
        );
    ShellRuntime::new(app)
}
fn make_test_runtime_with_session_port(session_port: Arc<dyn SessionCatalogPort>) -> ShellRuntime {
    let codex_port = Arc::new(FakeAppServerPort);
    let app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(session_port),
            ConversationService::new(codex_port),
            Arc::new(
                crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort,
            ),
            crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
            test_helpers::test_planning_services(Arc::new(
                FilesystemPlanningWorkspaceAdapter::new(),
            )),
        );
    ShellRuntime::new(app)
}
fn make_dispatch_ready_parallel_runtime(prefix: &str) -> ShellRuntimeParallelFixture {
    let workspace_dir = create_temp_git_repo(prefix);
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = PlanningServices::from_ports(
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        authority.clone(),
        authority,
        Arc::new(NoopPlanningWorkerPort),
    );
    bootstrap_active_planning_workspace_with_services(&planning, &workspace_dir);
    let proposal = planning
        .runtime
        .prepare_task_intake(PlanningTaskIntakeRequest {
            workspace_directory: workspace_dir.clone(),
            raw_prompt: "verify parallel entry does not auto dispatch".to_string(),
            legacy_source_turn_id: None,
            provenance: Default::default(),
            requested_direction_id: None,
            observed_planning_revision: None,
        })
        .expect("task intake proposal should prepare");
    planning
        .runtime
        .commit_task_intake(&proposal)
        .expect("task intake proposal should commit");

    let launch_count = Arc::new(AtomicUsize::new(0));
    let worker_port = Arc::new(CountingParallelAgentWorkerPort {
        launch_count: launch_count.clone(),
    });
    let codex_port = Arc::new(FakeAppServerPort);
    let mut app = NativeTuiApp::new(
        StartupService::new(codex_port.clone()),
        SessionService::new(codex_port.clone()),
        ConversationService::new(codex_port),
        worker_port,
        crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
        planning,
    );
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir));
    app.sync_draft_shell_workspace(&workspace_dir);
    app.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(&workspace_dir);

    ShellRuntimeParallelFixture {
        runtime: ShellRuntime::new(app),
        launch_count,
    }
}
fn sample_startup_diagnostics(workspace_path: &str) -> StartupDiagnostics {
    StartupDiagnostics {
        cwd: workspace_path.to_string(),
        codex_binary_ok: true,
        codex_binary_detail: "ok".to_string(),
        workspace_ok: true,
        workspace_path: workspace_path.to_string(),
        workspace_detail: "ok".to_string(),
        attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
        initialize_ok: true,
        initialize_detail: "ok".to_string(),
        account_ok: true,
        account_detail: "ok".to_string(),
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
    fs::create_dir_all(&path).expect("temp workspace should be created");
    path.display().to_string()
}
fn create_temp_git_repo(prefix: &str) -> String {
    let root = PathBuf::from(create_temp_workspace(prefix)).join("repo");
    fs::create_dir_all(&root).expect("temp git repo should be created");

    run_git(&root, &["init", "-q"]);
    run_git(&root, &["config", "user.name", "RefinedStone"]);
    run_git(&root, &["config", "user.email", "chem.en.9273@gmail.com"]);
    fs::write(root.join("README.md"), "seed\n").expect("seed file should write");
    fs::write(root.join(".gitignore"), "*.tmp\n").expect("gitignore should write");
    run_git(&root, &["add", "README.md", ".gitignore"]);
    run_git(&root, &["commit", "-qm", "init"]);
    run_git(&root, &["branch", "akra"]);
    run_git(&root, &["branch", "prerelease"]);
    run_git(
        &root,
        &["update-ref", "refs/remotes/origin/prerelease", "prerelease"],
    );

    fs::canonicalize(&root)
        .expect("temp git repo should canonicalize")
        .display()
        .to_string()
}

fn run_git(repo_root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git command should spawn");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

// Planning-aware session tests need an actual workspace on disk because the
// runtime status row is built from the same filesystem-backed authority that
// operators use in normal TUI sessions.
fn bootstrap_active_planning_workspace(workspace_dir: &str) {
    let planning =
        test_helpers::test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    bootstrap_active_planning_workspace_with_services(&planning, workspace_dir);
}

fn bootstrap_active_planning_workspace_with_services(
    planning: &PlanningServices,
    workspace_dir: &str,
) {
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
#[test]
fn ctrl_q_requests_quit() {
    let mut runtime = make_test_runtime();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('q'),
        KeyModifiers::CONTROL,
    )));

    assert!(runtime.should_quit());
}
#[test]
fn non_press_key_events_are_ignored() {
    let mut runtime = make_test_runtime();

    runtime.handle_terminal_event(Event::Key(KeyEvent {
        code: KeyCode::Char('q'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Release,
        state: KeyEventState::NONE,
    }));

    assert!(!runtime.should_quit());
}
// Background loads must surface planning authority and queue context when a
// resumed conversation points at a workspace that already has planning state.
#[test]
fn resumed_session_status_surfaces_planning_and_queue_context() {
    let mut runtime = make_test_runtime();
    let workspace_dir = create_temp_workspace("resume-planning-context");
    bootstrap_active_planning_workspace(&workspace_dir);
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics(&workspace_dir));
    runtime.take_redraw_request();

    runtime
        .app
        .tx
        .send(BackgroundMessage::ConversationLoaded(Ok(
            ConversationSnapshot {
                thread_id: "thread-1".to_string(),
                title: "Loaded thread".to_string(),
                cwd: workspace_dir.clone(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
            },
        )))
        .expect("background message should enqueue");

    runtime.poll_background_messages();
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(
        conversation
            .status_text
            .contains("thread loaded / planning status: ready")
    );
    assert!(
        conversation
            .status_text
            .contains("queue summary: now: none  |  next: none")
    );
    assert!(
        conversation
            .status_text
            .contains("proposed: none  |  blocked: none")
    );
    fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}
#[test]
fn startup_background_message_updates_app_state() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    runtime
        .app
        .tx
        .send(BackgroundMessage::StartupLoaded(Ok(
            sample_startup_diagnostics("/tmp/root"),
        )))
        .expect("startup message should send");

    runtime.poll_background_messages();
    match &runtime.app.startup_state {
        StartupState::Ready(diagnostics) => {
            assert_eq!(diagnostics.workspace_path, "/tmp/root");
        }
        other => panic!("expected ready startup state, got {other:?}"),
    }
}
#[test]
fn conversation_stream_background_message_is_routed_through_runtime_reducer() {
    /*
     * ConversationStream is a provider/runtime fact. ShellRuntime must route it
     * through the conversation runtime reducer instead of mutating conversation
     * projection fields directly in the background-message match arm.
     */
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();

    runtime
        .app
        .tx
        .send(BackgroundMessage::ConversationStream(
            ConversationStreamEvent::StatusUpdated {
                text: "provider is thinking".to_string(),
            },
        ))
        .expect("conversation stream message should enqueue");

    runtime.poll_background_messages();
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.status_text, "provider is thinking");
}
#[test]
fn session_catalog_request_uses_current_workspace_context() {
    let session_port = Arc::new(FakeSessionCatalogPort::default());
    let mut runtime = make_test_runtime_with_session_port(session_port.clone());
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/session-root"));

    runtime
        .app_mut()
        .dispatch_shell_chrome(ShellChromeEvent::SessionsRequested { limit: 7 });
    for _ in 0..20 {
        if !session_port
            .requests
            .lock()
            .expect("session catalog request mutex poisoned")
            .is_empty()
        {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        *session_port
            .requests
            .lock()
            .expect("session catalog request mutex poisoned"),
        vec![SessionCatalogRequest::for_workspace(7, "/tmp/session-root")]
    );
}
#[test]
fn idle_background_poll_does_not_request_redraw() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();

    runtime.poll_background_messages();

    assert!(!runtime.take_redraw_request());
}
// Live turns throttle redraws through the scheduler. This catches regressions
// where streaming activity either redraws too aggressively or never schedules
// the delayed pulse that keeps elapsed-time UI fresh.
#[test]
fn live_activity_schedules_delayed_draw_without_immediate_redraw() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    let now = Instant::now();
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());
    conversation.active_turn_started_at = Some(now - Duration::from_secs(5));
    runtime.last_live_activity_pulse = Some(5);

    runtime.poll_background_messages_at(now);

    assert!(!runtime.take_due_draw_request(now));
    assert_eq!(
        runtime.next_event_poll_timeout(now, Duration::from_secs(1)),
        Duration::from_millis(250)
    );
    assert!(runtime.take_due_draw_request(now + Duration::from_millis(250)));
}
// Post-turn evaluation messages are keyed by the completed turn. Stale or
// duplicate worker results must not overwrite the current transcript status,
// runtime notices, or planning worker panel.
#[test]
fn stale_post_turn_evaluation_background_message_is_ignored() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.thread_id = "thread-1".to_string();
    conversation.status_text = "session ready".to_string();
    conversation.turn_activity.last_completed_turn_id = Some("turn-2".to_string());

    runtime
            .app
            .tx
            .send(BackgroundMessage::PostTurnEvaluated {
                thread_id: "thread-1".to_string(),
                completed_turn_id: "turn-1".to_string(),
                evaluation: Box::new(ConversationPostTurnEvaluation {
                    runtime_snapshot: crate::application::service::planning::PlanningRuntimeSnapshot::invalid(
                        "stale snapshot".to_string(),
                    ),
                    planning_repair_state: None,
                    runtime_notices: vec!["stale notice".to_string()],
                    action: crate::adapter::inbound::tui::app::conversation_runtime::ConversationPostTurnAction::SkipAutoFollow {
                        reason: crate::adapter::inbound::tui::app::conversation_model::AutoFollowSkipReason::PostTurnContinuationPaused,
                    },
                    parallel_queue_signal: None,
                    operator_alerts: Vec::new(),
                }),
                planning_worker_panel_state: crate::adapter::inbound::tui::app::PlanningWorkerPanelState {
                    status: crate::adapter::inbound::tui::app::PlanningWorkerStatus::RefreshSucceeded,
                    last_operation_label: None,
                    last_queue_summary: Some("queue head: stale".to_string()),
                    last_summary: Some("stale".to_string()),
                    last_rejected_summary: None,
                    last_notice_detail: None,
                    last_prompt: None,
                    last_response: None,
                    last_host_detail: None,
                },
            })
            .expect("background message should enqueue");

    runtime.poll_background_messages();
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.status_text, "session ready");
    assert!(conversation.runtime_notices.is_empty());
    assert!(
        runtime
            .app()
            .planning_worker_panel_state
            .last_summary
            .is_none()
    );
}
#[test]
fn duplicate_post_turn_evaluation_for_same_turn_is_ignored() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.thread_id = "thread-1".to_string();
    conversation.turn_activity.last_completed_turn_id = Some("turn-1".to_string());
    let build_message = |notice: &str| BackgroundMessage::PostTurnEvaluated {
        thread_id: "thread-1".to_string(),
        completed_turn_id: "turn-1".to_string(),
        evaluation: Box::new(ConversationPostTurnEvaluation {
            runtime_snapshot: PlanningRuntimeSnapshot::invalid(notice.to_string()),
            planning_repair_state: None,
            runtime_notices: vec![notice.to_string()],
            action: ConversationPostTurnAction::SkipAutoFollow {
                reason: AutoFollowSkipReason::PlanningBlocked,
            },
            parallel_queue_signal: None,
            operator_alerts: Vec::new(),
        }),
        planning_worker_panel_state: PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshFailed,
            last_operation_label: None,
            last_queue_summary: None,
            last_summary: Some(notice.to_string()),
            last_rejected_summary: None,
            last_notice_detail: None,
            last_prompt: None,
            last_response: None,
            last_host_detail: None,
        },
    };

    runtime
        .app
        .tx
        .send(build_message("first evaluation"))
        .expect("background message should enqueue");
    runtime
        .app
        .tx
        .send(build_message("late duplicate"))
        .expect("background message should enqueue");

    runtime.poll_background_messages();
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(
        conversation
            .runtime_notices
            .contains(&"first evaluation".to_string())
    );
    assert!(
        !conversation
            .runtime_notices
            .contains(&"late duplicate".to_string())
    );
    assert_eq!(
        runtime
            .app()
            .planning_worker_panel_state
            .last_summary
            .as_deref(),
        Some("first evaluation")
    );
}
// Resize is a rendering concern only: it should request a redraw without
// mutating committed transcript rows or the active input buffer.
#[test]
fn resize_event_requests_redraw() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Resize(120, 40));

    assert!(runtime.take_redraw_request());
}
#[test]
fn resize_event_leaves_transcript_state_unchanged() {
    let mut runtime = make_test_runtime();
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "completed history stays committed".to_string(),
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.refresh_conversation_lines();
    conversation.input_buffer = "buffered prompt".to_string();
    let expected_lines = conversation.cached_conversation_lines.clone();

    runtime.take_redraw_request();
    runtime.handle_terminal_event(Event::Resize(120, 40));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.cached_conversation_lines, expected_lines);
    assert_eq!(conversation.input_buffer, "buffered prompt");
    assert_eq!(conversation.messages.len(), 1);
    assert!(runtime.take_redraw_request());
}
#[test]
fn manual_turn_elapsed_pulse_requests_redraw() {
    let mut runtime = make_test_runtime();
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());
    conversation.active_turn_started_at = Some(Instant::now() - Duration::from_secs(5));
    runtime.last_live_activity_pulse = Some(4);
    runtime.take_redraw_request();

    runtime.poll_background_messages();

    assert!(runtime.take_redraw_request());
}
// GitHub review polling starts from the runtime poll loop so it can share the
// same background-message cadence as app-server and session-catalog work.
#[test]
fn poll_background_messages_starts_github_review_polling_when_due() {
    let mut runtime = make_test_runtime();
    runtime.app_mut().configure_github_review_polling(
        super::super::github_polling::GithubReviewPollingBootstrap {
            service: Some(GithubReviewPollerService::new(Arc::new(
                FakeGithubReviewPollerPort,
            ))),
            state: super::super::github_polling::GithubReviewPollingState::active(
                super::super::github_polling::GithubReviewPollingConfig {
                    target: GithubPullRequestTarget::new("acme/widgets", 42),
                    interval: Duration::from_secs(30),
                },
                Instant::now(),
            ),
        },
    );

    runtime.poll_background_messages();
    thread::sleep(Duration::from_millis(20));
    runtime.poll_background_messages();
    let super::super::github_polling::GithubReviewPollingState::Active(polling_state) =
        &runtime.app().github_review_polling_state
    else {
        panic!("expected active github review polling state");
    };
    assert!(polling_state.snapshot.is_some());
    assert!(polling_state.last_error.is_none());
}

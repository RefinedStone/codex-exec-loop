use super::*;
use crate::adapter::inbound::tui::app::conversation_model::AutoFollowSkipReason;
use crate::adapter::inbound::tui::app::conversation_runtime::{
    ConversationRuntimeEffect, PostTurnContinuationAction, PostTurnEvaluationOutcome,
    PostTurnEvaluationProvenance,
};
use crate::adapter::inbound::tui::app::{
    ConversationInputState, ConversationState, InlineShellCommand, NativeTuiParallelModeBinding,
    PlanningWorkerPanelState, PlanningWorkerStatus, test_helpers,
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
    PlanningRuntimeProjection, PlanningServices, PlanningTaskIntakeRequest,
    PlanningTurnExecutionSnapshotCapture,
};
use crate::application::service::post_turn_evaluation as application_post_turn;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::core::app::{
    AppEvent, CoreInput, QueueAuthorityLoadCorrelation, QueueAuthorityLoadError,
    StartupReadySnapshot, TurnStreamEvent,
};
use crate::domain::conversation::{
    ConversationMessage, ConversationMessageKind, ConversationSnapshot,
};
use crate::domain::github_review::{GithubPullRequestActivitySnapshot, GithubPullRequestTarget};
use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogRequest};
use crate::domain::startup_diagnostics::StartupDiagnostics;
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
use anyhow::Result;
use crossterm::event::{KeyEventState, MouseButton, MouseEvent, MouseEventKind};
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
            item_lifecycle: Default::default(),
        })
    }
    fn request_stop_all_sessions(&self) -> Result<()> {
        Ok(())
    }
    fn run_new_thread_stream(
        &self,
        cwd: &str,
        _prompt: &str,
        _options: crate::domain::conversation::ConversationTurnOptions,
        event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
    ) -> Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
        crate::application::service::conversation_runtime_event::emit_confirmed_test_terminal_receipt(
            &event_sender,
            "test-thread",
            cwd,
        )
    }
    fn run_turn_stream(
        &self,
        thread_id: &str,
        _prompt: &str,
        _options: crate::domain::conversation::ConversationTurnOptions,
        event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
    ) -> Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
        crate::application::service::conversation_runtime_event::emit_confirmed_test_terminal_receipt(
            &event_sender,
            thread_id,
            "/tmp/test-workspace",
        )
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
        request: ParallelAgentWorkerStreamRequest<'_>,
        event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
    ) -> Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
        self.launch_count.fetch_add(1, Ordering::SeqCst);
        crate::application::service::conversation_runtime_event::emit_confirmed_test_terminal_receipt(
            &event_sender,
            "test-worker-thread",
            request.cwd,
        )
    }
}

struct ShellRuntimeParallelFixture {
    runtime: ShellRuntime,
    planning: PlanningServices,
    launch_count: Arc<AtomicUsize>,
}

// Runtime fixtures use real services around fake ports so tests cover
// ShellRuntime orchestration instead of isolated state mutations.
fn make_test_runtime() -> ShellRuntime {
    let codex_port = Arc::new(FakeAppServerPort);
    let planning =
        test_helpers::test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    let parallel_mode_control_plane_composition =
        test_helpers::test_parallel_mode_control_plane_composition(planning);
    let parallel_mode_binding =
        NativeTuiParallelModeBinding::from_composition(parallel_mode_control_plane_composition);
    let app = NativeTuiApp::new(
        StartupService::new(codex_port.clone()),
        SessionService::new(codex_port.clone()),
        ConversationService::new(codex_port),
        parallel_mode_binding,
    );
    ShellRuntime::new(app)
}

fn make_test_runtime_with_github_review_polling(
    github_review_polling: super::super::github_polling::GithubReviewPollingBootstrap,
    loader: impl Fn(
        &crate::core::app::GithubReviewPollingSetupRequest,
    ) -> anyhow::Result<Option<(GithubPullRequestTarget, GithubReviewPollerService)>>
    + Send
    + Sync
    + 'static,
) -> ShellRuntime {
    let codex_port = Arc::new(FakeAppServerPort);
    let planning =
        test_helpers::test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    let parallel_mode_control_plane_composition =
        test_helpers::test_parallel_mode_control_plane_composition(planning);
    let parallel_mode_binding =
        NativeTuiParallelModeBinding::from_composition(parallel_mode_control_plane_composition);
    let app = NativeTuiApp::new_with_github_review_polling_setup_loader(
        StartupService::new(codex_port.clone()),
        SessionService::new(codex_port.clone()),
        ConversationService::new(codex_port),
        parallel_mode_binding,
        github_review_polling,
        loader,
    );
    ShellRuntime::new(app)
}

fn make_test_runtime_with_session_port(session_port: Arc<dyn SessionCatalogPort>) -> ShellRuntime {
    let codex_port = Arc::new(FakeAppServerPort);
    let planning =
        test_helpers::test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    let parallel_mode_control_plane_composition =
        test_helpers::test_parallel_mode_control_plane_composition(planning);
    let parallel_mode_binding =
        NativeTuiParallelModeBinding::from_composition(parallel_mode_control_plane_composition);
    let app = NativeTuiApp::new(
        StartupService::new(codex_port.clone()),
        SessionService::new(session_port),
        ConversationService::new(codex_port),
        parallel_mode_binding,
    );
    ShellRuntime::new(app)
}

#[test]
fn native_tui_app_keeps_parallel_control_plane_behind_narrow_control_plane_handle() {
    /*
     * This guards the architecture boundary from regressing back to a TUI-owned
     * controller. The app stores only the typed control-plane handle; app_runtime
     * performs the TUI event-sink binding from the shared composition.
     */
    const APP_RS: &str = include_str!("../../app.rs");
    const APP_RUNTIME_RS: &str = include_str!("../app_runtime.rs");

    assert!(
        APP_RS.contains("ParallelModeControlPlaneHandle<TuiParallelModeControlPlaneEventSink>")
    );
    assert!(
        !APP_RS.contains("ParallelModeControlPlaneService<TuiParallelModeControlPlaneEventSink>")
    );
    assert!(APP_RUNTIME_RS.contains("NativeTuiParallelModeBinding"));
    assert!(APP_RUNTIME_RS.contains("from_composition"));
}

#[test]
fn queue_mutation_settlement_stays_correlated_and_off_the_input_path() {
    /*
     * Queue remove/undo input only submits a core intent. Core owns the
     * generation, worker effect, and exact completion gate; the TUI projects
     * started/completed events. Queue chrome is disposable; pending projection is not.
     */
    const APP_RS: &str = include_str!("../../app.rs");
    const APP_RUNTIME_RS: &str = include_str!("../app_runtime.rs");
    const SHELL_CONTROLLER_RS: &str = include_str!("../shell_controller.rs");
    const QUEUE_CONTROLLER_RS: &str = include_str!("../queue_overlay_controller.rs");
    const QUEUE_UI_RS: &str = include_str!("../queue_overlay_ui.rs");
    const SHELL_RUNTIME_RS: &str = include_str!("../shell_runtime.rs");

    let queue_controller_input_path = QUEUE_CONTROLLER_RS
        .split("#[cfg(test)]\nmod tests")
        .next()
        .expect("queue controller production source must exist");
    let queue_ui_input_path = QUEUE_UI_RS
        .split("#[cfg(test)]\nmod tests")
        .next()
        .expect("queue UI production source must exist");
    for input_path in [
        queue_controller_input_path,
        queue_ui_input_path,
        SHELL_RUNTIME_RS,
    ] {
        assert!(!input_path.contains(".cancel_tasks("));
    }
    assert!(!QUEUE_CONTROLLER_RS.contains("std::thread::spawn"));
    assert!(!QUEUE_CONTROLLER_RS.contains("BackgroundMessage::QueueMutationCompleted"));
    assert!(!SHELL_RUNTIME_RS.contains("BackgroundMessage::QueueMutationCompleted"));
    assert!(
        QUEUE_CONTROLLER_RS
            .contains(".dispatch_command(AppCommand::SubmitQueueMutation(Box::new(intent)))")
    );
    assert!(APP_RUNTIME_RS.contains("AppEvent::QueueMutationStarted { correlation }"));
    assert!(APP_RUNTIME_RS.contains("self.apply_queue_mutation_started(correlation)"));
    assert!(APP_RUNTIME_RS.contains("AppEvent::QueueMutationCompleted"));
    assert!(APP_RUNTIME_RS.contains("self.apply_queue_mutation_completion(correlation, *result)"));
    assert!(!APP_RUNTIME_RS.contains("QueueOverlayAuthorityLoaded"));
    assert!(!QUEUE_CONTROLLER_RS.contains("QueueOverlayAuthorityLoaded"));
    assert!(!SHELL_RUNTIME_RS.contains("QueueOverlayAuthorityLoaded"));
    assert!(QUEUE_CONTROLLER_RS.contains("AppCommand::LoadQueueAuthority"));
    assert!(QUEUE_CONTROLLER_RS.contains("AppEvent::QueueAuthorityLoadStarted"));
    assert!(APP_RUNTIME_RS.contains("AppEvent::QueueAuthorityLoaded"));
    assert!(APP_RUNTIME_RS.contains("apply_queue_overlay_authority_loaded(correlation, result)"));

    assert!(!QUEUE_UI_RS.contains("next_operation_id"));
    assert!(!QUEUE_UI_RS.contains("QueueMutationWorkerResult"));
    assert!(!QUEUE_UI_RS.contains("QueueMutationOperation"));
    assert!(QUEUE_UI_RS.contains("pending: Option<QueueMutationCorrelation>"));
    assert!(QUEUE_UI_RS.contains("workspace_directory: String"));
    assert!(QUEUE_UI_RS.contains("active_thread_id: Option<String>"));
    assert!(QUEUE_UI_RS.contains("if self.pending.as_ref() != Some(completed)"));
    assert!(QUEUE_CONTROLLER_RS.contains("expected_planning_revision: planning_revision"));
    assert!(QUEUE_CONTROLLER_RS.contains("expected_status: status"));
    assert!(QUEUE_CONTROLLER_RS.contains("expected_updated_at: updated_at"));
    assert!(QUEUE_CONTROLLER_RS.contains("expected_planning_revision: receipt.planning_revision"));
    assert!(QUEUE_CONTROLLER_RS.contains("expected_status: entry.after_status"));
    assert!(QUEUE_CONTROLLER_RS.contains("expected_updated_at: entry.after_updated_at.clone()"));
    assert!(
        QUEUE_CONTROLLER_RS.contains("let current_context = self.current_queue_mutation_context()")
    );
    assert!(QUEUE_CONTROLLER_RS.contains("current_context != operation_context"));
    let show_queue = QUEUE_CONTROLLER_RS
        .split("pub(super) fn show_queue_overlay")
        .nth(1)
        .expect("queue overlay opener must exist")
        .split("pub(super) fn start_queue_overlay_authority_load")
        .next()
        .expect("queue overlay opener must stay bounded");
    assert!(show_queue.contains("self.dispatch_shell_chrome"));
    assert!(show_queue.contains("self.start_queue_overlay_authority_load()"));
    assert!(!show_queue.contains(".application"));
    assert!(!show_queue.contains(".load_"));

    assert!(!APP_RUNTIME_RS.contains("execute_queue_mutation"));
    assert!(!APP_RUNTIME_RS.contains("load_queue_authority"));
    assert!(!QUEUE_CONTROLLER_RS.contains(".execute_cancellation_transaction("));
    assert!(!QUEUE_CONTROLLER_RS.contains(".load_coherent_authority("));

    assert!(APP_RS.contains("queue_overlay_ui_state: queue_overlay_ui::QueueOverlayUiState"));
    assert!(APP_RS.contains("queue_mutation_ui_state: queue_overlay_ui::QueueMutationUiState"));
    assert!(APP_RUNTIME_RS.contains(
        "if previous_overlay == ShellOverlay::Queue && self.shell_overlay != ShellOverlay::Queue"
    ));
    assert!(APP_RUNTIME_RS.contains("self.queue_overlay_ui_state.reset()"));
    assert!(!SHELL_CONTROLLER_RS.contains("refresh_queue_overlay_authority_binding"));
}

#[test]
fn parallel_post_turn_continuation_is_driven_by_control_plane_outcome() {
    /*
     * The parallel-mode TUI entrypoint must not inspect or consume
     * QueueAutoPrompt directly. The adapter asks application services for a
     * continuation outcome, then maps that outcome onto local effects and
     * presentation events.
     */
    const PARALLEL_MODE_RS: &str = include_str!("../parallel_mode.rs");
    const POST_TURN_ROUTING_RS: &str = include_str!("../post_turn_continuation.rs");
    const CONTROL_PLANE_HOST_RS: &str =
        include_str!("../../../../../application/service/parallel_mode/control_plane/host.rs");

    assert!(PARALLEL_MODE_RS.contains("continue_post_turn_queue("));
    assert!(!PARALLEL_MODE_RS.contains("QueueAutoPrompt"));
    assert!(!PARALLEL_MODE_RS.contains("record_auto_follow_parallel_dispatch"));
    assert!(!PARALLEL_MODE_RS.contains("handle_post_turn_queue_continuation"));
    assert!(POST_TURN_ROUTING_RS.contains("decide_post_turn_auto_prompt_route"));
    assert!(!POST_TURN_ROUTING_RS.contains("QueueAutoPrompt"));
    assert!(!POST_TURN_ROUTING_RS.contains(".retain("));
    assert!(CONTROL_PLANE_HOST_RS.contains("pub fn continue_post_turn_queue"));
    assert!(!CONTROL_PLANE_HOST_RS.contains("pub fn handle_post_turn_queue_continuation"));
}

#[test]
fn parallel_control_plane_presentation_bridge_maps_events_outside_tui_controller() {
    /*
     * The TUI controller may apply presentation actions, but control-plane event
     * interpretation should stay in the small bridge mapper so the main
     * parallel controller does not grow another orchestration switchboard.
     */
    const PARALLEL_MODE_RS: &str = include_str!("../parallel_mode.rs");
    const PRESENTATION_BRIDGE_RS: &str = include_str!("../parallel_mode/presentation_bridge.rs");
    let production_controller = PARALLEL_MODE_RS
        .split_once("#[cfg(test)]\nmod global_runtime_notice_tests")
        .map_or(PARALLEL_MODE_RS, |(production, _)| production);

    assert!(production_controller.contains("parallel_mode_presentation_actions"));
    assert!(!production_controller.contains("ParallelModeControlPlanePresentationEvent::"));
    assert!(PRESENTATION_BRIDGE_RS.contains("ParallelModePresentationAction"));
    assert!(PRESENTATION_BRIDGE_RS.contains("ParallelModeControlPlanePresentationEvent::"));
}

#[test]
fn post_turn_completion_payload_is_not_stashed_in_tui_pending_queue() {
    /*
     * Post-turn completion must re-enter core before the TUI applies the
     * payload, but the payload should not sit in a second TUI-owned pending
     * queue keyed by thread/turn. Stale and duplicate guards live at the
     * core completion boundary before the TUI sees accepted results.
     */
    const APP_RS: &str = include_str!("../../app.rs");
    const APP_RUNTIME_RS: &str = include_str!("../app_runtime.rs");
    const CONVERSATION_VIEW_MODEL_RS: &str = include_str!("../conversation_model/view_model.rs");
    const CONVERSATION_RUNTIME_RS: &str = include_str!("../conversation_runtime.rs");
    const POST_TURN_ROUTING_RS: &str = include_str!("../post_turn_continuation.rs");
    const SHELL_RUNTIME_RS: &str = include_str!("../shell_runtime.rs");
    const CORE_CONTROLLER_RS: &str = include_str!("../../../../../core/app/controller.rs");

    assert!(!APP_RS.contains("pending_post_turn_continuation_results"));
    assert!(!POST_TURN_ROUTING_RS.contains("route_pending_post_turn_continuation_result"));
    assert!(!POST_TURN_ROUTING_RS.contains("enqueue_post_turn_continuation_result"));
    assert!(!CONVERSATION_VIEW_MODEL_RS.contains("last_applied_post_turn_evaluation_id"));
    assert!(!CONVERSATION_VIEW_MODEL_RS.contains("accepts_post_turn_evaluation"));
    assert!(!CONVERSATION_VIEW_MODEL_RS.contains("record_post_turn_evaluation_applied"));
    for source in [
        APP_RUNTIME_RS,
        CONVERSATION_RUNTIME_RS,
        POST_TURN_ROUTING_RS,
        SHELL_RUNTIME_RS,
    ] {
        assert!(!source.contains("PostTurnEvaluated"));
        assert!(!source.contains("PostTurnAutomationBackgroundResult"));
        assert!(!source.contains("ConversationPostTurnEvaluation"));
        assert!(!source.contains("ConversationPostTurnAction"));
        assert!(!source.contains("PostTurnAutomationProvenance"));
        assert!(!source.contains("QueuedAutoPrompt"));
        assert!(!source.contains("EvaluatePostTurnAutomation"));
    }
    assert!(SHELL_RUNTIME_RS.contains("PostTurnEvaluationCompleted"));
    assert!(SHELL_RUNTIME_RS.contains("CoreEffectCompletion::PostTurnEvaluationCompleted"));
    assert!(
        CORE_CONTROLLER_RS.contains("accept_post_turn_evaluation_completion(execution.as_ref())")
    );
}

#[test]
fn conversation_lifecycle_body_state_is_driven_by_core_snapshot() {
    /*
     * Session selection may keep presentation chrome such as the highlighted
     * session row in TUI, but Loading/Ready/Failed conversation body state must
     * come back through correlated core ConversationChanged snapshots. The
     * test-only background shim uses the same TUI correlation gate.
     */
    const APP_RUNTIME_RS: &str = include_str!("../app_runtime.rs");
    const CONVERSATION_LIFECYCLE_RS: &str = include_str!("../conversation_lifecycle.rs");
    const SHELL_RUNTIME_RS: &str = include_str!("../shell_runtime.rs");

    assert!(APP_RUNTIME_RS.contains("AppEvent::ConversationChanged {"));
    assert!(APP_RUNTIME_RS.contains("apply_correlated_conversation_snapshot"));
    assert!(APP_RUNTIME_RS.contains("apply_core_conversation_snapshot(snapshot)"));
    assert!(CONVERSATION_LIFECYCLE_RS.contains("CoreConversationSnapshotApplied"));
    assert!(SHELL_RUNTIME_RS.contains("ConversationLoadCorrelation::new"));
    assert!(!APP_RUNTIME_RS.contains("apply_loaded_conversation_result"));
    assert!(!SHELL_RUNTIME_RS.contains("apply_loaded_conversation_result"));
    assert!(!CONVERSATION_LIFECYCLE_RS.contains("ConversationLifecycleEvent::ConversationLoaded"));
}

#[test]
fn tui_task_command_surface_is_removed() {
    /*
     * Runtime task creation stays available through planning services and admin
     * surfaces, but the terminal command switchboard should no longer expose a
     * `:task` entry point or its old pending replay path.
     */
    const APP_RS: &str = include_str!("../../app.rs");
    const INLINE_COMMANDS_RS: &str = include_str!("../inline_shell_commands.rs");
    const SHELL_CONTROLLER_RS: &str = include_str!("../shell_controller.rs");
    const TURN_SUBMISSION_RUNTIME_RS: &str = include_str!("../turn_submission_runtime.rs");

    assert!(!INLINE_COMMANDS_RS.contains("InlineShellCommand::Task"));
    assert!(!INLINE_COMMANDS_RS.contains("primary_name: \":task\""));
    assert!(!APP_RS.contains("task_intake_overlay_ui_state"));
    assert!(!APP_RS.contains("pending_task_intake_command"));
    assert!(!SHELL_CONTROLLER_RS.contains("route_planning_task_intake_command"));
    assert!(!SHELL_CONTROLLER_RS.contains("execute_pending_task_intake_command_if_ready"));
    assert!(!TURN_SUBMISSION_RUNTIME_RS.contains("PlanningTaskIntakeCommandRoute"));
}

#[test]
fn tui_projection_rendering_reads_core_snapshot_without_legacy_cache() {
    /*
     * Planning and parallel rendering must read core AppSnapshot projections
     * directly. This keeps future surfaces from recreating conversation or
     * NativeTuiApp cache fields as projection authority.
     */
    const APP_RS: &str = include_str!("../../app.rs");
    const CONVERSATION_VIEW_MODEL_RS: &str = include_str!("../conversation_model/view_model.rs");
    const PARALLEL_MODE_RS: &str = include_str!("../parallel_mode.rs");
    const PLAN_INDICATOR_RS: &str =
        include_str!("../shell_presentation/status_panels/plan_indicator.rs");
    const QUEUE_OVERLAY_RS: &str = include_str!("../shell_presentation/overlays/popup/queue.rs");
    const PLANNING_STATUS_PROJECTION_RS: &str = include_str!("../planning/status_projection.rs");
    const PLANNING_EXISTING_WORKSPACE_RS: &str =
        include_str!("../shell_presentation/overlays/popup/planning_existing_workspace.rs");
    const POST_TURN_EXECUTION_RS: &str =
        include_str!("../turn_submission_runtime/post_turn_execution.rs");

    assert!(PARALLEL_MODE_RS.contains("core_parallel_mode_readiness_snapshot"));
    assert!(PARALLEL_MODE_RS.contains("core_parallel_mode_supervisor_snapshot"));
    assert!(PARALLEL_MODE_RS.contains("self.core_runtime"));
    assert!(PARALLEL_MODE_RS.contains(".planning_parallel"));
    assert!(!APP_RS.contains("parallel_mode_readiness_snapshot:"));
    assert!(!APP_RS.contains("parallel_mode_supervisor_snapshot:"));
    assert!(!CONVERSATION_VIEW_MODEL_RS.contains("reducer_event_projection_cache"));
    assert!(!CONVERSATION_VIEW_MODEL_RS.contains("pub(crate) planning_runtime_projection"));
    assert!(
        !PARALLEL_MODE_RS.contains("self.parallel_mode_supervisor_snapshot.clone().map(Box::new)")
    );
    assert!(!PLAN_INDICATOR_RS.contains("load_planning_runtime_projection"));
    assert!(POST_TURN_EXECUTION_RS.contains("self.planning_runtime_projection_snapshot()"));
    assert!(!POST_TURN_EXECUTION_RS.contains("reducer_event_projection_cache"));
    assert!(!CONVERSATION_VIEW_MODEL_RS.contains("cached_planning_runtime_projection"));
    assert!(
        !CONVERSATION_VIEW_MODEL_RS
            .contains("planning_runtime_projection: PlanningRuntimeProjection")
    );
    for source in [
        PLAN_INDICATOR_RS,
        QUEUE_OVERLAY_RS,
        PLANNING_STATUS_PROJECTION_RS,
        PLANNING_EXISTING_WORKSPACE_RS,
    ] {
        assert!(!source.contains("conversation.planning_runtime_projection"));
    }
}

#[test]
fn conversation_projection_cache_stays_out_of_production_state() {
    fn collect_rust_sources(root: &Path, sources: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(root).expect("source directory should be readable") {
            let path = entry.expect("source entry should be readable").path();
            if path.is_dir() {
                collect_rust_sources(&path, sources);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                sources.push(path);
            }
        }
    }

    fn is_test_source(relative_path: &Path) -> bool {
        relative_path
            .components()
            .any(|component| component.as_os_str() == "tests")
            || relative_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "tests.rs" || name.ends_with("_tests.rs"))
    }

    let app_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapter/inbound/tui/app");
    let mut sources = Vec::new();
    collect_rust_sources(&app_dir, &mut sources);

    let mut cache_mentions = Vec::new();
    for path in sources {
        let relative_path = path
            .strip_prefix(&app_dir)
            .expect("source path should be below app dir");
        if is_test_source(relative_path) {
            continue;
        }
        let relative_name = relative_path.to_string_lossy().replace('\\', "/");
        let source = fs::read_to_string(&path).expect("source file should be readable");
        if source.contains("reducer_event_projection_cache")
            || source.contains("cached_planning_runtime_projection")
            || source.contains("replace_cached_planning_runtime_projection")
        {
            cache_mentions.push(relative_name);
        }
    }

    assert!(
        cache_mentions.is_empty(),
        "conversation-local planning projection caches remain in production sources: {cache_mentions:?}"
    );
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
    let fixture_planning = planning.clone();

    let launch_count = Arc::new(AtomicUsize::new(0));
    let worker_port = Arc::new(CountingParallelAgentWorkerPort {
        launch_count: launch_count.clone(),
    });
    let codex_port = Arc::new(FakeAppServerPort);
    let parallel_mode_control_plane_composition =
        test_helpers::test_parallel_mode_control_plane_composition_with_worker(
            crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
            planning,
            worker_port,
        );
    let parallel_mode_binding =
        NativeTuiParallelModeBinding::from_composition(parallel_mode_control_plane_composition);
    let mut app = NativeTuiApp::new(
        StartupService::new(codex_port.clone()),
        SessionService::new(codex_port.clone()),
        ConversationService::new(codex_port),
        parallel_mode_binding,
    );
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir));
    app.sync_draft_shell_workspace(&workspace_dir);
    app.refresh_ready_conversation_planning_runtime_projection_for_workspace(&workspace_dir);

    ShellRuntimeParallelFixture {
        runtime: ShellRuntime::new(app),
        planning: fixture_planning,
        launch_count,
    }
}
fn sample_startup_diagnostics(workspace_path: &str) -> Box<StartupReadySnapshot> {
    Box::new(StartupReadySnapshot::from_diagnostics(StartupDiagnostics {
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
    }))
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
    let origin = root
        .parent()
        .expect("temp repository should have a parent")
        .join("origin.git");
    let origin_path = origin
        .to_str()
        .expect("temp origin path should be valid utf-8");
    run_git(&root, &["init", "--bare", "-q", origin_path]);
    run_git(&root, &["remote", "add", "origin", origin_path]);
    run_git(&root, &["push", "-q", "-u", "origin", "prerelease"]);

    fs::canonicalize(&root)
        .expect("temp git repo should canonicalize")
        .display()
        .to_string()
}

fn post_turn_evaluation_completed_message(
    thread_id: impl Into<String>,
    completed_turn_id: impl Into<String>,
    runtime_projection_workspace_directory: impl Into<String>,
    runtime_projection: PlanningRuntimeProjection,
    evaluation: PostTurnEvaluationOutcome,
    planning_worker_panel_state: PlanningWorkerPanelState,
) -> BackgroundMessage {
    BackgroundMessage::PostTurnEvaluationCompleted(Box::new(
        application_post_turn::PostTurnEvaluationExecution {
            thread_id: thread_id.into(),
            completed_turn_id: completed_turn_id.into(),
            runtime_projection_workspace_directory: runtime_projection_workspace_directory.into(),
            evaluation: application_post_turn_evaluation_outcome(runtime_projection, evaluation),
            planning_worker_panel_state,
        },
    ))
}

fn mark_core_turn_completed(runtime: &mut ShellRuntime, thread_id: &str, turn_id: &str) {
    let correlation = runtime.app_mut().core_runtime.begin_test_turn_submission();
    runtime
        .app_mut()
        .core_runtime
        .dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation,
            event: TurnStreamEvent::ThreadPrepared {
                thread_id: thread_id.to_string(),
                title: "Post-turn test".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        });
    runtime
        .app_mut()
        .core_runtime
        .dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation,
            event: TurnStreamEvent::TurnStarted {
                turn_id: turn_id.to_string(),
                runtime_request: Box::default(),
            },
        });
    runtime
        .app_mut()
        .core_runtime
        .dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation,
            event: TurnStreamEvent::TurnTerminal {
                receipt: crate::domain::turn_terminal::ConversationTurnTerminalReceipt::completed(
                    thread_id,
                    turn_id,
                    Vec::new(),
                )
                .with_application_delivery(
                    crate::domain::turn_terminal::ConversationTurnApplicationDelivery::Confirmed,
                ),
                execution_snapshot_capture: Some(
                    PlanningTurnExecutionSnapshotCapture::capture_failed(
                        "/tmp/workspace",
                        "test capture skipped".to_string(),
                    ),
                ),
            },
        });
}

fn arm_core_post_turn_evaluation(runtime: &mut ShellRuntime, thread_id: &str, turn_id: &str) {
    runtime
        .app_mut()
        .core_runtime
        .begin_test_post_turn_evaluation(thread_id, turn_id);
}

fn application_post_turn_evaluation_outcome(
    runtime_projection: PlanningRuntimeProjection,
    outcome: PostTurnEvaluationOutcome,
) -> application_post_turn::PostTurnEvaluationOutcome {
    application_post_turn::PostTurnEvaluationOutcome {
        provenance: application_post_turn::PostTurnEvaluationProvenance::new(
            outcome.provenance.completed_turn_id,
        )
        .with_handoff_task(outcome.provenance.handoff_task)
        .with_parallel_queue_signal(outcome.provenance.parallel_queue_signal),
        runtime_projection,
        planning_repair_state: outcome.planning_repair_state.map(|state| {
            application_post_turn::PostTurnPlanningRepairState {
                attempts_used: state.attempts_used,
                max_attempts: state.max_attempts,
                latest_request: state.latest_request,
            }
        }),
        runtime_notices: outcome.runtime_notices,
        action: application_post_turn_action(outcome.action),
        operator_alerts: outcome.operator_alerts,
    }
}

fn application_post_turn_action(
    action: PostTurnContinuationAction,
) -> application_post_turn::PostTurnContinuationAction {
    match action {
        PostTurnContinuationAction::QueueAutoPrompt(prompt) => {
            application_post_turn::PostTurnContinuationAction::QueueAutoPrompt(Box::new(
                application_post_turn::PostTurnQueuedPrompt {
                    prompt: prompt.prompt,
                    mode_label: prompt.mode_label,
                    transcript_text: prompt.transcript_text,
                },
            ))
        }
        PostTurnContinuationAction::SkipAutoFollow { reason } => {
            application_post_turn::PostTurnContinuationAction::SkipAutoFollow {
                reason: application_post_turn_skip_reason(reason),
            }
        }
    }
}

fn application_post_turn_skip_reason(
    reason: AutoFollowSkipReason,
) -> application_post_turn::PostTurnAutoFollowSkipReason {
    match reason {
        AutoFollowSkipReason::PostTurnContinuationPaused => {
            application_post_turn::PostTurnAutoFollowSkipReason::PostTurnContinuationPaused
        }
        AutoFollowSkipReason::LimitReached => {
            application_post_turn::PostTurnAutoFollowSkipReason::LimitReached
        }
        AutoFollowSkipReason::NoAgentReply => {
            application_post_turn::PostTurnAutoFollowSkipReason::NoAgentReply
        }
        AutoFollowSkipReason::StopKeywordMatched => {
            application_post_turn::PostTurnAutoFollowSkipReason::StopKeywordMatched
        }
        AutoFollowSkipReason::NoFileChanges => {
            application_post_turn::PostTurnAutoFollowSkipReason::NoFileChanges
        }
        AutoFollowSkipReason::PlanningBlocked => {
            application_post_turn::PostTurnAutoFollowSkipReason::PlanningBlocked
        }
        AutoFollowSkipReason::PlanningQueueIdlePolicyStop => {
            application_post_turn::PostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop
        }
        AutoFollowSkipReason::PlanningQueueHeadRequired => {
            application_post_turn::PostTurnAutoFollowSkipReason::PlanningQueueHeadRequired
        }
        AutoFollowSkipReason::PlanningQueueDrained => {
            application_post_turn::PostTurnAutoFollowSkipReason::PlanningQueueDrained
        }
        AutoFollowSkipReason::PlanningRepeatedQueueHead => {
            application_post_turn::PostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead
        }
        AutoFollowSkipReason::ParallelSessionCompleted => {
            application_post_turn::PostTurnAutoFollowSkipReason::ParallelSessionCompleted
        }
        AutoFollowSkipReason::PostTurnEvaluationTimedOut => {
            application_post_turn::PostTurnAutoFollowSkipReason::PostTurnEvaluationTimedOut
        }
    }
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
fn confirmed_exit_hides_modal_then_quits_after_redraw() {
    let mut runtime = make_test_runtime();
    runtime
        .app_mut()
        .dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationShown);

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));

    assert!(!runtime.should_quit());
    assert!(!runtime.app().is_exit_confirmation_visible());
    assert!(runtime.take_redraw_request());

    runtime.finish_pending_quit_after_transaction(false);
    assert!(!runtime.should_quit());
    runtime.finish_pending_quit_after_transaction(true);
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

#[test]
fn queue_receipt_mouse_down_routes_to_the_inline_undo_action() {
    let mut runtime = make_test_runtime();
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("test runtime should start with a ready conversation");
    };
    conversation.latest_queue_mutation_receipt =
        Some(crate::domain::planning::PlanningQueueMutationReceipt {
            completed_turn_id: "turn-mouse".to_string(),
            planning_revision: 1,
            entries: vec![crate::domain::planning::PlanningQueueMutationReceiptEntry {
                task_id: "task-mouse".to_string(),
                task_title: "Mouse undo".to_string(),
                mutation_kind: crate::domain::planning::PlanningQueueMutationKind::Created,
                before_status: None,
                after_status: crate::domain::planning::TaskStatus::Ready,
                after_updated_at: "2026-07-15T00:00:00Z".to_string(),
                unchanged_since_mutation: true,
            }],
        });
    runtime
        .app_mut()
        .queue_overlay_ui_state
        .bind_receipt_undo_hit_area(Some(ratatui::layout::Rect::new(3, 5, 14, 1)));
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 3,
        row: 5,
        modifiers: KeyModifiers::NONE,
    }));

    assert!(runtime.take_redraw_request());
    assert_eq!(
        runtime.app().queue_overlay_ui_state.receipt_undo_hit_area(),
        None
    );
    assert_eq!(runtime.app().pending_queue_mutation_operation_id(), Some(1));
    assert!(!runtime.app().queue_receipt_undo_mouse_capture_requested());
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
                item_lifecycle: Default::default(),
            },
        )))
        .expect("background message should enqueue");

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        runtime.poll_background_messages();
        if matches!(
            &runtime.app().conversation_state,
            ConversationState::Ready(conversation)
                if conversation.status_text.contains("thread loaded / planning status: ready")
        ) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
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
            .contains("queue summary: now: queue idle: no executable planning task")
    );
    assert!(!conversation.status_text.contains("next: none"));
    assert!(!conversation.status_text.contains("proposed: none"));
    assert!(!conversation.status_text.contains("blocked: none"));
    fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn resumed_session_status_reads_core_projection() {
    let mut runtime = make_test_runtime();
    runtime
        .app_mut()
        .sync_core_planning_runtime_projection(PlanningRuntimeProjection::ready(
            "core snapshot prompt".to_string(),
            "core snapshot summary".to_string(),
            None,
        ));

    runtime.app_mut().surface_resumed_session_planning_context();

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(
        conversation.status_text.contains("planning status: ready"),
        "status should use core projection: {}",
        conversation.status_text
    );
}

#[test]
fn post_turn_evaluation_start_state_reads_core_projection() {
    let mut runtime = make_test_runtime();
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.thread_id = "thread-1".to_string();
    conversation.turn_activity.last_completed_turn_id = Some("turn-1".to_string());
    mark_core_turn_completed(&mut runtime, "thread-1", "turn-1");
    runtime
        .app_mut()
        .sync_core_planning_runtime_projection(PlanningRuntimeProjection::ready(
            "core snapshot prompt".to_string(),
            "queue idle from core snapshot".to_string(),
            None,
        ));

    runtime.app_mut().execute_conversation_runtime_effect(
        ConversationRuntimeEffect::EvaluatePostTurn {
            workspace_directory: "/tmp/workspace".to_string(),
            completed_turn_id: "turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
            execution_snapshot_capture: None,
        },
    );

    assert!(
        runtime
            .app()
            .core_runtime
            .test_post_turn_evaluation_is_in_flight("thread-1", "turn-1")
    );
    assert_eq!(
        runtime.app().planning_worker_panel_state.status,
        PlanningWorkerStatus::Idle,
        "ready/no-task core projection should preserve the panel"
    );
}

#[test]
fn post_turn_evaluation_started_event_applies_running_state_without_waiting_for_completion() {
    let mut runtime = make_test_runtime();
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.thread_id = "thread-1".to_string();
    conversation.turn_activity.last_completed_turn_id = Some("turn-1".to_string());
    mark_core_turn_completed(&mut runtime, "thread-1", "turn-1");
    runtime
        .app_mut()
        .sync_core_planning_runtime_projection(PlanningRuntimeProjection::invalid(
            "refresh required",
        ));
    runtime.app_mut().planning_worker_panel_state = PlanningWorkerPanelState {
        status: PlanningWorkerStatus::RefreshSucceeded,
        last_summary: Some("previous summary".to_string()),
        ..PlanningWorkerPanelState::default()
    };

    runtime.app_mut().execute_conversation_runtime_effect(
        ConversationRuntimeEffect::EvaluatePostTurn {
            workspace_directory: "/tmp/workspace".to_string(),
            completed_turn_id: "turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
            execution_snapshot_capture: None,
        },
    );

    assert!(
        runtime
            .app()
            .core_runtime
            .test_post_turn_evaluation_is_in_flight("thread-1", "turn-1")
    );
    assert_eq!(
        runtime.app().planning_worker_panel_state,
        PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshRunning,
            last_summary: Some("previous summary".to_string()),
            ..PlanningWorkerPanelState::default()
        }
    );
}

#[test]
fn ready_planning_projection_sync_is_ignored_without_a_ready_conversation() {
    let mut runtime = make_test_runtime();
    let expected_projection = PlanningRuntimeProjection::ready(
        "active prompt".to_string(),
        "active summary".to_string(),
        None,
    );
    runtime
        .app_mut()
        .sync_core_planning_runtime_projection(expected_projection.clone());
    runtime.app_mut().conversation_state = ConversationState::Loading;

    runtime
        .app_mut()
        .sync_ready_conversation_planning_runtime_projection(PlanningRuntimeProjection::invalid(
            "loading projection must not replace active state",
        ));

    assert_eq!(
        runtime.app().planning_runtime_projection_snapshot(),
        expected_projection
    );
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
fn stale_queue_authority_completion_requires_a_load_for_the_current_thread_context() {
    let mut runtime = make_test_runtime();
    runtime
        .app_mut()
        .dispatch_shell_chrome(ShellChromeEvent::QueueOverlayShown);
    let stale_context = runtime.app().current_queue_mutation_context();
    let stale_correlation = QueueAuthorityLoadCorrelation::new(
        1,
        stale_context.workspace_directory,
        stale_context.active_thread_id,
    );
    runtime
        .app_mut()
        .begin_queue_overlay_authority_load(stale_correlation.clone());
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.thread_id = "thread-after-load-started".to_string();
    let replacement_context = runtime.app().current_queue_mutation_context();
    let replacement_correlation = QueueAuthorityLoadCorrelation::new(
        1,
        replacement_context.workspace_directory.clone(),
        replacement_context.active_thread_id.clone(),
    );

    runtime
        .app_mut()
        .apply_core_event(AppEvent::QueueAuthorityLoaded {
            correlation: stale_correlation,
            result: Err(QueueAuthorityLoadError::RuntimeProjectionUnavailable),
        });

    assert!(
        runtime
            .app()
            .queue_overlay_ui_state
            .loading_request(&replacement_correlation)
            .is_some()
    );
    assert!(!runtime.app().queue_overlay_authority_load_required());
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
    let correlation = runtime.app.core_runtime.begin_test_turn_submission();

    runtime
        .app
        .tx
        .send(BackgroundMessage::ConversationStream {
            correlation,
            event: ConversationStreamEvent::StatusUpdated {
                text: "provider is thinking".to_string(),
            },
        })
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
fn idle_background_poll_after_initial_refresh_does_not_request_redraw() {
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().planning_workspace_directory();
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime
        .app()
        .core_runtime
        .snapshot()
        .planning_parallel
        .planning_runtime_workspace_directory
        .as_deref()
        != Some(workspace_directory.as_str())
        && Instant::now() < deadline
    {
        runtime.poll_background_messages();
        thread::yield_now();
    }
    assert_eq!(
        runtime
            .app()
            .core_runtime
            .snapshot()
            .planning_parallel
            .planning_runtime_workspace_directory
            .as_deref(),
        Some(workspace_directory.as_str()),
        "fixture planning runtime refresh should settle before idle polling"
    );
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
// duplicate worker results must not overwrite the core planning projection,
// current transcript status, runtime notices, or planning worker panel.
#[test]
fn stale_post_turn_evaluation_background_message_is_ignored() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    let workspace_directory = runtime.app().planning_workspace_directory();
    let initial_refresh_deadline = Instant::now() + Duration::from_secs(2);
    while runtime
        .app()
        .core_runtime
        .snapshot()
        .planning_parallel
        .planning_runtime_workspace_directory
        .as_deref()
        != Some(workspace_directory.as_str())
        && Instant::now() < initial_refresh_deadline
    {
        runtime.poll_background_messages();
        thread::yield_now();
    }
    assert_eq!(
        runtime
            .app()
            .core_runtime
            .snapshot()
            .planning_parallel
            .planning_runtime_workspace_directory
            .as_deref(),
        Some(workspace_directory.as_str()),
        "the fixture's exact initial refresh must settle before testing stale writer events"
    );
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.thread_id = "thread-1".to_string();
    conversation.status_text = "session ready".to_string();
    conversation.turn_activity.last_completed_turn_id = Some("turn-2".to_string());
    mark_core_turn_completed(&mut runtime, "thread-1", "turn-2");
    let expected_projection = PlanningRuntimeProjection::ready(
        "current prompt".to_string(),
        "current summary".to_string(),
        None,
    );
    runtime
        .app_mut()
        .sync_core_planning_runtime_projection(expected_projection.clone());

    runtime
        .app
        .tx
        .send(post_turn_evaluation_completed_message(
            "thread-1",
            "turn-1",
            workspace_directory,
            PlanningRuntimeProjection::invalid("stale projection".to_string()),
            PostTurnEvaluationOutcome {
                provenance: PostTurnEvaluationProvenance::new("turn-1".to_string()),
                planning_repair_state: None,
                runtime_notices: vec!["stale notice".to_string()],
                action: PostTurnContinuationAction::SkipAutoFollow {
                    reason: AutoFollowSkipReason::PostTurnContinuationPaused,
                },
                operator_alerts: Vec::new(),
            },
            PlanningWorkerPanelState {
                status: PlanningWorkerStatus::RefreshSucceeded,
                last_operation_label: None,
                last_queue_summary: Some("queue head: stale".to_string()),
                last_summary: Some("stale".to_string()),
                last_rejected_summary: None,
                last_notice_detail: None,
                last_prompt: None,
                last_response: None,
                last_host_detail: None,
            },
        ))
        .expect("background message should enqueue");

    runtime.poll_background_messages();
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.status_text, "session ready");
    assert!(conversation.runtime_notices.is_empty());
    assert_eq!(
        runtime.app().planning_runtime_projection_snapshot(),
        expected_projection
    );
    assert!(
        runtime
            .app()
            .planning_worker_panel_state
            .last_summary
            .is_none()
    );
}

#[test]
fn accepted_post_turn_evaluation_preserves_exact_domain_worker_state() {
    let mut runtime = make_test_runtime();
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.thread_id = "thread-1".to_string();
    conversation.turn_activity.last_completed_turn_id = Some("turn-1".to_string());
    mark_core_turn_completed(&mut runtime, "thread-1", "turn-1");
    arm_core_post_turn_evaluation(&mut runtime, "thread-1", "turn-1");
    let expected_worker_state = PlanningWorkerPanelState {
        status: PlanningWorkerStatus::RepairFailed,
        last_operation_label: Some("repair projection".to_string()),
        last_summary: Some("accepted summary".to_string()),
        last_rejected_summary: Some("rejected candidate".to_string()),
        last_queue_summary: Some("queue head: task-1".to_string()),
        last_notice_detail: Some("one blocker remains".to_string()),
        last_prompt: Some("worker prompt".to_string()),
        last_response: Some("worker response".to_string()),
        last_host_detail: Some("host detail".to_string()),
    };
    let workspace_directory = runtime.app().planning_workspace_directory();

    runtime
        .app
        .tx
        .send(post_turn_evaluation_completed_message(
            "thread-1",
            "turn-1",
            workspace_directory,
            PlanningRuntimeProjection::invalid("blocking projection".to_string()),
            PostTurnEvaluationOutcome {
                provenance: PostTurnEvaluationProvenance::new("turn-1".to_string()),
                planning_repair_state: None,
                runtime_notices: Vec::new(),
                action: PostTurnContinuationAction::SkipAutoFollow {
                    reason: AutoFollowSkipReason::PlanningBlocked,
                },
                operator_alerts: Vec::new(),
            },
            expected_worker_state.clone(),
        ))
        .expect("background message should enqueue");

    runtime.poll_background_messages();

    assert_eq!(
        runtime.app().planning_worker_panel_state,
        expected_worker_state
    );
}

#[test]
fn duplicate_post_turn_evaluation_for_same_turn_is_ignored() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    let workspace_directory = runtime.app().planning_workspace_directory();
    let initial_refresh_deadline = Instant::now() + Duration::from_secs(2);
    while runtime
        .app()
        .core_runtime
        .snapshot()
        .planning_parallel
        .planning_runtime_workspace_directory
        .as_deref()
        != Some(workspace_directory.as_str())
        && Instant::now() < initial_refresh_deadline
    {
        runtime.poll_background_messages();
        thread::yield_now();
    }
    assert_eq!(
        runtime
            .app()
            .core_runtime
            .snapshot()
            .planning_parallel
            .planning_runtime_workspace_directory
            .as_deref(),
        Some(workspace_directory.as_str()),
        "the fixture's exact initial refresh must settle before testing duplicate writer events"
    );
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.thread_id = "thread-1".to_string();
    conversation.turn_activity.last_completed_turn_id = Some("turn-1".to_string());
    mark_core_turn_completed(&mut runtime, "thread-1", "turn-1");
    arm_core_post_turn_evaluation(&mut runtime, "thread-1", "turn-1");
    let build_message = |notice: &str| {
        post_turn_evaluation_completed_message(
            "thread-1",
            "turn-1",
            workspace_directory.clone(),
            PlanningRuntimeProjection::invalid(notice.to_string()),
            PostTurnEvaluationOutcome {
                provenance: PostTurnEvaluationProvenance::new("turn-1".to_string()),
                planning_repair_state: None,
                runtime_notices: vec![notice.to_string()],
                action: PostTurnContinuationAction::SkipAutoFollow {
                    reason: AutoFollowSkipReason::PlanningBlocked,
                },
                operator_alerts: Vec::new(),
            },
            PlanningWorkerPanelState {
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
        )
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
    assert_eq!(
        runtime.app().planning_runtime_projection_snapshot(),
        PlanningRuntimeProjection::invalid("first evaluation")
    );
}
// Resize is a rendering concern only: it should request a redraw without
// mutating committed transcript rows or the active input buffer.
#[test]
fn resize_event_requests_redraw() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    assert_eq!(runtime.terminal_resize_epoch(), 0);

    runtime.handle_terminal_event(Event::Resize(48, 10));
    assert_eq!(runtime.terminal_resize_epoch(), 1);
    runtime.handle_terminal_event(Event::Resize(120, 40));

    assert!(runtime.take_redraw_request());
    assert_eq!(runtime.terminal_resize_epoch(), 2);
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
    conversation.composer.input_buffer = "buffered prompt".to_string();

    runtime.take_redraw_request();
    runtime.handle_terminal_event(Event::Resize(120, 40));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.composer.input_buffer, "buffered prompt");
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
    let mut runtime = make_test_runtime_with_github_review_polling(
        super::super::github_polling::GithubReviewPollingBootstrap::from_env_values(
            Some("acme/widgets#42".to_string()),
            Some("30".to_string()),
        ),
        |_| {
            Ok(Some((
                GithubPullRequestTarget::new("acme/widgets", 42),
                GithubReviewPollerService::new(Arc::new(FakeGithubReviewPollerPort)),
            )))
        },
    );

    runtime.record_successful_frame_delivery();
    runtime.poll_background_messages();

    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        runtime.poll_background_messages();
        if matches!(
            &runtime.app().github_review_polling_state,
            super::super::github_polling::GithubReviewPollingState::Active(polling_state)
                if polling_state.snapshot.is_some()
        ) {
            break;
        }
        thread::yield_now();
    }
    let super::super::github_polling::GithubReviewPollingState::Active(polling_state) =
        &runtime.app().github_review_polling_state
    else {
        panic!("expected active github review polling state");
    };
    assert!(polling_state.snapshot.is_some());
    assert!(polling_state.last_error.is_none());
    assert!(polling_state.pending_correlation.is_none());
    assert!(
        !runtime
            .app()
            .github_review_polling_state
            .poll_due(Instant::now())
    );
}

#[test]
fn github_review_setup_failure_stays_fail_closed() {
    let mut runtime = make_test_runtime_with_github_review_polling(
        super::super::github_polling::GithubReviewPollingBootstrap::from_env_values(
            Some("acme/widgets#42".to_string()),
            Some("30".to_string()),
        ),
        |_| Err(anyhow::anyhow!("github credentials unavailable")),
    );

    runtime.record_successful_frame_delivery();
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline
        && !matches!(
            runtime.app().github_review_polling_state,
            super::super::github_polling::GithubReviewPollingState::SetupError { .. }
        )
    {
        runtime.poll_background_messages();
        thread::yield_now();
    }

    let super::super::github_polling::GithubReviewPollingState::SetupError { message, .. } =
        &runtime.app().github_review_polling_state
    else {
        panic!("expected setup error state");
    };
    assert_eq!(message, "github credentials unavailable");
    assert!(
        !runtime
            .app()
            .github_review_polling_state
            .poll_due(Instant::now())
    );
}

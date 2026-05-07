use crate::adapter::inbound::tui::app::{ConversationState, NativeTuiApp, test_helpers};
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
use crate::application::port::outbound::startup_probe_port::{
    AppServerStartupContext, StartupProbePort,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::planning::{
    PlanningDraftEditorFile, PlanningDraftEditorSession, PlanningRuntimeSnapshot,
};
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModeReadinessSnapshot, ParallelModeReadinessState,
};
use crate::domain::recent_sessions::{RecentSessions, SessionCatalog};
use crate::domain::session_summary::SessionSummary;
use crate::domain::startup_diagnostics::StartupDiagnostics;
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
use anyhow::Result;
use std::sync::Arc;

/*
 * These fixtures are shared by shell rendering contract tests. They build a real NativeTuiApp and
 * real domain projections, but replace Codex app-server with deterministic data so assertions cover
 * presentation mapping instead of filesystem, process, or network behavior. The chosen values are
 * intentionally stable because many tests compare visible labels, paths, readiness summaries, and
 * overlay placement.
 */
struct FakeAppServerPort;

impl StartupProbePort for FakeAppServerPort {
    fn load_startup_context(&self) -> Result<AppServerStartupContext> {
        /*
         * StartupService reads this during app construction. Keeping every startup check healthy
         * lets rendering tests opt into failure states explicitly instead of inheriting unrelated
         * diagnostics from the fixture.
         */
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
    fn load_session_catalog(
        &self,
        _request: crate::domain::recent_sessions::SessionCatalogRequest,
    ) -> Result<SessionCatalog> {
        // Most shell contracts start from an empty history and inject session rows only when needed.
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
        /*
         * Session-loading tests care that the requested thread id survives the port boundary. The
         * rest of the snapshot stays sparse so history rendering is driven by the individual test.
         */
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
        // Shell rendering tests do not exercise stop side effects; they only need the command path to exist.
        Ok(())
    }

    fn run_new_thread_stream(
        &self,
        _cwd: &str,
        _prompt: &str,
        _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        // Streaming events are injected directly into app state by tests that need transcript output.
        Ok(())
    }

    fn run_turn_stream(
        &self,
        _thread_id: &str,
        _prompt: &str,
        _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        // Existing-thread turns use the same no-op boundary; rendering tests should not spawn workers.
        Ok(())
    }
}

pub(crate) fn make_test_app() -> NativeTuiApp {
    /*
     * Build through the production constructor so shell rendering tests observe the same service
     * graph as the TUI: startup/session/conversation services share one app-server port, planning
     * uses the filesystem workspace adapter, and parallel mode uses the test helper service. The
     * fixture then normalizes volatile UI state such as cwd, ASCII art, and planning runtime.
     */
    let codex_port = Arc::new(FakeAppServerPort);
    let mut app = NativeTuiApp::new(
        StartupService::new(codex_port.clone()),
        SessionService::new(codex_port.clone()),
        ConversationService::new(codex_port),
        Arc::new(
            crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort,
        ),
        test_helpers::test_parallel_mode_service(),
        crate::adapter::inbound::tui::app::test_helpers::test_planning_services(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        )),
    );
    app.show_startup_ascii_art = false;
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start with a ready draft conversation");
    };
    conversation.cwd = "/tmp/root".to_string();
    conversation.draft_workspace_directory = "/tmp/root".to_string();
    conversation.replace_planning_runtime_snapshot(PlanningRuntimeSnapshot::uninitialized());
    app
}

pub(crate) fn sample_startup_diagnostics() -> StartupDiagnostics {
    /*
     * This is the “fully ready” startup projection used by tests that need the shell beyond the
     * bootstrap screen. Keeping cwd/workspace/profile aligned with make_test_app prevents layout
     * assertions from changing based on mixed roots.
     */
    StartupDiagnostics {
        cwd: "/tmp/root".to_string(),
        codex_binary_ok: true,
        codex_binary_detail: "codex".to_string(),
        workspace_ok: true,
        workspace_path: "/tmp/root".to_string(),
        workspace_detail: "workspace found".to_string(),
        attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
        initialize_ok: true,
        initialize_detail: "app-server initialize ok".to_string(),
        account_ok: true,
        account_detail: "account ok".to_string(),
        warnings: Vec::new(),
        schema_snapshot: "snapshot.json".to_string(),
    }
}

pub(crate) fn sample_session(id: &str) -> SessionSummary {
    /*
     * Session browser tests need a row with every optional column populated: friendly name, preview,
     * source/model metadata, timestamp, status, file path, and branch. The id remains parameterized
     * so selection/order tests can create multiple rows without duplicating the full projection.
     */
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

pub(crate) fn sample_parallel_mode_snapshot(
    readiness: ParallelModeReadinessState,
) -> ParallelModeReadinessSnapshot {
    /*
     * Parallel-mode rendering has two layers: overall readiness and per-capability detail. The git
     * capability stays ready as a stable baseline, while the planning capability mirrors the input
     * readiness so tests can check each footer/sidebar variant without rebuilding the whole vector.
     */
    ParallelModeReadinessSnapshot::new(
        "/tmp/root",
        readiness,
        vec![
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GitRepository,
                ParallelModeCapabilityState::Ready,
                "git repo detected at /tmp/root",
                None,
            ),
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::Planning,
                match readiness {
                    ParallelModeReadinessState::Ready => ParallelModeCapabilityState::Ready,
                    ParallelModeReadinessState::Degraded => ParallelModeCapabilityState::Degraded,
                    ParallelModeReadinessState::Blocked => ParallelModeCapabilityState::Blocked,
                    ParallelModeReadinessState::Repairing => ParallelModeCapabilityState::Repairing,
                },
                "planning workspace is healthy",
                Some("review the readiness panel".to_string()),
            ),
        ],
        Some("planning: degraded / cause: planning workspace is healthy / next action: review the readiness panel".to_string()),
    )
}

pub(crate) fn sample_planning_editor_session() -> PlanningDraftEditorSession {
    /*
     * Planning editor contract tests need multiple editable files to exercise tab lists, path
     * labels, staged-file rendering, and body previews. The intentionally repetitive active_path
     * values stress presentation code that should disambiguate by staged path/body rather than
     * assuming active paths are unique.
     */
    PlanningDraftEditorSession {
        draft_name: "bootstrap-test".to_string(),
        draft_directory: "/tmp/root/.codex-exec-loop/planning/drafts/bootstrap-test".to_string(),
        editable_files: vec![
            PlanningDraftEditorFile {
                active_path: ".codex-exec-loop/planning/result-output.md".to_string(),
                staged_path:
                    "/tmp/root/.codex-exec-loop/planning/drafts/bootstrap-test/result-output.md"
                        .to_string(),
                body: "version = 1\n".to_string(),
            },
            PlanningDraftEditorFile {
                active_path: ".codex-exec-loop/planning/result-output.md".to_string(),
                staged_path:
                    "/tmp/root/.codex-exec-loop/planning/drafts/bootstrap-test/.codex-exec-loop/planning/result-output.md"
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

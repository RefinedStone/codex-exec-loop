/*
Inline terminal adapter tests exercise terminal drawing, viewport replay, and host scrollback insertion from a
fully assembled `NativeTuiApp`. This fixture keeps the production service graph shape while replacing outbound
boundaries with deterministic local fakes, so renderer failures point at screen contracts rather than app-server IO.
*/
use std::sync::Arc;

use anyhow::Result;

use crate::adapter::inbound::tui::app::{ConversationState, NativeTuiApp};
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
use crate::application::port::outbound::startup_probe_port::{
    AppServerStartupContext, StartupProbePort,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningServices};
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::recent_sessions::{RecentSessions, SessionCatalog};

// Shared app-server fake for inline rendering tests.
// It models a healthy bridge and quiet streams because these tests seed UI state directly on NativeTuiApp.
struct FakeAppServerPort;

impl StartupProbePort for FakeAppServerPort {
    // Startup diagnostics stay green so inline terminal tests do not accidentally validate startup failure copy.
    fn load_startup_context(&self) -> Result<AppServerStartupContext> {
        Ok(AppServerStartupContext {
            attachment_profile:
                crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile::codex_app_server(),
            initialize_detail: "ok".to_string(),
            account_detail: "ok".to_string(),
            account_ok: true,
            warnings: Vec::new(),
        })
    }
}

impl SessionCatalogPort for FakeAppServerPort {
    // Return a successful empty catalog: session capability is available, but no session rows pollute viewport assertions.
    fn load_session_catalog(
        &self,
        _request: crate::domain::recent_sessions::SessionCatalogRequest,
    ) -> Result<SessionCatalog> {
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

    // Resumed-session paths need a stable snapshot whose identity follows the request but whose content is fixed.
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

    // Process control is outside inline rendering scope; success keeps shutdown wiring from blocking fixture setup.
    fn request_stop_all_sessions(&self) -> Result<()> {
        Ok(())
    }

    // New-thread streams are intentionally silent because individual tests inject the conversation state they need.
    fn run_new_thread_stream(
        &self,
        _cwd: &str,
        _prompt: &str,
        _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        Ok(())
    }

    // Existing-thread streams follow the same deterministic contract; live deltas belong in targeted runtime tests.
    fn run_turn_stream(
        &self,
        _thread_id: &str,
        _prompt: &str,
        _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        Ok(())
    }
}

// Build the realistic app shell that inline terminal tests render against.
// The service graph mirrors production wiring while every nondeterministic boundary is pinned to a local fake/default.
pub(super) fn make_test_app() -> NativeTuiApp {
    // One Arc-backed port is shared across startup, session, and conversation services to preserve production ownership shape.
    let codex_port = Arc::new(FakeAppServerPort);
    let mut app = NativeTuiApp::new(
        StartupService::new(codex_port.clone()),
        SessionService::new(codex_port.clone()),
        ConversationService::new(codex_port),
        // Parallel agents are not part of inline frame rendering, so the worker boundary is present but inert.
        Arc::new(
            crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort,
        ),
        // The test parallel-mode service exposes the same control surface without slot/worktree orchestration.
        crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
        // Planning services stay real enough for status copy, then the conversation snapshot is reset to a neutral baseline below.
        PlanningServices::from_workspace_port(Arc::new(FilesystemPlanningWorkspaceAdapter::new())),
    );
    // Inline rendering fixtures assume the app opens on an editable draft; fail loudly if constructor semantics change.
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start with a ready draft conversation");
    };
    // Keep cwd and draft workspace identical so viewport assertions are not also testing workspace mismatch behavior.
    conversation.cwd = "/tmp/root".to_string();
    conversation.draft_workspace_directory = "/tmp/root".to_string();
    // The neutral planning baseline makes tests opt in explicitly when they need queue, pause, or task state.
    conversation.replace_planning_runtime_snapshot(PlanningRuntimeSnapshot::uninitialized());
    app
}

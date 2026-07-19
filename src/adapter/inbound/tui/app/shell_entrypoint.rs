#[cfg(test)]
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;

use super::github_polling::GithubReviewPollingBootstrap;
use super::shell_frontend::ShellFrontend;
use super::shell_runtime::ShellRuntime;
use super::{NativeTuiApp, NativeTuiParallelModeBinding, ShellChromeEvent};
use crate::composition::production;

// shell_entrypoint owns terminal bootstrap only. Production service wiring lives
// in crate::composition::production so TUI remains an inbound adapter.
pub fn run() -> Result<()> {
    let shutdown = crate::shutdown::GracefulShutdown::install()?;
    let frontend = ShellFrontend::new();
    let runtime = prepare_runtime(build_default_app());
    frontend.run(runtime, &shutdown)
}

fn build_default_app() -> NativeTuiApp {
    let services = production::build_native_tui_application_services();
    let parallel_mode_binding =
        NativeTuiParallelModeBinding::from_composition(services.parallel_mode_control_plane);
    let repo_root = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let github_review_polling =
        GithubReviewPollingBootstrap::from_environment(&repo_root, Instant::now());
    NativeTuiApp::new_with_github_review_polling(
        services.startup_service,
        services.session_service,
        services.conversation_service,
        parallel_mode_binding,
        github_review_polling,
    )
}

fn prepare_runtime(mut app: NativeTuiApp) -> ShellRuntime {
    /*
     * Startup checks are requested before the frontend event loop begins. ShellRuntime will process
     * the resulting background work through the same message path as later terminal events, so the
     * first frame can show loading state without blocking terminal initialization.
     */
    app.dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested);
    ShellRuntime::new(app)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;

    use super::super::ratatui_frontend::prepare_runtime_for_due_draw;
    use super::*;
    use crate::adapter::inbound::tui::shell_chrome::StartupState;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::app_server_prompt_log_port::{
        AppServerPromptLogMaintenanceMode, AppServerPromptLogMaintenancePort,
    };
    use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
    use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
    use crate::application::port::outbound::startup_probe_port::{
        AppServerStartupContext, StartupProbePort,
    };
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneComposition;
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::ConversationSnapshot;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog};
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    /*
     * The entrypoint tests need a healthy app-server port that never starts real streams. Returning
     * empty catalogs and snapshots is enough to prove prepare_runtime wires startup work without
     * letting background session/conversation behavior dominate the assertion.
     */
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

    fn make_test_app() -> NativeTuiApp {
        let codex_port = Arc::new(FakeAppServerPort);
        make_test_app_with_startup_service(StartupService::new(codex_port.clone()), codex_port)
    }

    fn make_test_app_with_startup_service(
        startup_service: StartupService,
        codex_port: Arc<FakeAppServerPort>,
    ) -> NativeTuiApp {
        /*
         * Test construction mirrors the production service shape but swaps external boundaries:
         * fake app-server, noop parallel worker, test parallel-mode service, and local filesystem
         * planning workspace. That keeps prepare_runtime tests about shell startup sequencing.
         */
        let planning = crate::adapter::inbound::tui::app::test_helpers::test_planning_services(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        );
        let parallel_mode_control_plane_composition = ParallelModeControlPlaneComposition::new(
            crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
            planning,
            Arc::new(
                crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort,
            ),
        );
        let parallel_mode_binding =
            NativeTuiParallelModeBinding::from_composition(parallel_mode_control_plane_composition);
        NativeTuiApp::new(
            startup_service,
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            parallel_mode_binding,
        )
    }

    struct GatedPromptLogMaintenancePort {
        entered: mpsc::SyncSender<(String, AppServerPromptLogMaintenanceMode)>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl AppServerPromptLogMaintenancePort for GatedPromptLogMaintenancePort {
        fn maintain_app_server_prompt_logs(
            &self,
            workspace_dir: &str,
            mode: AppServerPromptLogMaintenanceMode,
        ) -> Result<()> {
            let _ = self.entered.send((workspace_dir.to_string(), mode));
            self.release
                .lock()
                .expect("maintenance release mutex should not be poisoned")
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| anyhow::anyhow!("maintenance gate failed: {error}"))?;
            Ok(())
        }
    }

    #[test]
    fn prepare_runtime_requests_startup_checks_before_frontend_run() {
        let runtime = prepare_runtime(make_test_app());

        assert!(matches!(runtime.app().startup_state, StartupState::Loading));
    }

    #[test]
    fn prepare_runtime_keeps_runtime_ready_for_background_messages() {
        let runtime = prepare_runtime(make_test_app());

        assert!(!runtime.should_quit());
        assert!(matches!(runtime.app().startup_state, StartupState::Loading));
    }

    #[test]
    fn gated_prompt_log_maintenance_does_not_block_build_prepare_or_first_draw() {
        const GATE_DURATION: Duration = Duration::from_millis(650);
        const NONBLOCKING_DEADLINE: Duration = Duration::from_millis(300);
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let maintenance = Arc::new(GatedPromptLogMaintenancePort {
            entered: entered_tx,
            release: Mutex::new(release_rx),
        });
        let codex_port = Arc::new(FakeAppServerPort);

        let started = Instant::now();
        let app = make_test_app_with_startup_service(
            StartupService::new(codex_port.clone()).with_prompt_log_maintenance(
                maintenance,
                AppServerPromptLogMaintenanceMode::ClearAll,
            ),
            codex_port,
        );
        let mut runtime = prepare_runtime(app);
        assert!(
            started.elapsed() < NONBLOCKING_DEADLINE,
            "app construction and prepare_runtime must return before gated maintenance completes"
        );

        let (workspace_directory, mode) = entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("startup worker should enter prompt-log maintenance");
        assert!(!workspace_directory.trim().is_empty());
        assert_eq!(mode, AppServerPromptLogMaintenanceMode::ClearAll);
        assert!(
            prepare_runtime_for_due_draw(&mut runtime, || Ok(None))
                .expect("first draw preparation should remain available")
        );

        std::thread::sleep(GATE_DURATION);
        runtime.poll_background_messages();
        assert!(matches!(runtime.app().startup_state, StartupState::Loading));

        let _ = release_tx.send(());
    }
}

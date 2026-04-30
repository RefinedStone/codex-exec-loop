use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;

use crate::adapter::outbound::app_server::AppServerPlanningWorkerAdapter;
use crate::adapter::outbound::app_server::CodexAppServerAdapter;
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::github::GithubAutomationAdapter;
use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_worker_port::PlanningWorkerPort;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::planning::PlanningServices;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;

use super::github_polling::GithubReviewPollingBootstrap;
use super::shell_frontend::ShellFrontend;
use super::shell_runtime::ShellRuntime;
use super::{NativeTuiApp, ShellChromeEvent};

pub fn run() -> Result<()> {
    let frontend = ShellFrontend::new();
    let runtime = prepare_runtime(build_default_app());
    frontend.run(runtime)
}

fn build_default_app() -> NativeTuiApp {
    let app_server_adapter = Arc::new(CodexAppServerAdapter::new(
        "codex-exec-loop-native",
        env!("CARGO_PKG_VERSION"),
    ));
    let sqlite_planning_authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning_authority: Arc<dyn PlanningAuthorityPort> = sqlite_planning_authority.clone();
    let github_automation: Arc<dyn GithubAutomationPort> = Arc::new(GithubAutomationAdapter::new());
    let planning_workspace_port = Arc::new(FilesystemPlanningWorkspaceAdapter::new());
    let planning_worker_port: Arc<dyn PlanningWorkerPort> = Arc::new(
        AppServerPlanningWorkerAdapter::new(app_server_adapter.clone()),
    );
    let startup_service = StartupService::new(app_server_adapter.clone());
    let session_service = SessionService::new(app_server_adapter.clone());
    let conversation_service = ConversationService::new(app_server_adapter.clone());
    let parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort> = app_server_adapter.clone();
    let planning = PlanningServices::from_ports(
        planning_workspace_port,
        planning_authority.clone(),
        sqlite_planning_authority,
        planning_worker_port,
    );
    let parallel_mode_service = ParallelModeService::new(
        planning_authority,
        github_automation,
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    let mut app = NativeTuiApp::new(
        startup_service,
        session_service,
        conversation_service,
        parallel_agent_worker_port,
        parallel_mode_service,
        planning,
    );
    let repo_root = std::env::current_dir().unwrap_or_else(|_| ".".into());
    app.configure_github_review_polling(GithubReviewPollingBootstrap::from_environment(
        &repo_root,
        Instant::now(),
    ));

    app
}

fn prepare_runtime(mut app: NativeTuiApp) -> ShellRuntime {
    app.dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested);
    ShellRuntime::new(app)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::adapter::inbound::tui::shell_chrome::StartupState;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::codex_app_server_port::{
        AppServerStartupContext, CodexAppServerPort,
    };
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::domain::conversation::ConversationSnapshot;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog};
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    #[derive(Default)]
    struct FakeCodexAppServerPort;

    impl CodexAppServerPort for FakeCodexAppServerPort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            Ok(AppServerStartupContext {
                attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
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

    fn make_test_app() -> NativeTuiApp {
        let codex_port = Arc::new(FakeCodexAppServerPort);
        NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            Arc::new(
                crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort,
            ),
            crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
            PlanningServices::from_workspace_port(Arc::new(
                FilesystemPlanningWorkspaceAdapter::new(),
            )),
        )
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
}

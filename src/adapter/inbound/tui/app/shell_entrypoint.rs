use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;

use crate::adapter::outbound::app_server_planning_worker_adapter::AppServerPlanningWorkerAdapter;
use crate::adapter::outbound::codex_app_server_adapter::CodexAppServerAdapter;
use crate::adapter::outbound::filesystem_followup_template_adapter::FilesystemFollowupTemplateAdapter;
use crate::adapter::outbound::filesystem_planning_workspace_adapter::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::codex_app_server_port::CodexAppServerPort;
use crate::application::port::outbound::followup_template_port::FollowupTemplatePort;
use crate::application::port::outbound::planning_worker_port::PlanningWorkerPort;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::followup_template_service::FollowupTemplateService;
use crate::application::service::planning_services::PlanningServices;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;

use super::github_polling::GithubReviewPollingBootstrap;
use super::shell_frontend::ShellFrontend;
use super::shell_runtime::ShellRuntime;
use super::{NativeTuiApp, ShellChromeEvent};

pub fn run() -> Result<()> {
    let frontend = ShellFrontend::from_environment();
    let runtime = prepare_runtime(build_default_app(), frontend.mode());
    frontend.run(runtime)
}

fn build_default_app() -> NativeTuiApp {
    let app_server_adapter = Arc::new(CodexAppServerAdapter::new(
        "codex-exec-loop-native",
        env!("CARGO_PKG_VERSION"),
    ));
    let codex_app_server_port: Arc<dyn CodexAppServerPort> = app_server_adapter.clone();
    let followup_template_port: Arc<dyn FollowupTemplatePort> =
        Arc::new(FilesystemFollowupTemplateAdapter::new());
    let planning_worker_port: Arc<dyn PlanningWorkerPort> =
        Arc::new(AppServerPlanningWorkerAdapter::new(app_server_adapter));
    let startup_service = StartupService::new(codex_app_server_port.clone());
    let session_service = SessionService::new(codex_app_server_port.clone());
    let conversation_service = ConversationService::new(codex_app_server_port);
    let followup_template_service = FollowupTemplateService::new(followup_template_port);
    let planning_services = PlanningServices::from_ports(
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        planning_worker_port,
    );
    let mut app = NativeTuiApp::new(
        startup_service,
        session_service,
        conversation_service,
        followup_template_service,
        planning_services,
    );
    let repo_root = std::env::current_dir().unwrap_or_else(|_| ".".into());
    app.configure_github_review_polling(GithubReviewPollingBootstrap::from_environment(
        &repo_root,
        Instant::now(),
    ));

    app
}

fn prepare_runtime(
    mut app: NativeTuiApp,
    frontend_mode: super::shell_frontend::ShellFrontendMode,
) -> ShellRuntime {
    app.dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested);
    ShellRuntime::new(app, frontend_mode)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::adapter::inbound::tui::shell_chrome::StartupState;
    use crate::adapter::outbound::filesystem_planning_workspace_adapter::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::codex_app_server_port::{
        AppServerStartupContext, CodexAppServerPort,
    };
    use crate::application::port::outbound::followup_template_port::{
        FollowupTemplatePort, WorkspaceFollowupTemplateRecord,
    };
    use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};
    use crate::domain::recent_sessions::RecentSessions;

    #[derive(Default)]
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
            PlanningServices::from_workspace_port(Arc::new(
                FilesystemPlanningWorkspaceAdapter::new(),
            )),
        )
    }

    #[test]
    fn prepare_runtime_requests_startup_checks_before_frontend_run() {
        let runtime = prepare_runtime(
            make_test_app(),
            super::super::shell_frontend::ShellFrontendMode::InlineMainBuffer,
        );

        assert!(matches!(runtime.app().startup_state, StartupState::Loading));
    }

    #[test]
    fn prepare_runtime_keeps_runtime_ready_for_background_messages() {
        let runtime = prepare_runtime(
            make_test_app(),
            super::super::shell_frontend::ShellFrontendMode::InlineMainBuffer,
        );

        assert!(!runtime.should_quit());
        assert!(matches!(runtime.app().startup_state, StartupState::Loading));
    }
}

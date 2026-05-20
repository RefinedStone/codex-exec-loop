use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::adapter::outbound::app_server::{AppServerPlanningWorkerAdapter, CodexAppServerAdapter};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::github::{GithubAutomationAdapter, GithubReviewPollerAdapter};
use crate::adapter::outbound::telegram::CurlTelegramBotAdapter;
use crate::application::port::outbound::app_server_prompt_log_port::AppServerPromptLogPort;
use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_worker_port::PlanningWorkerPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::port::outbound::telegram_bot_port::TelegramBotPort;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::application::service::parallel_mode::{
    ParallelModeService, control_plane::ParallelModeControlPlaneComposition,
};
use crate::application::service::planning::{
    PlanningAdminFacadeService, PlanningControlFacadeService, PlanningControlService,
    PlanningServices,
};
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::github_review::GithubPullRequestTarget;

const APP_SERVER_CLIENT_NAME: &str = "codex-exec-loop-native";

pub(crate) struct ProductionAdminApplication {
    pub(crate) facade: Arc<PlanningAdminFacadeService>,
    pub(crate) parallel_mode_control_plane: Arc<ParallelModeControlPlaneComposition>,
    pub(crate) app_server_prompt_log_port: Arc<dyn AppServerPromptLogPort>,
}

pub(crate) struct ProductionTelegramApplication {
    pub(crate) control_service: PlanningControlService,
    pub(crate) parallel_mode_control_plane: Arc<ParallelModeControlPlaneComposition>,
}

pub(crate) struct ProductionNativeTuiApplicationServices {
    pub(crate) startup_service: StartupService,
    pub(crate) session_service: SessionService,
    pub(crate) conversation_service: ConversationService,
    pub(crate) parallel_mode_control_plane: ParallelModeControlPlaneComposition,
}

struct ProductionSharedPorts {
    app_server_adapter: Arc<CodexAppServerAdapter>,
    planning_authority_port: Arc<dyn PlanningAuthorityPort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_worker_port: Arc<dyn PlanningWorkerPort>,
    parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort>,
    app_server_prompt_log_port: Arc<dyn AppServerPromptLogPort>,
}

pub(crate) fn build_planning_services() -> PlanningServices {
    let ports = build_shared_ports();
    planning_services_from_ports(&ports)
}

pub(crate) fn build_planning_control_service(workspace_dir: String) -> PlanningControlService {
    PlanningControlService::new(Arc::new(PlanningControlFacadeService::new(
        workspace_dir,
        build_planning_services(),
    )))
}

pub(crate) fn build_parallel_mode_control_plane_composition() -> ParallelModeControlPlaneComposition
{
    let ports = build_shared_ports();
    let planning = planning_services_from_ports(&ports);
    parallel_mode_control_plane_from_parts(
        planning,
        ports.planning_authority_port,
        ports.parallel_agent_worker_port,
    )
}

pub(crate) fn build_admin_application(workspace_dir: String) -> ProductionAdminApplication {
    let ports = build_shared_ports();
    let planning = planning_services_from_ports(&ports);
    let parallel_mode_control_plane = Arc::new(parallel_mode_control_plane_from_parts(
        planning.clone(),
        ports.planning_authority_port.clone(),
        ports.parallel_agent_worker_port.clone(),
    ));
    let facade = Arc::new(PlanningAdminFacadeService::from_planning_with_authority(
        workspace_dir,
        planning,
        ports.planning_workspace_port,
        ports.planning_authority_port,
        ports.planning_task_repository_port,
    ));
    ProductionAdminApplication {
        facade,
        parallel_mode_control_plane,
        app_server_prompt_log_port: ports.app_server_prompt_log_port,
    }
}

pub(crate) fn build_telegram_application(workspace_dir: String) -> ProductionTelegramApplication {
    let ports = build_shared_ports();
    let planning = planning_services_from_ports(&ports);
    let control_service = PlanningControlService::new(Arc::new(PlanningControlFacadeService::new(
        workspace_dir,
        planning.clone(),
    )));
    let parallel_mode_control_plane = Arc::new(parallel_mode_control_plane_from_parts(
        planning,
        ports.planning_authority_port,
        ports.parallel_agent_worker_port,
    ));
    ProductionTelegramApplication {
        control_service,
        parallel_mode_control_plane,
    }
}

pub(crate) fn build_telegram_bot_port(token: String) -> Arc<dyn TelegramBotPort> {
    Arc::new(CurlTelegramBotAdapter::new(token))
}

pub(crate) fn build_native_tui_application_services() -> ProductionNativeTuiApplicationServices {
    let ports = build_shared_ports();
    let startup_service = StartupService::new(ports.app_server_adapter.clone());
    let session_service = SessionService::new(ports.app_server_adapter.clone());
    let conversation_service = ConversationService::new(ports.app_server_adapter.clone());
    let planning = planning_services_from_ports(&ports);
    let parallel_mode_control_plane = parallel_mode_control_plane_from_parts(
        planning,
        ports.planning_authority_port,
        ports.parallel_agent_worker_port,
    );
    ProductionNativeTuiApplicationServices {
        startup_service,
        session_service,
        conversation_service,
        parallel_mode_control_plane,
    }
}

pub(crate) fn build_github_review_poller_service(
    repo_root: &Path,
) -> Result<GithubReviewPollerService> {
    let adapter = GithubReviewPollerAdapter::from_local_github_credentials(repo_root)?;
    let port: Arc<dyn GithubReviewPollerPort> = Arc::new(adapter);
    Ok(GithubReviewPollerService::new(port))
}

pub(crate) fn discover_github_review_poller_service_for_current_branch(
    repo_root: &Path,
    base_branch: &str,
) -> Result<Option<(GithubPullRequestTarget, GithubReviewPollerService)>> {
    let adapter = match GithubReviewPollerAdapter::from_local_github_credentials(repo_root) {
        Ok(adapter) => adapter,
        Err(_) => return Ok(None),
    };
    let Some(target) = adapter.find_open_pull_request_for_current_branch(repo_root, base_branch)?
    else {
        return Ok(None);
    };
    let port: Arc<dyn GithubReviewPollerPort> = Arc::new(adapter);
    Ok(Some((target, GithubReviewPollerService::new(port))))
}

fn build_shared_ports() -> ProductionSharedPorts {
    let planning_authority_adapter = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning_authority_port: Arc<dyn PlanningAuthorityPort> =
        planning_authority_adapter.clone();
    let app_server_prompt_log_port: Arc<dyn AppServerPromptLogPort> =
        planning_authority_adapter.clone();
    let app_server_adapter = app_server_adapter(app_server_prompt_log_port.clone());
    let planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort> =
        planning_authority_adapter.clone();
    let planning_workspace_port: Arc<dyn PlanningWorkspacePort> =
        Arc::new(FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(
            planning_authority_adapter.clone(),
        ));
    let planning_worker_port: Arc<dyn PlanningWorkerPort> = Arc::new(
        AppServerPlanningWorkerAdapter::new(app_server_adapter.clone()),
    );
    let parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort> = app_server_adapter.clone();
    ProductionSharedPorts {
        app_server_adapter,
        planning_authority_port,
        planning_task_repository_port,
        planning_workspace_port,
        planning_worker_port,
        parallel_agent_worker_port,
        app_server_prompt_log_port,
    }
}

fn planning_services_from_ports(ports: &ProductionSharedPorts) -> PlanningServices {
    PlanningServices::from_ports(
        ports.planning_workspace_port.clone(),
        ports.planning_authority_port.clone(),
        ports.planning_task_repository_port.clone(),
        ports.planning_worker_port.clone(),
    )
}

fn parallel_mode_control_plane_from_parts(
    planning: PlanningServices,
    planning_authority_port: Arc<dyn PlanningAuthorityPort>,
    parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort>,
) -> ParallelModeControlPlaneComposition {
    let parallel_mode_service = ParallelModeService::new(
        planning_authority_port,
        github_automation_port(),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    ParallelModeControlPlaneComposition::new(
        parallel_mode_service,
        planning,
        parallel_agent_worker_port,
    )
}

fn app_server_adapter(
    prompt_log_port: Arc<dyn AppServerPromptLogPort>,
) -> Arc<CodexAppServerAdapter> {
    Arc::new(CodexAppServerAdapter::from_environment_with_prompt_log(
        APP_SERVER_CLIENT_NAME,
        env!("CARGO_PKG_VERSION"),
        prompt_log_port,
    ))
}

fn github_automation_port() -> Arc<dyn GithubAutomationPort> {
    Arc::new(GithubAutomationAdapter::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogRequest;
    use crate::application::service::planning::{PlanningControlCommand, PlanningControlRequest};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempWorkspace {
        path: PathBuf,
    }

    impl TempWorkspace {
        fn new(prefix: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "akra-production-composition-{prefix}-{}-{unique_suffix}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("temp workspace should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    // R10 behavior regression: production composition must execute use cases through
    // application services, not by letting inbound adapters own concrete wiring.
    #[test]
    fn production_composition_control_service_executes_status_use_case() {
        let workspace = TempWorkspace::new("status-use-case");
        let workspace_dir = workspace.path().display().to_string();
        let control_service = build_planning_control_service(workspace_dir.clone());

        let response = control_service
            .execute_request(PlanningControlRequest::new(PlanningControlCommand::Status))
            .expect("production planning control service should execute status");

        assert_eq!(response.workspace_dir, workspace_dir);
        assert!(response.reply.text.contains("상태 요약"));
        assert!(response.reply.text.contains("planning_state:"));
    }

    // R10 behavior regression for the R9 composition move: every inbound surface must be
    // constructible from the shared production composition path.
    #[test]
    fn production_composition_builds_shared_inbound_application_surfaces() {
        let workspace = TempWorkspace::new("inbound-surfaces");
        let workspace_dir = workspace.path().display().to_string();

        let admin = build_admin_application(workspace_dir.clone());
        assert_eq!(admin.facade.workspace_dir(), workspace_dir);
        let admin_projection = admin
            .facade
            .load_runtime_application_projection()
            .expect("admin facade should load the shared planning projection");
        assert!(!admin_projection.status_label.trim().is_empty());

        let telegram = build_telegram_application(workspace_dir.clone());
        let help = telegram
            .control_service
            .execute_request(PlanningControlRequest::new(PlanningControlCommand::Help))
            .expect("telegram control service should share the planning control surface");
        assert!(help.reply.text.contains("/status"));
        let telegram_snapshot = telegram
            .parallel_mode_control_plane
            .inspect_dashboard_snapshot(
                &workspace_dir,
                ParallelModeRuntimeEventLogRequest::recent(1),
            );
        assert!(telegram_snapshot.events.visible_count() <= 1);

        let tui = build_native_tui_application_services();
        assert!(
            !tui.parallel_mode_control_plane
                .planning()
                .runtime
                .load_runtime_projection_or_invalid(&workspace_dir)
                .preview_status_label()
                .is_empty()
        );
    }
}

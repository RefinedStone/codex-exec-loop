use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;

use crate::adapter::outbound::app_server::{AppServerPlanningWorkerAdapter, CodexAppServerAdapter};
use crate::adapter::outbound::db::{
    SqlitePlanningAuthorityAdapter, SqliteTelegramGlobalRunnerLeaseAdapter,
};
use crate::adapter::outbound::filesystem::{
    FilesystemParallelAgentProfileRepositoryAdapter, FilesystemPlanningWorkspaceAdapter,
};
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::github::{
    GithubAutomationAdapter, GithubPrValidationAdapter, GithubReviewPollerAdapter,
};
use crate::adapter::outbound::telegram::CurlTelegramBotAdapter;
use crate::application::port::inbound::admin_debug_port::{
    AdminDebugHarnessConfig, AdminDebugPort,
};
use crate::application::port::inbound::app_server_prompt_log_query_port::AppServerPromptLogQueryPort;
use crate::application::port::inbound::parallel_agent_profile_port::ParallelAgentProfilePort;
use crate::application::port::inbound::parallel_mode_admin_port::ParallelModeAdminPort;
use crate::application::port::inbound::parallel_mode_control_port::ParallelModeControlPort;
use crate::application::port::inbound::planning_admin_port::PlanningAdminPort;
use crate::application::port::inbound::planning_control_port::PlanningControlPort;
use crate::application::port::inbound::planning_task_tool_port::PlanningTaskToolPort;
use crate::application::port::inbound::planning_workspace_maintenance_port::PlanningWorkspaceMaintenancePort;
use crate::application::port::inbound::pr_validation_query_port::PrValidationQueryPort;
use crate::application::port::inbound::review_center_query_port::ReviewCenterQueryPort;
use crate::application::port::outbound::app_server_prompt_log_port::{
    AppServerPromptLogMaintenanceMode, AppServerPromptLogMaintenancePort, AppServerPromptLogPort,
    NoopAppServerPromptLogPort,
};
use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::github_pr_validation_port::GithubPrValidationPort;
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::application::port::outbound::parallel_agent_profile_repository_port::ParallelAgentProfileRepositoryPort;
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_worker_port::PlanningWorkerPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::port::outbound::review_center_repository_port::ReviewCenterRepositoryPort;
use crate::application::port::outbound::telegram_bot_port::TelegramBotPort;
use crate::application::port::outbound::telegram_global_runner_lease_port::TelegramGlobalRunnerLeasePort;
use crate::application::port::outbound::telegram_update_ledger_port::TelegramUpdateLedgerPort;
use crate::application::service::admin_debug_harness::AdminDebugHarnessService;
use crate::application::service::app_server_prompt_log_query::AppServerPromptLogQueryService;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::application::service::parallel_agent_profile::ParallelAgentProfileService;
use crate::application::service::parallel_mode::admin::ParallelModeAdminService;
use crate::application::service::parallel_mode::{
    ParallelModeService, control_plane::ParallelModeControlPlaneComposition,
};
use crate::application::service::planning::{
    PlanningAdminFacadeService, PlanningControlFacadeService, PlanningControlService,
    PlanningServices,
};
use crate::application::service::pr_validation_query::PrValidationQueryService;
use crate::application::service::review_center::ReviewCenterReadService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::composition::native_client_runtime::NativeTuiApplicationComposition;
use crate::domain::github_review::GithubPullRequestTarget;

const APP_SERVER_CLIENT_NAME: &str = "codex-exec-loop-native";
const AKRA_APP_SERVER_PROMPT_LOG_ENV_VAR: &str = "AKRA_APP_SERVER_PROMPT_LOG";

pub(crate) struct ProductionAdminApplication {
    pub(crate) facade: Arc<dyn PlanningAdminPort>,
    pub(crate) parallel_mode_admin_port: Arc<dyn ParallelModeAdminPort>,
    pub(crate) admin_debug_port: Arc<dyn AdminDebugPort>,
    pub(crate) app_server_prompt_log_query_port: Arc<dyn AppServerPromptLogQueryPort>,
    pub(crate) parallel_agent_profile_port: Arc<dyn ParallelAgentProfilePort>,
    #[allow(dead_code)]
    pub(crate) review_center_query_port: Arc<dyn ReviewCenterQueryPort>,
}

pub(crate) struct ProductionTelegramApplication {
    pub(crate) planning_control_port: Arc<dyn PlanningControlPort>,
    pub(crate) parallel_mode_control_port: Arc<dyn ParallelModeControlPort>,
    pub(crate) telegram_update_ledger_port: Arc<dyn TelegramUpdateLedgerPort>,
    pub(crate) telegram_global_runner_lease_port: Arc<dyn TelegramGlobalRunnerLeasePort>,
    #[allow(dead_code)]
    pub(crate) review_center_query_port: Arc<dyn ReviewCenterQueryPort>,
}

struct ProductionSharedPorts {
    app_server_adapter: Arc<CodexAppServerAdapter>,
    planning_authority_port: Arc<dyn PlanningAuthorityPort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    review_center_repository_port: Arc<dyn ReviewCenterRepositoryPort>,
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_worker_port: Arc<dyn PlanningWorkerPort>,
    parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort>,
    parallel_agent_profile_repository_port: Arc<dyn ParallelAgentProfileRepositoryPort>,
    app_server_prompt_log_maintenance_port: Arc<dyn AppServerPromptLogMaintenancePort>,
    app_server_prompt_log_port: Arc<dyn AppServerPromptLogPort>,
    telegram_update_ledger_port: Arc<dyn TelegramUpdateLedgerPort>,
    telegram_global_runner_lease_port: Arc<dyn TelegramGlobalRunnerLeasePort>,
}

#[cfg(all(test, windows))]
pub(crate) fn build_planning_services() -> PlanningServices {
    let ports = build_shared_ports();
    planning_services_from_ports(&ports)
}

pub(crate) fn build_planning_services_for_workspace(workspace_dir: &str) -> PlanningServices {
    let capture_enabled = app_server_prompt_logging_enabled();
    maintain_prompt_logs_best_effort(workspace_dir, capture_enabled);
    let ports = build_shared_ports_for_prompt_logging(capture_enabled);
    planning_services_from_ports(&ports)
}

pub(crate) fn build_planning_control_service(workspace_dir: String) -> PlanningControlService {
    let planning = build_planning_services_for_workspace(&workspace_dir);
    PlanningControlService::new(Arc::new(PlanningControlFacadeService::new(
        workspace_dir,
        planning,
    )))
}

pub(crate) fn build_planning_control_port(workspace_dir: String) -> Arc<dyn PlanningControlPort> {
    Arc::new(build_planning_control_service(workspace_dir))
}

pub(crate) fn build_pr_validation_query_port(
    workspace_dir: &str,
) -> Arc<dyn PrValidationQueryPort> {
    Arc::new(PrValidationQueryService::new(
        workspace_dir,
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
    ))
}

pub(crate) fn build_planning_task_tool_port(workspace_dir: &str) -> Arc<dyn PlanningTaskToolPort> {
    Arc::new(
        build_planning_services_for_workspace(workspace_dir)
            .task_tool
            .clone(),
    )
}

pub(crate) fn build_planning_workspace_maintenance_port(
    workspace_dir: &str,
) -> Arc<dyn PlanningWorkspaceMaintenancePort> {
    Arc::new(
        build_planning_services_for_workspace(workspace_dir)
            .workspace
            .clone(),
    )
}

pub(crate) fn build_parallel_mode_control_plane_composition(
    workspace_dir: &str,
) -> ParallelModeControlPlaneComposition {
    let capture_enabled = app_server_prompt_logging_enabled();
    maintain_prompt_logs_best_effort(workspace_dir, capture_enabled);
    let ports = build_shared_ports_for_prompt_logging(capture_enabled);
    let planning = planning_services_from_ports(&ports);
    let parallel_agent_profile_service = parallel_agent_profile_service_from_ports(&ports);
    parallel_mode_control_plane_from_parts(
        planning,
        ports.planning_authority_port,
        ports.parallel_agent_worker_port,
        parallel_agent_profile_service,
        Arc::new(GithubPrValidationAdapter::for_local_github_credentials(
            workspace_dir,
        )),
    )
}

pub(crate) fn build_parallel_mode_control_port(
    workspace_dir: &str,
) -> Arc<dyn ParallelModeControlPort> {
    Arc::new(build_parallel_mode_control_plane_composition(workspace_dir))
}

#[cfg(test)]
pub(crate) fn build_admin_application(workspace_dir: String) -> ProductionAdminApplication {
    build_admin_application_with_debug_harness(workspace_dir, false)
}

pub(crate) fn build_admin_application_with_debug_harness(
    workspace_dir: String,
    debug_harness_enabled: bool,
) -> ProductionAdminApplication {
    let capture_enabled = app_server_prompt_logging_enabled();
    maintain_prompt_logs_best_effort(&workspace_dir, capture_enabled);
    let ports = build_shared_ports_for_prompt_logging(capture_enabled);
    let planning = planning_services_from_ports(&ports);
    let review_center_read_service = ReviewCenterReadService::new(
        workspace_dir.clone(),
        ports.review_center_repository_port.clone(),
    );
    let app_server_prompt_log_query_port: Arc<dyn AppServerPromptLogQueryPort> = Arc::new(
        AppServerPromptLogQueryService::new(ports.app_server_prompt_log_port.clone()),
    );
    let parallel_agent_profile_service = parallel_agent_profile_service_from_ports(&ports);
    let parallel_agent_profile_port: Arc<dyn ParallelAgentProfilePort> =
        Arc::new(parallel_agent_profile_service.clone());
    let parallel_mode_control_plane = Arc::new(parallel_mode_control_plane_from_parts(
        planning.clone(),
        ports.planning_authority_port.clone(),
        ports.parallel_agent_worker_port.clone(),
        parallel_agent_profile_service.clone(),
        Arc::new(GithubPrValidationAdapter::for_local_github_credentials(
            &workspace_dir,
        )),
    ));
    let parallel_mode_admin_port: Arc<dyn ParallelModeAdminPort> =
        Arc::new(ParallelModeAdminService::new(parallel_mode_control_plane));
    let facade: Arc<dyn PlanningAdminPort> =
        Arc::new(PlanningAdminFacadeService::from_planning_with_authority(
            workspace_dir,
            planning,
            ports.planning_workspace_port,
            ports.planning_authority_port,
            ports.planning_task_repository_port,
        ));
    ProductionAdminApplication {
        facade,
        parallel_mode_admin_port,
        admin_debug_port: Arc::new(AdminDebugHarnessService::new(if debug_harness_enabled {
            AdminDebugHarnessConfig::enabled()
        } else {
            AdminDebugHarnessConfig::disabled()
        })),
        app_server_prompt_log_query_port,
        parallel_agent_profile_port,
        review_center_query_port: Arc::new(review_center_read_service),
    }
}

pub(crate) fn build_telegram_application(workspace_dir: String) -> ProductionTelegramApplication {
    let capture_enabled = app_server_prompt_logging_enabled();
    maintain_prompt_logs_best_effort(&workspace_dir, capture_enabled);
    let ports = build_shared_ports_for_prompt_logging(capture_enabled);
    let planning = planning_services_from_ports(&ports);
    let parallel_agent_profile_service = parallel_agent_profile_service_from_ports(&ports);
    let review_center_read_service = ReviewCenterReadService::new(
        workspace_dir.clone(),
        ports.review_center_repository_port.clone(),
    );
    let control_service = PlanningControlService::new(Arc::new(PlanningControlFacadeService::new(
        workspace_dir.clone(),
        planning.clone(),
    )));
    let parallel_mode_control_port = Arc::new(parallel_mode_control_plane_from_parts(
        planning,
        ports.planning_authority_port,
        ports.parallel_agent_worker_port,
        parallel_agent_profile_service,
        Arc::new(GithubPrValidationAdapter::for_local_github_credentials(
            &workspace_dir,
        )),
    ));
    ProductionTelegramApplication {
        planning_control_port: Arc::new(control_service),
        parallel_mode_control_port,
        telegram_update_ledger_port: ports.telegram_update_ledger_port,
        telegram_global_runner_lease_port: ports.telegram_global_runner_lease_port,
        review_center_query_port: Arc::new(review_center_read_service),
    }
}

pub(crate) fn build_telegram_bot_port(token: String) -> Arc<dyn TelegramBotPort> {
    Arc::new(CurlTelegramBotAdapter::new(token))
}

pub(crate) fn resolve_active_planning_workspace_root(workspace_dir: &str) -> PathBuf {
    SqlitePlanningAuthorityAdapter::resolve_active_workspace_root(workspace_dir)
}

pub(crate) fn build_native_tui_application() -> NativeTuiApplicationComposition {
    let workspace_dir = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let capture_enabled = app_server_prompt_logging_enabled();
    let ports = build_shared_ports_for_prompt_logging(capture_enabled);
    let maintenance_mode = native_tui_prompt_log_maintenance_mode(capture_enabled);
    let startup_service = StartupService::new(ports.app_server_adapter.clone())
        .with_prompt_log_maintenance(
            ports.app_server_prompt_log_maintenance_port.clone(),
            maintenance_mode,
        );
    let session_service = SessionService::new(ports.app_server_adapter.clone());
    let review_center_read_service = ReviewCenterReadService::new(
        workspace_dir.clone(),
        ports.review_center_repository_port.clone(),
    );
    let conversation_service = ConversationService::new(ports.app_server_adapter.clone())
        .with_review_center_read_service(review_center_read_service.clone());
    let planning = planning_services_from_ports(&ports);
    let parallel_agent_profile_service = parallel_agent_profile_service_from_ports(&ports);
    let parallel_mode_control_plane = parallel_mode_control_plane_from_parts(
        planning,
        ports.planning_authority_port,
        ports.parallel_agent_worker_port,
        parallel_agent_profile_service,
        Arc::new(GithubPrValidationAdapter::for_local_github_credentials(
            workspace_dir,
        )),
    );
    NativeTuiApplicationComposition::from_services(
        startup_service,
        session_service,
        conversation_service,
        parallel_mode_control_plane,
    )
}

fn native_tui_prompt_log_maintenance_mode(
    capture_enabled: bool,
) -> AppServerPromptLogMaintenanceMode {
    if capture_enabled {
        AppServerPromptLogMaintenanceMode::PurgeExpired
    } else {
        AppServerPromptLogMaintenanceMode::ClearAll
    }
}

fn maintain_prompt_logs_best_effort(workspace_dir: &str, capture_enabled: bool) {
    let result = if capture_enabled {
        SqlitePlanningAuthorityAdapter::purge_expired_app_server_prompt_interaction_records(
            workspace_dir,
        )
    } else {
        SqlitePlanningAuthorityAdapter::clear_app_server_prompt_interaction_records(workspace_dir)
    };
    if let Err(error) = result {
        tracing::warn!(
            error_chars = error.to_string().chars().count(),
            capture_enabled,
            "app-server prompt-log privacy cleanup failed"
        );
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

fn build_prompt_log_port_for_setting(
    planning_authority_adapter: Arc<SqlitePlanningAuthorityAdapter>,
    enabled: bool,
) -> Arc<dyn AppServerPromptLogPort> {
    if enabled {
        planning_authority_adapter
    } else {
        Arc::new(NoopAppServerPromptLogPort)
    }
}

fn app_server_prompt_logging_enabled() -> bool {
    app_server_prompt_logging_enabled_from_value(
        std::env::var(AKRA_APP_SERVER_PROMPT_LOG_ENV_VAR)
            .ok()
            .as_deref(),
    )
}

fn app_server_prompt_logging_enabled_from_value(value: Option<&str>) -> bool {
    value.and_then(parse_bool_env_flag).unwrap_or(false)
}

fn parse_bool_env_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(all(test, windows))]
fn build_shared_ports() -> ProductionSharedPorts {
    build_shared_ports_for_prompt_logging(app_server_prompt_logging_enabled())
}

fn build_shared_ports_for_prompt_logging(prompt_logging_enabled: bool) -> ProductionSharedPorts {
    let planning_authority_adapter = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning_authority_port: Arc<dyn PlanningAuthorityPort> =
        planning_authority_adapter.clone();
    let app_server_prompt_log_port = build_prompt_log_port_for_setting(
        planning_authority_adapter.clone(),
        prompt_logging_enabled,
    );
    let app_server_prompt_log_maintenance_port: Arc<dyn AppServerPromptLogMaintenancePort> =
        planning_authority_adapter.clone();
    let app_server_adapter = app_server_adapter(app_server_prompt_log_port.clone());
    let planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort> =
        planning_authority_adapter.clone();
    let review_center_repository_port: Arc<dyn ReviewCenterRepositoryPort> =
        planning_authority_adapter.clone();
    let telegram_global_adapter = Arc::new(SqliteTelegramGlobalRunnerLeaseAdapter::new());
    let telegram_update_ledger_port: Arc<dyn TelegramUpdateLedgerPort> =
        telegram_global_adapter.clone();
    let telegram_global_runner_lease_port: Arc<dyn TelegramGlobalRunnerLeasePort> =
        telegram_global_adapter;
    let planning_workspace_port: Arc<dyn PlanningWorkspacePort> =
        Arc::new(FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(
            planning_authority_adapter.clone(),
        ));
    let planning_worker_port: Arc<dyn PlanningWorkerPort> = Arc::new(
        AppServerPlanningWorkerAdapter::new(app_server_adapter.clone()),
    );
    let parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort> = app_server_adapter.clone();
    let parallel_agent_profile_repository_port: Arc<dyn ParallelAgentProfileRepositoryPort> =
        Arc::new(FilesystemParallelAgentProfileRepositoryAdapter::new());
    ProductionSharedPorts {
        app_server_adapter,
        planning_authority_port,
        planning_task_repository_port,
        review_center_repository_port,
        planning_workspace_port,
        planning_worker_port,
        parallel_agent_worker_port,
        parallel_agent_profile_repository_port,
        app_server_prompt_log_maintenance_port,
        app_server_prompt_log_port,
        telegram_update_ledger_port,
        telegram_global_runner_lease_port,
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

fn parallel_agent_profile_service_from_ports(
    ports: &ProductionSharedPorts,
) -> ParallelAgentProfileService {
    ParallelAgentProfileService::new(ports.parallel_agent_profile_repository_port.clone())
}

fn parallel_mode_control_plane_from_parts(
    planning: PlanningServices,
    planning_authority_port: Arc<dyn PlanningAuthorityPort>,
    parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort>,
    parallel_agent_profile_service: ParallelAgentProfileService,
    pr_validation_observation: Arc<dyn GithubPrValidationPort>,
) -> ParallelModeControlPlaneComposition {
    let parallel_mode_service = ParallelModeService::new(
        planning_authority_port,
        github_automation_port(),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    )
    .with_pr_validation_observation(pr_validation_observation)
    .with_parallel_agent_profile_service(parallel_agent_profile_service);
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
    use crate::application::port::outbound::app_server_prompt_log_port::{
        AppServerPromptInputRecord, AppServerPromptInteractionRecord,
    };
    use crate::application::port::outbound::github_pr_validation_port::{
        GithubPrMergeState, GithubPrValidationObservationRequest, GithubPrValidationPort,
        GithubPrValidationSnapshot, GithubValidationCheckRun, GithubValidationRunStatus,
        GithubValidationSource, GithubValidationSourceObservation, GithubValidationSourceStatus,
    };
    use crate::application::service::planning::{PlanningControlCommand, PlanningControlRequest};
    use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId, GithubPullRequestTarget};
    use crate::domain::parallel_mode::{
        PrValidationCommitSha, PrValidationRecord, PrValidationRecordKey, PrValidationTarget,
        PrValidationTargetShaSnapshot,
    };
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Mutex;
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
            let pool_parent = self.path.with_file_name(format!(
                "{}-akra-worktrees",
                self.path
                    .file_name()
                    .expect("temp workspace should have a name")
                    .to_string_lossy()
            ));
            let _ = std::fs::remove_dir_all(pool_parent);
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    struct RecordingValidationAdapter {
        snapshots: Mutex<Vec<GithubPrValidationSnapshot>>,
        requests: Mutex<Vec<GithubPrValidationObservationRequest>>,
    }

    impl GithubPrValidationPort for RecordingValidationAdapter {
        fn load_validation_snapshot(
            &self,
            request: &GithubPrValidationObservationRequest,
        ) -> Result<GithubPrValidationSnapshot> {
            self.requests.lock().unwrap().push(request.clone());
            Ok(self.snapshots.lock().unwrap().remove(0))
        }
    }

    fn validation_snapshot(status: GithubValidationRunStatus) -> GithubPrValidationSnapshot {
        const HEAD: &str = "1111111111111111111111111111111111111111";
        GithubPrValidationSnapshot {
            target: GithubPullRequestTarget::new("acme/widgets", 42),
            target_sha: GithubCommitSha::new(HEAD),
            evidence_sha: GithubCommitSha::new(HEAD),
            merge_state: GithubPrMergeState::Open,
            merge_sha: None,
            activities: Vec::new(),
            check_runs: vec![
                GithubValidationCheckRun::new(
                    GithubOpaqueId::new("check:ci"),
                    "Post-Merge Gate",
                    GithubCommitSha::new(HEAD),
                    status,
                )
                .with_attempt_metadata(
                    Some("github-actions".to_string()),
                    Some(GithubOpaqueId::new("check-suite:ci")),
                    Some("2026-08-08T00:00:00Z".to_string()),
                    Some("2026-08-08T00:01:00Z".to_string()),
                ),
            ],
            workflow_runs: Vec::new(),
            sources: GithubValidationSource::ALL
                .into_iter()
                .map(|source| {
                    GithubValidationSourceObservation::new(
                        source,
                        format!("rest:{source:?}"),
                        GithubValidationSourceStatus::Complete,
                        None,
                    )
                })
                .collect(),
            next_cursor: None,
        }
    }

    fn initialize_git_workspace(workspace: &TempWorkspace) {
        for args in [
            vec!["init", "-b", "prerelease"],
            vec!["config", "user.name", "Akra Test"],
            vec!["config", "user.email", "akra@example.invalid"],
            vec!["commit", "--allow-empty", "-m", "initial"],
        ] {
            let output = Command::new("git")
                .args(args)
                .current_dir(workspace.path())
                .output()
                .expect("git fixture command should start");
            assert!(
                output.status.success(),
                "git fixture command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn contains_lease_directory(path: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(path) else {
            return false;
        };
        entries.filter_map(Result::ok).any(|entry| {
            entry.file_name() == ".leases"
                || (entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && contains_lease_directory(&entry.path()))
        })
    }

    fn sample_prompt_record(workspace_dir: &str) -> AppServerPromptInteractionRecord {
        let now = chrono::Utc::now().to_rfc3339();
        AppServerPromptInteractionRecord {
            sequence: 0,
            interaction_id: "interaction-1".to_string(),
            session_kind: "main".to_string(),
            operation: "turn".to_string(),
            status: "completed".to_string(),
            workspace_dir: workspace_dir.to_string(),
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            service_name: Some("main session".to_string()),
            model: Some("gpt-test".to_string()),
            reasoning_effort: Some("medium".to_string()),
            developer_instructions: None,
            input_items: vec![AppServerPromptInputRecord::new(
                "text",
                "prompt",
                "sensitive prompt",
            )],
            output_items: Vec::new(),
            error_message: None,
            started_at: now.clone(),
            completed_at: now,
        }
    }

    #[test]
    fn production_composition_runtime_tick_activates_durable_pr_validation_without_a_lease() {
        const HEAD: &str = "1111111111111111111111111111111111111111";
        const BASE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let workspace = TempWorkspace::new("pr-validation-runtime-tick");
        initialize_git_workspace(&workspace);
        let workspace_dir = workspace.path().display().to_string();
        let ports = build_shared_ports_for_prompt_logging(false);
        let planning = planning_services_from_ports(&ports);
        planning
            .workspace
            .initialize_simple_workspace(&workspace_dir)
            .expect("production planning authority should initialize");
        let record = PrValidationRecord::register(
            PrValidationRecordKey::new("akra-unit-42").unwrap(),
            PrValidationTarget::new("acme/widgets", 42).unwrap(),
            PrValidationTargetShaSnapshot::new(
                PrValidationCommitSha::new(HEAD).unwrap(),
                PrValidationCommitSha::new(BASE).unwrap(),
            ),
        );
        assert!(
            ports
                .planning_authority_port
                .compare_and_swap_runtime_pr_validation_record(
                    &workspace_dir,
                    record.key(),
                    None,
                    Some(&record),
                )
                .expect("validation authority record should register")
        );
        let observation = Arc::new(RecordingValidationAdapter {
            snapshots: Mutex::new(vec![
                validation_snapshot(GithubValidationRunStatus::InProgress),
                validation_snapshot(GithubValidationRunStatus::Failed),
            ]),
            requests: Mutex::new(Vec::new()),
        });
        let composition = parallel_mode_control_plane_from_parts(
            planning.clone(),
            ports.planning_authority_port.clone(),
            ports.parallel_agent_worker_port.clone(),
            parallel_agent_profile_service_from_ports(&ports),
            observation.clone(),
        );

        composition
            .run_manual_orchestrator_tick(&workspace_dir)
            .expect("first production-composed runtime tick should complete");
        composition
            .run_manual_orchestrator_tick(&workspace_dir)
            .expect("second production-composed runtime tick should complete");

        let persisted = ports
            .planning_authority_port
            .load_runtime_pr_validation_record(&workspace_dir, record.key())
            .unwrap()
            .unwrap();
        assert_eq!(persisted.observation_revision(), 2);
        assert_eq!(observation.requests.lock().unwrap().len(), 2);
        let queue = planning
            .queue
            .load_authority_snapshot(&workspace_dir)
            .expect("normal planning queue should contain remediation");
        assert_eq!(queue.tasks.len(), 1);
        let pool_parent = workspace.path().with_file_name(format!(
            "{}-akra-worktrees",
            workspace.path().file_name().unwrap().to_string_lossy()
        ));
        assert!(
            !contains_lease_directory(&pool_parent),
            "runtime validation polling must not create or hold a slot lease"
        );
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
            .load_runtime_summary()
            .expect("admin facade should load the shared planning projection");
        assert!(!admin_projection.preview_status_label.trim().is_empty());

        let telegram = build_telegram_application(workspace_dir.clone());
        let help = telegram
            .planning_control_port
            .execute_request(PlanningControlRequest::new(PlanningControlCommand::Help))
            .expect("telegram control service should share the planning control surface");
        assert!(help.reply.text.contains("/status"));
        let telegram_snapshot = telegram
            .parallel_mode_control_port
            .load_status(&workspace_dir, 1)
            .expect("telegram parallel control port should inspect status");
        assert!(telegram_snapshot.visible_event_count <= 1);

        let _tui = build_native_tui_application();
    }

    #[cfg(windows)]
    #[test]
    fn windows_plain_workspace_production_flow_initializes_edits_and_resets_private_authority() {
        use crate::application::service::planning::PlanningResetTarget;

        let workspace = TempWorkspace::new("windows-plain-planning-flow");
        assert!(
            !workspace.path().join(".git").exists(),
            "fixture must remain a plain non-Git directory"
        );
        let workspace_dir = workspace.path().display().to_string();
        let planning = build_planning_services();

        assert!(
            !planning
                .workspace
                .has_planning_workspace(&workspace_dir)
                .expect("empty Windows authority should inspect")
        );
        planning
            .workspace
            .initialize_simple_workspace(&workspace_dir)
            .expect("plain Windows planning should initialize through private SQLite");
        assert!(
            planning
                .workspace
                .has_planning_workspace(&workspace_dir)
                .expect("initialized Windows authority should inspect")
        );
        assert!(
            planning
                .workspace
                .has_planning_candidate_workspace(&workspace_dir)
                .expect("Windows candidate read should use the private authority")
        );

        let staged = planning
            .workspace
            .stage_simple_mode_draft(&workspace_dir)
            .expect("plain Windows draft should stage through private SQLite");
        let mut editor = planning
            .workspace
            .load_manual_editor_session(&workspace_dir, &staged.draft_name)
            .expect("plain Windows draft should load through private SQLite");
        assert_eq!(editor.editable_files.len(), 1);
        editor.editable_files[0]
            .body
            .push_str("\nWindows private-authority edit.\n");
        planning
            .workspace
            .save_draft_editor_files(&workspace_dir, &editor.draft_name, &editor.editable_files)
            .expect("plain Windows draft edit should persist through private SQLite");
        let reloaded = planning
            .workspace
            .load_manual_editor_session(&workspace_dir, &editor.draft_name)
            .expect("edited Windows draft should reload");
        assert!(
            reloaded.editable_files[0]
                .body
                .contains("Windows private-authority edit")
        );

        planning
            .workspace
            .reset_workspace(&workspace_dir, PlanningResetTarget::All)
            .expect("plain Windows full reset should commit through one authority transaction");
        let stale_draft_error = planning
            .workspace
            .load_manual_editor_session(&workspace_dir, &editor.draft_name)
            .expect_err("full reset must remove staged Windows draft rows");
        assert!(stale_draft_error.to_string().contains("does not exist"));
        assert!(
            planning
                .workspace
                .has_planning_workspace(&workspace_dir)
                .expect("reset Windows authority should remain initialized")
        );
        assert!(
            !workspace.path().join(".codex-exec-loop").exists(),
            "private-authority flow must not fall back to full-path planning writes"
        );
    }

    #[test]
    fn production_composition_disables_prompt_logging_by_default() {
        let workspace = TempWorkspace::new("prompt-log-default");
        let workspace_dir = workspace.path().display().to_string();
        let prompt_log = build_prompt_log_port_for_setting(
            Arc::new(SqlitePlanningAuthorityAdapter::new()),
            app_server_prompt_logging_enabled_from_value(None),
        );

        prompt_log
            .append_app_server_prompt_interaction(
                &workspace_dir,
                sample_prompt_record(&workspace_dir),
            )
            .expect("noop prompt log should accept writes");
        let snapshot = prompt_log
            .load_recent_app_server_prompt_interactions(&workspace_dir, 10)
            .expect("noop prompt log should load empty snapshots");

        assert!(snapshot.records.is_empty());
    }

    #[test]
    fn production_composition_enables_prompt_logging_when_opted_in() {
        let workspace = TempWorkspace::new("prompt-log-enabled");
        let workspace_dir = workspace.path().display().to_string();
        let prompt_log = build_prompt_log_port_for_setting(
            Arc::new(SqlitePlanningAuthorityAdapter::new()),
            app_server_prompt_logging_enabled_from_value(Some("1")),
        );

        prompt_log
            .append_app_server_prompt_interaction(
                &workspace_dir,
                sample_prompt_record(&workspace_dir),
            )
            .expect("sqlite prompt log should persist writes");
        let snapshot = prompt_log
            .load_recent_app_server_prompt_interactions(&workspace_dir, 10)
            .expect("sqlite prompt log should load stored records");

        assert_eq!(snapshot.records.len(), 1);
        assert_eq!(
            snapshot.records[0].input_items[0].content,
            "sensitive prompt"
        );
    }

    #[test]
    fn production_compositions_clear_all_prompt_logs_when_capture_is_disabled() {
        let workspace = TempWorkspace::new("prompt-log-disabled-startup-purge");
        let workspace_dir = workspace.path().display().to_string();
        SqlitePlanningAuthorityAdapter::append_app_server_prompt_interaction_record(
            &workspace_dir,
            sample_prompt_record(&workspace_dir),
        )
        .expect("prompt log schema should initialize");
        let location = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
            &workspace_dir,
        )
        .expect("authority location should resolve");
        let connection = rusqlite::Connection::open(&location.authority_store_path)
            .expect("authority store should open for expired fixture insertion");
        connection
            .execute(
                "INSERT INTO app_server_prompt_interactions
                 (interaction_id, session_kind, operation, status, started_at, completed_at, content_json)
                 VALUES (?1, 'main', 'turn', 'completed', ?2, ?2, '{}')",
                rusqlite::params!["expired-startup-record", "2000-01-01T00:00:00Z"],
            )
            .expect("expired fixture should insert");
        drop(connection);

        let _application = build_admin_application(workspace_dir.clone());

        let connection = rusqlite::Connection::open(&location.authority_store_path)
            .expect("authority store should reopen after admin composition");
        let expired_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM app_server_prompt_interactions WHERE interaction_id = ?1",
                ["expired-startup-record"],
                |row| row.get(0),
            )
            .expect("expired record count should load");
        assert_eq!(expired_count, 0);
        let retained_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM app_server_prompt_interactions",
                [],
                |row| row.get(0),
            )
            .expect("disabled prompt record count should load");
        assert_eq!(retained_count, 0);

        connection
            .execute(
                "INSERT INTO app_server_prompt_interactions
                 (interaction_id, session_kind, operation, status, started_at, completed_at, content_json)
                 VALUES (?1, 'main', 'turn', 'completed', ?2, ?2, '{}')",
                rusqlite::params!["expired-parallel-tick-record", "2000-01-01T00:00:00Z"],
            )
            .expect("parallel-tick expired fixture should insert");
        drop(connection);

        let _control_plane = build_parallel_mode_control_plane_composition(&workspace_dir);

        let connection = rusqlite::Connection::open(&location.authority_store_path)
            .expect("authority store should reopen after parallel-tick composition");
        let expired_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM app_server_prompt_interactions WHERE interaction_id = ?1",
                ["expired-parallel-tick-record"],
                |row| row.get(0),
            )
            .expect("parallel-tick expired record count should load");
        assert_eq!(expired_count, 0);
        drop(connection);

        SqlitePlanningAuthorityAdapter::append_app_server_prompt_interaction_record(
            &workspace_dir,
            sample_prompt_record(&workspace_dir),
        )
        .expect("CLI planning prompt fixture should append");
        let _planning = build_planning_services_for_workspace(&workspace_dir);
        let connection = rusqlite::Connection::open(&location.authority_store_path)
            .expect("authority store should reopen after workspace planning composition");
        let retained_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM app_server_prompt_interactions",
                [],
                |row| row.get(0),
            )
            .expect("workspace planning prompt count should load");
        assert_eq!(retained_count, 0);
    }

    #[test]
    fn enabled_prompt_log_maintenance_preserves_unexpired_records() {
        let workspace = TempWorkspace::new("prompt-log-enabled-startup-retention");
        let workspace_dir = workspace.path().display().to_string();
        SqlitePlanningAuthorityAdapter::append_app_server_prompt_interaction_record(
            &workspace_dir,
            sample_prompt_record(&workspace_dir),
        )
        .expect("prompt log fixture should append");

        maintain_prompt_logs_best_effort(&workspace_dir, true);

        let snapshot =
            SqlitePlanningAuthorityAdapter::load_recent_app_server_prompt_interaction_records(
                &workspace_dir,
                10,
            )
            .expect("enabled prompt records should remain readable");
        assert_eq!(snapshot.records.len(), 1);
    }

    #[test]
    fn native_tui_prompt_log_maintenance_mode_preserves_capture_policy() {
        assert_eq!(
            native_tui_prompt_log_maintenance_mode(true),
            AppServerPromptLogMaintenanceMode::PurgeExpired
        );
        assert_eq!(
            native_tui_prompt_log_maintenance_mode(false),
            AppServerPromptLogMaintenanceMode::ClearAll
        );
    }
}

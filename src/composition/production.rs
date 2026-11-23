use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::composition::clipboard_image_probe::CompositionClipboardImageProbeTrigger;
use crate::composition::kernel_command_execution;
use crate::composition::repository_kernel::RepositoryKernelRegistry;

use anyhow::{Context, Result};

use crate::adapter::inbound::gateway::GatewayMcpHub;
use crate::adapter::outbound::app_server::cloud_chats::CloudChatsRestAdapter;
use crate::adapter::outbound::app_server::cloud_dots::CloudDotsRestAdapter;
use crate::adapter::outbound::app_server::cloud_upstream::{CloudGatewayUpstream, codex_auth_path};
use crate::adapter::outbound::app_server::gateway_upstream::AppServerGatewayUpstream;
use crate::adapter::outbound::app_server::thread_transcript::AppServerThreadTranscript;
use crate::adapter::outbound::app_server::{
    AppServerPlanningWorkerAdapter, CodexAppServerAdapter, GatewayUpstreamSocket,
};
use crate::adapter::outbound::clipboard::PlatformClipboardImageAdapter;
use crate::adapter::outbound::db::sqlite_work_context_store::SqliteWorkContextStore;
use crate::adapter::outbound::db::{
    SqlitePlanningAuthorityAdapter, SqliteTelegramGlobalRunnerLeaseAdapter,
};
use crate::adapter::outbound::filesystem::{
    FilesystemParallelAgentProfileRepositoryAdapter, FilesystemPlanningWorkspaceAdapter,
    FilesystemPrValidationRolloutEvidenceAdapter, RepositoryKernelLock,
    RepositoryKernelLockAdapter, RepositoryKernelLockHeld,
};
use crate::adapter::outbound::git::history::GitHistoryAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::git::repository_state::GitRepositoryStateAdapter;
use crate::adapter::outbound::github::{
    GithubAutomationAdapter, GithubPrValidationAdapter, GithubResourceVersionAdapter,
    GithubReviewPollerAdapter,
};
use crate::adapter::outbound::mcp_hub::{
    CodexMcpHubBackend, GatewayRoutedMcpHubBackend, NotionResourceVersionAdapter,
};
use crate::adapter::outbound::telegram::CurlTelegramBotAdapter;
use crate::application::port::inbound::admin_debug_port::{
    AdminDebugHarnessConfig, AdminDebugPort,
};
use crate::application::port::inbound::app_server_prompt_log_query_port::AppServerPromptLogQueryPort;
use crate::application::port::inbound::git_graph_port::GitGraphPort;
use crate::application::port::inbound::mcp_hub_port::McpHubPort;
use crate::application::port::inbound::parallel_agent_profile_port::ParallelAgentProfilePort;
use crate::application::port::inbound::parallel_mode_admin_port::ParallelModeAdminPort;
use crate::application::port::inbound::parallel_mode_control_port::ParallelModeControlPort;
use crate::application::port::inbound::planning_admin_port::PlanningAdminPort;
use crate::application::port::inbound::planning_control_port::PlanningControlPort;
use crate::application::port::inbound::planning_task_tool_port::PlanningTaskToolPort;
use crate::application::port::inbound::planning_workspace_maintenance_port::PlanningWorkspaceMaintenancePort;
use crate::application::port::inbound::pr_validation_command_port::PrValidationCommandPort;
use crate::application::port::inbound::pr_validation_query_port::PrValidationQueryPort;
use crate::application::port::inbound::pr_validation_rollout_evidence_query_port::PrValidationRolloutEvidenceQueryPort;
use crate::application::port::inbound::review_center_query_port::ReviewCenterQueryPort;
use crate::application::port::inbound::stored_command_port::StoredCommandPort;
use crate::application::port::inbound::work_context_port::WorkContextPort;
use crate::application::port::outbound::app_server_prompt_log_port::{
    AppServerPromptLogMaintenanceMode, AppServerPromptLogMaintenancePort, AppServerPromptLogPort,
    NoopAppServerPromptLogPort,
};
use crate::application::port::outbound::chats_port::ChatsPort;
use crate::application::port::outbound::conversation_thread_turn_options_port::ConversationThreadTurnOptionsPort;
use crate::application::port::outbound::dots_port::DotsPort;
use crate::application::port::outbound::gateway_upstream_port::GatewayUpstreamPort;
use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::github_pr_validation_port::GithubPrValidationPort;
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::application::port::outbound::parallel_agent_profile_repository_port::ParallelAgentProfileRepositoryPort;
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderRepositoryPort;
use crate::application::port::outbound::planning_worker_port::PlanningWorkerPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::port::outbound::pr_validation_rollout_evidence_store_port::PrValidationRolloutEvidenceStorePort;
use crate::application::port::outbound::repository_state_port::RepositoryStatePort;
use crate::application::port::outbound::review_center_repository_port::ReviewCenterRepositoryPort;
use crate::application::port::outbound::telegram_bot_port::TelegramBotPort;
use crate::application::port::outbound::telegram_global_runner_lease_port::TelegramGlobalRunnerLeasePort;
use crate::application::port::outbound::telegram_update_ledger_port::TelegramUpdateLedgerPort;
use crate::application::service::admin_debug_harness::AdminDebugHarnessService;
use crate::application::service::app_server_prompt_log_query::AppServerPromptLogQueryService;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::git_graph::GitGraphService;
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::application::service::mcp_hub::McpHubService;
use crate::application::service::parallel_agent_profile::ParallelAgentProfileService;
use crate::application::service::parallel_mode::admin::ParallelModeAdminService;
use crate::application::service::parallel_mode::{
    ParallelModeService, PrValidationSchedulerConfig, PrValidationSchedulerRuntime,
    PrValidationSchedulerService, control_plane::ParallelModeControlPlaneComposition,
};
use crate::application::service::planning::{
    PlanningAdminFacadeService, PlanningControlFacadeService, PlanningControlService,
    PlanningServices,
};
use crate::application::service::pr_validation_command::PrValidationCommandService;
use crate::application::service::pr_validation_query::PrValidationQueryService;
use crate::application::service::pr_validation_rollout_evidence_query::PrValidationRolloutEvidenceQueryService;
use crate::application::service::review_center::ReviewCenterReadService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::application::service::stored_command::StoredCommandService;
use crate::application::service::work_context::WorkContextService;
use crate::composition::native_client_runtime::{
    NativeClientRuntime, NativeTuiApplicationComposition,
};
use crate::domain::github_review::GithubPullRequestTarget;

const APP_SERVER_CLIENT_NAME: &str = "codex-exec-loop-native";
const AKRA_APP_SERVER_PROMPT_LOG_ENV_VAR: &str = "AKRA_APP_SERVER_PROMPT_LOG";

pub(crate) fn build_kernel_command_port(
    workspace: &Path,
) -> Result<Box<dyn crate::application::port::inbound::kernel_command_port::KernelCommandPort>> {
    kernel_command_execution::build(workspace, kernel_command_execution::CALLER_CLI)
}

pub(crate) struct ProductionAdminApplication {
    pub(crate) facade: Arc<dyn PlanningAdminPort>,
    pub(crate) parallel_mode_admin_port: Arc<dyn ParallelModeAdminPort>,
    pub(crate) admin_debug_port: Arc<dyn AdminDebugPort>,
    pub(crate) app_server_prompt_log_query_port: Arc<dyn AppServerPromptLogQueryPort>,
    pub(crate) parallel_agent_profile_port: Arc<dyn ParallelAgentProfilePort>,
    pub(crate) pr_validation_query_port: Arc<dyn PrValidationQueryPort>,
    pub(crate) pr_validation_rollout_evidence_query_port:
        Arc<dyn PrValidationRolloutEvidenceQueryPort>,
    pub(crate) pr_validation_command_port: Arc<dyn PrValidationCommandPort>,
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
    planning_work_order_repository_port: Arc<dyn PlanningWorkOrderRepositoryPort>,
    review_center_repository_port: Arc<dyn ReviewCenterRepositoryPort>,
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_worker_port: Arc<dyn PlanningWorkerPort>,
    parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort>,
    parallel_agent_profile_repository_port: Arc<dyn ParallelAgentProfileRepositoryPort>,
    app_server_prompt_log_maintenance_port: Arc<dyn AppServerPromptLogMaintenancePort>,
    app_server_prompt_log_port: Arc<dyn AppServerPromptLogPort>,
    conversation_thread_turn_options_port: Arc<dyn ConversationThreadTurnOptionsPort>,
    telegram_update_ledger_port: Arc<dyn TelegramUpdateLedgerPort>,
    telegram_global_runner_lease_port: Arc<dyn TelegramGlobalRunnerLeasePort>,
    pr_validation_rollout_evidence_store_port: Arc<dyn PrValidationRolloutEvidenceStorePort>,
}

#[cfg(all(test, windows))]
pub(crate) fn build_planning_services() -> PlanningServices {
    let ports = build_shared_ports();
    planning_services_from_ports(&ports)
}

/// CLI 명령이 쓰는 planning 서비스이다. 초기화는 저장소 커널의 명령으로 간다 (plan 18 K4-9a).
pub(crate) fn build_planning_services_for_workspace(workspace_dir: &str) -> PlanningServices {
    let capture_enabled = app_server_prompt_logging_enabled();
    maintain_prompt_logs_best_effort(workspace_dir, capture_enabled);
    let ports = build_shared_ports_for_prompt_logging(capture_enabled);
    entrance_planning_services(&ports, kernel_command_execution::CALLER_CLI)
}

/*
 * 입구(CLI, admin, Telegram)의 planning 서비스이다 (plan 18 K4-9a). 커널의 조립과 다른 점은 하나이다: 초기화를 직접
 * 실행하지 않고 저장소 커널의 명령 `reset_planning` 으로 보낸다. 그래서 초기화가 판단 사건 기록에 `caller` 와 함께
 * 남고, Gateway 의 커널이 도는 저장소에서는 그 커널이 실행한다. 커널의 조립(`planning_services_from_ports`)은 이
 * 함수를 쓰지 않는다. 쓰면 명령이 자기에게 돌아온다.
 */
fn entrance_planning_services(
    ports: &ProductionSharedPorts,
    caller: &'static str,
) -> PlanningServices {
    let mut planning = planning_services_from_ports(ports);
    planning.workspace = planning.workspace.with_reset_route(Arc::new(
        kernel_command_execution::KernelRoutedPlanningReset::new(caller),
    ));
    planning
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

/// CLI 용 hub 이다. 조회와 호출은 Gateway 의 상주 엔진을 먼저 쓰고, Gateway 가 없으면 직접 실행한다.
pub(crate) fn build_mcp_hub_port() -> Result<Arc<dyn McpHubPort>> {
    let local = CodexMcpHubBackend::from_environment()?;
    let hub_home = local.home().display().to_string();
    let routed = GatewayRoutedMcpHubBackend::new(Arc::new(local), hub_home);
    Ok(Arc::new(McpHubService::new(Arc::new(routed))))
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
        workspace_dir,
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
    let parallel_mode_service = parallel_mode_service_from_parts(
        ports.planning_authority_port.clone(),
        parallel_agent_profile_service.clone(),
        Arc::new(GithubPrValidationAdapter::for_local_github_credentials(
            &workspace_dir,
        )),
    );
    let scheduler_mode =
        PrValidationSchedulerConfig::from_repository(&parallel_mode_service, &workspace_dir)
            .map(|config| config.mode)
            .unwrap_or_default();
    let expected_repository = GithubAutomationAdapter::new()
        .repository_identity(&workspace_dir)
        .ok();
    let production_pr_validation_rollout_evidence_query_port: Arc<
        dyn PrValidationRolloutEvidenceQueryPort,
    > = Arc::new(
        PrValidationRolloutEvidenceQueryService::new(
            workspace_dir.clone(),
            expected_repository,
            "prerelease",
            Arc::new(FilesystemPrValidationRolloutEvidenceAdapter::new()),
        )
        .with_store(ports.pr_validation_rollout_evidence_store_port.clone()),
    );
    let parallel_mode_control_plane = Arc::new(parallel_mode_control_plane_from_service(
        &workspace_dir,
        planning.clone(),
        ports.parallel_agent_worker_port.clone(),
        parallel_mode_service,
    ));
    let parallel_mode_admin_port: Arc<dyn ParallelModeAdminPort> =
        Arc::new(ParallelModeAdminService::new(parallel_mode_control_plane));
    let production_pr_validation_query_port: Arc<dyn PrValidationQueryPort> = Arc::new(
        PrValidationQueryService::new(workspace_dir.clone(), ports.planning_authority_port.clone())
            .with_scheduler_mode(scheduler_mode)
            .with_rollout_evidence(production_pr_validation_rollout_evidence_query_port.clone()),
    );
    let production_pr_validation_command_port: Arc<dyn PrValidationCommandPort> =
        Arc::new(PrValidationCommandService::new(
            workspace_dir.clone(),
            ports.planning_authority_port.clone(),
            planning.queue.clone(),
            scheduler_mode,
        ));
    let admin_debug_service = Arc::new(AdminDebugHarnessService::new(if debug_harness_enabled {
        AdminDebugHarnessConfig::enabled()
    } else {
        AdminDebugHarnessConfig::disabled()
    }));
    let pr_validation_query_port: Arc<dyn PrValidationQueryPort> = if debug_harness_enabled {
        admin_debug_service.clone()
    } else {
        production_pr_validation_query_port
    };
    let pr_validation_command_port: Arc<dyn PrValidationCommandPort> = if debug_harness_enabled {
        admin_debug_service.clone()
    } else {
        production_pr_validation_command_port
    };
    let pr_validation_rollout_evidence_query_port: Arc<dyn PrValidationRolloutEvidenceQueryPort> =
        if debug_harness_enabled {
            admin_debug_service.clone()
        } else {
            production_pr_validation_rollout_evidence_query_port
        };
    // 입구의 작업 변경은 저장소 커널의 명령으로 간다 (plan 18 K4-9d). 읽기와 초안의 편집은 facade 가 직접 한다.
    let facade: Arc<dyn PlanningAdminPort> = Arc::new(
        super::kernel_routed_planning_admin::KernelRoutedPlanningAdmin::new(
            PlanningAdminFacadeService::from_planning_with_authority(
                workspace_dir,
                // 입구의 초기화는 저장소 커널의 명령으로 간다 (plan 18 K4-9a).
                entrance_planning_services(&ports, kernel_command_execution::CALLER_ADMIN),
                ports.planning_workspace_port,
                ports.planning_authority_port,
                ports.planning_task_repository_port,
            ),
            kernel_command_execution::CALLER_ADMIN,
        ),
    );
    ProductionAdminApplication {
        facade,
        parallel_mode_admin_port,
        admin_debug_port: admin_debug_service,
        app_server_prompt_log_query_port,
        parallel_agent_profile_port,
        pr_validation_query_port,
        pr_validation_rollout_evidence_query_port,
        pr_validation_command_port,
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
        // 입구의 초기화는 저장소 커널의 명령으로 간다 (plan 18 K4-9a).
        entrance_planning_services(&ports, kernel_command_execution::CALLER_TELEGRAM),
    )));
    let parallel_mode_control_port = Arc::new(parallel_mode_control_plane_from_parts(
        &workspace_dir,
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

/// akra-gateway 의 upstream 을 시작한다. `run_dir` 는 현재 사용자 전용 디렉터리여야 한다.
/// Gateway 가 쓰는 upstream app-server 와 저장소 커널 목록이다. 커널은 같은 upstream 에 붙는다 (K3-5).
pub(crate) struct GatewayBackend {
    pub(crate) upstream: Arc<dyn GatewayUpstreamPort>,
    /// `local` 외의 호스트이다. 지금은 `AKRA_DOTS_CLOUD` 이 켜졌을 때의 `cloud`(dots) 하나이다 (D-19).
    pub(crate) other_hosts: Vec<(String, Arc<dyn GatewayUpstreamPort>)>,
    pub(crate) kernels: Arc<RepositoryKernelRegistry>,
    pub(crate) mcp_hub: Arc<GatewayMcpHub>,
    pub(crate) work_context: Arc<dyn WorkContextPort>,
    pub(crate) resource_versions:
        Arc<crate::application::service::resource_version::ResourceVersionService>,
    /// 커밋 그래프 읽기이다 (plan 27 G1).
    pub(crate) git_graph: Arc<dyn GitGraphPort>,
    /// git 작업 엔진이다 (plan 27 G3).
    pub(crate) git_operations:
        Arc<dyn crate::application::port::inbound::git_operation_port::GitOperationPort>,
    /// 이슈 인계의 공유본이다 (#2428).
    pub(crate) issue_handoff:
        Arc<dyn crate::application::port::inbound::issue_handoff_port::IssueHandoffPort>,
    /// dot 목록이다. dots 클라우드가 켜졌을 때만 있다 (D-19).
    pub(crate) dots: Option<Arc<dyn DotsPort>>,
    /// ChatGPT 개인 대화 조회이다. dots 와 같은 조건으로 켜진다 (D-19, plan 25).
    pub(crate) chats: Option<Arc<dyn ChatsPort>>,
    /// 이슈 초안, 미리보기, 게시이다 (#2426).
    pub(crate) issue_drafts:
        Arc<dyn crate::application::port::inbound::issue_draft_port::IssueDraftPort>,
    pub(crate) delegations: Arc<crate::adapter::inbound::gateway::Delegations>,
}

pub(crate) fn build_delegations() -> crate::adapter::inbound::gateway::Delegations {
    use crate::adapter::outbound::app_server::codex_thread::CodexThreadDriver;
    use crate::application::port::outbound::agent_backend_port::TextDispatchProfile;
    use crate::application::service::issue_review::{
        ISSUE_DRAFT_REVIEW_SKILL_TEXT, IssueReviewOutcome, parse_model_reply,
    };

    crate::adapter::inbound::gateway::Delegations::new(
        Arc::new(CodexThreadDriver::default()),
        Arc::new(|| {
            #[cfg(test)]
            return None;
            #[cfg(not(test))]
            {
                use crate::adapter::outbound::app_server::cloud_upstream::{
                    chat_account_id, codex_auth_path,
                };
                chat_account_id(&codex_auth_path().ok()?)
            }
        }),
        TextDispatchProfile {
            developer_instructions: ISSUE_DRAFT_REVIEW_SKILL_TEXT,
            accepts_reply: |reply| {
                !matches!(parse_model_reply(reply), IssueReviewOutcome::Failed { .. })
            },
        },
    )
}

pub(crate) fn login_codex_delegation() -> Result<i32> {
    crate::adapter::outbound::app_server::codex_thread::login()
}

pub(crate) fn build_gateway_backend(run_dir: &Path) -> Result<GatewayBackend> {
    let upstream = AppServerGatewayUpstream::start(run_dir)?;
    let kernels = Arc::new(RepositoryKernelRegistry::sharing_app_server(
        upstream.kernel_socket(),
    ));
    let hub = Arc::new(CodexMcpHubBackend::resident_from_environment()?);
    let mcp_hub = Arc::new(GatewayMcpHub::new(
        hub.home().display().to_string(),
        hub.clone(),
    ));
    let upstream: Arc<dyn GatewayUpstreamPort> = Arc::new(upstream);
    let resource_versions =
        Arc::new(crate::application::service::resource_version::ResourceVersionService::default());
    let work_context: Arc<dyn WorkContextPort> = Arc::new(
        WorkContextService::new(
            Arc::new(AppServerThreadTranscript::new(upstream.clone())),
            Arc::new(SqliteWorkContextStore::from_environment()?),
        )
        .with_version_providers(vec![
            Arc::new(GithubResourceVersionAdapter::new(
                crate::configuration::akra_home_dir()?,
            )),
            resource_versions.clone(),
            Arc::new(NotionResourceVersionAdapter::new(hub)),
        ]),
    );
    Ok(GatewayBackend {
        upstream,
        other_hosts: build_other_hosts()?,
        kernels,
        mcp_hub,
        work_context,
        resource_versions,
        git_graph: Arc::new(GitGraphService::new(Arc::new(GitHistoryAdapter))),
        git_operations: build_git_operations(),
        issue_handoff: Arc::new(
            crate::application::service::issue_handoff::IssueHandoffService::new(
                Arc::new(
                    crate::adapter::outbound::github::issue_snapshot::GithubIssueSnapshotAdapter::new(
                        crate::configuration::akra_home_dir()?,
                    ),
                ),
                Arc::new(
                    crate::adapter::outbound::filesystem::issue_policy::FilesystemIssuePolicySourceAdapter::new(),
                ),
            ),
        ),
        dots: build_dots()?,
        chats: build_chats()?,
        issue_drafts: build_issue_drafts()?,
        delegations: Arc::new(build_delegations()),
    })
}

/// 이슈 초안 서비스를 조립한다 (#2426). 게시는 게시 의도를 기록하는 서비스로만 한다 (#2424). 일지는 git 작업 일지와
/// 같은 저장소 runtime 디렉터리에 둔다. `gh` 는 저장소가 아닌 Akra 의 폴더에서 실행한다.
pub(crate) fn build_issue_form_install()
-> Arc<dyn crate::application::port::inbound::issue_form_install_port::IssueFormInstallPort> {
    use crate::adapter::outbound::git::issue_form_repository::GitIssueFormRepositoryAdapter;
    use crate::adapter::outbound::github::automation::GithubAutomationAdapter;
    use crate::application::service::issue_form_install::IssueFormInstallService;
    Arc::new(IssueFormInstallService::new(
        Arc::new(GitIssueFormRepositoryAdapter::new()),
        Arc::new(GithubAutomationAdapter::new()),
    ))
}

/// 기존 이슈 게시와 Gateway 조립 회귀가 같은 저장소 runtime 해석을 사용한다.
pub(crate) fn build_issue_publish_journal()
-> Arc<dyn crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort>{
    use crate::adapter::outbound::filesystem::issue_publish_intent_journal::IssuePublishIntentJournalAdapter;

    Arc::new(IssuePublishIntentJournalAdapter::new(|workspace_dir| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace_dir)?;
        Ok(PathBuf::from(location.runtime_dir))
    }))
}

fn build_issue_drafts()
-> Result<Arc<dyn crate::application::port::inbound::issue_draft_port::IssueDraftPort>> {
    use crate::adapter::outbound::filesystem::evidence_vault_store::FilesystemEvidenceVaultStoreAdapter;
    use crate::adapter::outbound::filesystem::issue_automation_policy::FilesystemIssueAutomationPolicySourceAdapter;
    use crate::adapter::outbound::filesystem::issue_creation_journal::IssueCreationJournalAdapter;
    use crate::adapter::outbound::filesystem::issue_policy::FilesystemIssuePolicySourceAdapter;
    use crate::adapter::outbound::filesystem::issue_template::FilesystemIssueTemplateSourceAdapter;
    use crate::adapter::outbound::github::{
        GithubIssueCreateAdapter, GithubIssueSnapshotAdapter, GithubIssueUpdateAdapter,
    };
    use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
    use crate::application::service::issue_creation::IssueCreationService;
    use crate::application::service::issue_draft::IssueDraftService;
    use crate::application::service::issue_policy::IssuePolicyService;
    use crate::application::service::issue_publish_journal::IssueJournaledPublishService;
    use crate::application::service::issue_template::IssueTemplateService;

    let akra_home = crate::configuration::akra_home_dir()?;
    let vault_root = akra_home.join("evidence-vault");
    let _ = crate::private_directory::ensure_private_directory(&vault_root);
    let vault = Arc::new(FilesystemEvidenceVaultStoreAdapter::new(vault_root));
    let snapshot = Arc::new(GithubIssueSnapshotAdapter::new(akra_home.clone()));
    let journal = build_issue_publish_journal();
    // Creation and updates share the same ordering owner and restore-link mutex.
    let publisher = Arc::new(
        IssueJournaledPublishService::new(
            snapshot.clone(),
            Arc::new(GithubIssueUpdateAdapter::new(akra_home.clone())),
            journal,
            vault.clone(),
        )
        .with_feedback_store(super::issue_collection::build_feedback_store()),
    );
    let creation_journal = IssueCreationJournalAdapter::new(|workspace_dir| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace_dir)?;
        Ok(PathBuf::from(location.runtime_dir))
    })
    .with_workspace_resolver(|cwd| Ok(repository_kernel_target(cwd)?.workspace_root));
    let creator = Arc::new(IssueCreationService::new(
        Arc::new(GithubIssueCreateAdapter::new(akra_home)),
        snapshot.clone(),
        Arc::new(creation_journal),
        vault,
        publisher.clone(),
    ));
    Ok(Arc::new(IssueDraftService::new(
        IssuePolicyService::new(Arc::new(FilesystemIssuePolicySourceAdapter::new())),
        IssueTemplateService::new(Arc::new(FilesystemIssueTemplateSourceAdapter::new())),
        snapshot,
        publisher,
        Arc::new(
            crate::adapter::outbound::issue_code_validator::SandboxedIssueCodeValidatorRunnerAdapter::new(),
        ),
        Arc::new(
            crate::application::service::evidence_vault_unlock::EvidenceVaultUnlockService::new(
                Arc::new(crate::adapter::outbound::vault_key_store::OsVaultKeyStoreAdapter::new()),
            ),
        ),
    ).with_creation(creator)
        .with_automation_policy(Arc::new(IssueAutomationPolicyService::new(Arc::new(
            FilesystemIssueAutomationPolicySourceAdapter::new(),
        ))))))
}

/// git 작업 엔진을 조립한다 (plan 27 G3). 일지는 planning 기준 데이터와 같은 저장소 runtime 디렉터리에 둔다. 그
/// 디렉터리는 현재 사용자 전용이고 저장소 밖에 있다.
fn build_git_operations()
-> Arc<dyn crate::application::port::inbound::git_operation_port::GitOperationPort> {
    use crate::adapter::outbound::filesystem::git_operation_journal::GitOperationJournalAdapter;
    use crate::adapter::outbound::git::workspace::GitWorkspaceAdapter;
    use crate::application::service::git_operation::GitOperationService;

    let journal = GitOperationJournalAdapter::new(|repository_dir| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(repository_dir)?;
        Ok(PathBuf::from(location.runtime_dir))
    });
    Arc::new(GitOperationService::new(
        Arc::new(GitHistoryAdapter),
        Arc::new(GitWorkspaceAdapter),
        Arc::new(journal),
        Arc::new(|| (chrono::Utc::now().timestamp(), rand::random::<u32>())),
    ))
}

/// `local` 외의 호스트를 만든다. dots 클라우드 호스트는 `AKRA_DOTS_CLOUD` 이 켜졌을 때만 켠다 (D-19). 호스트를
/// 만들어도 클라이언트가 `/host/cloud` 로 붙기 전에는 클라우드에 접속하지 않는다.
fn build_other_hosts() -> Result<Vec<(String, Arc<dyn GatewayUpstreamPort>)>> {
    if !dots_cloud_enabled() {
        return Ok(Vec::new());
    }
    let auth_path = codex_auth_path()?;
    let cloud: Arc<dyn GatewayUpstreamPort> = Arc::new(CloudGatewayUpstream::new(auth_path));
    Ok(vec![("cloud".to_string(), cloud)])
}

/// dot 목록 adapter 이다. 클라우드 호스트와 같은 조건으로 켠다. 만들어도 요청 전에는 서비스에 붙지 않는다.
fn build_dots() -> Result<Option<Arc<dyn DotsPort>>> {
    if !dots_cloud_enabled() {
        return Ok(None);
    }
    let dots: Arc<dyn DotsPort> = Arc::new(CloudDotsRestAdapter::new(codex_auth_path()?));
    Ok(Some(dots))
}

/// ChatGPT 개인 대화 조회 adapter 이다. dots 와 같은 조건(클라우드 플래그)으로 켠다. 같은 토큰을 쓴다 (D-19, plan 25).
fn build_chats() -> Result<Option<Arc<dyn ChatsPort>>> {
    if !dots_cloud_enabled() {
        return Ok(None);
    }
    let chats: Arc<dyn ChatsPort> = Arc::new(CloudChatsRestAdapter::new(codex_auth_path()?));
    Ok(Some(chats))
}

/// dots 클라우드 호스트를 켜는 조건이다. 둘 중 하나면 켠다 (plan 25 §3.2, 기본값 꺼짐):
/// - 환경 변수 `AKRA_DOTS_CLOUD` 가 `1`/`true`.
/// - 플래그 파일 `$AKRA_HOME/dots/cloud-enabled` 가 있다. Desktop 이 설정에서 이 파일을 만들고 지운다. Gateway 는
///   시작할 때 한 번 읽으므로, 설정을 바꾼 뒤에는 Gateway 를 다시 시작해야 적용된다.
fn dots_cloud_enabled() -> bool {
    let from_env = std::env::var_os("AKRA_DOTS_CLOUD")
        .map(|value| value == "1" || value == "true")
        .unwrap_or(false);
    from_env
        || dots_cloud_flag_path()
            .map(|path| path.exists())
            .unwrap_or(false)
}

/// dots 클라우드 활성화 플래그 파일의 경로이다. Desktop 과 Gateway 가 같은 경로를 쓴다.
pub(crate) fn dots_cloud_flag_path() -> Option<PathBuf> {
    crate::configuration::akra_home_dir()
        .ok()
        .map(|home| dots_cloud_flag_in(&home))
}

/// `$AKRA_HOME/dots/cloud-enabled`. 경로 규칙만 담는 순수 함수이다. Desktop 의 `dotsSettings.ts` 와 같아야 한다.
pub(crate) fn dots_cloud_flag_in(akra_home: &Path) -> PathBuf {
    akra_home.join("dots").join("cloud-enabled")
}

pub(crate) fn build_telegram_bot_port(token: String) -> Arc<dyn TelegramBotPort> {
    Arc::new(CurlTelegramBotAdapter::new(token))
}

pub(crate) fn resolve_active_planning_workspace_root(workspace_dir: &str) -> PathBuf {
    SqlitePlanningAuthorityAdapter::resolve_active_workspace_root(workspace_dir)
}

pub(crate) fn delegation_runtime(workspace: &str) -> Result<PathBuf> {
    SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)
        .map(|location| PathBuf::from(location.runtime_dir))
}

pub(crate) fn delegation_route(
    workspace: &str,
    feature: &str,
) -> Result<crate::domain::delegation::policy::Route> {
    use crate::domain::delegation::policy::{PolicyFile, resolve};
    use sha2::{Digest, Sha256};

    fn load(path: &std::path::Path) -> Result<String> {
        match std::fs::read_to_string(path) {
            Ok(contents) if contents.len() <= 128 * 1024 => Ok(contents),
            Ok(_) => anyhow::bail!("delegation policy file is too large"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error.into()),
        }
    }

    let read = || {
        let home = crate::configuration::akra_home_dir()?;
        let root = resolve_active_planning_workspace_root(workspace);
        let user = load(&home.join("delegation.toml"))?;
        let repo = load(&root.join(".akra/delegation.toml"))?;
        let mut hash = Sha256::new();
        hash.update(user.len().to_le_bytes());
        hash.update(&user);
        hash.update(&repo);
        resolve(
            toml::from_str::<PolicyFile>(&user)?,
            toml::from_str::<PolicyFile>(&repo)?,
            feature,
            format!("{:x}", hash.finalize()),
        )
    };
    read().map_err(|_| anyhow::anyhow!("policy_blocked"))
}

pub(crate) fn delegation_store(
    runtime: PathBuf,
    existing_only: bool,
) -> Option<Arc<dyn crate::application::port::outbound::delegation_store_port::DelegationStorePort>>
{
    let store =
        crate::adapter::outbound::db::sqlite_delegation_store::DelegationStore::new(runtime);
    (!existing_only || store.exists()).then(|| Arc::new(store) as Arc<_>)
}

/// `workspace_dir` 가 속한 저장소의 커널 잠금을 잡는다 (plan 18 K3-4). 결정 고리를 돌리는 프로세스는 런타임을
/// 조립하기 전에 이것을 잡는다. 다른 프로세스가 쥐고 있으면 `RepositoryKernelLockHeld` 오류이다.
/// 잠금 파일은 planning 기준 데이터와 같은 사용자 전용 runtime 디렉터리에 둔다.
pub(crate) fn acquire_repository_kernel_lock(workspace_dir: &str) -> Result<RepositoryKernelLock> {
    let location =
        SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace_dir)?;
    RepositoryKernelLockAdapter::acquire(location, workspace_dir)
}

/// 클라이언트가 붙으려는 폴더가 속한 저장소의 커널 자리이다 (plan 18 K3-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryKernelTarget {
    /// 저장소 신원이다. 커널 잠금의 `repository_key` 와 같은 값이다.
    pub(crate) repository_key: String,
    /// 커널의 작업 공간이다. 저장소의 대표 checkout 이고, git 이 아닌 폴더는 정규화한 그 폴더이다. 어느 worktree
    /// 에서 처음 붙어도 같은 값이다.
    pub(crate) workspace_root: String,
}

/// `workspace_dir` 가 속한 저장소의 커널 자리를 찾는다. 잠금을 잡지 않는다. 병렬 pool 안의 worktree(slot,
/// 통합)는 커널이 관리하는 작업 공간이므로 거절한다. 대표 checkout 이 없는 저장소(bare 저장소, 대표 checkout 의
/// `.git` 이 없음)도 거절한다. 그 저장소는 처음 붙은 worktree 에 따라 병렬 pool 위치가 바뀌기 때문이다.
pub(crate) fn repository_kernel_target(workspace_dir: &str) -> Result<RepositoryKernelTarget> {
    let Some(primary_checkout) =
        SqlitePlanningAuthorityAdapter::resolve_primary_checkout_root(workspace_dir)?
    else {
        anyhow::bail!(
            "`{workspace_dir}` belongs to a repository without a primary checkout (a bare \
             repository, or the primary checkout lost its `.git`). The Akra kernel needs a stable \
             primary checkout for its parallel pool; attach from a regular clone instead"
        )
    };
    let location =
        SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace_dir)?;
    if crate::application::service::parallel_mode::workspace_is_inside_parallel_pool(
        &primary_checkout,
        Path::new(&location.workspace_root),
    ) {
        anyhow::bail!(
            "`{workspace_dir}` is an Akra parallel slot worktree. Attach from the repository checkout \
             `{}` instead; the repository kernel manages its slot worktrees",
            primary_checkout.display()
        )
    }
    Ok(RepositoryKernelTarget {
        repository_key: location.repository_identity,
        workspace_root: primary_checkout.display().to_string(),
    })
}

/// TUI 가 시작 폴더의 저장소 잠금을 잡는다 (plan 18 K3-4). 다른 프로세스가 그 저장소의 커널을 돌리면 오류를
/// 돌려주고, TUI 는 화면을 열기 전에 그 이유를 보이고 끝난다.
pub(crate) fn acquire_native_tui_kernel_lock() -> Result<Option<RepositoryKernelLock>> {
    acquire_native_tui_kernel_lock_for_workspace(&native_tui_workspace_dir())
}

/// 잠금 파일 오류(경합이 아닌 오류)는 이전 개입의 planning fence를 복구한 경우에만 우회한다.
/// 복구 실패는 runtime 시작을 막고, 복구 성공 시 경고를 남긴 뒤 잠금 없이 시작한다.
pub(crate) fn acquire_native_tui_kernel_lock_for_workspace(
    workspace_dir: &str,
) -> Result<Option<RepositoryKernelLock>> {
    match acquire_repository_kernel_lock(workspace_dir) {
        Ok(lock) => {
            recover_pending_issue_interventions(workspace_dir)?;
            Ok(Some(lock))
        }
        Err(error) if error.downcast_ref::<RepositoryKernelLockHeld>().is_some() => Err(error),
        Err(error) => {
            // 잠금 없이 시작하는 기존 fallback도 durable 제어 fence를 건너뛰지는 않는다.
            recover_pending_issue_interventions(workspace_dir)?;
            tracing::warn!(
                error = %format!("{error:#}"),
                "kernel lock unavailable; the TUI starts without it"
            );
            Ok(None)
        }
    }
}

/// 잠금 획득 또는 잠금 파일 오류 fallback에서 runtime을 만들기 전에 호출한다.
/// 수집 Off에서도 이전 제어 intent의 planning fence가 실행보다 먼저 적용되어야 한다.
/// 조립/복구는 새 worker나 모델 턴을 시작하지 않는다.
pub(crate) fn recover_pending_issue_interventions(workspace_dir: &str) -> Result<()> {
    // 이슈 장부는 대표 checkout 범위다. 같은 저장소의 linked worktree도 같은 intent를 읽는다.
    let workspace = repository_kernel_target(workspace_dir)?.workspace_root;
    super::issue_collection::build_interventions(
        &workspace,
        Arc::new(crate::application::port::outbound::issue_work_steering_port::UnavailableIssueWorkSteering),
    ).recover_before_runtime()?;
    super::issue_collection::build_autonomy(&workspace).recover_before_runtime()
}

fn native_tui_workspace_dir() -> String {
    std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| ".".to_string())
}

pub(crate) fn build_native_tui_application() -> NativeTuiApplicationComposition {
    build_native_application_for_workspace(&native_tui_workspace_dir(), None).application
}

/// 커널의 판단 사건 기록이다 (plan 19 Q3-1). planning SQLite 의 `kernel_events` 표이다.
pub(crate) fn kernel_event_log_port() -> Arc<dyn KernelEventLogPort> {
    Arc::new(SqlitePlanningAuthorityAdapter::new())
}

/// 그 작업 공간의 커널 기록에 남은 명령과 그 명령을 보낸 입구이다. 입구의 시험이 "커널을 거쳤다"를 볼 때 쓴다.
#[cfg(test)]
pub(crate) fn recorded_commands_for_test(workspace_dir: &str) -> Vec<(String, Option<String>)> {
    use crate::domain::kernel_event::{COMMAND_KINDS, KernelEvent};
    kernel_event_log_port()
        .load_kernel_events(workspace_dir, &COMMAND_KINDS)
        .expect("the kernel log of the test workspace should load")
        .into_iter()
        .filter_map(|event| match event {
            KernelEvent::CommandReceived {
                command, caller, ..
            } => Some((command, caller)),
            _ => None,
        })
        .collect()
}

/*
 * 저장 명령의 실행기이다 (plan 18 K4-9d). 커널의 조립이다: 변경을 직접 실행한다. 입구의 조립을 쓰면 명령이
 * 자기에게 돌아온다. 기본 명령 전용 조립은 저장소와 파일을 쓴다. 상주 커널이 주입한 steering은 이미
 * 실행 중인 worker만 제어한다. 새 app-server나 planning worker를 시작하지 않는다.
 */
pub(crate) fn stored_command_port(workspace_dir: &str) -> Arc<dyn StoredCommandPort> {
    stored_command_port_with_issues(
        workspace_dir,
        super::issue_collection::build(workspace_dir),
        Arc::new(crate::application::port::outbound::issue_work_steering_port::UnavailableIssueWorkSteering),
    )
}

pub(crate) fn stored_command_port_with_issues(
    workspace_dir: &str,
    issues: Arc<crate::application::service::issue_collection::IssueCollectionService>,
    steering: Arc<
        dyn crate::application::port::outbound::issue_work_steering_port::IssueWorkSteeringPort,
    >,
) -> Arc<dyn StoredCommandPort> {
    struct NoPlanningWorker;
    impl PlanningWorkerPort for NoPlanningWorker {
        fn run_planning_session(
            &self,
            _request: crate::application::port::outbound::planning_worker_port::PlanningWorkerRequest,
        ) -> Result<crate::application::port::outbound::planning_worker_port::PlanningWorkerResponse>
        {
            anyhow::bail!("a stored command does not run a planning worker")
        }
    }
    let (planning, workspace, store) = store_planning_services(Arc::new(NoPlanningWorker));
    Arc::new(
        StoredCommandService::new(PlanningAdminFacadeService::from_planning_with_authority(
            workspace_dir,
            planning,
            workspace,
            store.clone(),
            store,
        ))
        .with_issue_collection(issues)
        .with_issue_decisions(super::issue_collection::build_decisions(workspace_dir))
        .with_issue_interventions(super::issue_collection::build_interventions(
            workspace_dir,
            steering,
        )),
    )
}

/// 저장소와 파일만 쓰는 planning 서비스이다. app-server 를 시작하지 않는다.
fn store_planning_services(
    worker: Arc<dyn PlanningWorkerPort>,
) -> (
    PlanningServices,
    Arc<dyn PlanningWorkspacePort>,
    Arc<SqlitePlanningAuthorityAdapter>,
) {
    let store = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let workspace: Arc<dyn PlanningWorkspacePort> = Arc::new(
        FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(store.clone()),
    );
    let planning =
        PlanningServices::from_ports(workspace.clone(), store.clone(), store.clone(), worker)
            .with_work_order_repository(store.clone());
    (planning, workspace, store)
}

/*
 * 저장소에서 읽은 지금의 planning 상태이다 (plan 18 K4-9d). 커널이 떠 있지 않을 때 명령 전용 조립이 `planning` 조회에
 * 이 값으로 답한다. 낡은 revision 으로 거절된 클라이언트가 지금의 revision 을 읽을 수 있어야 하기 때문이다.
 */
pub(crate) fn stored_planning_projection(
    workspace_dir: &str,
) -> crate::domain::planning::RuntimeProjection {
    struct NoPlanningWorker;
    impl PlanningWorkerPort for NoPlanningWorker {
        fn run_planning_session(
            &self,
            _request: crate::application::port::outbound::planning_worker_port::PlanningWorkerRequest,
        ) -> Result<crate::application::port::outbound::planning_worker_port::PlanningWorkerResponse>
        {
            anyhow::bail!("a planning query does not run a planning worker")
        }
    }
    let (planning, _, _) = store_planning_services(Arc::new(NoPlanningWorker));
    planning
        .runtime
        .load_runtime_projection_or_invalid(workspace_dir)
}

/// 커널이 관찰하는 작업 공간의 git 상태이다 (plan 20 §4.1).
pub(crate) fn repository_state_port() -> Arc<dyn RepositoryStatePort> {
    Arc::new(GitRepositoryStateAdapter)
}

/// 화면 없는 커널의 런타임이다 (plan 18 K3-4). 호출자는 먼저 그 저장소의 커널 잠금을 잡는다. 조립이 PR 검증
/// 스케줄러 스레드를 시작하기 때문이다. `app_server` 가 있으면 커널은 자기 app-server 대신 Gateway upstream 에
/// 붙어 Desktop 과 같은 app-server 를 쓴다 (K3-5).
pub(crate) fn build_headless_kernel_runtime(
    workspace_dir: &str,
    app_server: Option<GatewayUpstreamSocket>,
    issues: Arc<crate::application::service::issue_collection::IssueCollectionService>,
) -> NativeClientRuntime {
    let assembly = build_native_application_for_workspace(workspace_dir, app_server);
    let mut runtime = assembly.application.into_headless_client_runtime();
    // 실행 중인 app-server의 제어 권한은 조립 안에서 저장 명령에 주입한다.
    // 클라이언트 facade에는 별도 제어 접근자를 남기지 않는다.
    runtime.attach_stored_commands(
        stored_command_port_with_issues(workspace_dir, issues, assembly.issue_work_steering),
        workspace_dir,
    );
    runtime
}

struct NativeApplicationAssembly {
    application: NativeTuiApplicationComposition,
    issue_work_steering: Arc<
        dyn crate::application::port::outbound::issue_work_steering_port::IssueWorkSteeringPort,
    >,
}

fn build_native_application_for_workspace(
    workspace_dir: &str,
    app_server: Option<GatewayUpstreamSocket>,
) -> NativeApplicationAssembly {
    let workspace_dir = workspace_dir.to_string();
    let capture_enabled = app_server_prompt_logging_enabled();
    let ports = build_shared_ports_with_app_server(capture_enabled, app_server);
    let steering_authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let issue_work_steering = Arc::new(
        crate::adapter::outbound::akra_issue_work_steering::AkraIssueWorkSteeringAdapter::new(
            steering_authority.clone(),
            steering_authority,
            ports.app_server_adapter.clone(),
        ),
    );
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
        .with_thread_turn_options_port(ports.conversation_thread_turn_options_port.clone())
        .with_review_center_read_service(review_center_read_service.clone());
    let planning = planning_services_from_ports(&ports);
    let parallel_agent_profile_service = parallel_agent_profile_service_from_ports(&ports);
    let parallel_mode_control_plane = parallel_mode_control_plane_from_parts(
        &workspace_dir,
        planning,
        ports.planning_authority_port,
        ports.parallel_agent_worker_port,
        parallel_agent_profile_service,
        Arc::new(GithubPrValidationAdapter::for_local_github_credentials(
            &workspace_dir,
        )),
    );
    let clipboard_image_probe_trigger = CompositionClipboardImageProbeTrigger::new_shared(
        Arc::new(PlatformClipboardImageAdapter::new()),
    );
    NativeApplicationAssembly {
        application: NativeTuiApplicationComposition::from_services(
            startup_service,
            session_service,
            conversation_service,
            parallel_mode_control_plane,
            ports.conversation_thread_turn_options_port,
            clipboard_image_probe_trigger,
        ),
        issue_work_steering,
    }
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
    Ok(compose_pr_review_poller(repo_root, adapter))
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
    Ok(Some((target, compose_pr_review_poller(repo_root, adapter))))
}

pub(crate) fn pr_review_settings(repo_root: &Path) -> Result<PrReviewSettings> {
    use crate::adapter::outbound::db::sqlite_pr_review_journal::ReviewJournal;
    let repository = GithubReviewPollerAdapter::review_repository(repo_root)?;
    let runtime = delegation_runtime(repo_root.to_str().context("workspace is not UTF-8")?)?;
    Ok(PrReviewSettings {
        repository,
        journal: ReviewJournal::new(runtime),
    })
}

/// Local operator settings bound to the repository resolved from the workspace.
pub(crate) struct PrReviewSettings {
    repository: String,
    journal: crate::adapter::outbound::db::sqlite_pr_review_journal::ReviewJournal,
}

impl PrReviewSettings {
    pub(crate) fn repository(&self) -> &str {
        &self.repository
    }

    pub(crate) fn set_mode(&self, mode: crate::domain::pr_review::ReviewMode) -> Result<()> {
        self.journal.set_mode(&self.repository, mode)
    }

    pub(crate) fn set_api_key(&self, key: &str) -> Result<()> {
        crate::adapter::outbound::openai_review::set_key(&self.repository, key)
    }
}

pub(crate) fn query_pr_review(
    workspace: &str,
    number: Option<u64>,
) -> Result<crate::adapter::inbound::kernel_protocol::KernelQueryAnswer> {
    use crate::adapter::inbound::kernel_protocol::KernelQueryAnswer;
    use crate::application::port::outbound::pr_review_port::PrReviewJournalPort;
    let PrReviewSettings {
        repository,
        journal,
    } = pr_review_settings(Path::new(workspace))?;
    Ok(match number {
        None => KernelQueryAnswer::FeatureBindings {
            pr_review: journal.mode(&repository)?,
            repository: Some(repository),
        },
        Some(number) => {
            anyhow::ensure!(
                (1..=9_007_199_254_740_991).contains(&number),
                "invalid pull request number"
            );
            KernelQueryAnswer::PrReviewResult {
                number,
                review: journal.latest(&GithubPullRequestTarget::new(repository, number))?,
            }
        }
    })
}

fn compose_pr_review_poller(
    repo_root: &Path,
    adapter: GithubReviewPollerAdapter,
) -> GithubReviewPollerService {
    use crate::application::service::pr_review::PrReviewService;
    let adapter = Arc::new(adapter);
    let port: Arc<dyn GithubReviewPollerPort> = adapter.clone();
    let service = GithubReviewPollerService::new(port);
    let review = || {
        let PrReviewSettings {
            repository,
            journal,
        } = pr_review_settings(repo_root).ok()?;
        let backend = super::pr_review::RoutedPrReview::new(repo_root.to_str()?, &repository);
        Some(PrReviewService::new(
            repository,
            Arc::new(journal),
            adapter,
            Some(Arc::new(backend)),
        ))
    };
    match review() {
        Some(review) => service.with_pr_review(review),
        None => service,
    }
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
    if let Some(config) = crate::configuration::current_process_config() {
        return config.config.app_server.prompt_log;
    }
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
    build_shared_ports_with_app_server(prompt_logging_enabled, None)
}

fn build_shared_ports_with_app_server(
    prompt_logging_enabled: bool,
    app_server: Option<GatewayUpstreamSocket>,
) -> ProductionSharedPorts {
    let planning_authority_adapter = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning_authority_port: Arc<dyn PlanningAuthorityPort> =
        planning_authority_adapter.clone();
    let app_server_prompt_log_port = build_prompt_log_port_for_setting(
        planning_authority_adapter.clone(),
        prompt_logging_enabled,
    );
    let app_server_prompt_log_maintenance_port: Arc<dyn AppServerPromptLogMaintenancePort> =
        planning_authority_adapter.clone();
    let conversation_thread_turn_options_port: Arc<dyn ConversationThreadTurnOptionsPort> =
        planning_authority_adapter.clone();
    let app_server_adapter = app_server_adapter(
        app_server_prompt_log_port.clone(),
        conversation_thread_turn_options_port.clone(),
        app_server,
    );
    let planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort> =
        planning_authority_adapter.clone();
    let planning_work_order_repository_port: Arc<dyn PlanningWorkOrderRepositoryPort> =
        planning_authority_adapter.clone();
    let review_center_repository_port: Arc<dyn ReviewCenterRepositoryPort> =
        planning_authority_adapter.clone();
    let pr_validation_rollout_evidence_store_port: Arc<dyn PrValidationRolloutEvidenceStorePort> =
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
        planning_work_order_repository_port,
        review_center_repository_port,
        planning_workspace_port,
        planning_worker_port,
        parallel_agent_worker_port,
        parallel_agent_profile_repository_port,
        app_server_prompt_log_maintenance_port,
        app_server_prompt_log_port,
        conversation_thread_turn_options_port,
        telegram_update_ledger_port,
        telegram_global_runner_lease_port,
        pr_validation_rollout_evidence_store_port,
    }
}

fn planning_services_from_ports(ports: &ProductionSharedPorts) -> PlanningServices {
    PlanningServices::from_ports(
        ports.planning_workspace_port.clone(),
        ports.planning_authority_port.clone(),
        ports.planning_task_repository_port.clone(),
        ports.planning_worker_port.clone(),
    )
    .with_work_order_repository(ports.planning_work_order_repository_port.clone())
}

fn parallel_agent_profile_service_from_ports(
    ports: &ProductionSharedPorts,
) -> ParallelAgentProfileService {
    ParallelAgentProfileService::new(ports.parallel_agent_profile_repository_port.clone())
}

fn parallel_mode_control_plane_from_parts(
    workspace_dir: &str,
    planning: PlanningServices,
    planning_authority_port: Arc<dyn PlanningAuthorityPort>,
    parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort>,
    parallel_agent_profile_service: ParallelAgentProfileService,
    pr_validation_observation: Arc<dyn GithubPrValidationPort>,
) -> ParallelModeControlPlaneComposition {
    let parallel_mode_service = parallel_mode_service_from_parts(
        planning_authority_port,
        parallel_agent_profile_service,
        pr_validation_observation,
    );
    parallel_mode_control_plane_from_service(
        workspace_dir,
        planning,
        parallel_agent_worker_port,
        parallel_mode_service,
    )
}

fn parallel_mode_service_from_parts(
    planning_authority_port: Arc<dyn PlanningAuthorityPort>,
    parallel_agent_profile_service: ParallelAgentProfileService,
    pr_validation_observation: Arc<dyn GithubPrValidationPort>,
) -> ParallelModeService {
    ParallelModeService::new(
        planning_authority_port,
        github_automation_port(),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    )
    .with_pr_validation_observation(pr_validation_observation)
    .with_parallel_agent_profile_service(parallel_agent_profile_service)
}

fn parallel_mode_control_plane_from_service(
    workspace_dir: &str,
    planning: PlanningServices,
    parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort>,
    parallel_mode_service: ParallelModeService,
) -> ParallelModeControlPlaneComposition {
    let scheduler =
        build_pr_validation_scheduler_runtime(&parallel_mode_service, &planning, workspace_dir);
    ParallelModeControlPlaneComposition::new(
        parallel_mode_service,
        planning,
        parallel_agent_worker_port,
    )
    .with_pr_validation_scheduler(scheduler)
}

fn build_pr_validation_scheduler_runtime(
    parallel_mode_service: &ParallelModeService,
    planning: &PlanningServices,
    workspace_dir: &str,
) -> PrValidationSchedulerRuntime {
    let result = PrValidationSchedulerConfig::from_repository(parallel_mode_service, workspace_dir)
        .and_then(|config| {
            PrValidationSchedulerService::new(
                parallel_mode_service.clone(),
                planning.queue.clone(),
                workspace_dir,
                config,
            )
        })
        .and_then(PrValidationSchedulerRuntime::start);
    match result {
        Ok(runtime) => runtime,
        Err(error) => {
            crate::diagnostics::event_log::emit_lazy(
                "pr_validation_scheduler_start_blocked",
                || {
                    serde_json::json!({
                        "workspace": workspace_dir,
                        "error": error,
                    })
                },
            );
            PrValidationSchedulerRuntime::default()
        }
    }
}

fn app_server_adapter(
    prompt_log_port: Arc<dyn AppServerPromptLogPort>,
    conversation_thread_turn_options_port: Arc<dyn ConversationThreadTurnOptionsPort>,
    app_server: Option<GatewayUpstreamSocket>,
) -> Arc<CodexAppServerAdapter> {
    let adapter = CodexAppServerAdapter::from_environment_with_prompt_log_and_thread_turn_options(
        APP_SERVER_CLIENT_NAME,
        env!("CARGO_PKG_VERSION"),
        prompt_log_port,
        conversation_thread_turn_options_port,
    );
    Arc::new(match app_server {
        Some(socket) => adapter.through_gateway_upstream(socket),
        None => adapter,
    })
}

fn github_automation_port() -> Arc<dyn GithubAutomationPort> {
    Arc::new(GithubAutomationAdapter::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
    use crate::application::port::outbound::app_server_prompt_log_port::{
        AppServerPromptInputRecord, AppServerPromptInteractionRecord,
    };
    use crate::application::port::outbound::github_pr_validation_port::{
        GithubPrMergeState, GithubPrValidationObservationRequest, GithubPrValidationPort,
        GithubPrValidationSnapshot, GithubValidationCheckRun, GithubValidationRunStatus,
        GithubValidationSource, GithubValidationSourceObservation, GithubValidationSourceStatus,
    };
    use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
    use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkControlRepositoryPort;
    use crate::application::service::planning::{PlanningControlCommand, PlanningControlRequest};
    use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId, GithubPullRequestTarget};
    use crate::domain::issue_automation::collection::IssueCollectionMode;
    use crate::domain::issue_automation::coordinator::issue_work_order_id;
    use crate::domain::issue_automation::intervention::{
        IssueInterventionAction, IssueInterventionRequest,
    };
    use crate::domain::issue_automation::{
        IssueAutomationBinding, IssueAutomationBindingId, IssueAutomationSnapshot, TrackedIssue,
    };
    use crate::domain::issue_work::{
        IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference,
        IssueSharedCopy,
    };
    use crate::domain::parallel_mode::{
        PrValidationCommitSha, PrValidationRecord, PrValidationRecordKey, PrValidationTarget,
        PrValidationTargetShaSnapshot,
    };
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn the_dots_cloud_flag_lives_under_the_akra_home() {
        let path = dots_cloud_flag_in(Path::new("/home/user/.akra"));
        assert_eq!(
            path,
            Path::new("/home/user/.akra")
                .join("dots")
                .join("cloud-enabled")
        );
    }

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
        ) -> std::result::Result<
            GithubPrValidationSnapshot,
            crate::application::port::outbound::github_pr_validation_port::GithubPrValidationError,
        > {
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
            provider_metadata: Default::default(),
        }
    }

    fn initialize_git_workspace(workspace: &TempWorkspace) {
        for args in [
            vec!["init", "-b", "prerelease"],
            vec!["config", "user.name", "Akra Test"],
            vec!["config", "user.email", "akra@example.invalid"],
            vec!["commit", "--allow-empty", "-m", "initial"],
        ] {
            let output = crate::test_utils::git_command()
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
    fn production_composition_scheduler_is_independent_from_manual_runtime_ticks() {
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
            snapshots: Mutex::new(vec![validation_snapshot(
                GithubValidationRunStatus::InProgress,
            )]),
            requests: Mutex::new(Vec::new()),
        });
        let composition = parallel_mode_control_plane_from_parts(
            &workspace_dir,
            planning.clone(),
            ports.planning_authority_port.clone(),
            ports.parallel_agent_worker_port.clone(),
            parallel_agent_profile_service_from_ports(&ports),
            observation.clone(),
        );

        for _ in 0..100 {
            if observation.requests.lock().unwrap().len() == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(observation.requests.lock().unwrap().len(), 1);
        for _ in 0..5 {
            composition
                .run_manual_orchestrator_tick(&workspace_dir)
                .expect("production-composed runtime tick should complete");
        }
        std::thread::sleep(Duration::from_millis(100));

        let persisted = ports
            .planning_authority_port
            .load_runtime_pr_validation_record(&workspace_dir, record.key())
            .unwrap()
            .unwrap();
        assert_eq!(persisted.observation_revision(), 1);
        assert_eq!(
            observation.requests.lock().unwrap().len(),
            1,
            "control-plane refreshes must not invoke the validation provider"
        );
        let queue = planning
            .queue
            .load_authority_snapshot(&workspace_dir)
            .expect("normal planning queue should remain readable");
        assert!(
            queue.tasks.is_empty(),
            "observe mode must not admit remediation"
        );
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
        use crate::application::service::planning::{PlanningResetRequest, PlanningResetTarget};

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
            .reset_workspace(
                &workspace_dir,
                PlanningResetRequest::new(PlanningResetTarget::All, true),
            )
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
    fn only_the_entrances_send_a_planning_reset_to_the_repository_kernel() {
        // plan 18 K4-9a: CLI, admin, and Telegram route the reset. A kernel runs it; a routed kernel would
        // send the command back to itself.
        let ports = build_shared_ports_for_prompt_logging(false);
        for caller in [
            kernel_command_execution::CALLER_CLI,
            kernel_command_execution::CALLER_ADMIN,
            kernel_command_execution::CALLER_TELEGRAM,
        ] {
            assert!(
                entrance_planning_services(&ports, caller)
                    .workspace
                    .routes_reset_to_kernel(),
                "{caller}"
            );
        }
        assert!(
            !planning_services_from_ports(&ports)
                .workspace
                .routes_reset_to_kernel()
        );
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

    fn pending_issue_hold_for_startup(
        workspace: &TempWorkspace,
    ) -> (
        String,
        SqliteIssueAutomationStoreAdapter,
        IssueInterventionRequest,
    ) {
        let canonical = repository_kernel_target(workspace.path().to_str().unwrap())
            .expect("the fixture has a canonical repository workspace")
            .workspace_root;
        let store = SqliteIssueAutomationStoreAdapter::new(|workspace| {
            Ok(PathBuf::from(
                SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?
                    .runtime_dir,
            ))
        });
        let issue = IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_owned(),
            repository_id: "R_startup_recovery".to_owned(),
            issue_id: "I_pending_hold".to_owned(),
            project_scope: IssueProjectScope {
                workspace_dir: canonical.clone(),
            },
        };
        let binding = IssueAutomationBinding::new(
            IssueAutomationBindingId::new("startup-recovery-binding").unwrap(),
            issue.provider,
            &issue.server_id,
            &issue.repository_id,
            issue.project_scope.clone(),
        )
        .unwrap();
        store.insert_binding(&canonical, &binding).unwrap();
        let shared_copy = IssueSharedCopy {
            title: "Pending startup hold".to_owned(),
            body: "A committed hold must fence work before the next runtime starts.".to_owned(),
        };
        let tracked = TrackedIssue::discovered(
            issue.clone(),
            &binding,
            IssueAutomationSnapshot {
                basis: IssueComparisonBasis {
                    field_hash: IssueFieldHash::of_fields(&shared_copy.title, &shared_copy.body),
                    provider_updated_at: "2026-11-21T00:00:00Z".to_owned(),
                },
                shared_copy,
                policy_version: "startup-recovery-policy".to_owned(),
            },
        )
        .unwrap();
        store.insert_issue(&canonical, &tracked).unwrap();
        let collection = store.ensure_collection(&canonical).unwrap();
        store
            .set_collection_mode(&canonical, Some(collection.epoch), IssueCollectionMode::Off)
            .unwrap();
        let request = IssueInterventionRequest {
            command_id: "startup-pending-hold".to_owned(),
            issue,
            action: IssueInterventionAction::Hold,
        };
        assert!(
            store
                .begin_issue_intervention(&canonical, &request)
                .unwrap()
                .receipt
                .is_none()
        );
        let control = SqlitePlanningAuthorityAdapter::new()
            .load_work_order_control(&canonical, &issue_work_order_id(&request.issue))
            .unwrap();
        assert!(!control.held);
        assert_eq!(control.authorization_epoch, 0);
        (canonical, store, request)
    }

    fn assert_startup_hold_recovered(
        workspace: &str,
        store: &SqliteIssueAutomationStoreAdapter,
        request: &IssueInterventionRequest,
    ) {
        let control = SqlitePlanningAuthorityAdapter::new()
            .load_work_order_control(workspace, &issue_work_order_id(&request.issue))
            .unwrap();
        assert!(control.held);
        assert!(!control.allows_automation());
        assert!(control.require_epoch(0).is_err());
        assert_eq!(control.authorization_epoch, 1);
        assert!(
            store
                .get_issue(workspace, &request.issue)
                .unwrap()
                .unwrap()
                .flags()
                .held
        );
        let receipt = store
            .get_issue_intervention(workspace, &request.command_id)
            .unwrap()
            .unwrap()
            .receipt
            .unwrap();
        assert_eq!(receipt.authorization_epoch, 1);
        assert!(
            store
                .pending_issue_interventions(workspace)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .get_collection(workspace)
                .unwrap()
                .unwrap()
                .config
                .mode,
            IssueCollectionMode::Off
        );
    }

    #[test]
    fn tui_startup_from_linked_worktree_recovers_the_primary_checkout_pending_hold() {
        let workspace = TempWorkspace::new("tui-issue-recovery-primary");
        initialize_git_workspace(&workspace);
        let linked = TempWorkspace::new("tui-issue-recovery-linked");
        let output = crate::test_utils::git_command()
            .args(["worktree", "add", "--detach"])
            .arg(linked.path())
            .arg("HEAD")
            .current_dir(workspace.path())
            .output()
            .expect("the linked worktree fixture should start");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let (canonical, store, request) = pending_issue_hold_for_startup(&workspace);
        let linked_dir = linked.path().to_str().unwrap();
        assert_ne!(linked_dir, canonical);
        assert!(
            store
                .pending_issue_interventions(linked_dir)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.pending_issue_interventions(&canonical).unwrap().len(),
            1
        );
        assert_eq!(
            repository_kernel_target(linked_dir).unwrap().workspace_root,
            canonical
        );

        // This is the production lock/recovery entry. No runtime or app-server is constructed.
        let lock = acquire_native_tui_kernel_lock_for_workspace(linked_dir)
            .expect("linked-worktree startup recovers its repository's control intents")
            .expect("the fixture kernel lock is free");
        assert_startup_hold_recovered(&canonical, &store, &request);
        drop(lock);
    }

    #[test]
    fn tui_startup_lock_file_failure_requires_pending_hold_recovery_to_succeed() {
        for broken_planning_control in [false, true] {
            let workspace = TempWorkspace::new("tui-issue-recovery-lock-error");
            initialize_git_workspace(&workspace);
            let (canonical, store, request) = pending_issue_hold_for_startup(&workspace);
            let location =
                SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&canonical)
                    .unwrap();
            let lock_path = Path::new(&location.runtime_dir).join("kernel.lock");
            std::fs::create_dir(&lock_path).expect("a directory must block opening kernel.lock");
            if broken_planning_control {
                let connection =
                    rusqlite::Connection::open(&location.authority_store_path).unwrap();
                connection.execute(
                    "INSERT INTO planning_work_order_controls (work_order_id, control_json) VALUES (?1, ?2)",
                    rusqlite::params![issue_work_order_id(&request.issue), "invalid-control-json"],
                ).unwrap();
            }
            let lock_error = acquire_repository_kernel_lock(&canonical)
                .expect_err("the fixture must take the non-contention lock error path");
            assert!(
                lock_error
                    .downcast_ref::<RepositoryKernelLockHeld>()
                    .is_none()
            );
            assert!(format!("{lock_error:#}").contains("kernel.lock"));

            let result = acquire_native_tui_kernel_lock_for_workspace(&canonical);
            if broken_planning_control {
                let error = result.expect_err("a failed planning fence must block runtime startup");
                assert!(
                    format!("{error:#}").contains("invalid work control record"),
                    "{error:#}"
                );
                assert!(
                    store
                        .get_issue_intervention(&canonical, &request.command_id)
                        .unwrap()
                        .unwrap()
                        .receipt
                        .is_none()
                );
                assert_eq!(
                    store.pending_issue_interventions(&canonical).unwrap().len(),
                    1
                );
                assert!(
                    !store
                        .get_issue(&canonical, &request.issue)
                        .unwrap()
                        .unwrap()
                        .flags()
                        .held
                );
            } else {
                assert!(
                    result
                        .expect("a recovered hold permits the existing lock fallback")
                        .is_none()
                );
                assert_startup_hold_recovered(&canonical, &store, &request);
            }
            std::fs::remove_dir(&lock_path).unwrap();
        }
    }

    #[test]
    fn tui_startup_refuses_a_repository_whose_kernel_runs_in_another_process() {
        // plan 18 K3-4: the TUI shows the reason and exits before it opens the screen.
        let workspace = TempWorkspace::new("tui-kernel-lock-held");
        initialize_git_workspace(&workspace);
        let workspace_dir = workspace.path().display().to_string();
        let _kernel = acquire_repository_kernel_lock(&workspace_dir).expect("kernel lock");

        let error = acquire_native_tui_kernel_lock_for_workspace(&workspace_dir)
            .expect_err("the TUI must not start a second kernel for the repository");

        let message = format!("{error:#}");
        assert!(
            message.starts_with("another Akra kernel already runs for repository")
                && message.contains("kernel.lock")
                && message.contains("Close that process and try again"),
            "{message}"
        );
    }

    #[test]
    fn tui_startup_keeps_running_without_the_lock_when_the_lock_file_cannot_open() {
        let workspace = TempWorkspace::new("tui-kernel-lock-broken");
        let workspace_dir = workspace.path().display().to_string();
        let lock_path =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&workspace_dir)
                .map(|location| Path::new(&location.runtime_dir).join("kernel.lock"))
                .expect("runtime directory should be prepared");
        let _ = std::fs::remove_file(&lock_path);
        std::fs::create_dir_all(&lock_path).expect("a directory blocks the lock file");

        let lock = acquire_native_tui_kernel_lock_for_workspace(&workspace_dir)
            .expect("a broken lock file does not stop the TUI");

        assert!(lock.is_none());
        let _ = std::fs::remove_dir(&lock_path);
    }
}

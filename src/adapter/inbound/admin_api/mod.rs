use crate::adapter::outbound::app_server::{AppServerPlanningWorkerAdapter, CodexAppServerAdapter};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::github::GithubAutomationAdapter;
use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_worker_port::PlanningWorkerPort;
use crate::application::service::parallel_mode::{
    ParallelModeService, control_plane::ParallelModeControlPlaneComposition,
};
use crate::application::service::planning::{
    PlanningAdminFacadeService, PlanningResetTarget, PlanningServices,
};
use anyhow::{Context, Result, anyhow, bail};
use axum::Router;
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use std::net::Ipv4Addr;
use std::sync::Arc;

/*
 * admin_api는 planning administration을 로컬 HTTP surface로 노출하는 inbound adapter다.
 * loopback bind, CLI server argument, route table, CSRF boundary, HTML/JSON handler wiring은 이
 * 모듈의 transport 책임이다. 반대로 queue/direction/task/draft의 의미, workspace mutation policy,
 * authority-store write rule은 PlanningAdminFacadeService 아래 application layer에 남긴다.
 * 그래서 이 파일은 "어떤 URL이 어떤 transport contract로 facade를 호출하는가"만 설명하고,
 * planning 자체의 판정은 직접 복제하지 않는다.
 */
mod akra_dashboard;
mod api;
mod forms;
mod helpers;
mod pages;
mod static_assets;
#[cfg(test)]
mod tests;
mod views;

use self::helpers::{ensure_csrf_cookie, internal_server_error, verify_header_csrf};

const DEFAULT_PORT: u16 = 18442;
const ADMIN_CHARACTER_SPRITES: &[u8] =
    include_bytes!("../../../../assets/admin/admin-character-sprites.svg");

#[derive(Clone)]
struct AdminAppState {
    /*
     * Axum은 handler마다 state를 clone한다.
     * 여기에는 Arc facade만 두어 HTTP layer가 별도 planning cache나 mutation policy를 갖지 못하게 한다.
     * HTML page handler와 JSON API handler가 같은 facade instance를 바라보므로 두 surface의 상태 해석도 함께 묶인다.
     */
    facade: Arc<PlanningAdminFacadeService>,
    parallel_mode_control_plane: Arc<ParallelModeControlPlaneComposition>,
    graphic: AdminGraphicConfig,
}

#[derive(Clone)]
struct AdminGraphicConfig {
    enabled: bool,
    api_base_url: String,
    polling_interval_ms: u64,
}

#[derive(Debug, Default)]
struct AdminServerArgs {
    port: u16,
}

pub async fn run_from_env() -> Result<()> {
    run_with_args(std::env::args().skip(1)).await
}

pub async fn run_with_args<I>(args: I) -> Result<()>
where
    I: IntoIterator<Item = String>,
{
    let args = parse_args(args)?;

    /*
     * admin surface는 의도적으로 현재 workspace에 묶인다.
     * outbound port를 만들기 전에 cwd를 canonicalize하면 symlink로 실행된 경우에도 page/API mutation이 같은
     * repository identity를 기준으로 planning file과 sqlite authority를 해석한다.
     * 이 값이 facade의 workspace_dir로 들어가므로, 이후 handler는 request마다 cwd를 다시 읽지 않는다.
     */
    let workspace_dir = std::env::current_dir()
        .context("failed to resolve current directory for admin server")?
        .canonicalize()
        .context("failed to canonicalize current directory for admin server")?;
    let workspace_dir = workspace_dir.display().to_string();
    let state = build_admin_state(workspace_dir);
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, args.port))
        .await
        .with_context(|| format!("failed to bind admin server on 127.0.0.1:{}", args.port))?;

    println!(
        "local planning admin server listening on http://127.0.0.1:{}",
        args.port
    );

    axum::serve(listener, build_router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("admin server exited unexpectedly")?;
    Ok(())
}

fn build_admin_state(workspace_dir: String) -> AdminAppState {
    /*
     * standalone admin server의 composition root다.
     * app-server worker, sqlite planning authority, filesystem workspace adapter를 여기서 조립해
     * PlanningServices와 PlanningAdminFacadeService에 주입한다. browser page와 JSON API는 이 결과 facade만 공유하므로
     * queue, direction, draft state를 서로 다른 adapter instance에서 따로 읽는 drift가 생기지 않는다.
     *
     * FilesystemPlanningWorkspaceAdapter는 repo-scoped store를 함께 받는다.
     * active planning authority가 git worktree 외부 integration checkout에 있을 수 있기 때문에, admin server의 파일 작업도
     * candidate workspace와 authoritative store를 facade 규칙에 맞춰 구분해야 한다.
     */
    let app_server_adapter = Arc::new(CodexAppServerAdapter::new(
        "codex-exec-loop-native",
        env!("CARGO_PKG_VERSION"),
    ));
    let sqlite_planning_authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning_authority: Arc<dyn PlanningAuthorityPort> = sqlite_planning_authority.clone();
    let planning_task_repository: Arc<dyn PlanningTaskRepositoryPort> =
        sqlite_planning_authority.clone();
    let planning_workspace_port =
        Arc::new(FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(
            sqlite_planning_authority.clone(),
        ));
    let planning_worker_port: Arc<dyn PlanningWorkerPort> = Arc::new(
        AppServerPlanningWorkerAdapter::new(app_server_adapter.clone()),
    );
    let parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort> = app_server_adapter;
    let planning = PlanningServices::from_ports(
        planning_workspace_port.clone(),
        planning_authority.clone(),
        planning_task_repository.clone(),
        planning_worker_port,
    );
    let github_automation: Arc<dyn GithubAutomationPort> = Arc::new(GithubAutomationAdapter::new());
    let parallel_mode = ParallelModeService::new(
        planning_authority.clone(),
        github_automation,
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    let parallel_mode_control_plane = Arc::new(ParallelModeControlPlaneComposition::new(
        parallel_mode,
        planning.clone(),
        parallel_agent_worker_port,
    ));
    let facade = Arc::new(PlanningAdminFacadeService::from_planning_with_authority(
        workspace_dir.clone(),
        planning.clone(),
        planning_workspace_port,
        planning_authority.clone(),
        planning_task_repository,
    ));
    AdminAppState {
        facade,
        parallel_mode_control_plane,
        graphic: AdminGraphicConfig::from_env(),
    }
}

impl AdminGraphicConfig {
    fn from_env() -> Self {
        let enabled = std::env::var("AKRA_ADMIN_GRAPHIC_ENABLED")
            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let api_base_url = std::env::var("AKRA_ADMIN_API_BASE_URL").unwrap_or_default();
        let polling_interval_ms = std::env::var("AKRA_ADMIN_GRAPHIC_POLL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value >= 5_000)
            .unwrap_or(10_000);
        Self {
            enabled,
            api_base_url,
            polling_interval_ms,
        }
    }
}

fn build_router(state: AdminAppState) -> Router {
    /*
     * browser route와 API route를 하나의 table에 둔다.
     * 두 surface는 같은 planning operation을 노출하지만 transport contract가 다르다.
     * pages.rs는 form field, redirect, HTMX fragment를 다루고 api.rs는 JSON body와 x-csrf-token header를 다룬다.
     * route registration을 한곳에 모으면 새 operation을 추가할 때 HTML/JSON 양쪽 노출 여부를 같은 diff에서 검토할 수 있다.
     */
    Router::new()
        .route("/", get(pages::dashboard_page))
        .route("/admin", get(pages::dashboard_page))
        .route("/admin/akra", get(pages::akra_dashboard_page))
        .route(
            "/admin/assets/graphics/{asset_name}",
            get(static_assets::admin_graphic_asset),
        )
        .route("/admin/directions", get(pages::directions_page))
        .route("/admin/tasks", get(pages::tasks_page))
        .route("/admin/controls", get(pages::controls_page))
        .route("/admin/drafts", post(pages::create_draft_page))
        .route(
            "/admin/directions/upsert",
            post(pages::upsert_direction_page),
        )
        .route(
            "/admin/directions/delete",
            post(pages::delete_direction_page),
        )
        .route("/admin/tasks/upsert", post(pages::upsert_task_page))
        .route("/admin/tasks/delete", post(pages::delete_task_page))
        .route("/admin/files/export", post(pages::export_files_page))
        .route("/admin/files/apply", post(pages::apply_files_page))
        .route(
            "/admin/controls/parallel-persona",
            post(pages::update_parallel_persona_page),
        )
        .route("/admin/drafts/{draft_name}", get(pages::editor_page))
        .route(
            "/admin/drafts/{draft_name}/save",
            post(pages::save_draft_page),
        )
        .route(
            "/admin/drafts/{draft_name}/validate",
            post(pages::validate_draft_page),
        )
        .route(
            "/admin/drafts/{draft_name}/promote",
            post(pages::promote_draft_page),
        )
        .route("/admin/controls/reset", post(pages::reset_page))
        .route(
            "/assets/admin/admin-character-sprites.svg",
            get(admin_character_sprites_asset),
        )
        .route("/api/planning/summary", get(api::summary_api))
        .route("/api/planning/runtime", get(api::runtime_api))
        .route("/api/planning/drafts", post(api::create_draft_api))
        .route(
            "/api/planning/drafts/{draft_name}",
            get(api::load_draft_api).put(api::save_draft_api),
        )
        .route(
            "/api/planning/drafts/{draft_name}/validate",
            post(api::validate_draft_api),
        )
        .route(
            "/api/planning/drafts/{draft_name}/promote",
            post(api::promote_draft_api),
        )
        .route("/api/planning/directions", post(api::upsert_direction_api))
        .route(
            "/api/planning/directions/delete",
            post(api::delete_direction_api),
        )
        .route("/api/planning/tasks", post(api::upsert_task_api))
        .route("/api/planning/tasks/delete", post(api::delete_task_api))
        .route("/api/planning/files/export", post(api::export_files_api))
        .route("/api/planning/files/apply", post(api::apply_files_api))
        .route("/api/planning/reset", post(api::reset_api))
        .route("/api/admin/akra/dashboard", get(api::akra_dashboard_api))
        .route("/api/admin/akra/pool", get(api::akra_pool_api))
        .route("/api/admin/akra/agents", get(api::akra_agents_api))
        .route(
            "/api/admin/akra/distributor",
            get(api::akra_distributor_api),
        )
        .route("/api/admin/akra/events", get(api::akra_events_api))
        .with_state(state)
}

async fn admin_character_sprites_asset() -> Response {
    (
        [
            (CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (CACHE_CONTROL, "public, max-age=3600"),
        ],
        ADMIN_CHARACTER_SPRITES,
    )
        .into_response()
}

fn parse_reset_target(target: &str) -> std::result::Result<PlanningResetTarget, StatusCode> {
    // HTML form과 JSON caller가 reset vocabulary를 공유해 queue/directions/all 의미가 route별로 갈라지지 않게 한다.
    match target.trim().to_ascii_lowercase().as_str() {
        "queue" => Ok(PlanningResetTarget::Queue),
        "directions" => Ok(PlanningResetTarget::Directions),
        "all" => Ok(PlanningResetTarget::All),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

fn parse_args<I>(args: I) -> Result<AdminServerArgs>
where
    I: IntoIterator<Item = String>,
{
    /*
     * admin server argument parsing은 이 debug/admin surface 안에 둔다.
     * 메인 CLI parser와 결합하면 실험적 admin-only flag가 일반 실행 경로의 contract처럼 굳어질 수 있으므로,
     * 여기서는 port와 help만 받아 standalone server bootstrap에 필요한 최소 surface를 유지한다.
     */
    let mut parsed = AdminServerArgs { port: DEFAULT_PORT };
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("--port requires a value"))?;
                parsed.port = value
                    .parse::<u16>()
                    .with_context(|| format!("invalid port: {value}"))?;
            }
            "-h" | "--help" => {
                println!("Usage: akra admin [--port <port>]");
                println!("Alias: akra admin-server [--port <port>]");
                std::process::exit(0);
            }
            _ => bail!("unsupported argument: {arg}"),
        }
    }
    Ok(parsed)
}

async fn shutdown_signal() {
    // local-only admin server는 Ctrl-C를 유일한 shutdown signal로 삼고, in-flight drain은 axum serve layer에 맡긴다.
    let _ = tokio::signal::ctrl_c().await;
}

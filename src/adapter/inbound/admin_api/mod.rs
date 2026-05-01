use std::net::Ipv4Addr;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use axum::Router;
use axum::http::StatusCode;
use axum::routing::{get, post};

use crate::adapter::outbound::app_server::{AppServerPlanningWorkerAdapter, CodexAppServerAdapter};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::service::planning::{
    PlanningAdminFacadeService, PlanningResetTarget, PlanningServices,
};

mod api;
mod forms;
mod helpers;
mod pages;
#[cfg(test)]
mod tests;
mod views;
use self::helpers::{ensure_csrf_cookie, internal_server_error, verify_header_csrf};

const DEFAULT_PORT: u16 = 18442;

#[derive(Clone)]
struct AdminAppState {
    facade: Arc<PlanningAdminFacadeService>,
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
    let workspace_dir = std::env::current_dir()
        .context("failed to resolve current directory for admin server")?
        .canonicalize()
        .context("failed to canonicalize current directory for admin server")?;
    let workspace_dir = workspace_dir.display().to_string();
    let facade = Arc::new(build_admin_facade(workspace_dir));
    let state = AdminAppState { facade };
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

fn build_admin_facade(workspace_dir: String) -> PlanningAdminFacadeService {
    let app_server_adapter = Arc::new(CodexAppServerAdapter::new(
        "codex-exec-loop-native",
        env!("CARGO_PKG_VERSION"),
    ));
    let planning_authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning_workspace_port = Arc::new(
        FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(planning_authority.clone()),
    );
    let planning = PlanningServices::from_ports(
        planning_workspace_port.clone(),
        planning_authority.clone(),
        planning_authority.clone(),
        Arc::new(AppServerPlanningWorkerAdapter::new(app_server_adapter)),
    );
    PlanningAdminFacadeService::from_planning_with_authority(
        workspace_dir,
        planning,
        planning_workspace_port,
        planning_authority.clone(),
        planning_authority,
    )
}

fn build_router(state: AdminAppState) -> Router {
    Router::new()
        .route("/", get(pages::dashboard_page))
        .route("/admin", get(pages::dashboard_page))
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
        .with_state(state)
}

fn parse_reset_target(target: &str) -> std::result::Result<PlanningResetTarget, StatusCode> {
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
    let _ = tokio::signal::ctrl_c().await;
}

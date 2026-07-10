use crate::application::port::outbound::app_server_prompt_log_port::AppServerPromptLogPort;
use crate::application::service::parallel_agent_profile::ParallelAgentProfileService;
use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneComposition;
use crate::application::service::planning::{PlanningAdminFacadeService, PlanningResetTarget};
use crate::application::service::review_center::ReviewCenterReadService;
use crate::composition::production;
use anyhow::{Context, Result, anyhow, bail};
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum_extra::extract::CookieJar;
use std::io::IsTerminal;
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
mod security;
mod static_assets;
#[cfg(test)]
mod tests;
mod views;

use self::helpers::{
    ensure_csrf_cookie, internal_server_error, verify_draft_name_path, verify_header_csrf,
};
use self::security::{AdminSecurityBootstrap, AdminSecurityConfig, verify_local_admin_request};

const DEFAULT_PORT: u16 = 18442;
#[derive(Clone)]
struct AdminAppState {
    /*
     * Axum은 handler마다 state를 clone한다.
     * 여기에는 Arc facade만 두어 HTTP layer가 별도 planning cache나 mutation policy를 갖지 못하게 한다.
     * HTML page handler와 JSON API handler가 같은 facade instance를 바라보므로 두 surface의 상태 해석도 함께 묶인다.
     */
    facade: Arc<PlanningAdminFacadeService>,
    parallel_mode_control_plane: Arc<ParallelModeControlPlaneComposition>,
    parallel_agent_profile_service: ParallelAgentProfileService,
    app_server_prompt_log_port: Arc<dyn AppServerPromptLogPort>,
    review_center_read_service: ReviewCenterReadService,
    graphic: AdminGraphicConfig,
    security: AdminSecurityConfig,
}

#[derive(Clone)]
struct AdminGraphicConfig {
    enabled: bool,
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
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, args.port))
        .await
        .with_context(|| format!("failed to bind admin server on 127.0.0.1:{}", args.port))?;

    let bound_port = listener
        .local_addr()
        .context("failed to read admin server local address")?
        .port();
    let security = AdminSecurityBootstrap::from_env(bound_port)?;
    let stdout_is_terminal = std::io::stdout().is_terminal();
    let stderr_is_terminal = std::io::stderr().is_terminal();
    if security.generated_capability_token.is_some() && !stdout_is_terminal && !stderr_is_terminal {
        bail!(
            "AKRA_ADMIN_TOKEN is required when the admin server is not attached to an interactive terminal"
        );
    }
    let public_origin = security.config.public_origin();
    let workspace_dir = workspace_dir.display().to_string();
    let state = build_admin_state(workspace_dir, security.config);
    println!("local planning admin server listening on {public_origin}");
    if let Some(token) = security.generated_capability_token {
        if stdout_is_terminal {
            println!("admin capability token (keep private): {token}");
        } else {
            eprintln!("admin capability token (keep private): {token}");
        }
    }
    if stdout_is_terminal || !stderr_is_terminal {
        println!("admin login: {public_origin}/admin/login");
    } else {
        eprintln!("admin login: {public_origin}/admin/login");
    }

    axum::serve(listener, build_router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("admin server exited unexpectedly")?;
    Ok(())
}

fn build_admin_state(workspace_dir: String, security: AdminSecurityConfig) -> AdminAppState {
    /*
     * Admin HTTP layer는 route와 transport contract만 소유한다.
     * app-server, sqlite authority, filesystem workspace, Git/GitHub runtime wiring은
     * production composition root에서 같은 graph로 받아 page/API handler가 동일 facade를 공유하게 한다.
     */
    let application = production::build_admin_application(workspace_dir);
    AdminAppState {
        facade: application.facade,
        parallel_mode_control_plane: application.parallel_mode_control_plane,
        parallel_agent_profile_service: application.parallel_agent_profile_service,
        app_server_prompt_log_port: application.app_server_prompt_log_port,
        review_center_read_service: application.review_center_read_service,
        graphic: AdminGraphicConfig::from_env(),
        security,
    }
}

impl AdminGraphicConfig {
    fn from_env() -> Self {
        let enabled = std::env::var("AKRA_ADMIN_GRAPHIC_ENABLED")
            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let polling_interval_ms = std::env::var("AKRA_ADMIN_GRAPHIC_POLL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value >= 5_000)
            .unwrap_or(10_000);
        Self {
            enabled,
            polling_interval_ms,
        }
    }
}

async fn local_admin_request_guard(
    State(state): State<AdminAppState>,
    request: Request,
    next: Next,
) -> Response {
    if let Err(status) = verify_local_admin_request(request.headers(), &state.security) {
        return harden_admin_response(status.into_response());
    }

    let path = request.uri().path();
    let login_route = path == "/admin/login";
    let authentication = (!login_route)
        .then(|| state.security.authenticate_request(request.headers()))
        .flatten();
    if !login_route && authentication.is_none() {
        return harden_admin_response(if path.starts_with("/api/") {
            StatusCode::UNAUTHORIZED.into_response()
        } else {
            Redirect::to("/admin/login").into_response()
        });
    }

    let refresh_cookie = (path != "/admin/logout")
        .then(|| authentication.and_then(|auth| auth.refreshed_session_cookie()))
        .flatten();
    let mut response = next.run(request).await;
    if let Some(cookie) = refresh_cookie {
        response = (CookieJar::new().add(cookie), response).into_response();
    }
    harden_admin_response(response)
}

fn harden_admin_response(mut response: Response) -> Response {
    response.headers_mut().insert(
        "cache-control",
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response.headers_mut().insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; base-uri 'none'; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; form-action 'self'; img-src 'self' data:; object-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; worker-src 'self' blob:",
        ),
    );
    response
        .headers_mut()
        .entry("referrer-policy")
        .or_insert(HeaderValue::from_static("no-referrer"));
    response.headers_mut().insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    response.headers_mut().insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    response.headers_mut().insert(
        "permissions-policy",
        HeaderValue::from_static(
            "camera=(), display-capture=(), geolocation=(), microphone=(), payment=(), usb=()",
        ),
    );
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert("x-frame-options", HeaderValue::from_static("DENY"));
    response
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
        .route(
            "/admin/login",
            get(security::login_page).post(security::login_submit),
        )
        .route("/admin/logout", post(security::logout_submit))
        .route("/admin", get(pages::dashboard_page))
        .route("/admin/akra", get(pages::akra_dashboard_page))
        .route("/admin/akra/metrics", get(pages::akra_metrics_page))
        .route("/admin/akra/directions", get(pages::akra_directions_page))
        .route("/admin/akra/tasks", get(pages::akra_tasks_page))
        .route(
            "/admin/assets/graphics/{asset_name}",
            get(static_assets::admin_graphic_asset),
        )
        .route(
            "/admin/assets/game/{asset_name}",
            get(static_assets::admin_game_asset),
        )
        .route(
            "/admin/assets/scripts/{asset_name}",
            get(static_assets::admin_script_asset),
        )
        .route(
            "/admin/assets/fonts/{asset_name}",
            get(static_assets::admin_font_asset),
        )
        .route("/admin/directions", get(pages::directions_page))
        .route("/admin/tasks", get(pages::tasks_page))
        .route("/admin/reviews", get(pages::reviews_page))
        .route("/admin/controls", get(pages::controls_page))
        .route(
            "/admin/app-server-prompts",
            get(pages::app_server_prompts_page),
        )
        .route("/admin/drafts", post(pages::create_draft_page))
        .route(
            "/admin/directions/upsert",
            post(pages::upsert_direction_page),
        )
        .route(
            "/admin/directions/delete",
            post(pages::delete_direction_page),
        )
        .route(
            "/admin/akra/directions/upsert",
            post(pages::upsert_akra_direction_page),
        )
        .route(
            "/admin/akra/directions/delete",
            post(pages::delete_akra_direction_page),
        )
        .route("/admin/tasks/upsert", post(pages::upsert_task_page))
        .route("/admin/tasks/delete", post(pages::delete_task_page))
        .route(
            "/admin/akra/tasks/upsert",
            post(pages::upsert_akra_task_page),
        )
        .route(
            "/admin/akra/tasks/delete",
            post(pages::delete_akra_task_page),
        )
        .route("/admin/files/export", post(pages::export_files_page))
        .route("/admin/files/apply", post(pages::apply_files_page))
        .route(
            "/admin/controls/agent-profiles",
            post(pages::update_agent_profiles_page),
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
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state,
            local_admin_request_guard,
        ))
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

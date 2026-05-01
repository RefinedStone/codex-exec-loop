use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum_extra::extract::CookieJar;

use crate::adapter::outbound::app_server::{AppServerPlanningWorkerAdapter, CodexAppServerAdapter};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::service::planning::{
    PlanningAdminDirectionDeleteRequest, PlanningAdminDirectionMutationRequest,
    PlanningAdminDraftFileUpdate, PlanningAdminDraftKind, PlanningAdminDraftLoadRequest,
    PlanningAdminDraftMutationRequest, PlanningAdminFacadeService, PlanningAdminFileKey,
    PlanningAdminSessionView, PlanningAdminTaskDeleteRequest, PlanningAdminTaskMutationRequest,
    PlanningResetTarget, PlanningServices,
};

mod api;
mod forms;
mod helpers;
#[cfg(test)]
mod tests;
mod views;

use self::forms::{
    CreateDraftForm, DirectionMutationForm, DraftMutationForm, EditorQuery, FileSyncForm,
    IdDeleteForm, ResetForm, TaskMutationForm,
};
use self::helpers::{
    encode_uri_component, ensure_csrf_cookie, internal_server_error, is_htmx_request,
    notice_location, render_fragment, render_html, verify_form_csrf, verify_header_csrf,
};
use self::views::{
    ControlsTemplate, DashboardTemplate, DirectionsTemplate, DraftStatusTemplate, EditorTemplate,
    TasksTemplate,
};

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
        .route("/", get(dashboard_page))
        .route("/admin", get(dashboard_page))
        .route("/admin/directions", get(directions_page))
        .route("/admin/tasks", get(tasks_page))
        .route("/admin/controls", get(controls_page))
        .route("/admin/drafts", post(create_draft_page))
        .route("/admin/directions/upsert", post(upsert_direction_page))
        .route("/admin/directions/delete", post(delete_direction_page))
        .route("/admin/tasks/upsert", post(upsert_task_page))
        .route("/admin/tasks/delete", post(delete_task_page))
        .route("/admin/files/export", post(export_files_page))
        .route("/admin/files/apply", post(apply_files_page))
        .route("/admin/drafts/{draft_name}", get(editor_page))
        .route("/admin/drafts/{draft_name}/save", post(save_draft_page))
        .route(
            "/admin/drafts/{draft_name}/validate",
            post(validate_draft_page),
        )
        .route(
            "/admin/drafts/{draft_name}/promote",
            post(promote_draft_page),
        )
        .route("/admin/controls/reset", post(reset_page))
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

async fn dashboard_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    let (jar, csrf_token) = ensure_csrf_cookie(jar);
    let overview = state
        .facade
        .load_overview()
        .map_err(internal_server_error)?;
    render_html(
        jar,
        DashboardTemplate {
            page_title: "Planning Admin".to_string(),
            current_nav: "dashboard",
            workspace_dir: state.facade.workspace_dir().to_string(),
            csrf_token,
            notice: query.get("notice").cloned(),
            overview,
        },
    )
}

async fn directions_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    let (jar, csrf_token) = ensure_csrf_cookie(jar);
    let overview = state
        .facade
        .load_overview()
        .map_err(internal_server_error)?;
    let management = state
        .facade
        .load_management_view()
        .map_err(internal_server_error)?;
    render_html(
        jar,
        DirectionsTemplate {
            page_title: "Directions".to_string(),
            current_nav: "directions",
            workspace_dir: state.facade.workspace_dir().to_string(),
            csrf_token,
            notice: query.get("notice").cloned(),
            overview,
            management,
        },
    )
}

async fn tasks_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    let (jar, csrf_token) = ensure_csrf_cookie(jar);
    let overview = state
        .facade
        .load_overview()
        .map_err(internal_server_error)?;
    let management = state
        .facade
        .load_management_view()
        .map_err(internal_server_error)?;
    render_html(
        jar,
        TasksTemplate {
            page_title: "Tasks".to_string(),
            current_nav: "tasks",
            workspace_dir: state.facade.workspace_dir().to_string(),
            csrf_token,
            notice: query.get("notice").cloned(),
            overview,
            management,
        },
    )
}

async fn upsert_direction_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<DirectionMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    verify_form_csrf(&jar, &form.csrf_token)?;
    let outcome = state
        .facade
        .upsert_direction(PlanningAdminDirectionMutationRequest {
            id: form.id,
            title: form.title,
            summary: form.summary,
            success_criteria_text: form.success_criteria_text,
            scope_hints_text: form.scope_hints_text,
            detail_doc_path: form.detail_doc_path,
            state: form.state,
        })
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&notice_location("/admin/directions", &outcome.notice)).into_response())
}

async fn delete_direction_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<IdDeleteForm>,
) -> std::result::Result<Response, StatusCode> {
    verify_form_csrf(&jar, &form.csrf_token)?;
    let outcome = state
        .facade
        .delete_direction(PlanningAdminDirectionDeleteRequest { id: form.id })
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&notice_location("/admin/directions", &outcome.notice)).into_response())
}

async fn upsert_task_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<TaskMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    verify_form_csrf(&jar, &form.csrf_token)?;
    let outcome = state
        .facade
        .upsert_task(PlanningAdminTaskMutationRequest {
            id: form.id,
            direction_id: form.direction_id,
            title: form.title,
            description: form.description,
            status: form.status,
            base_priority: form.base_priority,
            dynamic_priority_delta: form.dynamic_priority_delta,
            priority_reason: form.priority_reason,
            depends_on_text: form.depends_on_text,
            blocked_by_text: form.blocked_by_text,
        })
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&notice_location("/admin/tasks", &outcome.notice)).into_response())
}

async fn delete_task_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<IdDeleteForm>,
) -> std::result::Result<Response, StatusCode> {
    verify_form_csrf(&jar, &form.csrf_token)?;
    let outcome = state
        .facade
        .delete_task(PlanningAdminTaskDeleteRequest { id: form.id })
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&notice_location("/admin/tasks", &outcome.notice)).into_response())
}

async fn export_files_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<FileSyncForm>,
) -> std::result::Result<Response, StatusCode> {
    verify_form_csrf(&jar, &form.csrf_token)?;
    let outcome = state
        .facade
        .export_active_files_for_edit()
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&notice_location("/admin/controls", &outcome.notice)).into_response())
}

async fn apply_files_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<FileSyncForm>,
) -> std::result::Result<Response, StatusCode> {
    verify_form_csrf(&jar, &form.csrf_token)?;
    let outcome = state
        .facade
        .apply_exported_files()
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&notice_location("/admin/controls", &outcome.notice)).into_response())
}

async fn controls_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    let (jar, csrf_token) = ensure_csrf_cookie(jar);
    let overview = state
        .facade
        .load_overview()
        .map_err(internal_server_error)?;
    render_html(
        jar,
        ControlsTemplate {
            page_title: "Controls".to_string(),
            current_nav: "controls",
            workspace_dir: state.facade.workspace_dir().to_string(),
            csrf_token,
            notice: query.get("notice").cloned(),
            overview,
        },
    )
}

async fn editor_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Path(draft_name): Path<String>,
    Query(query): Query<EditorQuery>,
) -> std::result::Result<Response, StatusCode> {
    let (jar, csrf_token) = ensure_csrf_cookie(jar);
    let session = state
        .facade
        .load_draft_session(PlanningAdminDraftLoadRequest {
            draft_name,
            kind: query.kind,
            direction_id: query.direction_id,
        })
        .map_err(internal_server_error)?;
    render_editor_page(
        jar,
        state.facade.workspace_dir(),
        csrf_token,
        query.notice,
        session,
    )
}

async fn create_draft_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<CreateDraftForm>,
) -> std::result::Result<Response, StatusCode> {
    verify_form_csrf(&jar, &form.csrf_token)?;
    let session = state
        .facade
        .create_draft_session(form.kind, form.direction_id.as_deref())
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&draft_editor_location(
        &session.draft_name,
        session.kind,
        session.direction_id.as_deref(),
        None,
    ))
    .into_response())
}

async fn save_draft_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(draft_name): Path<String>,
    Form(form): Form<DraftMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    verify_form_csrf(&jar, &form.csrf_token)?;
    let csrf_token = form.csrf_token.clone();
    let (_, session) = state
        .facade
        .save_draft(page_mutation_request(draft_name, form))
        .map_err(internal_server_error)?;
    if is_htmx_request(&headers) {
        return render_fragment(DraftStatusTemplate {
            notice: Some("draft saved".to_string()),
            session,
        });
    }
    render_editor_page(
        jar,
        state.facade.workspace_dir(),
        csrf_token,
        Some("draft saved".to_string()),
        session,
    )
}

async fn validate_draft_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(draft_name): Path<String>,
    Form(form): Form<DraftMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    verify_form_csrf(&jar, &form.csrf_token)?;
    let csrf_token = form.csrf_token.clone();
    let (_, session) = state
        .facade
        .save_draft(page_mutation_request(draft_name, form))
        .map_err(internal_server_error)?;
    if is_htmx_request(&headers) {
        return render_fragment(DraftStatusTemplate {
            notice: Some("draft validated".to_string()),
            session,
        });
    }
    render_editor_page(
        jar,
        state.facade.workspace_dir(),
        csrf_token,
        Some("draft validated".to_string()),
        session,
    )
}

async fn promote_draft_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Path(draft_name): Path<String>,
    Form(form): Form<DraftMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    verify_form_csrf(&jar, &form.csrf_token)?;
    let csrf_token = form.csrf_token.clone();
    let (result, session) = state
        .facade
        .promote_draft(page_mutation_request(draft_name, form))
        .map_err(internal_server_error)?;
    let notice = if result.promoted_file_count > 0 && result.validation_report.is_valid() {
        Some(format!(
            "draft promoted into active planning ({} files)",
            result.promoted_file_count
        ))
    } else {
        Some("draft promotion blocked by validation".to_string())
    };
    render_editor_page(
        jar,
        state.facade.workspace_dir(),
        csrf_token,
        notice,
        session,
    )
}

async fn reset_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<ResetForm>,
) -> std::result::Result<Response, StatusCode> {
    verify_form_csrf(&jar, &form.csrf_token)?;
    let target = parse_reset_target(&form.target)?;
    state
        .facade
        .reset_workspace(target)
        .map_err(internal_server_error)?;
    Ok(Redirect::to("/admin/controls?notice=planning%20workspace%20reset").into_response())
}

fn page_mutation_request(
    draft_name: String,
    form: DraftMutationForm,
) -> PlanningAdminDraftMutationRequest {
    PlanningAdminDraftMutationRequest {
        draft_name,
        kind: form.kind,
        direction_id: form.direction_id,
        files: extract_file_updates(form.values),
    }
}

fn extract_file_updates(values: HashMap<String, String>) -> Vec<PlanningAdminDraftFileUpdate> {
    values
        .into_iter()
        .filter_map(|(field_name, body)| {
            let raw_key = field_name.strip_prefix("file_")?;
            let key = match raw_key {
                "result_output" => PlanningAdminFileKey::ResultOutput,
                "queue_idle_prompt" => PlanningAdminFileKey::QueueIdlePrompt,
                "direction_detail" => PlanningAdminFileKey::DirectionDetail,
                _ => return None,
            };
            Some(PlanningAdminDraftFileUpdate { key, body })
        })
        .collect()
}

fn parse_reset_target(target: &str) -> std::result::Result<PlanningResetTarget, StatusCode> {
    match target.trim().to_ascii_lowercase().as_str() {
        "queue" => Ok(PlanningResetTarget::Queue),
        "directions" => Ok(PlanningResetTarget::Directions),
        "all" => Ok(PlanningResetTarget::All),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

fn nav_for_kind(kind: PlanningAdminDraftKind) -> &'static str {
    match kind {
        PlanningAdminDraftKind::QueueIdlePrompt | PlanningAdminDraftKind::DirectionDetail => {
            "directions"
        }
        PlanningAdminDraftKind::FullPlanning => "dashboard",
    }
}

fn draft_editor_location(
    draft_name: &str,
    kind: PlanningAdminDraftKind,
    direction_id: Option<&str>,
    notice: Option<&str>,
) -> String {
    let mut location = format!(
        "/admin/drafts/{}?kind={}",
        encode_uri_component(draft_name),
        kind.slug()
    );
    if let Some(direction_id) = direction_id {
        location.push_str("&direction_id=");
        location.push_str(&encode_uri_component(direction_id));
    }
    if let Some(notice) = notice {
        location.push_str("&notice=");
        location.push_str(&encode_uri_component(notice));
    }
    location
}

fn render_editor_page(
    jar: CookieJar,
    workspace_dir: &str,
    csrf_token: String,
    notice: Option<String>,
    session: PlanningAdminSessionView,
) -> std::result::Result<Response, StatusCode> {
    render_html(
        jar,
        EditorTemplate {
            page_title: session.editor_heading.clone(),
            current_nav: nav_for_kind(session.kind),
            workspace_dir: workspace_dir.to_string(),
            csrf_token,
            notice,
            session,
        },
    )
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

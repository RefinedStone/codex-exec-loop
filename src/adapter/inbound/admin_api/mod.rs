use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use askama::Template;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::adapter::outbound::app_server::{AppServerPlanningWorkerAdapter, CodexAppServerAdapter};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::service::planning::{
    PlanningAdminDirectionDeleteRequest, PlanningAdminDirectionMutationRequest,
    PlanningAdminDraftFileUpdate, PlanningAdminDraftKind, PlanningAdminDraftLoadRequest,
    PlanningAdminDraftMutationRequest, PlanningAdminFacadeService, PlanningAdminFileKey,
    PlanningAdminManagementView, PlanningAdminOverview, PlanningAdminSessionView,
    PlanningAdminTaskDeleteRequest, PlanningAdminTaskMutationRequest, PlanningResetTarget,
    PlanningServices,
};

const DEFAULT_PORT: u16 = 18442;
const CSRF_COOKIE_NAME: &str = "akra_admin_csrf";

#[derive(Clone)]
struct AdminAppState {
    facade: Arc<PlanningAdminFacadeService>,
}

#[derive(Debug, Default)]
struct AdminServerArgs {
    port: u16,
}

#[derive(Debug, Clone, Deserialize)]
struct EditorQuery {
    kind: PlanningAdminDraftKind,
    #[serde(default)]
    direction_id: Option<String>,
    #[serde(default)]
    notice: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CreateDraftForm {
    csrf_token: String,
    kind: PlanningAdminDraftKind,
    #[serde(default)]
    direction_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DraftMutationForm {
    csrf_token: String,
    kind: PlanningAdminDraftKind,
    #[serde(default)]
    direction_id: Option<String>,
    #[serde(flatten)]
    values: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ResetForm {
    csrf_token: String,
    target: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DirectionMutationForm {
    csrf_token: String,
    id: String,
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    success_criteria_text: String,
    #[serde(default)]
    scope_hints_text: String,
    #[serde(default)]
    detail_doc_path: String,
    #[serde(default)]
    state: String,
}

#[derive(Debug, Clone, Deserialize)]
struct IdDeleteForm {
    csrf_token: String,
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TaskMutationForm {
    csrf_token: String,
    id: String,
    #[serde(default)]
    direction_id: String,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    base_priority: String,
    #[serde(default)]
    dynamic_priority_delta: String,
    #[serde(default)]
    priority_reason: String,
    #[serde(default)]
    depends_on_text: String,
    #[serde(default)]
    blocked_by_text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FileSyncForm {
    csrf_token: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CreateDraftRequest {
    kind: PlanningAdminDraftKind,
    #[serde(default)]
    direction_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SaveDraftRequest {
    kind: PlanningAdminDraftKind,
    #[serde(default)]
    direction_id: Option<String>,
    #[serde(default)]
    files: Vec<PlanningAdminDraftFileUpdate>,
}

#[derive(Debug, Clone, Deserialize)]
struct ResetRequest {
    target: String,
}

#[derive(Debug, Clone, Serialize)]
struct OverviewApiResponse {
    csrf_token: String,
    overview: PlanningAdminOverview,
}

#[derive(Debug, Clone, Serialize)]
struct DraftPromoteApiResponse {
    promoted_file_count: usize,
    is_valid: bool,
    session: PlanningAdminSessionView,
}

#[derive(Template)]
#[template(path = "admin/dashboard.html")]
struct DashboardTemplate {
    page_title: String,
    current_nav: &'static str,
    workspace_dir: String,
    csrf_token: String,
    notice: Option<String>,
    overview: PlanningAdminOverview,
}

#[derive(Template)]
#[template(path = "admin/directions.html")]
struct DirectionsTemplate {
    page_title: String,
    current_nav: &'static str,
    workspace_dir: String,
    csrf_token: String,
    notice: Option<String>,
    overview: PlanningAdminOverview,
    management: PlanningAdminManagementView,
}

#[derive(Template)]
#[template(path = "admin/tasks.html")]
struct TasksTemplate {
    page_title: String,
    current_nav: &'static str,
    workspace_dir: String,
    csrf_token: String,
    notice: Option<String>,
    overview: PlanningAdminOverview,
    management: PlanningAdminManagementView,
}

#[derive(Template)]
#[template(path = "admin/controls.html")]
struct ControlsTemplate {
    page_title: String,
    current_nav: &'static str,
    workspace_dir: String,
    csrf_token: String,
    notice: Option<String>,
    overview: PlanningAdminOverview,
}

#[derive(Template)]
#[template(path = "admin/editor.html")]
struct EditorTemplate {
    page_title: String,
    current_nav: &'static str,
    workspace_dir: String,
    csrf_token: String,
    notice: Option<String>,
    session: PlanningAdminSessionView,
}

#[derive(Template)]
#[template(path = "admin/partials/draft_status.html")]
struct DraftStatusTemplate {
    notice: Option<String>,
    session: PlanningAdminSessionView,
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
    let planning_workspace_port = Arc::new(FilesystemPlanningWorkspaceAdapter::new());
    let planning_authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
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
        .route("/api/planning/summary", get(summary_api))
        .route("/api/planning/runtime", get(runtime_api))
        .route("/api/planning/drafts", post(create_draft_api))
        .route(
            "/api/planning/drafts/{draft_name}",
            get(load_draft_api).put(save_draft_api),
        )
        .route(
            "/api/planning/drafts/{draft_name}/validate",
            post(validate_draft_api),
        )
        .route(
            "/api/planning/drafts/{draft_name}/promote",
            post(promote_draft_api),
        )
        .route("/api/planning/directions", post(upsert_direction_api))
        .route(
            "/api/planning/directions/delete",
            post(delete_direction_api),
        )
        .route("/api/planning/tasks", post(upsert_task_api))
        .route("/api/planning/tasks/delete", post(delete_task_api))
        .route("/api/planning/files/export", post(export_files_api))
        .route("/api/planning/files/apply", post(apply_files_api))
        .route("/api/planning/reset", post(reset_api))
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

async fn summary_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
) -> std::result::Result<Response, StatusCode> {
    let (jar, csrf_token) = ensure_csrf_cookie(jar);
    let overview = state
        .facade
        .load_overview()
        .map_err(internal_server_error)?;
    Ok((
        jar,
        Json(OverviewApiResponse {
            csrf_token,
            overview,
        }),
    )
        .into_response())
}

async fn runtime_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
) -> std::result::Result<Response, StatusCode> {
    let (jar, _) = ensure_csrf_cookie(jar);
    let runtime = state
        .facade
        .load_runtime_summary()
        .map_err(internal_server_error)?;
    Ok((jar, Json(runtime)).into_response())
}

async fn create_draft_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<CreateDraftRequest>,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    let session = state
        .facade
        .create_draft_session(request.kind, request.direction_id.as_deref())
        .map_err(internal_server_error)?;
    Ok(Json(session).into_response())
}

async fn load_draft_api(
    State(state): State<AdminAppState>,
    Path(draft_name): Path<String>,
    Query(query): Query<EditorQuery>,
) -> std::result::Result<Response, StatusCode> {
    let session = state
        .facade
        .load_draft_session(PlanningAdminDraftLoadRequest {
            draft_name,
            kind: query.kind,
            direction_id: query.direction_id,
        })
        .map_err(internal_server_error)?;
    Ok(Json(session).into_response())
}

async fn save_draft_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(draft_name): Path<String>,
    Json(request): Json<SaveDraftRequest>,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    let (_, session) = state
        .facade
        .save_draft(PlanningAdminDraftMutationRequest {
            draft_name,
            kind: request.kind,
            direction_id: request.direction_id,
            files: request.files,
        })
        .map_err(internal_server_error)?;
    Ok(Json(session).into_response())
}

async fn validate_draft_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(draft_name): Path<String>,
    Json(request): Json<SaveDraftRequest>,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    let (_, session) = state
        .facade
        .save_draft(PlanningAdminDraftMutationRequest {
            draft_name,
            kind: request.kind,
            direction_id: request.direction_id,
            files: request.files,
        })
        .map_err(internal_server_error)?;
    Ok(Json(session).into_response())
}

async fn promote_draft_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(draft_name): Path<String>,
    Json(request): Json<SaveDraftRequest>,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    let (result, session) = state
        .facade
        .promote_draft(PlanningAdminDraftMutationRequest {
            draft_name,
            kind: request.kind,
            direction_id: request.direction_id,
            files: request.files,
        })
        .map_err(internal_server_error)?;
    Ok(Json(DraftPromoteApiResponse {
        promoted_file_count: result.promoted_file_count,
        is_valid: result.validation_report.is_valid(),
        session,
    })
    .into_response())
}

async fn upsert_direction_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<PlanningAdminDirectionMutationRequest>,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .upsert_direction(request)
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

async fn delete_direction_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<PlanningAdminDirectionDeleteRequest>,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .delete_direction(request)
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

async fn upsert_task_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<PlanningAdminTaskMutationRequest>,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .upsert_task(request)
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

async fn delete_task_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<PlanningAdminTaskDeleteRequest>,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .delete_task(request)
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

async fn export_files_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .export_active_files_for_edit()
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

async fn apply_files_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .apply_exported_files()
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

async fn reset_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<ResetRequest>,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .reset_workspace(parse_reset_target(&request.target)?)
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
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
                "directions" => PlanningAdminFileKey::Directions,
                "task_ledger" => PlanningAdminFileKey::TaskLedger,
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
        PlanningAdminDraftKind::TaskLedger => "tasks",
        PlanningAdminDraftKind::Directions
        | PlanningAdminDraftKind::QueueIdlePrompt
        | PlanningAdminDraftKind::DirectionDetail => "directions",
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

fn notice_location(path: &str, notice: &str) -> String {
    format!("{path}?notice={}", encode_uri_component(notice))
}

fn encode_uri_component(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

fn ensure_csrf_cookie(jar: CookieJar) -> (CookieJar, String) {
    if let Some(existing) = jar
        .get(CSRF_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
    {
        return (jar, existing);
    }

    let token = new_csrf_token();
    let cookie = Cookie::build((CSRF_COOKIE_NAME, token.clone()))
        .path("/")
        .same_site(SameSite::Lax)
        .http_only(false)
        .build();
    (jar.add(cookie), token)
}

fn verify_form_csrf(jar: &CookieJar, token: &str) -> std::result::Result<(), StatusCode> {
    let cookie_value = jar
        .get(CSRF_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
        .ok_or(StatusCode::FORBIDDEN)?;
    if cookie_value == token {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

fn verify_header_csrf(jar: &CookieJar, headers: &HeaderMap) -> std::result::Result<(), StatusCode> {
    let cookie_value = jar
        .get(CSRF_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
        .ok_or(StatusCode::FORBIDDEN)?;
    let header_value = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .ok_or(StatusCode::FORBIDDEN)?;
    if cookie_value == header_value {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

fn new_csrf_token() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_htmx_request(headers: &HeaderMap) -> bool {
    headers
        .get("hx-request")
        .is_some_and(|value| value == HeaderValue::from_static("true"))
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

fn render_html<T: Template>(
    jar: CookieJar,
    template: T,
) -> std::result::Result<Response, StatusCode> {
    let body = template.render().map_err(internal_server_error)?;
    Ok((jar, Html(body)).into_response())
}

fn render_fragment<T: Template>(template: T) -> std::result::Result<Response, StatusCode> {
    let body = template.render().map_err(internal_server_error)?;
    Ok(Html(body).into_response())
}

fn internal_server_error(error: impl Into<anyhow::Error>) -> StatusCode {
    let _ = error.into();
    StatusCode::INTERNAL_SERVER_ERROR
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use axum::response::Response;
    use tower::ServiceExt;

    use super::{AdminAppState, CSRF_COOKIE_NAME, DEFAULT_PORT, build_router, parse_args};
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
    use crate::application::service::planning::{
        PlanningAdminDirectionDeleteRequest, PlanningAdminDirectionMutationRequest,
        PlanningAdminDraftKind, PlanningAdminFacadeService, PlanningAdminTaskMutationRequest,
        PlanningServices,
    };
    use crate::domain::planning::{DirectionCatalogDocument, TaskLedgerDocument};

    struct TempGitRepo {
        root: PathBuf,
        repo_root: PathBuf,
    }

    impl TempGitRepo {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be valid")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("{label}-{unique}"));
            let repo_root = root.join("repo");
            std::fs::create_dir_all(&repo_root).expect("repo root should exist");
            run_git(&repo_root, &["init", "-q"]);
            run_git(&repo_root, &["config", "user.name", "RefinedStone"]);
            run_git(
                &repo_root,
                &["config", "user.email", "chem.en.9273@gmail.com"],
            );
            std::fs::write(repo_root.join("README.md"), "seed\n").expect("seed file should write");
            run_git(&repo_root, &["add", "README.md"]);
            run_git(&repo_root, &["commit", "-qm", "init"]);
            Self { root, repo_root }
        }
    }

    impl Drop for TempGitRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn run_git(repo_root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(repo_root)
            .args(args)
            .status()
            .expect("git command should spawn");
        assert!(status.success(), "git command should succeed");
    }

    fn init_workspace(repo_root: &Path) {
        let planning = PlanningServices::from_workspace_port(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
        planning
            .workspace
            .initialize_simple_workspace(repo_root.to_string_lossy().as_ref())
            .expect("planning workspace should initialize");
    }

    fn build_test_app(repo_root: &Path) -> Router {
        let state = AdminAppState {
            facade: Arc::new(PlanningAdminFacadeService::new(
                repo_root.display().to_string(),
                Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            )),
        };
        build_router(state)
    }

    #[test]
    fn parse_args_defaults_to_high_local_port() {
        let parsed = parse_args(Vec::<String>::new()).expect("args should parse");

        assert_eq!(parsed.port, DEFAULT_PORT);
        assert_eq!(parsed.port, 18442);
    }

    #[test]
    fn parse_args_accepts_port_override() {
        let parsed =
            parse_args(["--port".to_string(), "19442".to_string()]).expect("args should parse");

        assert_eq!(parsed.port, 19442);
    }

    fn response_cookie(response: &Response, name: &str) -> Option<String> {
        response
            .headers()
            .get_all("set-cookie")
            .iter()
            .find_map(|value| {
                let text = value.to_str().ok()?;
                text.starts_with(name).then(|| text.to_string())
            })
    }

    fn cookie_value(set_cookie: &str) -> String {
        set_cookie
            .split(';')
            .next()
            .and_then(|pair| pair.split_once('='))
            .map(|(_, value)| value.to_string())
            .expect("cookie should parse")
    }

    fn cookie_header(name: &str, value: &str) -> String {
        format!("{name}={value}")
    }

    #[tokio::test]
    async fn summary_api_returns_csrf_and_ready_workspace_state() {
        let repo = TempGitRepo::new("admin-summary");
        init_workspace(&repo.repo_root);
        let app = build_test_app(&repo.repo_root);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/planning/summary")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response_cookie(&response, CSRF_COOKIE_NAME).is_some());
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("summary body should parse");
        assert_eq!(
            json["overview"]["doctor"]["planning_state"],
            "ready_without_task"
        );
        assert!(json["csrf_token"].as_str().is_some());
    }

    #[tokio::test]
    async fn draft_create_and_promote_api_updates_queue_summary() {
        let repo = TempGitRepo::new("admin-promote");
        init_workspace(&repo.repo_root);
        let app = build_test_app(&repo.repo_root);

        let summary_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/planning/summary")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let set_cookie = response_cookie(&summary_response, CSRF_COOKIE_NAME).expect("csrf cookie");
        let cookie = cookie_header(CSRF_COOKIE_NAME, &cookie_value(&set_cookie));
        let token = {
            let body = to_bytes(summary_response.into_body(), usize::MAX)
                .await
                .expect("body should read");
            let json: serde_json::Value =
                serde_json::from_slice(&body).expect("summary body should parse");
            json["csrf_token"]
                .as_str()
                .expect("csrf token should exist")
                .to_string()
        };

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/planning/drafts")
                    .header("cookie", cookie.clone())
                    .header("x-csrf-token", token.clone())
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "kind": PlanningAdminDraftKind::TaskLedger,
                        }))
                        .expect("json should serialize"),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("create request should succeed");
        assert_eq!(create_response.status(), StatusCode::OK);
        let create_body = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .expect("create body should read");
        let session: serde_json::Value =
            serde_json::from_slice(&create_body).expect("create session should parse");
        let draft_name = session["draft_name"]
            .as_str()
            .expect("draft name should exist");
        let task_ledger = r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-admin-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Admin UI queued the next actionable task.",
      "title": "Expose planning admin dashboard",
      "description": "Show the queue head in the new local admin server.",
      "status": "ready",
      "base_priority": 80,
      "created_by": "user",
      "last_updated_by": "user",
      "updated_at": "2026-04-18T12:00:00Z"
    }
  ]
}"#;

        let promote_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/planning/drafts/{draft_name}/promote"))
                    .header("cookie", cookie)
                    .header("x-csrf-token", token)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "kind": "task_ledger",
                            "files": [
                                {
                                    "key": "task_ledger",
                                    "body": task_ledger
                                }
                            ]
                        }))
                        .expect("json should serialize"),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("promote request should succeed");
        assert_eq!(promote_response.status(), StatusCode::OK);
        let promote_body = to_bytes(promote_response.into_body(), usize::MAX)
            .await
            .expect("promote body should read");
        let promote_json: serde_json::Value =
            serde_json::from_slice(&promote_body).expect("promote body should parse");
        assert_eq!(
            promote_json["is_valid"], true,
            "promote response: {promote_json}"
        );
        assert_eq!(promote_json["promoted_file_count"], 5);

        let summary_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/planning/summary")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("summary request should succeed");
        let body = to_bytes(summary_response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("summary body should parse");
        assert_eq!(
            json["overview"]["runtime"]["queue_head"]["task_title"],
            "Expose planning admin dashboard"
        );
    }

    #[tokio::test]
    async fn mutation_api_rejects_missing_csrf_header() {
        let repo = TempGitRepo::new("admin-csrf");
        init_workspace(&repo.repo_root);
        let app = build_test_app(&repo.repo_root);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/planning/drafts")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "kind": "task_ledger"
                        }))
                        .expect("json should serialize"),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn admin_crud_defaults_task_direction_and_deletes_direction_children() {
        let repo = TempGitRepo::new("admin-crud");
        init_workspace(&repo.repo_root);
        let adapter = Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let facade = PlanningAdminFacadeService::new(repo.repo_root.display().to_string(), adapter);

        facade
            .upsert_direction(PlanningAdminDirectionMutationRequest {
                id: "secondary".to_string(),
                title: "Secondary".to_string(),
                summary: "Secondary workstream".to_string(),
                success_criteria_text: "Keep secondary work isolated".to_string(),
                scope_hints_text: String::new(),
                detail_doc_path: String::new(),
                state: "active".to_string(),
            })
            .expect("direction should be added");
        facade
            .upsert_task(PlanningAdminTaskMutationRequest {
                id: "task-default".to_string(),
                direction_id: String::new(),
                title: "Defaulted task".to_string(),
                description: String::new(),
                status: "ready".to_string(),
                base_priority: "10".to_string(),
                dynamic_priority_delta: String::new(),
                priority_reason: String::new(),
                depends_on_text: String::new(),
                blocked_by_text: String::new(),
            })
            .expect("task should default to general workstream");
        facade
            .upsert_task(PlanningAdminTaskMutationRequest {
                id: "task-secondary".to_string(),
                direction_id: "secondary".to_string(),
                title: "Secondary task".to_string(),
                description: String::new(),
                status: "ready".to_string(),
                base_priority: "10".to_string(),
                dynamic_priority_delta: String::new(),
                priority_reason: String::new(),
                depends_on_text: "task-default".to_string(),
                blocked_by_text: String::new(),
            })
            .expect("secondary task should be added");

        facade
            .delete_direction(PlanningAdminDirectionDeleteRequest {
                id: "secondary".to_string(),
            })
            .expect("direction delete should remove child tasks");

        let record = FilesystemPlanningWorkspaceAdapter::new()
            .load_planning_workspace_files(repo.repo_root.display().to_string().as_str())
            .expect("workspace should load");
        let directions: DirectionCatalogDocument =
            toml::from_str(&record.directions_toml.expect("directions should exist"))
                .expect("directions should parse");
        let task_ledger: TaskLedgerDocument =
            serde_json::from_str(&record.task_ledger_json.expect("task ledger should exist"))
                .expect("task ledger should parse");

        assert!(
            directions
                .directions
                .iter()
                .all(|direction| direction.id != "secondary")
        );
        assert!(
            task_ledger
                .tasks
                .iter()
                .all(|task| task.id != "task-secondary")
        );
        assert_eq!(task_ledger.tasks[0].id, "task-default");
        assert_eq!(task_ledger.tasks[0].direction_id, "general-workstream");
    }
}

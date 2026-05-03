use super::forms::{
    CreateDraftForm, DirectionMutationForm, DraftMutationForm, EditorQuery, FileSyncForm,
    IdDeleteForm, ResetForm, TaskMutationForm,
};
use super::helpers::{
    encode_uri_component, ensure_csrf_cookie, internal_server_error, is_htmx_request,
    notice_location, render_fragment, render_html, verify_form_csrf,
};
use super::views::{
    ControlsTemplate, DashboardTemplate, DirectionsTemplate, DraftStatusTemplate, EditorTemplate,
    TasksTemplate,
};
use super::{AdminAppState, parse_reset_target};
use crate::application::service::planning::{
    PlanningAdminDirectionDeleteRequest, PlanningAdminDirectionMutationRequest,
    PlanningAdminDraftFileUpdate, PlanningAdminDraftKind, PlanningAdminDraftLoadRequest,
    PlanningAdminDraftMutationRequest, PlanningAdminFileKey, PlanningAdminSessionView,
    PlanningAdminTaskDeleteRequest, PlanningAdminTaskMutationRequest,
};
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use std::collections::HashMap;

/*
 * pages.rs is the browser/form half of the planning admin inbound adapter. It renders Askama
 * templates, accepts classic form posts, and uses redirects or HTMX fragments as the response
 * contract. The JSON adapter in api.rs can pass typed bodies directly; this file has to normalize
 * browser-specific details such as csrf_token form fields, notice query strings, and dynamic
 * editor field names before handing requests to PlanningAdminFacadeService.
 */
pub(super) async fn dashboard_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * The dashboard is the overview bootstrap for human operators. It refreshes the CSRF cookie and
     * pairs the service overview with the active workspace path so templates can show the same state
     * that JSON clients receive from summary_api, but with nav and notice chrome added.
     */
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

pub(super) async fn directions_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Direction editing needs both the compact overview and the management projection. Keeping both
     * loads here lets the template render navigation badges from overview while using the richer
     * management view for editable direction/task relationships.
     */
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

pub(super) async fn tasks_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    // Tasks use the same management projection as directions, but keep their own nav and notices.
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

pub(super) async fn upsert_direction_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<DirectionMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Browser forms keep every direction field as text. This adapter maps those fields into the
     * application mutation request, but leaves id generation, state normalization, and authority
     * document writes to the facade.
     */
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

pub(super) async fn delete_direction_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<IdDeleteForm>,
) -> std::result::Result<Response, StatusCode> {
    // The route supplies operation meaning; the shared IdDeleteForm only transports the selected id.
    verify_form_csrf(&jar, &form.csrf_token)?;
    let outcome = state
        .facade
        .delete_direction(PlanningAdminDirectionDeleteRequest { id: form.id })
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&notice_location("/admin/directions", &outcome.notice)).into_response())
}

pub(super) async fn upsert_task_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<TaskMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Task form strings intentionally pass through instead of being parsed here. The application
     * layer has the direction graph, dependency vocabulary, and priority rules needed to interpret
     * status, numeric priority text, dependency lists, and blockers consistently.
     */
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

pub(super) async fn delete_task_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<IdDeleteForm>,
) -> std::result::Result<Response, StatusCode> {
    // Task delete follows the same post-redirect-get shape as direction delete for browser safety.
    verify_form_csrf(&jar, &form.csrf_token)?;
    let outcome = state
        .facade
        .delete_task(PlanningAdminTaskDeleteRequest { id: form.id })
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&notice_location("/admin/tasks", &outcome.notice)).into_response())
}

pub(super) async fn export_files_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<FileSyncForm>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * File sync forms have no payload beyond CSRF because the active planning workspace is implicit.
     * Export prepares editable files from authority state; the redirect keeps refresh/back-button
     * behavior from repeating the mutation.
     */
    verify_form_csrf(&jar, &form.csrf_token)?;
    let outcome = state
        .facade
        .export_active_files_for_edit()
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&notice_location("/admin/controls", &outcome.notice)).into_response())
}

pub(super) async fn apply_files_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<FileSyncForm>,
) -> std::result::Result<Response, StatusCode> {
    // Apply is the inverse file-sync mutation: parse edited files, update authority, then redirect.
    verify_form_csrf(&jar, &form.csrf_token)?;
    let outcome = state
        .facade
        .apply_exported_files()
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&notice_location("/admin/controls", &outcome.notice)).into_response())
}

pub(super) async fn controls_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    // Controls intentionally render only overview-level state plus mutation forms for workspace-wide operations.
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

pub(super) async fn editor_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Path(draft_name): Path<String>,
    Query(query): Query<EditorQuery>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Editor identity is split across path and query by design: draft_name names the workspace file,
     * while kind and optional direction_id explain which planning context can interpret it. That
     * keeps URLs stable across queue, full-planning, and direction-detail draft types.
     */
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

pub(super) async fn create_draft_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<CreateDraftForm>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Creating a draft immediately redirects to the canonical editor URL returned by the facade.
     * The browser never has to construct draft file names itself; it only preserves kind and
     * direction_id in the URL so later saves load the same planning branch.
     */
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

pub(super) async fn save_draft_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(draft_name): Path<String>,
    Form(form): Form<DraftMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Save supports both full page posts and HTMX partial updates. The same facade call persists the
     * submitted editor fields; only the response shape changes so inline editor status can refresh
     * without redrawing the surrounding shell.
     */
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

pub(super) async fn validate_draft_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(draft_name): Path<String>,
    Form(form): Form<DraftMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Browser validation first saves the posted draft payload for the same reason as the JSON path:
     * the validation report should describe exactly what the operator submitted, not the previous
     * file contents left on disk.
     */
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

pub(super) async fn promote_draft_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Path(draft_name): Path<String>,
    Form(form): Form<DraftMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Promotion returns to the editor rather than redirecting away because failed validation is part
     * of the operator loop. The notice is derived from the facade result, while the refreshed session
     * carries detailed validation and file state into the template.
     */
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

pub(super) async fn reset_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<ResetForm>,
) -> std::result::Result<Response, StatusCode> {
    // Reset shares the same text-to-target parser as api.rs so HTML and JSON controls cannot diverge.
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
    // Convert the flattened browser form into the exact draft mutation request used by api.rs.
    PlanningAdminDraftMutationRequest {
        draft_name,
        kind: form.kind,
        direction_id: form.direction_id,
        files: extract_file_updates(form.values),
    }
}

pub(super) fn extract_file_updates(
    values: HashMap<String, String>,
) -> Vec<PlanningAdminDraftFileUpdate> {
    /*
     * DraftMutationForm flattens every dynamic editor input into a HashMap. Only file_* fields are
     * admitted here, and only keys known to PlanningAdminFileKey survive. That prevents unrelated
     * form controls from becoming arbitrary file updates while keeping the template free to render
     * different draft kinds with different editable file sets.
     */
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

pub(super) fn nav_for_kind(kind: PlanningAdminDraftKind) -> &'static str {
    // The editor sits under the nav area that owns the planning concept being edited.
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
    /*
     * Redirect targets are built centrally so draft identity, kind, optional direction scope, and
     * transient notice encoding stay consistent across create/save/validate/promote flows.
     */
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
    // Centralize editor template assembly so every draft action preserves CSRF, nav, and workspace context.
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

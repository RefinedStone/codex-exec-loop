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
 * pages.rs는 planning admin inbound adapter의 browser/form half다.
 * Askama template render, classic form POST, redirect, HTMX fragment response처럼 browser transport에만 필요한
 * 결정을 여기서 처리한다. api.rs는 typed JSON body를 바로 받을 수 있지만, 이 파일은 csrf_token form field,
 * notice query string, dynamic editor field name 같은 browser-specific detail을 정리한 뒤에만
 * PlanningAdminFacadeService로 넘긴다.
 *
 * 중요한 경계는 "HTML을 아는 곳"과 "planning을 판정하는 곳"의 분리다.
 * pages.rs는 form field를 application request DTO로 옮기고 response shape을 고르지만,
 * direction/task/draft의 유효성, authority mutation, workspace file write는 facade가 소유한다.
 */
pub(super) async fn dashboard_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * dashboard는 human operator가 admin surface에 들어오는 bootstrap page다.
     * CSRF cookie를 갱신하고 service overview에 active workspace path를 붙여 template context를 만든다.
     * JSON client가 summary_api에서 받는 state와 같은 projection을 쓰지만, browser page에는 nav marker와 redirect notice가
     * 추가된다.
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
     * direction edit 화면은 compact overview와 management projection을 동시에 필요로 한다.
     * overview는 navigation badge, runtime/doctor 상태, queue summary를 채우고, management view는 editable direction과
     * task cross-reference를 제공한다. 둘을 handler에서 로드해 template이 service를 다시 호출하지 않게 한다.
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
    // task page도 direction page와 같은 management projection을 쓰지만 nav marker와 redirect notice target은 task flow로 분리한다.
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
     * browser form은 모든 direction field를 text로 운반한다.
     * 이 adapter는 field name을 application mutation request의 field로 옮길 뿐, 빈 id의 create/update 해석,
     * state normalization, success criteria/scope hint parsing, authority document write는 facade에 남긴다.
     * mutation 뒤에는 post-redirect-get으로 돌아가 refresh/back-button이 같은 write를 반복하지 않게 한다.
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
    // route가 direction delete라는 operation 의미를 제공하고, shared IdDeleteForm은 선택된 id와 CSRF proof만 운반한다.
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
     * task form string은 여기서 parse하지 않고 의도적으로 그대로 통과시킨다.
     * status label, numeric priority text, dependency list, blocker list를 해석하려면 direction graph,
     * dependency vocabulary, queue priority rule이 필요하고 그 정보는 application layer에 있다.
     * pages.rs가 부분 파서를 갖지 않으면 browser form과 JSON/API mutation의 task semantics가 한 곳에 유지된다.
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
    // task delete도 direction delete와 같은 post-redirect-get shape를 써서 destructive POST가 browser refresh로 반복되지 않게 한다.
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
     * file sync form은 CSRF 외 operator payload가 없다.
     * 대상은 항상 active planning workspace이고, export는 authority state를 editable file tree로 mirror한다.
     * 이 mutation 역시 redirect로 끝내 browser refresh/back-button이 같은 export를 반복하지 않게 한다.
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
    // apply는 file-sync의 반대 방향이다. edited file을 parse해 authority를 갱신하고 redirect로 browser mutation cycle을 닫는다.
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

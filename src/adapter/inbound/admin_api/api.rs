use super::forms::{
    CreateDraftRequest, DraftPromoteApiResponse, EditorQuery, OverviewApiResponse, ResetRequest,
    SaveDraftRequest,
};
use super::{
    AdminAppState, ensure_csrf_cookie, internal_server_error, parse_reset_target,
    verify_header_csrf,
};
use crate::application::service::planning::{
    PlanningAdminDirectionDeleteRequest, PlanningAdminDirectionMutationRequest,
    PlanningAdminDraftLoadRequest, PlanningAdminDraftMutationRequest,
    PlanningAdminTaskDeleteRequest, PlanningAdminTaskMutationRequest,
};
use axum::extract::{Json, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;

/*
 * api.rs is the JSON half of the planning admin inbound adapter. It deliberately mirrors the
 * browser handlers in pages.rs, but it keeps three transport choices separate: request bodies are
 * typed JSON DTOs from forms.rs, CSRF proof comes from the x-csrf-token header, and responses are
 * application read models wrapped in Json. The facade remains the only place that knows planning
 * validation, workspace file policy, and authority-store mutation rules.
 */
pub(super) async fn summary_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Summary is the bootstrap endpoint for scriptable admin clients. It refreshes the same
     * cookie-bound CSRF token used by later mutation endpoints and returns the full overview so a
     * client can render directions, tasks, draft affordances, and controls without scraping HTML.
     */
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

pub(super) async fn runtime_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
) -> std::result::Result<Response, StatusCode> {
    // Runtime state is read-only, but it still carries the admin cookie forward for the JSON client.
    let (jar, _) = ensure_csrf_cookie(jar);
    let runtime = state
        .facade
        .load_runtime_summary()
        .map_err(internal_server_error)?;
    Ok((jar, Json(runtime)).into_response())
}

pub(super) async fn create_draft_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<CreateDraftRequest>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Draft creation is a mutating admin action even though it mostly prepares editable files.
     * Header CSRF verification keeps JSON clients on the same trust boundary as classic forms,
     * while the facade decides whether kind plus direction_id is a valid draft session request.
     */
    verify_header_csrf(&jar, &headers)?;
    let session = state
        .facade
        .create_draft_session(request.kind, request.direction_id.as_deref())
        .map_err(internal_server_error)?;
    Ok(Json(session).into_response())
}

pub(super) async fn load_draft_api(
    State(state): State<AdminAppState>,
    Path(draft_name): Path<String>,
    Query(query): Query<EditorQuery>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Loading a draft stays read-only: draft_name comes from the stable route identity, while the
     * query parameters select the interpretation branch. That matches the editor page route and
     * avoids encoding draft kind into the filesystem-facing name.
     */
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

pub(super) async fn save_draft_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(draft_name): Path<String>,
    Json(request): Json<SaveDraftRequest>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * JSON saves send already-typed file updates, so the handler can bypass the dynamic HTML
     * file_* field extraction used by pages.rs. The discarded facade return value is the write
     * result; JSON clients need the refreshed session because it carries current file contents and
     * validation state for redraw.
     */
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

pub(super) async fn validate_draft_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(draft_name): Path<String>,
    Json(request): Json<SaveDraftRequest>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Validation intentionally flows through save_draft first. That makes the checked report match
     * the exact payload the operator just submitted, instead of validating stale workspace files or
     * requiring the client to issue save and validate as two separate state-changing requests.
     */
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

pub(super) async fn promote_draft_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(draft_name): Path<String>,
    Json(request): Json<SaveDraftRequest>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Promotion is the point where draft edits become active planning files. The facade validates,
     * writes, and reloads the session in one transaction-shaped call; this adapter only compresses
     * the outcome into fields the browser client can display without understanding validation
     * report internals.
     */
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

pub(super) async fn upsert_direction_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<PlanningAdminDirectionMutationRequest>,
) -> std::result::Result<Response, StatusCode> {
    // Direction JSON bodies already match the application mutation request, so no adapter mapping is needed.
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .upsert_direction(request)
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

pub(super) async fn delete_direction_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<PlanningAdminDirectionDeleteRequest>,
) -> std::result::Result<Response, StatusCode> {
    // Deleting a direction can affect task planning context, so the facade owns all cascading rules.
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .delete_direction(request)
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

pub(super) async fn upsert_task_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<PlanningAdminTaskMutationRequest>,
) -> std::result::Result<Response, StatusCode> {
    // Task mutation stays in application request form to preserve priority and dependency semantics.
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .upsert_task(request)
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

pub(super) async fn delete_task_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<PlanningAdminTaskDeleteRequest>,
) -> std::result::Result<Response, StatusCode> {
    // The adapter accepts only the transport envelope; queue cleanup and authority writes stay below.
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .delete_task(request)
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

pub(super) async fn export_files_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Export/apply endpoints have no JSON body because the active planning workspace is the only
     * subject. CSRF is therefore the entire caller intent check before the facade mirrors authority
     * state into editable files.
     */
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .export_active_files_for_edit()
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

pub(super) async fn apply_files_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> std::result::Result<Response, StatusCode> {
    // Apply reverses export by asking the facade to parse edited files and update planning authority.
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .apply_exported_files()
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

pub(super) async fn reset_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<ResetRequest>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * Reset shares parse_reset_target with the HTML control path so queue, directions, and all keep
     * one accepted vocabulary. Invalid transport labels are rejected as BAD_REQUEST before the
     * facade gets a chance to mutate workspace state.
     */
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .reset_workspace(parse_reset_target(&request.target)?)
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

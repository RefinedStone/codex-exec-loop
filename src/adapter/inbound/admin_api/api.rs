use super::forms::{
    CreateDraftRequest, DraftPromoteApiResponse, EditorQuery, OverviewApiResponse, ResetRequest,
    SaveDraftRequest,
};
use super::{
    AdminAppState, ensure_csrf_cookie, internal_server_error, parse_reset_target,
    verify_header_csrf,
};
use crate::adapter::inbound::admin_api::akra_dashboard::{
    EventFeedView, RuntimeEventView, build_akra_dashboard_view, build_akra_events_view,
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
use serde::{Deserialize, Serialize};

/*
 * api.rs는 planning admin inbound adapter의 JSON half다.
 * 의도적으로 pages.rs의 browser handler와 같은 facade 흐름을 mirror하지만, transport 선택은 분리한다.
 * request body는 forms.rs의 typed JSON DTO, CSRF 증명은 x-csrf-token header, response는 Json으로 감싼
 * application read model이다. planning validation, workspace file policy, authority-store mutation rule을 아는 곳은
 * 여전히 facade 하나뿐이다.
 */
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AkraEventsQuery {
    pub limit: Option<usize>,
    pub after_sequence: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AkraEventsApiResponse {
    pub feed: EventFeedView,
    pub events: Vec<RuntimeEventView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AdminFriendlyErrorResponse {
    pub error: String,
    pub operator_message: String,
}

pub(super) async fn summary_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
) -> std::result::Result<Response, StatusCode> {
    /*
     * summary는 scriptable admin client의 bootstrap endpoint다.
     * 뒤 mutation endpoint가 쓸 cookie-bound CSRF token을 갱신하고 full overview를 돌려준다.
     * client는 HTML을 scraping하지 않고도 direction, task, draft affordance, control을 렌더링할 수 있다.
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
    // runtime state는 read-only지만 JSON client를 위해 admin cookie를 계속 전달한다.
    let (jar, _) = ensure_csrf_cookie(jar);
    let runtime = state
        .facade
        .load_runtime_summary()
        .map_err(internal_server_error)?;
    Ok((jar, Json(runtime)).into_response())
}

pub(super) async fn akra_dashboard_api(
    State(state): State<AdminAppState>,
) -> std::result::Result<Response, StatusCode> {
    let dashboard = build_akra_dashboard_view(
        state.facade.workspace_dir(),
        &state.planning,
        state.parallel_mode.as_ref(),
    );
    Ok(Json(dashboard).into_response())
}

pub(super) async fn akra_pool_api(
    State(state): State<AdminAppState>,
) -> std::result::Result<Response, StatusCode> {
    let dashboard = build_akra_dashboard_view(
        state.facade.workspace_dir(),
        &state.planning,
        state.parallel_mode.as_ref(),
    );
    Ok(Json(dashboard.pool).into_response())
}

pub(super) async fn akra_agents_api(
    State(state): State<AdminAppState>,
) -> std::result::Result<Response, StatusCode> {
    let dashboard = build_akra_dashboard_view(
        state.facade.workspace_dir(),
        &state.planning,
        state.parallel_mode.as_ref(),
    );
    Ok(Json(dashboard.agents).into_response())
}

pub(super) async fn akra_distributor_api(
    State(state): State<AdminAppState>,
) -> std::result::Result<Response, StatusCode> {
    let dashboard = build_akra_dashboard_view(
        state.facade.workspace_dir(),
        &state.planning,
        state.parallel_mode.as_ref(),
    );
    Ok(Json(dashboard.distributor).into_response())
}

pub(super) async fn akra_events_api(
    State(state): State<AdminAppState>,
    Query(query): Query<AkraEventsQuery>,
) -> std::result::Result<Response, StatusCode> {
    let limit = query.limit.unwrap_or(20);
    if limit > 200 {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(AdminFriendlyErrorResponse {
                error: "event_limit_too_large".to_string(),
                operator_message: "Runtime event API limit must be 200 or less.".to_string(),
            }),
        )
            .into_response());
    }
    let (feed, events) = build_akra_events_view(
        state.facade.workspace_dir(),
        state.parallel_mode.as_ref(),
        limit,
        query.after_sequence,
    );
    Ok(Json(AkraEventsApiResponse { feed, events }).into_response())
}

pub(super) async fn create_draft_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<CreateDraftRequest>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * draft creation은 주로 editable file을 준비하지만 mutating admin action이다.
     * header CSRF verification은 JSON client를 classic form과 같은 trust boundary에 두고, facade는 kind와
     * direction_id 조합이 valid draft session request인지 결정한다.
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
     * draft load는 read-only로 남는다.
     * draft_name은 stable route identity에서 오고, query parameter는 interpretation branch를 선택한다.
     * editor page route와 같은 형태이며, draft kind를 filesystem-facing name에 encoding하지 않게 한다.
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
     * JSON save는 이미 typed file update를 보내므로 pages.rs가 쓰는 dynamic HTML file_* field extraction을 우회한다.
     * 버리는 facade return value는 write result이고, JSON client에는 redraw에 필요한 current file content와 validation state를
     * 담은 refreshed session이 더 중요하다.
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
     * validation은 의도적으로 save_draft를 먼저 통과한다.
     * stale workspace file을 검증하거나 client가 save/validate를 별도 state-changing request로 나누게 하지 않고,
     * operator가 방금 제출한 정확한 payload에 대한 report를 만들기 위해서다.
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
     * promotion은 draft edit가 active planning file이 되는 지점이다.
     * facade는 validate/write/reload를 하나의 transaction-shaped call로 수행하고, adapter는 browser client가
     * validation report internals를 몰라도 표시할 수 있는 field로 outcome을 압축한다.
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
    // direction JSON body는 이미 application mutation request와 같은 shape라 adapter mapping이 필요 없다.
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
    // direction 삭제는 task planning context에 영향을 줄 수 있으므로 cascading rule은 facade가 소유한다.
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
    // task mutation은 priority/dependency semantics를 보존하기 위해 application request form 그대로 유지한다.
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
    // adapter는 transport envelope만 받는다. queue cleanup과 authority write는 아래 계층에 남긴다.
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
     * export/apply endpoint는 active planning workspace 하나만 대상으로 하므로 JSON body가 없다.
     * 그래서 facade가 authority state를 editable file로 mirror하기 전, CSRF가 caller intent를 확인하는 전체 gate다.
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
    // apply는 export의 반대 방향이다. facade에게 edited file을 parse하고 planning authority를 갱신하게 한다.
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
     * reset은 HTML control path와 parse_reset_target을 공유해 queue/directions/all이 하나의 accepted vocabulary를 유지하게 한다.
     * invalid transport label은 facade가 workspace state를 mutate하기 전에 BAD_REQUEST로 거절된다.
     */
    verify_header_csrf(&jar, &headers)?;
    let outcome = state
        .facade
        .reset_workspace(parse_reset_target(&request.target)?)
        .map_err(internal_server_error)?;
    Ok(Json(outcome).into_response())
}

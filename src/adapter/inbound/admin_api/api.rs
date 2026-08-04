use super::admin_debug_dashboard::{
    AdminDebugHarnessView, build_admin_dashboard_view, build_admin_events_view, map_harness_view,
};
use super::forms::{
    AkraControlRequest, CreateDraftRequest, DraftPromoteApiResponse, EditorQuery,
    OverviewApiResponse, ResetRequest, SaveDraftRequest,
};
use super::realtime::AkraCommandView;
use super::{
    AdminAppState, ensure_csrf_cookie, internal_server_error, parse_reset_target,
    verify_draft_name_path, verify_header_csrf,
};
use crate::adapter::inbound::admin_api::akra_dashboard::{EventFeedView, RuntimeEventView};
use crate::application::port::inbound::admin_debug_port::{
    AdminDebugHarnessCommand, AdminDebugScenario,
};
use crate::application::port::inbound::parallel_mode_admin_port::{
    ParallelModeAdminCommand, ParallelModeAdminCommandEffect,
};
use crate::application::port::inbound::planning_admin_port::{
    PlanningAdminDirectionDeleteRequest, PlanningAdminDirectionMutationRequest,
    PlanningAdminDraftLoadRequest, PlanningAdminDraftMutationRequest,
    PlanningAdminTaskDeleteRequest, PlanningAdminTaskMutationRequest,
};
use axum::extract::{Json, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::IntervalStream;

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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AkraStreamQuery {
    pub after_sequence: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AkraDebugHarnessRequest {
    pub action: String,
    pub scenario: Option<String>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AkraControlApiResponse {
    pub mode_enabled: bool,
    pub control_effect_in_flight: bool,
    pub current_epoch_id: Option<u64>,
    pub last_dispatch_withheld_reason: Option<String>,
    pub latest_command: Option<AkraCommandView>,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AkraStreamFrame {
    schema_version: u8,
    reason: &'static str,
    refresh_dashboard: bool,
    cursor_reset_required: bool,
    feed: EventFeedView,
    events: Vec<RuntimeEventView>,
    control: AkraControlApiResponse,
    debug_harness: AdminDebugHarnessView,
    server_time: String,
}

fn akra_control_view(state: &AdminAppState, message: impl Into<String>) -> AkraControlApiResponse {
    let projection = state.parallel_mode_admin_port.load_control_status();
    let latest_command = state.command_ledger.reconcile_latest(
        projection.control_effect_in_flight,
        projection.last_dispatch_withheld_reason.as_deref(),
    );
    AkraControlApiResponse {
        mode_enabled: projection.mode_enabled,
        control_effect_in_flight: projection.control_effect_in_flight,
        current_epoch_id: projection.current_epoch_id,
        last_dispatch_withheld_reason: projection.last_dispatch_withheld_reason,
        latest_command,
        message: message.into(),
    }
}

fn akra_control_response(state: &AdminAppState, message: impl Into<String>) -> Response {
    Json(akra_control_view(state, message)).into_response()
}

pub(super) async fn akra_control_api(
    State(state): State<AdminAppState>,
) -> std::result::Result<Response, StatusCode> {
    Ok(akra_control_response(
        &state,
        "control projection refreshed",
    ))
}

pub(super) async fn mutate_akra_control_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<AkraControlRequest>,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    if state.admin_debug_port.projection().enabled {
        return Err(StatusCode::CONFLICT);
    }
    let workspace_directory = state.facade.workspace_dir().to_string();
    let action = request.action.trim();
    let command = match action {
        "enable" => ParallelModeAdminCommand::Enable,
        "dispatch" => ParallelModeAdminCommand::Dispatch,
        "refresh" => ParallelModeAdminCommand::Refresh,
        "disable" => ParallelModeAdminCommand::Disable,
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let outcome = state
        .parallel_mode_admin_port
        .execute_control(&workspace_directory, command);
    let message = match outcome.effect {
        ParallelModeAdminCommandEffect::Enabled => "자동 루프 시작을 요청했습니다.",
        ParallelModeAdminCommandEffect::DispatchRequested => {
            "승인된 다음 작업 투입을 요청했습니다."
        }
        ParallelModeAdminCommandEffect::EnabledInsteadOfDispatch => {
            "루프가 꺼져 있어 시작 요청으로 전환했습니다."
        }
        ParallelModeAdminCommandEffect::RefreshRequested => "관제 투영 동기화를 요청했습니다.",
        ParallelModeAdminCommandEffect::Disabled => "자동 루프 정지를 요청했습니다.",
    };
    state.command_ledger.begin(action, message);
    Ok(akra_control_response(&state, message))
}

pub(super) async fn akra_command_api(
    State(state): State<AdminAppState>,
    Path(command_id): Path<String>,
) -> std::result::Result<Response, StatusCode> {
    let _ = akra_control_view(&state, "control projection refreshed");
    state
        .command_ledger
        .get(&command_id)
        .map(|command| Json(command).into_response())
        .ok_or(StatusCode::NOT_FOUND)
}

pub(super) async fn akra_stream_api(
    State(state): State<AdminAppState>,
    Query(query): Query<AkraStreamQuery>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    let header_cursor = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok());
    let mut after_sequence = header_cursor.or(query.after_sequence);
    let mut first_frame = true;
    let mut last_control_signature = String::new();
    let mut last_debug_revision = 0;
    let mut interval = tokio::time::interval(Duration::from_millis(1_500));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let stream = IntervalStream::new(interval).map(move |_| {
        let control = akra_control_view(&state, "realtime control projection");
        let debug_projection = state.admin_debug_port.projection();
        let debug_harness = map_harness_view(&debug_projection);
        let (feed, events) = build_admin_events_view(&state, 50, after_sequence);
        let control_signature =
            serde_json::to_string(&control).expect("AKRA control projection should serialize");
        let control_changed = control_signature != last_control_signature;
        let cursor_reset_required =
            feed.incremental && feed.total_event_count > feed.visible_event_count;
        let debug_changed = debug_harness.enabled && debug_harness.revision != last_debug_revision;
        let refresh_dashboard =
            first_frame || control_changed || debug_changed || !events.is_empty();
        let reason = if first_frame {
            "connected"
        } else if cursor_reset_required {
            "cursor_reset"
        } else if !events.is_empty() {
            "runtime_event"
        } else if control_changed {
            "control"
        } else if debug_changed {
            "debug_harness"
        } else {
            "heartbeat"
        };
        let event_id = feed.newest_sequence;
        if let Some(sequence) = event_id {
            after_sequence = Some(sequence);
        }
        first_frame = false;
        last_control_signature = control_signature;
        last_debug_revision = debug_harness.revision;
        let frame = AkraStreamFrame {
            schema_version: 1,
            reason,
            refresh_dashboard,
            cursor_reset_required,
            feed,
            events,
            control,
            debug_harness,
            server_time: chrono::Utc::now().to_rfc3339(),
        };
        let mut event = Event::default()
            .event("update")
            .retry(Duration::from_millis(1_500))
            .json_data(frame)
            .expect("AKRA realtime frame should serialize");
        if let Some(sequence) = event_id {
            event = event.id(sequence.to_string());
        }
        Ok(event)
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("akra-realtime"),
    )
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
    let dashboard = build_admin_dashboard_view(&state).map_err(internal_server_error)?;
    Ok(Json(dashboard).into_response())
}

pub(super) async fn akra_pool_api(
    State(state): State<AdminAppState>,
) -> std::result::Result<Response, StatusCode> {
    let dashboard = build_admin_dashboard_view(&state).map_err(internal_server_error)?;
    Ok(Json(dashboard.pool).into_response())
}

pub(super) async fn akra_agents_api(
    State(state): State<AdminAppState>,
) -> std::result::Result<Response, StatusCode> {
    let dashboard = build_admin_dashboard_view(&state).map_err(internal_server_error)?;
    Ok(Json(dashboard.agents).into_response())
}

pub(super) async fn akra_distributor_api(
    State(state): State<AdminAppState>,
) -> std::result::Result<Response, StatusCode> {
    let dashboard = build_admin_dashboard_view(&state).map_err(internal_server_error)?;
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
    let (feed, events) = build_admin_events_view(&state, limit, query.after_sequence);
    Ok(Json(AkraEventsApiResponse { feed, events }).into_response())
}

pub(super) async fn akra_debug_harness_api(
    State(state): State<AdminAppState>,
) -> std::result::Result<Response, StatusCode> {
    Ok(Json(map_harness_view(&state.admin_debug_port.projection())).into_response())
}

pub(super) async fn mutate_akra_debug_harness_api(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<AkraDebugHarnessRequest>,
) -> std::result::Result<Response, StatusCode> {
    verify_header_csrf(&jar, &headers)?;
    let command = match request.action.trim() {
        "play" => AdminDebugHarnessCommand::Play,
        "pause" => AdminDebugHarnessCommand::Pause,
        "step" => AdminDebugHarnessCommand::Step,
        "reset" => AdminDebugHarnessCommand::Reset,
        "scenario" => AdminDebugHarnessCommand::SelectScenario(
            request
                .scenario
                .as_deref()
                .and_then(AdminDebugScenario::from_key)
                .ok_or(StatusCode::BAD_REQUEST)?,
        ),
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let projection = state
        .admin_debug_port
        .execute(command)
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(map_harness_view(&projection)).into_response())
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
    verify_draft_name_path(&draft_name)?;
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
    verify_draft_name_path(&draft_name)?;
    let session = state
        .facade
        .save_draft_session(PlanningAdminDraftMutationRequest {
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
    verify_draft_name_path(&draft_name)?;
    let session = state
        .facade
        .save_draft_session(PlanningAdminDraftMutationRequest {
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
    verify_draft_name_path(&draft_name)?;
    let result = state
        .facade
        .promote_draft_session(PlanningAdminDraftMutationRequest {
            draft_name,
            kind: request.kind,
            direction_id: request.direction_id,
            files: request.files,
        })
        .map_err(internal_server_error)?;
    Ok(Json(DraftPromoteApiResponse {
        promoted_file_count: result.promoted_file_count,
        is_valid: result.is_valid,
        session: result.session,
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

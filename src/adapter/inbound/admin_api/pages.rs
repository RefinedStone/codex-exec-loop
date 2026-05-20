use super::forms::{
    AgentProfilesForm, CreateDraftForm, DirectionMutationForm, DraftMutationForm, EditorQuery,
    FileSyncForm, IdDeleteForm, ResetForm, TaskMutationForm,
};
use super::helpers::{
    encode_uri_component, ensure_csrf_cookie, internal_server_error, is_htmx_request,
    notice_location, render_fragment, render_html, verify_form_csrf,
};
use super::views::{
    AkraDashboardTemplate, AkraMetricsTemplate, AppServerPromptLogView, AppServerPromptsTemplate,
    ControlsTemplate, DashboardTemplate, DirectionsTemplate, DraftStatusTemplate, EditorTemplate,
    TasksTemplate,
};
use super::{AdminAppState, parse_reset_target};
use crate::adapter::inbound::admin_api::akra_dashboard::build_akra_dashboard_view;
use crate::application::service::parallel_agent_profile::{
    load_parallel_agent_profile_config, parse_parallel_agent_profile_config_json,
    save_parallel_agent_profile_config,
};
use crate::application::service::planning::{
    PlanningAdminDirectionDeleteRequest, PlanningAdminDirectionMutationRequest,
    PlanningAdminDraftFileUpdate, PlanningAdminDraftKind, PlanningAdminDraftLoadRequest,
    PlanningAdminDraftMutationRequest, PlanningAdminFileKey, PlanningAdminSessionView,
    PlanningAdminTaskDeleteRequest, PlanningAdminTaskMutationRequest,
};
use anyhow::anyhow;
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
#[derive(Clone, Copy)]
enum PlanningAdminSurface {
    Default,
    Akra,
}

impl PlanningAdminSurface {
    fn directions_title(self) -> &'static str {
        match self {
            Self::Default => "Directions",
            Self::Akra => "게임발전국 작전 방향",
        }
    }

    fn directions_nav(self) -> &'static str {
        match self {
            Self::Default => "directions",
            Self::Akra => "akra_directions",
        }
    }

    fn directions_path(self) -> &'static str {
        match self {
            Self::Default => "/admin/directions",
            Self::Akra => "/admin/akra/directions",
        }
    }

    fn direction_upsert_path(self) -> &'static str {
        match self {
            Self::Default => "/admin/directions/upsert",
            Self::Akra => "/admin/akra/directions/upsert",
        }
    }

    fn direction_delete_path(self) -> &'static str {
        match self {
            Self::Default => "/admin/directions/delete",
            Self::Akra => "/admin/akra/directions/delete",
        }
    }

    fn tasks_title(self) -> &'static str {
        match self {
            Self::Default => "Tasks",
            Self::Akra => "게임발전국 작업 관리",
        }
    }

    fn tasks_nav(self) -> &'static str {
        match self {
            Self::Default => "tasks",
            Self::Akra => "akra_tasks",
        }
    }

    fn tasks_path(self) -> &'static str {
        match self {
            Self::Default => "/admin/tasks",
            Self::Akra => "/admin/akra/tasks",
        }
    }

    fn task_upsert_path(self) -> &'static str {
        match self {
            Self::Default => "/admin/tasks/upsert",
            Self::Akra => "/admin/akra/tasks/upsert",
        }
    }

    fn task_delete_path(self) -> &'static str {
        match self {
            Self::Default => "/admin/tasks/delete",
            Self::Akra => "/admin/akra/tasks/delete",
        }
    }
}

pub(super) async fn akra_dashboard_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    let (jar, csrf_token) = ensure_csrf_cookie(jar);
    if !state.graphic.enabled {
        let overview = state
            .facade
            .load_overview()
            .map_err(internal_server_error)?;
        return render_html(
            jar,
            DashboardTemplate {
                page_title: "Planning Admin".to_string(),
                current_nav: "dashboard",
                workspace_dir: state.facade.workspace_dir().to_string(),
                csrf_token,
                notice: query.get("notice").cloned(),
                overview,
            },
        );
    }
    let dashboard = build_akra_dashboard_view(
        state.facade.as_ref(),
        state.parallel_mode_control_plane.as_ref(),
    )
    .map_err(internal_server_error)?;
    render_html(
        jar,
        AkraDashboardTemplate {
            page_title: "게임발전국".to_string(),
            current_nav: "akra_dashboard",
            workspace_dir: state.facade.workspace_dir().to_string(),
            csrf_token,
            notice: query.get("notice").cloned(),
            dashboard,
            api_base_url: state.graphic.api_base_url.clone(),
            polling_interval_ms: state.graphic.polling_interval_ms,
        },
    )
}

pub(super) async fn akra_metrics_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    let (jar, csrf_token) = ensure_csrf_cookie(jar);
    let dashboard = build_akra_dashboard_view(
        state.facade.as_ref(),
        state.parallel_mode_control_plane.as_ref(),
    )
    .map_err(internal_server_error)?;
    render_html(
        jar,
        AkraMetricsTemplate {
            page_title: "게임발전국 지표".to_string(),
            current_nav: "akra_metrics",
            workspace_dir: state.facade.workspace_dir().to_string(),
            csrf_token,
            notice: query.get("notice").cloned(),
            dashboard,
        },
    )
}

pub(super) async fn dashboard_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * dashboard는 human operator가 admin surface에 들어오는 bootstrap page다.
     * parallel/Akra 관제 화면과 분리해, 기본 `/admin` 진입은 항상 planning overview를 보여 준다.
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
    render_directions_page(state, jar, query, PlanningAdminSurface::Default).await
}

pub(super) async fn akra_directions_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    render_directions_page(state, jar, query, PlanningAdminSurface::Akra).await
}

async fn render_directions_page(
    state: AdminAppState,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
    surface: PlanningAdminSurface,
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
            page_title: surface.directions_title().to_string(),
            current_nav: surface.directions_nav(),
            workspace_dir: state.facade.workspace_dir().to_string(),
            csrf_token,
            notice: query.get("notice").cloned(),
            direction_upsert_path: surface.direction_upsert_path(),
            direction_delete_path: surface.direction_delete_path(),
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
    render_tasks_page(state, jar, query, PlanningAdminSurface::Default).await
}

pub(super) async fn akra_tasks_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    render_tasks_page(state, jar, query, PlanningAdminSurface::Akra).await
}

async fn render_tasks_page(
    state: AdminAppState,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
    surface: PlanningAdminSurface,
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
            page_title: surface.tasks_title().to_string(),
            current_nav: surface.tasks_nav(),
            workspace_dir: state.facade.workspace_dir().to_string(),
            csrf_token,
            notice: query.get("notice").cloned(),
            task_upsert_path: surface.task_upsert_path(),
            task_delete_path: surface.task_delete_path(),
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
    upsert_direction_for_surface(state, jar, form, PlanningAdminSurface::Default).await
}

pub(super) async fn upsert_akra_direction_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<DirectionMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    upsert_direction_for_surface(state, jar, form, PlanningAdminSurface::Akra).await
}

async fn upsert_direction_for_surface(
    state: AdminAppState,
    jar: CookieJar,
    form: DirectionMutationForm,
    surface: PlanningAdminSurface,
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
    Ok(Redirect::to(&notice_location(surface.directions_path(), &outcome.notice)).into_response())
}

pub(super) async fn delete_direction_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<IdDeleteForm>,
) -> std::result::Result<Response, StatusCode> {
    delete_direction_for_surface(state, jar, form, PlanningAdminSurface::Default).await
}

pub(super) async fn delete_akra_direction_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<IdDeleteForm>,
) -> std::result::Result<Response, StatusCode> {
    delete_direction_for_surface(state, jar, form, PlanningAdminSurface::Akra).await
}

async fn delete_direction_for_surface(
    state: AdminAppState,
    jar: CookieJar,
    form: IdDeleteForm,
    surface: PlanningAdminSurface,
) -> std::result::Result<Response, StatusCode> {
    // route가 direction delete라는 operation 의미를 제공하고, shared IdDeleteForm은 선택된 id와 CSRF proof만 운반한다.
    verify_form_csrf(&jar, &form.csrf_token)?;
    let outcome = state
        .facade
        .delete_direction(PlanningAdminDirectionDeleteRequest { id: form.id })
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&notice_location(surface.directions_path(), &outcome.notice)).into_response())
}

pub(super) async fn upsert_task_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<TaskMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    upsert_task_for_surface(state, jar, form, PlanningAdminSurface::Default).await
}

pub(super) async fn upsert_akra_task_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<TaskMutationForm>,
) -> std::result::Result<Response, StatusCode> {
    upsert_task_for_surface(state, jar, form, PlanningAdminSurface::Akra).await
}

async fn upsert_task_for_surface(
    state: AdminAppState,
    jar: CookieJar,
    form: TaskMutationForm,
    surface: PlanningAdminSurface,
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
    Ok(Redirect::to(&notice_location(surface.tasks_path(), &outcome.notice)).into_response())
}

pub(super) async fn delete_task_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<IdDeleteForm>,
) -> std::result::Result<Response, StatusCode> {
    delete_task_for_surface(state, jar, form, PlanningAdminSurface::Default).await
}

pub(super) async fn delete_akra_task_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<IdDeleteForm>,
) -> std::result::Result<Response, StatusCode> {
    delete_task_for_surface(state, jar, form, PlanningAdminSurface::Akra).await
}

async fn delete_task_for_surface(
    state: AdminAppState,
    jar: CookieJar,
    form: IdDeleteForm,
    surface: PlanningAdminSurface,
) -> std::result::Result<Response, StatusCode> {
    // task delete도 direction delete와 같은 post-redirect-get shape를 써서 destructive POST가 browser refresh로 반복되지 않게 한다.
    verify_form_csrf(&jar, &form.csrf_token)?;
    let outcome = state
        .facade
        .delete_task(PlanningAdminTaskDeleteRequest { id: form.id })
        .map_err(internal_server_error)?;
    Ok(Redirect::to(&notice_location(surface.tasks_path(), &outcome.notice)).into_response())
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
    // controls page는 workspace-wide operation만 다루므로 overview-level state와 mutation form context만 렌더링한다.
    let (jar, csrf_token) = ensure_csrf_cookie(jar);
    let overview = state
        .facade
        .load_overview()
        .map_err(internal_server_error)?;
    let agent_profile_config = load_parallel_agent_profile_config(state.facade.workspace_dir())
        .map_err(|error| internal_server_error(anyhow!(error)))?;
    let agent_profile_config_json = agent_profile_config.to_pretty_json();
    render_html(
        jar,
        ControlsTemplate {
            page_title: "Controls".to_string(),
            current_nav: "controls",
            workspace_dir: state.facade.workspace_dir().to_string(),
            csrf_token,
            notice: query.get("notice").cloned(),
            overview,
            agent_profile_config,
            agent_profile_config_json,
        },
    )
}

pub(super) async fn app_server_prompts_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    query: Query<HashMap<String, String>>,
) -> std::result::Result<Response, StatusCode> {
    let (jar, csrf_token) = ensure_csrf_cookie(jar);
    let snapshot = state
        .app_server_prompt_log_port
        .load_recent_app_server_prompt_interactions(state.facade.workspace_dir(), 80)
        .map_err(internal_server_error)?;
    render_html(
        jar,
        AppServerPromptsTemplate {
            page_title: "App-server Prompts".to_string(),
            current_nav: "app_server_prompts",
            workspace_dir: state.facade.workspace_dir().to_string(),
            csrf_token,
            notice: query.get("notice").cloned(),
            prompt_log: AppServerPromptLogView::from_records(snapshot.records),
        },
    )
}

pub(super) async fn update_agent_profiles_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<AgentProfilesForm>,
) -> std::result::Result<Response, StatusCode> {
    verify_form_csrf(&jar, &form.csrf_token)?;
    let config = parse_parallel_agent_profile_config_json(&form.profiles_json)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    save_parallel_agent_profile_config(state.facade.workspace_dir(), &config)
        .map_err(|error| internal_server_error(anyhow!(error)))?;
    Ok(Redirect::to(&notice_location(
        "/admin/controls",
        "parallel agent profiles saved",
    ))
    .into_response())
}

pub(super) async fn editor_page(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Path(draft_name): Path<String>,
    Query(query): Query<EditorQuery>,
) -> std::result::Result<Response, StatusCode> {
    /*
     * editor identity는 의도적으로 path와 query에 나뉜다.
     * draft_name은 workspace file 이름이고, kind와 optional direction_id는 그 파일을 어떤 planning context에서 해석할지
     * 설명한다. 이렇게 하면 queue, full-planning, direction-detail draft가 같은 editor route를 공유하면서도
     * service가 load branch를 정확히 선택할 수 있다.
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
     * draft 생성은 곧바로 facade가 만든 canonical editor URL로 redirect한다.
     * browser는 draft file name을 직접 조립하지 않고, kind와 direction_id만 URL에 보존한다.
     * 이후 save/validate/promote는 같은 path/query identity를 다시 읽어 같은 planning branch의 session을 갱신한다.
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
     * save는 full page POST와 HTMX partial update를 같은 mutation으로 처리한다.
     * facade call은 submitted editor field를 저장하고 refreshed session을 돌려준다.
     * transport 차이는 response shape뿐이다. HTMX는 draft status fragment만 다시 그리고, 일반 form submit은 같은 session으로
     * editor page 전체를 다시 렌더링한다.
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
     * browser validation은 JSON path와 같이 posted draft payload를 먼저 저장한다.
     * validation report가 disk에 남아 있던 이전 file content가 아니라 operator가 방금 제출한 form field를 설명해야 하기 때문이다.
     * HTMX와 full page response가 같은 session을 받으므로 inline status와 editor shell의 validation copy도 같은 source를 쓴다.
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
     * promotion은 실패 가능성이 operator loop의 일부라서 다른 page로 redirect하지 않고 editor를 다시 렌더링한다.
     * facade result는 promoted file count와 validation report를 제공하고, handler는 그것을 browser notice로 압축한다.
     * detailed validation과 file state는 refreshed session 안에 남겨 template이 같은 editor context에서 표시한다.
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
    // reset은 api.rs와 같은 text-to-target parser를 써서 HTML control과 JSON control의 accepted label이 갈라지지 않게 한다.
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
    // flattened browser form을 api.rs가 쓰는 것과 같은 draft mutation request shape로 변환한다.
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
     * DraftMutationForm은 dynamic editor input을 모두 HashMap으로 flatten한다.
     * 여기서는 file_* field만 후보로 받고, 그중 PlanningAdminFileKey가 아는 key만 살아남는다.
     * 이 필터가 없으면 stale browser field, hidden control, 임의 form input이 arbitrary file update로 승격될 수 있다.
     * 동시에 template은 draft kind별로 다른 editable file set을 자유롭게 렌더링할 수 있다.
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
    // editor는 현재 수정 중인 planning concept을 소유한 nav 영역 아래에 놓인다.
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
     * draft editor redirect target은 한곳에서 만든다.
     * draft identity, kind, optional direction scope, transient notice encoding이 create/save/validate/promote flow마다
     * 조금씩 달라지면 같은 draft session을 다른 URL로 가리키는 문제가 생긴다.
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
    // 모든 draft action이 CSRF, nav, workspace context를 같은 방식으로 보존하도록 editor template assembly를 중앙화한다.
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

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::application::service::planning::{
    PlanningAdminDraftFileUpdate, PlanningAdminDraftKind, PlanningAdminOverview,
    PlanningAdminSessionView,
};

/*
 * forms.rs는 planning admin inbound adapter의 transport DTO 층이다.
 * Axum이 URL query, classic browser form, JSON body에서 뽑아낼 수 있는 shape만 여기에 둔다.
 * validation, domain enum 해석, filesystem mutation policy는 이미 application service가 소유하므로,
 * 이 모듈은 "wire에서 어떤 이름과 타입으로 들어오는가"를 보존하는 데 집중한다.
 * 이 경계를 분리하면 pages.rs와 api.rs가 서로 다른 ad-hoc HashMap parsing rule을 공유하지 않아도 된다.
 */
#[derive(Debug, Clone, Deserialize)]
pub(super) struct EditorQuery {
    /*
     * draft name은 route path에 있고, kind와 optional direction_id는 그 이름을 어떤 service branch가
     * 해석할지 고른다. direction detail draft는 direction_id가 있어야 의미가 있지만, 그 조합의 유효성은
     * handler가 아니라 facade가 판단한다. notice는 redirect 뒤 UI에만 쓰이는 flash state라 draft session request에
     * 섞이지 않는다.
     */
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
    #[serde(default)]
    pub(super) notice: Option<String>,
}

/*
 * browser form DTO는 csrf_token을 body에 포함한다.
 * pages.rs는 typed application request를 만들기 전에 이 token을 cookie와 비교한다.
 * service-layer mutation method는 호출이 HTML form, HTMX fragment submit, JSON API 중 어디에서 왔는지 몰라도 되며,
 * CSRF 방어는 inbound adapter의 transport boundary에 남는다.
 */
#[derive(Debug, Clone, Deserialize)]
pub(super) struct CreateDraftForm {
    pub(super) csrf_token: String,
    // direction_id는 direction-detail draft에서만 의미가 있으므로 kind와의 pair validation은 service로 보낸다.
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DraftMutationForm {
    pub(super) csrf_token: String,
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
    /*
     * editor는 file_result_output 같은 dynamic field name으로 editable file을 렌더링한다.
     * Axum은 그 이름들을 fixed struct로 deserialize할 수 없으므로 unknown field를 flatten해 보관한다.
     * 이후 pages::extract_file_updates가 PlanningAdminFileKey로 알려진 field만 통과시켜 stale browser나 임의 field가
     * application-level file mutation으로 올라가지 못하게 한다.
     */
    #[serde(flatten)]
    pub(super) values: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ResetForm {
    pub(super) csrf_token: String,
    // page handler와 API handler가 parse_reset_target을 공유하도록 reset target은 transport text로 유지한다.
    pub(super) target: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ParallelPersonaForm {
    pub(super) csrf_token: String,
    pub(super) persona: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DirectionMutationForm {
    pub(super) csrf_token: String,
    /*
     * HTML form control은 direction field를 전부 text로 보낸다.
     * 빈 id는 create, 값이 있는 id는 update를 뜻한다. success criteria나 scope hint처럼 multi-line 입력이 가능한
     * field도 여기서는 문자열 그대로 보존하고, PlanningAdminDirectionMutation 처리 단계에서 line normalization과
     * authority document shape으로 변환한다. browser quoting/line ending 차이가 domain document에 직접 새지 않게 하기 위해서다.
     */
    #[serde(default)]
    pub(super) id: String,
    pub(super) title: String,
    #[serde(default)]
    pub(super) summary: String,
    #[serde(default)]
    pub(super) success_criteria_text: String,
    #[serde(default)]
    pub(super) scope_hints_text: String,
    #[serde(default)]
    pub(super) detail_doc_path: String,
    #[serde(default)]
    pub(super) state: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct IdDeleteForm {
    pub(super) csrf_token: String,
    // direction/task delete route가 같은 envelope를 쓰고, 어떤 service delete request로 갈지는 route handler가 결정한다.
    pub(super) id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct TaskMutationForm {
    pub(super) csrf_token: String,
    /*
     * task edit은 form boundary에서 의도적으로 stringly하다.
     * priority number, status label, dependency list, blocker list를 여기서 parse하면 direction graph와 queue semantics 없이
     * 부분 검증을 하게 된다. application mutation service가 direction cross-reference와 authority vocabulary를 가진 상태에서
     * 이 문자열들을 해석해야 accepted/proposed/skipped task rule과 일관된다.
     */
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) direction_id: String,
    pub(super) title: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default)]
    pub(super) status: String,
    #[serde(default)]
    pub(super) base_priority: String,
    #[serde(default)]
    pub(super) dynamic_priority_delta: String,
    #[serde(default)]
    pub(super) priority_reason: String,
    #[serde(default)]
    pub(super) depends_on_text: String,
    #[serde(default)]
    pub(super) blocked_by_text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct FileSyncForm {
    // export/apply control은 operator payload가 없으므로 CSRF token 자체가 browser form contract의 전부다.
    pub(super) csrf_token: String,
}

/*
 * JSON request는 body에서 csrf_token을 뺀다.
 * api.rs가 같은 cookie-bound token을 x-csrf-token header로 검증하기 때문이다.
 * 따라서 JSON body는 classic HTML form struct보다 service input에 더 가까운 모양을 가질 수 있고,
 * dynamic field name 대신 typed update vector를 보낼 수 있다.
 */
#[derive(Debug, Clone, Deserialize)]
pub(super) struct CreateDraftRequest {
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SaveDraftRequest {
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
    // JSON client는 typed file update를 직접 보내 HTML의 dynamic file_* field map extraction을 우회한다.
    #[serde(default)]
    pub(super) files: Vec<PlanningAdminDraftFileUpdate>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ResetRequest {
    // API reset도 ResetForm과 같은 label vocabulary를 쓰며 최종 허용 목록은 parse_reset_target 하나가 결정한다.
    pub(super) target: String,
}

/*
 * API response는 browser client에 필요한 adapter metadata와 application read model을 함께 노출한다.
 * summary가 돌려주는 csrf_token은 single-page admin client가 이후 JSON mutation을 시작하기 위한 bootstrap 값이다.
 * planning state 자체는 PlanningAdminOverview나 PlanningAdminSessionView 같은 application-owned projection에 남기므로,
 * response DTO는 UI transport affordance만 얇게 덧붙인다.
 */
#[derive(Debug, Clone, Serialize)]
pub(super) struct OverviewApiResponse {
    pub(super) csrf_token: String,
    pub(super) overview: PlanningAdminOverview,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DraftPromoteApiResponse {
    /*
     * promotion response는 compact success summary와 refreshed session을 함께 돌려준다.
     * count와 boolean은 client toast/button state 같은 가벼운 feedback에 쓰이고, session은 editor가 별도 load request 없이
     * validation, queue preview, file state를 다시 그리는 데 쓴다.
     */
    pub(super) promoted_file_count: usize,
    pub(super) is_valid: bool,
    pub(super) session: PlanningAdminSessionView,
}

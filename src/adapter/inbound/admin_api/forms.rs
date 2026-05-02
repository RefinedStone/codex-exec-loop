/*
 * 학습 주석: admin_api forms 모듈은 브라우저/JSON transport shape를 application planning admin
 * service의 typed request와 분리하는 inbound adapter 경계다. `pages.rs`는 HTML form body를 이 타입들로
 * 파싱한 뒤 CSRF 검증과 redirect/fragment 렌더링을 맡고, `api.rs`는 JSON body와 header CSRF로 같은
 * admin facade를 호출한다.
 */
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::application::service::planning::{
    PlanningAdminDraftFileUpdate, PlanningAdminDraftKind, PlanningAdminOverview,
    PlanningAdminSessionView,
};

#[derive(Debug, Clone, Deserialize)]
/*
 * 학습 주석: EditorQuery는 draft editor를 열 때 URL query로 전달되는 routing context다.
 * draft_name은 path segment에 있고, 이 query가 draft kind와 direction-specific draft 여부를 보완한다.
 */
pub(super) struct EditorQuery {
    // 학습 주석: 어떤 draft renderer/service branch를 열지 결정하는 핵심 discriminator다.
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    // 학습 주석: direction detail draft일 때만 채워지는 direction authority id다.
    pub(super) direction_id: Option<String>,
    #[serde(default)]
    // 학습 주석: redirect 후 editor 상단에 보여 줄 일회성 결과 메시지다.
    pub(super) notice: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
/*
 * 학습 주석: CreateDraftForm은 classic HTML form에서 draft session 생성을 요청하는 shape다.
 * hidden CSRF token은 page handler에서 검증하고, kind/direction_id만 application facade로 전달된다.
 */
pub(super) struct CreateDraftForm {
    // 학습 주석: cookie token과 비교되는 hidden field다. service layer로는 넘어가지 않는다.
    pub(super) csrf_token: String,
    // 학습 주석: full planning, queue-idle prompt, direction detail 중 어떤 draft를 만들지 정한다.
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    // 학습 주석: direction detail draft 생성 시 대상 direction을 service에 알려 주는 optional context다.
    pub(super) direction_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
/*
 * 학습 주석: DraftMutationForm은 editor page의 save/validate/promote form body다. 실제 편집 파일들은
 * 고정 필드가 아니라 `file_result_output` 같은 동적 field name으로 들어오므로 나머지 값을 flatten해
 * `extract_file_updates`가 application의 `PlanningAdminDraftFileUpdate` 목록으로 바꾼다.
 */
pub(super) struct DraftMutationForm {
    // 학습 주석: save/promote도 workspace를 바꾸는 mutation이라 page handler에서 먼저 CSRF 검증을 한다.
    pub(super) csrf_token: String,
    // 학습 주석: 같은 draft_name이라도 draft kind가 달라지면 허용 파일과 validation path가 달라진다.
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    // 학습 주석: direction detail draft의 service request에만 의미가 있고, 다른 draft kind에서는 비어 있을 수 있다.
    pub(super) direction_id: Option<String>,
    #[serde(flatten)]
    // 학습 주석: unknown field를 버리지 않고 받은 뒤 허용된 `file_*` key만 추려 legacy/raw authority 수정을 막는다.
    pub(super) values: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
/*
 * 학습 주석: ResetForm은 admin controls page의 reset POST body다. target은 transport에서는 문자열로
 * 들어오고, page handler의 `parse_reset_target`에서 application enum으로 좁혀진다.
 */
pub(super) struct ResetForm {
    // 학습 주석: reset은 destructive control action이므로 form CSRF 검증을 필수로 한다.
    pub(super) csrf_token: String,
    // 학습 주석: queue/directions/all 같은 UI 문자열이며 service에 직접 넘기지 않는다.
    pub(super) target: String,
}

#[derive(Debug, Clone, Deserialize)]
/*
 * 학습 주석: DirectionMutationForm은 directions management page의 add/edit form shape다. field들은
 * application의 `PlanningAdminDirectionMutationRequest`와 거의 1:1이지만, HTML form 호환을 위해
 * 빈 optional 값이 빈 문자열로 들어올 수 있게 default를 둔다.
 */
pub(super) struct DirectionMutationForm {
    // 학습 주석: direction catalog와 task authority를 바꾸는 mutation이므로 hidden token을 검증한다.
    pub(super) csrf_token: String,
    #[serde(default)]
    // 학습 주석: 비어 있으면 application service가 새 direction 생성으로 해석하고, 값이 있으면 update 경로가 된다.
    pub(super) id: String,
    // 학습 주석: direction title은 필수 사용자 입력이며 service validation이 blank 여부와 중복을 판단한다.
    pub(super) title: String,
    #[serde(default)]
    // 학습 주석: 목록/queue prompt에서 direction을 설명하는 짧은 운영자-facing summary다.
    pub(super) summary: String,
    #[serde(default)]
    // 학습 주석: textarea의 여러 줄 criteria를 service가 direction authority 항목으로 정규화한다.
    pub(super) success_criteria_text: String,
    #[serde(default)]
    // 학습 주석: scope hint textarea는 planning worker prompt에 들어갈 방향성 보조 문장으로 이어진다.
    pub(super) scope_hints_text: String,
    #[serde(default)]
    // 학습 주석: direction detail document path는 supporting file validation과 detail-doc editor 진입점에 연결된다.
    pub(super) detail_doc_path: String,
    #[serde(default)]
    // 학습 주석: active/inactive 같은 transport label이며 service가 domain state로 해석한다.
    pub(super) state: String,
}

#[derive(Debug, Clone, Deserialize)]
/*
 * 학습 주석: IdDeleteForm은 directions/tasks delete form이 공유하는 최소 mutation shape다.
 * delete handler는 route별로 다른 service request를 만들지만, browser form에서는 CSRF와 id만 필요하다.
 */
pub(super) struct IdDeleteForm {
    // 학습 주석: 삭제는 되돌리기 어려운 admin mutation이라 동일한 hidden CSRF policy를 쓴다.
    pub(super) csrf_token: String,
    // 학습 주석: route에 따라 direction id 또는 task id로 해석된다.
    pub(super) id: String,
}

#[derive(Debug, Clone, Deserialize)]
/*
 * 학습 주석: TaskMutationForm은 task management page의 add/edit form body다. 모든 값은 HTML input에서
 * 문자열로 들어오고, application service가 priority 숫자, status enum, relation list를 validation과
 * 함께 변환한다. adapter는 브라우저 field naming과 service request 사이의 얇은 mapping만 유지한다.
 */
pub(super) struct TaskMutationForm {
    // 학습 주석: task authority를 수정하므로 page handler에서 cookie token과 비교한다.
    pub(super) csrf_token: String,
    #[serde(default)]
    // 학습 주석: 비어 있으면 create, 채워져 있으면 update로 service가 분기한다.
    pub(super) id: String,
    #[serde(default)]
    // 학습 주석: task가 속한 direction id이며 queue projection과 validation의 cross-reference 기준이다.
    pub(super) direction_id: String,
    // 학습 주석: task title은 queue/table에서 사람에게 보이는 primary label이다.
    pub(super) title: String,
    #[serde(default)]
    // 학습 주석: worker에게 전달될 상세 작업 설명 원문이다.
    pub(super) description: String,
    #[serde(default)]
    // 학습 주석: proposed/ready/in-progress/done 같은 form label을 service가 domain status로 해석한다.
    pub(super) status: String,
    #[serde(default)]
    // 학습 주석: base priority는 문자열 input으로 받아 service에서 정수 범위 검증을 수행한다.
    pub(super) base_priority: String,
    #[serde(default)]
    // 학습 주석: runtime에서 조정되는 delta도 form에서는 문자열이며 mutation service가 숫자로 좁힌다.
    pub(super) dynamic_priority_delta: String,
    #[serde(default)]
    // 학습 주석: priority 변경 이유는 queue ranking의 설명 가능성을 위해 task authority에 보존된다.
    pub(super) priority_reason: String,
    #[serde(default)]
    // 학습 주석: newline/comma 입력을 service가 dependency id 목록으로 정규화한다.
    pub(super) depends_on_text: String,
    #[serde(default)]
    // 학습 주석: external/user blockers도 text field로 받아 queue blocking 판단에 쓰이는 relation으로 변환한다.
    pub(super) blocked_by_text: String,
}

#[derive(Debug, Clone, Deserialize)]
/*
 * 학습 주석: FileSyncForm은 exported planning files를 active workspace와 동기화하는 control form이다.
 * 추가 payload 없이 CSRF 검증만 통과하면 facade가 workspace 기준으로 export/apply 대상을 계산한다.
 */
pub(super) struct FileSyncForm {
    // 학습 주석: export/apply 모두 filesystem mutation이므로 CSRF token만 form payload로 요구한다.
    pub(super) csrf_token: String,
}

#[derive(Debug, Clone, Deserialize)]
/*
 * 학습 주석: CreateDraftRequest는 JSON API의 draft 생성 request다. HTML form과 달리 CSRF는 body가
 * 아니라 `x-csrf-token` header에서 검증되므로 request body에는 service에 필요한 context만 남긴다.
 */
pub(super) struct CreateDraftRequest {
    // 학습 주석: API client도 page form과 같은 draft kind vocabulary를 사용한다.
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    // 학습 주석: direction detail draft 생성을 API로 요청할 때 target direction을 지정한다.
    pub(super) direction_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
/*
 * 학습 주석: SaveDraftRequest는 JSON API의 draft save/promote body다. JSON transport는 이미 file update
 * 배열을 typed shape로 보낼 수 있으므로 HTML form의 flatten/extract 단계를 거치지 않는다.
 */
pub(super) struct SaveDraftRequest {
    // 학습 주석: facade가 draft session kind별 validation/promote policy를 선택하는 값이다.
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    // 학습 주석: direction detail draft일 때 service request에 그대로 전달되는 context다.
    pub(super) direction_id: Option<String>,
    #[serde(default)]
    // 학습 주석: JSON client가 명시한 파일 변경 목록이며 application surface type을 재사용한다.
    pub(super) files: Vec<PlanningAdminDraftFileUpdate>,
}

#[derive(Debug, Clone, Deserialize)]
/*
 * 학습 주석: ResetRequest는 JSON API reset body다. header CSRF가 별도로 검증되고, target 문자열은
 * HTML form reset과 같은 `parse_reset_target` 경로를 타야 두 transport의 허용 값이 어긋나지 않는다.
 */
pub(super) struct ResetRequest {
    // 학습 주석: destructive reset 대상 label이며 API handler가 application enum으로 변환한다.
    pub(super) target: String,
}

#[derive(Debug, Clone, Serialize)]
/*
 * 학습 주석: OverviewApiResponse는 admin summary JSON response다. 브라우저 UI도 같은 cookie CSRF를
 * 쓰므로 read response에 fresh token을 싣고, 실제 planning 상태는 application overview projection을
 * 그대로 전달한다.
 */
pub(super) struct OverviewApiResponse {
    // 학습 주석: 이후 JSON mutation에서 header로 되돌려 보낼 token이다.
    pub(super) csrf_token: String,
    // 학습 주석: doctor/runtime/directions summary를 묶은 application read model이다.
    pub(super) overview: PlanningAdminOverview,
}

#[derive(Debug, Clone, Serialize)]
/*
 * 학습 주석: DraftPromoteApiResponse는 JSON promote 결과를 API-friendly summary로 줄인다. full session은
 * 다시 editor 상태를 갱신하는 데 쓰고, promoted_file_count/is_valid는 client가 성공 toast와 validation
 * 상태를 빠르게 판단할 수 있게 한다.
 */
pub(super) struct DraftPromoteApiResponse {
    // 학습 주석: active planning으로 실제 반영된 파일 수다. validation 실패 시 0일 수 있다.
    pub(super) promoted_file_count: usize,
    // 학습 주석: promote 뒤 session validation report의 최종 정상 여부를 평평한 boolean으로 노출한다.
    pub(super) is_valid: bool,
    // 학습 주석: promote 이후에도 editor 화면을 최신 draft/session projection으로 다시 그리기 위한 view다.
    pub(super) session: PlanningAdminSessionView,
}

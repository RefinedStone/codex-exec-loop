use std::fmt;

use serde::{Deserialize, Serialize};

/*
 * admin surface는 의도적으로 data-heavy한 모듈이다. inbound admin route/template과 planning application
 * service 사이의 안정적인 JSON/view 계약을 여기에 모아두면, domain document는 service 경계 뒤에 남고 admin
 * UI는 editor-friendly label, markdown body, 요약, mutation form만 다룬다. 이 계층이 얇아 보여도 중요한
 * 이유는 route가 domain enum이나 persistence snapshot을 직접 serialize하기 시작하면 authority format 변경이
 * 곧바로 admin API breaking change가 되기 때문이다.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningAdminDraftKind {
    FullPlanning,
    QueueIdlePrompt,
    DirectionDetail,
}
impl PlanningAdminDraftKind {
    pub fn label(self) -> &'static str {
        // label은 notice와 inline copy에 들어가는 작은 문자열이다. page heading처럼 title case를 강제하지 않고
        // lower-case 문구로 고정해 draft 종류가 flash message 안에서 자연스럽게 이어지게 한다.
        match self {
            Self::FullPlanning => "full planning",
            Self::QueueIdlePrompt => "queue-idle prompt",
            Self::DirectionDetail => "direction detail",
        }
    }
    pub fn editor_heading(self) -> &'static str {
        // draft kind는 editor shell의 제목과 navigation 소유권을 함께 결정한다. full planning은 overview에서
        // 시작한 전체 편집이고, queue-idle/detail draft는 directions 영역의 세부 문서 편집이다.
        match self {
            Self::FullPlanning => "Full Planning Draft",
            Self::QueueIdlePrompt => "Queue-Idle Prompt Draft",
            Self::DirectionDetail => "Direction Detail Draft",
        }
    }
    pub fn return_path(self) -> &'static str {
        // return path는 backing document의 소유 화면을 따른다. editor가 저장/취소 후 돌아갈 곳을 route가 따로
        // 추론하지 않게 enum 표면에서 한 번만 정의한다.
        match self {
            Self::FullPlanning => "/admin",
            Self::QueueIdlePrompt | Self::DirectionDetail => "/admin/directions",
        }
    }
    pub fn slug(self) -> &'static str {
        // slug는 route parameter와 draft directory name으로 쓰이는 persisted-ish 값이다. Display가 debug name이
        // 아니라 이 slug를 위임하게 해 Rust enum 이름 변경이 외부 path 계약을 흔들지 않게 한다.
        match self {
            Self::FullPlanning => "full_planning",
            Self::QueueIdlePrompt => "queue_idle_prompt",
            Self::DirectionDetail => "direction_detail",
        }
    }
}
impl fmt::Display for PlanningAdminDraftKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PlanningAdminFileKey {
    ResultOutput,
    QueueIdlePrompt,
    DirectionDetail,
}
impl PlanningAdminFileKey {
    pub fn label(self) -> &'static str {
        // file key label은 draft session 안의 editor pane 제목이다. active path보다 짧은 표시명을 내려 UI가
        // filesystem layout을 그대로 노출하지 않아도 된다.
        match self {
            Self::ResultOutput => "Result Output",
            Self::QueueIdlePrompt => "Queue-Idle Prompt",
            Self::DirectionDetail => "Direction Detail",
        }
    }
    pub fn editor_language(self) -> &'static str {
        // 지금 admin planning file은 모두 markdown이지만, editor contract는 file key에서 language를 얻는다.
        // 나중에 JSON/YAML pane이 생겨도 route/template은 path suffix를 검사하지 않고 이 메서드만 따르면 된다.
        match self {
            Self::ResultOutput | Self::QueueIdlePrompt | Self::DirectionDetail => "markdown",
        }
    }
    pub fn slug(self) -> &'static str {
        // file key slug는 request body의 key와 draft file 식별자로 쓰인다. enum variant 이름 대신 명시 문자열을
        // 유지해 serde rename, form field, editor state가 같은 값을 공유하게 한다.
        match self {
            Self::ResultOutput => "result_output",
            Self::QueueIdlePrompt => "queue_idle_prompt",
            Self::DirectionDetail => "direction_detail",
        }
    }
}
impl fmt::Display for PlanningAdminFileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

/*
 * draft request는 committed authority가 아니라 임시 editing session을 가리킨다. draft_name은 staged file
 * directory를 찾는 handle이고, kind/direction_id는 그 directory 안에서 어떤 editor surface를 노출할지 정한다.
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDraftLoadRequest {
    pub draft_name: String,
    pub kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub direction_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDraftFileUpdate {
    pub key: PlanningAdminFileKey,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDraftMutationRequest {
    pub draft_name: String,
    pub kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub direction_id: Option<String>,
    #[serde(default)]
    pub files: Vec<PlanningAdminDraftFileUpdate>,
}

/*
 * validation view는 parser/consistency diagnostics를 UI-ready 문자열로 평탄화한 계약이다. admin layer는 count와
 * file별 issue를 렌더링하지만, domain error enum이나 PlanningFileKind variant를 route/template에 직접 노출하지
 * 않는다. 실제 severity 판정은 projection 쪽 validation report가 authority로 유지한다.
 */
#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminValidationIssueView {
    pub severity: String,
    pub file_kind: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminValidationView {
    pub is_valid: bool,
    pub error_count: usize,
    pub warning_count: usize,
    pub issues: Vec<PlanningAdminValidationIssueView>,
}

/*
 * queue preview는 domain queue state의 read-only projection이다. 일반 row는 목록 스캔에 필요한 최소 필드만
 * 갖고, queue head만 rank_reasons를 포함한다. 다음 handoff가 왜 선택됐는지 설명하는 정보는 중요하지만 모든 row에
 * 실으면 overview와 draft validation 응답이 불필요하게 커진다.
 */
#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminQueueTaskView {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: String,
    pub combined_priority: i32,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminQueueHeadView {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: String,
    pub combined_priority: i32,
    pub updated_at: String,
    pub rank_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminQueuePreview {
    pub queue_summary: String,
    pub proposal_summary: Option<String>,
    pub queue_head: Option<PlanningAdminQueueHeadView>,
    pub visible_tasks: Vec<PlanningAdminQueueTaskView>,
    pub proposed_tasks: Vec<PlanningAdminQueueTaskView>,
}

/*
 * draft session view는 editor pane과 live validation context를 한 응답에 결합한다. 저장된 staged file만
 * 보여주는 것이 아니라 같은 staged content로 validation/queue preview를 계산해, operator가 promote 전에
 * "지금 이 draft가 accepted authority로 들어가도 되는가"를 한 화면에서 판단하게 한다.
 */
#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDraftFileView {
    pub key: PlanningAdminFileKey,
    pub label: String,
    pub active_path: String,
    pub editor_language: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminSessionView {
    pub kind: PlanningAdminDraftKind,
    pub direction_id: Option<String>,
    pub draft_name: String,
    pub draft_directory: String,
    pub editor_heading: String,
    pub return_path: String,
    pub files: Vec<PlanningAdminDraftFileView>,
    pub validation: PlanningAdminValidationView,
    pub queue_preview: Option<PlanningAdminQueuePreview>,
}

/*
 * direction summary는 high-level directions page의 목록 계약이다. detail document health, queue-idle prompt
 * 상태, detail editor로 이동하는 데 필요한 최소 identity만 내려보낸다. accepted direction catalog 전체를
 * 노출하지 않으므로 direction 문서가 커져도 목록 화면의 response shape는 작게 유지된다.
 */
#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDirectionSummaryView {
    pub id: String,
    pub title: String,
    pub detail_doc_path: Option<String>,
    pub detail_doc_status: String,
    pub needs_attention: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDirectionsSummaryView {
    pub missing_detail_doc_count: usize,
    pub broken_detail_doc_count: usize,
    pub queue_idle_policy: String,
    pub queue_idle_prompt_path: Option<String>,
    pub queue_idle_prompt_status: String,
    pub parse_error: Option<String>,
    pub directions: Vec<PlanningAdminDirectionSummaryView>,
}

/*
 * management view는 accepted direction/task authority를 form-shaped snapshot으로 바꾼 값이다. list field는
 * 이미 text block으로 join되어 있어 inbound route가 mutation request와 같은 payload shape를 왕복시킬 수 있다.
 * 이 계약 덕분에 template은 domain Vec<String>과 textarea 사이의 변환 규칙을 몰라도 된다.
 */
#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDirectionManagementView {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub success_criteria_text: String,
    pub scope_hints_text: String,
    pub detail_doc_path: String,
    pub state: String,
    pub task_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminTaskManagementView {
    pub id: String,
    pub direction_id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub base_priority: i32,
    pub dynamic_priority_delta: i32,
    pub priority_reason: String,
    pub depends_on_text: String,
    pub blocked_by_text: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminManagementView {
    pub default_direction_id: String,
    pub directions: Vec<PlanningAdminDirectionManagementView>,
    pub tasks: Vec<PlanningAdminTaskManagementView>,
}

/*
 * mutation request는 validation/coercion 이전의 browser form payload를 그대로 닮는다. 숫자와 status도 먼저
 * String으로 받아 admin CRUD service에서 typed command로 변환하므로, 잘못된 입력에 대해 field 문맥이 살아 있는
 * operator-facing 오류를 만들 수 있다.
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDirectionMutationRequest {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub success_criteria_text: String,
    #[serde(default)]
    pub scope_hints_text: String,
    #[serde(default)]
    pub detail_doc_path: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDirectionDeleteRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminTaskMutationRequest {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub direction_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub base_priority: String,
    #[serde(default)]
    pub dynamic_priority_delta: String,
    #[serde(default)]
    pub priority_reason: String,
    #[serde(default)]
    pub depends_on_text: String,
    #[serde(default)]
    pub blocked_by_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminTaskDeleteRequest {
    pub id: String,
}

/*
 * command outcome은 operator notice와 갱신된 admin surface를 함께 돌려준다. mutation 후 caller가 별도 reload를
 * 하지 않아도 최신 management/path 상태를 렌더링할 수 있고, notice는 실제 적용된 command 결과를 짧게 설명한다.
 */
#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminCrudOutcome {
    pub notice: String,
    pub management: PlanningAdminManagementView,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminFileSyncOutcome {
    pub notice: String,
    pub paths: Vec<String>,
}

/*
 * overview summary는 admin landing page가 필요로 하는 doctor/runtime/queue/direction 상태를 축약한다. raw
 * document를 피하는 이유는 방향 카탈로그나 task authority format이 바뀌어도 overview가 같은 표시 계약을 유지해야
 * 하기 때문이다. 세부 편집은 management/draft surface가 맡고, overview는 운영 상태 판단에 집중한다.
 */
#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDoctorSummary {
    pub planning_state: String,
    pub queue_idle_policy: Option<String>,
    pub queue_summary: Option<String>,
    pub proposal_summary: Option<String>,
    pub health: Option<String>,
    pub issue: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminRuntimeSummary {
    pub workspace_present: bool,
    pub task_authority_signature: Option<u64>,
    pub queue_head_task_signature: Option<u64>,
    pub preview_status_label: String,
    pub preview_detail: Option<String>,
    pub queue_head: Option<PlanningAdminQueueHeadView>,
    pub visible_tasks: Vec<PlanningAdminQueueTaskView>,
    pub proposed_tasks: Vec<PlanningAdminQueueTaskView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminOverview {
    pub workspace_dir: String,
    pub doctor: PlanningAdminDoctorSummary,
    pub runtime: PlanningAdminRuntimeSummary,
    pub directions: Option<PlanningAdminDirectionsSummaryView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminResetOutcome {
    pub target: String,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
    pub doctor: PlanningAdminDoctorSummary,
}

#[cfg(test)]
mod tests {
    use super::{PlanningAdminDraftKind, PlanningAdminFileKey};

    #[test]
    fn draft_kind_contracts_keep_labels_routes_and_slugs_stable() {
        let cases = [
            (
                PlanningAdminDraftKind::FullPlanning,
                "full planning",
                "Full Planning Draft",
                "/admin",
                "full_planning",
            ),
            (
                PlanningAdminDraftKind::QueueIdlePrompt,
                "queue-idle prompt",
                "Queue-Idle Prompt Draft",
                "/admin/directions",
                "queue_idle_prompt",
            ),
            (
                PlanningAdminDraftKind::DirectionDetail,
                "direction detail",
                "Direction Detail Draft",
                "/admin/directions",
                "direction_detail",
            ),
        ];

        for (kind, label, heading, return_path, slug) in cases {
            assert_eq!(kind.label(), label);
            assert_eq!(kind.editor_heading(), heading);
            assert_eq!(kind.return_path(), return_path);
            assert_eq!(kind.slug(), slug);
            assert_eq!(kind.to_string(), slug);
        }
    }

    #[test]
    fn file_key_contracts_keep_editor_labels_languages_and_slugs_stable() {
        let cases = [
            (
                PlanningAdminFileKey::ResultOutput,
                "Result Output",
                "markdown",
                "result_output",
            ),
            (
                PlanningAdminFileKey::QueueIdlePrompt,
                "Queue-Idle Prompt",
                "markdown",
                "queue_idle_prompt",
            ),
            (
                PlanningAdminFileKey::DirectionDetail,
                "Direction Detail",
                "markdown",
                "direction_detail",
            ),
        ];

        for (key, label, language, slug) in cases {
            assert_eq!(key.label(), label);
            assert_eq!(key.editor_language(), language);
            assert_eq!(key.slug(), slug);
            assert_eq!(key.to_string(), slug);
        }
    }
}

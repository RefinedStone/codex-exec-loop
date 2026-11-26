use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::domain::issue_creation::{
    IssueCreatedLocation, IssueCreationApproval, IssueCreationIntentState, IssueRepositoryReference,
};

use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::policy::{
    IssueAutomationGate, IssueAutomationGateAction, IssueAutomationRiskClass,
};
use crate::domain::issue_automation::triage::{IssueTriageReadiness, IssueTriageReasonCode};

use crate::domain::issue_policy::{IssueSensitiveSpanAction, IssueSensitiveSpanClass};
use crate::domain::issue_publish_intent::IssuePublishIntentState;
use crate::domain::issue_work::{IssueFieldHash, IssueSharedCopy};

// 이슈 초안 inbound 계약이다 (#2426). Gateway 의 `akra/issue` 메서드들이 쓴다. 클라이언트는 로컬 글과 민감 구간을
// 보내고 결과를 그린다. 검증, 마스킹, 승인 검사, 게시 의도 기록은 이 계약 뒤의 서비스가 한다.
//
// 로컬 글이 있는 요청 타입에는 `Debug` 와 `Serialize` 가 없다. 로그와 응답에 들어가지 않는다.

/// 사용자가 고른 이슈와 그 이슈를 쓰는 로컬 프로젝트이다.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftTarget {
    /// 프로젝트의 작업 폴더이다. 이슈 정책과 템플릿을 여기서 읽는다.
    pub cwd: String,
    pub owner: String,
    pub repository: String,
    pub number: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueDraftField {
    Title,
    Body,
}

/// 사용자가 지정한 민감 구간이다. 위치는 그 칸의 글에서 본 UTF-16 코드 단위이다 (클라이언트 입력창의 단위).
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftSpan {
    pub field: IssueDraftField,
    pub start: usize,
    pub end: usize,
    /// 구간의 종류이다. 모르면 `None` 이다.
    #[serde(default)]
    pub class: Option<IssueSensitiveSpanClass>,
    /// 사용자의 답이다. 정책이 묻는 구간과 종류를 모르는 구간에 쓴다.
    #[serde(default)]
    pub decision: Option<IssueSensitiveSpanAction>,
}

/// 사용자의 로컬 글이다. 가리기 전의 값이다.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftText {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub spans: Vec<IssueDraftSpan>,
}

/// 사용자가 미리보기에서 승인한 값이다. 미리보기가 준 값을 그대로 돌려보낸다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftApproval {
    pub shared_copy_hash: IssueFieldHash,
    pub policy_version: String,
    pub template_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftRequiredInput {
    pub label: String,
    /// `section`, `input`, `textarea`, `dropdown`, `checkbox` 중 하나이다.
    pub kind: &'static str,
    /// `dropdown` 의 선택지 또는 `checkbox` 의 꼭 체크할 항목이다.
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftUnsupportedInput {
    pub label: String,
    pub reason: String,
}

/// 템플릿을 읽은 결과이다. 읽지 못한 템플릿은 "요구 없음"이 아니다: 검증기가 없어서 게시를 막는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftTemplate {
    /// `not_configured`, `resolved`, `missing`, `unreadable`, `invalid` 또는 생성 결과 전용 `recovery_only` 이다.
    pub status: &'static str,
    pub path: Option<String>,
    pub reason: Option<String>,
    pub required_inputs: Vec<IssueDraftRequiredInput>,
    /// 검증기가 검사하지 못하는 필수 입력이다. 사용자가 직접 확인한다.
    pub unsupported_inputs: Vec<IssueDraftUnsupportedInput>,
}

/// 게시 정책이 민감 구간의 종류마다 하는 처리이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct IssueDraftSpanActions {
    pub secret: IssueSensitiveSpanAction,
    pub name: IssueSensitiveSpanAction,
    pub path: IssueSensitiveSpanAction,
    pub log: IssueSensitiveSpanAction,
}

/// 이 이슈의 가장 최근 게시 의도이다. 글은 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftIntent {
    pub intent_id: String,
    pub state: IssuePublishIntentState,
}

/// 편집을 시작할 때의 값이다. `title` 과 `body` 는 지금 원격의 글이고, 민감 입력 경로로만 나간다.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftOpened {
    pub title: String,
    pub body: String,
    /// 그 원격 글의 필드 해시이다. 게시 요청에 그대로 돌려보낸다.
    pub edit_basis: IssueFieldHash,
    pub template: IssueDraftTemplate,
    /// 프로젝트 정책이 요구하는 섹션의 제목이다.
    pub required_sections: Vec<String>,
    pub span_actions: IssueDraftSpanActions,
    pub intent: Option<IssueDraftIntent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftValidationError {
    /// `title`, `template_field`, `required_section`, `code_validator` 중 하나이다.
    pub field: &'static str,
    /// 템플릿 필드나 섹션의 제목 또는 코드 검증기의 ID 이다. 제목 칸이면 `None` 이다.
    pub label: Option<String>,
    /// `missing`, `empty`, `invalid_format`, `failed`(필수 코드 검증기가 통과하지 않았다),
    /// `cancelled`(사용자가 코드 검증을 취소했다) 중 하나이다.
    pub kind: &'static str,
}

/// 공유본에 남는 가림 표식이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftMarker {
    pub marker: String,
    pub field: IssueDraftField,
}

/// 준비검사는 현재 AI 전달 공유본에 대한 예상이다. 게시 승인, 실제 라벨, 실행 허가를 만들지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IssueDraftReadiness {
    Assessed {
        assessment: Box<IssueDraftReadinessAssessment>,
    },
    #[serde(rename_all = "camelCase")]
    CannotAssess {
        reason: IssueDraftReadinessBlocker,
        summary: &'static str,
        next_summary: &'static str,
        policy_error: Option<IssueDraftReadinessPolicyError>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftReadinessAssessment {
    pub preview_only: bool,
    pub readiness: IssueTriageReadiness,
    pub summary: &'static str,
    pub missing_sections: Vec<String>,
    pub errors: Vec<IssueDraftValidationError>,
    pub risk_class: IssueAutomationRiskClass,
    pub risk_summary: &'static str,
    pub effective_mode: IssueAutomationAutonomy,
    pub assumed_mode: IssueAutomationAutonomy,
    pub mode_summary: String,
    pub expected_gates: Vec<IssueDraftExpectedGate>,
    pub reasons: Vec<IssueDraftReadinessReason>,
    pub policy_version: String,
    pub disclosure_policy_version: String,
    /// 준비검사에 사용한 AI 전달 공유본의 해시이다. 게시 승인 해시와 구별한다.
    pub shared_copy_hash: IssueFieldHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftExpectedGate {
    pub gate: IssueAutomationGate,
    pub action: IssueAutomationGateAction,
    pub summary: &'static str,
    pub action_summary: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftReadinessReason {
    pub code: IssueTriageReasonCode,
    pub summary: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueDraftReadinessBlocker {
    AutomationPolicyUnavailable,
    PolicyBlocked,
    RequirementsUnavailable,
    DisclosureConfirmationRequired,
    InvalidSpan,
    InvalidLabels,
    FrozenDraft,
    InvalidInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftReadinessPolicyError {
    pub scope: &'static str,
    pub field: &'static str,
    pub reason: &'static str,
}

/// 미리보기이다. 로컬 글과 가린 평문은 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IssueDraftPreview {
    /// 공유본을 만들었다.
    #[serde(rename_all = "camelCase")]
    Ready {
        /// 게시될 제목과 본문이다.
        shared_copy: IssueSharedCopy,
        markers: Vec<IssueDraftMarker>,
        errors: Vec<IssueDraftValidationError>,
        /// 템플릿을 읽지 못해서 검증기가 없다. 게시를 막는다.
        validator_absent: bool,
        /// 오류가 없고 검증기가 있다.
        publishable: bool,
        /// 사용자가 승인하면 게시 요청에 그대로 넣는 값이다.
        approval: IssueDraftApproval,
        #[serde(skip_serializing_if = "Option::is_none")]
        readiness: Option<Box<IssueDraftReadiness>>,
    },
    /// 사용자의 답이 필요한 구간이 있다. 값은 요청 `spans` 의 번호이다.
    #[serde(rename_all = "camelCase")]
    NeedsConfirmation {
        span_indexes: Vec<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        readiness: Option<Box<IssueDraftReadiness>>,
    },
    /// 범위가 틀린 구간이다.
    #[serde(rename_all = "camelCase")]
    InvalidSpan {
        span_index: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        readiness: Option<Box<IssueDraftReadiness>>,
    },
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftPreviewRequest {
    pub target: IssueDraftTarget,
    pub text: IssueDraftText,
    /// 준비검사에만 쓰는 가정이다. 외부 라벨을 바꾸지 않는다.
    #[serde(default)]
    pub expected_labels: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraftPublishRequest {
    pub target: IssueDraftTarget,
    pub text: IssueDraftText,
    pub approval: IssueDraftApproval,
    /// `IssueDraftOpened::edit_basis` 이다.
    pub edit_basis: IssueFieldHash,
    /// 사용자가 복원 연결 없이 공유본만 게시한다고 확인했으면 `true` 이다.
    #[serde(default)]
    pub shared_copy_only: bool,
    /// 클라이언트가 정한 검증 ID 이다. 주면 실행 중인 코드 검증을 `cancel_validation` 으로 취소할 수 있다.
    #[serde(default)]
    pub validation_id: Option<String>,
}

/// 게시 요청의 결과이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IssueDraftPublished {
    /// 게시 확인. 원격의 글이 승인한 공유본과 같다.
    Confirmed { intent: IssueDraftIntent },
    /// 게시 결과 확인 필요. 쓰기를 보냈고 결과를 모른다. 앞의 의도가 남아 있을 때도 이 값이다.
    NeedsConfirmation { intent: IssueDraftIntent },
    /// 복원 가능 구간을 저장하지 못했다. 보내지 않았다.
    Held { intent: IssueDraftIntent },
    /// 알려진 외부 변경. 원격 글이 편집 기준과 다르다. 쓰지 않았다.
    Conflict,
    /// 승인이 지금 보낼 값과 다르다. `reason` 은 `target`, `shared_copy`, `policy_version`, `template_version` 중 하나이다.
    Refused { reason: &'static str },
    /// 공유본을 만들지 못했거나 검증을 통과하지 못했다. 보내지 않았다.
    Blocked { preview: IssueDraftPreview },
}

// 새 이슈 생성도 같은 초안 Port의 승인·마스킹 경계를 사용한다.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationTarget {
    pub cwd: String,
    pub owner: String,
    pub repository: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationIntent {
    pub intent_id: String,
    pub state: IssueCreationIntentState,
    pub created: Option<IssueCreatedLocation>,
    pub shared_copy: Option<IssueSharedCopy>,
    pub restore_link_required: bool,
    pub restore_link_finalized: bool,
    pub predecessor_intent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationOpened {
    pub repository: IssueRepositoryReference,
    pub intent: IssueCreationIntent,
    pub title: String,
    pub body: String,
    pub template: IssueDraftTemplate,
    pub required_sections: Vec<String>,
    pub span_actions: IssueDraftSpanActions,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationPreviewRequest {
    pub target: IssueCreationTarget,
    pub intent_id: String,
    pub text: IssueDraftText,
    #[serde(default)]
    pub expected_labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationPreview {
    pub preview: IssueDraftPreview,
    pub approval: Option<IssueCreationApproval>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationPublishRequest {
    pub target: IssueCreationTarget,
    pub intent_id: String,
    pub text: IssueDraftText,
    pub approval: IssueCreationApproval,
    #[serde(default)]
    pub shared_copy_only: bool,
    /// 클라이언트가 정한 검증 ID 이다. 주면 실행 중인 코드 검증을 `cancel_validation` 으로 취소할 수 있다.
    #[serde(default)]
    pub validation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IssueCreationPublished {
    Confirmed { intent: IssueCreationIntent },
    NeedsConfirmation { intent: IssueCreationIntent },
    Held { intent: IssueCreationIntent },
    Refused { reason: &'static str },
    Blocked { preview: IssueCreationPreview },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationIntentRequest {
    pub target: IssueCreationTarget,
    pub intent_id: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationConfirmRequest {
    pub target: IssueCreationTarget,
    pub intent_id: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationRestartRequest {
    pub target: IssueCreationTarget,
    pub previous_intent_id: String,
    /// An unresolved predecessor may already exist on GitHub. Never infer this consent.
    #[serde(default)]
    pub acknowledge_uncertain: bool,
}

pub struct PreparedIssueDraftReview {
    pub workspace: String,
    pub draft_json: String,
}

pub trait IssueDraftPort: Send + Sync {
    /// Builds the delegation input from a freshly validated, masked shared copy.
    fn prepare_review_draft(
        &self,
        _request: IssueDraftPreviewRequest,
        _approval: IssueDraftApproval,
    ) -> Result<PreparedIssueDraftReview> {
        anyhow::bail!("issue draft review is unavailable")
    }

    fn creation_open(&self, _target: IssueCreationTarget) -> Result<IssueCreationOpened> {
        anyhow::bail!("new-issue creation is unavailable")
    }

    fn creation_preview(
        &self,
        _request: IssueCreationPreviewRequest,
    ) -> Result<IssueCreationPreview> {
        anyhow::bail!("new-issue creation is unavailable")
    }

    fn creation_publish(
        &self,
        _request: IssueCreationPublishRequest,
    ) -> Result<IssueCreationPublished> {
        anyhow::bail!("new-issue creation is unavailable")
    }

    fn creation_recheck(
        &self,
        _request: IssueCreationIntentRequest,
    ) -> Result<IssueCreationIntent> {
        anyhow::bail!("new-issue creation is unavailable")
    }

    fn creation_confirm(
        &self,
        _request: IssueCreationConfirmRequest,
    ) -> Result<IssueCreationIntent> {
        anyhow::bail!("new-issue creation is unavailable")
    }

    fn creation_restart(
        &self,
        _request: IssueCreationRestartRequest,
    ) -> Result<IssueCreationOpened> {
        anyhow::bail!("new-issue creation is unavailable")
    }

    /// 편집을 시작한다. 원격 글, 요구 사항, 이 이슈의 게시 의도 상태를 준다. 쓰지 않는다.
    fn open(&self, target: IssueDraftTarget) -> Result<IssueDraftOpened>;

    /// 로컬 글을 가리고 검사한다. 쓰지 않는다.
    fn preview(&self, request: IssueDraftPreviewRequest) -> Result<IssueDraftPreview>;

    /// 승인한 공유본을 게시 의도 기록 뒤에 게시한다.
    fn publish(&self, request: IssueDraftPublishRequest) -> Result<IssueDraftPublished>;

    /// 이 이슈의 게시 의도를 원격과 다시 맞춘다. 읽기만 한다.
    fn recheck(&self, target: IssueDraftTarget) -> Result<Option<IssueDraftIntent>>;

    /// 사용자가 결과 확인을 그만둔다.
    fn abandon(&self, cwd: &str, intent_id: &str) -> Result<IssueDraftIntent>;

    /// 실행 중인 게시 검증을 취소한다. 받으면 `true`, 이미 끝났거나 모르는 ID 이면 `false` 이다.
    fn cancel_validation(&self, _validation_id: &str) -> Result<bool> {
        anyhow::bail!("issue validation cancel is unavailable")
    }
}

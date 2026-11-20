//! 판정의 안전한 조회 DTO다. 기준 문장·원문·과거 공유본은 포함하지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::{NextStep, reason_next};
use super::IssueHashState;
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::collection::{
    IssueTriageObserveReport, IssueTriageProjection,
};
use crate::domain::issue_automation::policy::{
    IssueAutomationGate, IssueAutomationGateAction, IssueAutomationRiskClass,
};
use crate::domain::issue_automation::triage::{
    IssueObservationDisposition, IssueTriageEligibility, IssueTriageReadiness,
    IssueTriageReasonCode, IssueTriageSourceState,
};
use crate::domain::kernel_event::IssueEventReference;
use crate::domain::kernel_spec::ReasonCode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueTriageState {
    pub revision: u64,
    pub observation_revision: Option<u64>,
    pub tracked_revision: Option<u64>,
    pub stale: bool,
    pub eligibility: IssueTriageEligibilityState,
    pub readiness: IssueTriageReadinessState,
    pub risk_class: IssueTriageRiskState,
    pub effective_mode: IssueTriageModeState,
    /// 자동화 정책의 판이다. 공개 정책 판과 구별한다.
    pub policy_version: String,
    /// provider 원본의 제목/본문 지문이며 공유본 해시가 아니다.
    pub field_hash: Option<IssueHashState>,
    pub basis: IssueTriageBasisState,
    pub gates: Vec<IssueTriageGateState>,
    pub reasons: Vec<IssueTriageReasonState>,
    pub observation: IssueTriageObservationState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueTriageBasisState {
    pub version: u32,
    pub issue: IssueEventReference,
    pub source: IssueTriageSourceStateValue,
    pub disclosure_policy_version: Option<String>,
    pub validation_policy_version: String,
    pub evidence_sha256: String,
    pub requirements_sha256: String,
    pub pinned_source_commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueTriageGateState {
    pub gate: IssueTriageGateKind,
    pub action: IssueTriageGateActionState,
    /// 판정의 처리 방식이며 실행 허가가 아니다. Off/Observe와 미연결 close는 false다.
    pub enabled: bool,
    pub gate_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueTriageObservationState {
    pub observation_only: bool,
    pub assumed_mode: IssueTriageModeState,
    pub disposition: IssueObservationDispositionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueTriageReasonState {
    pub code: ReasonCode,
    pub summary: String,
    pub next: Vec<NextStep>,
}

impl From<&IssueTriageProjection> for IssueTriageState {
    fn from(projection: &IssueTriageProjection) -> Self {
        let assessment = &projection.assessment;
        let basis = assessment.basis();
        let observation = assessment.observation();
        Self {
            revision: projection.revision,
            observation_revision: projection.observation_revision,
            tracked_revision: projection.tracked_revision,
            stale: projection.stale,
            eligibility: assessment.eligibility().into(),
            readiness: assessment.readiness().into(),
            risk_class: assessment.risk_class().into(),
            effective_mode: assessment.effective_mode().into(),
            policy_version: basis.automation_policy_version.clone(),
            field_hash: basis.field_hash.as_ref().map(|hash| IssueHashState {
                version: hash.version,
                sha256: hash.sha256.clone(),
            }),
            basis: IssueTriageBasisState {
                version: basis.version,
                issue: (&basis.issue).into(),
                source: basis.source.into(),
                disclosure_policy_version: basis.disclosure_policy_version.clone(),
                validation_policy_version: basis.validation_policy_version.clone(),
                evidence_sha256: basis.evidence_sha256.clone(),
                requirements_sha256: basis.requirements_sha256.clone(),
                pinned_source_commit: basis.pinned_source_commit.clone(),
            },
            gates: assessment
                .gates()
                .iter()
                .map(|gate| IssueTriageGateState {
                    gate: gate.gate.into(),
                    action: gate.action.into(),
                    enabled: !projection.stale && gate.enabled,
                    gate_revision: gate.gate_revision,
                })
                .collect(),
            reasons: assessment
                .reasons()
                .iter()
                .copied()
                .map(Into::into)
                .collect(),
            observation: IssueTriageObservationState {
                observation_only: observation.observation_only,
                assumed_mode: observation.assumed_mode.into(),
                disposition: observation.disposition.into(),
            },
        }
    }
}

pub(super) fn reason_summary(reason: IssueTriageReasonCode) -> &'static str {
    reason_parts(reason).1
}

fn reason_parts(reason: IssueTriageReasonCode) -> (ReasonCode, &'static str) {
    match reason {
        IssueTriageReasonCode::NotYetObserved => (
            ReasonCode::NotYetObserved,
            "아직 현재 이슈 정보를 읽지 않았습니다.",
        ),
        IssueTriageReasonCode::SourceUnavailable => (
            ReasonCode::SourceUnavailable,
            "현재 이슈 정보를 읽지 못했습니다.",
        ),
        IssueTriageReasonCode::MetadataIncomplete => (
            ReasonCode::MetadataIncomplete,
            "판정에 필요한 메타데이터를 모두 확인하지 못했습니다.",
        ),
        IssueTriageReasonCode::DisclosurePolicyStale => (
            ReasonCode::DisclosurePolicyStale,
            "공개 정책이 바뀌어 공유본을 다시 확인해야 합니다.",
        ),
        IssueTriageReasonCode::SelectionNotMatched => (
            ReasonCode::SelectionNotMatched,
            "설정한 이슈 선택 조건에 맞지 않습니다.",
        ),
        IssueTriageReasonCode::IssueClosed => {
            (ReasonCode::IssueClosed, "원격 이슈가 닫혀 있습니다.")
        }
        IssueTriageReasonCode::UntrustedActor => (
            ReasonCode::UntrustedActor,
            "신뢰하는 작성자나 현재 라벨 행위자를 확인하지 못했습니다.",
        ),
        IssueTriageReasonCode::AlreadyClaimed => (
            ReasonCode::AlreadyClaimed,
            "이미 담당자나 연결된 PR이 있습니다.",
        ),
        IssueTriageReasonCode::TitleMissing => {
            (ReasonCode::TitleMissing, "공유본의 제목이 비어 있습니다.")
        }
        IssueTriageReasonCode::RequiredSectionMissing => (
            ReasonCode::RequiredSectionMissing,
            "공유본에 필수 섹션이 없거나 비어 있습니다.",
        ),
        IssueTriageReasonCode::CriteriaMissing => {
            (ReasonCode::CriteriaMissing, "완료 기준이 없습니다.")
        }
        IssueTriageReasonCode::CriteriaNotList => (
            ReasonCode::CriteriaNotList,
            "완료 기준을 지원하는 Markdown 목록으로 읽을 수 없습니다.",
        ),
        IssueTriageReasonCode::IssueTooLarge => (
            ReasonCode::IssueTooLarge,
            "이슈 내용이 판정 크기 한도를 넘었습니다.",
        ),
        IssueTriageReasonCode::UnsupportedContent => (
            ReasonCode::UnsupportedContent,
            "현재 지원하지 않는 내용이 있어 자동으로 판정할 수 없습니다.",
        ),
        IssueTriageReasonCode::DisclosureConfirmationRequired => (
            ReasonCode::DisclosureConfirmationRequired,
            "내용 공개에 사용자 확인이 필요합니다.",
        ),
        IssueTriageReasonCode::MaskedContent => (
            ReasonCode::MaskedContent,
            "마스킹된 부분 때문에 내용을 충분히 판정할 수 없습니다.",
        ),
        IssueTriageReasonCode::HighRisk => (
            ReasonCode::HighRisk,
            "위험 등급이 높아 모든 관문에 사람 확인이 필요합니다.",
        ),
        IssueTriageReasonCode::HeldOrManual => (
            ReasonCode::HeldOrManual,
            "보류 또는 수동 처리로 지정된 이슈입니다.",
        ),
        IssueTriageReasonCode::ProtectedPath => (
            ReasonCode::ProtectedPath,
            "언급한 범위에 보호 경로가 포함되어 있습니다.",
        ),
        IssueTriageReasonCode::PolicyHumanRequired => (
            ReasonCode::PolicyHumanRequired,
            "정책의 추가 규칙이 해당 관문에 사람 확인을 요구합니다.",
        ),
        IssueTriageReasonCode::BudgetExhausted => (
            ReasonCode::BudgetExhausted,
            "예산 상한이 0이어서 자동 시작할 수 없습니다.",
        ),
        IssueTriageReasonCode::ModeOff => {
            (ReasonCode::ModeOff, "유효 자동화 수준이 꺼져 있습니다.")
        }
        IssueTriageReasonCode::ObservationOnly => (
            ReasonCode::ObservationOnly,
            "관찰 결과이며 작업이나 실행 허가를 만들지 않습니다.",
        ),
        IssueTriageReasonCode::ExecutionGrantRequired => (
            ReasonCode::ExecutionGrantRequired,
            "실행 허가 연결 전이므로 판정만으로 작업을 시작할 수 없습니다.",
        ),
        IssueTriageReasonCode::CloseDisabled => (
            ReasonCode::CloseDisabled,
            "이 판정에서 이슈 닫힘은 활성화되지 않습니다.",
        ),
    }
}

impl From<IssueTriageReasonCode> for IssueTriageReasonState {
    fn from(reason: IssueTriageReasonCode) -> Self {
        let (code, summary) = reason_parts(reason);
        Self {
            code,
            summary: summary.to_owned(),
            next: reason_next(code),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueTriageObserveReportState {
    /// 현재 페이지뿐 아니라 전체 조회 범위의 같은 장부 snapshot에서 계산한 수다.
    pub revision: u64,
    pub binding_id: Option<String>,
    pub policy_version: Option<String>,
    /// 전체 보고의 가정 상한이다. 각 행의 라벨이 더 낮출 수 있으며 실행 허가가 아니다.
    pub assumed_mode: Option<IssueTriageModeState>,
    pub policy_blocked: bool,
    pub total: u64,
    pub automatic_start: u64,
    pub human_approval: u64,
    pub deferred: u64,
    pub excluded: u64,
    pub needs_info: u64,
    pub unknown: u64,
    pub observation_only: u64,
    pub stale: u64,
    pub untriaged: u64,
    pub blocked: u64,
}

impl From<&IssueTriageObserveReport> for IssueTriageObserveReportState {
    fn from(report: &IssueTriageObserveReport) -> Self {
        Self {
            revision: report.revision,
            binding_id: report.binding_id.as_ref().map(ToString::to_string),
            policy_version: report.policy_version.clone(),
            assumed_mode: report.assumed_mode.map(Into::into),
            policy_blocked: report.policy_blocked,
            total: report.total,
            automatic_start: report.automatic_start,
            human_approval: report.human_approval,
            deferred: report.deferred,
            excluded: report.excluded,
            needs_info: report.needs_info,
            unknown: report.unknown,
            observation_only: report.observation_only,
            stale: report.stale,
            untriaged: report.untriaged,
            blocked: report.blocked,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueTriageEligibilityState {
    Eligible,
    Ineligible,
    Unknown,
}

impl From<IssueTriageEligibility> for IssueTriageEligibilityState {
    fn from(value: IssueTriageEligibility) -> Self {
        match value {
            IssueTriageEligibility::Eligible => Self::Eligible,
            IssueTriageEligibility::Ineligible => Self::Ineligible,
            IssueTriageEligibility::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueTriageReadinessState {
    Ready,
    NeedsInfo,
    Unknown,
}

impl From<IssueTriageReadiness> for IssueTriageReadinessState {
    fn from(value: IssueTriageReadiness) -> Self {
        match value {
            IssueTriageReadiness::Ready => Self::Ready,
            IssueTriageReadiness::NeedsInfo => Self::NeedsInfo,
            IssueTriageReadiness::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueTriageRiskState {
    Low,
    Normal,
    High,
}

impl From<IssueAutomationRiskClass> for IssueTriageRiskState {
    fn from(value: IssueAutomationRiskClass) -> Self {
        match value {
            IssueAutomationRiskClass::Low => Self::Low,
            IssueAutomationRiskClass::Normal => Self::Normal,
            IssueAutomationRiskClass::High => Self::High,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueTriageModeState {
    Off,
    Observe,
    Propose,
    Gated,
    Auto,
}

impl From<IssueAutomationAutonomy> for IssueTriageModeState {
    fn from(value: IssueAutomationAutonomy) -> Self {
        match value {
            IssueAutomationAutonomy::Off => Self::Off,
            IssueAutomationAutonomy::Observe => Self::Observe,
            IssueAutomationAutonomy::Propose => Self::Propose,
            IssueAutomationAutonomy::Gated => Self::Gated,
            IssueAutomationAutonomy::Auto => Self::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueTriageSourceStateValue {
    NotYetObserved,
    Unknown,
    Shared,
    Withheld,
}

impl From<IssueTriageSourceState> for IssueTriageSourceStateValue {
    fn from(value: IssueTriageSourceState) -> Self {
        match value {
            IssueTriageSourceState::NotYetObserved => Self::NotYetObserved,
            IssueTriageSourceState::Unknown => Self::Unknown,
            IssueTriageSourceState::Shared => Self::Shared,
            IssueTriageSourceState::Withheld => Self::Withheld,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueTriageGateKind {
    Start,
    Spec,
    Plan,
    Delivery,
    Merge,
    Close,
}

impl From<IssueAutomationGate> for IssueTriageGateKind {
    fn from(value: IssueAutomationGate) -> Self {
        match value {
            IssueAutomationGate::Start => Self::Start,
            IssueAutomationGate::Spec => Self::Spec,
            IssueAutomationGate::Plan => Self::Plan,
            IssueAutomationGate::Delivery => Self::Delivery,
            IssueAutomationGate::Merge => Self::Merge,
            IssueAutomationGate::Close => Self::Close,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueTriageGateActionState {
    Human,
    TimeoutDefault,
    TimeoutArbiter,
    Auto,
}

impl From<IssueAutomationGateAction> for IssueTriageGateActionState {
    fn from(value: IssueAutomationGateAction) -> Self {
        match value {
            IssueAutomationGateAction::Human => Self::Human,
            IssueAutomationGateAction::TimeoutDefault => Self::TimeoutDefault,
            IssueAutomationGateAction::TimeoutArbiter => Self::TimeoutArbiter,
            IssueAutomationGateAction::Auto => Self::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueObservationDispositionState {
    AutomaticStart,
    HumanApproval,
    Deferred,
    Excluded,
    NeedsInfo,
    Unknown,
    ObservationOnly,
}

impl From<IssueObservationDisposition> for IssueObservationDispositionState {
    fn from(value: IssueObservationDisposition) -> Self {
        match value {
            IssueObservationDisposition::AutomaticStart => Self::AutomaticStart,
            IssueObservationDisposition::HumanApproval => Self::HumanApproval,
            IssueObservationDisposition::Deferred => Self::Deferred,
            IssueObservationDisposition::Excluded => Self::Excluded,
            IssueObservationDisposition::NeedsInfo => Self::NeedsInfo,
            IssueObservationDisposition::Unknown => Self::Unknown,
            IssueObservationDisposition::ObservationOnly => Self::ObservationOnly,
        }
    }
}

#[cfg(test)]
mod tests;

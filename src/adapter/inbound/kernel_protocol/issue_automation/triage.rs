//! 판정의 안전한 조회 DTO다. 기준 문장·원문·과거 공유본은 포함하지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::{NextStep, reason_next};
use super::IssueHashState;
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::collection::{
    IssueCollectionItem, IssueTriageObserveReport, IssueTriageProjection,
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
    /// 서버가 정한 다음 행동 설명이다. renderer가 reason code에서 추론하지 않는다.
    pub next_summary: String,
    pub next: Vec<NextStep>,
}

impl IssueTriageState {
    /// Budget usage is a separate projection, never part of an approval fingerprint.
    pub(super) fn from_item(item: &IssueCollectionItem) -> Option<Self> {
        let projection = item.triage.as_ref()?;
        let mut state = Self::from(projection);
        if !projection.stale
            && let Some(wait) = &item.start_budget_wait
        {
            use crate::domain::issue_automation::coordinator::IssueStartBudgetLimit;
            let limits = wait
                .exhausted
                .iter()
                .map(|limit| match limit {
                    IssueStartBudgetLimit::Running => "동시 작업",
                    IssueStartBudgetLimit::StartedPerDay => "UTC 일일 시작",
                    IssueStartBudgetLimit::Attempts => "이슈별 시도",
                })
                .collect::<Vec<_>>()
                .join("·");
            state
                .reasons
                .retain(|reason| reason.code != ReasonCode::BudgetExhausted);
            state.reasons.insert(0, IssueTriageReasonState {
                code: ReasonCode::BudgetExhausted,
                summary: format!("{limits} 한도에 도달해 작업 시작을 기다립니다."),
                next_summary: "진행 중인 작업이 끝나거나 UTC 날짜가 바뀌면 현재 정책과 한도를 다시 확인합니다.".into(),
                next: reason_next(ReasonCode::BudgetExhausted),
            });
            state.observation.disposition = IssueObservationDispositionState::Deferred;
        }
        Some(state)
    }
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
        IssueTriageReasonCode::HumanSelectionRequired => (
            ReasonCode::HumanSelectionRequired,
            "사람이 선택 라벨을 적용한 근거가 필요합니다.",
        ),
        IssueTriageReasonCode::AutoRepositoryRestricted => (
            ReasonCode::AutoRepositoryRestricted,
            "공개 저장소이거나 저장소의 공개 범위와 출처를 확인할 수 없어 제안 단계로 제한합니다.",
        ),
        IssueTriageReasonCode::LinearPullRequestLookupUnsupported => (
            ReasonCode::LinearPullRequestLookupUnsupported,
            "Linear 연결에서 관련 PR의 유무를 확인할 수 없어 자동으로 시작하지 않습니다.",
        ),
        IssueTriageReasonCode::JiraPullRequestLookupUnsupported => (
            ReasonCode::JiraPullRequestLookupUnsupported,
            "Jira 연결에서 관련 PR의 유무를 확인할 수 없어 자동으로 시작하지 않습니다.",
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

fn reason_next_summary(reason: IssueTriageReasonCode) -> &'static str {
    match reason {
        IssueTriageReasonCode::NotYetObserved => "현재 이슈 정보를 동기화한 뒤 다시 확인하세요.",
        IssueTriageReasonCode::SourceUnavailable => {
            "공급자 연결과 접근 권한을 확인한 뒤 다시 동기화하세요."
        }
        IssueTriageReasonCode::MetadataIncomplete => {
            "라벨, 담당자와 연결된 PR 정보를 모두 읽을 수 있는지 확인한 뒤 다시 동기화하세요."
        }
        IssueTriageReasonCode::DisclosurePolicyStale => {
            "현재 공개 정책으로 이슈를 다시 동기화한 뒤 판정을 확인하세요."
        }
        IssueTriageReasonCode::SelectionNotMatched => {
            "이슈의 라벨과 정책의 선택 조건을 확인하세요."
        }
        IssueTriageReasonCode::IssueClosed => {
            "원격 이슈의 현재 상태와 후속 작업이 필요한지 확인하세요."
        }
        IssueTriageReasonCode::UntrustedActor => {
            "신뢰하는 작성자 또는 선택 라벨을 붙인 사람의 근거를 확인하세요."
        }
        IssueTriageReasonCode::AlreadyClaimed => {
            "기존 담당자나 연결된 PR의 진행 상황을 먼저 확인하세요."
        }
        IssueTriageReasonCode::TitleMissing => "작업을 구별할 수 있는 제목을 보완하세요.",
        IssueTriageReasonCode::RequiredSectionMissing => {
            "이슈에 정책이 요구하는 필수 섹션을 보완하세요."
        }
        IssueTriageReasonCode::CriteriaMissing => {
            "완료 여부를 확인할 수 있는 기준을 비어 있지 않은 목록으로 작성하세요."
        }
        IssueTriageReasonCode::CriteriaNotList => {
            "완료 기준을 글머리표, 번호 또는 체크 목록으로 정리하세요."
        }
        IssueTriageReasonCode::IssueTooLarge => "이슈를 검토할 수 있는 크기의 작업으로 나누세요.",
        IssueTriageReasonCode::UnsupportedContent => {
            "작업 내용을 지원하는 텍스트와 Markdown 형식으로 정리하세요."
        }
        IssueTriageReasonCode::DisclosureConfirmationRequired => {
            "공개 범위를 사람이 확인한 뒤 다시 동기화하세요."
        }
        IssueTriageReasonCode::MaskedContent => {
            "가린 공유본으로 작업 범위를 이해할 수 있는지 사람이 확인하세요."
        }
        IssueTriageReasonCode::HighRisk => "작업의 위험과 변경 범위를 사람이 검토하세요.",
        IssueTriageReasonCode::HeldOrManual => {
            "보류 또는 수동 처리 조건을 확인하고 사람의 결정을 기다리세요."
        }
        IssueTriageReasonCode::ProtectedPath => {
            "보호 경로의 변경 범위와 필요한 권한을 사람이 확인하세요."
        }
        IssueTriageReasonCode::PolicyHumanRequired => "정책이 요구하는 사람의 확인을 기다리세요.",
        IssueTriageReasonCode::HumanSelectionRequired => {
            "신뢰하는 사람이 현재 이슈에 선택 라벨을 적용한 뒤 다시 동기화하세요."
        }
        IssueTriageReasonCode::AutoRepositoryRestricted => {
            "원본 저장소의 현재 공개 범위와 연결을 확인하고 제안된 작업을 사람이 검토하세요."
        }
        IssueTriageReasonCode::JiraPullRequestLookupUnsupported
        | IssueTriageReasonCode::LinearPullRequestLookupUnsupported => {
            "진행 중인 PR이나 중복 작업이 없는지 직접 확인하세요. 다른 준비 조건이 모두 충족되면 현재 이슈의 시작 결정을 Native 확인으로 승인할 수 있습니다."
        }
        IssueTriageReasonCode::BudgetExhausted => {
            "예산과 시도 한도를 확인하고 사람의 검토를 기다리세요."
        }
        IssueTriageReasonCode::ModeOff => "관찰이 필요하면 정책의 동작 모드를 확인하세요.",
        IssueTriageReasonCode::ObservationOnly => {
            "관찰 보고를 확인하고 진행 수준과 사람의 확인 조건을 검토하세요."
        }
        IssueTriageReasonCode::ExecutionGrantRequired => {
            "작업을 실행하기 전에 승인과 실행 권한을 확인하세요."
        }
        IssueTriageReasonCode::CloseDisabled => "이슈를 닫아야 한다면 원격 상태를 직접 확인하세요.",
    }
}

impl From<IssueTriageReasonCode> for IssueTriageReasonState {
    fn from(reason: IssueTriageReasonCode) -> Self {
        let (code, summary) = reason_parts(reason);
        Self {
            code,
            summary: summary.to_owned(),
            next_summary: reason_next_summary(reason).to_owned(),
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

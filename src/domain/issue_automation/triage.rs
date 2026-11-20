//! 모델·시계·I/O 없이 현재 관찰과 정책으로 계산하는 이슈 판정이다 (#2541).
//! 관문은 설명용 정책 결과다. 실행 허가나 작업 수명 상태를 만들지 않는다.
use serde::{Deserialize, Serialize};

use crate::domain::issue_validation::IssueRequirements;
use crate::domain::issue_work::{IssueFieldHash, IssueReference};

use super::policy::{
    IssueAutomationGate, IssueAutomationGateAction, IssueAutomationPolicy, IssueAutomationRiskClass,
};
use super::source::IssueSourceRead;
use super::{IssueAutomationAutonomy, MAX_ISSUE_AUTOMATION_REVISION};

mod basis;
mod criteria;
mod evaluate;
mod gates;
mod paths;
mod readiness;

pub use readiness::{
    IssueExpectedGate, IssueReadinessInput, IssueReadinessResult, inspect_issue_readiness,
};

pub const ISSUE_TRIAGE_BASIS_VERSION: u32 = 1;
pub const MAX_ISSUE_TRIAGE_TITLE_BYTES: usize = 1024;
pub const MAX_ISSUE_TRIAGE_BODY_BYTES: usize = 64 * 1024;
pub const MAX_ISSUE_CRITERIA: usize = 100;
pub const MAX_ISSUE_CRITERION_BYTES: usize = 2 * 1024;

pub struct IssueTriageInput<'a> {
    pub issue: &'a IssueReference,
    pub current: Option<&'a IssueSourceRead>,
    pub policy: &'a IssueAutomationPolicy,
    pub automation_policy_version: &'a str,
    pub requirements: &'a IssueRequirements,
    /// required_sections와 공개 정책을 해석한 기존 issue-policy의 판이다.
    pub validation_policy_version: &'a str,
    pub held: bool,
    pub pinned_source_commit: Option<&'a str>,
    pub previous: Option<&'a IssueTriageAssessment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueTriageSourceState {
    NotYetObserved,
    Unknown,
    Shared,
    Withheld,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueTriageBasis {
    pub version: u32,
    pub issue: IssueReference,
    pub source: IssueTriageSourceState,
    pub automation_policy_version: String,
    /// 현재 관찰의 공개 정책 판이다. Unknown/legacy에는 이전 판을 가져오지 않는다.
    pub disclosure_policy_version: Option<String>,
    pub validation_policy_version: String,
    pub field_hash: Option<IssueFieldHash>,
    pub evidence_sha256: String,
    pub requirements_sha256: String,
    pub pinned_source_commit: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueTriageEligibility {
    Eligible,
    Ineligible,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueTriageReadiness {
    Ready,
    NeedsInfo,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueTriageGate {
    pub gate: IssueAutomationGate,
    pub action: IssueAutomationGateAction,
    pub enabled: bool,
    pub gate_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueObservationDisposition {
    AutomaticStart,
    HumanApproval,
    Deferred,
    Excluded,
    NeedsInfo,
    Unknown,
    ObservationOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueTriageObservation {
    /// 실행기가 소비할 허가가 아닌 반사실 관찰임을 wire에도 명시한다.
    pub observation_only: bool,
    pub assumed_mode: IssueAutomationAutonomy,
    pub disposition: IssueObservationDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueTriageReasonCode {
    NotYetObserved,
    SourceUnavailable,
    MetadataIncomplete,
    DisclosurePolicyStale,
    SelectionNotMatched,
    IssueClosed,
    UntrustedActor,
    HumanSelectionRequired,
    AlreadyClaimed,
    TitleMissing,
    RequiredSectionMissing,
    CriteriaMissing,
    CriteriaNotList,
    IssueTooLarge,
    UnsupportedContent,
    DisclosureConfirmationRequired,
    MaskedContent,
    HighRisk,
    HeldOrManual,
    ProtectedPath,
    PolicyHumanRequired,
    ModeOff,
    ObservationOnly,
    ExecutionGrantRequired,
    CloseDisabled,
    BudgetExhausted,
}

impl IssueTriageReasonCode {
    pub const ALL: [Self; 26] = [
        Self::NotYetObserved,
        Self::SourceUnavailable,
        Self::MetadataIncomplete,
        Self::DisclosurePolicyStale,
        Self::SelectionNotMatched,
        Self::IssueClosed,
        Self::UntrustedActor,
        Self::HumanSelectionRequired,
        Self::AlreadyClaimed,
        Self::TitleMissing,
        Self::RequiredSectionMissing,
        Self::CriteriaMissing,
        Self::CriteriaNotList,
        Self::IssueTooLarge,
        Self::UnsupportedContent,
        Self::DisclosureConfirmationRequired,
        Self::MaskedContent,
        Self::HighRisk,
        Self::HeldOrManual,
        Self::ProtectedPath,
        Self::PolicyHumanRequired,
        Self::ModeOff,
        Self::ObservationOnly,
        Self::ExecutionGrantRequired,
        Self::CloseDisabled,
        Self::BudgetExhausted,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotYetObserved => "not_yet_observed",
            Self::SourceUnavailable => "source_unavailable",
            Self::MetadataIncomplete => "metadata_incomplete",
            Self::DisclosurePolicyStale => "disclosure_policy_stale",
            Self::SelectionNotMatched => "selection_not_matched",
            Self::IssueClosed => "issue_closed",
            Self::UntrustedActor => "untrusted_actor",
            Self::HumanSelectionRequired => "human_selection_required",
            Self::AlreadyClaimed => "already_claimed",
            Self::TitleMissing => "title_missing",
            Self::RequiredSectionMissing => "required_section_missing",
            Self::CriteriaMissing => "criteria_missing",
            Self::CriteriaNotList => "criteria_not_list",
            Self::IssueTooLarge => "issue_too_large",
            Self::UnsupportedContent => "unsupported_content",
            Self::DisclosureConfirmationRequired => "disclosure_confirmation_required",
            Self::MaskedContent => "masked_content",
            Self::HighRisk => "high_risk",
            Self::HeldOrManual => "held_or_manual",
            Self::ProtectedPath => "protected_path",
            Self::PolicyHumanRequired => "policy_human_required",
            Self::ModeOff => "mode_off",
            Self::ObservationOnly => "observation_only",
            Self::ExecutionGrantRequired => "execution_grant_required",
            Self::CloseDisabled => "close_disabled",
            Self::BudgetExhausted => "budget_exhausted",
        }
    }
}

/// DB codec은 이 값으로 읽은 뒤 hydrate를 통과시킨다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistedIssueTriageAssessment {
    pub basis: IssueTriageBasis,
    pub eligibility: IssueTriageEligibility,
    pub readiness: IssueTriageReadiness,
    pub risk_class: IssueAutomationRiskClass,
    pub effective_mode: IssueAutomationAutonomy,
    pub gates: Vec<IssueTriageGate>,
    pub reasons: Vec<IssueTriageReasonCode>,
    pub observation: IssueTriageObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    try_from = "PersistedIssueTriageAssessment",
    into = "PersistedIssueTriageAssessment"
)]
pub struct IssueTriageAssessment {
    record: PersistedIssueTriageAssessment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueTriageError {
    InvalidIdentity,
    InvalidPolicy,
    InvalidBasis,
    InvalidAssessment,
    RevisionExhausted,
    PreviousMismatch,
}

impl std::fmt::Display for IssueTriageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "invalid issue triage identity",
            Self::InvalidPolicy => "invalid issue triage policy",
            Self::InvalidBasis => "invalid issue triage basis",
            Self::InvalidAssessment => "invalid issue triage assessment",
            Self::RevisionExhausted => "issue gate revision exhausted",
            Self::PreviousMismatch => "issue triage predecessor mismatch",
        })
    }
}

impl std::error::Error for IssueTriageError {}

impl IssueTriageAssessment {
    pub fn hydrate(record: PersistedIssueTriageAssessment) -> Result<Self, IssueTriageError> {
        basis::validate_basis(&record.basis)?;
        gates::validate_record(&record)?;
        if record.reasons.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(IssueTriageError::InvalidAssessment);
        }
        Ok(Self { record })
    }

    pub fn to_record(&self) -> PersistedIssueTriageAssessment {
        self.record.clone()
    }
    pub fn basis(&self) -> &IssueTriageBasis {
        &self.record.basis
    }
    pub fn eligibility(&self) -> IssueTriageEligibility {
        self.record.eligibility
    }
    pub fn readiness(&self) -> IssueTriageReadiness {
        self.record.readiness
    }
    pub fn risk_class(&self) -> IssueAutomationRiskClass {
        self.record.risk_class
    }
    pub fn effective_mode(&self) -> IssueAutomationAutonomy {
        self.record.effective_mode
    }
    pub fn gates(&self) -> &[IssueTriageGate] {
        &self.record.gates
    }
    pub fn reasons(&self) -> &[IssueTriageReasonCode] {
        &self.record.reasons
    }
    pub fn observation(&self) -> &IssueTriageObservation {
        &self.record.observation
    }

    pub fn input_fingerprint(&self) -> String {
        basis::canonical_hash(self.basis())
    }

    pub fn validate_for(&self, issue: &IssueReference) -> Result<(), IssueTriageError> {
        if self.basis().issue != *issue {
            return Err(IssueTriageError::InvalidIdentity);
        }
        basis::validate_basis(self.basis())?;
        gates::validate_record(&self.record)
    }

    pub fn validate_current(
        &self,
        issue: &IssueReference,
        current: Option<&IssueSourceRead>,
        held: bool,
    ) -> Result<(), IssueTriageError> {
        self.validate_for(issue)?;
        let actual = basis::source_basis(issue, current, held)?;
        if self.basis().source != actual.source
            || self.basis().field_hash != actual.field_hash
            || self.basis().disclosure_policy_version != actual.disclosure_policy_version
            || self.basis().evidence_sha256 != actual.evidence_sha256
        {
            return Err(IssueTriageError::InvalidBasis);
        }
        Ok(())
    }

    pub fn validate_successor(&self, previous: Option<&Self>) -> Result<(), IssueTriageError> {
        let Some(previous) = previous else {
            if self.gates().iter().any(|gate| gate.gate_revision != 1) {
                return Err(IssueTriageError::PreviousMismatch);
            }
            return Ok(());
        };
        previous.validate_for(&self.basis().issue)?;
        if self.input_fingerprint() == previous.input_fingerprint() {
            if self != previous {
                return Err(IssueTriageError::PreviousMismatch);
            }
            return Ok(());
        }
        for (current, before) in self.gates().iter().zip(previous.gates()) {
            let expected = before
                .gate_revision
                .checked_add(1)
                .filter(|revision| *revision <= MAX_ISSUE_AUTOMATION_REVISION)
                .ok_or(IssueTriageError::RevisionExhausted)?;
            if current.gate != before.gate || current.gate_revision != expected {
                return Err(IssueTriageError::PreviousMismatch);
            }
        }
        Ok(())
    }
}

impl TryFrom<PersistedIssueTriageAssessment> for IssueTriageAssessment {
    type Error = IssueTriageError;
    fn try_from(record: PersistedIssueTriageAssessment) -> Result<Self, Self::Error> {
        Self::hydrate(record)
    }
}

impl From<IssueTriageAssessment> for PersistedIssueTriageAssessment {
    fn from(value: IssueTriageAssessment) -> Self {
        value.record
    }
}

pub fn triage_issue(
    input: IssueTriageInput<'_>,
) -> Result<IssueTriageAssessment, IssueTriageError> {
    input
        .policy
        .validate()
        .map_err(|_| IssueTriageError::InvalidPolicy)?;
    let basis = basis::build_basis(&input)?;
    if let Some(previous) = input.previous {
        previous.validate_for(input.issue)?;
    }
    let evaluation = evaluate::evaluate(&input)?;
    let gates = gates::calculate(&basis, &evaluation, input.policy, input.previous)?;
    let observation = gates::observe(&evaluation, input.policy);
    let mut reasons = evaluation.reasons;
    reasons.push(IssueTriageReasonCode::ExecutionGrantRequired);
    reasons.push(IssueTriageReasonCode::CloseDisabled);
    match evaluation.effective_mode {
        IssueAutomationAutonomy::Off => reasons.push(IssueTriageReasonCode::ModeOff),
        IssueAutomationAutonomy::Observe => reasons.push(IssueTriageReasonCode::ObservationOnly),
        _ => {}
    }
    reasons.sort();
    reasons.dedup();
    let result = IssueTriageAssessment::hydrate(PersistedIssueTriageAssessment {
        basis,
        eligibility: evaluation.eligibility,
        readiness: evaluation.readiness,
        risk_class: evaluation.risk,
        effective_mode: evaluation.effective_mode,
        gates,
        reasons,
        observation,
    })?;
    result.validate_successor(input.previous)?;
    Ok(result)
}

#[cfg(test)]
mod tests;

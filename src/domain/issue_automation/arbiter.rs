//! 제한된 마감 판정의 입력 지문·호출 예약·결과다. 모델 원문이나 실행 권한은 저장하지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::IssueAutomationAutonomy;
use super::MAX_ISSUE_AUTOMATION_REVISION;
use super::decision::{
    IssueDecisionAnswerBy, IssueDecisionBasis, IssueDecisionDefinition, IssueDecisionFact,
    IssueDecisionId, IssueDecisionKind, IssueDecisionRequest, IssueDecisionSharedText,
    IssueDecisionState, IssueDecisionTimeout,
};
use super::policy::{IssueAutomationGate, IssueAutomationGateAction, IssueAutomationRiskClass};
use super::triage::{IssueTriageAssessment, IssueTriageEligibility, IssueTriageReadiness};

pub const ISSUE_ARBITER_CHANNEL: &str = "issue-arbiter";
pub const MAX_ISSUE_ARBITER_BATCH: usize = 256;
pub const MAX_ISSUE_ARBITER_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_ITEMS: usize = 8;
const MAX_RESPONSE_TEXT_BYTES: usize = 1024;

/// 저장소/모델을 읽기 전의 순수 사전 검사다. 현재 DB 판·인가 세대 검사는 별도다.
pub fn arbiter_eligible(
    request: &IssueDecisionRequest,
    current: &IssueTriageAssessment,
    now_unix_ms: u64,
) -> bool {
    request.state() == IssueDecisionState::Open
        && request.issue() == &current.basis().issue
        && arbiter_gate_allowed(request.definition(), current)
        && deadline_reached(request.definition(), now_unix_ms)
}

pub(crate) fn arbiter_gate_allowed(
    definition: &IssueDecisionDefinition,
    current: &IssueTriageAssessment,
) -> bool {
    if definition.on_timeout != IssueDecisionTimeout::Arbiter
        || definition.basis.work_order_id.is_some()
        || definition.basis.delivery.is_some()
        || definition.delivery_review.is_some()
        || current.eligibility() != IssueTriageEligibility::Eligible
        || current.readiness() != IssueTriageReadiness::Ready
        || current.risk_class() == IssueAutomationRiskClass::High
    {
        return false;
    }
    let supported = match (definition.kind, definition.gate) {
        (IssueDecisionKind::StartApproval, IssueAutomationGate::Start) => {
            matches!(
                current.effective_mode(),
                IssueAutomationAutonomy::Gated | IssueAutomationAutonomy::Auto
            )
        }
        (IssueDecisionKind::PlanApproval, IssueAutomationGate::Plan) => matches!(
            current.effective_mode(),
            IssueAutomationAutonomy::Propose
                | IssueAutomationAutonomy::Gated
                | IssueAutomationAutonomy::Auto
        ),
        _ => false,
    };
    supported
        && current.gates().iter().any(|gate| {
            gate.gate == definition.gate
                && gate.enabled
                && gate.action == IssueAutomationGateAction::TimeoutArbiter
        })
}

pub(crate) fn deadline_reached(definition: &IssueDecisionDefinition, now_unix_ms: u64) -> bool {
    definition
        .due_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .zip(i64::try_from(now_unix_ms).ok())
        .is_some_and(|(due, now)| now >= due.timestamp_millis())
}

/// confidence는 부동소수점이 아닌 만분율이다. 0인 호출 예산은 새 호출을 금지한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct IssueArbiterSettings {
    pub min_confidence: u16,
    pub max_running: u32,
    pub max_started_per_day: u32,
    pub timeout_seconds: u64,
}

impl Default for IssueArbiterSettings {
    fn default() -> Self {
        Self {
            min_confidence: 9000,
            max_running: 1,
            max_started_per_day: 5,
            timeout_seconds: 180,
        }
    }
}

impl IssueArbiterSettings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.min_confidence > 10_000
            || self.timeout_seconds == 0
            || self.timeout_seconds > super::policy::MAX_ISSUE_AUTOMATION_TIMEOUT_SECONDS
        {
            return Err("invalid issue arbiter settings");
        }
        Ok(())
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// host가 읽은 같은 입력에만 결과를 적용한다. workspace 절대 경로나 근거 본문은 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueArbiterInputBasis {
    pub decision_basis: IssueDecisionBasis,
    pub decision_revision: u64,
    #[serde(default, skip_serializing_if = "arbiter_zero_revision")]
    pub binding_run_revision: u64,
    pub repository_commit_sha: String,
    pub evidence_sha256: String,
    pub model_route_revision: String,
    pub input_sha256: String,
}

impl IssueArbiterInputBasis {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.decision_basis
            .validate()
            .map_err(|_| "invalid arbiter decision basis")?;
        if self.decision_revision == 0
            || self.decision_revision > MAX_ISSUE_AUTOMATION_REVISION
            || self.binding_run_revision > MAX_ISSUE_AUTOMATION_REVISION
            || !matches!(self.repository_commit_sha.len(), 40 | 64)
            || !self
                .repository_commit_sha
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !sha256(&self.evidence_sha256)
            || !sha256(&self.input_sha256)
            || !identifier(&self.model_route_revision, 128)
        {
            return Err("invalid issue arbiter input basis");
        }
        Ok(())
    }
}

fn arbiter_zero_revision(revision: &u64) -> bool {
    *revision == 0
}

/// 이 JSON은 메모리에서만 검증한다. reasons/blockers 원문을 장부에 쓰지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueArbiterModelReply {
    pub decision: String,
    pub confidence: u16,
    pub reasons: Vec<String>,
    pub blockers: Vec<String>,
}

impl IssueArbiterModelReply {
    pub fn parse_json(raw: &str, request: &IssueDecisionRequest) -> Result<Self, &'static str> {
        let reply = Self::parse_shape(raw)?;
        reply.validate(request)?;
        Ok(reply)
    }

    pub fn parse_shape(raw: &str) -> Result<Self, &'static str> {
        if raw.len() > MAX_ISSUE_ARBITER_RESPONSE_BYTES {
            return Err("the arbiter reply exceeds its bound");
        }
        let reply: Self =
            serde_json::from_str(raw).map_err(|_| "the arbiter reply is not valid JSON")?;
        reply.validate_shape()?;
        Ok(reply)
    }

    pub fn validate(&self, request: &IssueDecisionRequest) -> Result<(), &'static str> {
        self.validate_shape()?;
        if !request
            .definition()
            .options
            .iter()
            .any(|option| option.id == self.decision)
        {
            return Err("the arbiter reply names an unknown option");
        }
        Ok(())
    }

    fn validate_shape(&self) -> Result<(), &'static str> {
        if self.confidence > 10_000
            || !identifier(
                &self.decision,
                super::decision::MAX_ISSUE_DECISION_OPTION_BYTES,
            )
            || self.reasons.len() > MAX_RESPONSE_ITEMS
            || self.blockers.len() > MAX_RESPONSE_ITEMS
            || self.reasons.iter().chain(&self.blockers).any(|text| {
                text.trim().is_empty()
                    || text.len() > MAX_RESPONSE_TEXT_BYTES
                    || text.chars().any(|character| {
                        character.is_control() && !matches!(character, '\n' | '\r' | '\t')
                    })
            })
        {
            return Err("the arbiter reply does not match its bounded decision contract");
        }
        Ok(())
    }
}

/// application이 원문 이유를 정제한 뒤에 만드는 내부 값이다. by/actor/permit 입력은 없다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueArbiterProposal {
    pub decision: String,
    pub confidence: u16,
    pub has_blockers: bool,
    pub note: Option<IssueDecisionSharedText>,
}

impl IssueArbiterProposal {
    pub fn wait_reason(&self, settings: &IssueArbiterSettings) -> Option<IssueArbiterWaitReason> {
        if self.confidence > 10_000 {
            Some(IssueArbiterWaitReason::InvalidResponse)
        } else if self.has_blockers {
            Some(IssueArbiterWaitReason::BlockersPresent)
        } else if self.confidence < settings.min_confidence {
            Some(IssueArbiterWaitReason::LowConfidence)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueArbiterWaitReason {
    LowConfidence,
    BlockersPresent,
    InvalidResponse,
    OutcomeUnknown,
    ModelUnavailable,
    BudgetExhausted,
    EvidenceUnavailable,
    EvidenceChanged,
    ContextUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueArbiterResult {
    Proposal(IssueArbiterProposal),
    Waiting(IssueArbiterWaitReason),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueArbiterFinishedOutcome {
    Applied {},
    Waiting { reason: IssueArbiterWaitReason },
    Superseded {},
    Discarded {},
}

/// 각 결정 판의 결과 알림은 한 번만 만들어지고 ACK 이후에도 본문을 보존한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueArbiterFinishedEvent {
    pub event_id: String,
    pub fact: IssueDecisionFact,
    pub outcome: IssueArbiterFinishedOutcome,
    pub at: String,
    pub catalog_revision: u64,
}

impl IssueArbiterFinishedEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !identifier(&self.event_id, 128)
            || self.catalog_revision == 0
            || self.catalog_revision > MAX_ISSUE_AUTOMATION_REVISION
            || chrono::DateTime::parse_from_rfc3339(&self.at).is_err()
        {
            return Err("invalid issue arbiter event");
        }
        self.fact
            .basis
            .validate()
            .map_err(|_| "invalid arbiter event basis")?;
        let valid = match self.outcome {
            IssueArbiterFinishedOutcome::Applied {} => {
                self.fact.state == IssueDecisionState::Answered
                    && self.fact.answer_by == Some(IssueDecisionAnswerBy::Arbiter)
                    && self.fact.option.is_some()
            }
            IssueArbiterFinishedOutcome::Waiting { .. } => {
                self.fact.state == IssueDecisionState::Open
                    && self.fact.answer_by.is_none()
                    && self.fact.option.is_none()
            }
            IssueArbiterFinishedOutcome::Superseded {} => {
                self.fact.state == IssueDecisionState::Superseded
                    && self.fact.answer_by.is_none()
                    && self.fact.option.is_none()
            }
            IssueArbiterFinishedOutcome::Discarded {} => {
                self.fact.state != IssueDecisionState::Open
            }
        };
        if !valid {
            return Err("the arbiter event outcome does not match its decision fact");
        }
        Ok(())
    }
}

/// Claimed 자체가 호출 예산을 소비한다. dispatch 응답 유실·재시작 때 환급하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueArbiterAttempt {
    pub attempt_id: String,
    pub decision_id: IssueDecisionId,
    pub input_basis: IssueArbiterInputBasis,
    pub settings: IssueArbiterSettings,
    pub claimed_at_unix_ms: u64,
    pub deadline_at_unix_ms: u64,
    pub utc_day: u64,
    pub revision: u64,
    pub finished_event_id: Option<String>,
}

impl IssueArbiterAttempt {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.input_basis.validate()?;
        self.settings.validate()?;
        let deadline = self
            .settings
            .timeout_seconds
            .checked_mul(1000)
            .and_then(|duration| self.claimed_at_unix_ms.checked_add(duration));
        if !identifier(&self.attempt_id, 128)
            || self.revision == 0
            || self.revision > MAX_ISSUE_AUTOMATION_REVISION
            || deadline != Some(self.deadline_at_unix_ms)
            || self.deadline_at_unix_ms > i64::MAX as u64
            || self.utc_day < self.claimed_at_unix_ms / 86_400_000
            || self
                .finished_event_id
                .as_ref()
                .is_some_and(|id| !identifier(id, 128))
        {
            return Err("invalid issue arbiter attempt");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueArbiterWaitEntry {
    pub decision_id: IssueDecisionId,
    pub decision_revision: u64,
    pub basis: IssueDecisionBasis,
    pub reason: IssueArbiterWaitReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueArbiterWaitSnapshot {
    pub catalog_revision: u64,
    pub waits: Vec<IssueArbiterWaitEntry>,
}

fn sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn identifier(value: &str, bound: usize) -> bool {
    !value.is_empty()
        && value.len() <= bound
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

#[cfg(test)]
mod tests;

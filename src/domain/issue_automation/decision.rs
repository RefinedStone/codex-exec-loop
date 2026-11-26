//! 판에 묶인 결정 요청과 첫 답의 불변성이다 (#2544, plan 28 §10.1).
//! 시계·모델·장부 I/O는 없으며, 답을 받아도 실행 허가를 만들지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::issue_work::{IssueFieldHash, IssueReference};

use super::MAX_ISSUE_AUTOMATION_REVISION;
use super::policy::{IssueAutomationGate, IssueAutomationGateAction};
use super::triage::IssueTriageAssessment;

mod arbiter;
mod deadline;
mod validation;

pub use deadline::{ISSUE_DECISION_TIMEOUT_CHANNEL, IssueDecisionDeadline, IssueDecisionReminder};

pub const MAX_ISSUE_DECISION_ID_BYTES: usize = 128;
pub const MAX_ISSUE_DECISION_OPTION_BYTES: usize = 128;
pub const MAX_ISSUE_DECISION_NOTE_BYTES: usize = 4096;
pub const MAX_ISSUE_DECISION_TEXT_BYTES: usize = 8 * 1024;
pub const MAX_ISSUE_DECISION_OPTIONS: usize = 32;
pub const MAX_ISSUE_DECISION_EVIDENCE: usize = 32;

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct IssueDecisionId(String);

impl IssueDecisionId {
    pub fn new(value: impl Into<String>) -> Result<Self, IssueDecisionError> {
        let value = value.into();
        validation::identifier(&value, "id", MAX_ISSUE_DECISION_ID_BYTES)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for IssueDecisionId {
    type Error = IssueDecisionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<IssueDecisionId> for String {
    fn from(value: IssueDecisionId) -> Self {
        value.0
    }
}

/// application의 공개 정책을 이미 지난 글만 담는다.
/// 이 생성자는 마스킹을 수행하지 않는다. 원문을 이 타입으로 바꾸는 책임은
/// 기존 AiHandoff 공개 경계를 호출하는 application service에 있다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct IssueDecisionSharedText(String);

impl IssueDecisionSharedText {
    pub fn new_shared(value: impl Into<String>) -> Result<Self, IssueDecisionError> {
        let value = value.into();
        validation::shared_text(&value, "shared_text", MAX_ISSUE_DECISION_TEXT_BYTES)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for IssueDecisionSharedText {
    type Error = IssueDecisionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new_shared(value)
    }
}

impl From<IssueDecisionSharedText> for String {
    fn from(value: IssueDecisionSharedText) -> Self {
        value.0
    }
}

/// 대상은 종류를 가진 일반 객체다. 첫 구현에서 지원하는 대상만 Issue이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "issue",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum IssueDecisionSubject {
    Issue(IssueReference),
}

impl IssueDecisionSubject {
    pub fn issue(&self) -> &IssueReference {
        match self {
            Self::Issue(issue) => issue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueDecisionKind {
    StartApproval,
    SpecConfirmation,
    PlanApproval,
    RiskException,
    IssueChanged,
    FailureRecovery,
    AgentQuestion,
    CloseApproval,
}

impl IssueDecisionKind {
    pub const ALL: [Self; 8] = [
        Self::StartApproval,
        Self::SpecConfirmation,
        Self::PlanApproval,
        Self::RiskException,
        Self::IssueChanged,
        Self::FailureRecovery,
        Self::AgentQuestion,
        Self::CloseApproval,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueDecisionDeliveryContext {
    pub base_commit_sha: String,
    pub source_commit_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueDecisionBasis {
    /// 개입 뒤에는 같은 원문/정책이어도 새 사람 승인을 요구한다.
    #[serde(default, skip_serializing_if = "initial_authorization_epoch")]
    pub authorization_epoch: u64,
    pub policy_version: String,
    pub field_hash: Option<IssueFieldHash>,
    pub work_order_id: Option<String>,
    pub gate_revision: u64,
    /// Start의 기존 직렬화/식별자는 유지하고 실제 전달 범위만 별도로 고정한다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<IssueDecisionDeliveryContext>,
}

fn initial_authorization_epoch(value: &u64) -> bool {
    *value == 0
}

impl IssueDecisionBasis {
    pub fn from_assessment(
        assessment: &IssueTriageAssessment,
        gate: IssueAutomationGate,
        work_order_id: Option<String>,
    ) -> Result<Self, IssueDecisionError> {
        assessment
            .validate_for(&assessment.basis().issue)
            .map_err(|_| IssueDecisionError::InvalidBasis)?;
        let gate = assessment
            .gates()
            .iter()
            .find(|entry| entry.gate == gate)
            .ok_or(IssueDecisionError::InvalidBasis)?;
        let basis = Self {
            authorization_epoch: 0,
            policy_version: assessment.basis().automation_policy_version.clone(),
            field_hash: assessment.basis().field_hash.clone(),
            work_order_id,
            gate_revision: gate.gate_revision,
            delivery: None,
        };
        basis.validate()?;
        Ok(basis)
    }

    pub fn validate(&self) -> Result<(), IssueDecisionError> {
        validation::basis(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueDecisionOption {
    pub id: String,
    pub label: IssueDecisionSharedText,
    /// 설명만 보관한다. 이 값으로 작업, 허가, provider 쓰기를 실행하지 않는다.
    pub effect: IssueDecisionSharedText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueDecisionTimeout {
    Wait,
    Default,
    Arbiter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueDecisionDefinition {
    pub subject: IssueDecisionSubject,
    pub gate: IssueAutomationGate,
    pub kind: IssueDecisionKind,
    pub question: IssueDecisionSharedText,
    pub options: Vec<IssueDecisionOption>,
    pub recommended: Option<String>,
    pub basis: IssueDecisionBasis,
    pub on_timeout: IssueDecisionTimeout,
    pub due_at: Option<String>,
    /// 표시할 자유 본문이 아닌 요청자 식별자이다.
    pub asked_by: String,
    /// 공유된 근거의 식별자/참조만 넣는다. 근거 원문은 넣지 않는다.
    pub evidence: Vec<String>,
    /// 결정 당시 diff 요약이다. 이후 checkpoint가 바뀌어도 이 기록은 불변이다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_review: Option<super::delivery::IssueDeliveryDiffVerdict>,
}

impl IssueDecisionDefinition {
    pub fn is_delivery_risk_exception(&self) -> bool {
        self.kind == IssueDecisionKind::RiskException
            && self.gate == IssueAutomationGate::Delivery
            && self.basis.work_order_id.is_some()
            && self.basis.delivery.is_some()
            && self.delivery_review.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueDecisionState {
    Open,
    Answered,
    Expired,
    Withdrawn,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueDecisionAnswerBy {
    Human,
    Agent,
    Default,
    Arbiter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueDecisionAnswer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria: Option<Vec<super::criteria::IssueWorkCriterion>>,
    pub option: String,
    pub note: Option<IssueDecisionSharedText>,
    pub by: IssueDecisionAnswerBy,
    pub confirmation: Option<String>,
    pub caller: Option<String>,
    pub channel: String,
    pub at: String,
}

/// 호출 경로가 주는 metadata와 공개 정책을 지난 note이다.
/// by/confirmation은 입력받지 않으므로 client 이름으로 human 답을 만들 수 없다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDecisionAgentAnswer {
    pub option: String,
    pub note: Option<IssueDecisionSharedText>,
    pub caller: Option<String>,
    pub channel: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistedIssueDecisionRequest {
    pub id: IssueDecisionId,
    pub definition: IssueDecisionDefinition,
    pub state: IssueDecisionState,
    pub answer: Option<IssueDecisionAnswer>,
}

/// 장부를 읽을 때에도 hydrate를 통과한다. 공개 필드로 상태를 우회 변경할 수 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    try_from = "PersistedIssueDecisionRequest",
    into = "PersistedIssueDecisionRequest"
)]
pub struct IssueDecisionRequest {
    record: PersistedIssueDecisionRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredIssueDecision {
    pub request: IssueDecisionRequest,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueDecisionOutcome {
    Answered,
    Superseded,
    AlreadyResolved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDecisionResolution {
    pub request: IssueDecisionRequest,
    pub outcome: IssueDecisionOutcome,
    pub changed: bool,
}

/// 이벤트·기록에는 이 metadata만 복사한다. 질문, note와 caller는 들어 있지 않다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueDecisionFact {
    pub decision_id: IssueDecisionId,
    pub subject: IssueDecisionSubject,
    pub gate: IssueAutomationGate,
    pub kind: IssueDecisionKind,
    pub basis: IssueDecisionBasis,
    pub state: IssueDecisionState,
    pub answer_by: Option<IssueDecisionAnswerBy>,
    pub option: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_review: Option<super::delivery::IssueDeliveryDiffVerdict>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueDecisionError {
    InvalidField { field: &'static str },
    InvalidBasis,
    InvalidState,
    InvalidOption,
    DefinitionChanged,
    AlreadyClosed,
    UnsupportedAnswerOrigin,
    HumanConfirmationRequired,
}

impl std::fmt::Display for IssueDecisionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidField { field } => {
                write!(formatter, "invalid issue decision field: {field}")
            }
            Self::InvalidBasis => formatter.write_str("invalid issue decision basis"),
            Self::InvalidState => formatter.write_str("invalid issue decision state"),
            Self::InvalidOption => formatter.write_str("unknown issue decision option"),
            Self::DefinitionChanged => {
                formatter.write_str("issue decision definition cannot change")
            }
            Self::AlreadyClosed => formatter.write_str("closed issue decision cannot change"),
            Self::HumanConfirmationRequired => {
                formatter.write_str("this issue gate requires a verified human answer")
            }
            Self::UnsupportedAnswerOrigin => {
                formatter.write_str("unsupported issue decision answer origin")
            }
        }
    }
}

impl std::error::Error for IssueDecisionError {}

/// 식별자는 감사용 참조다. 이 값만으로 다음 명령을 인증할 수 없다.
pub fn validate_issue_decision_confirmation_reference(
    value: &str,
) -> Result<(), IssueDecisionError> {
    let Some(id) = value.strip_prefix("desktop-confirmation:") else {
        return Err(IssueDecisionError::UnsupportedAnswerOrigin);
    };
    if id.len() != 32
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(IssueDecisionError::UnsupportedAnswerOrigin);
    }
    Ok(())
}

impl IssueDecisionRequest {
    pub fn new(
        id: IssueDecisionId,
        definition: IssueDecisionDefinition,
    ) -> Result<Self, IssueDecisionError> {
        Self::hydrate(PersistedIssueDecisionRequest {
            id,
            definition,
            state: IssueDecisionState::Open,
            answer: None,
        })
    }

    pub fn hydrate(record: PersistedIssueDecisionRequest) -> Result<Self, IssueDecisionError> {
        validation::definition(&record.definition)?;
        validation::record_state(&record)?;
        Ok(Self { record })
    }

    pub fn to_record(&self) -> PersistedIssueDecisionRequest {
        self.record.clone()
    }

    pub fn id(&self) -> &IssueDecisionId {
        &self.record.id
    }

    pub fn definition(&self) -> &IssueDecisionDefinition {
        &self.record.definition
    }

    pub fn issue(&self) -> &IssueReference {
        self.definition().subject.issue()
    }

    pub fn state(&self) -> IssueDecisionState {
        self.record.state
    }

    pub fn answer(&self) -> Option<&IssueDecisionAnswer> {
        self.record.answer.as_ref()
    }

    /// 같은 불변 정의의 반복 요청은 ID가 달라도 하나로 묶는다.
    /// 같은 판의 서로 다른 질문은 서로 다른 결정 요청으로 보존한다.
    pub fn uniqueness_key(&self) -> String {
        #[derive(Serialize)]
        struct Key<'a> {
            version: u32,
            definition: &'a IssueDecisionDefinition,
        }
        let key = Key {
            version: 1,
            definition: self.definition(),
        };
        // 문자열·정수·닫힌 enum의 고정 필드만 있어 JSON 인코딩은 실패하지 않는다.
        let bytes = serde_json::to_vec(&key).expect("typed issue decision identity serializes");
        format!("{:x}", Sha256::digest(bytes))
    }

    pub fn fact(&self) -> IssueDecisionFact {
        let definition = self.definition();
        IssueDecisionFact {
            decision_id: self.id().clone(),
            subject: definition.subject.clone(),
            gate: definition.gate,
            kind: definition.kind,
            basis: definition.basis.clone(),
            state: self.state(),
            answer_by: self.answer().map(|answer| answer.by),
            option: self.answer().map(|answer| answer.option.clone()),
            delivery_review: definition.delivery_review.clone(),
        }
    }

    /// Agent 응답의 순수 전이이다. application/store는 별도로 현재 관문을 확인한다.
    pub fn answer_agent(
        &self,
        current_basis: &IssueDecisionBasis,
        input: IssueDecisionAgentAnswer,
    ) -> Result<IssueDecisionResolution, IssueDecisionError> {
        self.answer_for_gate(current_basis, IssueAutomationGateAction::Auto, input, None)
    }

    /// 확인 참조는 application이 비직렬화 capability의 대상을 검사한 뒤에만 전달한다.
    /// 낡은 판을 닫는 것은 관문 승인과 별개이므로 human 요구보다 먼저 처리한다.
    pub(crate) fn answer_for_gate(
        &self,
        current_basis: &IssueDecisionBasis,
        gate_action: IssueAutomationGateAction,
        input: IssueDecisionAgentAnswer,
        confirmation_reference: Option<&str>,
    ) -> Result<IssueDecisionResolution, IssueDecisionError> {
        self.answer_for_gate_with_criteria(
            current_basis,
            gate_action,
            input,
            confirmation_reference,
            None,
        )
    }

    pub(crate) fn answer_for_gate_with_criteria(
        &self,
        current_basis: &IssueDecisionBasis,
        gate_action: IssueAutomationGateAction,
        input: IssueDecisionAgentAnswer,
        confirmation_reference: Option<&str>,
        criteria: Option<Vec<super::criteria::IssueWorkCriterion>>,
    ) -> Result<IssueDecisionResolution, IssueDecisionError> {
        if self.state() != IssueDecisionState::Open {
            return Ok(IssueDecisionResolution {
                request: self.clone(),
                outcome: IssueDecisionOutcome::AlreadyResolved,
                changed: false,
            });
        }
        current_basis.validate()?;
        let mut answer = IssueDecisionAnswer {
            criteria: None,
            option: input.option,
            note: input.note,
            by: IssueDecisionAnswerBy::Agent,
            confirmation: None,
            caller: input.caller,
            channel: input.channel,
            at: input.at,
        };
        validation::answer(&answer, self.definition())?;
        let mut record = self.to_record();
        let outcome = if self.definition().basis != *current_basis {
            record.state = IssueDecisionState::Superseded;
            IssueDecisionOutcome::Superseded
        } else {
            if let Some(reference) = confirmation_reference {
                validate_issue_decision_confirmation_reference(reference)?;
                answer.by = IssueDecisionAnswerBy::Human;
                answer.confirmation = Some(reference.to_owned());
            } else if matches!(
                gate_action,
                IssueAutomationGateAction::Human
                    | IssueAutomationGateAction::TimeoutDefault
                    | IssueAutomationGateAction::TimeoutArbiter
            ) || self.definition().on_timeout == IssueDecisionTimeout::Arbiter
                || self.definition().is_delivery_risk_exception()
            {
                return Err(IssueDecisionError::HumanConfirmationRequired);
            }
            answer.criteria = criteria;
            validation::answer(&answer, self.definition())?;
            record.state = IssueDecisionState::Answered;
            record.answer = Some(answer);
            IssueDecisionOutcome::Answered
        };
        let request = Self::hydrate(record)?;
        request.validate_successor(self)?;
        Ok(IssueDecisionResolution {
            request,
            outcome,
            changed: true,
        })
    }

    pub fn validate_successor(&self, previous: &Self) -> Result<(), IssueDecisionError> {
        if self.id() != previous.id() || self.definition() != previous.definition() {
            return Err(IssueDecisionError::DefinitionChanged);
        }
        if previous.state() != IssueDecisionState::Open && self != previous {
            return Err(IssueDecisionError::AlreadyClosed);
        }
        if self.state() == IssueDecisionState::Open && self != previous {
            return Err(IssueDecisionError::InvalidState);
        }
        Ok(())
    }
}

impl TryFrom<PersistedIssueDecisionRequest> for IssueDecisionRequest {
    type Error = IssueDecisionError;

    fn try_from(record: PersistedIssueDecisionRequest) -> Result<Self, Self::Error> {
        Self::hydrate(record)
    }
}

impl From<IssueDecisionRequest> for PersistedIssueDecisionRequest {
    fn from(value: IssueDecisionRequest) -> Self {
        value.record
    }
}

impl StoredIssueDecision {
    pub fn validate(&self) -> Result<(), IssueDecisionError> {
        if self.revision == 0 || self.revision > MAX_ISSUE_AUTOMATION_REVISION {
            return Err(IssueDecisionError::InvalidField { field: "revision" });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

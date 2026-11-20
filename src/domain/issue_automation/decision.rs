//! 판에 묶인 결정 요청과 첫 답의 불변성이다 (#2544, plan 28 §10.1).
//! 시계·모델·장부 I/O는 없으며, 답을 받아도 실행 허가를 만들지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::issue_work::{IssueFieldHash, IssueReference};

use super::MAX_ISSUE_AUTOMATION_REVISION;
use super::policy::IssueAutomationGate;
use super::triage::IssueTriageAssessment;

mod validation;

pub const MAX_ISSUE_DECISION_ID_BYTES: usize = 128;
pub const MAX_ISSUE_DECISION_OPTION_BYTES: usize = 128;
pub const MAX_ISSUE_DECISION_NOTE_BYTES: usize = 4096;
pub const MAX_ISSUE_DECISION_TEXT_BYTES: usize = 8 * 1024;
pub const MAX_ISSUE_DECISION_OPTIONS: usize = 32;
pub const MAX_ISSUE_DECISION_EVIDENCE: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
}

impl IssueDecisionKind {
    pub const ALL: [Self; 7] = [
        Self::StartApproval,
        Self::SpecConfirmation,
        Self::PlanApproval,
        Self::RiskException,
        Self::IssueChanged,
        Self::FailureRecovery,
        Self::AgentQuestion,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueDecisionBasis {
    pub policy_version: String,
    pub field_hash: Option<IssueFieldHash>,
    pub work_order_id: Option<String>,
    pub gate_revision: u64,
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
            policy_version: assessment.basis().automation_policy_version.clone(),
            field_hash: assessment.basis().field_hash.clone(),
            work_order_id,
            gate_revision: gate.gate_revision,
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
            Self::UnsupportedAnswerOrigin => {
                formatter.write_str("unsupported issue decision answer origin")
            }
        }
    }
}

impl std::error::Error for IssueDecisionError {}

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
        }
    }

    pub fn answer_agent(
        &self,
        current_basis: &IssueDecisionBasis,
        input: IssueDecisionAgentAnswer,
    ) -> Result<IssueDecisionResolution, IssueDecisionError> {
        if self.state() != IssueDecisionState::Open {
            return Ok(IssueDecisionResolution {
                request: self.clone(),
                outcome: IssueDecisionOutcome::AlreadyResolved,
                changed: false,
            });
        }
        current_basis.validate()?;
        let answer = IssueDecisionAnswer {
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
        let outcome = if self.definition().basis == *current_basis {
            record.state = IssueDecisionState::Answered;
            record.answer = Some(answer);
            IssueDecisionOutcome::Answered
        } else {
            record.state = IssueDecisionState::Superseded;
            IssueDecisionOutcome::Superseded
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

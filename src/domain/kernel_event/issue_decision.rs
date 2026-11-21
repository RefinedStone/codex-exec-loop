//! 결정 감사 기록의 메타데이터. 질문, 메모, 본문, 호출자 표시 이름은 담지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{IssueEventReference, KernelEvent, ReasonCode};
use crate::domain::issue_automation::decision::{
    IssueDecisionAnswerBy, IssueDecisionBasis, IssueDecisionFact, IssueDecisionKind,
    IssueDecisionState, IssueDecisionSubject,
};
use crate::domain::issue_automation::policy::IssueAutomationGate;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueDecisionFactRecord {
    pub decision_id: String,
    pub subject: IssueDecisionSubjectRecord,
    pub gate: IssueDecisionGateRecord,
    pub kind: IssueDecisionKind,
    pub basis: IssueDecisionBasisRecord,
    pub state: IssueDecisionState,
    pub answer_by: Option<IssueDecisionAnswerBy>,
    pub option: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueDecisionSubjectRecord {
    Issue { issue: IssueEventReference },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueDecisionGateRecord {
    Start,
    Spec,
    Plan,
    Delivery,
    Merge,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueDecisionBasisRecord {
    pub policy_version: String,
    pub field_hash: Option<IssueDecisionFieldHashRecord>,
    pub work_order_id: Option<String>,
    pub gate_revision: u64,
    #[serde(default)]
    pub authorization_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueDecisionFieldHashRecord {
    pub version: u32,
    pub sha256: String,
}

impl From<&IssueDecisionFact> for IssueDecisionFactRecord {
    fn from(fact: &IssueDecisionFact) -> Self {
        Self {
            decision_id: fact.decision_id.as_str().to_owned(),
            subject: (&fact.subject).into(),
            gate: fact.gate.into(),
            kind: fact.kind,
            basis: (&fact.basis).into(),
            state: fact.state,
            answer_by: fact.answer_by,
            option: fact.option.clone(),
        }
    }
}

impl From<&IssueDecisionSubject> for IssueDecisionSubjectRecord {
    fn from(subject: &IssueDecisionSubject) -> Self {
        match subject {
            IssueDecisionSubject::Issue(issue) => Self::Issue {
                issue: issue.into(),
            },
        }
    }
}

impl From<IssueAutomationGate> for IssueDecisionGateRecord {
    fn from(gate: IssueAutomationGate) -> Self {
        match gate {
            IssueAutomationGate::Start => Self::Start,
            IssueAutomationGate::Spec => Self::Spec,
            IssueAutomationGate::Plan => Self::Plan,
            IssueAutomationGate::Delivery => Self::Delivery,
            IssueAutomationGate::Merge => Self::Merge,
            IssueAutomationGate::Close => Self::Close,
        }
    }
}

impl From<&IssueDecisionBasis> for IssueDecisionBasisRecord {
    fn from(basis: &IssueDecisionBasis) -> Self {
        Self {
            policy_version: basis.policy_version.clone(),
            field_hash: basis
                .field_hash
                .as_ref()
                .map(|hash| IssueDecisionFieldHashRecord {
                    version: hash.version,
                    sha256: hash.sha256.clone(),
                }),
            work_order_id: basis.work_order_id.clone(),
            gate_revision: basis.gate_revision,
            authorization_epoch: basis.authorization_epoch,
        }
    }
}

impl KernelEvent {
    pub fn issue_decision_requested(fact: &IssueDecisionFact, revision: u64) -> Self {
        Self::IssueDecisionRequested {
            decision: fact.into(),
            revision,
        }
    }

    pub fn issue_decision_resolved(fact: &IssueDecisionFact, revision: u64) -> Self {
        Self::IssueDecisionResolved {
            decision: fact.into(),
            revision,
            reason: (fact.state == IssueDecisionState::Superseded)
                .then_some(ReasonCode::DecisionSuperseded),
        }
    }
}

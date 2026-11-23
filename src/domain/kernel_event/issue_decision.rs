//! 결정 감사 기록의 메타데이터. 질문, 메모, 본문, 호출자 표시 이름은 담지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{IssueEventReference, KernelEvent, ReasonCode};
use crate::domain::issue_automation::decision::{
    IssueDecisionAnswerBy, IssueDecisionBasis, IssueDecisionFact, IssueDecisionKind,
    IssueDecisionState, IssueDecisionSubject,
};
use crate::domain::issue_automation::delivery::{
    IssueDeliveryDiffVerdict, IssueDeliveryRiskReason,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_review: Option<IssueDeliveryDiffVerdictRecord>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<IssueDecisionDeliveryContextRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueDecisionDeliveryContextRecord {
    pub base_commit_sha: String,
    pub source_commit_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueDeliveryDiffVerdictRecord {
    pub changed_files: u64,
    pub changed_lines: Option<u64>,
    pub reasons: Vec<IssueDeliveryRiskReason>,
}

impl From<&IssueDeliveryDiffVerdict> for IssueDeliveryDiffVerdictRecord {
    fn from(verdict: &IssueDeliveryDiffVerdict) -> Self {
        Self {
            changed_files: verdict.changed_files,
            changed_lines: verdict.changed_lines,
            reasons: verdict.reasons.clone(),
        }
    }
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
            delivery_review: fact.delivery_review.as_ref().map(Into::into),
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
            delivery: basis
                .delivery
                .as_ref()
                .map(|range| IssueDecisionDeliveryContextRecord {
                    base_commit_sha: range.base_commit_sha.clone(),
                    source_commit_sha: range.source_commit_sha.clone(),
                }),
        }
    }
}

impl KernelEvent {
    pub fn issue_decision_reminded(
        event: &crate::domain::issue_automation::decision::IssueDecisionReminder,
    ) -> Self {
        Self::IssueDecisionReminded {
            event_id: event.event_id.clone(),
            decision: (&event.fact).into(),
            due_at: event.due_at.clone(),
            reminded_at: event.reminded_at.clone(),
            revision: event.revision,
        }
    }

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

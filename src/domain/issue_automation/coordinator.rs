//! 승인 판과 영속 작업 연결을 읽는 순수 조정 규칙이다 (#2548).
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::decision::{
    IssueDecisionAnswerBy, IssueDecisionBasis, IssueDecisionId, IssueDecisionRequest,
    IssueDecisionState,
};
use super::policy::{IssueAutomationGate, IssueAutomationGateAction};
use super::triage::{IssueTriageAssessment, IssueTriageEligibility, IssueTriageReadiness};
use super::{IssueAutomationAutonomy, IssueAutomationFlags, IssueAutomationState};
use crate::domain::issue_work::{IssueFieldHash, IssueReference, IssueSharedCopy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCoordinatorAction {
    Triage,
    OpenDecision,
    ExpireDecision,
    SubmitWork,
    CancelWork,
    PostFeedback,
    MarkDone,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueWorkBlockReason {
    ExecutionDisabled,
    AwaitingSource,
    ApprovalChanged,
    SourceUnavailable,
    SubmissionUnavailable,
    FeedbackUnconfirmed,
}

/// Missing authority fields in legacy checkpoints mean Human, never Policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueWorkStartAuthority {
    #[default]
    Human,
    Policy,
}

impl IssueWorkStartAuthority {
    pub fn validate(self, decision_id: Option<&IssueDecisionId>) -> Result<(), &'static str> {
        match (self, decision_id) {
            (Self::Human, Some(_)) | (Self::Policy, None) => Ok(()),
            _ => Err("issue work start authority does not match its decision"),
        }
    }
}

/// Waiting consumes no reservation and cannot authorize execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct IssueStartBudgetWait {
    pub exhausted: Vec<IssueStartBudgetLimit>,
    pub utc_day: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueStartBudgetLimit {
    Running,
    StartedPerDay,
    Attempts,
}

/// 실행기 중립 명령이다. 본문은 현재 AiHandoff 공유본이며 원본을 담지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueWorkOrder {
    pub work_order_id: String,
    pub issue: IssueReference,
    pub field_hash: IssueFieldHash,
    pub title: String,
    pub handoff: IssueSharedCopy,
    pub acceptance: Vec<String>,
    pub scope: String,
    pub branch_hint: Option<String>,
    pub link_reference: Option<String>,
    /// Frozen user close policy; legacy orders never emit a closing keyword.
    #[serde(default)]
    pub provider_link: bool,
    pub profile: Option<String>,
    pub priority: i32,
    pub attempt: u32,
}

/// planning 영수증 보존 기간과 독립적인 영속 연결이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueWorkCheckpoint {
    /// 첫 승인/작업 원문은 불변이다. 개입 뒤 승인은 별도로 이어 쓴다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization: Option<IssueWorkAuthorization>,
    #[serde(default)]
    pub permit_authorization_epoch: u64,
    pub order: IssueWorkOrder,
    #[serde(default)]
    pub start_authority: IssueWorkStartAuthority,
    pub decision_id: Option<IssueDecisionId>,
    pub approved_basis: IssueDecisionBasis,
    pub task_ids: Vec<String>,
    pub permit_source: Option<String>,
    pub permit_revision: u64,
    pub delivery_granted: bool,
    pub blocked_reason: Option<IssueWorkBlockReason>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueWorkAuthorization {
    #[serde(default)]
    pub start_authority: IssueWorkStartAuthority,
    pub decision_id: Option<IssueDecisionId>,
    pub basis: IssueDecisionBasis,
}

impl IssueWorkCheckpoint {
    pub fn current_authorization_basis(&self) -> &IssueDecisionBasis {
        self.authorization
            .as_ref()
            .map(|auth| &auth.basis)
            .unwrap_or(&self.approved_basis)
    }
    pub fn current_authorization_decision_id(&self) -> Option<&IssueDecisionId> {
        match &self.authorization {
            Some(authorization) => authorization.decision_id.as_ref(),
            None => self.decision_id.as_ref(),
        }
    }
    pub fn current_start_authority(&self) -> IssueWorkStartAuthority {
        self.authorization
            .as_ref()
            .map(|authorization| authorization.start_authority)
            .unwrap_or(self.start_authority)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        self.start_authority.validate(self.decision_id.as_ref())?;
        super::validate_reference(&self.order.issue).map_err(|_| "invalid work issue")?;
        self.approved_basis
            .validate()
            .map_err(|_| "invalid work approval")?;
        if self.order.work_order_id != issue_work_order_id(&self.order.issue)
            || self.order.scope != self.order.issue.project_scope.workspace_dir
            || self.order.attempt != 1
            || self.order.title != self.order.handoff.title
            || self.order.title.trim().is_empty()
            || self.order.title.len() > super::triage::MAX_ISSUE_TRIAGE_TITLE_BYTES
            || self.permit_source.as_ref().is_some_and(|sha| {
                !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            || self.order.handoff.body.len() > super::triage::MAX_ISSUE_TRIAGE_BODY_BYTES
            || self.order.acceptance.is_empty()
            || self.approved_basis.field_hash.as_ref() != Some(&self.order.field_hash)
            || self.approved_basis.work_order_id.is_some()
            || self.revision > super::MAX_ISSUE_AUTOMATION_REVISION
            || self.permit_revision > super::MAX_ISSUE_AUTOMATION_REVISION
            || (self.permit_source.is_some() != (self.permit_revision > 0))
            || (self.delivery_granted && self.permit_source.is_none())
            || self.task_ids.iter().any(|id| id.trim().is_empty())
        {
            return Err("invalid issue work checkpoint");
        }
        if let Some(authorization) = &self.authorization {
            authorization
                .start_authority
                .validate(authorization.decision_id.as_ref())?;
            authorization
                .basis
                .validate()
                .map_err(|_| "invalid renewed work approval")?;
            if authorization.basis.authorization_epoch < self.approved_basis.authorization_epoch
                || authorization.basis.field_hash.as_ref() != Some(&self.order.field_hash)
                || authorization.basis.work_order_id.is_some()
            {
                return Err("renewed work approval does not match the immutable work");
            }
        }
        Ok(())
    }
}

pub struct IssueCoordinatorInput<'a> {
    pub authorization_epoch: u64,
    pub state: IssueAutomationState,
    pub flags: IssueAutomationFlags,
    pub assessment: Option<&'a IssueTriageAssessment>,
    pub decision: Option<&'a IssueDecisionRequest>,
    pub work: Option<&'a IssueWorkCheckpoint>,
    pub frozen_source: Option<&'a str>,
    /// propose에는 deadline이 없다. 외부 시계에 의존하지 않는 입력 경계만 유지한다.
    pub now_unix_ms: u64,
}

pub fn next_action(input: IssueCoordinatorInput<'_>) -> IssueCoordinatorAction {
    let _ = input.now_unix_ms; // propose는 시간 경과만으로 승인/만료를 만들지 않는다.
    use IssueCoordinatorAction::*;
    if matches!(
        input.state,
        IssueAutomationState::Done | IssueAutomationState::Cancelled | IssueAutomationState::Failed
    ) {
        return None;
    }
    let Some(assessment) = input.assessment else {
        return Triage;
    };
    let eligible = ready_for_start(assessment)
        && !input.flags.held
        && !input.flags.stale
        && !input.flags.unknown;
    let current_basis =
        IssueDecisionBasis::from_assessment(assessment, IssueAutomationGate::Start, Option::None)
            .ok()
            .map(|mut basis| {
                basis.authorization_epoch = input.authorization_epoch;
                basis
            });
    if let Some(work) = input.work.filter(|work| {
        work.current_authorization_basis().authorization_epoch == input.authorization_epoch
    }) {
        let approved = eligible
            && current_basis.as_ref() == Some(work.current_authorization_basis())
            && match work.current_start_authority() {
                IssueWorkStartAuthority::Policy => policy_start(assessment),
                IssueWorkStartAuthority::Human => input.decision.is_some_and(|decision| {
                    human_start(decision)
                        && Some(decision.id()) == work.current_authorization_decision_id()
                        && decision.issue() == &work.order.issue
                        && &decision.definition().basis == work.current_authorization_basis()
                }),
            };
        if !approved {
            return CancelWork;
        }
        if work.task_ids.is_empty()
            || work.permit_source.is_none()
            || work.permit_authorization_epoch != input.authorization_epoch
            || input.frozen_source.is_some_and(|source| {
                !work.delivery_granted || work.permit_source.as_deref() != Some(source)
            })
        {
            return SubmitWork;
        }
        return None;
    }
    // A lower budget blocks new starts but must still retire an obsolete open approval.
    if input.decision.is_some_and(|decision| {
        decision.issue() == &assessment.basis().issue
            && decision.state() == IssueDecisionState::Open
            && current_basis
                .as_ref()
                .is_some_and(|basis| decision.definition().basis != *basis)
    }) {
        return ExpireDecision;
    }
    // A configured zero is already visible as a stable Ready/budget_exhausted assessment.
    if assessment
        .reasons()
        .contains(&super::triage::IssueTriageReasonCode::BudgetExhausted)
    {
        return None;
    }
    if eligible && policy_start(assessment) {
        return SubmitWork;
    }
    let Some(decision) = input.decision else {
        return if eligible { OpenDecision } else { None };
    };
    if decision.issue() != &assessment.basis().issue {
        return None;
    }
    if decision.definition().basis
        != current_basis.unwrap_or_else(|| decision.definition().basis.clone())
    {
        return if decision.state() == IssueDecisionState::Open {
            ExpireDecision
        } else if eligible {
            OpenDecision
        } else {
            None
        };
    }
    if !eligible {
        return None;
    }
    if human_start(decision) {
        return SubmitWork;
    }
    None
}

/// The start actions implemented by propose/gated; timeout policies are a later slice.
pub fn ready_for_start(assessment: &IssueTriageAssessment) -> bool {
    matches!(
        assessment.effective_mode(),
        IssueAutomationAutonomy::Propose | IssueAutomationAutonomy::Gated
    ) && assessment.eligibility() == IssueTriageEligibility::Eligible
        && assessment.readiness() == IssueTriageReadiness::Ready
        && assessment.gates().iter().any(|gate| {
            gate.gate == IssueAutomationGate::Start
                && gate.enabled
                && matches!(
                    gate.action,
                    IssueAutomationGateAction::Human | IssueAutomationGateAction::Auto
                )
        })
}

pub fn policy_start(assessment: &IssueTriageAssessment) -> bool {
    ready_for_start(assessment)
        && assessment.effective_mode() == IssueAutomationAutonomy::Gated
        && assessment.gates().iter().any(|gate| {
            gate.gate == IssueAutomationGate::Start
                && gate.enabled
                && gate.action == IssueAutomationGateAction::Auto
        })
}

pub fn human_start(decision: &IssueDecisionRequest) -> bool {
    decision.state() == IssueDecisionState::Answered
        && decision.definition().kind == super::decision::IssueDecisionKind::StartApproval
        && decision.definition().gate == IssueAutomationGate::Start
        && decision.answer().is_some_and(|answer| {
            answer.option == "start"
                && answer.by == IssueDecisionAnswerBy::Human
                && answer.confirmation.is_some()
        })
}

pub fn issue_work_order_id(issue: &IssueReference) -> String {
    // 한 이슈의 첫 작업은 field/policy/decision 판이 바뀌어도 재등록되지 않는다.
    let encoded = serde_json::to_vec(issue).expect("typed issue identity serializes");
    format!("issue-work-{:x}", Sha256::digest(encoded))
}

pub fn start_decision_id(issue: &IssueReference, basis: &IssueDecisionBasis) -> IssueDecisionId {
    let encoded = serde_json::to_vec(&(issue, basis)).expect("typed decision basis serializes");
    IssueDecisionId::new(format!("issue-start-{:x}", Sha256::digest(encoded)))
        .expect("bounded decision ID")
}

/// 목록/상세에는 실행 참조와 대기 이유만 보낸다. 과거 handoff를 다시 노출하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct IssueWorkSummary {
    pub work_order_id: String,
    pub task_ids: Vec<String>,
    pub blocked_reason: Option<IssueWorkBlockReason>,
    pub delivery_granted: bool,
}

impl From<&IssueWorkCheckpoint> for IssueWorkSummary {
    fn from(work: &IssueWorkCheckpoint) -> Self {
        Self {
            work_order_id: work.order.work_order_id.clone(),
            task_ids: work.task_ids.clone(),
            blocked_reason: work.blocked_reason,
            delivery_granted: work.delivery_granted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueWorkEvent {
    pub event_id: String,
    pub issue: IssueReference,
    pub work_order_id: String,
    pub kind: IssueWorkEventKind,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueWorkEventKind {
    Submitted {
        task_ids: Vec<String>,
    },
    ExecutionProgressed {
        progress: super::execution::IssueExecutionProgress,
    },
    Integrated {
        integration: super::execution::IssueExecutionIntegration,
    },
    /// 현재 provider 관찰의 Open→Closed 전이다. 닫은 사람이나 원인은 추정하지 않는다.
    ExternalClosed {
        observation_revision: u64,
    },
    PermitGranted {
        source_sha: String,
        gate_revision: u64,
        delivery: bool,
    },
}

impl IssueWorkEvent {
    pub fn new(
        issue: IssueReference,
        work_order_id: String,
        kind: IssueWorkEventKind,
        revision: u64,
    ) -> Self {
        let bytes = serde_json::to_vec(&(&issue, &work_order_id, &kind, revision))
            .expect("typed work event identity serializes");
        Self {
            event_id: format!("issue-event-{:x}", Sha256::digest(bytes)),
            issue,
            work_order_id,
            kind,
            revision,
        }
    }
}

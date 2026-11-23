//! Arbiter 호출 예약과 결과는 같은 결정 판의 단일 영속 전이로 저장한다.
use super::{IssueDecisionFence, IssueTriageAssessment, StoredIssueDecision};
use crate::domain::issue_automation::arbiter::{
    IssueArbiterAttempt, IssueArbiterFinishedEvent, IssueArbiterInputBasis, IssueArbiterResult,
    IssueArbiterSettings, IssueArbiterWaitReason,
};
use crate::domain::issue_automation::decision::IssueDecisionId;

#[derive(Debug, Clone)]
pub struct IssueArbiterClaimRequest {
    pub fence: IssueDecisionFence,
    pub current: IssueTriageAssessment,
    pub decision_id: IssueDecisionId,
    pub input_basis: IssueArbiterInputBasis,
    pub settings: IssueArbiterSettings,
    pub now_unix_ms: u64,
}

#[derive(Debug, Clone)]
pub struct IssueArbiterCurrentContext {
    pub fence: IssueDecisionFence,
    pub current: IssueTriageAssessment,
    pub input_basis: IssueArbiterInputBasis,
}

#[derive(Debug, Clone)]
pub struct IssueArbiterFinishRequest {
    pub attempt_id: String,
    pub expected_revision: u64,
    /// None은 현재 입력을 확인하지 못했다는 뜻이다. 어떤 모델 답도 적용하지 않는다.
    pub current: Option<IssueArbiterCurrentContext>,
    pub result: IssueArbiterResult,
    pub now_unix_ms: u64,
}

/// 모델을 부르기 전 예산·근거가 없어 기다리는 경우에도 같은 판의 알림을 영속한다.
#[derive(Debug, Clone)]
pub struct IssueArbiterWaitRequest {
    pub fence: IssueDecisionFence,
    pub current: IssueTriageAssessment,
    pub decision_id: IssueDecisionId,
    pub reason: IssueArbiterWaitReason,
    pub now_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueArbiterFinishCommit {
    pub record: StoredIssueDecision,
    pub event: IssueArbiterFinishedEvent,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueArbiterClaimResult {
    /// 이 결과를 받은 호출자만 dispatch할 수 있다.
    Claimed(Box<IssueArbiterAttempt>),
    /// 재시작을 포함하여 기존 pending은 observe만 하고 다시 dispatch하지 않는다.
    Existing(Box<IssueArbiterAttempt>),
    Ineligible,
    /// durable Wait다. 같은 결정 판에서는 예산이 회복돼도 자동 재호출하지 않는다.
    Deferred(Box<IssueArbiterFinishCommit>),
    Resolved(Box<StoredIssueDecision>),
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueArbiterFinishResult {
    Saved(Box<IssueArbiterFinishCommit>),
    Conflict,
}

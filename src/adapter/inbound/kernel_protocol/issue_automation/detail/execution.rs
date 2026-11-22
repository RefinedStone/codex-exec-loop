//! 실행 표시 계약이다. 필드와 문구는 저장 사실의 domain projection에서만 옮긴다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::issue_automation::execution::{
    IssueExecutionDetail, IssueExecutionReadyDetail, IssueExecutionRecovery,
    IssueExecutionRecoveryReason, IssueExecutionState, IssueExecutionTerminalClass,
    IssueExecutionUnavailableReason,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueExecutionDetailState {
    Ready(Box<IssueExecutionReadyState>),
    Unavailable {
        reason: IssueExecutionUnavailableReasonState,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueExecutionUnavailableReasonState {
    ReadFailed,
    WorkIdMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueExecutionReadyState {
    pub work_order_id: String,
    pub revision: u64,
    pub state: IssueExecutionState,
    pub state_label: String,
    pub execution_attempts: u32,
    pub execution_attempt_limit: Option<u32>,
    pub verification_attempts: u32,
    pub review_pull_request_url: Option<String>,
    pub terminal_class: Option<IssueExecutionTerminalClass>,
    pub recovery: Option<IssueExecutionRecoveryState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueExecutionRecoveryState {
    pub reason: IssueExecutionRecoveryReasonState,
    pub summary: String,
    pub next_summary: String,
    pub requires_human: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueExecutionRecoveryReasonState {
    AttemptsExhausted,
    ReworkNeeded,
    ObserveAgain,
    ContractRecovery,
    IdentityConflict,
    PullRequestClosed,
    QueueRecovery,
}

impl From<&IssueExecutionDetail> for IssueExecutionDetailState {
    fn from(detail: &IssueExecutionDetail) -> Self {
        match detail {
            IssueExecutionDetail::Ready(ready) => Self::Ready(Box::new(ready.as_ref().into())),
            IssueExecutionDetail::Unavailable { reason } => Self::Unavailable {
                reason: match reason {
                    IssueExecutionUnavailableReason::ReadFailed => {
                        IssueExecutionUnavailableReasonState::ReadFailed
                    }
                    IssueExecutionUnavailableReason::WorkIdMismatch => {
                        IssueExecutionUnavailableReasonState::WorkIdMismatch
                    }
                },
            },
        }
    }
}

impl From<&IssueExecutionReadyDetail> for IssueExecutionReadyState {
    fn from(ready: &IssueExecutionReadyDetail) -> Self {
        Self {
            work_order_id: ready.work_order_id.clone(),
            revision: ready.revision,
            state: ready.state,
            state_label: ready.state_label.clone(),
            execution_attempts: ready.execution_attempts,
            execution_attempt_limit: ready.execution_attempt_limit,
            verification_attempts: ready.verification_attempts,
            review_pull_request_url: ready.review_pull_request_url.clone(),
            terminal_class: ready.terminal_class,
            recovery: ready.recovery.as_ref().map(Into::into),
        }
    }
}

impl From<&IssueExecutionRecovery> for IssueExecutionRecoveryState {
    fn from(recovery: &IssueExecutionRecovery) -> Self {
        Self {
            reason: match recovery.reason {
                IssueExecutionRecoveryReason::AttemptsExhausted => {
                    IssueExecutionRecoveryReasonState::AttemptsExhausted
                }
                IssueExecutionRecoveryReason::ReworkNeeded => {
                    IssueExecutionRecoveryReasonState::ReworkNeeded
                }
                IssueExecutionRecoveryReason::ObserveAgain => {
                    IssueExecutionRecoveryReasonState::ObserveAgain
                }
                IssueExecutionRecoveryReason::ContractRecovery => {
                    IssueExecutionRecoveryReasonState::ContractRecovery
                }
                IssueExecutionRecoveryReason::IdentityConflict => {
                    IssueExecutionRecoveryReasonState::IdentityConflict
                }
                IssueExecutionRecoveryReason::PullRequestClosed => {
                    IssueExecutionRecoveryReasonState::PullRequestClosed
                }
                IssueExecutionRecoveryReason::QueueRecovery => {
                    IssueExecutionRecoveryReasonState::QueueRecovery
                }
            },
            summary: recovery.summary.clone(),
            next_summary: recovery.next_summary.clone(),
            requires_human: recovery.requires_human,
        }
    }
}

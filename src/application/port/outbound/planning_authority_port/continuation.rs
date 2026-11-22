//! Explicit answers resume an owned worker session; they never acquire a fresh slot.
use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningWorkerQuestionIdentity {
    pub question_id: String,
    pub task_id: String,
    pub session_key: String,
    pub thread_id: String,
    pub slot_id: String,
    pub worktree_path: String,
    pub branch_name: String,
    pub lease_generation: String,
    pub awaiting_task_updated_at: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningWorkerContinuationRequest {
    pub authorization_epoch: u64,
    pub command_id: String,
    pub work_order_id: String,
    pub question: PlanningWorkerQuestionIdentity,
    pub message: String,
}

impl std::fmt::Debug for PlanningWorkerContinuationRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlanningWorkerContinuationRequest")
            .field("command_id", &self.command_id)
            .field("work_order_id", &self.work_order_id)
            .field("question_id", &self.question.question_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningWorkerContinuationReceipt {
    pub command_id: String,
    pub question_id: String,
    pub delivery: PlanningWorkerContinuationDelivery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanningWorkerContinuationDelivery {
    Queued,
    Unknown,
    Accepted {
        thread_id: String,
        turn_id: String,
        lease_generation: String,
    },
    RejectedBeforeStart {
        reason: PlanningWorkerContinuationRejection,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningWorkerContinuationRejection {
    StaleQuestion,
    WorkControlled,
    InvalidLease,
    PermitUnavailable,
    ContinuationPending,
}

impl PlanningWorkerContinuationRejection {
    pub fn code(self) -> &'static str {
        match self {
            Self::StaleQuestion => "stale_question",
            Self::WorkControlled => "work_controlled",
            Self::InvalidLease => "invalid_lease",
            Self::PermitUnavailable => "permit_unavailable",
            Self::ContinuationPending => "continuation_pending",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkerContinuationClaim {
    pub request: PlanningWorkerContinuationRequest,
    pub task: crate::domain::planning::TaskDefinition,
    pub previous_lease: ParallelModeSlotLeaseSnapshot,
    pub lease: ParallelModeSlotLeaseSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningWorkerContinuationClaimOutcome {
    Claimed(Box<PlanningWorkerContinuationClaim>),
    Waiting,
    Settled(PlanningWorkerContinuationReceipt),
}

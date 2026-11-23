//! Exact queue selection and the final authority CAS for an evidence observation.
use anyhow::Result;

use super::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use super::planning_work_order_repository_port::PlanningWorkPermitDecision;
use super::task_evidence_repository_port::TaskEvidenceTaskSnapshot;
use crate::domain::planning::delivery_evidence::{
    TaskDeliveryEvidenceError, TaskDeliveryEvidenceObservation, TaskEvidenceDeliveryStage,
};

#[derive(Debug, Clone, PartialEq)]
pub struct StoredTaskDeliveryEvidence {
    pub planning_revision: i64,
    pub evidence_revision: u64,
    pub queue: PlanningAuthorityDistributorQueueRecord,
    /// Missing legacy task/scope is an observable lack of evidence, not a fabricated empty task.
    pub task: Option<TaskEvidenceTaskSnapshot>,
    pub permit: PlanningWorkPermitDecision,
    pub requires_preservation: bool,
}

#[derive(Clone)]
pub enum TaskDeliveryEvidenceOperation {
    /// Only the active distributor claim may authorize this immediately following effect.
    Effect { owner_token: String },
    /// A read/rearm of one typed evidence wait; never authorizes an external effect.
    Recheck,
}

impl std::fmt::Debug for TaskDeliveryEvidenceOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Effect { .. } => "Effect { .. }",
            Self::Recheck => "Recheck",
        })
    }
}

#[derive(Debug, Clone)]
pub struct TaskDeliveryEvidenceObservationCommit {
    pub expected: StoredTaskDeliveryEvidence,
    pub operation: TaskDeliveryEvidenceOperation,
    pub observation: TaskDeliveryEvidenceObservation,
}

#[derive(Debug, Clone)]
pub struct TaskDeliveryEvidenceObservationCommitResult {
    pub updated_queue: PlanningAuthorityDistributorQueueRecord,
    /// The original immutable observation is returned when a duplicate is suppressed.
    pub observation: TaskDeliveryEvidenceObservation,
    pub changed: bool,
    /// A Recheck can only rearm a queue, even when the evidence is satisfied.
    pub effect_allowed: bool,
}

pub trait TaskDeliveryEvidenceRepositoryPort: Send + Sync {
    fn load_delivery_evidence(
        &self,
        _workspace: &str,
        _expected_queue: &PlanningAuthorityDistributorQueueRecord,
        _stage: TaskEvidenceDeliveryStage,
    ) -> Result<StoredTaskDeliveryEvidence> {
        Err(TaskDeliveryEvidenceError::Unavailable.into())
    }

    fn commit_delivery_evidence_observation(
        &self,
        _workspace: &str,
        _commit: &TaskDeliveryEvidenceObservationCommit,
    ) -> Result<TaskDeliveryEvidenceObservationCommitResult> {
        Err(TaskDeliveryEvidenceError::Unavailable.into())
    }
}

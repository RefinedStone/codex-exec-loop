//! Durable evidence is separate from task definitions. Every write compares current acceptance
//! scope and, for worker records, the exact active lease in one authority transaction.
use anyhow::Result;

use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;
use crate::domain::planning::TaskDefinition;
use crate::domain::planning::evidence::{
    TaskCriterionWaiverReceipt, TaskEvidenceArtifact, TaskEvidenceAttempt, TaskEvidenceEvent,
    TaskEvidenceReceipt, TaskEvidenceScope, TaskEvidenceStatus, TaskEvidenceTree,
    TaskEvidenceWriteResult,
};

use super::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use super::task_verification_repository_port::{
    StoredTaskVerification, TaskVerificationRepositoryPort,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEvidenceSnapshotQuery {
    /// Empty selects all canonical tasks, including done/skipped tasks, ordered by task id.
    pub task_ids: Vec<String>,
    /// Only permitted for the all-task page. Revisions must match before pages are combined.
    pub after_task_id: Option<String>,
    /// 1..=MAX_TASK_EVIDENCE_PAGE. A limit is never silently expanded.
    pub limit: usize,
}

impl Default for TaskEvidenceSnapshotQuery {
    fn default() -> Self {
        Self {
            task_ids: Vec::new(),
            after_task_id: None,
            limit: crate::domain::planning::evidence::MAX_TASK_EVIDENCE_PAGE,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskEvidenceStoredSnapshot {
    pub planning_revision: i64,
    pub evidence_revision: u64,
    pub tasks: Vec<TaskEvidenceTaskSnapshot>,
    pub truncated: bool,
    pub next_task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskEvidenceTaskSnapshot {
    pub task: TaskDefinition,
    pub scope: TaskEvidenceScope,
    pub leases: Vec<ParallelModeSlotLeaseSnapshot>,
    pub queue_records: Vec<PlanningAuthorityDistributorQueueRecord>,
    /// At most one latest finished record per current criterion. A prior scope may be returned
    /// for Stale projection; it must never become current by task/criterion id alone.
    pub evidence: Vec<TaskEvidenceReceipt>,
    pub waivers: Vec<TaskCriterionWaiverReceipt>,
    pub reviews: Vec<crate::domain::planning::evidence::HumanEvidenceReview>,
    /// Latest host attempt per criterion; worker-reported evidence remains separate.
    pub host_verifications: Vec<StoredTaskVerification>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredTaskEvidenceAttempt {
    pub attempt: TaskEvidenceAttempt,
    pub lease: ParallelModeSlotLeaseSnapshot,
    pub receipt: Option<TaskEvidenceReceipt>,
}

#[derive(Debug, Clone)]
pub struct TaskEvidencePrepareCommit {
    pub request_id: String,
    pub scope: TaskEvidenceScope,
    pub criterion_id: String,
    pub lease: ParallelModeSlotLeaseSnapshot,
    pub prepared_tree: TaskEvidenceTree,
    pub at: String,
}

#[derive(Debug, Clone)]
pub struct TaskEvidenceRecordCommit {
    pub attempt: TaskEvidenceAttempt,
    pub lease: ParallelModeSlotLeaseSnapshot,
    pub status: TaskEvidenceStatus,
    pub artifact: TaskEvidenceArtifact,
    pub recorded_tree: TaskEvidenceTree,
    pub note: Option<String>,
    pub by: String,
    pub at: String,
}

#[derive(Debug, Clone)]
pub struct TaskCriterionWaiverCommit {
    pub request_id: String,
    pub scope: TaskEvidenceScope,
    pub criterion_id: String,
    pub reason: String,
    /// Supplied only after the application validates a VerifiedHumanConfirmation exact target.
    pub confirmation_ref: String,
    pub at: String,
}

pub trait TaskEvidenceRepositoryPort: TaskVerificationRepositoryPort + Send + Sync {
    fn record_human_review(
        &self,
        _workspace: &str,
        _review: &crate::domain::planning::evidence::HumanEvidenceReview,
    ) -> Result<()> {
        anyhow::bail!("human review storage unavailable")
    }
    fn load_current_snapshot(
        &self,
        workspace: &str,
        query: &TaskEvidenceSnapshotQuery,
    ) -> Result<TaskEvidenceStoredSnapshot>;

    fn load_attempt(
        &self,
        workspace: &str,
        attempt_id: &str,
    ) -> Result<Option<StoredTaskEvidenceAttempt>>;

    fn prepare_attempt(
        &self,
        workspace: &str,
        commit: &TaskEvidencePrepareCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceAttempt>>;

    fn record_evidence(
        &self,
        workspace: &str,
        commit: &TaskEvidenceRecordCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceReceipt>>;

    fn waive_criterion(
        &self,
        workspace: &str,
        commit: &TaskCriterionWaiverCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskCriterionWaiverReceipt>>;

    fn pending_events(&self, workspace: &str, limit: usize) -> Result<Vec<TaskEvidenceEvent>>;

    fn acknowledge_event(&self, workspace: &str, event_id: &str) -> Result<()>;
}

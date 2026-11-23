//! The live repository owner reserves one execution; reads and retries never dispatch it.
use anyhow::Result;

use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;
use crate::domain::planning::evidence::{
    TaskEvidenceArtifact, TaskEvidenceError, TaskEvidenceWriteResult,
};
use crate::domain::planning::verification::{
    TaskVerificationAttempt, TaskVerificationExecutionManifest, TaskVerificationPhase,
    TaskVerificationReceipt, TaskVerificationResult,
};

#[derive(Clone, PartialEq, Eq)]
pub struct TaskVerificationExecutionKey {
    pub attempt_id: String,
    pub owner_token: String,
    pub run_token: String,
}

impl std::fmt::Debug for TaskVerificationExecutionKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TaskVerificationExecutionKey")
            .field("attempt_id", &self.attempt_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTaskVerification {
    pub attempt: TaskVerificationAttempt,
    pub lease: ParallelModeSlotLeaseSnapshot,
    pub phase: TaskVerificationPhase,
    pub manifest: Option<TaskVerificationExecutionManifest>,
    pub receipt: Option<TaskVerificationReceipt>,
}

#[derive(Clone)]
pub struct TaskVerificationClaimCommit {
    pub request_id: String,
    pub task_id: String,
    pub criterion_id: String,
    pub scope_id: String,
    pub expected_acceptance_revision: u64,
    pub owner_token: String,
    pub at: String,
}

#[derive(Debug, Clone)]
pub struct TaskVerificationClaimResult {
    pub stored: StoredTaskVerification,
    /// Present only for the transaction that first reserved the execution.
    pub execution: Option<TaskVerificationExecutionKey>,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct TaskVerificationBindCommit {
    pub execution: TaskVerificationExecutionKey,
    pub manifest: TaskVerificationExecutionManifest,
    pub at: String,
}

#[derive(Debug, Clone)]
pub struct TaskVerificationFinishCommit {
    pub execution: TaskVerificationExecutionKey,
    pub result: TaskVerificationResult,
    pub artifact: Option<TaskEvidenceArtifact>,
    pub at: String,
}

pub trait TaskVerificationRepositoryPort: Send + Sync {
    fn claim_host_verification(
        &self,
        _workspace: &str,
        _commit: &TaskVerificationClaimCommit,
    ) -> Result<TaskVerificationClaimResult> {
        Err(TaskEvidenceError::Unavailable.into())
    }

    /// Only the first Pending -> Running transition returns true. A retry cannot run again.
    fn bind_host_verification(
        &self,
        _workspace: &str,
        _commit: &TaskVerificationBindCommit,
    ) -> Result<bool> {
        Err(TaskEvidenceError::Unavailable.into())
    }

    fn finish_host_verification(
        &self,
        _workspace: &str,
        _commit: &TaskVerificationFinishCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskVerificationReceipt>> {
        Err(TaskEvidenceError::Unavailable.into())
    }

    fn load_host_verification(
        &self,
        _workspace: &str,
        _attempt_id: &str,
    ) -> Result<Option<StoredTaskVerification>> {
        Err(TaskEvidenceError::Unavailable.into())
    }

    /// Called only by a fresh live owner under the repository OS lock, never by queries.
    fn recover_incomplete_host_verifications(
        &self,
        _workspace: &str,
        _owner_token: &str,
        _at: &str,
    ) -> Result<Vec<TaskVerificationReceipt>> {
        Err(TaskEvidenceError::Unavailable.into())
    }
}

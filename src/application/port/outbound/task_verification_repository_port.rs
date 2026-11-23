//! The live repository owner reserves one execution; reads and retries never dispatch it.
use anyhow::Result;
use serde::{Deserialize, Serialize};

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
    /// Automatic completion must retain the captured lease instead of selecting a replacement.
    /// None preserves the original explicit-command request fingerprint and replay behavior.
    pub expected_lease: Option<ParallelModeSlotLeaseSnapshot>,
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

/// Internal automatic-completion policy, explicitly requested after a durable terminal fact.
/// Owner identity is intentionally neither serialized nor exposed through Debug.
#[derive(Clone)]
pub struct TaskVerificationFailureLimitCommit {
    pub attempt_id: String,
    pub expected_lease: ParallelModeSlotLeaseSnapshot,
    pub owner_token: String,
    pub at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskVerificationFailureLimitOutcome {
    NotFailed,
    BelowLimit,
    Superseded,
    AwaitingUser,
}

/// The original policy decision is returned on replay; it is not current execution authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskVerificationFailureLimitResult {
    pub outcome: TaskVerificationFailureLimitOutcome,
    pub failure_count: u32,
    pub planning_revision: Option<i64>,
    /// True only when this call changed the task to AwaitingUser, never on replay.
    pub changed: bool,
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

    /// Only automatic completion invokes this policy. Reads, explicit Host verification and
    /// outbox acknowledgement do not pause tasks or apply the policy retrospectively.
    fn apply_host_verification_failure_limit(
        &self,
        _workspace: &str,
        _commit: &TaskVerificationFailureLimitCommit,
    ) -> Result<TaskVerificationFailureLimitResult> {
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

//! Task evidence records the reported result and host-observed artifact/tree separately from
//! task definitions. A current acceptance scope is required before any historical fact applies.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod validation;
pub use validation::{
    criteria_sha256, normalize_evidence_note, normalize_waiver_reason,
    validate_evidence_identifier, validate_evidence_request_id, validate_evidence_token,
};

pub const MAX_TASK_EVIDENCE_PAGE: usize = 256;
pub const MAX_TASK_EVIDENCE_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_TASK_EVIDENCE_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_TASK_EVIDENCE_ATTEMPTS_PER_CRITERION: u32 = 64;
pub const MAX_TASK_EVIDENCE_RECEIPTS: u64 = 4096;
pub const MAX_TASK_EVIDENCE_NOTE_BYTES: usize = 4096;
pub const MAX_TASK_EVIDENCE_REVISION: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskEvidenceError {
    Invalid,
    Missing,
    ArtifactMissing,
    Stale,
    ScopeChanged,
    LeaseChanged,
    Conflict,
    Unavailable,
    Limit,
    HumanConfirmationRequired,
}

impl std::fmt::Display for TaskEvidenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "task evidence input is invalid",
            Self::Missing => "task evidence authority or record was not found",
            Self::ArtifactMissing => "the prepared evidence artifact is missing or empty",
            Self::Stale => "the evidence source changed",
            Self::ScopeChanged => "the task acceptance scope changed",
            Self::LeaseChanged => "the task no longer has the expected running lease",
            Self::Conflict => "the evidence request conflicts with its immutable receipt",
            Self::Unavailable => "current task evidence could not be observed",
            Self::Limit => "the bounded task evidence limit was reached",
            Self::HumanConfirmationRequired => {
                "criterion waiver requires Desktop human confirmation"
            }
        })
    }
}

impl std::error::Error for TaskEvidenceError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceScope {
    pub task_id: String,
    pub scope_id: String,
    pub acceptance_revision: u64,
    pub criteria_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceTree {
    pub head_commit_sha: String,
    pub tree_sha: String,
    pub worktree_clean: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskEvidenceStatus {
    Pass,
    Fail,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceAttempt {
    pub attempt_id: String,
    pub request_id: String,
    pub scope: TaskEvidenceScope,
    pub criterion_id: String,
    pub attempt: u32,
    pub slot_id: String,
    pub lease_generation: String,
    pub session_key: String,
    pub prepared_tree: TaskEvidenceTree,
    pub prepared_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceArtifact {
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCriterionEvidence {
    pub attempt: TaskEvidenceAttempt,
    pub status: TaskEvidenceStatus,
    pub artifact: TaskEvidenceArtifact,
    pub recorded_tree: TaskEvidenceTree,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Diagnostic attribution from the existing kernel caller, not authentication.
    pub by: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceReceipt {
    pub event_id: String,
    pub evidence_revision: u64,
    pub evidence: TaskCriterionEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCriterionWaiver {
    pub request_id: String,
    pub scope: TaskEvidenceScope,
    pub criterion_id: String,
    pub reason: String,
    pub reason_digest: String,
    pub confirmation_ref: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCriterionWaiverReceipt {
    pub event_id: String,
    pub evidence_revision: u64,
    pub waiver: TaskCriterionWaiver,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceWriteResult<T> {
    pub value: T,
    pub changed: bool,
}

/// Only this private command result exposes a writable artifact path. Planning projections and
/// durable kernel facts use the attempt and hash, never this path or artifact contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidencePrepared {
    pub attempt: TaskEvidenceAttempt,
    pub artifact_path: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskEvidenceEvent {
    Recorded {
        receipt: Box<TaskEvidenceReceipt>,
    },
    Waived {
        receipt: Box<TaskCriterionWaiverReceipt>,
    },
    VerificationFinished {
        receipt: Box<super::verification::TaskVerificationReceipt>,
    },
}

impl TaskEvidenceEvent {
    pub fn event_id(&self) -> &str {
        match self {
            Self::Recorded { receipt } => &receipt.event_id,
            Self::Waived { receipt } => &receipt.event_id,
            Self::VerificationFinished { receipt } => &receipt.event_id,
        }
    }

    pub fn evidence_revision(&self) -> u64 {
        match self {
            Self::Recorded { receipt } => receipt.evidence_revision,
            Self::Waived { receipt } => receipt.evidence_revision,
            Self::VerificationFinished { receipt } => receipt.evidence_revision,
        }
    }

    pub fn validate(&self) -> Result<(), TaskEvidenceError> {
        match self {
            Self::Recorded { receipt } => receipt.validate(),
            Self::Waived { receipt } => receipt.validate(),
            Self::VerificationFinished { receipt } => receipt.validate(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskCriterionEvidenceState {
    Pending,
    Pass,
    Fail,
    Blocked,
    Stale,
    Waived,
}

impl From<TaskEvidenceStatus> for TaskCriterionEvidenceState {
    fn from(status: TaskEvidenceStatus) -> Self {
        match status {
            TaskEvidenceStatus::Pass => Self::Pass,
            TaskEvidenceStatus::Fail => Self::Fail,
            TaskEvidenceStatus::Blocked => Self::Blocked,
        }
    }
}

#[cfg(test)]
mod tests;

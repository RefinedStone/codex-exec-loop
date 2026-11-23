//! Host execution facts are distinct from worker-reported evidence and current authorization.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::evidence::{TaskEvidenceArtifact, TaskEvidenceScope, TaskEvidenceTree};

mod validation;
pub(crate) use validation::validate_time;
#[cfg(test)]
mod tests;

pub const MAX_TASK_VERIFICATION_STREAM_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskVerificationRecipe {
    GitDiffCheckV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskVerificationPhase {
    Pending,
    Running,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskVerificationCurrentState {
    Pending,
    Running,
    Passed,
    Failed,
    NotRun,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskVerificationAttempt {
    pub attempt_id: String,
    pub request_id: String,
    pub scope: TaskEvidenceScope,
    pub criterion_id: String,
    pub recipe: TaskVerificationRecipe,
    pub attempt: u32,
    pub slot_id: String,
    pub lease_generation: String,
    pub session_key: String,
    pub requested_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskVerificationExecutionManifest {
    pub recipe: TaskVerificationRecipe,
    pub base_commit_sha: String,
    pub base_tree_sha: String,
    pub source: TaskEvidenceTree,
    pub git_binary_sha256: String,
    pub command_sha256: String,
    pub environment_sha256: String,
    pub configuration_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskVerificationStream {
    pub sha256: String,
    pub size_bytes: u64,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskVerificationExecutionOutcome {
    Exited { exit_code: i32 },
    TimedOut {},
    Cancelled {},
    OutputLimit {},
    Signalled {},
    IoFailure {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskVerificationObservation {
    pub outcome: TaskVerificationExecutionOutcome,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: u64,
    pub stdout: TaskVerificationStream,
    pub stderr: TaskVerificationStream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskVerificationNotRunReason {
    Unavailable,
    InvalidInput,
    Stale,
    ScopeChanged,
    LeaseChanged,
    ConfigurationChanged,
    SpawnFailed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskVerificationResult {
    Executed {
        observation: Box<TaskVerificationObservation>,
    },
    NotRun {
        reason: TaskVerificationNotRunReason,
    },
    Interrupted {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskVerificationReceipt {
    pub event_id: String,
    pub evidence_revision: u64,
    pub attempt: TaskVerificationAttempt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<TaskVerificationExecutionManifest>,
    pub result: TaskVerificationResult,
    /// A failed seal must retain the observed execution without claiming a verified pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<TaskEvidenceArtifact>,
    pub finished_at: String,
}

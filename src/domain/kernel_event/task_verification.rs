//! 호스트 실행의 공개 사실이다. 실행 capability와 원문 출력은 복사하지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::planning::evidence::{TaskEvidenceArtifact, TaskEvidenceScope};
use crate::domain::planning::verification::{
    TaskVerificationAttempt, TaskVerificationExecutionManifest, TaskVerificationReceipt,
    TaskVerificationRecipe, TaskVerificationResult,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskVerificationAttemptRecord {
    pub attempt_id: String,
    pub scope: TaskEvidenceScope,
    pub criterion_id: String,
    pub recipe: TaskVerificationRecipe,
    pub attempt: u32,
    pub requested_at: String,
}

impl From<&TaskVerificationAttempt> for TaskVerificationAttemptRecord {
    fn from(attempt: &TaskVerificationAttempt) -> Self {
        Self {
            attempt_id: attempt.attempt_id.clone(),
            scope: attempt.scope.clone(),
            criterion_id: attempt.criterion_id.clone(),
            recipe: attempt.recipe,
            attempt: attempt.attempt,
            requested_at: attempt.requested_at.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskVerificationRecord {
    pub attempt: TaskVerificationAttemptRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<TaskVerificationExecutionManifest>,
    pub result: TaskVerificationResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<TaskEvidenceArtifact>,
    pub finished_at: String,
}

impl From<&TaskVerificationReceipt> for TaskVerificationRecord {
    fn from(receipt: &TaskVerificationReceipt) -> Self {
        Self {
            attempt: (&receipt.attempt).into(),
            manifest: receipt.manifest.clone(),
            result: receipt.result.clone(),
            artifact: receipt.artifact.clone(),
            finished_at: receipt.finished_at.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::sample_task_verification_event;

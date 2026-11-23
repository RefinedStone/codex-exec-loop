//! 호스트 검증의 현재 판정과 불변 종료 사실을 분리한 공개 metadata다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::application::port::inbound::task_evidence_query_port as projection;
use crate::domain::kernel_event::{TaskVerificationAttemptRecord, TaskVerificationRecord};
use crate::domain::planning::evidence::TaskEvidenceError;
use crate::domain::planning::verification::{
    TaskVerificationCurrentState, TaskVerificationPhase, TaskVerificationReceipt,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskVerificationState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<TaskVerificationAttemptRecord>,
    pub phase: TaskVerificationPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<TaskVerificationReceiptState>,
    pub state: TaskVerificationCurrentState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<TaskEvidenceError>,
}

impl From<projection::TaskVerificationProjection> for TaskVerificationState {
    fn from(value: projection::TaskVerificationProjection) -> Self {
        Self {
            attempt: value.attempt.map(Into::into),
            phase: value.phase,
            receipt: value.receipt.map(Into::into),
            state: value.state,
            unavailable: value.unavailable,
        }
    }
}

impl From<projection::TaskVerificationAttemptProjection> for TaskVerificationAttemptRecord {
    fn from(value: projection::TaskVerificationAttemptProjection) -> Self {
        Self {
            attempt_id: value.attempt_id,
            scope: value.scope,
            criterion_id: value.criterion_id,
            recipe: value.recipe,
            attempt: value.attempt,
            requested_at: value.requested_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskVerificationReceiptState {
    pub event_id: String,
    pub evidence_revision: u64,
    pub verification: Box<TaskVerificationRecord>,
}

impl From<&TaskVerificationReceipt> for TaskVerificationReceiptState {
    fn from(receipt: &TaskVerificationReceipt) -> Self {
        Self {
            event_id: receipt.event_id.clone(),
            evidence_revision: receipt.evidence_revision,
            verification: Box::new(receipt.into()),
        }
    }
}

impl From<projection::TaskVerificationReceiptProjection> for TaskVerificationReceiptState {
    fn from(value: projection::TaskVerificationReceiptProjection) -> Self {
        Self {
            event_id: value.event_id,
            evidence_revision: value.evidence_revision,
            verification: Box::new(TaskVerificationRecord {
                attempt: value.attempt.into(),
                manifest: value.manifest,
                result: value.result,
                artifact: value.artifact,
                finished_at: value.finished_at,
            }),
        }
    }
}

#[cfg(test)]
mod tests;

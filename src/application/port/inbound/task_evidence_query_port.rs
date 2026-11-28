//! Planning projections expose metadata only. Artifact previews are a separate on-demand read.
use anyhow::Result;
use serde::Deserialize;

use crate::domain::planning::TaskAcceptanceCriterion;
use crate::domain::planning::evidence::{
    TaskCriterionEvidenceState, TaskEvidenceArtifact, TaskEvidenceError, TaskEvidenceScope,
    TaskEvidenceStatus, TaskEvidenceTree,
};
use crate::domain::planning::verification::{
    TaskVerificationAttempt, TaskVerificationCurrentState, TaskVerificationExecutionManifest,
    TaskVerificationPhase, TaskVerificationReceipt, TaskVerificationRecipe, TaskVerificationResult,
};

pub use crate::application::port::outbound::task_evidence_repository_port::TaskEvidenceSnapshotQuery;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEvidencePlanningSnapshot {
    pub planning_revision: Option<i64>,
    pub evidence_revision: Option<u64>,
    pub unavailable: Option<TaskEvidenceError>,
    pub tasks: Vec<TaskEvidenceTaskProjection>,
    pub truncated: bool,
    pub next_task_id: Option<String>,
}

impl TaskEvidencePlanningSnapshot {
    pub fn unavailable(reason: TaskEvidenceError) -> Self {
        Self {
            planning_revision: None,
            evidence_revision: None,
            unavailable: Some(reason),
            tasks: Vec::new(),
            truncated: false,
            next_task_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEvidenceTaskProjection {
    pub scope: TaskEvidenceScope,
    pub source: Option<TaskEvidenceSourceProjection>,
    pub unavailable: Option<TaskEvidenceError>,
    pub criteria: Vec<TaskCriterionEvidenceProjection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskEvidenceSourceKind {
    RunningLease,
    QueuedSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEvidenceSourceProjection {
    pub kind: TaskEvidenceSourceKind,
    pub tree: TaskEvidenceTree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCriterionEvidenceProjection {
    pub criterion: TaskAcceptanceCriterion,
    /// None means a current source could not be observed. It does not mean Pending or Pass.
    pub state: Option<TaskCriterionEvidenceState>,
    pub evidence: Option<TaskEvidenceRecordProjection>,
    pub waiver: Option<TaskCriterionWaiverProjection>,
    /// 호스트의 실행 사실은 작업자가 보고한 상태와 별도로 보존한다.
    pub host_verification: Option<TaskVerificationProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskVerificationProjection {
    pub attempt: Option<TaskVerificationAttemptProjection>,
    pub phase: TaskVerificationPhase,
    pub receipt: Option<TaskVerificationReceiptProjection>,
    pub state: TaskVerificationCurrentState,
    pub unavailable: Option<TaskEvidenceError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskVerificationAttemptProjection {
    pub attempt_id: String,
    pub scope: TaskEvidenceScope,
    pub criterion_id: String,
    pub recipe: TaskVerificationRecipe,
    pub attempt: u32,
    pub requested_at: String,
}

impl From<&TaskVerificationAttempt> for TaskVerificationAttemptProjection {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskVerificationReceiptProjection {
    pub event_id: String,
    pub evidence_revision: u64,
    pub attempt: TaskVerificationAttemptProjection,
    pub manifest: Option<TaskVerificationExecutionManifest>,
    pub result: TaskVerificationResult,
    pub artifact: Option<TaskEvidenceArtifact>,
    pub finished_at: String,
}

impl From<&TaskVerificationReceipt> for TaskVerificationReceiptProjection {
    fn from(receipt: &TaskVerificationReceipt) -> Self {
        Self {
            event_id: receipt.event_id.clone(),
            evidence_revision: receipt.evidence_revision,
            attempt: (&receipt.attempt).into(),
            manifest: receipt.manifest.clone(),
            result: receipt.result.clone(),
            artifact: receipt.artifact.clone(),
            finished_at: receipt.finished_at.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEvidenceRecordProjection {
    pub event_id: String,
    pub evidence_revision: u64,
    pub scope: TaskEvidenceScope,
    pub criterion_id: String,
    pub attempt_id: String,
    pub attempt: u32,
    pub status: TaskEvidenceStatus,
    pub artifact: TaskEvidenceArtifact,
    pub recorded_tree: TaskEvidenceTree,
    pub recorded_at: String,
    pub by: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCriterionWaiverProjection {
    pub event_id: String,
    pub evidence_revision: u64,
    pub scope: TaskEvidenceScope,
    pub criterion_id: String,
    pub reason_digest: String,
    pub recorded_at: String,
}

/// Selects the sealed worker report or host verification capture.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewKind {
    Worker,
    Host,
}

/// Identifies sealed evidence without accepting paths, content, or byte limits.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewTarget {
    pub task_id: String,
    pub criterion_id: String,
    pub scope_id: String,
    pub revision: u64,
    pub event_id: String,
    pub sha256: String,
    pub kind: PreviewKind,
}

pub trait TaskEvidenceQueryPort: Send + Sync {
    fn planning_snapshot(
        &self,
        query: &TaskEvidenceSnapshotQuery,
    ) -> Result<TaskEvidencePlanningSnapshot>;

    /// Returns sanitized text only for the matching current sealed evidence.
    fn sanitized_preview(&self, target: &PreviewTarget) -> Result<String>;
}

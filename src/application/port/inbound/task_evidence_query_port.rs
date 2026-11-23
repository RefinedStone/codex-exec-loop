//! Read projections contain host metadata, never writable paths or artifact/note contents.
use anyhow::Result;

use crate::domain::planning::TaskAcceptanceCriterion;
use crate::domain::planning::evidence::{
    TaskCriterionEvidenceState, TaskEvidenceArtifact, TaskEvidenceError, TaskEvidenceScope,
    TaskEvidenceStatus, TaskEvidenceTree,
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

pub trait TaskEvidenceQueryPort: Send + Sync {
    fn planning_snapshot(
        &self,
        query: &TaskEvidenceSnapshotQuery,
    ) -> Result<TaskEvidencePlanningSnapshot>;
}

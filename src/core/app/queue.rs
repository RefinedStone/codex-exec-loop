use crate::domain::planning::{
    PlanningQueueMutationReceipt, RuntimeProjection, TaskDefinition, TaskStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAuthoritySnapshot {
    pub runtime_projection: RuntimeProjection,
    pub planning_revision: i64,
    pub tasks: Vec<TaskDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueAuthorityLoadError {
    AuthorityUnavailable(String),
    RevisionsKeptChanging {
        projection_revision: i64,
        authority_revision: i64,
    },
    RuntimeProjectionUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMutationKind {
    RemoveSelected,
    UndoLatestRegistration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueMutationTarget {
    pub task_id: String,
    pub expected_status: TaskStatus,
    pub expected_updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueMutationIntent {
    pub workspace_directory: String,
    pub active_thread_id: Option<String>,
    pub kind: QueueMutationKind,
    pub expected_planning_revision: i64,
    pub targets: Vec<QueueMutationTarget>,
    pub receipt_at_start: Option<PlanningQueueMutationReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueMutationCorrelation {
    pub generation: u64,
    pub intent: QueueMutationIntent,
}

impl QueueMutationCorrelation {
    pub const fn new(generation: u64, intent: QueueMutationIntent) -> Self {
        Self { generation, intent }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueMutationCommitSnapshot {
    pub committed_planning_revision: i64,
    pub committed_task_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueMutationResult {
    pub mutation: Result<QueueMutationCommitSnapshot, String>,
    pub authority: Result<QueueAuthoritySnapshot, QueueAuthorityLoadError>,
}

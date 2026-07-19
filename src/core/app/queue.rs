use crate::domain::planning::{RuntimeProjection, TaskDefinition};

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

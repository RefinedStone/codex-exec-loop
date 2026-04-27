use anyhow::Result;

use crate::domain::planning::{PriorityQueueProjection, TaskLedgerDocument};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskAuthoritySnapshot {
    pub planning_revision: i64,
    pub task_ledger: TaskLedgerDocument,
    pub queue_projection: PriorityQueueProjection,
}

#[derive(Debug, Clone, Copy)]
pub struct PlanningTaskAuthorityCommit<'a> {
    pub observed_planning_revision: Option<i64>,
    pub task_ledger: &'a TaskLedgerDocument,
    pub queue_projection: &'a PriorityQueueProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningTaskAuthorityCommitResult {
    Committed {
        planning_revision: i64,
    },
    Conflict {
        observed_planning_revision: i64,
        current_planning_revision: i64,
    },
}

pub trait PlanningTaskRepositoryPort: Send + Sync {
    fn load_task_authority_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<PlanningTaskAuthoritySnapshot>>;

    fn commit_task_authority_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningTaskAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult>;

    fn clear_task_authority_snapshot(&self, workspace_dir: &str) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct NoopPlanningTaskRepositoryPort;

impl PlanningTaskRepositoryPort for NoopPlanningTaskRepositoryPort {
    fn load_task_authority_snapshot(
        &self,
        _workspace_dir: &str,
    ) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
        Ok(None)
    }

    fn commit_task_authority_snapshot(
        &self,
        _workspace_dir: &str,
        commit: PlanningTaskAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        Ok(PlanningTaskAuthorityCommitResult::Committed {
            planning_revision: commit.observed_planning_revision.unwrap_or(0),
        })
    }

    fn clear_task_authority_snapshot(&self, _workspace_dir: &str) -> Result<()> {
        Ok(())
    }
}

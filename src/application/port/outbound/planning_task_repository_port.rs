use anyhow::Result;

use crate::domain::planning::{PriorityQueueSnapshot, TaskLedgerDocument};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskAuthoritySnapshot {
    pub task_ledger: TaskLedgerDocument,
    pub queue_snapshot: PriorityQueueSnapshot,
}

#[derive(Debug, Clone, Copy)]
pub struct PlanningTaskAuthorityCommit<'a> {
    pub task_ledger: &'a TaskLedgerDocument,
    pub queue_snapshot: &'a PriorityQueueSnapshot,
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
    ) -> Result<()>;

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
        _commit: PlanningTaskAuthorityCommit<'_>,
    ) -> Result<()> {
        Ok(())
    }

    fn clear_task_authority_snapshot(&self, _workspace_dir: &str) -> Result<()> {
        Ok(())
    }
}

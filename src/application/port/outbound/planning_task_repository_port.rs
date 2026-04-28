use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use anyhow::Result;

use crate::domain::planning::{
    DirectionCatalogDocument, PriorityQueueProjection, TaskAuthorityDocument,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskAuthoritySnapshot {
    pub planning_revision: i64,
    pub task_authority: TaskAuthorityDocument,
    pub queue_projection: PriorityQueueProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDirectionAuthoritySnapshot {
    pub planning_revision: i64,
    pub directions: DirectionCatalogDocument,
}

#[derive(Debug, Clone, Copy)]
pub struct PlanningTaskAuthorityCommit<'a> {
    pub observed_planning_revision: Option<i64>,
    pub task_authority: &'a TaskAuthorityDocument,
    pub queue_projection: &'a PriorityQueueProjection,
}

#[derive(Debug, Clone, Copy)]
pub struct PlanningDirectionAuthorityCommit<'a> {
    pub observed_planning_revision: Option<i64>,
    pub directions: &'a DirectionCatalogDocument,
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
    fn load_direction_authority_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<PlanningDirectionAuthoritySnapshot>>;

    fn commit_direction_authority_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningDirectionAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult>;

    fn clear_direction_authority_snapshot(&self, workspace_dir: &str) -> Result<()>;

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
    fn load_direction_authority_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<PlanningDirectionAuthoritySnapshot>> {
        Ok(noop_direction_authority_store()
            .lock()
            .expect("noop direction authority store should not be poisoned")
            .get(workspace_dir)
            .cloned())
    }

    fn commit_direction_authority_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningDirectionAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        let mut store = noop_direction_authority_store()
            .lock()
            .expect("noop direction authority store should not be poisoned");
        let current_revision = store
            .get(workspace_dir)
            .map(|snapshot| snapshot.planning_revision)
            .unwrap_or(0);
        if let Some(observed_revision) = commit.observed_planning_revision
            && observed_revision != current_revision
        {
            return Ok(PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: observed_revision,
                current_planning_revision: current_revision,
            });
        }
        let planning_revision = current_revision + 1;
        store.insert(
            workspace_dir.to_string(),
            PlanningDirectionAuthoritySnapshot {
                planning_revision,
                directions: commit.directions.clone(),
            },
        );
        Ok(PlanningTaskAuthorityCommitResult::Committed { planning_revision })
    }

    fn clear_direction_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        noop_direction_authority_store()
            .lock()
            .expect("noop direction authority store should not be poisoned")
            .remove(workspace_dir);
        Ok(())
    }

    fn load_task_authority_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
        Ok(noop_task_authority_store()
            .lock()
            .expect("noop task authority store should not be poisoned")
            .get(workspace_dir)
            .cloned())
    }

    fn commit_task_authority_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningTaskAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        let mut store = noop_task_authority_store()
            .lock()
            .expect("noop task authority store should not be poisoned");
        let current_revision = store
            .get(workspace_dir)
            .map(|snapshot| snapshot.planning_revision)
            .unwrap_or(0);
        if let Some(observed_revision) = commit.observed_planning_revision
            && observed_revision != current_revision
        {
            return Ok(PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: observed_revision,
                current_planning_revision: current_revision,
            });
        }
        let planning_revision = current_revision + 1;
        store.insert(
            workspace_dir.to_string(),
            PlanningTaskAuthoritySnapshot {
                planning_revision,
                task_authority: commit.task_authority.clone(),
                queue_projection: commit.queue_projection.clone(),
            },
        );
        Ok(PlanningTaskAuthorityCommitResult::Committed { planning_revision })
    }

    fn clear_task_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        noop_task_authority_store()
            .lock()
            .expect("noop task authority store should not be poisoned")
            .remove(workspace_dir);
        Ok(())
    }
}

fn noop_task_authority_store() -> &'static Mutex<BTreeMap<String, PlanningTaskAuthoritySnapshot>> {
    static STORE: OnceLock<Mutex<BTreeMap<String, PlanningTaskAuthoritySnapshot>>> =
        OnceLock::new();
    STORE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn noop_direction_authority_store()
-> &'static Mutex<BTreeMap<String, PlanningDirectionAuthoritySnapshot>> {
    static STORE: OnceLock<Mutex<BTreeMap<String, PlanningDirectionAuthoritySnapshot>>> =
        OnceLock::new();
    STORE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

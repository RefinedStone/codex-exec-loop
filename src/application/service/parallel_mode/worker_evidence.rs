//! Optional evidence output preparation uses the existing lease/pool boundary.
//! Preparation failure is a notice; it never becomes a new E2 dispatch gate.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::task_evidence_artifact_port::TaskEvidenceArtifactPort;
use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};
use crate::domain::planning::evidence::validate_evidence_token;
use crate::domain::planning::validate_task_acceptance;

use super::{
    ParallelModeService, acquire_pool_mutation_lock, resolve_workspace_slot_lease_with_runtime,
};

pub(crate) const EVIDENCE_PREPARATION_NOTICE: &str =
    "완료 기준 증거를 기록할 준비가 되지 않았습니다. 작업 실행은 계속합니다.";

#[derive(Clone)]
pub(super) struct WorkerEvidenceConfiguration {
    pub(super) repository: Arc<dyn PlanningTaskRepositoryPort>,
    pub(super) artifacts: Arc<dyn TaskEvidenceArtifactPort>,
}

impl ParallelModeService {
    pub(crate) fn prepare_worker_evidence_incoming(
        &self,
        workspace: &str,
        expected: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<Option<PathBuf>, String> {
        let Some(configuration) = self.task_evidence.as_ref() else {
            return Ok(None);
        };
        let snapshot = configuration
            .repository
            .load_task_authority_snapshot(workspace)
            .map_err(|error| error.to_string())?
            .ok_or("worker task authority is unavailable for evidence preparation")?;
        let task = snapshot
            .task_authority
            .tasks
            .iter()
            .find(|task| task.id == expected.task_id)
            .ok_or_else(|| {
                "worker task authority is unavailable for evidence preparation".to_string()
            })?;
        if task.acceptance.is_empty() {
            return Ok(None);
        }
        validate_task_acceptance(&task.acceptance, task.acceptance_revision)
            .map_err(|error| error.to_string())?;
        if task.acceptance_revision == 0 {
            return Err("worker acceptance revision is unavailable".into());
        }
        let generation = expected
            .lease_generation
            .as_deref()
            .ok_or("worker evidence needs a lease generation")?;
        validate_evidence_token(generation).map_err(|error| error.to_string())?;
        let permit = acquire_pool_mutation_lock(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &expected.worktree_path,
        )?;
        let verify = || -> Result<(), String> {
            let current = resolve_workspace_slot_lease_with_runtime(
                self.parallel_runtime.as_ref(),
                self.planning_authority.as_ref(),
                &expected.worktree_path,
            )?
            .ok_or("worker evidence lease disappeared")?;
            permit.verify_pool_root(&current.context.pool_root)?;
            if !matches!(
                current.lease.state,
                ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running
            ) || !current.lease.same_generation_as(expected)
                || !self.parallel_runtime.paths_match_securely(
                    Path::new(workspace),
                    &current.context.canonical_repo_root,
                )
            {
                return Err("worker evidence lease identity changed".into());
            }
            Ok(())
        };
        verify()?;
        let root = configuration
            .artifacts
            .prepare_lease_incoming_root(workspace, generation)
            .map_err(|error| error.to_string())?;
        verify()?;
        let refreshed = configuration
            .repository
            .load_task_authority_snapshot(workspace)
            .map_err(|error| error.to_string())?
            .ok_or("worker task authority disappeared during evidence preparation")?;
        let unchanged = refreshed
            .task_authority
            .tasks
            .iter()
            .find(|current| current.id == task.id)
            .is_some_and(|current| {
                current.acceptance_revision == task.acceptance_revision
                    && current.acceptance == task.acceptance
            });
        if !unchanged {
            return Err("worker acceptance changed during evidence preparation".into());
        }
        Ok(Some(root))
    }
}

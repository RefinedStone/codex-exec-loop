use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelWorkerCommitOutcome, ParallelWorkerCommitRequest,
};
use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};

use super::{
    ParallelModeService, acquire_pool_mutation_lock, resolve_workspace_head_sha,
    resolve_workspace_slot_lease,
};

impl ParallelModeService {
    pub(crate) fn prepare_workspace_worker_commit(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<ParallelWorkerCommitOutcome, String> {
        let mutation_lock = acquire_pool_mutation_lock(
            self.planning_authority.as_ref(),
            &expected_lease.worktree_path,
        )?;
        let resolution = self.resolve_exact_running_worker_lease(expected_lease)?;
        mutation_lock.verify_pool_root(&resolution.context.pool_root)?;
        let delivery_target = resolution
            .lease
            .delivery_target
            .as_ref()
            .ok_or_else(|| "parallel worker lease has no frozen delivery target".to_string())?;
        let expected_head =
            resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
                format!(
                    "slot `{}` HEAD could not be resolved before host commit",
                    resolution.lease.slot_id
                )
            })?;
        let commit_message = format!("akra: complete {}", resolution.lease.branch_name);
        let outcome =
            self.parallel_runtime
                .prepare_parallel_worker_commit(ParallelWorkerCommitRequest {
                    workspace_directory: &resolution.lease.worktree_path,
                    expected_branch_name: &resolution.lease.branch_name,
                    expected_base_commit_sha: &delivery_target.integration_base_commit_sha,
                    expected_head_commit_sha: &expected_head,
                    commit_message: &commit_message,
                    commit_timestamp: &resolution.lease.leased_at,
                })?;

        let refreshed = self.resolve_exact_running_worker_lease(expected_lease)?;
        mutation_lock.verify_pool_root(&refreshed.context.pool_root)?;
        let refreshed_head =
            resolve_workspace_head_sha(&refreshed.workspace_path).ok_or_else(|| {
                format!(
                    "slot `{}` HEAD could not be resolved after host commit",
                    refreshed.lease.slot_id
                )
            })?;
        if refreshed_head != outcome.commit_sha {
            return Err(format!(
                "slot `{}` HEAD changed after host commit; expected `{}`",
                refreshed.lease.slot_id, outcome.commit_sha
            ));
        }
        Ok(outcome)
    }

    fn resolve_exact_running_worker_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<super::WorkspaceSlotLeaseResolution, String> {
        let resolution = resolve_workspace_slot_lease(
            self.planning_authority.as_ref(),
            &expected_lease.worktree_path,
        )?
        .ok_or_else(|| {
            format!(
                "slot `{}` no longer has an active worker lease",
                expected_lease.slot_id
            )
        })?;
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Err(format!(
                "slot `{}` is not Running during host commit",
                expected_lease.slot_id
            ));
        }
        if !resolution.lease.same_generation_as(expected_lease) {
            return Err(format!(
                "slot `{}` lease identity changed before host commit",
                expected_lease.slot_id
            ));
        }
        Ok(resolution)
    }
}

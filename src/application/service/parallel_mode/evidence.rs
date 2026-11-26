//! Evidence inspection shares the worker-pool mutation exclusion used by host completion.
//! A busy pool is an unavailable observation; a UI query never waits for allocation.
//! Official completion waits for the pool until its own deadline, so one contended
//! lock does not fail the completion.
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelModeRuntimePort, ParallelWorkerTreeRequest,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};
use crate::domain::planning::evidence::{TaskEvidenceError, TaskEvidenceTree};

use super::pool::{
    derive_default_pool_root, detect_canonical_repo_root, resolve_workspace_head_sha_with_runtime,
    resolve_workspace_slot_lease_with_runtime, try_acquire_pool_mutation_lock_at,
};

const POOL_LOCK_RETRY: Duration = Duration::from_millis(25);

pub(crate) fn with_running_worker<T>(
    authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    workspace: &str,
    expected: &ParallelModeSlotLeaseSnapshot,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    with_running_worker_until(authority, runtime, workspace, expected, None, operation)
}

/// `wait_until` 까지 pool 잠금을 다시 시도한다. `None` 이면 한 번만 시도한다.
pub(crate) fn with_running_worker_until<T>(
    authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    workspace: &str,
    expected: &ParallelModeSlotLeaseSnapshot,
    wait_until: Option<Instant>,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let canonical =
        detect_canonical_repo_root(authority, workspace).ok_or(TaskEvidenceError::Unavailable)?;
    if !runtime.paths_match_securely(&canonical, Path::new(workspace)) {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    let pool = derive_default_pool_root(&canonical);
    let guard = loop {
        if let Some(guard) = try_acquire_pool_mutation_lock_at(runtime, &pool)
            .map_err(|_| TaskEvidenceError::Unavailable)?
        {
            break guard;
        }
        match wait_until {
            Some(deadline) if Instant::now() + POOL_LOCK_RETRY < deadline => {
                std::thread::sleep(POOL_LOCK_RETRY)
            }
            _ => return Err(TaskEvidenceError::Unavailable.into()),
        }
    };
    let verify = || -> Result<()> {
        let resolution =
            resolve_workspace_slot_lease_with_runtime(runtime, authority, &expected.worktree_path)
                .map_err(|_| TaskEvidenceError::Unavailable)?
                .ok_or(TaskEvidenceError::LeaseChanged)?;
        guard
            .verify_pool_root(&resolution.context.pool_root)
            .map_err(|_| TaskEvidenceError::Unavailable)?;
        if !runtime.paths_match_securely(&canonical, &resolution.context.canonical_repo_root)
            || resolution.lease.state != ParallelModeSlotLeaseState::Running
            || !resolution.lease.same_generation_as(expected)
            || resolution.lease.lease_generation.is_none()
        {
            return Err(TaskEvidenceError::LeaseChanged.into());
        }
        if authority
            .worker_task_requires_preservation(workspace, &expected.task_id)
            .map_err(|_| TaskEvidenceError::Unavailable)?
        {
            return Err(TaskEvidenceError::LeaseChanged.into());
        }
        Ok(())
    };
    verify()?;
    let result = operation()?;
    verify()?;
    Ok(result)
}

pub(crate) fn snapshot_worker_tree(
    runtime: &dyn ParallelModeRuntimePort,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<TaskEvidenceTree> {
    let target = lease
        .delivery_target
        .as_ref()
        .ok_or(TaskEvidenceError::LeaseChanged)?;
    let head = resolve_workspace_head_sha_with_runtime(runtime, Path::new(&lease.worktree_path))
        .ok_or(TaskEvidenceError::Unavailable)?;
    let value = runtime
        .snapshot_parallel_worker_tree(ParallelWorkerTreeRequest {
            workspace_directory: &lease.worktree_path,
            expected_branch_name: &lease.branch_name,
            expected_base_commit_sha: &target.integration_base_commit_sha,
            expected_head_commit_sha: &head,
        })
        .map_err(|_| TaskEvidenceError::Unavailable)?;
    let tree = TaskEvidenceTree {
        head_commit_sha: value.head_commit_sha,
        tree_sha: value.tree_sha,
        worktree_clean: value.worktree_clean,
    };
    tree.validate()?;
    Ok(tree)
}

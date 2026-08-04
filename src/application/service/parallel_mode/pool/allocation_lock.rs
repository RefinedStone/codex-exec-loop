use std::path::Path;
use std::time::Duration;

use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelModeRuntimePort, ParallelPoolMutationPermit,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;

use super::{derive_default_pool_root, detect_canonical_repo_root};

pub(super) const POOL_MUTATION_LOCK_FILE: &str = ".allocation-lock";
const POOL_MUTATION_LOCK_TIMEOUT: Duration = Duration::from_secs(120);
const POOL_MUTATION_LOCK_RETRY: Duration = Duration::from_millis(25);

pub(in crate::application::service::parallel_mode) struct PoolMutationLock {
    permit: Box<dyn ParallelPoolMutationPermit>,
}

impl PoolMutationLock {
    pub(in crate::application::service::parallel_mode) fn verify_pool_root(
        &self,
        pool_root: &Path,
    ) -> Result<(), String> {
        self.permit.verify_pool_root(pool_root)
    }
}

pub(in crate::application::service::parallel_mode) fn acquire_pool_mutation_lock(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    workspace_dir: &str,
) -> Result<PoolMutationLock, String> {
    let canonical_repo_root = detect_canonical_repo_root(planning_authority, workspace_dir)
        .ok_or_else(|| "canonical root inspection failed".to_string())?;
    let pool_root = derive_default_pool_root(&canonical_repo_root);
    runtime
        .acquire_pool_mutation_permit(
            &canonical_repo_root,
            &pool_root,
            POOL_MUTATION_LOCK_TIMEOUT,
            POOL_MUTATION_LOCK_RETRY,
        )
        .map(|permit| PoolMutationLock { permit })
}

pub(in crate::application::service::parallel_mode) fn try_acquire_pool_mutation_lock_at(
    runtime: &dyn ParallelModeRuntimePort,
    pool_root: &Path,
) -> Result<Option<PoolMutationLock>, String> {
    runtime
        .try_acquire_pool_mutation_permit(pool_root)
        .map(|permit| permit.map(|permit| PoolMutationLock { permit }))
}

use std::collections::BTreeSet;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;

use super::pool::{PoolRuntimeContext, detect_canonical_repo_root, inspect_slot_git_status};
use super::{DISTRIBUTOR_INTEGRATION_BRANCH, current_branch_name};

pub(super) fn parallel_dispatch_excluded_task_ids(context: &PoolRuntimeContext) -> Vec<String> {
    let mut task_ids = BTreeSet::new();
    task_ids.extend(
        context
            .slot_leases
            .values()
            .map(|lease| lease.task_id.trim().to_string())
            .filter(|task_id| !task_id.is_empty()),
    );
    task_ids.extend(
        context
            .distributor_queue_records
            .iter()
            .filter(|record| record.queue_state.is_active())
            .map(|record| record.task_id.trim().to_string())
            .filter(|task_id| !task_id.is_empty()),
    );

    task_ids.into_iter().collect()
}

pub(super) fn inspect_akra_integration_worktree_blocker(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> Option<String> {
    let canonical_repo_root = detect_canonical_repo_root(planning_authority, workspace_dir)?;
    let branch_name = current_branch_name(&canonical_repo_root)?;
    if branch_name != DISTRIBUTOR_INTEGRATION_BRANCH {
        return Some(format!(
            "orchestrator blocked / integration worktree must be checked out to `{DISTRIBUTOR_INTEGRATION_BRANCH}` but is `{branch_name}`"
        ));
    }

    let status = inspect_slot_git_status(&canonical_repo_root)?;
    if !status.is_ready_for_integration() {
        return Some(format!(
            "orchestrator blocked / integration worktree must be clean before queue processing: {}",
            status.detail_label()
        ));
    }

    None
}

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
};

use super::paths::{derive_default_pool_root, display_pool_path};
use super::slot_inspection::inspect_pool_slot;
use super::{DEFAULT_POOL_SIZE, PoolRuntimeContext, detect_canonical_repo_root, slot_id};

pub(super) fn build_pool_board_from_context(
    context: &PoolRuntimeContext,
    reconcile_status: impl Into<String>,
) -> ParallelModePoolBoardSnapshot {
    let slots = build_pool_slots(context);
    let pool_root_label = display_pool_path(&context.canonical_repo_root, &context.pool_root);

    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
}

pub(super) fn build_pool_slots(context: &PoolRuntimeContext) -> Vec<ParallelModePoolSlotSnapshot> {
    (1..=DEFAULT_POOL_SIZE)
        .map(|slot_number| inspect_pool_slot(context, &slot_id(slot_number)))
        .collect::<Vec<_>>()
}

pub(super) fn build_unavailable_pool_board(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    reconcile_status: impl Into<String>,
    branch_name: &str,
    worktree_label: &str,
    owner_label: &str,
) -> ParallelModePoolBoardSnapshot {
    let pool_root_label = derive_pool_root_label(planning_authority, workspace_dir);
    let slots = (1..=DEFAULT_POOL_SIZE)
        .map(|slot_number| {
            ParallelModePoolSlotSnapshot::new(
                slot_id(slot_number),
                ParallelModePoolSlotState::Unavailable,
                branch_name,
                worktree_label,
                owner_label,
            )
        })
        .collect::<Vec<_>>();

    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
}

pub(super) fn build_blocked_pool_board(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    reconcile_status: impl Into<String>,
    detail: &str,
) -> ParallelModePoolBoardSnapshot {
    let pool_root_label = derive_pool_root_label(planning_authority, workspace_dir);
    let slots = (1..=DEFAULT_POOL_SIZE)
        .map(|slot_number| {
            ParallelModePoolSlotSnapshot::new(
                slot_id(slot_number),
                ParallelModePoolSlotState::Blocked,
                "unknown",
                detail,
                "operator recovery",
            )
        })
        .collect::<Vec<_>>();

    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
}
fn derive_pool_root_label(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> String {
    detect_canonical_repo_root(planning_authority, workspace_dir)
        .map(|canonical_repo_root| {
            let pool_root = derive_default_pool_root(&canonical_repo_root);
            display_pool_path(&canonical_repo_root, &pool_root)
        })
        .unwrap_or_else(|| "not available".to_string())
}

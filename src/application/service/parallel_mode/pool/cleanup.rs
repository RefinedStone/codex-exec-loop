use std::path::Path;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModePoolSlotCleanupDecision, ParallelModeSlotLeaseState,
};

use super::super::git_sequence::{GitCommandStep, run_git_sequence};
use super::super::readiness::command_succeeds;
use super::{
    AKRA_AGENT_BRANCH_PREFIX, DEFAULT_POOL_SIZE, GitWorktreeRecord, POOL_BASELINE_BRANCH,
    SlotGitStatus, inspect_slot_git_status, load_runtime_projection_snapshot, remove_slot_lease,
    slot_id,
};

pub(super) fn cleanup_reusable_slots(
    planning_authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
) -> usize {
    let mut cleaned_slots = 0;
    let slot_leases = load_runtime_projection_snapshot(planning_authority, repo_root).slot_leases;

    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_id = slot_id(slot_number);
        let slot_path = pool_root.join(&slot_id);
        let Some(worktree_record) = worktree_records
            .iter()
            .find(|record| record.path == slot_path)
        else {
            continue;
        };
        let Some(branch_name) = worktree_record.branch_name.as_deref() else {
            continue;
        };
        let expected_agent_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
        if !branch_name.starts_with(&expected_agent_prefix) {
            continue;
        }
        let slot_lease = slot_leases.get(&slot_id);
        let lease_state = slot_lease.map(|lease| lease.state);
        let worktree_clean = lease_state.is_none()
            && inspect_slot_git_status(&slot_path).is_some_and(SlotGitStatus::is_clean_baseline);
        let branch_integrated = !matches!(
            lease_state,
            Some(ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running)
        ) && (matches!(
            lease_state,
            Some(ParallelModeSlotLeaseState::CleanupPending)
        ) || worktree_clean)
            && branch_is_cleanup_ready(repo_root, branch_name);
        let cleanup_ready = ParallelModePoolSlotCleanupDecision::new(
            lease_state,
            worktree_clean,
            branch_integrated,
        )
        .is_cleanup_ready();
        if !cleanup_ready {
            continue;
        }
        if cleanup_slot(
            planning_authority,
            repo_root,
            pool_root,
            &slot_id,
            &slot_path,
            branch_name,
        ) {
            cleaned_slots += 1;
        }
    }

    cleaned_slots
}

fn branch_is_integrated_into_akra(repo_root: &str, branch_name: &str) -> bool {
    branch_is_integrated_into(repo_root, branch_name, POOL_BASELINE_BRANCH)
}

pub(in crate::application::service::parallel_mode) fn branch_is_integrated_into(
    repo_root: &str,
    branch_name: &str,
    base_branch: &str,
) -> bool {
    command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "merge-base",
            "--is-ancestor",
            branch_name,
            base_branch,
        ],
    )
}

pub(in crate::application::service::parallel_mode) fn branch_is_cleanup_ready(
    repo_root: &str,
    branch_name: &str,
) -> bool {
    branch_is_integrated_into_akra(repo_root, branch_name)
}

pub(in crate::application::service::parallel_mode) fn cleanup_slot(
    planning_authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    pool_root: &Path,
    slot_id: &str,
    slot_path: &Path,
    branch_name: &str,
) -> bool {
    let reset_report = reset_slot_worktree_to_akra(slot_path);
    if !reset_report.succeeded() {
        let _failure_summary = reset_report.failure_summary();
        return false;
    }
    let delete_branch = run_git_sequence(
        "delete cleaned slot branch",
        vec![GitCommandStep::new(
            "delete agent branch",
            ["-C", repo_root, "branch", "-D", branch_name],
        )],
    );
    if !delete_branch.succeeded() {
        let _failure_summary = delete_branch.failure_summary();
        return false;
    }
    if !remove_slot_lease(planning_authority, repo_root, pool_root, slot_id) {
        return false;
    }

    inspect_slot_git_status(slot_path).is_some_and(SlotGitStatus::is_clean_baseline)
}

pub(in crate::application::service::parallel_mode) fn reset_slot_worktree_to_akra(
    slot_path: &Path,
) -> super::super::git_sequence::GitCommandSequenceReport {
    let slot_path_string = slot_path.display().to_string();
    run_git_sequence(
        "reset slot worktree to pool baseline",
        vec![
            GitCommandStep::new(
                "checkout pool baseline detached",
                [
                    "-C",
                    slot_path_string.as_str(),
                    "checkout",
                    "--detach",
                    POOL_BASELINE_BRANCH,
                ],
            ),
            GitCommandStep::new(
                "hard reset to pool baseline",
                [
                    "-C",
                    slot_path_string.as_str(),
                    "reset",
                    "--hard",
                    POOL_BASELINE_BRANCH,
                ],
            ),
            GitCommandStep::new(
                "clean untracked files",
                ["-C", slot_path_string.as_str(), "clean", "-fdx"],
            ),
        ],
    )
}

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityPort,
    PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModePoolBoardSnapshot,
    ParallelModePoolSlotCleanupDecision, ParallelModePoolSlotSnapshot,
    ParallelModeReadinessSnapshot, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};

use super::current_branch_name;
use super::git_sequence::{GitCommandStep, run_git_sequence};
use super::readiness::{command_succeeds, detect_git_repo_root, run_command};
use super::{
    AKRA_AGENT_BRANCH_PREFIX, DEFAULT_POOL_SIZE, NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL,
    NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_NEXT_ACTION, POOL_BASELINE_BRANCH,
    ensure_directory_exists,
};

mod allocation_lock;
mod board;
mod lease_store;
mod paths;
mod slot_inspection;

pub(super) use self::allocation_lock::acquire_pool_allocation_lock;
use self::board::{
    build_blocked_pool_board, build_pool_board_from_context,
    build_pool_slots as build_pool_slots_from_context, build_unavailable_pool_board,
};
#[cfg(test)]
pub(super) use self::lease_store::slot_lease_file_path;
pub(super) use self::lease_store::{remove_slot_lease, write_slot_lease};
use self::paths::{
    annotate_worktree_label, canonicalize_best_effort, parse_worktree_records, resolve_branch_head,
    resolve_pool_baseline_head, worktree_paths_match,
};
pub(super) use self::paths::{derive_default_pool_root, inspect_slot_git_status};
pub(super) use self::slot_inspection::pool_operator_recovery_notice;
use self::slot_inspection::summarize_pool_reconcile_status;

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitWorktreeRecord {
    path: PathBuf,
    head_sha: String,
    branch_name: Option<String>,
    detached: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SlotGitStatus {
    has_staged: bool,
    has_unstaged: bool,
    has_untracked: bool,
    pub(super) has_pending_operation: bool,
}

impl SlotGitStatus {
    pub(super) fn is_clean_baseline(self) -> bool {
        !self.has_staged && !self.has_unstaged && !self.has_untracked && !self.has_pending_operation
    }

    pub(super) fn is_ready_for_integration(self) -> bool {
        !self.has_staged && !self.has_unstaged && !self.has_pending_operation
    }

    pub(super) fn detail_label(self) -> String {
        let mut details = Vec::new();
        if self.has_staged {
            details.push("staged changes");
        }
        if self.has_unstaged {
            details.push("unstaged changes");
        }
        if self.has_untracked {
            details.push("untracked files");
        }
        if self.has_pending_operation {
            details.push("merge/rebase metadata");
        }

        if details.is_empty() {
            "clean".to_string()
        } else {
            details.join(", ")
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PoolReconcileExecution {
    created_baseline_branch: bool,
    created_pool_root: bool,
    provisioned_slots: usize,
    cleaned_slots: usize,
}

impl PoolReconcileExecution {
    fn has_actions(self) -> bool {
        self.created_baseline_branch
            || self.created_pool_root
            || self.provisioned_slots > 0
            || self.cleaned_slots > 0
    }
}

#[derive(Debug, Clone)]
pub(super) struct PoolRuntimeContext {
    pub(super) repo_root: String,
    pub(super) canonical_repo_root: PathBuf,
    pub(super) pool_root: PathBuf,
    baseline_head: String,
    worktree_records: Vec<GitWorktreeRecord>,
    pub(super) slot_leases: BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
    invalid_slot_leases: BTreeSet<String>,
    pub(super) session_details: Vec<ParallelModeAgentSessionDetailSnapshot>,
    pub(super) distributor_queue_records: Vec<PlanningAuthorityDistributorQueueRecord>,
}

pub(super) type PoolBoardWithContextResult = Result<
    (PoolRuntimeContext, ParallelModePoolBoardSnapshot),
    Box<(ParallelModePoolBoardSnapshot, String)>,
>;

#[derive(Debug, Clone)]
pub(super) struct WorkspaceSlotLeaseResolution {
    pub(super) context: PoolRuntimeContext,
    pub(super) lease: ParallelModeSlotLeaseSnapshot,
    pub(super) workspace_path: PathBuf,
}

pub(super) fn build_pool_board(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModePoolBoardSnapshot {
    match readiness_snapshot {
        Some(snapshot) if snapshot.allows_parallel_mode() => {
            inspect_pool_board(planning_authority, workspace_dir)
        }
        Some(snapshot) => build_unavailable_pool_board(
            planning_authority,
            workspace_dir,
            format!(
                "reconcile blocked / readiness: {}",
                snapshot.readiness_label()
            ),
            "not leased",
            "reconcile blocked by readiness gate",
            "supervisor gate",
        ),
        None => build_unavailable_pool_board(
            planning_authority,
            workspace_dir,
            "reconcile pending / run readiness first",
            "not inspected",
            "readiness has not been checked",
            "n/a",
        ),
    }
}

pub(super) fn reconcile_pool_board(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> ParallelModePoolBoardSnapshot {
    match reconcile_pool_board_and_context(planning_authority, workspace_dir) {
        Ok((_, pool)) => pool,
        Err(error) => {
            let (pool, _) = *error;
            pool
        }
    }
}

pub(super) fn reconcile_pool_board_and_context(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> PoolBoardWithContextResult {
    let Some(repo_root) = detect_git_repo_root(workspace_dir) else {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile failed / git repository is unavailable",
                "repository inspection failed",
            ),
            "repository inspection failed".to_string(),
        )));
    };
    let Some(canonical_repo_root) = detect_canonical_repo_root(planning_authority, workspace_dir)
    else {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile failed / canonical repository root is unavailable",
                "canonical root inspection failed",
            ),
            "canonical root inspection failed".to_string(),
        )));
    };
    let pool_root = derive_default_pool_root(&canonical_repo_root);
    let pool_root_existed = pool_root.exists();
    if ensure_directory_exists(&pool_root).is_err() {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile failed / pool root could not be created",
                "pool root creation failed",
            ),
            "pool root creation failed".to_string(),
        )));
    }
    let created_pool_root = !pool_root_existed;
    let runtime_projection = load_runtime_projection_snapshot(planning_authority, &repo_root);
    let can_refresh_pool_baseline =
        can_refresh_pool_baseline_from_workspace(&repo_root, &runtime_projection);
    let Ok((_baseline_head, created_baseline_branch)) =
        ensure_pool_baseline_branch(&repo_root, can_refresh_pool_baseline)
    else {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile blocked / pool baseline could not be created",
                "pool baseline is unavailable during reconcile",
            ),
            "pool baseline is unavailable during reconcile".to_string(),
        )));
    };
    let Some(mut worktree_records) = load_worktree_records(&repo_root) else {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile failed / git worktree inventory could not be loaded",
                "worktree list inspection failed",
            ),
            "worktree list inspection failed".to_string(),
        )));
    };
    let reset_reusable_baseline_slots = reset_reusable_detached_baseline_slots(
        &repo_root,
        &pool_root,
        &worktree_records,
        &runtime_projection.slot_leases,
    );
    if reset_reusable_baseline_slots > 0
        && let Some(refreshed_records) = load_worktree_records(&repo_root)
    {
        worktree_records = refreshed_records;
    }

    let provisioned_slots = provision_missing_slots(&repo_root, &pool_root, &worktree_records);
    let Some(reloaded_worktree_records) = load_worktree_records(&repo_root) else {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile failed / git worktree inventory could not be reloaded",
                "worktree list reload failed",
            ),
            "worktree list reload failed".to_string(),
        )));
    };
    let cleaned_slots = cleanup_reusable_slots(
        planning_authority,
        &repo_root,
        &pool_root,
        &reloaded_worktree_records,
    );

    let Ok(context) =
        load_pool_runtime_context_from_roots(planning_authority, &repo_root, &canonical_repo_root)
    else {
        return Err(Box::new((
            build_blocked_pool_board(
                planning_authority,
                workspace_dir,
                "reconcile failed / pool runtime state could not be loaded",
                "pool runtime load failed",
            ),
            "pool runtime load failed".to_string(),
        )));
    };

    let pool = build_pool_board_from_context(
        &context,
        summarize_pool_reconcile_status(
            &build_pool_slots(&context),
            &context.pool_root,
            Some(PoolReconcileExecution {
                created_baseline_branch,
                created_pool_root,
                provisioned_slots,
                cleaned_slots,
            }),
        ),
    );

    Ok((context, pool))
}

fn inspect_pool_board(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> ParallelModePoolBoardSnapshot {
    match inspect_pool_board_and_context(planning_authority, workspace_dir) {
        Ok((_, pool)) => pool,
        Err(error) => {
            let (pool, _) = *error;
            pool
        }
    }
}

pub(super) fn inspect_pool_board_and_context(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> PoolBoardWithContextResult {
    match load_pool_runtime_context(planning_authority, workspace_dir) {
        Ok(context) => {
            let pool = build_pool_board_from_context(
                &context,
                summarize_pool_reconcile_status(
                    &build_pool_slots(&context),
                    &context.pool_root,
                    None,
                ),
            );
            Ok((context, pool))
        }
        Err((reconcile_status, detail)) => Err(Box::new((
            build_blocked_pool_board(planning_authority, workspace_dir, reconcile_status, detail),
            detail.to_string(),
        ))),
    }
}

pub(super) fn build_pool_slots(context: &PoolRuntimeContext) -> Vec<ParallelModePoolSlotSnapshot> {
    build_pool_slots_from_context(context)
}

fn can_refresh_pool_baseline_from_workspace(
    repo_root: &str,
    runtime_projection: &PlanningAuthorityRuntimeProjectionSnapshot,
) -> bool {
    runtime_projection.distributor_queue_records.is_empty()
        && runtime_projection.slot_leases.values().all(|lease| {
            matches!(
                lease.state,
                ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running
            )
        })
        && current_branch_name(Path::new(repo_root)).is_some_and(|branch_name| {
            branch_name != POOL_BASELINE_BRANCH
                && !branch_name.starts_with(&format!("{AKRA_AGENT_BRANCH_PREFIX}/"))
        })
}

fn ensure_pool_baseline_branch(
    repo_root: &str,
    reset_to_current_head: bool,
) -> Result<(String, bool), ()> {
    if reset_to_current_head && let Some(head_sha) = resolve_branch_head(repo_root, "HEAD") {
        let existed = resolve_branch_head(repo_root, POOL_BASELINE_BRANCH).is_some();
        if command_succeeds(
            "git",
            [
                "-C",
                repo_root,
                "branch",
                "-f",
                POOL_BASELINE_BRANCH,
                "HEAD",
            ],
        ) {
            return Ok((head_sha, !existed));
        }
    }

    if let Some(baseline_head) = resolve_branch_head(repo_root, POOL_BASELINE_BRANCH) {
        return Ok((baseline_head, false));
    }

    let remote_ref = format!("refs/remotes/origin/{POOL_BASELINE_BRANCH}");
    let created = if command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "show-ref",
            "--verify",
            "--quiet",
            remote_ref.as_str(),
        ],
    ) {
        command_succeeds(
            "git",
            [
                "-C",
                repo_root,
                "branch",
                POOL_BASELINE_BRANCH,
                remote_ref.as_str(),
            ],
        )
    } else if command_succeeds("git", ["-C", repo_root, "rev-parse", "--verify", "HEAD"]) {
        command_succeeds(
            "git",
            ["-C", repo_root, "branch", POOL_BASELINE_BRANCH, "HEAD"],
        )
    } else {
        false
    };

    if !created {
        return Err(());
    }

    resolve_branch_head(repo_root, POOL_BASELINE_BRANCH)
        .map(|baseline_head| (baseline_head, true))
        .ok_or(())
}
pub(super) fn load_pool_runtime_context(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> Result<PoolRuntimeContext, (&'static str, &'static str)> {
    let Some(repo_root) = detect_git_repo_root(workspace_dir) else {
        return Err((
            "reconcile failed / git repository is unavailable",
            "repository inspection failed",
        ));
    };
    let Some(canonical_repo_root) = detect_canonical_repo_root(planning_authority, workspace_dir)
    else {
        return Err((
            "reconcile failed / canonical repository root is unavailable",
            "canonical root inspection failed",
        ));
    };

    load_pool_runtime_context_from_roots(planning_authority, &repo_root, &canonical_repo_root)
        .map_err(|detail| {
            (
                "reconcile failed / pool runtime state could not be loaded",
                detail,
            )
        })
}

pub(super) fn resolve_workspace_slot_lease(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> Result<Option<WorkspaceSlotLeaseResolution>, String> {
    let context = load_pool_runtime_context(planning_authority, workspace_dir)
        .map_err(|(_, detail)| detail.to_string())?;
    let workspace_path = canonicalize_best_effort(Path::new(&context.repo_root));
    let Some(current_branch) = current_branch_name(&workspace_path) else {
        return Err(format!(
            "workspace `{}` does not currently resolve to a branch",
            workspace_path.display()
        ));
    };

    let mut matching_leases = context
        .slot_leases
        .values()
        .filter(|lease| worktree_paths_match(&workspace_path, Path::new(&lease.worktree_path)))
        .cloned()
        .collect::<Vec<_>>();

    if matching_leases.is_empty() {
        return Ok(None);
    }
    if matching_leases.len() > 1 {
        return Err(format!(
            "workspace `{}` matched multiple slot leases",
            workspace_path.display()
        ));
    }

    let lease = matching_leases
        .pop()
        .expect("matching lease count should be one");
    if lease.branch_name != current_branch {
        return Err(format!(
            "workspace `{}` is on `{}` but slot lease expects `{}`",
            workspace_path.display(),
            current_branch,
            lease.branch_name
        ));
    }

    Ok(Some(WorkspaceSlotLeaseResolution {
        context,
        lease,
        workspace_path,
    }))
}

fn load_pool_runtime_context_from_roots(
    planning_authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    canonical_repo_root: &Path,
) -> Result<PoolRuntimeContext, &'static str> {
    let Some(baseline_head) = resolve_pool_baseline_head(repo_root) else {
        return Err("pool baseline is unavailable during inspection");
    };
    let Some(worktree_records) = load_worktree_records(repo_root) else {
        return Err("worktree list inspection failed");
    };
    let pool_root = derive_default_pool_root(canonical_repo_root);
    let runtime_projections = load_runtime_projection_snapshot(planning_authority, repo_root);

    Ok(PoolRuntimeContext {
        repo_root: repo_root.to_string(),
        canonical_repo_root: canonical_repo_root.to_path_buf(),
        pool_root,
        baseline_head,
        worktree_records,
        slot_leases: runtime_projections.slot_leases,
        invalid_slot_leases: runtime_projections.invalid_slot_leases,
        session_details: runtime_projections.session_details,
        distributor_queue_records: runtime_projections.distributor_queue_records,
    })
}

fn load_runtime_projection_snapshot(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> PlanningAuthorityRuntimeProjectionSnapshot {
    planning_authority
        .load_runtime_projections(workspace_dir)
        .unwrap_or_default()
}

fn load_worktree_records(repo_root: &str) -> Option<Vec<GitWorktreeRecord>> {
    let worktree_output = run_command(
        "git",
        ["-C", repo_root, "worktree", "list", "--porcelain"],
        None,
    )?;
    Some(parse_worktree_records(&worktree_output))
}

fn provision_missing_slots(
    repo_root: &str,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
) -> usize {
    let mut provisioned_slots = 0;

    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_path = pool_root.join(slot_id(slot_number));
        if worktree_records
            .iter()
            .any(|record| record.path == slot_path)
            || slot_path.exists()
        {
            continue;
        }

        let Some(slot_parent) = slot_path.parent() else {
            continue;
        };
        if ensure_directory_exists(slot_parent).is_err() {
            continue;
        }

        let slot_path_string = slot_path.display().to_string();
        if command_succeeds(
            "git",
            [
                "-C",
                repo_root,
                "worktree",
                "add",
                "--detach",
                slot_path_string.as_str(),
                POOL_BASELINE_BRANCH,
            ],
        ) {
            provisioned_slots += 1;
        }
    }

    provisioned_slots
}

fn reset_reusable_detached_baseline_slots(
    repo_root: &str,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
    slot_leases: &BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
) -> usize {
    let baseline_head = resolve_pool_baseline_head(repo_root).unwrap_or_default();
    if baseline_head.is_empty() {
        return 0;
    }

    let mut reset_slots = 0;
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_id = slot_id(slot_number);
        if slot_leases.contains_key(&slot_id) {
            continue;
        }
        let slot_path = pool_root.join(&slot_id);
        let Some(worktree_record) = worktree_records
            .iter()
            .find(|record| record.path == slot_path)
        else {
            continue;
        };
        if !worktree_record.detached {
            continue;
        }
        let slot_status = inspect_slot_git_status(&slot_path);
        if worktree_record.head_sha == baseline_head
            && slot_status.is_some_and(SlotGitStatus::is_clean_baseline)
        {
            continue;
        }
        if reset_slot_worktree_to_akra(&slot_path).succeeded() {
            reset_slots += 1;
        }
    }

    reset_slots
}

fn cleanup_reusable_slots(
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

pub(super) fn branch_is_integrated_into_akra(repo_root: &str, branch_name: &str) -> bool {
    branch_is_integrated_into(repo_root, branch_name, POOL_BASELINE_BRANCH)
}

pub(super) fn branch_is_integrated_into(
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

pub(super) fn branch_is_cleanup_ready(repo_root: &str, branch_name: &str) -> bool {
    branch_is_integrated_into_akra(repo_root, branch_name)
}

pub(super) fn cleanup_slot(
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

pub(super) fn reset_slot_worktree_to_akra(
    slot_path: &Path,
) -> super::git_sequence::GitCommandSequenceReport {
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

pub(super) fn detect_canonical_repo_root(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> Option<PathBuf> {
    planning_authority
        .resolve_authority_location(workspace_dir)
        .ok()
        .map(|location| PathBuf::from(location.canonical_repo_root))
}

pub(super) fn slot_id(slot_number: usize) -> String {
    format!("slot-{slot_number}")
}

pub(super) fn short_sha(commit_sha: &str) -> String {
    commit_sha.chars().take(7).collect::<String>()
}

pub(super) fn resolve_workspace_head_sha(workspace_path: &Path) -> Option<String> {
    let workspace = workspace_path.display().to_string();
    run_command("git", ["-C", workspace.as_str(), "rev-parse", "HEAD"], None)
}

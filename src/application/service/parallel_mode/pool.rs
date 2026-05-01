use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityPort,
    PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModePoolBoardSnapshot,
    ParallelModePoolSlotCleanupDecision, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
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

const POOL_ALLOCATION_LOCK_DIR: &str = ".allocation-lock";
const POOL_ALLOCATION_LOCK_OWNER_FILE: &str = "owner";
const POOL_ALLOCATION_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const POOL_ALLOCATION_LOCK_RETRY: Duration = Duration::from_millis(25);
const POOL_ALLOCATION_LOCK_STALE_AFTER: Duration = Duration::from_secs(300);

pub(super) struct PoolAllocationLock {
    lock_path: PathBuf,
    owner_token: String,
}

impl Drop for PoolAllocationLock {
    fn drop(&mut self) {
        release_pool_allocation_lock(&self.lock_path, &self.owner_token);
    }
}

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

pub(super) fn acquire_pool_allocation_lock(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> Result<PoolAllocationLock, String> {
    let canonical_repo_root = detect_canonical_repo_root(planning_authority, workspace_dir)
        .ok_or_else(|| "canonical root inspection failed".to_string())?;
    let pool_root = derive_default_pool_root(&canonical_repo_root);
    ensure_directory_exists(&pool_root)
        .map_err(|error| format!("pool root creation failed before allocation lock: {error}"))?;
    acquire_pool_allocation_lock_at(&pool_root)
}

fn acquire_pool_allocation_lock_at(pool_root: &Path) -> Result<PoolAllocationLock, String> {
    let lock_path = pool_root.join(POOL_ALLOCATION_LOCK_DIR);
    let deadline = Instant::now() + POOL_ALLOCATION_LOCK_TIMEOUT;
    let owner_token = pool_allocation_lock_owner_token();

    loop {
        match fs::create_dir(&lock_path) {
            Ok(()) => {
                if let Err(error) = fs::write(
                    lock_path.join(POOL_ALLOCATION_LOCK_OWNER_FILE),
                    &owner_token,
                ) {
                    let _ = fs::remove_dir_all(&lock_path);
                    return Err(format!(
                        "pool allocation lock owner could not be written at `{}`: {error}",
                        lock_path.display()
                    ));
                }
                return Ok(PoolAllocationLock {
                    lock_path,
                    owner_token,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                remove_stale_pool_allocation_lock(&lock_path);
                if Instant::now() >= deadline {
                    return Err(format!(
                        "pool allocation lock is busy at `{}`",
                        lock_path.display()
                    ));
                }
                thread::sleep(POOL_ALLOCATION_LOCK_RETRY);
            }
            Err(error) => {
                return Err(format!(
                    "pool allocation lock could not be acquired at `{}`: {error}",
                    lock_path.display()
                ));
            }
        }
    }
}

fn pool_allocation_lock_owner_token() -> String {
    let created_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("pid={}\ncreated_at_ms={created_at}\n", std::process::id())
}

fn release_pool_allocation_lock(lock_path: &Path, owner_token: &str) {
    let owner_path = lock_path.join(POOL_ALLOCATION_LOCK_OWNER_FILE);
    let Ok(current_owner) = fs::read_to_string(&owner_path) else {
        return;
    };
    if current_owner == owner_token {
        let _ = fs::remove_dir_all(lock_path);
    }
}

fn remove_stale_pool_allocation_lock(lock_path: &Path) {
    let Ok(metadata) = fs::metadata(lock_path) else {
        return;
    };
    let Ok(modified_at) = metadata.modified() else {
        return;
    };
    let Ok(age) = SystemTime::now().duration_since(modified_at) else {
        return;
    };
    if age >= POOL_ALLOCATION_LOCK_STALE_AFTER {
        let owner_path = lock_path.join(POOL_ALLOCATION_LOCK_OWNER_FILE);
        if !matches!(
            fs::read_to_string(owner_path)
                .ok()
                .and_then(|owner| pool_allocation_lock_owner_pid(&owner))
                .map(pool_allocation_lock_owner_liveness),
            None | Some(PoolAllocationLockOwnerLiveness::Dead)
        ) {
            return;
        }
        let _ = fs::remove_dir_all(lock_path);
    }
}

fn pool_allocation_lock_owner_pid(owner_token: &str) -> Option<u32> {
    owner_token
        .lines()
        .find_map(|line| line.strip_prefix("pid=")?.parse::<u32>().ok())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PoolAllocationLockOwnerLiveness {
    Alive,
    Dead,
    Unknown,
}

fn pool_allocation_lock_owner_liveness(pid: u32) -> PoolAllocationLockOwnerLiveness {
    platform_process_liveness(pid)
}

#[cfg(unix)]
fn platform_process_liveness(pid: u32) -> PoolAllocationLockOwnerLiveness {
    match std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
    {
        Ok(status) if status.success() => PoolAllocationLockOwnerLiveness::Alive,
        Ok(_) => PoolAllocationLockOwnerLiveness::Dead,
        Err(_) => PoolAllocationLockOwnerLiveness::Unknown,
    }
}

#[cfg(windows)]
fn platform_process_liveness(pid: u32) -> PoolAllocationLockOwnerLiveness {
    let filter = format!("PID eq {pid}");
    match std::process::Command::new("tasklist")
        .args(["/FI", filter.as_str(), "/NH"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout
                .split_whitespace()
                .any(|field| field.trim() == pid.to_string())
            {
                PoolAllocationLockOwnerLiveness::Alive
            } else {
                PoolAllocationLockOwnerLiveness::Dead
            }
        }
        Ok(_) => PoolAllocationLockOwnerLiveness::Dead,
        Err(_) => PoolAllocationLockOwnerLiveness::Unknown,
    }
}

#[cfg(not(any(unix, windows)))]
fn platform_process_liveness(_pid: u32) -> PoolAllocationLockOwnerLiveness {
    PoolAllocationLockOwnerLiveness::Unknown
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

fn build_pool_board_from_context(
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

fn build_unavailable_pool_board(
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

fn build_blocked_pool_board(
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

fn inspect_pool_slot(context: &PoolRuntimeContext, slot_id: &str) -> ParallelModePoolSlotSnapshot {
    let slot_path = context.pool_root.join(slot_id);
    let base_worktree_label = display_pool_path(&context.canonical_repo_root, &slot_path);
    let slot_lease = context.slot_leases.get(slot_id);

    if context.invalid_slot_leases.contains(slot_id) {
        return ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Blocked,
            "unknown",
            annotate_worktree_label(base_worktree_label, "invalid lease metadata"),
            "operator recovery",
        );
    }

    let Some(worktree_record) = context
        .worktree_records
        .iter()
        .find(|record| record.path == slot_path)
    else {
        if let Some(slot_lease) = slot_lease {
            return ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                slot_lease.branch_name.clone(),
                annotate_worktree_label(
                    base_worktree_label,
                    "lease exists but worktree is missing",
                ),
                slot_lease.owner_label(),
            );
        }
        if slot_path.exists() {
            return ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                "unknown",
                annotate_worktree_label(
                    base_worktree_label,
                    "directory exists outside git worktree inventory",
                ),
                "operator recovery",
            );
        }

        return ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Missing,
            POOL_BASELINE_BRANCH,
            base_worktree_label,
            "reconcile pending",
        );
    };

    let Some(slot_status) = inspect_slot_git_status(&slot_path) else {
        return ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Blocked,
            slot_lease
                .map(|lease| lease.branch_name.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            annotate_worktree_label(base_worktree_label, "git status inspection failed"),
            slot_lease
                .map(ParallelModeSlotLeaseSnapshot::owner_label)
                .unwrap_or_else(|| "operator recovery".to_string()),
        );
    };

    if worktree_record.branch_name.as_deref() == Some(POOL_BASELINE_BRANCH)
        || (worktree_record.detached && worktree_record.head_sha == context.baseline_head)
    {
        let branch_label = if worktree_record.detached {
            format!("{POOL_BASELINE_BRANCH} (detached)")
        } else {
            POOL_BASELINE_BRANCH.to_string()
        };

        if let Some(slot_lease) = slot_lease {
            return ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                branch_label,
                annotate_worktree_label(base_worktree_label, "lease exists on idle baseline"),
                slot_lease.owner_label(),
            );
        }

        return if slot_status.is_clean_baseline() {
            ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Idle,
                branch_label,
                base_worktree_label,
                "idle baseline",
            )
        } else {
            ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                branch_label,
                annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                "operator recovery",
            )
        };
    }

    if let Some(branch_name) = worktree_record.branch_name.as_deref() {
        let expected_agent_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
        if branch_name.starts_with(&expected_agent_prefix) {
            if slot_status.has_pending_operation {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                    slot_lease
                        .map(ParallelModeSlotLeaseSnapshot::owner_label)
                        .unwrap_or_else(|| "operator recovery".to_string()),
                );
            }
            let worktree_clean = slot_status.is_clean_baseline();
            let cleanup_ready = slot_lease.is_none()
                && ParallelModePoolSlotCleanupDecision::new(
                    None,
                    worktree_clean,
                    worktree_clean && branch_is_cleanup_ready(&context.repo_root, branch_name),
                )
                .is_cleanup_ready();
            if cleanup_ready {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::AwaitingCleanup,
                    branch_name,
                    annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                    slot_lease
                        .map(ParallelModeSlotLeaseSnapshot::owner_label)
                        .unwrap_or_else(|| "cleanup pending".to_string()),
                );
            }

            let Some(slot_lease) = slot_lease else {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(
                        base_worktree_label,
                        &orphan_agent_branch_without_lease_detail(
                            &context.repo_root,
                            branch_name,
                            slot_status,
                        ),
                    ),
                    "operator recovery",
                );
            };
            if slot_lease.branch_name != branch_name {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(
                        base_worktree_label,
                        "lease branch does not match worktree branch",
                    ),
                    slot_lease.owner_label(),
                );
            }
            if slot_lease.worktree_path != slot_path.display().to_string() {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(
                        base_worktree_label,
                        "lease worktree path does not match slot path",
                    ),
                    slot_lease.owner_label(),
                );
            }

            return ParallelModePoolSlotSnapshot::from_lease(
                slot_id,
                branch_name,
                annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                slot_lease,
            );
        }

        let detail = if branch_name.starts_with(&format!("{AKRA_AGENT_BRANCH_PREFIX}/")) {
            "agent branch belongs to a different slot"
        } else {
            "unexpected branch for pool slot"
        };

        return ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Blocked,
            branch_name,
            annotate_worktree_label(base_worktree_label, detail),
            slot_lease
                .map(ParallelModeSlotLeaseSnapshot::owner_label)
                .unwrap_or_else(|| "operator recovery".to_string()),
        );
    }

    let detached_label = format!("detached@{}", short_sha(&worktree_record.head_sha));
    ParallelModePoolSlotSnapshot::new(
        slot_id,
        ParallelModePoolSlotState::Blocked,
        detached_label,
        annotate_worktree_label(
            base_worktree_label,
            &format!("detached away from `{POOL_BASELINE_BRANCH}` baseline"),
        ),
        slot_lease
            .map(ParallelModeSlotLeaseSnapshot::owner_label)
            .unwrap_or_else(|| "operator recovery".to_string()),
    )
}

fn summarize_pool_reconcile_status(
    slots: &[ParallelModePoolSlotSnapshot],
    pool_root: &Path,
    execution: Option<PoolReconcileExecution>,
) -> String {
    let idle_slots = slots
        .iter()
        .filter(|slot| slot.state == ParallelModePoolSlotState::Idle)
        .count();
    let awaiting_cleanup_slots = slots
        .iter()
        .filter(|slot| slot.state == ParallelModePoolSlotState::AwaitingCleanup)
        .count();
    let blocked_slots = slots
        .iter()
        .filter(|slot| slot.state == ParallelModePoolSlotState::Blocked)
        .count();
    let missing_slots = slots
        .iter()
        .filter(|slot| slot.state == ParallelModePoolSlotState::Missing)
        .count();
    let mut prefix = String::new();
    if let Some(execution) = execution.filter(|execution| execution.has_actions()) {
        let mut action_parts = Vec::new();
        if execution.created_baseline_branch {
            action_parts.push(format!("created `{POOL_BASELINE_BRANCH}`"));
        }
        if execution.created_pool_root {
            action_parts.push("created pool root".to_string());
        }
        if execution.provisioned_slots > 0 {
            action_parts.push(format!("provisioned {}", execution.provisioned_slots));
        }
        if execution.cleaned_slots > 0 {
            action_parts.push(format!("cleaned {}", execution.cleaned_slots));
        }
        prefix = format!("actions: {} / ", action_parts.join(", "));
    }

    if blocked_slots > 0 {
        if let Some(slot) = find_non_merged_orphan_slot_branch(slots) {
            return format!(
                "{}reconcile blocked / cause: {} / blocked: {blocked_slots} / missing: {missing_slots} / cleanup: {awaiting_cleanup_slots} / root {}",
                prefix,
                non_merged_orphan_slot_branch_notice(&slot.slot_id, &slot.branch_name),
                pool_root.display()
            );
        }
        return format!(
            "{}reconcile blocked / blocked: {blocked_slots} / missing: {missing_slots} / cleanup: {awaiting_cleanup_slots} / root {}",
            prefix,
            pool_root.display()
        );
    }

    if missing_slots > 0 && awaiting_cleanup_slots > 0 {
        return format!(
            "{}reconcile pending / missing: {missing_slots} / cleanup pending: {awaiting_cleanup_slots} / root {}",
            prefix,
            pool_root.display()
        );
    }

    if missing_slots > 0 {
        return format!(
            "{}reconcile pending / create {missing_slots} missing slot(s) under {}",
            prefix,
            pool_root.display()
        );
    }

    if awaiting_cleanup_slots > 0 {
        return format!(
            "{}cleanup pending / {awaiting_cleanup_slots} slot(s) still need reset to `{POOL_BASELINE_BRANCH}`",
            prefix
        );
    }

    if idle_slots == slots.len() && !slots.is_empty() {
        return format!(
            "{}reconcile complete / all slots are clean on `{POOL_BASELINE_BRANCH}` baseline",
            prefix
        );
    }

    format!(
        "{}reconcile complete / pool root {}",
        prefix,
        pool_root.display()
    )
}

fn orphan_agent_branch_without_lease_detail(
    repo_root: &str,
    branch_name: &str,
    slot_status: SlotGitStatus,
) -> String {
    let mut parts = Vec::new();
    if branch_is_cleanup_ready(repo_root, branch_name) {
        parts.push("cleanup-ready agent branch has no lease metadata".to_string());
    } else {
        parts.push(NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL.to_string());
    }
    if !slot_status.is_clean_baseline() {
        parts.push(slot_status.detail_label());
    }

    parts.join(" / ")
}

fn find_non_merged_orphan_slot_branch(
    slots: &[ParallelModePoolSlotSnapshot],
) -> Option<&ParallelModePoolSlotSnapshot> {
    slots.iter().find(|slot| {
        slot.state == ParallelModePoolSlotState::Blocked
            && slot.owner_label == "operator recovery"
            && slot
                .worktree_label
                .contains(NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL)
    })
}

pub(super) fn pool_operator_recovery_notice(
    pool: &ParallelModePoolBoardSnapshot,
) -> Option<String> {
    let slot = find_non_merged_orphan_slot_branch(&pool.slots)?;
    Some(format!(
        "pool: blocked / cause: {}",
        non_merged_orphan_slot_branch_notice(&slot.slot_id, &slot.branch_name)
    ))
}

fn non_merged_orphan_slot_branch_notice(slot_id: &str, branch_name: &str) -> String {
    format!(
        "{slot_id} branch `{branch_name}` is not integrated into `{POOL_BASELINE_BRANCH}` and has no lease metadata / next action: {NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_NEXT_ACTION}"
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

pub(super) fn derive_default_pool_root(canonical_repo_root: &Path) -> PathBuf {
    let repo_name = canonical_repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    let parent_dir = canonical_repo_root.parent().unwrap_or(canonical_repo_root);

    parent_dir
        .join(format!("{repo_name}-akra-worktrees"))
        .join(stable_short_hash(&canonical_repo_root.to_string_lossy()))
        .join("akra-pool")
}

fn stable_short_hash(value: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..12].to_string()
}

fn resolve_pool_baseline_head(repo_root: &str) -> Option<String> {
    resolve_branch_head(repo_root, POOL_BASELINE_BRANCH)
        .or_else(|| {
            resolve_branch_head(
                repo_root,
                &format!("refs/remotes/origin/{POOL_BASELINE_BRANCH}"),
            )
        })
        .or_else(|| {
            run_command(
                "git",
                ["-C", repo_root, "rev-parse", "--verify", "HEAD"],
                None,
            )
        })
}

fn resolve_branch_head(repo_root: &str, branch_name: &str) -> Option<String> {
    run_command("git", ["-C", repo_root, "rev-parse", branch_name], None)
}

fn parse_worktree_records(output: &str) -> Vec<GitWorktreeRecord> {
    #[derive(Default)]
    struct Builder {
        path: Option<PathBuf>,
        head_sha: Option<String>,
        branch_name: Option<String>,
        detached: bool,
    }

    impl Builder {
        fn build(self) -> Option<GitWorktreeRecord> {
            Some(GitWorktreeRecord {
                path: self.path?,
                head_sha: self.head_sha.unwrap_or_default(),
                branch_name: self.branch_name,
                detached: self.detached,
            })
        }
    }

    let mut records = Vec::new();
    let mut current = Builder::default();

    for line in output.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let Some(record) = std::mem::take(&mut current).build() {
                records.push(record);
            }
            continue;
        }

        if let Some(path) = line.strip_prefix("worktree ") {
            current.path = Some(PathBuf::from(path));
            continue;
        }
        if let Some(head_sha) = line.strip_prefix("HEAD ") {
            current.head_sha = Some(head_sha.to_string());
            continue;
        }
        if let Some(branch_name) = line.strip_prefix("branch refs/heads/") {
            current.branch_name = Some(branch_name.to_string());
            continue;
        }
        if line == "detached" {
            current.detached = true;
        }
    }

    records
}

pub(super) fn inspect_slot_git_status(slot_path: &Path) -> Option<SlotGitStatus> {
    let slot_path_string = slot_path.display().to_string();
    let status_output = run_command(
        "git",
        [
            "-C",
            slot_path_string.as_str(),
            "status",
            "--porcelain=v1",
            "--branch",
            "--untracked-files=all",
        ],
        None,
    )?;

    let mut status = SlotGitStatus::default();
    for line in status_output.lines().skip(1) {
        if line.starts_with("??") {
            status.has_untracked = true;
            continue;
        }

        let x = line.chars().next().unwrap_or(' ');
        let y = line.chars().nth(1).unwrap_or(' ');
        if x != ' ' {
            status.has_staged = true;
        }
        if y != ' ' {
            status.has_unstaged = true;
        }
    }

    let git_dir = resolve_git_dir(slot_path)?;
    status.has_pending_operation = [
        "MERGE_HEAD",
        "REBASE_HEAD",
        "rebase-merge",
        "rebase-apply",
        "CHERRY_PICK_HEAD",
    ]
    .into_iter()
    .any(|path| git_dir.join(path).exists());

    Some(status)
}

fn resolve_git_dir(slot_path: &Path) -> Option<PathBuf> {
    let slot_path_string = slot_path.display().to_string();
    let git_dir = run_command(
        "git",
        ["-C", slot_path_string.as_str(), "rev-parse", "--git-dir"],
        None,
    )?;
    Some(absolutize_path(slot_path, Path::new(&git_dir)))
}

fn absolutize_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn display_pool_path(canonical_repo_root: &Path, path: &Path) -> String {
    let display_root = canonical_repo_root.parent().unwrap_or(canonical_repo_root);
    path.strip_prefix(display_root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn annotate_worktree_label(base_label: String, detail: &str) -> String {
    if detail.is_empty() || detail == "clean" {
        base_label
    } else {
        format!("{base_label} / {detail}")
    }
}

fn canonicalize_best_effort(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn worktree_paths_match(left: &Path, right: &Path) -> bool {
    canonicalize_best_effort(left) == canonicalize_best_effort(right)
}

pub(super) fn slot_id(slot_number: usize) -> String {
    format!("slot-{slot_number}")
}

pub(super) fn short_sha(commit_sha: &str) -> String {
    commit_sha.chars().take(7).collect::<String>()
}

fn slot_leases_root(pool_root: &Path) -> PathBuf {
    pool_root.join(".leases")
}

pub(super) fn slot_lease_file_path(pool_root: &Path, slot_id: &str) -> PathBuf {
    slot_leases_root(pool_root).join(format!("{slot_id}.json"))
}

pub(super) fn write_slot_lease(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<(), String> {
    planning_authority
        .upsert_runtime_slot_lease(workspace_dir, lease)
        .map_err(|error| format!("failed to store slot lease `{}`: {error}", lease.slot_id))?;

    let leases_root = slot_leases_root(pool_root);
    ensure_directory_exists(&leases_root)
        .map_err(|error| format!("failed to create lease directory: {error}"))?;
    let lease_path = slot_lease_file_path(pool_root, &lease.slot_id);
    let temp_path = lease_path.with_extension("tmp");
    let lease_body = serde_json::to_string_pretty(lease)
        .map_err(|error| format!("failed to serialize slot lease: {error}"))?;
    fs::write(&temp_path, lease_body).map_err(|error| {
        format!(
            "failed to write temporary slot lease `{}`: {error}",
            lease.slot_id
        )
    })?;
    fs::rename(&temp_path, &lease_path)
        .map_err(|error| format!("failed to persist slot lease `{}`: {error}", lease.slot_id))
}

fn remove_slot_lease(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    slot_id: &str,
) -> bool {
    if planning_authority
        .remove_runtime_slot_lease(workspace_dir, slot_id)
        .is_err()
    {
        return false;
    }
    let lease_path = slot_lease_file_path(pool_root, slot_id);
    !lease_path.exists() || fs::remove_file(lease_path).is_ok()
}

pub(super) fn resolve_workspace_head_sha(workspace_path: &Path) -> Option<String> {
    let workspace = workspace_path.display().to_string();
    run_command("git", ["-C", workspace.as_str(), "rev-parse", "HEAD"], None)
}

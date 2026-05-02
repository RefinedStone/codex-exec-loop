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
    ParallelModeReadinessSnapshot, ParallelModeSlotLeaseSnapshot,
};

use super::current_branch_name;
use super::readiness::{command_succeeds, detect_git_repo_root, run_command};
use super::{
    AKRA_AGENT_BRANCH_PREFIX, DEFAULT_POOL_SIZE, NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL,
    NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_NEXT_ACTION, POOL_BASELINE_BRANCH,
    ensure_directory_exists,
};

/*
pool 모듈은 병렬 실행의 filesystem-facing 경계다. public surface는 supervisor,
completion, orchestration이 쓰는 얇은 함수로 제한하고, worktree inventory, slot cleanup,
lease mirror, board projection은 하위 모듈로 나눠 git 조작과 화면 projection이 섞이지 않게 한다.
*/
mod allocation_lock;
mod board;
mod cleanup;
mod lease_store;
mod paths;
mod reconcile;
mod slot_inspection;

pub(super) use self::allocation_lock::acquire_pool_allocation_lock;
use self::board::{
    build_blocked_pool_board, build_pool_board_from_context,
    build_pool_slots as build_pool_slots_from_context, build_unavailable_pool_board,
};
use self::cleanup::cleanup_reusable_slots;
pub(super) use self::cleanup::{
    branch_is_cleanup_ready, branch_is_integrated_into, cleanup_slot, reset_slot_worktree_to_akra,
};
#[cfg(test)]
pub(super) use self::lease_store::slot_lease_file_path;
pub(super) use self::lease_store::{remove_slot_lease, write_slot_lease};
use self::paths::{
    annotate_worktree_label, canonicalize_best_effort, parse_worktree_records, resolve_branch_head,
    resolve_pool_baseline_head, worktree_paths_match,
};
pub(super) use self::paths::{derive_default_pool_root, inspect_slot_git_status};
use self::reconcile::{
    can_refresh_pool_baseline_from_workspace, ensure_pool_baseline_branch, provision_missing_slots,
    reset_reusable_detached_baseline_slots,
};
pub(super) use self::slot_inspection::pool_operator_recovery_notice;
use self::slot_inspection::summarize_pool_reconcile_status;

/*
Git worktree inventory는 git porcelain 출력에서 얻은 최소 read model이다. 이 타입은
slot path와 branch/head 상태만 담고, lease나 planning authority 상태와의 join은
`PoolRuntimeContext` 이후 단계에서 수행한다.
*/
#[derive(Debug, Clone, PartialEq, Eq)]
struct GitWorktreeRecord {
    path: PathBuf,
    head_sha: String,
    branch_name: Option<String>,
    detached: bool,
}

/*
SlotGitStatus는 자동 cleanup/reset 여부를 결정하는 safety gate다. integration worktree는
untracked 파일을 허용하지만 pool baseline slot은 untracked까지 없어야 재사용 가능하므로
`is_clean_baseline`과 `is_ready_for_integration`을 분리한다.
*/
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

/*
reconcile execution은 이번 tick이 실제로 filesystem을 바꿨는지 요약한다. board summary는
이 값을 통해 "단순 inspection"과 "slot 생성/정리까지 수행한 reconcile"을 구분한다.
*/
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

/*
PoolRuntimeContext는 pool 화면, distributor snapshot, slot lifecycle이 공유하는 단일
runtime projection이다. git worktree inventory와 planning authority projection을 한 번에
묶어 하위 projection 함수들이 각자 store와 git을 다시 읽지 않게 한다.
*/
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

/*
workspace slot lease resolution은 현재 process가 실행 중인 workspace를 lease 관점으로 되찾는
경로다. startup/turn cleanup은 이 결과로 "내 workspace가 실제 slot worktree인가"와
"branch가 lease와 일치하는가"를 함께 확인한다.
*/
#[derive(Debug, Clone)]
pub(super) struct WorkspaceSlotLeaseResolution {
    pub(super) context: PoolRuntimeContext,
    pub(super) lease: ParallelModeSlotLeaseSnapshot,
    pub(super) workspace_path: PathBuf,
}

/*
build_pool_board는 read-only board entrypoint다. readiness가 아직 없거나 막혀 있으면
filesystem reconcile을 실행하지 않고 unavailable board를 반환해 TUI refresh가 slot 상태를
바꾸지 않게 한다.
*/
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

/*
reconcile_pool_board는 사용자가 parallel mode를 켜거나 명시적으로 refresh할 때 호출되는
mutating path다. baseline branch 확보, pool root 생성, missing slot provision, reusable slot
cleanup을 수행한 뒤 같은 board projection으로 돌아온다.
*/
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
    /*
    baseline refresh는 active lease나 distributor queue가 없는 경우에만 허용된다. pool
    baseline이 움직이면 idle slot reset 기준도 함께 움직이므로, 진행 중인 작업이 있을 때는
    현재 baseline을 보존해야 한다.
    */
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
    /*
    detached baseline slot은 이미 lease가 없고 clean하면 재사용 가능한 slot이다. reset 후
    worktree inventory를 다시 읽어 provision 단계가 stale head/branch 정보를 보지 않게 한다.
    */
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

/*
inspect_pool_board_and_context는 filesystem을 고치지 않는 projection path다. 실패해도
사용자에게 보여 줄 blocked board를 함께 반환해 caller가 error string만으로 UI 상태를
재구성하지 않게 한다.
*/
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

/*
runtime context loading은 inspection과 reconciliation이 공유하는 read phase다. git root,
canonical authority root, pool baseline head, worktree list, authority projections를 같은
순서로 읽어 board/distributor/cleanup이 서로 다른 기준 시점을 쓰는 일을 줄인다.
*/
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

/*
workspace lease resolution은 path match만으로 끝내지 않고 현재 checked-out branch까지 검증한다.
slot worktree path가 맞더라도 사용자가 수동 checkout을 바꾼 상태면 turn cleanup이 잘못된
branch를 reset할 수 있기 때문이다.
*/
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

/*
authority projection load는 best-effort다. projection 파일이 아직 없거나 일부 mirror가
손상되어도 pool inspection은 git inventory를 보여 줄 수 있어야 하므로, store error는
empty projection으로 접고 이후 recovery notice가 구체 상태를 드러내게 한다.
*/
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

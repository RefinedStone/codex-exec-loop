use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeDelta, Utc};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityPort,
    PlanningAuthorityRuntimeEventRecord, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::diagnostics::raw_event_log;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModePoolBoardSnapshot,
    ParallelModePoolResetPolicy, ParallelModePoolResetReport, ParallelModePoolResetRunId,
    ParallelModePoolResetSlotAction, ParallelModePoolResetSlotOutcome,
    ParallelModePoolResetSlotReport, ParallelModePoolSlotCleanupDecision,
    ParallelModePoolSlotSnapshot, ParallelModeReadinessSnapshot, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState, ParallelModeTaskDispatchBlockSnapshot,
};

use super::current_branch_name;
use super::readiness::{command_succeeds, detect_git_repo_root, run_command};
use super::{
    AKRA_AGENT_BRANCH_PREFIX, DEFAULT_POOL_SIZE, DEFAULT_PUSH_REMOTE_NAME,
    NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL, NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_NEXT_ACTION,
    POOL_BASELINE_BRANCH, ensure_directory_exists, remote_tracking_branch_ref,
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
pub(super) use self::cleanup::{
    branch_is_cleanup_ready, branch_is_integrated_into, cleanup_slot, reset_slot_worktree_to_akra,
};
use self::cleanup::{cleanup_reusable_slots, cleanup_stale_leased_startup_slots};
#[cfg(test)]
pub(super) use self::lease_store::slot_lease_file_path;
#[cfg(not(test))]
use self::lease_store::slot_lease_file_path;
pub(super) use self::lease_store::{remove_slot_lease, write_slot_lease};
use self::paths::{
    annotate_worktree_label, canonicalize_best_effort, parse_worktree_records, resolve_branch_head,
    resolve_pool_baseline_head, worktree_paths_match,
};
pub(super) use self::paths::{derive_default_pool_root, inspect_slot_git_status};
use self::reconcile::{
    ensure_pool_baseline_branch, provision_missing_slots, reset_reusable_detached_baseline_slots,
};
pub(super) use self::slot_inspection::pool_operator_recovery_notice;
use self::slot_inspection::summarize_pool_reconcile_status;
use super::session_detail::agent_session_detail_record_path;

const RECENT_LEASE_PROTECTION_SECS: i64 = 120;

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
    pub(super) task_dispatch_blocks: Vec<ParallelModeTaskDispatchBlockSnapshot>,
    pub(super) distributor_queue_records: Vec<PlanningAuthorityDistributorQueueRecord>,
    pub(super) runtime_events: Vec<PlanningAuthorityRuntimeEventRecord>,
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

pub(super) fn reset_pool_for_parallel_enable(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> Result<ParallelModePoolResetReport, String> {
    let Some(repo_root) = detect_git_repo_root(workspace_dir) else {
        return Err("git repository is unavailable".to_string());
    };
    let Some(canonical_repo_root) = detect_canonical_repo_root(planning_authority, workspace_dir)
    else {
        return Err("canonical repository root is unavailable".to_string());
    };
    let pool_root = derive_default_pool_root(&canonical_repo_root);
    ensure_directory_exists(&pool_root)
        .map_err(|error| format!("pool root could not be created: {error}"))?;
    ensure_pool_baseline_branch(&repo_root)
        .map_err(|_| "pool baseline could not be created".to_string())?;
    let context =
        load_pool_runtime_context_from_roots(planning_authority, &repo_root, &canonical_repo_root)
            .map_err(|detail| detail.to_string())?;

    let mut report = ParallelModePoolResetReport::new(
        ParallelModePoolResetRunId::new(format!("{}:{}", repo_root, Utc::now().to_rfc3339())),
        ParallelModePoolResetPolicy::ProtectLive,
    );
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_id = slot_id(slot_number);
        let slot_path = pool_root.join(&slot_id);
        let Some(_worktree_record) = context
            .worktree_records
            .iter()
            .find(|record| record.path == slot_path)
        else {
            report
                .slot_reports
                .push(ParallelModePoolResetSlotReport::new(
                    slot_id,
                    ParallelModePoolResetSlotAction::SkipMissing,
                    ParallelModePoolResetSlotOutcome::Skipped,
                    "slot worktree is not registered",
                ));
            continue;
        };

        if let Some(lease) = context.slot_leases.get(&slot_id)
            && live_lease_blocks_parallel_entry_reset(lease, &context.session_details)
        {
            report
                .slot_reports
                .push(ParallelModePoolResetSlotReport::new(
                    slot_id,
                    ParallelModePoolResetSlotAction::PreserveLive,
                    ParallelModePoolResetSlotOutcome::Blocked,
                    format!("live {} lease is protected", lease.state.label()),
                ));
        }
    }

    if report.has_live_blockers() {
        raw_event_log::emit_lazy("parallel_pool_reset_blocked", || {
            serde_json::json!({
                "workspace": workspace_dir,
                "repo_root": repo_root,
                "pool_root": pool_root,
                "run_id": report.run_id.as_str(),
                "policy": report.policy,
                "live_blockers": report.live_blocker_count(),
            })
        });
        return Ok(report);
    }

    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_id = slot_id(slot_number);
        let slot_path = pool_root.join(&slot_id);
        if !context
            .worktree_records
            .iter()
            .any(|record| record.path == slot_path)
        {
            continue;
        }

        raw_event_log::emit_lazy("parallel_pool_slot_reset_started", || {
            serde_json::json!({
                "workspace": workspace_dir,
                "repo_root": repo_root,
                "pool_root": pool_root,
                "slot_id": slot_id,
                "slot_path": slot_path,
                "baseline_branch": POOL_BASELINE_BRANCH,
            })
        });
        let reset_report = reset_slot_worktree_to_akra(&slot_path);
        if reset_report.succeeded() {
            collect_reset_projection_keys(&mut report, &context, &slot_id);
            report
                .slot_reports
                .push(ParallelModePoolResetSlotReport::new(
                    &slot_id,
                    ParallelModePoolResetSlotAction::Reset,
                    ParallelModePoolResetSlotOutcome::Succeeded,
                    "slot worktree reset to baseline",
                ));
            raw_event_log::emit_lazy("parallel_pool_slot_reset_completed", || {
                serde_json::json!({
                    "workspace": workspace_dir,
                    "repo_root": repo_root,
                    "pool_root": pool_root,
                    "slot_id": slot_id,
                    "slot_path": slot_path,
                    "baseline_branch": POOL_BASELINE_BRANCH,
                    "succeeded": true,
                })
            });
            continue;
        }

        let failure_summary = reset_report
            .failure_summary()
            .unwrap_or_else(|| "slot reset failed".to_string());
        report
            .slot_reports
            .push(ParallelModePoolResetSlotReport::new(
                &slot_id,
                ParallelModePoolResetSlotAction::Reset,
                ParallelModePoolResetSlotOutcome::Failed,
                failure_summary.clone(),
            ));
        raw_event_log::emit_lazy("parallel_pool_slot_reset_failed", || {
            serde_json::json!({
                "workspace": workspace_dir,
                "repo_root": repo_root,
                "pool_root": pool_root,
                "slot_id": slot_id,
                "slot_path": slot_path,
                "baseline_branch": POOL_BASELINE_BRANCH,
                "succeeded": false,
                "failure": failure_summary,
            })
        });
    }

    if report.succeeded_reset_slot_count() > 0 {
        planning_authority
            .apply_parallel_pool_reset_report(&repo_root, &report)
            .map_err(|error| format!("parallel runtime projection reset report failed: {error}"))?;
        clear_pool_runtime_mirrors_for_report(&pool_root, &report)?;
    }

    Ok(report)
}

fn live_lease_blocks_parallel_entry_reset(
    lease: &ParallelModeSlotLeaseSnapshot,
    session_details: &[ParallelModeAgentSessionDetailSnapshot],
) -> bool {
    match lease.state {
        ParallelModeSlotLeaseState::Running | ParallelModeSlotLeaseState::CleanupPending => true,
        ParallelModeSlotLeaseState::Leased => {
            !stale_unstarted_lease_can_be_reset(lease, session_details)
        }
    }
}

fn stale_unstarted_lease_can_be_reset(
    lease: &ParallelModeSlotLeaseSnapshot,
    session_details: &[ParallelModeAgentSessionDetailSnapshot],
) -> bool {
    if !leased_at_is_stale(&lease.leased_at) {
        return false;
    }
    session_details
        .iter()
        .find(|detail| detail.session_key == lease.session_key())
        .is_some_and(|detail| {
            detail.thread_id.is_none()
                && detail.state_label == "assigned"
                && detail.completion_state_label == "in_progress"
        })
}

fn leased_at_is_stale(leased_at: &str) -> bool {
    let Ok(timestamp) = DateTime::parse_from_rfc3339(leased_at) else {
        return false;
    };
    Utc::now().signed_duration_since(timestamp.with_timezone(&Utc))
        >= TimeDelta::seconds(RECENT_LEASE_PROTECTION_SECS)
}

fn collect_reset_projection_keys(
    report: &mut ParallelModePoolResetReport,
    context: &PoolRuntimeContext,
    slot_id: &str,
) {
    for detail in &context.session_details {
        if detail.slot_id == slot_id && !report.reset_session_keys.contains(&detail.session_key) {
            report.reset_session_keys.push(detail.session_key.clone());
        }
    }
    for queue_record in &context.distributor_queue_records {
        if queue_record.slot_id == slot_id
            && !report
                .reset_queue_item_ids
                .contains(&queue_record.queue_item_id)
        {
            report
                .reset_queue_item_ids
                .push(queue_record.queue_item_id.clone());
        }
    }
}

fn clear_pool_runtime_mirrors_for_report(
    pool_root: &Path,
    report: &ParallelModePoolResetReport,
) -> Result<(), String> {
    for slot_id in report.succeeded_reset_slot_ids() {
        let path = slot_lease_file_path(pool_root, &slot_id);
        if path.exists() {
            fs::remove_file(&path).map_err(|error| {
                format!(
                    "failed to remove reset lease mirror `{}`: {error}",
                    path.display()
                )
            })?;
        } else {
            continue;
        }
    }
    for session_key in &report.reset_session_keys {
        let path = agent_session_detail_record_path(pool_root, session_key);
        if path.exists() {
            fs::remove_file(&path).map_err(|error| {
                format!(
                    "failed to remove reset session mirror `{}`: {error}",
                    path.display()
                )
            })?;
        }
    }
    let queue_root = pool_root.join(".distributor-queue");
    for queue_item_id in &report.reset_queue_item_ids {
        let path = queue_root.join(format!("{queue_item_id}.json"));
        if path.exists() {
            fs::remove_file(&path).map_err(|error| {
                format!(
                    "failed to remove reset distributor mirror `{}`: {error}",
                    path.display()
                )
            })?;
        }
    }
    Ok(())
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
    /*
    pool root는 canonical repo sibling 아래에 둔다. 사용자가 slot worktree 안에서
    reconcile을 호출해도 pool 위치가 slot 기준으로 흔들리지 않아야 모든 lane이 같은
    slot inventory를 공유한다.
    */
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
    let mut runtime_projection = load_runtime_projection_snapshot(planning_authority, &repo_root);
    /*
    pool baseline은 표준 remote branch가 있으면 그 ref에서 갱신한다. fresh repository처럼
    local/remote 표준 branch가 모두 없으면 reconcile이 현재 workspace HEAD를 표준 branch로
    seed하고 push한 뒤 slot 출발점을 확정한다.
    */
    let Ok((_baseline_head, created_baseline_branch)) = ensure_pool_baseline_branch(&repo_root)
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
    let stale_startup_cleaned_slots = cleanup_stale_leased_startup_slots(
        planning_authority,
        &repo_root,
        &pool_root,
        &worktree_records,
        &runtime_projection.slot_leases,
        &runtime_projection.session_details,
    );
    if stale_startup_cleaned_slots > 0 {
        runtime_projection = load_runtime_projection_snapshot(planning_authority, &repo_root);
        if let Some(refreshed_records) = load_worktree_records(&repo_root) {
            worktree_records = refreshed_records;
        }
    }
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
    /*
    reset count 자체는 board summary에 직접 드러내지 않는다. reset된 slot은 곧 idle
    baseline으로 다시 관측되며, 사용자가 알아야 하는 action count는 아래 cleanup pass가
    반환하는 "실제로 slot을 돌려놓은 수"에 더 가깝다.
    */
    if reset_reusable_baseline_slots > 0
        && let Some(refreshed_records) = load_worktree_records(&repo_root)
    {
        worktree_records = refreshed_records;
    }
    let provisioned_slots = provision_missing_slots(
        &repo_root,
        &pool_root,
        &worktree_records,
        &runtime_projection.slot_leases,
    );
    /*
    provision 직후 worktree list를 다시 읽는다. 새 slot worktree가 생긴 뒤의 inventory로
    cleanup과 board projection을 돌려야 missing slot이 같은 reconcile tick 안에서
    계속 missing으로 보이는 일이 없다.
    */
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
    let cleaned_slots = stale_startup_cleaned_slots
        + cleanup_reusable_slots(
            planning_authority,
            &repo_root,
            &pool_root,
            &reloaded_worktree_records,
            &runtime_projection.slot_leases,
        );
    /*
    cleanup은 planning authority의 lease/session mirror를 바꿀 수 있으므로 context는
    cleanup 이후에 다시 로드한다. 이전 projection을 재사용하면 반환된 slot이 roster나
    detail에 남는 stale supervisor 상태가 된다.
    */
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
    let context = match load_pool_runtime_context(planning_authority, workspace_dir) {
        Ok(context) => context,
        Err((_, "pool baseline is unavailable during inspection")) => return Ok(None),
        Err((_, detail)) => return Err(detail.to_string()),
    };
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
    /*
    Path matching uses best-effort canonicalization because callers may be inside
    nested directories of a slot worktree. Branch matching below is the stricter
    guard that prevents a reused path with the wrong checkout from being treated
    as the lease owner.
    */
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
    let runtime_projections = load_runtime_projection_snapshot(
        planning_authority,
        canonical_repo_root.to_str().unwrap_or(repo_root),
    );

    /*
    Context stores the raw authority projections instead of immediately reducing
    them to board rows. Distributor, supervisor detail, and pool rendering each
    need a different join shape over the same leases, sessions, and queue records.
    */
    Ok(PoolRuntimeContext {
        repo_root: repo_root.to_string(),
        canonical_repo_root: canonical_repo_root.to_path_buf(),
        pool_root,
        baseline_head,
        worktree_records,
        slot_leases: runtime_projections.slot_leases,
        invalid_slot_leases: runtime_projections.invalid_slot_leases,
        session_details: runtime_projections.session_details,
        task_dispatch_blocks: runtime_projections.task_dispatch_blocks,
        distributor_queue_records: runtime_projections.distributor_queue_records,
        runtime_events: runtime_projections.runtime_events,
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
    /*
    `git worktree list --porcelain` is the inventory source for both reconcile
    and inspection. Keeping it as an Option lets callers choose their own blocked
    board copy instead of leaking command failures through a generic error.
    */
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

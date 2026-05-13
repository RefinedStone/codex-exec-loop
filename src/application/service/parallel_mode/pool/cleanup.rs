use std::{fs, path::Path, thread, time::Duration};

use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModePoolSlotCleanupDecision,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};
use chrono::{DateTime, TimeDelta, Utc};

use super::super::git_sequence::{GitCommandStep, run_git_sequence};
use super::super::readiness::command_succeeds;
use super::super::{
    branch_exists, record_cleaned_session_detail, record_failed_start_session_detail,
    record_stale_active_lease_released_session_detail,
};
use super::paths::resolve_git_dir;
use super::{
    AKRA_AGENT_BRANCH_PREFIX, DEFAULT_POOL_SIZE, GitWorktreeRecord, POOL_BASELINE_BRANCH,
    SlotGitStatus, inspect_slot_git_status, remove_slot_lease, slot_id,
};

const STALE_LEASED_SLOT_RELEASE_AFTER_SECS: i64 = 120;
#[cfg(not(test))]
const STALE_INDEX_LOCK_RELEASE_AFTER: Duration = Duration::from_secs(120);
#[cfg(test)]
const STALE_INDEX_LOCK_RELEASE_AFTER: Duration = Duration::from_secs(0);

/*
reusable slot cleanup은 reconcile 과정에서 "이제 pool baseline으로 되돌려도 되는" slot을
찾아 자동으로 정리하는 후처리 경로다. 대상은 slot 번호별 worktree inventory를 기준으로
찾고, agent branch prefix, lease state, worktree clean 여부, branch가 baseline에
통합되었다는 증거를 모두 만족해야 한다.

이 함수가 보수적인 이유는 slot worktree가 사용자의 미완성 변경이나 아직 통합되지 않은
agent branch를 품을 수 있기 때문이다. lease가 Leased/Running이면 건드리지 않고, 통합
증거가 없으면 cleanup하지 않으며, 실제 cleanup도 `cleanup_slot`의 단계별 성공 여부를 보고
count를 올린다.
*/
pub(super) fn cleanup_reusable_slots(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
    slot_leases: &std::collections::BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
) -> usize {
    let mut cleaned_slots = 0;

    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_id = slot_id(slot_number);
        let slot_path = pool_root.join(&slot_id);
        let Some(worktree_record) = worktree_records
            .iter()
            .find(|record| record.path == slot_path)
        else {
            // git worktree inventory에 없으면 cleanup보다 provisioning/inspection 경로가 먼저 다룬다.
            continue;
        };
        let Some(branch_name) = worktree_record.branch_name.as_deref() else {
            // detached slot은 agent branch가 아니므로 reusable baseline reset 경로의 책임이다.
            continue;
        };
        let expected_agent_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
        if !branch_name.starts_with(&expected_agent_prefix) {
            // 다른 slot의 agent branch나 사용자 branch를 현재 slot cleanup이 지우지 않게 prefix를 엄격히 맞춘다.
            continue;
        }
        let slot_lease = slot_leases.get(&slot_id);
        let lease_state = slot_lease.map(|lease| lease.state);
        // lease가 없을 때만 worktree cleanliness가 cleanup 근거가 된다. lease가 있으면 lease state가 우선이다.
        let worktree_clean = lease_state.is_none()
            && inspect_slot_git_status(&slot_path).is_some_and(SlotGitStatus::is_clean_baseline);
        // branch integration은 active lease가 아닌 경우에만 확인하며, cleanup pending은 명시적 승인 신호다.
        let branch_integrated = !matches!(
            lease_state,
            Some(ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running)
        ) && (matches!(
            lease_state,
            Some(ParallelModeSlotLeaseState::CleanupPending)
        ) || worktree_clean)
            && branch_is_cleanup_ready(repo_root, branch_name);
        // domain decision object가 lease/git/integration 조합의 최종 cleanup 가능 여부를 단일 규칙으로 판정한다.
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
            runtime,
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

pub(super) fn cleanup_stale_leased_startup_slots(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
    slot_leases: &std::collections::BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
    session_details: &[ParallelModeAgentSessionDetailSnapshot],
) -> usize {
    let mut cleaned_slots = 0;

    for lease in slot_leases.values() {
        if !stale_leased_startup_slot_can_be_released(lease, session_details) {
            continue;
        }
        let slot_path = pool_root.join(&lease.slot_id);
        let Some(worktree_record) = worktree_records
            .iter()
            .find(|record| record.path == slot_path)
        else {
            continue;
        };
        if worktree_record.branch_name.as_deref() != Some(lease.branch_name.as_str()) {
            continue;
        }
        let Some(slot_status) = inspect_slot_git_status(&slot_path) else {
            continue;
        };
        if !slot_status.is_clean_baseline() {
            continue;
        }
        if cleanup_slot(
            planning_authority,
            runtime,
            repo_root,
            pool_root,
            &lease.slot_id,
            &slot_path,
            &lease.branch_name,
        ) {
            let _ = record_failed_start_session_detail(
                planning_authority,
                runtime,
                repo_root,
                pool_root,
                lease,
            );
            cleaned_slots += 1;
        }
    }

    cleaned_slots
}

/*
clean baseline split-brain cleanup handles the state where the source-of-truth
lease still says Leased/Running/CleanupPending, but git has already returned
the slot worktree to the pool baseline. This can happen if cleanup deletes or
detaches the branch and then fails before removing the authority lease, or if a
late worker event observes a recycled worktree. A clean baseline with a missing
active agent branch has no remaining worktree result to preserve, and a
CleanupPending branch that is already integrated is safe to close. Other active
branch drift is intentionally left blocked for operator recovery.
*/
pub(super) fn cleanup_clean_baseline_split_brain_leases(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    pool_root: &Path,
    baseline_head: &str,
    worktree_records: &[GitWorktreeRecord],
    slot_leases: &std::collections::BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
) -> usize {
    let mut cleaned_slots = 0;

    for lease in slot_leases.values() {
        let slot_path = pool_root.join(&lease.slot_id);
        let Some(worktree_record) = worktree_records
            .iter()
            .find(|record| record.path == slot_path)
        else {
            continue;
        };
        if !worktree_is_clean_reusable_baseline(
            repo_root,
            worktree_record,
            baseline_head,
            &slot_path,
        ) {
            continue;
        }
        let branch_still_exists = branch_exists(repo_root, &lease.branch_name);
        if branch_still_exists && lease.state != ParallelModeSlotLeaseState::CleanupPending {
            continue;
        }
        if branch_still_exists
            && (!branch_is_cleanup_ready(repo_root, &lease.branch_name)
                || !delete_stale_agent_branch(repo_root, &lease.branch_name))
        {
            continue;
        }
        if !remove_slot_lease(
            planning_authority,
            runtime,
            repo_root,
            pool_root,
            &lease.slot_id,
        ) {
            continue;
        }
        if lease.state == ParallelModeSlotLeaseState::CleanupPending {
            let _ = record_cleaned_session_detail(
                planning_authority,
                runtime,
                repo_root,
                pool_root,
                lease,
            );
        } else {
            let _ = record_stale_active_lease_released_session_detail(
                planning_authority,
                runtime,
                repo_root,
                pool_root,
                lease,
                "stale active lease reconciled after slot worktree returned to clean baseline",
            );
        }
        cleaned_slots += 1;
    }

    cleaned_slots
}

fn worktree_is_clean_reusable_baseline(
    repo_root: &str,
    worktree_record: &GitWorktreeRecord,
    baseline_head: &str,
    slot_path: &Path,
) -> bool {
    if !inspect_slot_git_status(slot_path).is_some_and(SlotGitStatus::is_clean_baseline) {
        return false;
    }
    let branch_is_baseline = worktree_record.branch_name.as_deref() == Some(POOL_BASELINE_BRANCH);
    let detached_at_baseline =
        worktree_record.detached && worktree_record.head_sha == baseline_head;
    if branch_is_baseline || detached_at_baseline {
        return true;
    }

    worktree_record.detached
        && branch_is_integrated_into(repo_root, &worktree_record.head_sha, POOL_BASELINE_BRANCH)
}

fn delete_stale_agent_branch(repo_root: &str, branch_name: &str) -> bool {
    command_succeeds("git", ["-C", repo_root, "branch", "-D", branch_name])
}

fn stale_leased_startup_slot_can_be_released(
    lease: &ParallelModeSlotLeaseSnapshot,
    session_details: &[ParallelModeAgentSessionDetailSnapshot],
) -> bool {
    if lease.state != ParallelModeSlotLeaseState::Leased || !leased_at_is_stale(&lease.leased_at) {
        return false;
    }

    let Some(detail) = session_details
        .iter()
        .find(|detail| detail.session_key == lease.session_key())
    else {
        return false;
    };

    detail.thread_id.is_none()
        && detail.state_label == "assigned"
        && detail.completion_state_label == "in_progress"
}

fn leased_at_is_stale(leased_at: &str) -> bool {
    let Ok(timestamp) = DateTime::parse_from_rfc3339(leased_at) else {
        return false;
    };
    Utc::now().signed_duration_since(timestamp.with_timezone(&Utc))
        >= TimeDelta::seconds(STALE_LEASED_SLOT_RELEASE_AFTER_SECS)
}

fn branch_is_integrated_into_akra(repo_root: &str, branch_name: &str) -> bool {
    branch_is_integrated_into(repo_root, branch_name, POOL_BASELINE_BRANCH)
}

/*
cleanup readiness의 핵심 git 질문은 "agent branch의 변경이 pool baseline에 이미
포함되었는가"다. `merge-base --is-ancestor`는 branch tip이 base branch의 조상인지
확인하므로, true이면 branch를 지워도 baseline이 그 변경을 잃지 않는다는 뜻이다.
*/
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

/*
slot cleanup은 세 단계를 모두 성공해야 true를 반환한다. 먼저 slot worktree를 pool baseline
detached 상태로 reset/clean하고, repo에서 agent branch를 삭제하고, planning authority에
남은 lease metadata를 제거한다. 마지막으로 git status가 clean baseline인지 다시 확인해
실제 pool 재사용 가능 상태까지 검증한다.

중간 단계가 실패하면 false만 반환한다. 호출자는 이 false를 이용해 queue record를 Blocked로
남기거나 reconcile count를 올리지 않는다. 즉 cleanup 실패는 조용히 성공으로 간주되지 않고
supervisor가 계속 복구 대상으로 볼 수 있게 된다.
*/
pub(in crate::application::service::parallel_mode) fn cleanup_slot(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    pool_root: &Path,
    slot_id: &str,
    slot_path: &Path,
    branch_name: &str,
) -> bool {
    // worktree reset을 branch deletion보다 먼저 수행해 checkout 중인 branch를 안전하게 지울 수 있게 한다.
    let mut reset_report = reset_slot_worktree_to_akra(slot_path);
    for _ in 0..4 {
        if reset_report.succeeded() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
        reset_report = reset_slot_worktree_to_akra(slot_path);
    }
    if !reset_report.succeeded() {
        // failure summary는 디버깅용으로 계산하지만, 이 helper의 공개 계약은 성공 여부 bool이다.
        let _failure_summary = reset_report.failure_summary();
        return false;
    }
    // reset 뒤 agent branch를 삭제해 같은 slot slug 재사용 때 stale branch collision을 줄인다.
    if !delete_cleaned_slot_branch(repo_root, branch_name) {
        return false;
    }
    if !remove_slot_lease(planning_authority, runtime, repo_root, pool_root, slot_id) {
        return false;
    }

    // 마지막 git status 재검증은 metadata 제거 성공과 실제 worktree 재사용 가능 상태를 함께 확인한다.
    inspect_slot_git_status(slot_path).is_some_and(SlotGitStatus::is_clean_baseline)
}

fn delete_cleaned_slot_branch(repo_root: &str, branch_name: &str) -> bool {
    for attempt in 0..3 {
        let delete_branch = run_git_sequence(
            "delete cleaned slot branch",
            vec![GitCommandStep::new(
                "delete agent branch",
                ["-C", repo_root, "branch", "-D", branch_name],
            )],
        );
        if delete_branch.succeeded() {
            return true;
        }
        let _failure_summary = delete_branch.failure_summary();
        if attempt < 2 {
            thread::sleep(Duration::from_millis(100));
        }
    }
    false
}

/*
pool slot을 baseline으로 되돌리는 git sequence다. checkout detach, hard reset, clean 순서를
한 리포트로 묶어 호출자가 실패 단계를 확인할 수 있게 한다. branch를 직접 checkout하지 않고
detached baseline으로 두는 이유는 idle slot이 특정 작업 branch를 소유하지 않는 중립
상태여야 다음 lease가 새 agent branch를 안전하게 만들 수 있기 때문이다.
*/
pub(in crate::application::service::parallel_mode) fn reset_slot_worktree_to_akra(
    slot_path: &Path,
) -> super::super::git_sequence::GitCommandSequenceReport {
    remove_stale_slot_index_lock(slot_path);
    // git sequence API는 argv 조각을 문자열로 받으므로 Path 변환은 sequence 조립 직전에만 수행한다.
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
                    "--force",
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

fn remove_stale_slot_index_lock(slot_path: &Path) {
    let Some(git_dir) = resolve_git_dir(slot_path) else {
        return;
    };
    let index_lock_path = git_dir.join("index.lock");
    let Ok(metadata) = fs::symlink_metadata(&index_lock_path) else {
        return;
    };
    let Ok(modified_at) = metadata.modified() else {
        return;
    };
    let Ok(lock_age) = modified_at.elapsed() else {
        return;
    };
    if lock_age < STALE_INDEX_LOCK_RELEASE_AFTER {
        return;
    }
    let _ = fs::remove_file(index_lock_path);
}

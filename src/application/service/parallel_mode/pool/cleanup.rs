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

/*
학습 주석: reusable slot cleanup은 reconcile 과정에서 "이제 pool baseline으로 되돌려도 되는"
slot을 찾아 자동으로 정리합니다. 대상은 slot 번호별 worktree inventory를 기준으로 찾고,
agent branch prefix, lease state, worktree clean 여부, branch가 baseline에 통합되었는지를
모두 만족해야 합니다.

이 함수가 conservative한 이유는 slot worktree가 사용자의 미완성 변경이나 아직 통합되지
않은 agent branch를 품을 수 있기 때문입니다. lease가 Leased/Running이면 건드리지 않고,
통합 증거가 없으면 cleanup하지 않으며, 실제 cleanup도 `cleanup_slot`의 단계별 성공 여부를
보고 count를 올립니다.
*/
pub(super) fn cleanup_reusable_slots(
    planning_authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
) -> usize {
    let mut cleaned_slots = 0;
    // 학습 주석: lease state는 planning authority가 가진 runtime projection을 기준으로 보며, git inventory만으로 판단하지 않습니다.
    let slot_leases = load_runtime_projection_snapshot(planning_authority, repo_root).slot_leases;

    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_id = slot_id(slot_number);
        let slot_path = pool_root.join(&slot_id);
        let Some(worktree_record) = worktree_records
            .iter()
            .find(|record| record.path == slot_path)
        else {
            // 학습 주석: git worktree inventory에 없으면 cleanup보다 provisioning/inspection 경로가 먼저 다룹니다.
            continue;
        };
        let Some(branch_name) = worktree_record.branch_name.as_deref() else {
            // 학습 주석: detached slot은 agent branch가 아니므로 reusable baseline reset 경로의 책임입니다.
            continue;
        };
        let expected_agent_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
        if !branch_name.starts_with(&expected_agent_prefix) {
            // 학습 주석: 다른 slot의 agent branch나 사용자 branch를 현재 slot cleanup이 지우지 않게 prefix를 엄격히 맞춥니다.
            continue;
        }
        let slot_lease = slot_leases.get(&slot_id);
        let lease_state = slot_lease.map(|lease| lease.state);
        // 학습 주석: lease가 없을 때만 worktree cleanliness가 cleanup 근거가 됩니다. lease가 있으면 lease state가 우선입니다.
        let worktree_clean = lease_state.is_none()
            && inspect_slot_git_status(&slot_path).is_some_and(SlotGitStatus::is_clean_baseline);
        // 학습 주석: branch integration은 active lease가 아닌 경우에만 확인하며, cleanup pending은 명시적 승인 신호입니다.
        let branch_integrated = !matches!(
            lease_state,
            Some(ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running)
        ) && (matches!(
            lease_state,
            Some(ParallelModeSlotLeaseState::CleanupPending)
        ) || worktree_clean)
            && branch_is_cleanup_ready(repo_root, branch_name);
        // 학습 주석: domain decision object가 lease/git/integration 조합의 최종 cleanup 가능 여부를 단일 규칙으로 판정합니다.
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

/*
학습 주석: cleanup readiness의 핵심 git 질문은 "agent branch의 변경이 pool baseline에
이미 포함되었는가"입니다. `merge-base --is-ancestor`는 branch tip이 base branch의 조상인지
확인하므로, true이면 branch를 지워도 baseline이 그 변경을 잃지 않는다는 뜻입니다.
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
학습 주석: slot cleanup은 세 단계를 모두 성공해야 true를 반환합니다. 먼저 slot worktree를
pool baseline detached 상태로 reset/clean하고, repo에서 agent branch를 삭제하고, planning
authority에 남은 lease metadata를 제거합니다. 마지막으로 git status가 clean baseline인지
다시 확인해 실제 pool 재사용 가능 상태까지 검증합니다.

중간 단계가 실패하면 false만 반환합니다. 호출자는 이 false를 이용해 queue record를
Blocked로 남기거나 reconcile count를 올리지 않습니다. 즉 cleanup 실패는 조용히 성공으로
간주되지 않고 supervisor가 계속 복구 대상으로 볼 수 있게 됩니다.
*/
pub(in crate::application::service::parallel_mode) fn cleanup_slot(
    planning_authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    pool_root: &Path,
    slot_id: &str,
    slot_path: &Path,
    branch_name: &str,
) -> bool {
    // 학습 주석: worktree reset을 branch deletion보다 먼저 수행해 checkout 중인 branch를 안전하게 지울 수 있게 합니다.
    let reset_report = reset_slot_worktree_to_akra(slot_path);
    if !reset_report.succeeded() {
        // 학습 주석: failure summary는 디버깅용으로 계산하지만, 이 helper의 공개 계약은 성공 여부 bool입니다.
        let _failure_summary = reset_report.failure_summary();
        return false;
    }
    // 학습 주석: reset 뒤 agent branch를 삭제해 같은 slot slug 재사용 때 stale branch collision을 줄입니다.
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

    // 학습 주석: 마지막 git status 재검증은 metadata 제거 성공과 실제 worktree 재사용 가능 상태를 함께 확인합니다.
    inspect_slot_git_status(slot_path).is_some_and(SlotGitStatus::is_clean_baseline)
}

/*
학습 주석: pool slot을 baseline으로 되돌리는 git sequence입니다. checkout detach, hard reset,
clean 순서를 한 리포트로 묶어 호출자가 실패 단계를 확인할 수 있게 합니다. branch를 직접
checkout하지 않고 detached baseline으로 두는 이유는 idle slot이 특정 작업 branch를 소유하지
않는 중립 상태여야 다음 lease가 새 agent branch를 안전하게 만들 수 있기 때문입니다.
*/
pub(in crate::application::service::parallel_mode) fn reset_slot_worktree_to_akra(
    slot_path: &Path,
) -> super::super::git_sequence::GitCommandSequenceReport {
    // 학습 주석: git sequence API는 argv 조각을 문자열로 받으므로 Path 변환은 sequence 조립 직전에만 수행합니다.
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

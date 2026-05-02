// 학습 주석: 제외할 task id는 slot lease와 queue record 두 출처에서 모이므로 중복 없는 정렬 집합이 필요합니다.
use std::collections::BTreeSet;

// 학습 주석: integration worktree 위치를 planning authority가 알고 있는 workspace 상태에서 찾기 위해 port를 받습니다.
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;

// 학습 주석: pool helper는 repo root 탐색과 슬롯 worktree git 상태 검사를 담당합니다.
use super::pool::{PoolRuntimeContext, detect_canonical_repo_root, inspect_slot_git_status};
// 학습 주석: orchestration layer는 정해진 integration branch 이름과 현재 브랜치 조회 helper만 사용합니다.
use super::{DISTRIBUTOR_INTEGRATION_BRANCH, current_branch_name};

/*
학습 주석: 병렬 디스패처가 새 작업을 고를 때 이미 "누군가 처리 중인" 작업을
다시 뽑으면 같은 planning task가 두 슬롯에서 중복 실행됩니다. 이 함수는 그
중복 실행을 막기 위한 제외 목록을 만듭니다. 실행 중인 슬롯 lease와 distributor
queue의 활성 record를 함께 보는 이유는, 작업이 실제 슬롯에서 돌고 있는 단계와
공식 완료 후 통합 큐에 들어간 단계가 모두 아직 끝나지 않은 병렬 파이프라인의
일부이기 때문입니다.

`BTreeSet`을 쓰면 두 출처에 같은 task id가 있어도 하나로 합쳐지고, 반환 순서도
안정적입니다. 이 안정성은 TUI 표시와 테스트 결과를 예측 가능하게 만듭니다.
*/
// 학습 주석: 다음 dispatch 후보에서 빼야 하는 task id 목록을 계산합니다.
pub(super) fn parallel_dispatch_excluded_task_ids(context: &PoolRuntimeContext) -> Vec<String> {
    // 학습 주석: 두 출처에서 같은 id가 들어와도 한 번만 반환하고, 정렬된 Vec로 바꾸기 위해 BTreeSet에 누적합니다.
    let mut task_ids = BTreeSet::new();
    // 학습 주석: slot lease에 task_id가 있으면 그 task는 이미 특정 슬롯에서 실행 중입니다.
    // 공백만 있는 값은 의미 있는 task id가 아니므로 trim 후 제거합니다.
    task_ids.extend(
        context
            .slot_leases
            .values()
            .map(|lease| lease.task_id.trim().to_string())
            .filter(|task_id| !task_id.is_empty()),
    );
    // 학습 주석: distributor queue의 active record는 슬롯 실행 이후 통합 단계가 아직 끝나지 않은 task입니다.
    // 그래서 새 슬롯에 다시 배정하면 같은 완료물을 두 번 통합하려는 경합이 생깁니다.
    task_ids.extend(
        context
            .distributor_queue_records
            .iter()
            .filter(|record| record.queue_state.is_active())
            .map(|record| record.task_id.trim().to_string())
            .filter(|task_id| !task_id.is_empty()),
    );

    // 학습 주석: BTreeSet의 정렬 순서를 그대로 Vec로 반환해 caller의 비교와 로그가 매번 같은 순서가 됩니다.
    task_ids.into_iter().collect()
}

/*
학습 주석: distributor queue는 완성된 슬롯 결과를 integration worktree에서 차례대로
처리합니다. 이때 integration worktree가 기대 브랜치가 아니거나 로컬 변경이 남아
있으면, queue 처리가 엉뚱한 브랜치에 merge/rebase 결과를 남기거나 사용자의
미완성 변경과 섞일 수 있습니다. 이 함수는 오케스트레이터 tick의 앞단에서 그런
상태를 문자열 blocker로 바꿔 상위 서비스가 "지금은 진행하지 말라"는 결정을 내리게
합니다.

`detect_canonical_repo_root`는 사용자가 슬롯 worktree나 하위 디렉터리에서 실행해도
기준 저장소 루트를 찾아 줍니다. 그 다음 브랜치 이름과 `SlotGitStatus`를 검사해,
통합 처리에 필요한 최소 조건인 "정해진 integration 브랜치 + staged/unstaged/rebase
메타데이터 없음"을 확인합니다.
*/
// 학습 주석: distributor queue를 처리하기 전에 integration worktree가 안전한 상태인지 검사합니다.
pub(super) fn inspect_akra_integration_worktree_blocker(
    // 학습 주석: canonical repo root를 찾을 때 planning authority의 workspace 정보를 조회하는 port입니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 사용자가 실행한 현재 workspace 경로입니다. slot worktree 안에서 실행되어도 root 탐색의 출발점이 됩니다.
    workspace_dir: &str,
) -> Option<String> {
    // 학습 주석: repo root를 찾지 못하면 이 검사만으로 blocker를 만들 수 없으므로 None을 반환합니다.
    // 상위 흐름은 다른 startup/workspace 검증에서 더 구체적인 오류를 낼 수 있습니다.
    let canonical_repo_root = detect_canonical_repo_root(planning_authority, workspace_dir)?;
    // 학습 주석: integration queue 처리는 항상 지정 브랜치에서만 수행되어야 하므로 현재 브랜치를 먼저 확인합니다.
    let branch_name = current_branch_name(&canonical_repo_root)?;
    if branch_name != DISTRIBUTOR_INTEGRATION_BRANCH {
        return Some(format!(
            "orchestrator blocked / integration worktree must be checked out to `{DISTRIBUTOR_INTEGRATION_BRANCH}` but is `{branch_name}`"
        ));
    }

    // 학습 주석: 브랜치가 맞아도 staged/unstaged/rebase 상태가 있으면 queue 통합 결과가 사용자 변경과 섞일 수 있습니다.
    let status = inspect_slot_git_status(&canonical_repo_root)?;
    if !status.is_ready_for_integration() {
        return Some(format!(
            "orchestrator blocked / integration worktree must be clean before queue processing: {}",
            status.detail_label()
        ));
    }

    // 학습 주석: None은 queue 처리를 막는 worktree 문제가 없다는 신호입니다.
    None
}

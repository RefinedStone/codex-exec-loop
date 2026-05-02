use std::collections::BTreeSet;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;

// orchestration service는 slot pool의 runtime 관찰값과 distributor queue를 함께 본다.
// 하위 pool helper가 repo root 탐색과 git 상태 판정을 맡고, 이 파일은 tick을 막을지 결정한다.
use super::pool::{PoolRuntimeContext, detect_canonical_repo_root, inspect_slot_git_status};
use super::{DISTRIBUTOR_INTEGRATION_BRANCH, current_branch_name};

/*
병렬 디스패처가 새 작업을 고를 때 이미 "누군가 처리 중인" 작업을 다시 뽑으면
같은 planning task가 두 슬롯에서 중복 실행된다. 이 함수는 그 중복 실행을 막기 위한
제외 목록을 만든다. 실행 중인 슬롯 lease와 distributor queue의 활성 record를 함께
보는 이유는, 작업이 슬롯에서 돌고 있는 단계와 공식 완료 후 통합 큐에 들어간 단계가
모두 아직 끝나지 않은 병렬 파이프라인의 일부이기 때문이다.

`BTreeSet`을 쓰면 두 출처에 같은 task id가 있어도 하나로 합쳐지고, 반환 순서도
안정적이다. 이 안정성은 TUI 표시와 테스트 결과를 예측 가능하게 만든다.
*/
pub(super) fn parallel_dispatch_excluded_task_ids(context: &PoolRuntimeContext) -> Vec<String> {
    // 두 출처에서 같은 id가 들어와도 한 번만 반환하고, 정렬된 Vec로 바꾸기 위해 누적한다.
    let mut task_ids = BTreeSet::new();
    // slot lease의 task_id는 이미 특정 worktree와 agent에 배정된 작업을 뜻한다.
    // 공백뿐인 값은 이전 저장 포맷이나 손상 데이터의 잔여물로 보고 제외한다.
    task_ids.extend(
        context
            .slot_leases
            .values()
            .map(|lease| lease.task_id.trim().to_string())
            .filter(|task_id| !task_id.is_empty()),
    );
    // active queue record는 슬롯 실행 이후 prerelease 통합이 끝나지 않은 작업이다.
    // 다시 배정하면 같은 완료물을 두 번 통합하려는 경합이 생긴다.
    task_ids.extend(
        context
            .distributor_queue_records
            .iter()
            .filter(|record| record.queue_state.is_active())
            .map(|record| record.task_id.trim().to_string())
            .filter(|task_id| !task_id.is_empty()),
    );

    // 정렬 순서를 그대로 반환해 caller의 비교, 로그, snapshot 테스트가 매번 같은 순서를 본다.
    task_ids.into_iter().collect()
}

/*
distributor queue는 완성된 슬롯 결과를 integration worktree에서 차례대로 처리한다.
이때 integration worktree가 기대 브랜치가 아니거나 로컬 변경이 남아 있으면 queue
처리가 엉뚱한 브랜치에 merge/rebase 결과를 남기거나 사용자의 미완성 변경과 섞일 수
있다. 이 함수는 오케스트레이터 tick 앞단에서 그런 상태를 문자열 blocker로 바꿔
상위 서비스가 "지금은 진행하지 말라"는 결정을 내리게 한다.

`detect_canonical_repo_root`는 사용자가 슬롯 worktree나 하위 디렉터리에서 실행해도
기준 저장소 루트를 찾아 준다. 그 다음 브랜치 이름과 `SlotGitStatus`를 검사해,
통합 처리에 필요한 최소 조건인 "정해진 integration 브랜치 + staged/unstaged/rebase
메타데이터 없음"을 확인한다.
*/
pub(super) fn inspect_akra_integration_worktree_blocker(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> Option<String> {
    // repo root를 찾지 못하면 이 검사만으로 blocker를 만들 수 없다.
    // 상위 startup/workspace 검증이 더 구체적인 오류를 담당한다.
    let canonical_repo_root = detect_canonical_repo_root(planning_authority, workspace_dir)?;
    // integration queue 처리는 항상 지정 브랜치에서만 수행되어야 하므로 현재 브랜치를 먼저 본다.
    let branch_name = current_branch_name(&canonical_repo_root)?;
    if branch_name != DISTRIBUTOR_INTEGRATION_BRANCH {
        return Some(format!(
            "orchestrator blocked / integration worktree must be checked out to `{DISTRIBUTOR_INTEGRATION_BRANCH}` but is `{branch_name}`"
        ));
    }

    // 브랜치가 맞아도 staged/unstaged/rebase 상태가 있으면 queue 통합 결과가 사용자 변경과 섞인다.
    let status = inspect_slot_git_status(&canonical_repo_root)?;
    if !status.is_ready_for_integration() {
        return Some(format!(
            "orchestrator blocked / integration worktree must be clean before queue processing: {}",
            status.detail_label()
        ));
    }

    // None은 queue 처리를 막는 integration worktree 문제가 없다는 신호다.
    None
}

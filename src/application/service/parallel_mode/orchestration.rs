// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::collections::BTreeSet;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::pool::{PoolRuntimeContext, detect_canonical_repo_root, inspect_slot_git_status};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
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
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn parallel_dispatch_excluded_task_ids(context: &PoolRuntimeContext) -> Vec<String> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut task_ids = BTreeSet::new();
    task_ids.extend(
        context
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .slot_leases
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .values()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .map(|lease| lease.task_id.trim().to_string())
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .filter(|task_id| !task_id.is_empty()),
    );
    task_ids.extend(
        context
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .distributor_queue_records
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .iter()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .filter(|record| record.queue_state.is_active())
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .map(|record| record.task_id.trim().to_string())
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .filter(|task_id| !task_id.is_empty()),
    );

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
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn inspect_akra_integration_worktree_blocker(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    workspace_dir: &str,
) -> Option<String> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let canonical_repo_root = detect_canonical_repo_root(planning_authority, workspace_dir)?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let branch_name = current_branch_name(&canonical_repo_root)?;
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if branch_name != DISTRIBUTOR_INTEGRATION_BRANCH {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Some(format!(
            "orchestrator blocked / integration worktree must be checked out to `{DISTRIBUTOR_INTEGRATION_BRANCH}` but is `{branch_name}`"
        ));
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let status = inspect_slot_git_status(&canonical_repo_root)?;
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !status.is_ready_for_integration() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Some(format!(
            "orchestrator blocked / integration worktree must be clean before queue processing: {}",
            status.detail_label()
        ));
    }

    None
}

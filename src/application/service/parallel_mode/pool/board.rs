// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::parallel_mode::{
    ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
};

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::paths::{derive_default_pool_root, display_pool_path};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::slot_inspection::inspect_pool_slot;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::{DEFAULT_POOL_SIZE, PoolRuntimeContext, detect_canonical_repo_root, slot_id};

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_pool_board_from_context(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    context: &PoolRuntimeContext,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    reconcile_status: impl Into<String>,
) -> ParallelModePoolBoardSnapshot {
    /*
    학습 주석: 이 함수는 이미 `PoolRuntimeContext`를 성공적으로 읽은 정상 경로의 board builder입니다.
    context 안에는 canonical repo root, pool root, git worktree inventory, runtime lease projection,
    baseline head가 들어 있으므로 각 slot을 실제 상태로 검사할 수 있습니다. 여기서 만든 board는
    supervisor snapshot, dispatch capacity 계산, operator recovery notice의 공통 입력이 됩니다.
    */
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let slots = build_pool_slots(context);
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let pool_root_label = display_pool_path(&context.canonical_repo_root, &context.pool_root);

    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_pool_slots(context: &PoolRuntimeContext) -> Vec<ParallelModePoolSlotSnapshot> {
    /*
    학습 주석: slot vector는 항상 `DEFAULT_POOL_SIZE`개의 고정 순서 항목을 만듭니다. TUI board는
    slot-1, slot-2, slot-3처럼 안정된 위치를 기대하고, dispatch plan도 idle count를 이 projection
    에서 계산합니다. 각 slot의 복잡한 판정은 `inspect_pool_slot`에 위임해 board builder는 목록
    구성에만 집중합니다.
    */
    (1..=DEFAULT_POOL_SIZE)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|slot_number| inspect_pool_slot(context, &slot_id(slot_number)))
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect::<Vec<_>>()
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_unavailable_pool_board(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    workspace_dir: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    reconcile_status: impl Into<String>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    branch_name: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    worktree_label: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    owner_label: &str,
) -> ParallelModePoolBoardSnapshot {
    /*
    학습 주석: unavailable board는 pool을 검사할 수 없는 readiness 단계에서 쓰는 placeholder입니다.
    예를 들어 git repo root를 찾지 못했거나 planning capability가 아직 준비되지 않았을 때도 TUI는
    같은 shape의 board를 그려야 합니다. 모든 slot을 Unavailable로 채워 UI layout은 유지하되,
    branch/worktree/owner label을 caller가 준 readiness 원인으로 통일합니다.
    */
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let pool_root_label = derive_pool_root_label(planning_authority, workspace_dir);
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let slots = (1..=DEFAULT_POOL_SIZE)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|slot_number| {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ParallelModePoolSlotSnapshot::new(
                slot_id(slot_number),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                ParallelModePoolSlotState::Unavailable,
                branch_name,
                worktree_label,
                owner_label,
            )
        })
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect::<Vec<_>>();

    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_blocked_pool_board(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    workspace_dir: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    reconcile_status: impl Into<String>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    detail: &str,
) -> ParallelModePoolBoardSnapshot {
    /*
    학습 주석: blocked board는 repo/pool root는 어느 정도 계산할 수 있지만, 전체 pool inspection을
    신뢰할 수 없는 상태에서 사용합니다. unavailable이 "아직 준비되지 않음"에 가깝다면, blocked는
    "operator가 복구해야 하는 위험 상태"에 가깝습니다. 모든 slot에 같은 detail을 부여해 화면이
    개별 slot 상태보다 상위 blocker를 먼저 드러내게 합니다.
    */
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let pool_root_label = derive_pool_root_label(planning_authority, workspace_dir);
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let slots = (1..=DEFAULT_POOL_SIZE)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|slot_number| {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ParallelModePoolSlotSnapshot::new(
                slot_id(slot_number),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                ParallelModePoolSlotState::Blocked,
                "unknown",
                detail,
                "operator recovery",
            )
        })
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect::<Vec<_>>();

    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
}

/*
학습 주석: placeholder board에서도 pool root label을 최대한 실제 값으로 보여 주기 위한 helper입니다.
runtime context를 만들 수 없는 상황이라도 canonical repo root만 찾을 수 있으면 default pool root가
어디일지 계산할 수 있습니다. 이 값은 사용자가 파일시스템에서 pool 상태를 확인하거나 수동 복구할
때 첫 단서가 됩니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn derive_pool_root_label(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    workspace_dir: &str,
) -> String {
    /*
    학습 주석: canonical root detection이 실패하면 pool root 자체도 안전하게 추정할 수 없습니다.
    이때는 `"not available"`을 반환해 TUI가 path처럼 보이는 잘못된 문자열을 표시하지 않게 합니다.
    성공하면 실제 default pool root를 계산하고, repo parent 기준 상대 label로 줄여 board에 넣습니다.
    */
    detect_canonical_repo_root(planning_authority, workspace_dir)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|canonical_repo_root| {
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let pool_root = derive_default_pool_root(&canonical_repo_root);
            display_pool_path(&canonical_repo_root, &pool_root)
        })
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .unwrap_or_else(|| "not available".to_string())
}

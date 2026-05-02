use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
};

use super::paths::{derive_default_pool_root, display_pool_path};
use super::slot_inspection::inspect_pool_slot;
use super::{DEFAULT_POOL_SIZE, PoolRuntimeContext, detect_canonical_repo_root, slot_id};

/*
pool board builder는 git/worktree 검사 결과를 TUI와 API가 소비하는 domain snapshot으로
투영하는 application 경계다. 실제 slot 판정은 `slot_inspection`에 두고, 이 파일은 고정 slot
목록 shape, fallback board, pool root label 계산을 한곳에서 맞춘다.
*/
pub(super) fn build_pool_board_from_context(
    context: &PoolRuntimeContext,
    reconcile_status: impl Into<String>,
) -> ParallelModePoolBoardSnapshot {
    /*
    이 함수는 이미 `PoolRuntimeContext`를 성공적으로 읽은 정상 경로의 board builder이다.
    context 안에는 canonical repo root, pool root, git worktree inventory, runtime lease projection,
    baseline head가 들어 있으므로 각 slot을 실제 상태로 검사할 수 있다. 여기서 만든 board는
    supervisor snapshot, dispatch capacity 계산, operator recovery notice의 공통 입력이 된다.
    */
    // runtime context가 준비된 경우에는 모든 slot을 실제 Git/worktree 상태로 검사한다.
    let slots = build_pool_slots(context);
    // pool root label은 절대경로 대신 repo 기준으로 접어, TUI board에서 긴 path가
    // slot 목록을 밀어내지 않게 한다.
    let pool_root_label = display_pool_path(&context.canonical_repo_root, &context.pool_root);

    // snapshot에는 고정 pool 크기와 실제 slot 목록을 함께 담아, UI와 dispatch capacity
    // 계산이 같은 source of truth를 보게 한다.
    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
}

pub(super) fn build_pool_slots(context: &PoolRuntimeContext) -> Vec<ParallelModePoolSlotSnapshot> {
    /*
    slot vector는 항상 `DEFAULT_POOL_SIZE`개의 고정 순서 항목을 만든다. TUI board는
    slot-1, slot-2, slot-3처럼 안정된 위치를 기대하고, dispatch plan도 idle count를 이 projection
    에서 계산한다. 각 slot의 복잡한 판정은 `inspect_pool_slot`에 위임해 board builder는 목록
    구성에만 집중한다.
    */
    (1..=DEFAULT_POOL_SIZE)
        // 숫자 slot은 `slot-1` 같은 canonical id로 바꾼 뒤 inspection에 넘긴다.
        .map(|slot_number| inspect_pool_slot(context, &slot_id(slot_number)))
        // board snapshot이 소유하는 Vec로 확정해 caller가 context 수명과 무관하게 들고 갈 수 있다.
        .collect::<Vec<_>>()
}

pub(super) fn build_unavailable_pool_board(
    // runtime context를 만들지 못한 경우에도 pool root label을 유도하기 위해 쓰는 포트이다.
    planning_authority: &dyn PlanningAuthorityPort,
    // canonical repo root 탐지의 출발점이 되는 현재 workspace 디렉터리이다.
    workspace_dir: &str,
    // readiness가 어디에서 멈췄는지 board 전체에 붙일 상태 문구이다.
    reconcile_status: impl Into<String>,
    // 모든 unavailable slot에 공통으로 표시할 branch placeholder이다.
    branch_name: &str,
    // 모든 unavailable slot에 공통으로 표시할 worktree/reason label이다.
    worktree_label: &str,
    // 모든 unavailable slot에 공통으로 표시할 owner/recovery label이다.
    owner_label: &str,
) -> ParallelModePoolBoardSnapshot {
    /*
    unavailable board는 pool을 검사할 수 없는 readiness 단계에서 쓰는 placeholder이다.
    예를 들어 git repo root를 찾지 못했거나 planning capability가 아직 준비되지 않았을 때도 TUI는
    같은 shape의 board를 그려야 한다. 모든 slot을 Unavailable로 채워 UI layout은 유지하되,
    branch/worktree/owner label을 caller가 준 readiness 원인으로 통일한다.
    */
    // 실제 context가 없어도 가능한 한 실제 pool root 후보를 표시해 operator의 다음 행동을 돕는다.
    let pool_root_label = derive_pool_root_label(planning_authority, workspace_dir);
    // 정상 board와 같은 slot 개수를 유지해 TUI layout과 downstream capacity 계산의 shape를 보존한다.
    let slots = (1..=DEFAULT_POOL_SIZE)
        // 실제 slot 검사 대신 caller가 준 공통 readiness label로 slot snapshot을 합성한다.
        .map(|slot_number| {
            ParallelModePoolSlotSnapshot::new(
                slot_id(slot_number),
                // Unavailable은 아직 위험 상태가 아니라 prerequisites가 준비되지 않은 상태로 표시된다.
                ParallelModePoolSlotState::Unavailable,
                branch_name,
                worktree_label,
                owner_label,
            )
        })
        // placeholder slot들도 실제 board와 같은 owned snapshot 목록으로 반환한다.
        .collect::<Vec<_>>();

    // unavailable board도 정상 board와 같은 DTO를 써서 renderer가 별도 예외 경로 없이 그린다.
    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
}

pub(super) fn build_blocked_pool_board(
    // blocked 상태에서도 pool root label을 가능한 만큼 실제 값으로 계산하기 위한 포트이다.
    planning_authority: &dyn PlanningAuthorityPort,
    // canonical repo root를 찾을 기준 workspace이다.
    workspace_dir: &str,
    // board 상단에 표시할 reconcile/blocking 상태이다.
    reconcile_status: impl Into<String>,
    // 모든 slot의 worktree label에 넣어 operator가 복구해야 할 원인을 반복 노출한다.
    detail: &str,
) -> ParallelModePoolBoardSnapshot {
    /*
    blocked board는 repo/pool root는 어느 정도 계산할 수 있지만, 전체 pool inspection을
    신뢰할 수 없는 상태에서 사용한다. unavailable이 "아직 준비되지 않음"에 가깝다면, blocked는
    "operator가 복구해야 하는 위험 상태"에 가깝다. 모든 slot에 같은 detail을 부여해 화면이
    개별 slot 상태보다 상위 blocker를 먼저 드러내게 한다.
    */
    // blocked board 역시 실제 pool root 후보를 보여 줘, 사용자가 어느 디렉터리를 확인할지 알게 한다.
    let pool_root_label = derive_pool_root_label(planning_authority, workspace_dir);
    // 전체 pool이 차단된 상태라 개별 slot inspection을 하지 않고 같은 blocked snapshot을 반복한다.
    let slots = (1..=DEFAULT_POOL_SIZE)
        // slot id만 고유하게 유지하고, branch/worktree/owner label은 공통 복구 메시지로 통일한다.
        .map(|slot_number| {
            ParallelModePoolSlotSnapshot::new(
                slot_id(slot_number),
                // Blocked는 unavailable보다 강한 신호라 dispatch가 capacity로 해석하면 안 된다.
                ParallelModePoolSlotState::Blocked,
                "unknown",
                detail,
                "operator recovery",
            )
        })
        // renderer가 정상 board와 같은 리스트 렌더링 경로를 쓰도록 Vec snapshot으로 반환한다.
        .collect::<Vec<_>>();

    // 전체 board를 blocked로 만들지만 DTO shape는 유지해 화면과 API 계약을 안정화한다.
    ParallelModePoolBoardSnapshot::new(DEFAULT_POOL_SIZE, pool_root_label, reconcile_status, slots)
}

/*
placeholder board에서도 pool root label을 최대한 실제 값으로 보여 주기 위한 helper이다.
runtime context를 만들 수 없는 상황이라도 canonical repo root만 찾을 수 있으면 default pool root가
어디일지 계산할 수 있다. 이 값은 사용자가 파일시스템에서 pool 상태를 확인하거나 수동 복구할
때 첫 단서가 된다.
*/
fn derive_pool_root_label(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> String {
    /*
    canonical root detection이 실패하면 pool root 자체도 안전하게 추정할 수 없다.
    이때는 `"not available"`을 반환해 TUI가 path처럼 보이는 잘못된 문자열을 표시하지 않게 한다.
    성공하면 실제 default pool root를 계산하고, repo parent 기준 상대 label로 줄여 board에 넣는다.
    */
    detect_canonical_repo_root(planning_authority, workspace_dir)
        // repo root를 찾은 경우에는 production pool root 규칙을 그대로 적용해 placeholder도
        // 실제 runtime board와 같은 위치를 가리키게 한다.
        .map(|canonical_repo_root| {
            // default pool root 계산은 paths 모듈에 위임해 board builder가 경로 규칙을 복제하지 않는다.
            let pool_root = derive_default_pool_root(&canonical_repo_root);
            display_pool_path(&canonical_repo_root, &pool_root)
        })
        // repo root 탐지 실패 시에는 path처럼 보이는 추측값 대신 명시적인 unavailable label을 쓴다.
        .unwrap_or_else(|| "not available".to_string())
}

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::fs;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::path::{Path, PathBuf};

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::super::ensure_directory_exists;

/*
학습 주석: lease 파일은 planning authority의 runtime lease record를 사람이 확인하거나
복구할 수 있게 pool root 아래에 미러링한 JSON입니다. 실제 권위 있는 저장소는
`PlanningAuthorityPort`이지만, `.leases/<slot>.json` 미러는 worktree pool을 파일시스템에서
점검할 때 중요한 단서가 됩니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn slot_leases_root(pool_root: &Path) -> PathBuf {
    pool_root.join(".leases")
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(in crate::application::service::parallel_mode) fn slot_lease_file_path(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pool_root: &Path,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    slot_id: &str,
) -> PathBuf {
    slot_leases_root(pool_root).join(format!("{slot_id}.json"))
}

/*
학습 주석: slot lease 저장은 두 저장소를 함께 갱신합니다. 먼저 planning authority에
upsert해 application이 읽는 runtime projection을 갱신하고, 그 다음 pool root의 JSON 파일을
temp file + rename 방식으로 기록합니다. rename을 쓰는 이유는 중간에 프로세스가 죽어도
부분적으로 쓰인 lease 파일을 최종 파일명으로 남기지 않기 위해서입니다.

이 함수가 실패를 `String`으로 자세히 반환하는 이유는 슬롯 획득/상태 전이 중 어디서
원장 갱신이 막혔는지 TUI notice와 테스트에서 바로 드러내기 위해서입니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(in crate::application::service::parallel_mode) fn write_slot_lease(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    workspace_dir: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pool_root: &Path,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<(), String> {
    planning_authority
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .upsert_runtime_slot_lease(workspace_dir, lease)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map_err(|error| format!("failed to store slot lease `{}`: {error}", lease.slot_id))?;

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let leases_root = slot_leases_root(pool_root);
    ensure_directory_exists(&leases_root)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map_err(|error| format!("failed to create lease directory: {error}"))?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let lease_path = slot_lease_file_path(pool_root, &lease.slot_id);
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let temp_path = lease_path.with_extension("tmp");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let lease_body = serde_json::to_string_pretty(lease)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map_err(|error| format!("failed to serialize slot lease: {error}"))?;
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    fs::write(&temp_path, lease_body).map_err(|error| {
        format!(
            "failed to write temporary slot lease `{}`: {error}",
            lease.slot_id
        )
    })?;
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    fs::rename(&temp_path, &lease_path)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map_err(|error| format!("failed to persist slot lease `{}`: {error}", lease.slot_id))
}

/*
학습 주석: slot lease 제거는 cleanup의 마지막 원장 정리 단계입니다. planning authority에서
runtime lease를 먼저 지우고, 성공한 경우에만 파일 미러를 지웁니다. 권위 저장소 삭제가
실패했는데 파일만 지우면 application projection과 파일시스템 단서가 엇갈리므로 false를
반환해 호출자가 cleanup 실패로 취급하게 합니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(in crate::application::service::parallel_mode) fn remove_slot_lease(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    workspace_dir: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pool_root: &Path,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    slot_id: &str,
) -> bool {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if planning_authority
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .remove_runtime_slot_lease(workspace_dir, slot_id)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .is_err()
    {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return false;
    }
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let lease_path = slot_lease_file_path(pool_root, slot_id);
    !lease_path.exists() || fs::remove_file(lease_path).is_ok()
}

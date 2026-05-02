// 학습 주석: lease mirror는 pool root 아래 JSON 파일로 남기므로 표준 fs write/rename/remove API를 사용합니다.
use std::fs;
// 학습 주석: pool root와 `.leases/<slot>.json` mirror 경로를 조립하기 위해 Path/PathBuf를 함께 씁니다.
use std::path::{Path, PathBuf};

// 학습 주석: 권위 있는 lease 상태는 planning authority port에 저장됩니다. 이 파일의 JSON mirror는
// 관찰/복구 보조물이므로 write/remove 순서가 authority projection과 어긋나지 않아야 합니다.
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
// 학습 주석: lease snapshot은 slot id, task owner, branch, state를 모두 담는 domain DTO입니다. 같은
// snapshot을 authority store와 filesystem mirror에 저장해 두 저장소가 동일한 상태를 표현하게 합니다.
use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;

// 학습 주석: pool-local persistence helper와 같은 directory creation 정책을 공유합니다.
use super::super::ensure_directory_exists;

/*
학습 주석: lease 파일은 planning authority의 runtime lease record를 사람이 확인하거나
복구할 수 있게 pool root 아래에 미러링한 JSON입니다. 실제 권위 있는 저장소는
`PlanningAuthorityPort`이지만, `.leases/<slot>.json` 미러는 worktree pool을 파일시스템에서
점검할 때 중요한 단서가 됩니다.
*/
// 학습 주석: pool root에서 lease mirror 디렉터리를 계산하는 작은 path helper입니다. 모든 write/remove가
// 이 함수를 지나가게 해 `.leases` namespace가 코드 여러 곳에 흩어지지 않게 합니다.
fn slot_leases_root(pool_root: &Path) -> PathBuf {
    /*
    학습 주석: `.leases`는 pool root 아래의 lease mirror namespace입니다. 실제 slot worktree
    내부에 lease 파일을 두지 않는 이유는 cleanup 과정에서 worktree가 reset/clean되어도 운영
    metadata는 살아 있어야 하기 때문입니다. pool root 기준으로 모아 두면 reconciliation과
    수동 점검이 slot 디렉터리와 lease 파일을 나란히 확인할 수 있습니다.
    */
    pool_root.join(".leases")
}

// 학습 주석: 특정 slot lease mirror 파일의 최종 경로를 계산합니다. pool inspector와 테스트가 같은 helper를
// 쓰므로 실제 저장 위치와 검증 위치가 갈라지지 않습니다.
pub(in crate::application::service::parallel_mode) fn slot_lease_file_path(
    // 학습 주석: pool_root는 병렬 worktree pool의 루트입니다. slot worktree 자체가 아니라 root 아래
    // `.leases`에 기록해야 cleanup 중 worktree 내용 변화와 lease metadata가 분리됩니다.
    pool_root: &Path,
    // 학습 주석: slot_id는 domain에서 생성한 안정적인 id입니다. 파일명으로 바로 사용해 operator가
    // `slot-2.json`처럼 눈으로 대응 관계를 찾을 수 있게 합니다.
    slot_id: &str,
) -> PathBuf {
    /*
    학습 주석: lease mirror filename은 slot id를 그대로 사용합니다. slot id는 `slot-1`처럼
    pool이 생성한 안전한 값이라 별도 sanitization이 필요 없고, 테스트와 운영자가 특정 slot의
    lease JSON을 예측 가능한 경로에서 찾을 수 있습니다.
    */
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
// 학습 주석: slot lease 상태 전이를 영속화합니다. dispatcher/supervisor가 읽는 authority projection을
// 먼저 갱신하고, 그 다음 사람이 확인 가능한 `.leases` mirror를 atomic-ish 방식으로 갱신합니다.
pub(in crate::application::service::parallel_mode) fn write_slot_lease(
    // 학습 주석: planning_authority는 runtime projection의 source of truth입니다. SQLite adapter든
    // 테스트 fake든 같은 port를 통해 lease row를 갱신합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: workspace_dir은 authority row scope입니다. 같은 pool이라도 workspace별 runtime projection이
    // 다를 수 있으므로 lease upsert/remove에는 항상 workspace를 같이 넘깁니다.
    workspace_dir: &str,
    // 학습 주석: pool_root는 mirror 파일의 filesystem scope입니다. authority write와 달리 이 값은
    // `.leases` directory와 temp file 경로를 만드는 데만 쓰입니다.
    pool_root: &Path,
    // 학습 주석: lease는 저장할 완성 snapshot입니다. caller가 Leased/Running/CleanupPending 같은
    // 상태 전이를 이미 결정하고, 이 함수는 그 결정을 두 저장소에 반영합니다.
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<(), String> {
    /*
    학습 주석: write 순서는 의도적으로 authority store가 먼저입니다. application runtime은
    `PlanningAuthorityPort`의 projection을 읽어 lease state를 판단하므로, 파일 mirror만 먼저
    쓰고 authority upsert에 실패하면 supervisor와 dispatcher가 서로 다른 상태를 보게 됩니다.
    이 함수는 authority write가 실패하면 mirror를 건드리지 않고 즉시 중단합니다.
    */
    planning_authority
        // 학습 주석: authority upsert가 성공해야 이후 supervisor snapshot과 distributor delivery가 같은
        // lease state를 보게 됩니다. 실패하면 mirror를 쓰지 않아 split-brain 상태를 피합니다.
        .upsert_runtime_slot_lease(workspace_dir, lease)
        .map_err(|error| format!("failed to store slot lease `{}`: {error}", lease.slot_id))?;

    /*
    학습 주석: mirror write는 authority 성공 뒤의 보조 기록입니다. 그래도 오류를 무시하지 않는
    이유는 `.leases` 파일이 recovery 테스트, 수동 디버깅, legacy mirror-loss 시나리오에서
    중요한 관찰 지점이기 때문입니다. caller가 오류를 받으면 slot lease 전이를 실패로 보고
    사용자에게 명확한 원인을 표시할 수 있습니다.
    */
    // 학습 주석: mirror directory는 authority write 이후에 만듭니다. directory 생성 실패는 authority에는
    // 이미 반영된 상태라 caller에게 오류를 돌려 cleanup/retry가 가능하게 합니다.
    let leases_root = slot_leases_root(pool_root);
    ensure_directory_exists(&leases_root)
        .map_err(|error| format!("failed to create lease directory: {error}"))?;
    // 학습 주석: 최종 파일과 임시 파일 경로를 분리합니다. 같은 slot의 lease를 덮어쓸 때도 기존 JSON은
    // rename 직전까지 유지됩니다.
    let lease_path = slot_lease_file_path(pool_root, &lease.slot_id);
    let temp_path = lease_path.with_extension("tmp");
    // 학습 주석: pretty JSON을 쓰는 이유는 mirror가 프로그램뿐 아니라 사람의 복구/점검 입력이기도 하기 때문입니다.
    let lease_body = serde_json::to_string_pretty(lease)
        .map_err(|error| format!("failed to serialize slot lease: {error}"))?;
    /*
    학습 주석: temp path 확장자는 최종 `.json`과 구분되는 `.tmp`입니다. 같은 slot lease를 덮어쓸
    때도 먼저 temp에 완성된 JSON을 쓰고 rename하므로, reader가 중간에 파일을 열어도 깨진
    최종 JSON을 볼 가능성을 줄입니다. 이 패턴은 distributor queue와 session detail mirror에도
    반복되는 pool-local persistence 규칙입니다.
    */
    // 학습 주석: temp write 실패에는 slot id를 붙입니다. pool에는 여러 slot이 동시에 존재하므로
    // path보다 운영자가 알아보는 slot id가 오류 triage에 바로 필요합니다.
    fs::write(&temp_path, lease_body).map_err(|error| {
        format!(
            "failed to write temporary slot lease `{}`: {error}",
            lease.slot_id
        )
    })?;
    // 학습 주석: rename이 성공하는 순간 mirror의 최종 파일이 새 snapshot으로 교체됩니다. 이 단계가
    // 실패하면 authority는 이미 갱신됐지만 파일 관찰 상태가 낡을 수 있어 오류를 반환합니다.
    fs::rename(&temp_path, &lease_path)
        .map_err(|error| format!("failed to persist slot lease `{}`: {error}", lease.slot_id))
}

/*
학습 주석: slot lease 제거는 cleanup의 마지막 원장 정리 단계입니다. planning authority에서
runtime lease를 먼저 지우고, 성공한 경우에만 파일 미러를 지웁니다. 권위 저장소 삭제가
실패했는데 파일만 지우면 application projection과 파일시스템 단서가 엇갈리므로 false를
반환해 호출자가 cleanup 실패로 취급하게 합니다.
*/
// 학습 주석: slot lease를 authority projection과 filesystem mirror 양쪽에서 제거합니다. cleanup 성공 후
// slot이 idle로 다시 보이려면 권위 저장소 삭제가 먼저 성공해야 합니다.
pub(in crate::application::service::parallel_mode) fn remove_slot_lease(
    // 학습 주석: lease row 삭제의 source of truth입니다. 삭제 실패는 slot이 아직 runtime projection에서
    // active로 보일 수 있음을 뜻하므로 false로 반환합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: workspace_dir은 삭제할 authority projection scope입니다.
    workspace_dir: &str,
    // 학습 주석: pool_root는 삭제할 mirror file scope입니다.
    pool_root: &Path,
    // 학습 주석: slot_id는 authority key와 mirror filename을 동시에 식별합니다.
    slot_id: &str,
) -> bool {
    /*
    학습 주석: remove는 cleanup_slot의 마지막 상태 정리 단계에서 호출됩니다. agent branch가
    baseline에 통합되고 slot worktree가 detached baseline으로 돌아간 뒤에 lease를 지워야 slot이
    다시 idle로 보입니다. 따라서 authority delete 실패는 단순 mirror 정리 실패가 아니라
    "pool이 아직 이 slot을 leased로 볼 수 있음"이라는 의미라 false로 보고합니다.
    */
    if planning_authority
        .remove_runtime_slot_lease(workspace_dir, slot_id)
        .is_err()
    {
        return false;
    }
    /*
    학습 주석: authority에서 lease가 제거된 뒤에는 mirror 파일 삭제를 시도합니다. 파일이 이미
    없다면 이전 recovery나 수동 정리로 mirror가 사라진 상태일 수 있으므로 성공으로 취급합니다.
    반대로 파일이 있고 삭제가 실패하면 pool root의 관찰 가능한 상태가 남아 있으므로 false를
    반환해 cleanup caller가 재시도/복구 대상으로 남길 수 있게 합니다.
    */
    // 학습 주석: mirror가 이미 없으면 성공으로 봅니다. authority가 지워진 뒤의 mirror deletion은
    // idempotent cleanup 성격이라, missing file을 오류로 만들면 수동 복구 후 cleanup 재시도가 불필요하게 실패합니다.
    let lease_path = slot_lease_file_path(pool_root, slot_id);
    !lease_path.exists() || fs::remove_file(lease_path).is_ok()
}

use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;
use std::path::{Path, PathBuf};

/*
lease 파일은 planning authority의 runtime lease record를 사람이 확인하거나
복구할 수 있게 pool root 아래에 미러링한 JSON이다. 실제 권위 있는 저장소는
`PlanningAuthorityPort`이지만, `.leases/<slot>.json` 미러는 worktree pool을 파일시스템에서
점검할 때 중요한 단서가 된다.
*/
fn slot_lease_relative_path(slot_id: &str) -> PathBuf {
    PathBuf::from(".leases").join(format!("{slot_id}.json"))
}

// 특정 slot lease mirror 파일의 최종 경로를 계산한다. pool inspector와 테스트가 같은 helper를
// 쓰므로 실제 저장 위치와 검증 위치가 갈라지지 않는다.
pub(in crate::application::service::parallel_mode) fn slot_lease_file_path(
    // pool_root는 병렬 worktree pool의 루트이다. slot worktree 자체가 아니라 root 아래
    // `.leases`에 기록해야 cleanup 중 worktree 내용 변화와 lease metadata가 분리된다.
    pool_root: &Path,
    // slot_id는 domain에서 생성한 안정적인 id이다. 파일명으로 바로 사용해 operator가
    // `slot-2.json`처럼 눈으로 대응 관계를 찾을 수 있게 한다.
    slot_id: &str,
) -> PathBuf {
    /*
    lease mirror filename은 slot id를 그대로 사용한다. slot id는 `slot-1`처럼
    pool이 생성한 안전한 값이라 별도 sanitization이 필요 없고, 테스트와 운영자가 특정 slot의
    lease JSON을 예측 가능한 경로에서 찾을 수 있다.
    */
    pool_root.join(slot_lease_relative_path(slot_id))
}

/*
slot lease 저장은 두 저장소를 함께 갱신한다. 먼저 planning authority에
upsert해 application이 읽는 runtime projection을 갱신하고, 그 다음 pool root의 JSON 파일을
FD-anchored private atomic mirror capability로 기록한다. application은 임시 파일명을 만들지
않으며 adapter가 무작위 exclusive temp와 최종 경로 identity를 함께 검증한다.

이 함수가 실패를 `String`으로 자세히 반환하는 이유는 슬롯 획득/상태 전이 중 어디서
원장 갱신이 막혔는지 TUI notice와 테스트에서 바로 드러내기 위해서이다.
*/
// 새 slot generation의 최초 lease를 영속화한다. 기존 generation의 lifecycle state 변경은
// `transition_slot_lease`가 양쪽 store에 exact previous-snapshot CAS를 적용한다.
pub(in crate::application::service::parallel_mode) fn write_slot_lease(
    // planning_authority는 runtime projection의 source of truth이다. SQLite adapter든
    // 테스트 fake든 같은 port를 통해 lease row를 갱신한다.
    planning_authority: &dyn PlanningAuthorityPort,
    // runtime은 pool-local mirror 파일 I/O의 outbound boundary이다. authority write 순서는
    // application이 결정하지만, 실제 private atomic install은 이 port 뒤에서 수행한다.
    runtime: &dyn ParallelModeRuntimePort,
    // workspace_dir은 authority row scope이다. 같은 pool이라도 workspace별 runtime projection이
    // 다를 수 있으므로 lease upsert/remove에는 항상 workspace를 같이 넘긴다.
    workspace_dir: &str,
    // pool_root는 mirror 파일의 filesystem scope이다. authority write와 달리 이 값은
    // `.leases` 아래의 final relative path를 고정하는 데만 쓴다.
    pool_root: &Path,
    // lease는 저장할 완성 snapshot이다. caller가 Leased/Running/CleanupPending 같은
    // 상태 전이를 이미 결정하고, 이 함수는 그 결정을 두 저장소에 반영한다.
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<(), String> {
    /*
    write 순서는 의도적으로 authority store가 먼저이다. application runtime은
    `PlanningAuthorityPort`의 projection을 읽어 lease state를 판단하므로, 파일 mirror만 먼저
    쓰고 authority upsert에 실패하면 supervisor와 dispatcher가 서로 다른 상태를 보게 된다.
    이 함수는 authority write가 실패하면 mirror를 건드리지 않고 즉시 중단한다.
    */
    planning_authority
        // authority upsert가 성공해야 이후 supervisor snapshot과 distributor delivery가 같은
        // lease state를 보게 된다. 실패하면 mirror를 쓰지 않아 split-brain 상태를 피한다.
        .upsert_runtime_slot_lease(workspace_dir, lease)
        .map_err(|error| format!("failed to store slot lease `{}`: {error}", lease.slot_id))?;

    /*
    mirror write는 authority 성공 뒤의 보조 기록이다. 그래도 오류를 무시하지 않는
    이유는 `.leases` 파일이 recovery 테스트, 수동 디버깅, legacy mirror-loss 시나리오에서
    중요한 관찰 지점이기 때문이다. caller가 오류를 받으면 slot lease 전이를 실패로 보고
    사용자에게 명확한 원인을 표시할 수 있다.
    */
    // pretty JSON을 쓰는 이유는 mirror가 프로그램뿐 아니라 사람의 복구/점검 입력이기도 하기 때문이다.
    let lease_body = serde_json::to_string_pretty(lease)
        .map_err(|error| format!("failed to serialize slot lease: {error}"))?;
    /*
    application은 pool root와 normalized relative final path만 넘긴다. adapter가 pinned
    directory descriptor 아래에서 private random temp를 만들고 fsync 후 원자 교체하므로,
    예측 가능한 temp alias나 metadata-directory symlink를 따라가지 않는다.
    */
    // write 실패에는 slot id를 붙인다. pool에는 여러 slot이 동시에 존재하므로
    // path보다 운영자가 알아보는 slot id가 오류 triage에 바로 필요하다.
    runtime
        .write_runtime_mirror_atomic(
            pool_root,
            &slot_lease_relative_path(&lease.slot_id),
            &lease_body,
        )
        .map_err(|error| format!("failed to persist slot lease `{}`: {error}", lease.slot_id))
}

pub(in crate::application::service::parallel_mode) fn transition_slot_lease(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    workspace_dir: &str,
    pool_root: &Path,
    previous: &ParallelModeSlotLeaseSnapshot,
    next: &ParallelModeSlotLeaseSnapshot,
) -> Result<(), String> {
    if !previous.same_generation_as(next) {
        return Err(format!(
            "slot lease `{}` lifecycle transition changed immutable generation identity",
            previous.slot_id
        ));
    }
    let next_body = serde_json::to_string_pretty(next)
        .map_err(|error| format!("failed to serialize next slot lease: {error}"))?;
    let relative = slot_lease_relative_path(&next.slot_id);
    let observed_mirror = runtime
        .read_runtime_mirror_optional(pool_root, &relative)
        .map_err(|error| {
            format!(
                "failed to inspect slot lease transition mirror `{}`: {error}",
                next.slot_id
            )
        })?;
    if let Some(body) = observed_mirror.as_deref() {
        let observed =
            serde_json::from_str::<ParallelModeSlotLeaseSnapshot>(body).map_err(|_| {
                format!(
                    "slot lease transition mirror `{}` is malformed",
                    next.slot_id
                )
            })?;
        if !observed.same_generation_as(previous) {
            return Err(format!(
                "slot lease transition mirror `{}` belongs to a different generation",
                next.slot_id
            ));
        }
    }

    let authority_transitioned = match planning_authority.replace_runtime_slot_lease_if_matches(
        workspace_dir,
        previous,
        next,
    ) {
        Ok(transitioned) => transitioned,
        Err(error) => {
            let restored = planning_authority
                .replace_runtime_slot_lease_if_matches(workspace_dir, next, previous)
                .unwrap_or(false);
            return Err(format!(
                "failed to transition slot lease `{}` in authority: {error}; ambiguous write rollback: {}",
                next.slot_id,
                if restored {
                    "exact previous snapshot restored"
                } else {
                    "no matching transition snapshot was replaced"
                }
            ));
        }
    };
    if !authority_transitioned {
        return Err(format!(
            "slot lease `{}` changed before the lifecycle transition",
            next.slot_id
        ));
    }

    let mirror_failure = match runtime.compare_and_swap_runtime_mirror_file(
        pool_root,
        &relative,
        observed_mirror.as_deref(),
        Some(&next_body),
    ) {
        Ok(true) => return Ok(()),
        Ok(false) => "mirror changed after its exact snapshot was inspected".to_string(),
        Err(error) => error.to_string(),
    };

    let mirror_restored = runtime
        .compare_and_swap_runtime_mirror_file(
            pool_root,
            &relative,
            Some(&next_body),
            observed_mirror.as_deref(),
        )
        .unwrap_or(false)
        || runtime
            .read_runtime_mirror_optional(pool_root, &relative)
            .is_ok_and(|body| body.as_deref() == observed_mirror.as_deref());
    let authority_restored = planning_authority
        .replace_runtime_slot_lease_if_matches(workspace_dir, next, previous)
        .unwrap_or(false);
    let rollback = match (authority_restored, mirror_restored) {
        (true, true) => "exact previous snapshot restored in authority and mirror",
        (true, false) => "authority restored; mirror was replaced or could not be restored",
        (false, true) => "mirror restored; authority was replaced or could not be restored",
        (false, false) => {
            "replacement state preserved; neither store matched the failed transition"
        }
    };
    Err(format!(
        "failed to persist slot lease transition `{}`: {mirror_failure}; rollback: {rollback}",
        next.slot_id
    ))
}

/*
slot lease 제거는 cleanup의 마지막 원장 정리 단계이다. mirror와 authority 모두 cleanup이
검증한 exact snapshot과 일치할 때만 삭제한다. mirror CAS를 먼저 실행해야 authority 삭제 뒤
교체된 mirror를 발견하는 부분 성공을 피할 수 있고, authority CAS는 늦게 들어온 새 세대를
절대로 지우지 않는다.
*/
// exact lease generation을 authority projection과 filesystem mirror 양쪽에서 제거한다.
pub(in crate::application::service::parallel_mode) fn remove_slot_lease(
    // lease row 삭제의 source of truth이다. exact compare-and-delete 실패는 replacement가
    // 존재할 수 있음을 뜻하므로 false로 반환한다.
    planning_authority: &dyn PlanningAuthorityPort,
    // runtime은 mirror file compare-and-delete의 outbound boundary이다.
    runtime: &dyn ParallelModeRuntimePort,
    // workspace_dir은 삭제할 authority projection scope이다.
    workspace_dir: &str,
    // pool_root는 삭제할 mirror file scope이다.
    pool_root: &Path,
    // expected는 cleanup이 처음 검증한 exact lease generation이다.
    expected: &ParallelModeSlotLeaseSnapshot,
) -> bool {
    /*
    remove는 agent branch가 통합되고 slot worktree가 detached baseline으로 돌아간 뒤 호출된다.
    두 저장소 중 어느 하나라도 expected generation과 다르면 replacement ownership을 보존하고
    false를 반환한다.
    */
    let Ok(expected_body) = serde_json::to_string_pretty(expected) else {
        return false;
    };
    // missing mirror는 idempotent success이지만, 다른 body는 replacement이므로 삭제를 거부한다.
    if !runtime
        .remove_runtime_mirror_file_if_matches(
            pool_root,
            &slot_lease_relative_path(&expected.slot_id),
            &expected_body,
        )
        .unwrap_or(false)
    {
        return false;
    }
    planning_authority
        .remove_runtime_slot_lease_if_matches(workspace_dir, expected)
        .unwrap_or(false)
}

pub(in crate::application::service::parallel_mode) fn rollback_slot_lease_write_failure(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    workspace_dir: &str,
    pool_root: &Path,
    expected: &ParallelModeSlotLeaseSnapshot,
) -> bool {
    let authority_removed = planning_authority
        .remove_runtime_slot_lease_if_matches(workspace_dir, expected)
        .unwrap_or(false);
    let mirror_removed = serde_json::to_string_pretty(expected).is_ok_and(|expected_body| {
        runtime
            .remove_runtime_mirror_file_if_matches(
                pool_root,
                &slot_lease_relative_path(&expected.slot_id),
                &expected_body,
            )
            .unwrap_or(false)
    });
    authority_removed && mirror_removed
}

pub(in crate::application::service::parallel_mode) fn slot_lease_mirror_matches_or_missing(
    runtime: &dyn ParallelModeRuntimePort,
    pool_root: &Path,
    expected: &ParallelModeSlotLeaseSnapshot,
) -> bool {
    let Ok(expected_body) = serde_json::to_string_pretty(expected) else {
        return false;
    };
    runtime
        .read_runtime_mirror_optional(pool_root, &slot_lease_relative_path(&expected.slot_id))
        .is_ok_and(|body| body.as_deref().is_none_or(|body| body == expected_body))
}

pub(in crate::application::service::parallel_mode) fn orphaned_slot_lease_mirror_matches_identity_or_missing(
    runtime: &dyn ParallelModeRuntimePort,
    pool_root: &Path,
    slot_id: &str,
    branch_name: &str,
    worktree_path: &Path,
) -> bool {
    let Ok(body) =
        runtime.read_runtime_mirror_optional(pool_root, &slot_lease_relative_path(slot_id))
    else {
        return false;
    };
    body.is_none_or(|body| {
        serde_json::from_str::<ParallelModeSlotLeaseSnapshot>(&body).is_ok_and(|lease| {
            lease.slot_id == slot_id
                && lease.branch_name == branch_name
                && lease.worktree_path == worktree_path.display().to_string()
        })
    })
}

pub(in crate::application::service::parallel_mode) fn remove_orphaned_slot_lease_mirror_if_matches(
    runtime: &dyn ParallelModeRuntimePort,
    pool_root: &Path,
    slot_id: &str,
    branch_name: &str,
    worktree_path: &Path,
) -> bool {
    let relative = slot_lease_relative_path(slot_id);
    let Ok(body) = runtime.read_runtime_mirror_optional(pool_root, &relative) else {
        return false;
    };
    let Some(body) = body else {
        return true;
    };
    let Ok(lease) = serde_json::from_str::<ParallelModeSlotLeaseSnapshot>(&body) else {
        return false;
    };
    if lease.slot_id != slot_id
        || lease.branch_name != branch_name
        || lease.worktree_path != worktree_path.display().to_string()
    {
        return false;
    }
    runtime
        .remove_runtime_mirror_file_if_matches(pool_root, &relative, &body)
        .unwrap_or(false)
}

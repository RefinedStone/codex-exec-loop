// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::collections::BTreeMap;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::path::Path;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityRuntimeProjectionSnapshot;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::super::{AKRA_AGENT_BRANCH_PREFIX, DEFAULT_POOL_SIZE, POOL_BASELINE_BRANCH};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::{
    GitWorktreeRecord, SlotGitStatus, command_succeeds, current_branch_name,
    ensure_directory_exists, inspect_slot_git_status, reset_slot_worktree_to_akra,
    resolve_branch_head, resolve_pool_baseline_head, slot_id,
};

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn can_refresh_pool_baseline_from_workspace(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    repo_root: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    runtime_projection: &PlanningAuthorityRuntimeProjectionSnapshot,
) -> bool {
    runtime_projection.distributor_queue_records.is_empty()
        && runtime_projection.slot_leases.values().all(|lease| {
            matches!(
                lease.state,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running
            )
        })
        && current_branch_name(Path::new(repo_root)).is_some_and(|branch_name| {
            branch_name != POOL_BASELINE_BRANCH
                && !branch_name.starts_with(&format!("{AKRA_AGENT_BRANCH_PREFIX}/"))
        })
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn ensure_pool_baseline_branch(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    repo_root: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    reset_to_current_head: bool,
) -> Result<(String, bool), ()> {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if reset_to_current_head && let Some(head_sha) = resolve_branch_head(repo_root, "HEAD") {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let existed = resolve_branch_head(repo_root, POOL_BASELINE_BRANCH).is_some();
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if command_succeeds(
            "git",
            [
                "-C",
                repo_root,
                "branch",
                "-f",
                POOL_BASELINE_BRANCH,
                "HEAD",
            ],
        ) {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok((head_sha, !existed));
        }
    }

    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if let Some(baseline_head) = resolve_branch_head(repo_root, POOL_BASELINE_BRANCH) {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok((baseline_head, false));
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let remote_ref = format!("refs/remotes/origin/{POOL_BASELINE_BRANCH}");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let created = if command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "show-ref",
            "--verify",
            "--quiet",
            remote_ref.as_str(),
        ],
    ) {
        command_succeeds(
            "git",
            [
                "-C",
                repo_root,
                "branch",
                POOL_BASELINE_BRANCH,
                remote_ref.as_str(),
            ],
        )
    } else if command_succeeds("git", ["-C", repo_root, "rev-parse", "--verify", "HEAD"]) {
        command_succeeds(
            "git",
            ["-C", repo_root, "branch", POOL_BASELINE_BRANCH, "HEAD"],
        )
    } else {
        false
    };

    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !created {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Err(());
    }

    resolve_branch_head(repo_root, POOL_BASELINE_BRANCH)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|baseline_head| (baseline_head, true))
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .ok_or(())
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn provision_missing_slots(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    repo_root: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pool_root: &Path,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    worktree_records: &[GitWorktreeRecord],
) -> usize {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut provisioned_slots = 0;

    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let slot_path = pool_root.join(slot_id(slot_number));
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if worktree_records
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .iter()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .any(|record| record.path == slot_path)
            // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
            || slot_path.exists()
        {
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        }

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let Some(slot_parent) = slot_path.parent() else {
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        };
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if ensure_directory_exists(slot_parent).is_err() {
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        }

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let slot_path_string = slot_path.display().to_string();
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if command_succeeds(
            "git",
            [
                "-C",
                repo_root,
                "worktree",
                "add",
                "--detach",
                slot_path_string.as_str(),
                POOL_BASELINE_BRANCH,
            ],
        ) {
            provisioned_slots += 1;
        }
    }

    provisioned_slots
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn reset_reusable_detached_baseline_slots(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    repo_root: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pool_root: &Path,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    worktree_records: &[GitWorktreeRecord],
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    slot_leases: &BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
) -> usize {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let baseline_head = resolve_pool_baseline_head(repo_root).unwrap_or_default();
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if baseline_head.is_empty() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return 0;
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut reset_slots = 0;
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let slot_id = slot_id(slot_number);
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if slot_leases.contains_key(&slot_id) {
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        }
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let slot_path = pool_root.join(&slot_id);
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let Some(worktree_record) = worktree_records
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .iter()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .find(|record| record.path == slot_path)
        // 학습 주석: `else` 분기는 앞 조건이 실패했을 때 실행되어 흐름의 대안을 제공합니다.
        else {
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        };
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !worktree_record.detached {
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        }
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let slot_status = inspect_slot_git_status(&slot_path);
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if worktree_record.head_sha == baseline_head
            && slot_status.is_some_and(SlotGitStatus::is_clean_baseline)
        {
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        }
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if reset_slot_worktree_to_akra(&slot_path).succeeded() {
            reset_slots += 1;
        }
    }

    reset_slots
}

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

/*
학습 주석: pool baseline branch는 idle slot worktree들이 되돌아갈 기준점입니다. 이 함수는
현재 workspace HEAD로 baseline을 새로 잡아도 되는지를 결정합니다. distributor queue가
비어 있어야 하고, slot lease가 있더라도 아직 Leased/Running인 작업만 있어야 하며, 현재
workspace branch가 baseline branch나 agent branch가 아니어야 합니다.

이 조건은 baseline refresh가 진행 중인 통합 결과나 agent branch를 기준점으로 삼는 일을
막습니다. baseline은 pool의 공통 출발선이므로, queue/slot pipeline이 안정적인 순간에만
갱신할 수 있습니다.
*/
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

/*
학습 주석: reconcile은 pool baseline branch가 반드시 존재한다고 가정하고 slot worktree를
만듭니다. 이 함수는 그 branch를 찾거나 만듭니다. `reset_to_current_head`가 true이면 현재
HEAD로 baseline을 강제로 맞추고, 아니면 기존 local baseline, origin baseline, 마지막으로
현재 HEAD 순서로 기준점을 찾습니다.

반환값의 bool은 branch를 새로 만들었는지를 나타냅니다. 상위 reconcile summary는 이 값을
사용해 사용자가 방금 어떤 pool 구조 변화가 일어났는지 알 수 있게 합니다.
*/
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

/*
학습 주석: missing slot provisioning은 pool size만큼 정해진 slot path를 확인하고, 아직
git worktree inventory에도 없고 파일시스템에도 없는 slot만 새로 만듭니다. 이미 디렉터리가
있는데 git worktree가 아니라면 안전하게 덮어쓸 수 없으므로 provisioning하지 않고, 나중에
slot inspection에서 Blocked로 보여 줍니다.

새 slot은 `--detach POOL_BASELINE_BRANCH`로 만들어집니다. idle slot은 특정 branch checkout이
아니라 baseline commit에 매달린 중립 worktree여야 lease 획득 시 새 agent branch로 전환하기
쉽기 때문입니다.
*/
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

/*
학습 주석: detached baseline slot은 idle pool의 정상 형태지만, baseline branch가 갱신되면
기존 slot이 이전 commit에 머물 수 있습니다. 이 함수는 lease가 없는 detached slot만 검사해
현재 `POOL_BASELINE_BRANCH` head와 다르거나 clean하지 않으면 reset sequence를 실행합니다.

lease가 있는 slot은 agent 작업이 걸려 있을 수 있으므로 건드리지 않습니다. 이 경계가 있어야
reconcile이 pool 위생을 맞추면서도 실행 중인 병렬 작업을 방해하지 않습니다.
*/
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

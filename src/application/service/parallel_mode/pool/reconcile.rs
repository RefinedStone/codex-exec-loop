use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;

use super::super::{
    AKRA_AGENT_BRANCH_PREFIX, DEFAULT_POOL_SIZE, DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH,
    local_branch_ref, remote_tracking_branch_ref,
};
use super::{
    GitWorktreeRecord, SlotGitStatus, command_succeeds, current_branch_name,
    ensure_directory_exists, inspect_slot_git_status, reset_slot_worktree_to_akra,
    resolve_branch_head, resolve_pool_baseline_head, slot_id,
};

/*
reconcile은 pool baseline branch가 반드시 존재한다고 가정하고 slot worktree를 만든다. 이
함수는 표준 branch의 remote-tracking ref가 있으면 거기에서 local branch를 맞추고, fresh
repository처럼 local/remote 표준 branch가 모두 없으면 현재 workspace HEAD를 표준 branch로 seed한
뒤 origin에 push한다. local 표준 branch가 drift한 경우에는 remote-tracking ref로 되돌린다.

반환값의 bool은 branch를 새로 만들었는지를 나타낸다. 상위 reconcile summary는 이 값을
사용해 사용자가 방금 어떤 pool 구조 변화가 일어났는지 알 수 있게 한다.
*/
pub(super) fn ensure_pool_baseline_branch(repo_root: &str) -> Result<(String, bool), ()> {
    let remote_ref = remote_tracking_branch_ref(DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH);
    if command_succeeds(
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
        return sync_pool_baseline_branch_from_remote(repo_root, &remote_ref);
    }

    seed_pool_baseline_branch_from_workspace_head(repo_root, &remote_ref)
}

fn sync_pool_baseline_branch_from_remote(
    repo_root: &str,
    remote_ref: &str,
) -> Result<(String, bool), ()> {
    let remote_head = resolve_branch_head(repo_root, remote_ref).ok_or(())?;
    let local_head = resolve_branch_head(repo_root, POOL_BASELINE_BRANCH);
    if current_branch_name(Path::new(repo_root)).as_deref() == Some(POOL_BASELINE_BRANCH) {
        return if local_head.as_deref() == Some(remote_head.as_str()) {
            Ok((remote_head, false))
        } else {
            Err(())
        };
    }

    let existed = local_head.is_some();
    if !command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "branch",
            "-f",
            POOL_BASELINE_BRANCH,
            remote_ref,
        ],
    ) {
        return Err(());
    }

    Ok((remote_head, !existed))
}

fn seed_pool_baseline_branch_from_workspace_head(
    repo_root: &str,
    remote_ref: &str,
) -> Result<(String, bool), ()> {
    let current_branch = current_branch_name(Path::new(repo_root)).ok_or(())?;
    if current_branch.starts_with(&format!("{AKRA_AGENT_BRANCH_PREFIX}/")) {
        return Err(());
    }

    let workspace_head = resolve_branch_head(repo_root, "HEAD").ok_or(())?;
    let local_head = resolve_branch_head(repo_root, POOL_BASELINE_BRANCH);
    let existed = local_head.is_some();
    if current_branch != POOL_BASELINE_BRANCH
        && !command_succeeds(
            "git",
            [
                "-C",
                repo_root,
                "branch",
                "-f",
                POOL_BASELINE_BRANCH,
                "HEAD",
            ],
        )
    {
        return Err(());
    }

    let local_ref = local_branch_ref(POOL_BASELINE_BRANCH);
    let push_refspec = format!("{local_ref}:{local_ref}");
    if !command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "push",
            DEFAULT_PUSH_REMOTE_NAME,
            push_refspec.as_str(),
        ],
    ) {
        return Err(());
    }

    if !command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "update-ref",
            remote_ref,
            workspace_head.as_str(),
        ],
    ) {
        return Err(());
    }

    Ok((workspace_head, !existed))
}

/*
missing slot provisioning은 pool size만큼 정해진 slot path를 확인하고, 아직 git worktree
inventory에도 없고 파일시스템에도 없는 slot만 새로 만든다. 이미 디렉터리가 있는데 git
worktree가 아니라면 안전하게 덮어쓸 수 없으므로 provisioning하지 않고, 나중에 slot
inspection에서 Blocked로 보여 준다.

새 slot은 `--detach POOL_BASELINE_BRANCH`로 만들어진다. idle slot은 특정 branch checkout이
아니라 baseline commit에 매달린 중립 worktree여야 lease 획득 시 새 agent branch로 전환하기
쉽기 때문이다.
*/
pub(super) fn provision_missing_slots(
    repo_root: &str,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
    slot_leases: &BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
) -> usize {
    /*
    provisioning은 missing slot을 "pool이 소유한 disposable cache"로 취급한다. git worktree
    inventory에는 없지만 slot path만 남은 경우, lease가 없으면 이전 실패나 수동 삭제 뒤의 잔여물로
    보고 제거한 뒤 새 worktree를 만든다. lease가 남은 slot은 runtime projection이 아직 소유자를
    말하고 있으므로 여기서 삭제하지 않고 inspection이 split-brain 상태를 드러내게 둔다.
    */
    let mut provisioned_slots = 0;

    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_id = slot_id(slot_number);
        let slot_path = pool_root.join(&slot_id);
        if worktree_records
            .iter()
            .any(|record| record.path == slot_path)
        {
            /*
            worktree inventory에 있으면 이미 git이 관리하는 slot이다. stale/dirty detached 상태는
            provisioning이 아니라 reset_reusable_detached_baseline_slots가 hard reset + clean으로
            회수한다.
            */
            continue;
        }
        if slot_path.exists()
            && (slot_leases.contains_key(&slot_id) || remove_slot_residue(&slot_path).is_err())
        {
            continue;
        }

        let Some(slot_parent) = slot_path.parent() else {
            continue;
        };
        if ensure_directory_exists(slot_parent).is_err() {
            /*
            parent directory 생성 실패는 전체 reconcile을 중단하지 않고 해당 slot만 건너뛴다. pool
            board는 남은 slot 상태를 계속 계산할 수 있어야 하고, 실패한 path는 다음 reconcile에서
            다시 시도될 수 있다.
            */
            continue;
        }

        // git command boundary는 Path를 문자열로 넘겨야 하므로 여기서만 display string으로 변환한다.
        let slot_path_string = slot_path.display().to_string();
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
            /*
            성공 count만 올리는 이유는 reconcile summary가 "이번 tick에서 실제로 provision된 slot
            수"를 보여 주기 때문이다. 실패한 worktree add는 slot inspection의 blocked/missing
            상태로 남아 다음 refresh에서 다시 관찰된다.
            */
            provisioned_slots += 1;
        }
    }

    provisioned_slots
}

fn remove_slot_residue(slot_path: &Path) -> std::io::Result<()> {
    if slot_path.is_dir() {
        fs::remove_dir_all(slot_path)
    } else {
        fs::remove_file(slot_path)
    }
}

/*
detached baseline slot은 idle pool의 정상 형태지만, baseline branch가 갱신되면 기존 slot이
이전 commit에 머물 수 있다. 이 함수는 lease가 없는 detached slot만 검사해 현재
`POOL_BASELINE_BRANCH` head와 다르거나 clean하지 않으면 reset sequence를 실행한다.

lease가 있는 slot은 agent 작업이 걸려 있을 수 있으므로 건드리지 않는다. 이 경계가 있어야
reconcile이 pool 위생을 맞추면서도 실행 중인 병렬 작업을 방해하지 않는다.
*/
pub(super) fn reset_reusable_detached_baseline_slots(
    repo_root: &str,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
    slot_leases: &BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
) -> usize {
    /*
    reusable detached reset은 lease가 없는 idle 후보만 대상으로 한다. 이 함수는 "slot worktree가
    detached baseline이어야 한다"는 pool invariant를 baseline branch 이동 뒤에도 유지한다.
    active lease가 있는 slot은 branch/head가 baseline과 달라도 agent 작업일 수 있어 절대
    reset하지 않는다.
    */
    // baseline head를 못 찾으면 reset 기준이 없으므로 모든 slot을 관찰 전용으로 둔다.
    let baseline_head = resolve_pool_baseline_head(repo_root).unwrap_or_default();
    if baseline_head.is_empty() {
        return 0;
    }

    let mut reset_slots = 0;
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_id = slot_id(slot_number);
        if slot_leases.contains_key(&slot_id) {
            /*
            lease record가 있다는 것은 slot 상태 판단의 권위가 runtime projection에 있다는 뜻이다.
            파일시스템만 보고 reset하면 Running agent나 cleanup pending 작업의 산출물을 잃을 수
            있으므로, lease가 있는 slot은 이 helper의 책임 밖으로 둔다.
            */
            continue;
        }
        let slot_path = pool_root.join(&slot_id);
        let Some(worktree_record) = worktree_records
            .iter()
            .find(|record| record.path == slot_path)
        else {
            // inventory에 없는 slot은 provisioning/inspection 단계가 다루며, reset 대상이 아니다.
            continue;
        };
        if !worktree_record.detached {
            /*
            branch checkout 상태의 worktree는 idle pool invariant가 이미 깨진 상태일 수 있지만, 이
            함수는 detached baseline refresh 전용이다. branch checkout 불일치는 slot inspection이
            더 구체적인 recovery notice로 보여 주게 남겨 둔다.
            */
            continue;
        }
        // head SHA와 worktree dirtiness를 함께 봐야 stale baseline과 dirty idle slot을 모두 잡을 수 있다.
        let slot_status = inspect_slot_git_status(&slot_path);
        if worktree_record.head_sha == baseline_head
            && slot_status.is_some_and(SlotGitStatus::is_clean_baseline)
        {
            /*
            head가 현재 baseline이고 git status도 clean이면 reset은 불필요하다. 불필요한 hard
            reset/clean을 피하면 사용자가 보고 있는 idle worktree timestamp나 git metadata churn도
            줄어든다.
            */
            continue;
        }
        if reset_slot_worktree_to_akra(&slot_path).succeeded() {
            reset_slots += 1;
        }
    }

    reset_slots
}

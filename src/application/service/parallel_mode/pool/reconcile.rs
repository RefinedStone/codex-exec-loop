use std::collections::BTreeMap;
use std::path::Path;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityRuntimeProjectionSnapshot;
use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};

use super::super::{AKRA_AGENT_BRANCH_PREFIX, DEFAULT_POOL_SIZE, POOL_BASELINE_BRANCH};
use super::{
    GitWorktreeRecord, SlotGitStatus, command_succeeds, current_branch_name,
    ensure_directory_exists, inspect_slot_git_status, reset_slot_worktree_to_akra,
    resolve_branch_head, resolve_pool_baseline_head, slot_id,
};

/*
pool baseline branch는 idle slot worktree들이 되돌아갈 기준점이다. 이 함수는 현재 workspace
HEAD로 baseline을 새로 잡아도 되는지를 결정한다. distributor queue가 비어 있어야 하고,
slot lease가 있더라도 아직 Leased/Running인 작업만 있어야 하며, 현재 workspace branch가
baseline branch나 agent branch가 아니어야 한다.

이 조건은 baseline refresh가 진행 중인 통합 결과나 agent branch를 기준점으로 삼는 일을
막는다. baseline은 pool의 공통 출발선이므로, queue/slot pipeline이 안정적인 순간에만
갱신할 수 있다.
*/
pub(super) fn can_refresh_pool_baseline_from_workspace(
    repo_root: &str,
    runtime_projection: &PlanningAuthorityRuntimeProjectionSnapshot,
) -> bool {
    /*
    이 predicate는 "현재 workspace HEAD를 pool baseline으로 삼아도 되는가"를 single boolean으로
    압축한다. pool baseline을 잘못 움직이면 이후 새 agent branch가 잘못된 출발점에서
    만들어지므로, distributor queue가 비어 있고 현재 branch가 통합/agent 전용 branch가 아닌
    아주 제한된 상황만 허용한다.
    */
    runtime_projection.distributor_queue_records.is_empty()
        && runtime_projection.slot_leases.values().all(|lease| {
            /*
            여기서는 Leased/Running만 허용한다. 직관과 달리 lease가 아예 없거나 CleanupPending이
            섞인 상태보다, 아직 완료되지 않은 active lease만 있는 상태가 baseline refresh에 더
            안전하다. 완료/통합 대기 산출물이 queue나 cleanup 경계에 걸려 있으면 baseline 이동이
            그 산출물의 통합 여부 판단을 흐릴 수 있기 때문이다.
            */
            matches!(
                lease.state,
                ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running
            )
        })
        && current_branch_name(Path::new(repo_root)).is_some_and(|branch_name| {
            /*
            current branch guard는 사용자가 이미 `prerelease`나 `akra-agent/...`에 들어와 있는
            상황을 제외한다. baseline branch 위에서 baseline을 다시 잡는 것은 의미가 없고, agent
            branch 위에서 baseline을 잡으면 아직 distributor를 통과하지 않은 작업 결과가 pool
            전체 출발선이 될 수 있다.
            */
            branch_name != POOL_BASELINE_BRANCH
                && !branch_name.starts_with(&format!("{AKRA_AGENT_BRANCH_PREFIX}/"))
        })
}

/*
reconcile은 pool baseline branch가 반드시 존재한다고 가정하고 slot worktree를 만든다. 이
함수는 그 branch를 찾거나 만든다. `reset_to_current_head`가 true이면 현재 HEAD로 baseline을
강제로 맞추고, 아니면 기존 local baseline, origin baseline, 마지막으로 현재 HEAD 순서로
기준점을 찾는다.

반환값의 bool은 branch를 새로 만들었는지를 나타낸다. 상위 reconcile summary는 이 값을
사용해 사용자가 방금 어떤 pool 구조 변화가 일어났는지 알 수 있게 한다.
*/
pub(super) fn ensure_pool_baseline_branch(
    repo_root: &str,
    reset_to_current_head: bool,
) -> Result<(String, bool), ()> {
    if reset_to_current_head && let Some(head_sha) = resolve_branch_head(repo_root, "HEAD") {
        /*
        reset_to_current_head는 readiness/reconcile이 "지금 workspace HEAD를 새 baseline으로
        고정해도 된다"고 판단한 뒤에만 true로 들어온다. `branch -f prerelease HEAD`는 local
        baseline ref를 이동시키는 강한 작업이므로, 위 predicate를 통과한 경로에서만 사용되어야
        한다.
        */
        // existed flag는 "새 baseline을 만들었는가"와 "기존 baseline을 이동했는가"를 summary에서 구분하게 한다.
        let existed = resolve_branch_head(repo_root, POOL_BASELINE_BRANCH).is_some();
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
            return Ok((head_sha, !existed));
        }
    }

    if let Some(baseline_head) = resolve_branch_head(repo_root, POOL_BASELINE_BRANCH) {
        /*
        이미 local baseline branch가 있으면 그것이 가장 명확한 기준이다. remote나 HEAD fallback보다
        먼저 반환해, 사용자가 의도적으로 local prerelease를 고정해 둔 경우 reconcile이 그 선택을
        덮어쓰지 않게 한다.
        */
        return Ok((baseline_head, false));
    }

    /*
    local baseline이 없을 때만 origin/prerelease 또는 HEAD fallback으로 branch를 만든다. fresh
    clone에서는 remote tracking branch만 있을 수 있고, 테스트 repo나 초기 로컬 repo에서는 HEAD만
    있을 수 있다. created flag는 실제 `git branch` 성공 여부만 반영한다.
    */
    // remote ref를 명시 경로로 확인해 로컬 branch 이름과 remote tracking branch 이름을 혼동하지 않는다.
    let remote_ref = format!("refs/remotes/origin/{POOL_BASELINE_BRANCH}");
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

    if !created {
        return Err(());
    }

    resolve_branch_head(repo_root, POOL_BASELINE_BRANCH)
        .map(|baseline_head| (baseline_head, true))
        .ok_or(())
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
) -> usize {
    /*
    provisioning은 missing slot을 "만들 수 있는 경우에만" 만든다. slot path가 이미 존재하면
    그것이 빈 디렉터리인지, 사용자 파일인지, 깨진 worktree인지 여기서 추측하지 않는다. 그런
    애매한 상태는 inspection/reconcile board에서 operator recovery 대상으로 드러내는 쪽이 자동
    삭제보다 안전하다.
    */
    let mut provisioned_slots = 0;

    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_path = pool_root.join(slot_id(slot_number));
        if worktree_records
            .iter()
            .any(|record| record.path == slot_path)
            || slot_path.exists()
        {
            /*
            worktree inventory에 있거나 파일시스템 path가 이미 있으면 skip한다. 둘 중 하나만 true인
            경우도 중요하다. inventory에는 없지만 path가 있으면 git worktree가 아닌 충돌물이므로
            `git worktree add`로 덮지 않는다.
            */
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

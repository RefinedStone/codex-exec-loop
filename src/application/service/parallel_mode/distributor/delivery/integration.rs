// delivery 하위 모듈의 공통 타입과 helper를 끌어와, integration worktree 검증이
// queue record 차단, slot lease context, Git 상태 조회와 같은 주변 흐름을 같은 어휘로 다루게 한다.
use super::*;

/*
integration worktree readiness는 cherry-pick 직전의 마지막 안전 게이트이다.
distributor는 source branch commit을 integration branch에 로컬 cherry-pick하므로, 현재 worktree가
정확히 `DISTRIBUTOR_INTEGRATION_BRANCH`에 있어야 하고 staged/unstaged/rebase/cherry-pick
메타데이터가 없어야 한다.

조건을 만족하지 않으면 queue record를 즉시 blocked로 바꾼다. 그래야 오케스트레이터가
같은 head를 계속 밀어붙이지 않고, supervisor가 operator에게 어떤 worktree 정리가 필요한지
표시할 수 있다.
*/
pub(super) fn ensure_distributor_integration_worktree_ready(
    // 준비 실패를 발견했을 때 queue record를 blocked로 저장하는 영속 포트이다.
    planning_authority: &dyn PlanningAuthorityPort,
    // block 기록에 repo root, pool root, lease id를 같이 남기기 위한 slot 해석 결과이다.
    resolution: &WorkspaceSlotLeaseResolution,
    // 이 함수가 직접 상태를 바꾸는 delivery 대상 queue record이다.
    record: &mut ParallelModeDistributorQueueRecord,
    // cherry-pick을 실행할 별도 integration worktree의 루트 경로이다.
    integration_repo_root: &str,
) -> Result<(), String> {
    // 다른 브랜치에서 cherry-pick하면 integration branch가 아닌 곳에 source patch를
    // 밀어 넣게 되므로, 브랜치 이름 검사는 dirty check보다 먼저 실패시켜야 한다.
    if current_branch_name(Path::new(integration_repo_root)).as_deref()
        != Some(DISTRIBUTOR_INTEGRATION_BRANCH)
    {
        // 같은 message를 queue block record와 함수 오류 양쪽에 사용해 UI와 caller 로그가
        // 서로 다른 원인을 말하지 않게 한다.
        let message = format!(
            "integration worktree must be checked out to `{DISTRIBUTOR_INTEGRATION_BRANCH}` before cherry-pick delivery"
        );
        // lease를 포함해 차단하면 supervisor가 어떤 slot worktree를 operator가 정리해야
        // 하는지 추적할 수 있다. 저장 실패는 `?`로 caller에게 올려 delivery loop를 멈춘다.
        let _ = block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        // block 상태 저장 뒤에도 오류를 반환해야, 호출자가 실제 cherry-pick 단계로
        // 계속 진행하지 않고 현재 queue item 처리를 끝낼 수 있다.
        return Err(message);
    }

    // Git 상태 조회 자체가 실패한 경우에는 clean 여부를 판단할 수 없으므로, 안전한
    // 기본값으로 delivery를 막고 사람이 worktree를 점검하게 한다.
    let Some(status) = inspect_slot_git_status(Path::new(integration_repo_root)) else {
        // status detail이 없는 실패라서 고정 문구만 남긴다. 이 문구는 block reason과
        // 함수 오류 문자열로 그대로 공유된다.
        let message = "integration worktree git status could not be inspected".to_string();
        // 저장되는 block record는 retry loop가 같은 head를 반복 delivery하지 못하게 하는
        // 제어 신호이기도 한다.
        let _ = block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        // status를 못 읽은 상태에서는 이후 `is_ready_for_integration` 판정도 불가능하므로
        // 즉시 실패로 빠져나간다.
        return Err(message);
    };
    // readiness는 unstaged/staged 변경뿐 아니라 rebase, merge, cherry-pick 같은 Git
    // operation metadata까지 포함한다. 남은 작업이 있으면 새 cherry-pick은 기존 복구 상태를 덮을 수 있다.
    if !status.is_ready_for_integration() {
        // detail label을 message에 넣어 단순히 "not clean"이 아니라 어떤 Git 상태가
        // 막고 있는지 TUI에서 바로 보이게 한다.
        let message = format!(
            "integration worktree must be clean before cherry-pick delivery: {}",
            status.detail_label()
        );
        // dirty worktree는 자동 복구보다 사람 판단이 필요한 상태라 queue record를 blocked로
        // 굳혀 다음 distributor tick이 같은 위험한 cherry-pick을 반복하지 못하게 한다.
        let _ = block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        // block 저장을 완료한 뒤 `Err`를 반환해 상위 delivery 함수가 실패 메시지를
        // 그대로 notice나 history에 연결할 수 있게 한다.
        return Err(message);
    }

    // 여기까지 도달했다는 것은 branch와 cleanliness gate가 모두 통과했다는 뜻이다.
    // 실제 cherry-pick 실행은 caller가 맡고, 이 함수는 readiness만 보증한다.
    Ok(())
}

/*
`git cherry`는 patch-id 기준으로 commit이 base branch에 이미 반영되었는지 확인할
수 있다. source commit SHA가 직접 조상으로 들어간 것은 아니어도 같은 patch가 이미 적용된
경우가 있으므로, distributor는 중복 cherry-pick 대신 "patch-equivalent already integrated"로
기록할 수 있다.

이 함수가 false를 반환한다고 해서 오류는 아니다. 단지 아직 patch-equivalent 증거가 없으니
일반 cherry-pick 경로로 진행해야 한다는 뜻이다.
*/
pub(super) fn commit_patch_equivalent_in_branch(
    // patch 동등성을 확인할 Git repository 루트이다.
    repo_root: &str,
    // source commit이 이미 반영되었는지 비교할 integration/base branch이다.
    base_branch: &str,
    // delivery queue가 가져온 source branch head commit이다.
    commit_sha: &str,
) -> bool {
    // `git cherry <base> <commit>`은 각 candidate commit 앞에 `+` 또는 `-`를 붙인다.
    // 명령 실행이 실패하면 evidence가 없는 것으로 보고 일반 cherry-pick 경로에 맡긴다.
    let Some(cherry_output) = run_command(
        "git",
        ["-C", repo_root, "cherry", base_branch, commit_sha],
        None,
    ) else {
        // false는 "동등하지 않다"가 아니라 "동등하다는 증거를 얻지 못했다"에 가깝다.
        // 그래서 caller는 실패로 기록하지 않고 cherry-pick을 계속 시도할 수 있다.
        return false;
    };

    cherry_output
        // Git 출력은 한 줄당 한 commit 판정이므로 줄 단위로 검사한다.
        .lines()
        // 앞쪽 공백을 걷어낸 뒤 `-`로 시작하면 patch-id가 base에 이미 존재한다는
        // 뜻이라 중복 cherry-pick을 건너뛸 수 있다.
        .any(|line| line.trim_start().starts_with('-'))
}

pub(super) fn fetch_integration_remote_branch(repo_root: &str) -> bool {
    command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "fetch",
            "--quiet",
            DEFAULT_PUSH_REMOTE_NAME,
            &format!(
                "{DISTRIBUTOR_INTEGRATION_BRANCH}:{}",
                remote_tracking_branch_ref(
                    DEFAULT_PUSH_REMOTE_NAME,
                    DISTRIBUTOR_INTEGRATION_BRANCH
                )
            ),
        ],
    )
}

pub(super) fn commit_patch_equivalent_in_remote_integration_branch(
    repo_root: &str,
    commit_sha: &str,
) -> bool {
    let remote_branch =
        remote_branch_name(DEFAULT_PUSH_REMOTE_NAME, DISTRIBUTOR_INTEGRATION_BRANCH);
    branch_is_integrated_into(repo_root, commit_sha, &remote_branch)
        || commit_patch_equivalent_in_branch(repo_root, &remote_branch, commit_sha)
}

pub(super) fn reset_integration_branch_to_remote(repo_root: &str) -> bool {
    let remote_branch =
        remote_branch_name(DEFAULT_PUSH_REMOTE_NAME, DISTRIBUTOR_INTEGRATION_BRANCH);
    command_succeeds(
        "git",
        ["-C", repo_root, "reset", "--hard", remote_branch.as_str()],
    )
}

/*
cherry-pick이 충돌하면 Git은 conflicted file 목록을 index에 남긴다. 이 함수는
unmerged file만 수집해 queue record의 conflict_files에 저장할 짧은 목록으로 바꾼다.
그 목록은 supervisor orchestrator status와 blocked notice에서 사용자가 어디를 봐야 하는지
알려 주는 복구 단서가 된다.
*/
pub(super) fn collect_cherry_pick_conflict_files(repo_root: &str) -> Vec<String> {
    run_command(
        "git",
        ["-C", repo_root, "diff", "--name-only", "--diff-filter=U"],
        None,
    )
    // 충돌 파일 수집은 차단 message를 보강하는 보조 정보라, Git 명령 실패만으로
    // delivery 실패 이유를 바꾸지 않고 빈 목록으로 낮춘다.
    .unwrap_or_default()
    // `git diff --name-only` 출력은 파일마다 한 줄이라 그대로 record 목록의 후보가 된다.
    .lines()
    // 줄 끝 개행이나 주변 공백이 history에 들어가지 않도록 정규화한다.
    .map(str::trim)
    // 빈 줄은 사용자가 열어볼 수 있는 파일 경로가 아니므로 record에서 제외한다.
    .filter(|line| !line.is_empty())
    // queue record가 owned `String` 목록을 저장하므로 Git 출력 버퍼에서 독립된 값을 만든다.
    .map(str::to_string)
    // caller가 conflict files를 block reason과 별도 필드에 같이 넣을 수 있도록 Vec로 확정한다.
    .collect::<Vec<_>>()
}

/*
conflict suffix는 block message에 붙는 사람이 읽는 요약이다. 충돌 파일이 없으면
불필요한 빈 "conflicts" 문구를 붙이지 않고, 목록이 있으면 한 줄에 합쳐 TUI notice와 queue
record history가 같은 형식으로 원인을 보여 주게 한다.
*/
pub(super) fn format_conflict_file_suffix(conflict_files: &[String]) -> String {
    // 충돌 파일을 찾지 못한 경우에는 원래 block reason만 남겨, 비어 있는 suffix가
    // notice 문장을 어색하게 만들지 않도록 한다.
    if conflict_files.is_empty() {
        String::new()
    } else {
        // 여러 conflict file을 한 줄 suffix로 접어 queue history, notice, test assertion이
        // 모두 같은 사람이 읽는 형식을 공유하게 한다.
        format!(" / conflicts: {}", conflict_files.join(", "))
    }
}

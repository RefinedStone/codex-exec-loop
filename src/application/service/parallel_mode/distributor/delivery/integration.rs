// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::*;

/*
학습 주석: integration worktree readiness는 cherry-pick 직전의 마지막 안전 게이트입니다.
distributor는 source branch commit을 integration branch에 로컬 cherry-pick하므로, 현재 worktree가
정확히 `DISTRIBUTOR_INTEGRATION_BRANCH`에 있어야 하고 staged/unstaged/rebase/cherry-pick
메타데이터가 없어야 합니다.

조건을 만족하지 않으면 queue record를 즉시 blocked로 바꿉니다. 그래야 오케스트레이터가
같은 head를 계속 밀어붙이지 않고, supervisor가 operator에게 어떤 worktree 정리가 필요한지
표시할 수 있습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn ensure_distributor_integration_worktree_ready(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    resolution: &WorkspaceSlotLeaseResolution,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    record: &mut ParallelModeDistributorQueueRecord,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    integration_repo_root: &str,
) -> Result<(), String> {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if current_branch_name(Path::new(integration_repo_root)).as_deref()
        != Some(DISTRIBUTOR_INTEGRATION_BRANCH)
    {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let message = format!(
            "integration worktree must be checked out to `{DISTRIBUTOR_INTEGRATION_BRANCH}` before cherry-pick delivery"
        );
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let _ = block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Err(message);
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let Some(status) = inspect_slot_git_status(Path::new(integration_repo_root)) else {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let message = "integration worktree git status could not be inspected".to_string();
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let _ = block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Err(message);
    };
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !status.is_ready_for_integration() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let message = format!(
            "integration worktree must be clean before cherry-pick delivery: {}",
            status.detail_label()
        );
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let _ = block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Err(message);
    }

    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(())
}

/*
학습 주석: `git cherry`는 patch-id 기준으로 commit이 base branch에 이미 반영되었는지 확인할
수 있습니다. source commit SHA가 직접 조상으로 들어간 것은 아니어도 같은 patch가 이미 적용된
경우가 있으므로, distributor는 중복 cherry-pick 대신 "patch-equivalent already integrated"로
기록할 수 있습니다.

이 함수가 false를 반환한다고 해서 오류는 아닙니다. 단지 아직 patch-equivalent 증거가 없으니
일반 cherry-pick 경로로 진행해야 한다는 뜻입니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn commit_patch_equivalent_in_branch(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    repo_root: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    base_branch: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    commit_sha: &str,
) -> bool {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let Some(cherry_output) = run_command(
        "git",
        ["-C", repo_root, "cherry", base_branch, commit_sha],
        None,
    ) else {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return false;
    };

    cherry_output
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .lines()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .any(|line| line.trim_start().starts_with('-'))
}

/*
학습 주석: cherry-pick이 충돌하면 Git은 conflicted file 목록을 index에 남깁니다. 이 함수는
unmerged file만 수집해 queue record의 conflict_files에 저장할 짧은 목록으로 바꿉니다.
그 목록은 supervisor orchestrator status와 blocked notice에서 사용자가 어디를 봐야 하는지
알려 주는 복구 단서가 됩니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn collect_cherry_pick_conflict_files(repo_root: &str) -> Vec<String> {
    run_command(
        "git",
        ["-C", repo_root, "diff", "--name-only", "--diff-filter=U"],
        None,
    )
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .unwrap_or_default()
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .lines()
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .map(str::trim)
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .filter(|line| !line.is_empty())
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .map(str::to_string)
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .collect::<Vec<_>>()
}

/*
학습 주석: conflict suffix는 block message에 붙는 사람이 읽는 요약입니다. 충돌 파일이 없으면
불필요한 빈 "conflicts" 문구를 붙이지 않고, 목록이 있으면 한 줄에 합쳐 TUI notice와 queue
record history가 같은 형식으로 원인을 보여 주게 합니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn format_conflict_file_suffix(conflict_files: &[String]) -> String {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if conflict_files.is_empty() {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        String::new()
    } else {
        format!(" / conflicts: {}", conflict_files.join(", "))
    }
}

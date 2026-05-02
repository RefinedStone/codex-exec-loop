// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::*;

/*
학습 주석: distributor delivery의 첫 GitHub 단계는 slot agent branch를 원격에 push하는
것입니다. PR 생성과 원격 리뷰 흐름은 push된 branch가 있어야 가능하므로, capability를 먼저
검사하고 queue record를 Pushing으로 저장한 뒤 push를 시도합니다.

push 실패나 remote 미준비는 block record로 전환합니다. 이때 session detail도 pushing으로
기록해 supervisor가 "현재 통합 큐가 remote push 단계에서 멈췄다"는 사실을 보여 줍니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn distributor_push_source_branch(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    resolution: &WorkspaceSlotLeaseResolution,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    record: &mut ParallelModeDistributorQueueRecord,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    github_automation: &dyn GithubAutomationPort,
) -> Result<String, String> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let repo_root = resolution.context.repo_root.clone();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let capabilities = github_automation.inspect_capabilities(&repo_root);
    record.github_capabilities = Some(capabilities.clone());
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !capabilities.push_ready() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "push capability is unavailable for distributor delivery: {}",
                capabilities.push_remote.summary()
            ),
        );
    }

    record.queue_state = ParallelModeQueueItemState::Pushing;
    record.integration_note = format!(
        "distributor is pushing `{}` to `{DEFAULT_PUSH_REMOTE_NAME}`",
        record.branch_name
    );
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let _ = record_pushing_session_detail(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if let Err(error) = github_automation.push_branch(&repo_root, &record.branch_name, false) {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "source branch `{}` could not be pushed to `{DEFAULT_PUSH_REMOTE_NAME}`: {error}",
                record.branch_name
            ),
        );
    }

    record.integration_note = format!(
        "source branch pushed to `{DEFAULT_PUSH_REMOTE_NAME}` and is waiting for pull request ensure"
    );
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;

    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(format!(
        "distributor pushed source branch / agent: {} / branch: {}",
        record.agent_id, record.branch_name
    ))
}

/*
학습 주석: source branch가 원격에 올라간 뒤에는 integration branch를 대상으로 하는 PR을
보장합니다. GitHub CLI/auth 준비 상태를 별도로 검사하는 이유는 push 가능성과 PR 조작 가능성이
다른 capability이기 때문입니다. push는 되었지만 GitHub automation이 없으면 record를 blocked로
남겨, 이후 auth가 복구되었을 때 retryable block으로 다시 queue에 올릴 수 있습니다.

ensure_pull_request는 새 PR을 만들 수도 있고 기존 PR을 재사용할 수도 있습니다. 성공하면
PR 번호와 URL을 queue record에 저장해 이후 readiness 검사와 closing 단계가 같은 PR을 추적합니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn distributor_ensure_pull_request(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    resolution: &WorkspaceSlotLeaseResolution,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    record: &mut ParallelModeDistributorQueueRecord,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    github_automation: &dyn GithubAutomationPort,
) -> Result<String, String> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let repo_root = resolution.context.repo_root.clone();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let capabilities = github_automation.inspect_capabilities(&repo_root);
    record.github_capabilities = Some(capabilities.clone());
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !capabilities.github_ready() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let capability_summary = if capabilities.gh_binary.state
            != crate::domain::parallel_mode::ParallelModeCapabilityState::Ready
        {
            capabilities.gh_binary.summary()
        } else {
            capabilities.gh_auth.summary()
        };
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "source branch was pushed but GitHub automation is unavailable: {capability_summary}"
            ),
        );
    }

    record.queue_state = ParallelModeQueueItemState::PrPending;
    record.integration_note =
        "source branch pushed and pull request ensure is in progress".to_string();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let _ = record_pr_pending_session_detail(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let pull_request = match github_automation.ensure_pull_request(
        &repo_root,
        DISTRIBUTOR_INTEGRATION_BRANCH,
        &record.branch_name,
        &build_distributor_pull_request_title(record),
        &build_distributor_pull_request_body(record),
    ) {
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(pull_request) => pull_request,
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Err(error) => {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return block_distributor_queue_record(
                planning_authority,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!(
                    "pull request ensure failed for `{}`: {error}",
                    record.branch_name
                ),
            );
        }
    };

    record.pull_request_number = Some(pull_request.number);
    record.pull_request_url = Some(pull_request.url.clone());
    record.integration_note = format!(
        "pull request #{} is open for `{}`",
        pull_request.number, record.branch_name
    );
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;

    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(format!(
        "distributor ensured pull request / agent: {} / pr: #{}",
        record.agent_id, pull_request.number
    ))
}

/*
학습 주석: PR readiness 검사는 cherry-pick 전에 사람이 볼 수 있는 GitHub 상태가 기대와
맞는지 확인하는 gate입니다. PR이 열려 있어야 하고, draft가 아니어야 하며, base branch가
integration branch이고 head branch가 queue record의 source branch와 같아야 합니다.

이 검사는 GitHub의 실제 merge 버튼을 누르기 위한 준비가 아니라, distributor가 로컬에서
integration branch에 반영하기 전에 remote 협업 표면이 drift하지 않았는지 검증하는 단계입니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn distributor_check_pull_request_merge_readiness(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    resolution: &WorkspaceSlotLeaseResolution,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    record: &mut ParallelModeDistributorQueueRecord,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    github_automation: &dyn GithubAutomationPort,
) -> Result<String, String> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let Some(pr_number) = record.pull_request_number else {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            "pull request metadata is missing after PR ensure".to_string(),
        );
    };

    record.queue_state = ParallelModeQueueItemState::MergePending;
    record.integration_note =
        format!("pull request #{pr_number} is open and merge readiness is being checked");
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let _ = record_merge_pending_session_detail(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let repo_root = resolution.context.repo_root.clone();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let pull_request = match github_automation.inspect_pull_request(&repo_root, pr_number) {
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(pull_request) => pull_request,
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Err(error) => {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return block_distributor_queue_record(
                planning_authority,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!("pull request #{pr_number} could not be inspected: {error}"),
            );
        }
    };

    record.pull_request_url = Some(pull_request.url.clone());
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !pull_request.state.eq_ignore_ascii_case("open") {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "pull request #{} is not open (`{}`)",
                pull_request.number, pull_request.state
            ),
        );
    }
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if pull_request.is_draft {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!("pull request #{} is still a draft", pull_request.number),
        );
    }
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if pull_request.base_branch != DISTRIBUTOR_INTEGRATION_BRANCH {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "pull request #{} targets `{}` instead of `{DISTRIBUTOR_INTEGRATION_BRANCH}`",
                pull_request.number, pull_request.base_branch
            ),
        );
    }
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if pull_request.head_branch != record.branch_name {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "pull request #{} head drifted from `{}` to `{}`",
                pull_request.number, record.branch_name, pull_request.head_branch
            ),
        );
    }

    record.integration_note = format!(
        "pull request #{} is open and ready for integration into `{DISTRIBUTOR_INTEGRATION_BRANCH}`",
        pull_request.number
    );
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;

    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(format!(
        "distributor verified pull request readiness / agent: {} / pr: #{}",
        record.agent_id, pull_request.number
    ))
}

/*
학습 주석: PR 제목은 distributor가 만든 자동 PR임을 짧게 드러내고 task title을 중심에 둡니다.
queue record의 task_id보다 title을 쓰는 이유는 GitHub PR 목록에서 사람이 어떤 작업 결과인지
빠르게 구분해야 하기 때문입니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn build_distributor_pull_request_title(record: &ParallelModeDistributorQueueRecord) -> String {
    format!("supersession: {}", record.task_title.trim())
}

/*
학습 주석: PR body에는 distributor가 나중에 복구하거나 사람이 확인할 수 있는 provenance를
넣습니다. agent, task id, branch, commit, validation, official refresh 결과를 남기면
queue record 없이 GitHub 화면만 봐도 이 PR이 어떤 slot 결과를 대변하는지 추적할 수 있습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn build_distributor_pull_request_body(record: &ParallelModeDistributorQueueRecord) -> String {
    format!(
        "Automated distributor delivery for a supersession result.\n\n- Agent: {}\n- Task ID: {}\n- Branch: `{}`\n- Commit: `{}`\n- Validation: {}\n- Official refresh: {}",
        record.agent_id,
        record.task_id,
        record.effective_source_branch(),
        record.effective_source_commit_sha(),
        record.validation_summary.trim(),
        record.authority_refresh_outcome.trim()
    )
}

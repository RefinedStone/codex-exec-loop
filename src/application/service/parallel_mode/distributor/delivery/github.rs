// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::*;

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

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn build_distributor_pull_request_title(record: &ParallelModeDistributorQueueRecord) -> String {
    format!("supersession: {}", record.task_title.trim())
}

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

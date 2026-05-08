use super::*;

/*
distributor delivery의 첫 GitHub 단계는 slot agent branch를 원격에 push하는
것이다. PR 생성과 원격 리뷰 흐름은 push된 branch가 있어야 가능하므로, capability를 먼저
검사하고 queue record를 Pushing으로 저장한 뒤 push를 시도한다.

push 실패나 remote 미준비는 block record로 전환한다. 이때 session detail도 pushing으로
기록해 supervisor가 "현재 통합 큐가 remote push 단계에서 멈췄다"는 사실을 보여 준다.
*/
pub(super) fn distributor_push_source_branch(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
) -> Result<String, String> {
    // capability snapshot을 record에 보관해 blocked supervisor 화면이 "왜 push 불가인지" 즉시 설명하게 한다.
    let repo_root = resolution.context.repo_root.clone();
    let capabilities = github_automation.inspect_capabilities(&repo_root);
    record.github_capabilities = Some(capabilities.clone());
    if !capabilities.push_ready() {
        // push가 준비되지 않은 경우는 일시 장애일 수 있으므로 queue record를 blocked retry 지점으로 남긴다.
        return block_distributor_queue_record(
            planning_authority,
            runtime,
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
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;
    // session detail은 queue record보다 operator-facing timeline에 가깝기 때문에 실패해도 delivery를 중단하지 않는다.
    let _ = record_pushing_session_detail(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    if let Err(error) = github_automation.push_branch(&repo_root, &record.branch_name, false) {
        // 실제 push 실패는 remote/auth/network 상태와 연결되므로 같은 block path로 복구 가능하게 만든다.
        return block_distributor_queue_record(
            planning_authority,
            runtime,
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
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;

    Ok(format!(
        "distributor pushed source branch / agent: {} / branch: {}",
        record.agent_id, record.branch_name
    ))
}

/*
source branch가 원격에 올라간 뒤에는 integration branch를 대상으로 하는 PR을
보장한다. GitHub CLI/auth 준비 상태를 별도로 검사하는 이유는 push 가능성과 PR 조작 가능성이
다른 capability이기 때문이다. push는 되었지만 GitHub automation이 없으면 record를 blocked로
남겨, 이후 auth가 복구되었을 때 retryable block으로 다시 queue에 올릴 수 있다.

ensure_pull_request는 새 PR을 만들 수도 있고 기존 PR을 재사용할 수도 있다. 성공하면
PR 번호와 URL을 queue record에 저장해 이후 readiness 검사와 closing 단계가 같은 PR을 추적한다.
*/
pub(super) fn distributor_ensure_pull_request(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
) -> Result<String, String> {
    // PR 조작은 push와 다른 capability라 여기서 gh binary/auth 상태를 다시 읽고 기록한다.
    let repo_root = resolution.context.repo_root.clone();
    let capabilities = github_automation.inspect_capabilities(&repo_root);
    record.github_capabilities = Some(capabilities.clone());
    if !capabilities.github_ready() {
        // binary 부재와 auth 부재 중 더 직접적인 원인을 골라 block note를 짧고 실행 가능하게 만든다.
        let capability_summary = if capabilities.gh_binary.state
            != crate::domain::parallel_mode::ParallelModeCapabilityState::Ready
        {
            capabilities.gh_binary.summary()
        } else {
            capabilities.gh_auth.summary()
        };
        return block_distributor_queue_record(
            planning_authority,
            runtime,
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
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;
    // PR pending detail은 "remote branch는 있음, GitHub PR 표면을 만드는 중"인 중간 상태를 노출한다.
    let _ = record_pr_pending_session_detail(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    // ensure는 idempotent boundary이다. retry 시 기존 PR을 재사용해야 queue가 중복 PR을 만들지 않는다.
    let pull_request = match github_automation.ensure_pull_request(
        &repo_root,
        DISTRIBUTOR_INTEGRATION_BRANCH,
        &record.branch_name,
        &build_distributor_pull_request_title(record),
        &build_distributor_pull_request_body(record),
    ) {
        Ok(pull_request) => pull_request,
        Err(error) => {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
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
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;

    Ok(format!(
        "distributor ensured pull request / agent: {} / pr: #{}",
        record.agent_id, pull_request.number
    ))
}

/*
PR readiness 검사는 cherry-pick 전에 사람이 볼 수 있는 GitHub 상태가 기대와
맞는지 확인하는 gate이다. PR이 열려 있어야 하고, draft가 아니어야 하며, base branch가
integration branch이고 head branch가 queue record의 source branch와 같아야 한다.

이 검사는 GitHub의 실제 merge 버튼을 누르기 위한 준비가 아니라, distributor가 로컬에서
integration branch에 반영하기 전에 remote 협업 표면이 drift하지 않았는지 검증하는 단계이다.
*/
pub(super) fn distributor_check_pull_request_merge_readiness(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
) -> Result<String, String> {
    let Some(pr_number) = record.pull_request_number else {
        // PR 번호가 없다면 이전 ensure 단계의 durable write가 깨진 것이므로 operator recovery로 넘긴다.
        return block_distributor_queue_record(
            planning_authority,
            runtime,
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
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;
    // merge pending detail은 이후 local integration/cherry-pick 단계로 넘어가기 전의 원격 검증 상태이다.
    let _ = record_merge_pending_session_detail(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    let repo_root = resolution.context.repo_root.clone();
    // readiness는 queue record의 저장 값이 아니라 GitHub 현재 상태를 다시 읽어 drift를 잡는다.
    let pull_request = match github_automation.inspect_pull_request(&repo_root, pr_number) {
        Ok(pull_request) => pull_request,
        Err(error) => {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!("pull request #{pr_number} could not be inspected: {error}"),
            );
        }
    };

    record.pull_request_url = Some(pull_request.url.clone());
    if !pull_request.state.eq_ignore_ascii_case("open") {
        // closed/merged PR은 source branch와 queue state가 이미 외부에서 변했을 수 있어 자동 통합을 멈춘다.
        return block_distributor_queue_record(
            planning_authority,
            runtime,
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
    if pull_request.is_draft {
        // draft PR은 사람이 아직 통합 표면을 확정하지 않은 신호라 distributor가 로컬 반영하지 않는다.
        return block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!("pull request #{} is still a draft", pull_request.number),
        );
    }
    if pull_request.base_branch != DISTRIBUTOR_INTEGRATION_BRANCH {
        // base drift는 다른 integration lane으로 향한 PR일 수 있으므로 현재 distributor queue에서 통합하지 않는다.
        return block_distributor_queue_record(
            planning_authority,
            runtime,
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
    if pull_request.head_branch != record.branch_name {
        // head drift는 queue record가 가리키는 agent result와 PR content가 달라졌다는 강한 불일치이다.
        return block_distributor_queue_record(
            planning_authority,
            runtime,
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
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;

    Ok(format!(
        "distributor verified pull request readiness / agent: {} / pr: #{}",
        record.agent_id, pull_request.number
    ))
}

/*
PR 제목은 distributor가 만든 자동 PR임을 짧게 드러내고 task title을 중심에 둔다.
queue record의 task_id보다 title을 쓰는 이유는 GitHub PR 목록에서 사람이 어떤 작업 결과인지
빠르게 구분해야 하기 때문이다.
*/
fn build_distributor_pull_request_title(record: &ParallelModeDistributorQueueRecord) -> String {
    format!("supersession: {}", record.task_title.trim())
}

/*
PR body에는 distributor가 나중에 복구하거나 사람이 확인할 수 있는 provenance를
넣는다. agent, task id, branch, commit, validation, official refresh 결과를 남기면
queue record 없이 GitHub 화면만 봐도 이 PR이 어떤 slot 결과를 대변하는지 추적할 수 있다.
*/
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

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::*;

// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod github;
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod integration;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use self::github::{
    distributor_check_pull_request_merge_readiness, distributor_ensure_pull_request,
    distributor_push_source_branch,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use self::integration::{
    collect_cherry_pick_conflict_files, commit_patch_equivalent_in_branch,
    ensure_distributor_integration_worktree_ready, format_conflict_file_suffix,
};

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn process_distributor_queue_record(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    workspace_dir: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pool_root: &Path,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    record: &mut ParallelModeDistributorQueueRecord,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    github_automation: &dyn GithubAutomationPort,
) -> Result<Vec<String>, String> {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !Path::new(&record.worktree_path).exists() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(vec![block_distributor_queue_record(
            planning_authority,
            workspace_dir,
            pool_root,
            None,
            record,
            "source worktree is missing; distributor cannot continue".to_string(),
        )?]);
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let resolution = match resolve_workspace_slot_lease(planning_authority, &record.worktree_path) {
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(Some(resolution)) => resolution,
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(None) => {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(vec![block_distributor_queue_record(
                planning_authority,
                workspace_dir,
                pool_root,
                None,
                record,
                "slot lease disappeared before distributor integration".to_string(),
            )?]);
        }
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Err(error) => {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(vec![block_distributor_queue_record(
                planning_authority,
                workspace_dir,
                pool_root,
                None,
                record,
                format!("slot lease could not be resolved for distributor delivery: {error}"),
            )?]);
        }
    };

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut notices = Vec::new();
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if matches!(
        record.queue_state,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeQueueItemState::Queued
            // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
            | ParallelModeQueueItemState::Pushing
            // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
            | ParallelModeQueueItemState::PrPending
            // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
            | ParallelModeQueueItemState::MergePending
            // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
            | ParallelModeQueueItemState::Integrating
    ) {
        notices.push(distributor_push_source_branch(
            planning_authority,
            &resolution,
            record,
            github_automation,
        )?);
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(notices);
        }

        notices.push(distributor_ensure_pull_request(
            planning_authority,
            &resolution,
            record,
            github_automation,
        )?);
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(notices);
        }

        notices.push(distributor_check_pull_request_merge_readiness(
            planning_authority,
            &resolution,
            record,
            github_automation,
        )?);
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(notices);
        }

        notices.push(distributor_integrate_branch(
            planning_authority,
            &resolution,
            record,
            github_automation,
        )?);
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(notices);
        }
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let cleanup_notice =
        distributor_cleanup_integrated_slot(planning_authority, &resolution, record)?;
    notices.push(cleanup_notice);
    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(notices)
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn distributor_integrate_branch(
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
    let source_branch = record.effective_source_branch();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let source_commit_sha = record.effective_source_commit_sha();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let slot_status = inspect_slot_git_status(&resolution.workspace_path).ok_or_else(|| {
        format!(
            "slot `{}` git status could not be inspected for distributor delivery",
            resolution.lease.slot_id
        )
    })?;
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if slot_status.has_pending_operation {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "slot `{}` has pending merge or rebase metadata and cannot be integrated",
                resolution.lease.slot_id
            ),
        );
    }

    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if current_branch_name(&resolution.workspace_path).as_deref() != Some(source_branch.as_str()) {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "slot `{}` is no longer checked out to `{}`",
                resolution.lease.slot_id, source_branch
            ),
        );
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let current_head = resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
        format!(
            "slot `{}` workspace head could not be resolved for distributor delivery",
            resolution.lease.slot_id
        )
    })?;
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if current_head != source_commit_sha {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "branch head drifted from expected commit `{}` to `{}`",
                short_sha(&source_commit_sha),
                short_sha(&current_head)
            ),
        );
    }

    record.queue_state = ParallelModeQueueItemState::Integrating;
    record.integration_note = match record.pull_request_number {
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        Some(pr_number) => format!(
            "pull request #{pr_number} is ready and distributor is integrating the queued branch into {DISTRIBUTOR_INTEGRATION_BRANCH}"
        ),
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        None => format!(
            "distributor is integrating the queued branch into {DISTRIBUTOR_INTEGRATION_BRANCH}"
        ),
    };
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let _ = record_integrating_session_detail(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let integration_repo_root = resolution.context.canonical_repo_root.display().to_string();

    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !branch_is_integrated_into(
        &integration_repo_root,
        &source_branch,
        DISTRIBUTOR_INTEGRATION_BRANCH,
    ) {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if let Err(notice) = ensure_distributor_integration_worktree_ready(
            planning_authority,
            resolution,
            record,
            &integration_repo_root,
        ) {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(notice);
        }

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if commit_patch_equivalent_in_branch(
            &integration_repo_root,
            DISTRIBUTOR_INTEGRATION_BRANCH,
            &source_commit_sha,
        ) {
            record.integration_state = "done".to_string();
            record.integration_note = format!(
                "commit `{}` from `{}` is already patch-equivalent in `{DISTRIBUTOR_INTEGRATION_BRANCH}`",
                short_sha(&source_commit_sha),
                source_branch
            );
            record.updated_at = current_timestamp();
            write_distributor_queue_record(
                planning_authority,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                record,
            )?;
        } else if !command_succeeds(
            "git",
            [
                "-C",
                integration_repo_root.as_str(),
                "cherry-pick",
                source_commit_sha.as_str(),
            ],
        ) {
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let conflict_files = collect_cherry_pick_conflict_files(&integration_repo_root);
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let _ = command_succeeds(
                "git",
                [
                    "-C",
                    integration_repo_root.as_str(),
                    "cherry-pick",
                    "--abort",
                ],
            );
            record.conflict_files = conflict_files.clone();
            record.recovery_note = Some(
                "resolve the conflict manually or update the source branch, then rerun orchestration"
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .to_string(),
            );
            record.integration_state = "blocked".to_string();
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return block_distributor_queue_record(
                planning_authority,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!(
                    "commit `{}` from `{}` could not cherry-pick into `{DISTRIBUTOR_INTEGRATION_BRANCH}` cleanly{}",
                    short_sha(&source_commit_sha),
                    source_branch,
                    format_conflict_file_suffix(&conflict_files),
                ),
            );
        }

        record.integration_state = "done".to_string();
        record.integration_note = format!(
            "commit `{}` from `{}` cherry-picked into `{DISTRIBUTOR_INTEGRATION_BRANCH}`",
            short_sha(&source_commit_sha),
            source_branch
        );
        record.updated_at = current_timestamp();
        write_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            record,
        )?;
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let repo_root = integration_repo_root;
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if let Err(error) =
        github_automation.push_integration_branch(&repo_root, DISTRIBUTOR_INTEGRATION_BRANCH)
    {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "`{DISTRIBUTOR_INTEGRATION_BRANCH}` could not be pushed to `{DEFAULT_PUSH_REMOTE_NAME}`: {error}"
            ),
        );
    }
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if let Some(pr_number) = record.pull_request_number {
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
                    format!(
                        "pull request #{pr_number} could not be reloaded before close: {error}"
                    ),
                );
            }
        };
        record.pull_request_url = Some(pull_request.url.clone());
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if pull_request.state.eq_ignore_ascii_case("open")
            && let Err(error) = github_automation.close_pull_request(&repo_root, pr_number)
        {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return block_distributor_queue_record(
                planning_authority,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!("pull request #{pr_number} could not be closed: {error}"),
            );
        }
    }

    record.queue_state = ParallelModeQueueItemState::Cleaning;
    record.integration_note = format!(
        "branch integrated into {DISTRIBUTOR_INTEGRATION_BRANCH}, pushed to origin, and the slot is entering cleanup"
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
        "distributor integrated queue head into {DISTRIBUTOR_INTEGRATION_BRANCH} / slot: {} / agent: {} / commit: {}",
        resolution.lease.slot_id,
        resolution.lease.agent_id,
        short_sha(&record.commit_sha)
    ))
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn distributor_cleanup_integrated_slot(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    resolution: &WorkspaceSlotLeaseResolution,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<String, String> {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if resolution.lease.state == ParallelModeSlotLeaseState::Running {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut cleanup_pending_lease = resolution.lease.clone();
        cleanup_pending_lease.state = ParallelModeSlotLeaseState::CleanupPending;
        write_slot_lease(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &cleanup_pending_lease,
        )?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let _ = record_cleanup_pending_session_detail(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &cleanup_pending_lease,
        );
    }

    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !cleanup_slot(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease.slot_id,
        &resolution.workspace_path,
        &resolution.lease.branch_name,
    ) {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "slot `{}` cleanup failed after distributor delivery",
                resolution.lease.slot_id
            ),
        );
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let _ = record_cleaned_session_detail(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease,
    );
    record.queue_state = ParallelModeQueueItemState::Done;
    record.integration_note = format!(
        "branch integrated into {DISTRIBUTOR_INTEGRATION_BRANCH}, GitHub delivery completed, and the slot returned to idle"
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
        "distributor returned slot to idle / slot: {} / agent: {}",
        resolution.lease.slot_id, resolution.lease.agent_id
    ))
}

use super::*;

// delivery는 GitHub-facing 단계와 local integration 단계를 나눠 각 boundary의 실패 복구를 독립시킨다.
mod github;
mod integration;

use self::github::{
    distributor_check_pull_request_merge_readiness, distributor_ensure_pull_request,
    distributor_push_source_branch,
};
use self::integration::{
    collect_cherry_pick_conflict_files, commit_patch_equivalent_in_branch,
    commit_patch_equivalent_in_remote_integration_branch,
    ensure_distributor_integration_worktree_ready, fetch_integration_remote_branch,
    format_conflict_file_suffix, reset_integration_branch_to_remote,
};

/*
이 함수는 queue head 하나를 end-to-end로 delivery하는 상태 기계이다.
입력 record는 planning authority에 저장된 durable queue item이고, 각 단계는 record 상태를
갱신한 뒤 다음 단계로 넘어간다. 순서는 source branch push, PR 준비/검사, integration
branch 반영, slot cleanup이다.

각 단계 뒤에 Blocked 상태를 확인하고 즉시 반환하는 구조가 중요하다. delivery 중 어느
단계라도 사람이 개입해야 하는 문제가 생기면, 뒤 단계가 잘못 실행되지 않고 현재까지의
notice만 TUI에 표시된다.
*/
pub(super) fn process_distributor_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    workspace_dir: &str,
    pool_root: &Path,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
) -> Result<Vec<String>, String> {
    if !Path::new(&record.worktree_path).exists() {
        // source worktree가 없으면 lease를 신뢰할 수 없어 delivery를 시작하지 않고 durable block으로 남긴다.
        return Ok(vec![block_distributor_queue_record(
            planning_authority,
            runtime,
            workspace_dir,
            pool_root,
            None,
            record,
            "source worktree is missing; distributor cannot continue".to_string(),
        )?]);
    }

    // delivery는 queue record의 worktree path를 runtime lease로 되검증해 stale queue item을 차단한다.
    let resolution = match resolve_workspace_slot_lease(planning_authority, &record.worktree_path) {
        Ok(Some(resolution)) => resolution,
        Ok(None) => {
            return Ok(vec![block_distributor_queue_record(
                planning_authority,
                runtime,
                workspace_dir,
                pool_root,
                None,
                record,
                "slot lease disappeared before distributor integration".to_string(),
            )?]);
        }
        Err(error) => {
            return Ok(vec![block_distributor_queue_record(
                planning_authority,
                runtime,
                workspace_dir,
                pool_root,
                None,
                record,
                format!("slot lease could not be resolved for distributor delivery: {error}"),
            )?]);
        }
    };

    let mut notices = Vec::new();
    // queued부터 integrating까지는 GitHub/integration 단계가 idempotent하게 재시도될 수 있는 delivery window이다.
    if matches!(
        record.queue_state,
        ParallelModeQueueItemState::Queued
            | ParallelModeQueueItemState::Pushing
            | ParallelModeQueueItemState::PrPending
            | ParallelModeQueueItemState::MergePending
            | ParallelModeQueueItemState::Integrating
    ) {
        notices.push(distributor_push_source_branch(
            planning_authority,
            runtime,
            &resolution,
            record,
            github_automation,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            // block은 operator recovery contract라 뒤 단계를 실행하지 않고 현재 notices만 반환한다.
            return Ok(notices);
        }

        notices.push(distributor_ensure_pull_request(
            planning_authority,
            runtime,
            &resolution,
            record,
            github_automation,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            return Ok(notices);
        }

        notices.push(distributor_check_pull_request_merge_readiness(
            planning_authority,
            runtime,
            &resolution,
            record,
            github_automation,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            return Ok(notices);
        }

        notices.push(distributor_integrate_branch(
            planning_authority,
            runtime,
            &resolution,
            record,
            github_automation,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            return Ok(notices);
        }
    }

    // 이미 Cleaning인 record도 이 경로로 들어와 slot cleanup만 재시도할 수 있다.
    let cleanup_notice =
        distributor_cleanup_integrated_slot(planning_authority, runtime, &resolution, record)?;
    notices.push(cleanup_notice);
    Ok(notices)
}

/*
integration 단계는 슬롯 branch의 특정 commit을 integration worktree에 반영한다.
먼저 slot worktree가 예상 branch와 예상 head commit에 머물러 있는지 확인한다. 이 확인이
없으면 agent가 낸 결과가 아닌 다른 commit을 cherry-pick할 수 있다.

이미 patch-equivalent commit이 integration branch에 있으면 중복 cherry-pick 대신 완료로
기록한다. 그렇지 않으면 cherry-pick을 시도하고, conflict가 나면 abort 후 conflict file
목록과 recovery note를 record에 남겨 사용자가 복구할 수 있게 한다. 성공 후에는 integration
branch를 push하고, source PR이 있으면 닫은 뒤 Cleaning 상태로 넘어간다.
*/
fn distributor_integrate_branch(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
) -> Result<String, String> {
    let source_branch = record.effective_source_branch();
    let source_commit_sha = record.effective_source_commit_sha();
    // slot git status는 cherry-pick 전에 pending merge/rebase metadata를 잡는 첫 guard이다.
    let slot_status = inspect_slot_git_status(&resolution.workspace_path).ok_or_else(|| {
        format!(
            "slot `{}` git status could not be inspected for distributor delivery",
            resolution.lease.slot_id
        )
    })?;
    if slot_status.has_pending_operation {
        return block_distributor_queue_record(
            planning_authority,
            runtime,
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

    if current_branch_name(&resolution.workspace_path).as_deref() != Some(source_branch.as_str()) {
        // branch drift는 queue record가 가리키는 agent output과 실제 worktree가 달라졌다는 뜻이다.
        return block_distributor_queue_record(
            planning_authority,
            runtime,
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

    // commit SHA까지 고정해 force-push나 추가 commit이 섞인 source branch를 자동 통합하지 않는다.
    let current_head = resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
        format!(
            "slot `{}` workspace head could not be resolved for distributor delivery",
            resolution.lease.slot_id
        )
    })?;
    if current_head != source_commit_sha {
        return block_distributor_queue_record(
            planning_authority,
            runtime,
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
        Some(pr_number) => format!(
            "pull request #{pr_number} is ready and distributor is integrating the queued branch into {DISTRIBUTOR_INTEGRATION_BRANCH}"
        ),
        None => format!(
            "distributor is integrating the queued branch into {DISTRIBUTOR_INTEGRATION_BRANCH}"
        ),
    };
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;
    // session detail은 queue record와 별개로 supervisor detail timeline을 갱신하므로 실패를 무시한다.
    let _ = record_integrating_session_detail(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    // integration은 canonical repo root에서 수행해 slot worktree가 아닌 prerelease worktree 기준으로 반영한다.
    let integration_repo_root = resolution.context.canonical_repo_root.display().to_string();

    if !branch_is_integrated_into(
        &integration_repo_root,
        &source_branch,
        DISTRIBUTOR_INTEGRATION_BRANCH,
    ) {
        if let Err(notice) = ensure_distributor_integration_worktree_ready(
            planning_authority,
            runtime,
            resolution,
            record,
            &integration_repo_root,
        ) {
            return Ok(notice);
        }

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
                runtime,
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
            // conflict file list는 abort 전에 수집해야 Git index가 충돌 path를 아직 알고 있다.
            let conflict_files = collect_cherry_pick_conflict_files(&integration_repo_root);
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
                    .to_string(),
            );
            record.integration_state = "blocked".to_string();
            return block_distributor_queue_record(
                planning_authority,
                runtime,
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
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            record,
        )?;
    }

    let repo_root = integration_repo_root;
    if let Err(error) =
        github_automation.push_integration_branch(&repo_root, DISTRIBUTOR_INTEGRATION_BRANCH)
    {
        if fetch_integration_remote_branch(&repo_root)
            && commit_patch_equivalent_in_remote_integration_branch(&repo_root, &source_commit_sha)
            && reset_integration_branch_to_remote(&repo_root)
        {
            record.integration_state = "done".to_string();
            record.integration_note = format!(
                "remote `{DEFAULT_PUSH_REMOTE_NAME}/{DISTRIBUTOR_INTEGRATION_BRANCH}` already contains commit `{}` from `{}`; local integration branch was aligned to remote after push rejection",
                short_sha(&source_commit_sha),
                source_branch
            );
            record.updated_at = current_timestamp();
            write_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                record,
            )?;
        } else {
            // local integration이 성공해도 remote push 실패는 operator가 다시 밀어야 하는 delivery block이다.
            return block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!(
                    "`{DISTRIBUTOR_INTEGRATION_BRANCH}` could not be pushed to `{DEFAULT_PUSH_REMOTE_NAME}`: {error}"
                ),
            );
        }
    }
    if let Some(pr_number) = record.pull_request_number {
        // PR close 전 다시 inspect해 URL을 최신화하고, 이미 닫힌 PR은 close 호출을 생략한다.
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
                    format!(
                        "pull request #{pr_number} could not be reloaded before close: {error}"
                    ),
                );
            }
        };
        record.pull_request_url = Some(pull_request.url.clone());
        if pull_request.state.eq_ignore_ascii_case("open")
            && let Err(error) = github_automation.close_pull_request(&repo_root, pr_number)
        {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
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
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;

    Ok(format!(
        "distributor integrated queue head into {DISTRIBUTOR_INTEGRATION_BRANCH} / slot: {} / agent: {} / commit: {}",
        resolution.lease.slot_id,
        resolution.lease.agent_id,
        short_sha(&record.commit_sha)
    ))
}

/*
delivery가 integration branch 반영까지 끝나면 슬롯 worktree를 다시 idle pool로
돌려야 한다. Running lease는 먼저 CleanupPending으로 저장해 supervisor가 "통합은 끝났고
반환 대기 중"인 상태를 볼 수 있게 한다. 실제 `cleanup_slot`이 성공하면 session detail에
cleaned 이력을 남기고 queue record를 Done으로 닫는다.

cleanup 실패는 통합 실패가 아니라 slot 반환 실패이다. 그래서 record를 block 처리해
operator가 worktree/branch 상태를 복구한 뒤 같은 queue item을 다시 진행할 수 있게 한다.
*/
fn distributor_cleanup_integrated_slot(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<String, String> {
    if resolution.lease.state == ParallelModeSlotLeaseState::Running {
        // Running lease를 먼저 CleanupPending으로 바꿔 통합 완료와 slot 반환 사이의 중간 상태를 보존한다.
        let mut cleanup_pending_lease = resolution.lease.clone();
        cleanup_pending_lease.state = ParallelModeSlotLeaseState::CleanupPending;
        write_slot_lease(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &cleanup_pending_lease,
        )?;
        let _ = record_cleanup_pending_session_detail(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &cleanup_pending_lease,
        );
    }

    if !cleanup_slot(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease.slot_id,
        &resolution.workspace_path,
        &resolution.lease.branch_name,
    ) {
        // cleanup 실패는 integration 결과를 되돌리지 않고, slot 반환 문제로 block 처리한다.
        return block_distributor_queue_record(
            planning_authority,
            runtime,
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

    // cleaned detail은 queue Done 상태와 별도로 session history에 slot 반환 완료를 남긴다.
    let _ = record_cleaned_session_detail(
        planning_authority,
        runtime,
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
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;

    Ok(format!(
        "distributor returned slot to idle / slot: {} / agent: {}",
        resolution.lease.slot_id, resolution.lease.agent_id
    ))
}

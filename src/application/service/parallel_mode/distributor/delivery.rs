use super::*;

#[cfg(test)]
use std::cell::RefCell;

use crate::application::service::parallel_mode::git_sequence::{GitCommandStep, run_git_sequence};
use crate::application::service::parallel_mode::{
    ParallelModeAutomationPermit, derive_integration_worktree_path,
};
// delivery는 GitHub-facing 단계와 local integration 단계를 나눠 각 boundary의 실패 복구를 독립시킨다.
mod github;
mod integration;

use self::github::{
    distributor_prepare_pull_request_or_skip, distributor_push_source_branch,
    distributor_recheck_pull_request_before_integration_push,
};
use self::integration::{
    PreparedIntegrationState, collect_cherry_pick_conflict_files, fetch_integration_remote_branch,
    format_conflict_file_suffix, prepare_distributor_integration_worktree,
};

#[cfg(test)]
thread_local! {
    static BEFORE_DISTRIBUTOR_CLEANUP_LOCK_HOOK: RefCell<Option<Box<dyn FnOnce()>>> =
        RefCell::new(None);
}

#[cfg(test)]
pub(in crate::application::service::parallel_mode) fn install_before_distributor_cleanup_lock_hook(
    hook: impl FnOnce() + 'static,
) {
    BEFORE_DISTRIBUTOR_CLEANUP_LOCK_HOOK.with(|slot| {
        let previous = slot.borrow_mut().replace(Box::new(hook));
        assert!(
            previous.is_none(),
            "distributor cleanup test hook already installed"
        );
    });
}

#[cfg(test)]
fn run_before_distributor_cleanup_lock_hook() {
    BEFORE_DISTRIBUTOR_CLEANUP_LOCK_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_before_distributor_cleanup_lock_hook() {}

/*
이 함수는 queue head 하나를 end-to-end로 delivery하는 상태 기계이다.
입력 record는 planning authority에 저장된 durable queue item이고, 각 단계는 record 상태를
갱신한 뒤 다음 단계로 넘어간다. 순서는 source branch push, PR 준비/검사, integration
branch 반영, slot cleanup이다.

각 단계 뒤에 Blocked 상태를 확인하고 즉시 반환하는 구조가 중요하다. delivery 중 어느
단계라도 사람이 개입해야 하는 문제가 생기면, 뒤 단계가 잘못 실행되지 않고 현재까지의
notice만 TUI에 표시된다.
*/
// Keep the authority/runtime/GitHub boundaries and the two independent permits
// explicit at this orchestration boundary; bundling them would hide ownership.
#[allow(clippy::too_many_arguments)]
pub(super) fn process_distributor_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    workspace_dir: &str,
    pool_root: &Path,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
    automation_permit: Option<&ParallelModeAutomationPermit>,
    claim_permit: &DistributorQueueHeadClaimPermit,
    integration_target_oid: &str,
    autonomous_delivery_allowed: &Result<bool, String>,
) -> Result<Vec<String>, String> {
    if !runtime.path_exists(Path::new(&record.worktree_path)) {
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
    let resolution = match resolve_workspace_slot_lease_with_runtime(
        runtime,
        planning_authority,
        &record.worktree_path,
    ) {
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

    let delivery_window = matches!(
        record.queue_state,
        ParallelModeQueueItemState::Queued
            | ParallelModeQueueItemState::Pushing
            | ParallelModeQueueItemState::PrPending
            | ParallelModeQueueItemState::MergePending
            | ParallelModeQueueItemState::Integrating
    );
    if delivery_window
        && let Err(detail) = validate_frozen_source_workspace(runtime, &resolution, record)
    {
        return Ok(vec![block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            detail,
        )?]);
    }

    claim_permit.renew("delivery target validation")?;
    if let Err(detail) = validate_distributor_delivery_target(
        runtime,
        github_automation,
        &resolution.context.repo_root,
        record,
    ) {
        return Ok(vec![block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            detail,
        )?]);
    }

    let mut notices = Vec::new();
    let mut cleanup_target_oid = integration_target_oid.to_string();
    // queued부터 integrating까지는 GitHub/integration 단계가 idempotent하게 재시도될 수 있는 delivery window이다.
    if delivery_window {
        notices.push(distributor_push_source_branch(
            planning_authority,
            runtime,
            &resolution,
            record,
            github_automation,
            automation_permit,
            claim_permit,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            // block은 operator recovery contract라 뒤 단계를 실행하지 않고 현재 notices만 반환한다.
            return Ok(notices);
        }

        notices.extend(distributor_prepare_pull_request_or_skip(
            planning_authority,
            runtime,
            &resolution,
            record,
            github_automation,
            automation_permit,
            claim_permit,
            autonomous_delivery_allowed,
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
            automation_permit,
            claim_permit,
            autonomous_delivery_allowed,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            return Ok(notices);
        }
        cleanup_target_oid = record.integration_commit_sha.clone().ok_or_else(|| {
            "verified integration result is missing before slot cleanup".to_string()
        })?;
    }

    // 이미 Cleaning인 record도 이 경로로 들어와 slot cleanup만 재시도할 수 있다.
    let cleanup_notice = distributor_cleanup_integrated_slot(
        planning_authority,
        runtime,
        &resolution,
        record,
        automation_permit,
        claim_permit,
        &cleanup_target_oid,
    )?;
    notices.push(cleanup_notice);
    Ok(notices)
}

fn validate_frozen_source_workspace(
    runtime: &dyn ParallelModeRuntimePort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
    let source_branch = record.effective_source_branch();
    let source_commit_sha = record.effective_source_commit_sha();
    let lease = &resolution.lease;
    let expected_session_key = lease.session_key();

    if record.slot_id != lease.slot_id
        || record.session_key != expected_session_key
        || record.agent_id != lease.agent_id
        || record.task_id != lease.task_id
        || record.branch_name != lease.branch_name
        || source_branch != lease.branch_name
    {
        return Err(format!(
            "slot `{}` lease identity no longer matches frozen distributor source metadata",
            lease.slot_id
        ));
    }

    let slot_status = inspect_slot_git_status_with_runtime(runtime, &resolution.workspace_path)
        .map_err(|error| {
            format!(
                "slot `{}` git status could not be inspected for distributor delivery: {error}",
                lease.slot_id
            )
        })?;
    if slot_status.has_pending_operation {
        return Err(format!(
            "slot `{}` has pending merge or rebase metadata and cannot be delivered",
            lease.slot_id
        ));
    }
    if !slot_status.is_clean_for_frozen_delivery() {
        return Err(format!(
            "slot `{}` has staged, unstaged, nonignored-untracked, or pending changes after the frozen commit and cannot be delivered",
            lease.slot_id
        ));
    }
    if current_branch_name(runtime, &resolution.workspace_path).as_deref()
        != Some(source_branch.as_str())
    {
        return Err(format!(
            "slot `{}` is no longer checked out to frozen source branch `{source_branch}`",
            lease.slot_id
        ));
    }

    let current_head = resolve_workspace_head_sha_with_runtime(runtime, &resolution.workspace_path)
        .ok_or_else(|| {
            format!(
                "slot `{}` workspace head could not be resolved for distributor delivery",
                lease.slot_id
            )
        })?;
    if current_head != source_commit_sha {
        return Err(format!(
            "branch head drifted from expected commit `{}` to `{}` before source delivery",
            short_sha(&source_commit_sha),
            short_sha(&current_head)
        ));
    }

    resolve_linear_distributor_source_range(
        runtime,
        &resolution.context.repo_root,
        &record.source_base_commit_sha,
        &source_commit_sha,
    )
    .map(|_| ())
    .map_err(|detail| format!("frozen source commit range is invalid: {detail}"))
}

fn block_if_automation_epoch_closed(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    automation_permit: Option<&ParallelModeAutomationPermit>,
    side_effect: &str,
) -> Result<Option<String>, String> {
    if automation_permit.is_none_or(ParallelModeAutomationPermit::is_active) {
        return Ok(None);
    }
    block_distributor_queue_record(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        Some(&resolution.lease),
        record,
        format!(
            "parallel automation epoch closed before distributor {side_effect}; no new side effect was started"
        ),
    )
    .map(Some)
}

fn block_if_delivery_target_changed(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
    claim_permit: &DistributorQueueHeadClaimPermit,
    side_effect: &str,
) -> Result<Option<String>, String> {
    claim_permit.renew(&format!(
        "delivery target verification before {side_effect}"
    ))?;
    let Err(detail) = validate_distributor_delivery_target(
        runtime,
        github_automation,
        &resolution.context.repo_root,
        record,
    ) else {
        return Ok(None);
    };

    block_distributor_queue_record(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        Some(&resolution.lease),
        record,
        format!("delivery target verification failed before {side_effect}: {detail}"),
    )
    .map(Some)
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
#[allow(clippy::too_many_arguments)]
fn distributor_integrate_branch(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
    automation_permit: Option<&ParallelModeAutomationPermit>,
    claim_permit: &DistributorQueueHeadClaimPermit,
    autonomous_delivery_allowed: &Result<bool, String>,
) -> Result<String, String> {
    let source_branch = record.effective_source_branch();
    let source_base_commit_sha = record.source_base_commit_sha.clone();
    let source_commit_sha = record.effective_source_commit_sha();
    // slot git status는 cherry-pick 전에 pending merge/rebase metadata를 잡는 첫 guard이다.
    let slot_status = inspect_slot_git_status_with_runtime(runtime, &resolution.workspace_path)
        .map_err(|error| {
            format!(
                "slot `{}` git status could not be inspected for distributor delivery: {error}",
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
    if !slot_status.is_clean_for_frozen_delivery() {
        return block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "slot `{}` has staged, unstaged, nonignored-untracked, or pending changes after the frozen commit and cannot be integrated or cleaned",
                resolution.lease.slot_id
            ),
        );
    }

    if current_branch_name(runtime, &resolution.workspace_path).as_deref()
        != Some(source_branch.as_str())
    {
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
    let current_head = resolve_workspace_head_sha_with_runtime(runtime, &resolution.workspace_path)
        .ok_or_else(|| {
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

    if let Some(notice) = block_if_automation_epoch_closed(
        planning_authority,
        runtime,
        resolution,
        record,
        automation_permit,
        "dedicated integration worktree preparation",
    )? {
        return Ok(notice);
    }

    record.queue_state = ParallelModeQueueItemState::Integrating;
    let target = record
        .delivery_target
        .clone()
        .expect("validated delivery target must be present");
    let credential_redacted_push_url = target
        .credential_redacted_push_url
        .clone()
        .expect("validated delivery target must contain a credential-redacted push URL");
    let integration_branch = target.integration_branch.clone();
    record.integration_note = match record.pull_request_number {
        Some(pr_number) => format!(
            "pull request #{pr_number} is ready and distributor is integrating the queued branch into {}",
            integration_branch
        ),
        None => format!(
            "distributor is integrating the queued branch into {}",
            integration_branch
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
    let _ = record_integrating_session_detail(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    claim_permit.renew("dedicated integration worktree preparation")?;
    let integration_mutation_lock = match acquire_pool_mutation_lock(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
    ) {
        Ok(mutation_lock) => mutation_lock,
        Err(error) => {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!(
                    "dedicated integration worktree preparation could not acquire the pool mutation lock: {error}"
                ),
            );
        }
    };
    if let Err(error) = integration_mutation_lock.verify_pool_root(&resolution.context.pool_root) {
        return block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!("dedicated integration worktree mutation permit is invalid: {error}"),
        );
    }
    let prepared_integration = match prepare_distributor_integration_worktree(
        planning_authority,
        runtime,
        github_automation,
        resolution,
        record,
        automation_permit,
        &integration_mutation_lock,
    ) {
        Ok(integration_repo_root) => integration_repo_root,
        Err(notice) => return Ok(notice),
    };
    let integration_repo_root = prepared_integration.repo_root;

    claim_permit.renew("remote frozen source verification")?;
    let remote_source_head = match github_automation.remote_branch_head_for_delivery_target(
        &resolution.context.repo_root,
        &target.push_remote,
        &credential_redacted_push_url,
        &source_branch,
    ) {
        Ok(remote_source_head) => remote_source_head,
        Err(error) => {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!("remote source branch head could not be verified: {error}"),
            );
        }
    };
    if remote_source_head.as_deref() != Some(source_commit_sha.as_str()) {
        return block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "remote source branch `{}/{}` no longer points to frozen commit `{}`",
                target.push_remote,
                source_branch,
                short_sha(&source_commit_sha)
            ),
        );
    }

    if let Some(notice) = block_if_automation_epoch_closed(
        planning_authority,
        runtime,
        resolution,
        record,
        automation_permit,
        "cherry-pick",
    )? {
        return Ok(notice);
    }

    if prepared_integration.state == PreparedIntegrationState::Fresh {
        claim_permit.renew("frozen source range comparison and cherry-pick")?;
        if let Err(error) =
            integration_mutation_lock.verify_pool_root(&resolution.context.pool_root)
        {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!("cherry-pick lost its pool mutation permit: {error}"),
            );
        }
        if let Err(error) = runtime.ensure_git_execution_safe(Path::new(&integration_repo_root)) {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!("cherry-pick was blocked by Git execution configuration: {error}"),
            );
        }
        let cherry_states =
            match distributor_source_cherry_states(runtime, &integration_repo_root, "HEAD", record)
            {
                Ok(states) => states,
                Err(detail) => {
                    return block_distributor_queue_record(
                        planning_authority,
                        runtime,
                        &resolution.context.repo_root,
                        &resolution.context.pool_root,
                        Some(&resolution.lease),
                        record,
                        format!("frozen source commit range could not be compared: {detail}"),
                    );
                }
            };
        let source_commit_count = cherry_states.len();
        let pending_commits = cherry_states
            .into_iter()
            .filter_map(|(sha, equivalent)| (!equivalent).then_some(sha))
            .collect::<Vec<_>>();

        if pending_commits.is_empty() {
            record.integration_note = format!(
                "all {source_commit_count} frozen source commits from `{source_branch}` are already patch-equivalent in `{integration_branch}`"
            );
        } else {
            let mut cherry_pick_args = vec![
                "-C".to_string(),
                integration_repo_root.clone(),
                "cherry-pick".to_string(),
            ];
            cherry_pick_args.extend(pending_commits.iter().cloned());
            let cherry_pick = run_git_sequence(
                runtime,
                "cherry-pick frozen distributor source range",
                vec![GitCommandStep::new(
                    "cherry-pick reviewed source commits",
                    cherry_pick_args,
                )],
            );
            if !cherry_pick.succeeded() {
                // conflict file list는 dedicated worktree에 보존된 index에서 수집한다.
                let conflict_files =
                    collect_cherry_pick_conflict_files(runtime, &integration_repo_root);
                record.conflict_files = conflict_files.clone();
                record.recovery_note = Some(
                    "inspect the dedicated integration worktree and resolve or abort the conflict manually before retry"
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
                        "frozen source range `{}`..`{}` from `{}` could not cherry-pick into `{}` cleanly{}",
                        short_sha(&source_base_commit_sha),
                        short_sha(&source_commit_sha),
                        source_branch,
                        integration_branch,
                        format_conflict_file_suffix(&conflict_files),
                    ),
                );
            }
            record.integration_note = format!(
                "cherry-picked {} of {source_commit_count} reviewed source commits from `{source_branch}` into `{integration_branch}`",
                pending_commits.len()
            );
        }
        let Some(integration_commit_sha) =
            resolve_workspace_head_sha_with_runtime(runtime, Path::new(&integration_repo_root))
        else {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                "reviewed integration result HEAD could not be resolved after cherry-pick"
                    .to_string(),
            );
        };
        record.integration_commit_sha = Some(integration_commit_sha);
    } else {
        record.integration_note = match prepared_integration.state {
            PreparedIntegrationState::ResumePendingPush => format!(
                "resuming the persisted reviewed integration result in `{integration_branch}` after restart"
            ),
            PreparedIntegrationState::AlreadyPushed => format!(
                "the persisted reviewed integration result is already present on `{integration_branch}`"
            ),
            PreparedIntegrationState::Fresh => unreachable!(),
        };
    }

    drop(integration_mutation_lock);
    record.integration_state = "done".to_string();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;

    let repo_root = integration_repo_root;
    let push_remote = target.push_remote.clone();
    if let Some(notice) = block_if_automation_epoch_closed(
        planning_authority,
        runtime,
        resolution,
        record,
        automation_permit,
        "integration push",
    )? {
        return Ok(notice);
    }
    if let Some(notice) = block_if_delivery_target_changed(
        planning_authority,
        runtime,
        resolution,
        record,
        github_automation,
        claim_permit,
        "integration push",
    )? {
        return Ok(notice);
    }
    if let Some(notice) = distributor_recheck_pull_request_before_integration_push(
        planning_authority,
        runtime,
        resolution,
        record,
        github_automation,
        claim_permit,
        autonomous_delivery_allowed,
    )? {
        return Ok(notice);
    }
    if let Some(notice) = block_if_delivery_target_changed(
        planning_authority,
        runtime,
        resolution,
        record,
        github_automation,
        claim_permit,
        "final integration push target verification",
    )? {
        return Ok(notice);
    }
    let integration_base_commit_sha = record
        .integration_base_commit_sha
        .clone()
        .ok_or_else(|| "frozen integration base is missing before push".to_string())?;
    let integration_commit_sha = record
        .integration_commit_sha
        .clone()
        .ok_or_else(|| "reviewed integration result is missing before push".to_string())?;
    claim_permit.renew("integration target compare-and-swap verification")?;
    let remote_integration_head = github_automation
        .remote_branch_head_for_delivery_target(
            &repo_root,
            &target.push_remote,
            &credential_redacted_push_url,
            &integration_branch,
        )
        .map_err(|error| format!("integration target head could not be verified: {error}"))?;
    let integration_push_required = match remote_integration_head.as_deref() {
        Some(remote_head) if remote_head == integration_commit_sha => false,
        Some(remote_head) if remote_head == integration_base_commit_sha => true,
        Some(remote_head) => {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!(
                    "integration target moved from frozen base `{}` to `{}` before reviewed result `{}` could be pushed",
                    short_sha(&integration_base_commit_sha),
                    short_sha(remote_head),
                    short_sha(&integration_commit_sha)
                ),
            );
        }
        None => {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!(
                    "integration target `{}/{}` disappeared after it was frozen",
                    target.push_remote, integration_branch
                ),
            );
        }
    };
    claim_permit.renew("integration target push")?;
    if integration_push_required
        && let Err(error) = github_automation.push_integration_branch_to_delivery_target(
            &repo_root,
            &target.push_remote,
            &credential_redacted_push_url,
            &integration_branch,
            &integration_base_commit_sha,
        )
    {
        let remote_ref = remote_branch_name(&target.push_remote, &target.integration_branch);
        claim_permit.renew("failed integration push recovery fetch")?;
        let remote_equivalent =
            fetch_integration_remote_branch(&repo_root, &target, github_automation)
                && distributor_source_cherry_states(runtime, &repo_root, &remote_ref, record)
                    .is_ok_and(|states| states.iter().all(|(_, equivalent)| *equivalent));
        let recovery_detail = if remote_equivalent {
            " remote now contains an equivalent patch, but local integration history was preserved; reconcile the branch manually"
        } else {
            " local integration history was preserved; inspect the remote and reconcile manually"
        };
        return block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "integration branch `{}` could not be pushed to `{}`: {error};{}",
                integration_branch, push_remote, recovery_detail
            ),
        );
    }
    claim_permit.renew("integration target verification fetch")?;
    if !fetch_integration_remote_branch(&repo_root, &target, github_automation) {
        return block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "integration push succeeded but `{}/{}` could not be verified; preserve the dedicated worktree for operator inspection",
                target.push_remote, target.integration_branch
            ),
        );
    }
    let pushed_head = resolve_workspace_head_sha_with_runtime(runtime, Path::new(&repo_root));
    claim_permit.renew("integration target head verification")?;
    let verified_remote_head = match github_automation.remote_branch_head_for_delivery_target(
        &repo_root,
        &target.push_remote,
        &credential_redacted_push_url,
        &target.integration_branch,
    ) {
        Ok(Some(verified_remote_head)) => verified_remote_head,
        Ok(None) => {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!(
                    "integration push succeeded but `{}/{}` disappeared during remote verification",
                    target.push_remote, target.integration_branch
                ),
            );
        }
        Err(error) => {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                format!(
                    "integration push succeeded but `{}/{}` could not be remotely verified: {error}",
                    target.push_remote, target.integration_branch
                ),
            );
        }
    };
    if pushed_head.as_deref() != Some(integration_commit_sha.as_str())
        || verified_remote_head != integration_commit_sha
    {
        return block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!(
                "integration push succeeded but `{}/{}` did not verify at the dedicated worktree HEAD; preserve the worktree for operator inspection",
                target.push_remote, target.integration_branch
            ),
        );
    }
    if let Err(error) = super::super::pr_validation::attest_distributor_pr_validation_with_ports(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    ) {
        return block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            format!("verified integration evidence could not be attested: {error}"),
        );
    }
    let mut preserved_review_surface_note = None;
    if let Some(pr_number) = record.pull_request_number {
        // PR close 전 다시 inspect해 URL을 최신화하고, 이미 닫힌 PR은 close 호출을 생략한다.
        claim_permit.renew("post-integration pull request inspection")?;
        let pull_request = match github_automation.inspect_pull_request_for_delivery_target(
            &repo_root,
            &target.push_remote,
            &credential_redacted_push_url,
            pr_number,
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
                        "pull request #{pr_number} could not be reloaded before close: {error}"
                    ),
                );
            }
        };
        record.pull_request_url = Some(pull_request.url.clone());
        let close_drift = pull_request_close_drift_details(
            &pull_request.base_branch,
            &pull_request.head_branch,
            pull_request.head_commit_sha.as_deref(),
            &integration_branch,
            &source_branch,
            &source_commit_sha,
        );
        if !close_drift.is_empty() {
            let note = format!(
                "pull request #{pr_number} changed after its reviewed commit was integrated ({}); the pull request and remote source branch were preserved for operator review",
                close_drift.join(", ")
            );
            record.recovery_note = Some(note.clone());
            preserved_review_surface_note = Some(note);
        } else if pull_request.state.eq_ignore_ascii_case("open") {
            if let Some(notice) = block_if_automation_epoch_closed(
                planning_authority,
                runtime,
                resolution,
                record,
                automation_permit,
                "pull request close",
            )? {
                return Ok(notice);
            }
            if let Some(notice) = block_if_delivery_target_changed(
                planning_authority,
                runtime,
                resolution,
                record,
                github_automation,
                claim_permit,
                "pull request close",
            )? {
                return Ok(notice);
            }
            claim_permit.renew("pull request close")?;
            if let Err(error) = github_automation.close_pull_request_for_delivery_target(
                &repo_root,
                &target.push_remote,
                &credential_redacted_push_url,
                pr_number,
            ) {
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
    }

    let source_branch_cleanup_note = if preserved_review_surface_note.is_some() {
        format!(
            "remote source branch `{}/{}` was preserved with the drifted pull request review surface",
            target.push_remote, source_branch
        )
    } else {
        if let Some(notice) = block_if_automation_epoch_closed(
            planning_authority,
            runtime,
            resolution,
            record,
            automation_permit,
            "conditional source branch cleanup",
        )? {
            return Ok(notice);
        }
        if let Some(notice) = block_if_delivery_target_changed(
            planning_authority,
            runtime,
            resolution,
            record,
            github_automation,
            claim_permit,
            "conditional source branch cleanup",
        )? {
            return Ok(notice);
        }
        claim_permit.renew("conditional remote source branch cleanup")?;
        match github_automation.delete_branch_if_unchanged_for_delivery_target(
            &repo_root,
            &target.push_remote,
            &credential_redacted_push_url,
            &source_branch,
            &source_commit_sha,
        ) {
            Ok(true) => format!(
                "remote source branch `{}/{}` was deleted or already absent under the frozen SHA lease",
                target.push_remote, source_branch
            ),
            Ok(false) => format!(
                "remote source branch `{}/{}` was preserved because it moved from frozen commit `{}`",
                target.push_remote,
                source_branch,
                short_sha(&source_commit_sha)
            ),
            Err(error) => format!(
                "remote source branch `{}/{}` was preserved because conditional cleanup failed: {error}",
                target.push_remote, source_branch
            ),
        }
    };

    record.queue_state = ParallelModeQueueItemState::Cleaning;
    record.integration_note = format!(
        "{}; {}",
        delivery_completion_note(
            record.pull_request_number.is_some(),
            &integration_branch,
            &push_remote,
        ),
        source_branch_cleanup_note
    );
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;

    let operator_notice = preserved_review_surface_note
        .map(|note| format!(" / {note}"))
        .unwrap_or_default();
    Ok(format!(
        "distributor integrated queue head into {} / slot: {} / agent: {} / commit: {}",
        integration_branch,
        resolution.lease.slot_id,
        resolution.lease.agent_id,
        short_sha(&record.commit_sha)
    ) + &operator_notice)
}

fn pull_request_close_drift_details(
    actual_base_branch: &str,
    actual_head_branch: &str,
    actual_head_commit_sha: Option<&str>,
    expected_base_branch: &str,
    expected_head_branch: &str,
    expected_head_commit_sha: &str,
) -> Vec<String> {
    let mut drift = Vec::new();
    if actual_base_branch != expected_base_branch {
        drift.push(format!(
            "base `{actual_base_branch}` != `{expected_base_branch}`"
        ));
    }
    if actual_head_branch != expected_head_branch {
        drift.push(format!(
            "head `{actual_head_branch}` != `{expected_head_branch}`"
        ));
    }
    if actual_head_commit_sha != Some(expected_head_commit_sha) {
        let actual = actual_head_commit_sha
            .map(short_sha)
            .unwrap_or_else(|| "missing".to_string());
        drift.push(format!(
            "head commit `{actual}` != `{}`",
            short_sha(expected_head_commit_sha)
        ));
    }
    drift
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
    automation_permit: Option<&ParallelModeAutomationPermit>,
    claim_permit: &DistributorQueueHeadClaimPermit,
    integration_target_oid: &str,
) -> Result<String, String> {
    if let Some(notice) = block_if_automation_epoch_closed(
        planning_authority,
        runtime,
        resolution,
        record,
        automation_permit,
        "slot cleanup",
    )? {
        return Ok(notice);
    }
    run_before_distributor_cleanup_lock_hook();
    let mutation_lock =
        acquire_pool_mutation_lock(planning_authority, runtime, &resolution.context.repo_root)?;
    let Some(locked_resolution) = resolve_workspace_slot_lease_with_runtime(
        runtime,
        planning_authority,
        &resolution.lease.worktree_path,
    )?
    else {
        return block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            None,
            record,
            "slot lease disappeared before locked distributor cleanup".to_string(),
        );
    };
    mutation_lock.verify_pool_root(&locked_resolution.context.pool_root)?;
    if locked_resolution.lease != resolution.lease {
        return block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            None,
            record,
            "slot lease generation changed before locked distributor cleanup".to_string(),
        );
    }
    let cleanup_pending_lease =
        if locked_resolution.lease.state == ParallelModeSlotLeaseState::Running {
            // Running lease를 먼저 CleanupPending으로 바꿔 통합 완료와 slot 반환 사이의 중간 상태를 보존한다.
            let mut cleanup_pending_lease = locked_resolution.lease.clone();
            cleanup_pending_lease.state = ParallelModeSlotLeaseState::CleanupPending;
            transition_slot_lease(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                &locked_resolution.lease,
                &cleanup_pending_lease,
            )?;
            let _ = record_cleanup_pending_session_detail(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                &cleanup_pending_lease,
            );
            cleanup_pending_lease
        } else if locked_resolution.lease.state == ParallelModeSlotLeaseState::CleanupPending {
            locked_resolution.lease.clone()
        } else {
            return block_distributor_queue_record(
                planning_authority,
                runtime,
                &locked_resolution.context.repo_root,
                &locked_resolution.context.pool_root,
                Some(&locked_resolution.lease),
                record,
                format!(
                    "slot `{}` is {} and cannot enter distributor cleanup",
                    locked_resolution.lease.slot_id,
                    locked_resolution.lease.state.label()
                ),
            );
        };

    super::super::pr_validation::transition_pr_validation_remediation_with_ports(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &record.task_id,
        true,
    )?;
    claim_permit.renew("slot cleanup")?;
    if !cleanup_slot_to_ref_locked(
        planning_authority,
        runtime,
        &PoolSlotCleanupIdentity::new(
            &locked_resolution.context.repo_root,
            &locked_resolution.context.canonical_repo_root,
            &locked_resolution.context.pool_root,
            &cleanup_pending_lease.slot_id,
            &locked_resolution.workspace_path,
            &cleanup_pending_lease.branch_name,
        ),
        integration_target_oid,
        PoolSlotCleanupLeaseAuthority::CleanupPending(&cleanup_pending_lease),
        &mutation_lock,
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
    let integration_branch = record
        .delivery_target
        .as_ref()
        .map(|target| target.integration_branch.as_str())
        .unwrap_or("the frozen integration branch");
    let delivery_note = record.integration_note.clone();
    record.integration_note = format!(
        "{}; {delivery_note}",
        cleanup_completion_note(record.pull_request_number.is_some(), integration_branch)
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

fn delivery_completion_note(
    used_pull_request: bool,
    integration_branch: &str,
    push_remote: &str,
) -> String {
    if used_pull_request {
        format!(
            "branch integrated into {}, pushed to {}, PR delivery completed, and the slot is entering cleanup",
            integration_branch, push_remote
        )
    } else {
        format!(
            "branch integrated into {}, pushed to {} without PR automation, and the slot is entering cleanup",
            integration_branch, push_remote
        )
    }
}

fn cleanup_completion_note(used_pull_request: bool, integration_branch: &str) -> String {
    if used_pull_request {
        format!(
            "branch integrated into {}, PR delivery completed, and the slot returned to idle",
            integration_branch
        )
    } else {
        format!(
            "branch integrated into {}, direct delivery completed without PR automation, and the slot returned to idle",
            integration_branch
        )
    }
}

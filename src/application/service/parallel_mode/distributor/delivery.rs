use super::*;

mod github;
mod integration;

use self::github::{
    distributor_check_pull_request_merge_readiness, distributor_ensure_pull_request,
    distributor_push_source_branch,
};
use self::integration::{
    collect_cherry_pick_conflict_files, commit_patch_equivalent_in_branch,
    ensure_distributor_integration_worktree_ready, format_conflict_file_suffix,
};

pub(super) fn process_distributor_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
) -> Result<Vec<String>, String> {
    if !Path::new(&record.worktree_path).exists() {
        return Ok(vec![block_distributor_queue_record(
            planning_authority,
            workspace_dir,
            pool_root,
            None,
            record,
            "source worktree is missing; distributor cannot continue".to_string(),
        )?]);
    }

    let resolution = match resolve_workspace_slot_lease(planning_authority, &record.worktree_path) {
        Ok(Some(resolution)) => resolution,
        Ok(None) => {
            return Ok(vec![block_distributor_queue_record(
                planning_authority,
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
                workspace_dir,
                pool_root,
                None,
                record,
                format!("slot lease could not be resolved for distributor delivery: {error}"),
            )?]);
        }
    };

    let mut notices = Vec::new();
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
            &resolution,
            record,
            github_automation,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            return Ok(notices);
        }

        notices.push(distributor_ensure_pull_request(
            planning_authority,
            &resolution,
            record,
            github_automation,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            return Ok(notices);
        }

        notices.push(distributor_check_pull_request_merge_readiness(
            planning_authority,
            &resolution,
            record,
            github_automation,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            return Ok(notices);
        }

        notices.push(distributor_integrate_branch(
            planning_authority,
            &resolution,
            record,
            github_automation,
        )?);
        if record.queue_state == ParallelModeQueueItemState::Blocked {
            return Ok(notices);
        }
    }

    let cleanup_notice =
        distributor_cleanup_integrated_slot(planning_authority, &resolution, record)?;
    notices.push(cleanup_notice);
    Ok(notices)
}

fn distributor_integrate_branch(
    planning_authority: &dyn PlanningAuthorityPort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    github_automation: &dyn GithubAutomationPort,
) -> Result<String, String> {
    let source_branch = record.effective_source_branch();
    let source_commit_sha = record.effective_source_commit_sha();
    let slot_status = inspect_slot_git_status(&resolution.workspace_path).ok_or_else(|| {
        format!(
            "slot `{}` git status could not be inspected for distributor delivery",
            resolution.lease.slot_id
        )
    })?;
    if slot_status.has_pending_operation {
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

    if current_branch_name(&resolution.workspace_path).as_deref() != Some(source_branch.as_str()) {
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

    let current_head = resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
        format!(
            "slot `{}` workspace head could not be resolved for distributor delivery",
            resolution.lease.slot_id
        )
    })?;
    if current_head != source_commit_sha {
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
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        record,
    )?;
    let _ = record_integrating_session_detail(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease,
        &record.integration_note,
    );

    let integration_repo_root = resolution.context.canonical_repo_root.display().to_string();

    if !branch_is_integrated_into(
        &integration_repo_root,
        &source_branch,
        DISTRIBUTOR_INTEGRATION_BRANCH,
    ) {
        if let Err(notice) = ensure_distributor_integration_worktree_ready(
            planning_authority,
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

    let repo_root = integration_repo_root;
    if let Err(error) =
        github_automation.push_integration_branch(&repo_root, DISTRIBUTOR_INTEGRATION_BRANCH)
    {
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
    if let Some(pr_number) = record.pull_request_number {
        let pull_request = match github_automation.inspect_pull_request(&repo_root, pr_number) {
            Ok(pull_request) => pull_request,
            Err(error) => {
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
        if pull_request.state.eq_ignore_ascii_case("open")
            && let Err(error) = github_automation.close_pull_request(&repo_root, pr_number)
        {
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

    Ok(format!(
        "distributor integrated queue head into {DISTRIBUTOR_INTEGRATION_BRANCH} / slot: {} / agent: {} / commit: {}",
        resolution.lease.slot_id,
        resolution.lease.agent_id,
        short_sha(&record.commit_sha)
    ))
}

fn distributor_cleanup_integrated_slot(
    planning_authority: &dyn PlanningAuthorityPort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<String, String> {
    if resolution.lease.state == ParallelModeSlotLeaseState::Running {
        let mut cleanup_pending_lease = resolution.lease.clone();
        cleanup_pending_lease.state = ParallelModeSlotLeaseState::CleanupPending;
        write_slot_lease(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &cleanup_pending_lease,
        )?;
        let _ = record_cleanup_pending_session_detail(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &cleanup_pending_lease,
        );
    }

    if !cleanup_slot(
        planning_authority,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        &resolution.lease.slot_id,
        &resolution.workspace_path,
        &resolution.lease.branch_name,
    ) {
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

    Ok(format!(
        "distributor returned slot to idle / slot: {} / agent: {}",
        resolution.lease.slot_id, resolution.lease.agent_id
    ))
}

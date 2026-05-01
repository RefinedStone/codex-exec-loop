use super::*;

pub(super) fn ensure_distributor_integration_worktree_ready(
    planning_authority: &dyn PlanningAuthorityPort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    integration_repo_root: &str,
) -> Result<(), String> {
    if current_branch_name(Path::new(integration_repo_root)).as_deref()
        != Some(DISTRIBUTOR_INTEGRATION_BRANCH)
    {
        let message = format!(
            "integration worktree must be checked out to `{DISTRIBUTOR_INTEGRATION_BRANCH}` before cherry-pick delivery"
        );
        let _ = block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        return Err(message);
    }

    let Some(status) = inspect_slot_git_status(Path::new(integration_repo_root)) else {
        let message = "integration worktree git status could not be inspected".to_string();
        let _ = block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        return Err(message);
    };
    if !status.is_ready_for_integration() {
        let message = format!(
            "integration worktree must be clean before cherry-pick delivery: {}",
            status.detail_label()
        );
        let _ = block_distributor_queue_record(
            planning_authority,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        return Err(message);
    }

    Ok(())
}

pub(super) fn commit_patch_equivalent_in_branch(
    repo_root: &str,
    base_branch: &str,
    commit_sha: &str,
) -> bool {
    let Some(cherry_output) = run_command(
        "git",
        ["-C", repo_root, "cherry", base_branch, commit_sha],
        None,
    ) else {
        return false;
    };

    cherry_output
        .lines()
        .any(|line| line.trim_start().starts_with('-'))
}

pub(super) fn collect_cherry_pick_conflict_files(repo_root: &str) -> Vec<String> {
    run_command(
        "git",
        ["-C", repo_root, "diff", "--name-only", "--diff-filter=U"],
        None,
    )
    .unwrap_or_default()
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(str::to_string)
    .collect::<Vec<_>>()
}

pub(super) fn format_conflict_file_suffix(conflict_files: &[String]) -> String {
    if conflict_files.is_empty() {
        String::new()
    } else {
        format!(" / conflicts: {}", conflict_files.join(", "))
    }
}

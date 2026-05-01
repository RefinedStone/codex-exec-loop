use std::path::Path;

use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModePoolSlotState,
    ParallelModeSlotLeaseRequest, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};

use super::{
    POOL_BASELINE_BRANCH, ParallelModeService, acquire_pool_allocation_lock,
    allocate_agent_branch_name, branch_is_cleanup_ready, build_pool_slots, cleanup_slot,
    command_succeeds, current_branch_name, current_timestamp, discard_unstarted_slot_branch,
    inspect_slot_git_status, load_pool_runtime_context, reconcile_pool_board,
    record_assigned_session_detail, record_cleanup_pending_session_detail,
    record_failed_start_session_detail, record_running_session_detail,
    record_thread_prepared_session_detail, resolve_workspace_slot_lease, write_slot_lease,
};

impl ParallelModeService {
    pub fn acquire_slot_lease(
        &self,
        workspace_dir: &str,
        request: ParallelModeSlotLeaseRequest,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let _allocation_lock =
            acquire_pool_allocation_lock(self.planning_authority.as_ref(), workspace_dir)?;
        let _ = reconcile_pool_board(self.planning_authority.as_ref(), workspace_dir);
        let context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;

        if context
            .slot_leases
            .values()
            .any(|lease| lease.task_id == request.task_id)
        {
            return Err(format!(
                "task `{}` already has an active slot lease",
                request.task_id
            ));
        }
        if context
            .slot_leases
            .values()
            .any(|lease| lease.agent_id == request.agent_id)
        {
            return Err(format!(
                "agent `{}` already owns an active slot lease",
                request.agent_id
            ));
        }

        let Some(idle_slot) = build_pool_slots(&context)
            .into_iter()
            .find(|slot| slot.state == ParallelModePoolSlotState::Idle)
        else {
            return Err("no idle slot is available for lease".to_string());
        };

        let slot_path = context.pool_root.join(&idle_slot.slot_id);
        let slot_path_string = slot_path.display().to_string();
        let branch_name = allocate_agent_branch_name(
            &context.repo_root,
            &idle_slot.slot_id,
            &request.task_slug,
            &request.task_id,
            &request.task_title,
        );
        if !command_succeeds(
            "git",
            [
                "-C",
                slot_path_string.as_str(),
                "checkout",
                "-b",
                branch_name.as_str(),
                POOL_BASELINE_BRANCH,
            ],
        ) {
            return Err(format!(
                "failed to create branch `{branch_name}` in slot `{}`",
                idle_slot.slot_id
            ));
        }

        let lease = ParallelModeSlotLeaseSnapshot::new(
            idle_slot.slot_id.clone(),
            request.task_id,
            request.task_title,
            request.agent_id,
            branch_name.clone(),
            slot_path_string.clone(),
            ParallelModeSlotLeaseState::Leased,
            current_timestamp(),
            None,
        );
        if let Err(error) = write_slot_lease(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        ) {
            let _ =
                discard_unstarted_slot_branch(&context.repo_root, &slot_path, branch_name.as_str());
            return Err(error);
        }
        let _ = record_assigned_session_detail(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        );

        Ok(lease)
    }

    pub fn mark_slot_running(
        &self,
        workspace_dir: &str,
        slot_id: &str,
        agent_id: &str,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;
        let mut lease = context
            .slot_leases
            .get(slot_id)
            .cloned()
            .ok_or_else(|| format!("slot `{slot_id}` does not have an active lease"))?;

        if lease.agent_id != agent_id {
            return Err(format!(
                "slot `{slot_id}` is leased by `{}` instead of `{agent_id}`",
                lease.agent_id
            ));
        }
        if lease.state == ParallelModeSlotLeaseState::CleanupPending {
            return Err(format!("slot `{slot_id}` is already waiting for cleanup",));
        }
        if current_branch_name(Path::new(&lease.worktree_path)).as_deref()
            != Some(lease.branch_name.as_str())
        {
            return Err(format!(
                "slot `{slot_id}` is no longer checked out to `{}`",
                lease.branch_name
            ));
        }

        lease.state = ParallelModeSlotLeaseState::Running;
        if lease.running_started_at.is_none() {
            lease.running_started_at = Some(current_timestamp());
        }
        write_slot_lease(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        )?;
        let _ = record_running_session_detail(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        );
        Ok(lease)
    }

    pub fn record_workspace_slot_thread_prepared(
        &self,
        workspace_dir: &str,
        thread_id: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };

        record_thread_prepared_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            thread_id,
        )
        .map(Some)
    }

    pub fn mark_slot_cleanup_pending(
        &self,
        workspace_dir: &str,
        slot_id: &str,
        agent_id: &str,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;
        let mut lease = context
            .slot_leases
            .get(slot_id)
            .cloned()
            .ok_or_else(|| format!("slot `{slot_id}` does not have an active lease"))?;

        if lease.agent_id != agent_id {
            return Err(format!(
                "slot `{slot_id}` is leased by `{}` instead of `{agent_id}`",
                lease.agent_id
            ));
        }
        if lease.state == ParallelModeSlotLeaseState::Leased {
            return Err(format!(
                "slot `{slot_id}` has not entered running state yet",
            ));
        }
        if lease.state == ParallelModeSlotLeaseState::CleanupPending {
            return Ok(lease);
        }
        if current_branch_name(Path::new(&lease.worktree_path)).as_deref()
            != Some(lease.branch_name.as_str())
        {
            return Err(format!(
                "slot `{slot_id}` is no longer checked out to `{}`",
                lease.branch_name
            ));
        }
        if !branch_is_cleanup_ready(&context.repo_root, &lease.branch_name) {
            return Err(format!(
                "slot `{slot_id}` branch `{}` is not integrated into `{POOL_BASELINE_BRANCH}` yet",
                lease.branch_name
            ));
        }

        lease.state = ParallelModeSlotLeaseState::CleanupPending;
        write_slot_lease(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        )?;
        let _ = record_cleanup_pending_session_detail(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        );
        Ok(lease)
    }

    pub fn mark_workspace_slot_running(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };

        self.mark_slot_running(
            workspace_dir,
            &resolution.lease.slot_id,
            &resolution.lease.agent_id,
        )
        .map(Some)
    }

    pub fn release_workspace_slot_lease_after_failed_start(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Leased {
            return Ok(None);
        }

        let Some(slot_status) = inspect_slot_git_status(&resolution.workspace_path) else {
            return Err(format!(
                "slot `{}` could not be inspected after startup failure",
                resolution.lease.slot_id
            ));
        };
        if !slot_status.is_clean_baseline() {
            return Err(format!(
                "slot `{}` could not be released after startup failure because worktree is not clean: {}",
                resolution.lease.slot_id,
                slot_status.detail_label()
            ));
        }

        if !cleanup_slot(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease.slot_id,
            &resolution.workspace_path,
            &resolution.lease.branch_name,
        ) {
            return Err(format!(
                "slot `{}` could not be reset to `{POOL_BASELINE_BRANCH}` after startup failure",
                resolution.lease.slot_id
            ));
        }
        let _ = record_failed_start_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
        );

        Ok(Some(resolution.lease))
    }
}

use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState,
};
use crate::domain::planning::{
    PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
};

use super::pool::{
    branch_is_cleanup_ready, cleanup_slot, resolve_workspace_head_sha, resolve_workspace_slot_lease,
};
use super::session_detail::{
    ReportedCompleteSessionDetailUpdate, record_cleaned_session_detail,
    record_commit_ready_session_detail, record_ledger_refreshing_session_detail,
    record_official_completion_failed_session_detail, record_reported_complete_session_detail,
};
use super::{
    POOL_BASELINE_BRANCH, ParallelModeOfficialCompletionReport, ParallelModeService,
    current_timestamp,
};

impl ParallelModeService {
    pub fn begin_workspace_official_completion(
        &self,
        workspace_dir: &str,
        root_turn_id: &str,
        official_completion_refresh_order: Option<u64>,
        final_response_text: Option<&str>,
        validation_summary: Option<&str>,
        failure_context: Option<&str>,
    ) -> Result<Option<ParallelModeOfficialCompletionReport>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        let commit_sha =
            resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
                format!(
                    "slot `{}` workspace head could not be resolved for official completion",
                    resolution.lease.slot_id
                )
            })?;
        let completed_at = current_timestamp();
        let refresh_order = official_completion_refresh_order
            .map(Ok)
            .unwrap_or_else(|| {
                self.planning_authority
                    .reserve_next_official_refresh_order(&resolution.lease.worktree_path)
                    .map_err(|error| error.to_string())
            })?;
        let final_response_text = normalized_optional_text(final_response_text).map(str::to_string);
        let validation_summary = normalized_optional_text(validation_summary)
            .unwrap_or("validation status was not reported by runtime")
            .to_string();
        let failure_context = normalized_optional_text(failure_context).map(str::to_string);
        let final_response_summary = completion_summary_from_text(
            final_response_text.as_deref(),
            failure_context.as_deref(),
        );

        record_reported_complete_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            ReportedCompleteSessionDetailUpdate {
                completed_at: &completed_at,
                final_response_summary: &final_response_summary,
                validation_summary: &validation_summary,
                failure_context: failure_context.as_deref(),
            },
        )?;

        Ok(Some(PlanningOfficialCompletionRefreshContract::new(
            root_turn_id,
            refresh_order,
            PlanningOfficialCompletionRefreshPayload::new(
                resolution.lease.agent_id,
                resolution.lease.task_id,
                resolution.lease.task_title,
                resolution.lease.branch_name,
                resolution.lease.worktree_path,
                commit_sha,
                validation_summary,
                final_response_summary,
                final_response_text,
                failure_context,
                completed_at,
            ),
        )))
    }

    pub fn mark_workspace_official_completion_refreshing(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        record_ledger_refreshing_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
        )
        .map(Some)
    }

    pub fn mark_workspace_commit_ready(
        &self,
        workspace_dir: &str,
        authority_refresh_outcome: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        record_commit_ready_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            authority_refresh_outcome,
        )
        .map(Some)
    }

    pub fn enqueue_workspace_commit_ready_result(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<crate::domain::parallel_mode::ParallelModeDistributorQueueItem>, String>
    {
        self.distributor_service
            .enqueue_workspace_commit_ready_result(workspace_dir)
    }

    pub fn process_distributor_queue(&self, workspace_dir: &str) -> Result<Vec<String>, String> {
        self.distributor_service.process_queue(workspace_dir)
    }

    pub fn mark_workspace_official_completion_failed(
        &self,
        workspace_dir: &str,
        failure_detail: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        record_official_completion_failed_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            failure_detail,
        )
        .map(Some)
    }

    pub fn mark_workspace_slot_cleanup_pending_if_ready(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state == ParallelModeSlotLeaseState::CleanupPending {
            return Ok(Some(resolution.lease));
        }
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }
        if !branch_is_cleanup_ready(&resolution.context.repo_root, &resolution.lease.branch_name) {
            return Ok(None);
        }

        self.mark_slot_cleanup_pending(
            workspace_dir,
            &resolution.lease.slot_id,
            &resolution.lease.agent_id,
        )
        .map(Some)
    }

    pub fn cleanup_workspace_slot_if_pending(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::CleanupPending {
            return Ok(None);
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
                "slot `{}` could not be reset to `{POOL_BASELINE_BRANCH}` after successful completion",
                resolution.lease.slot_id
            ));
        }
        let _ = record_cleaned_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
        );

        Ok(Some(resolution.lease))
    }
}

fn normalized_optional_text(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|value| !value.is_empty())
}

fn completion_summary_from_text(
    final_response_text: Option<&str>,
    failure_context: Option<&str>,
) -> String {
    if let Some(summary) = final_response_text
        .and_then(first_non_empty_line)
        .filter(|summary| !summary.is_empty())
    {
        return summary.to_string();
    }
    if let Some(context) = failure_context {
        return format!("agent session finished with follow-up context: {context}");
    }

    "agent session reported completion without a structured final summary".to_string()
}

fn first_non_empty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

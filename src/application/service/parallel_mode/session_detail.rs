use std::path::Path;

use chrono::Utc;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeLiveSessionDetailDefaults,
    ParallelModeSlotLeaseSnapshot,
};

use super::current_timestamp;

mod store;

#[cfg(test)]
pub(super) use self::store::{agent_session_detail_record_path, read_agent_session_detail_record};

use self::store::{
    push_session_history, update_agent_session_detail_record, write_agent_session_detail_record,
};

pub(super) fn default_validation_summary() -> &'static str {
    "validation summary is not recorded in runtime yet"
}

pub(super) fn default_authority_refresh_outcome() -> &'static str {
    "no official completion has been reported yet"
}

pub(super) fn lease_session_key(lease: &ParallelModeSlotLeaseSnapshot) -> String {
    lease.session_key()
}

pub(super) fn build_assigned_session_detail(
    lease: &ParallelModeSlotLeaseSnapshot,
) -> ParallelModeAgentSessionDetailSnapshot {
    ParallelModeAgentSessionDetailSnapshot::assigned_for_lease(
        lease,
        ParallelModeLiveSessionDetailDefaults {
            validation_summary: default_validation_summary(),
            authority_refresh_outcome: default_authority_refresh_outcome(),
        },
    )
}

pub(super) fn record_assigned_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    let detail = build_assigned_session_detail(lease);
    write_agent_session_detail_record(planning_authority, workspace_dir, pool_root, &detail)?;
    Ok(detail)
}

pub(super) fn record_thread_prepared_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    thread_id: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.thread_id = Some(thread_id.to_string());
            detail.state_label = "starting".to_string();
            detail.completion_state_label = "in_progress".to_string();
            let summary = format!("thread prepared for the leased session / thread: {thread_id}");
            detail.latest_summary = summary.clone();
            detail.updated_at = timestamp.clone();
            push_session_history(&mut detail, "starting", timestamp, summary);
            detail
        },
    )
}

pub(super) fn record_running_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = lease
                .running_started_at
                .clone()
                .unwrap_or_else(current_timestamp);
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "running".to_string();
            detail.completion_state_label = "in_progress".to_string();
            detail.latest_summary = "agent session entered the running state".to_string();
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "running",
                timestamp,
                "agent session entered the running state".to_string(),
            );
            detail
        },
    )
}

pub(super) struct ReportedCompleteSessionDetailUpdate<'a> {
    pub(super) completed_at: &'a str,
    pub(super) final_response_summary: &'a str,
    pub(super) validation_summary: &'a str,
    pub(super) failure_context: Option<&'a str>,
}

pub(super) fn record_reported_complete_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    update: ReportedCompleteSessionDetailUpdate<'_>,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "reported_complete".to_string();
            detail.completion_state_label = "reported_complete".to_string();
            detail.latest_summary = update.final_response_summary.to_string();
            detail.validation_summary = update.validation_summary.to_string();
            detail.authority_refresh_outcome =
                "completion reported; official ledger refresh is pending".to_string();
            detail.distributor_outcome = None;
            detail.updated_at = update.completed_at.to_string();

            let history_summary = update.failure_context.map_or_else(
                || update.final_response_summary.to_string(),
                |context| format!("{} / context: {context}", update.final_response_summary),
            );
            push_session_history(
                &mut detail,
                "reported_complete",
                update.completed_at.to_string(),
                history_summary,
            );
            detail
        },
    )
}

pub(super) fn record_ledger_refreshing_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "ledger_refreshing".to_string();
            detail.completion_state_label = "ledger_refreshing".to_string();
            detail.latest_summary =
                "completion reported and hidden planning worker is refreshing the ledger"
                    .to_string();
            detail.authority_refresh_outcome =
                "hidden planning worker is refreshing the official task ledger".to_string();
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "ledger_refreshing",
                timestamp,
                "hidden planning worker is refreshing the official task ledger".to_string(),
            );
            detail
        },
    )
}

pub(super) fn record_commit_ready_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    authority_refresh_outcome: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "commit_ready".to_string();
            detail.completion_state_label = "commit_ready".to_string();
            detail.latest_summary =
                "official ledger refresh accepted the completion report".to_string();
            detail.authority_refresh_outcome = authority_refresh_outcome.trim().to_string();
            detail.distributor_outcome =
                Some("commit-ready result is waiting for distributor integration".to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "commit_ready",
                timestamp,
                "official ledger refresh accepted the completion report".to_string(),
            );
            detail
        },
    )
}

pub(super) fn record_merge_queued_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "merge_queued".to_string();
            detail.completion_state_label = "merge_queued".to_string();
            detail.latest_summary =
                "commit-ready result accepted into the distributor queue".to_string();
            detail.distributor_outcome = Some(
                "distributor accepted the result and queued it for GitHub delivery".to_string(),
            );
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "merge_queued",
                timestamp,
                "distributor accepted the result and queued it for GitHub delivery".to_string(),
            );
            detail
        },
    )
}

pub(super) fn record_pushing_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    summary: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "pushing".to_string();
            detail.completion_state_label = "merge_queued".to_string();
            detail.latest_summary = summary.trim().to_string();
            detail.distributor_outcome = Some(summary.trim().to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "pushing",
                timestamp,
                summary.trim().to_string(),
            );
            detail
        },
    )
}

pub(super) fn record_pr_pending_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    summary: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "pr_pending".to_string();
            detail.completion_state_label = "merge_queued".to_string();
            detail.latest_summary = summary.trim().to_string();
            detail.distributor_outcome = Some(summary.trim().to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "pr_pending",
                timestamp,
                summary.trim().to_string(),
            );
            detail
        },
    )
}

pub(super) fn record_merge_pending_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    summary: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "merge_pending".to_string();
            detail.completion_state_label = "merge_queued".to_string();
            detail.latest_summary = summary.trim().to_string();
            detail.distributor_outcome = Some(summary.trim().to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "merge_pending",
                timestamp,
                summary.trim().to_string(),
            );
            detail
        },
    )
}

pub(super) fn record_integrating_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    summary: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "integrating".to_string();
            detail.completion_state_label = "merge_queued".to_string();
            detail.latest_summary = summary.trim().to_string();
            detail.distributor_outcome = Some(summary.trim().to_string());
            detail.updated_at = timestamp.clone();
            if let Some(last_entry) = detail.history.last_mut()
                && last_entry.state_label == "integrating"
            {
                last_entry.timestamp = timestamp;
                last_entry.summary = summary.trim().to_string();
            } else {
                push_session_history(
                    &mut detail,
                    "integrating",
                    timestamp,
                    summary.trim().to_string(),
                );
            }
            detail
        },
    )
}

pub(super) fn record_distributor_failed_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    failure_detail: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "failed".to_string();
            detail.completion_state_label = "failed".to_string();
            detail.latest_summary = "distributor delivery failed".to_string();
            detail.distributor_outcome = Some(failure_detail.trim().to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "failed",
                timestamp,
                failure_detail.trim().to_string(),
            );
            detail
        },
    )
}

pub(super) fn record_official_completion_failed_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    failure_detail: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "failed".to_string();
            detail.completion_state_label = "failed".to_string();
            detail.latest_summary = "official completion refresh failed".to_string();
            detail.authority_refresh_outcome = failure_detail.trim().to_string();
            detail.distributor_outcome = Some(
                "not queued for distributor integration because official refresh failed"
                    .to_string(),
            );
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "failed",
                timestamp,
                failure_detail.trim().to_string(),
            );
            detail
        },
    )
}

pub(super) fn record_cleanup_pending_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "cleanup_pending".to_string();
            detail.completion_state_label = "merged".to_string();
            detail.latest_summary =
                "agent branch is merged into prerelease and awaiting slot cleanup".to_string();
            detail.distributor_outcome = Some(
                "branch is merged into prerelease and the slot is awaiting cleanup".to_string(),
            );
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "merged",
                timestamp.clone(),
                "branch is integrated into prerelease".to_string(),
            );
            push_session_history(
                &mut detail,
                "cleanup_pending",
                timestamp,
                "slot is waiting for cleanup before it can be reused".to_string(),
            );
            detail
        },
    )
}

pub(super) fn record_cleaned_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "cleaned".to_string();
            detail.completion_state_label = "cleaned".to_string();
            detail.latest_summary =
                "merged session cleaned up and the slot returned to the idle pool".to_string();
            detail.distributor_outcome =
                Some("branch merged into prerelease and the slot returned to idle".to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "cleaned",
                timestamp,
                "slot cleaned and returned to the idle pool".to_string(),
            );
            detail
        },
    )
}

pub(super) fn record_failed_start_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "failed".to_string();
            detail.completion_state_label = "aborted".to_string();
            detail.latest_summary =
                "launch failed before the session reached the running state".to_string();
            detail.distributor_outcome =
                Some("not queued for distributor work; slot returned to idle".to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "failed",
                timestamp.clone(),
                "launch failed before the session reached the running state".to_string(),
            );
            push_session_history(
                &mut detail,
                "cleaned",
                timestamp,
                "slot cleaned and returned to the idle pool after launch failure".to_string(),
            );
            detail
        },
    )
}

pub(super) fn format_elapsed_label_from_timestamp(timestamp: &str) -> Option<String> {
    let started_at = chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()?
        .with_timezone(&Utc);
    let elapsed_seconds = Utc::now()
        .signed_duration_since(started_at)
        .num_seconds()
        .max(0);

    Some(format_elapsed_seconds(elapsed_seconds))
}

fn format_elapsed_seconds(elapsed_seconds: i64) -> String {
    if elapsed_seconds < 60 {
        return format!("{elapsed_seconds}s");
    }
    if elapsed_seconds < 60 * 60 {
        let minutes = elapsed_seconds / 60;
        let seconds = elapsed_seconds % 60;
        if seconds == 0 {
            return format!("{minutes}m");
        }
        return format!("{minutes}m {seconds}s");
    }
    if elapsed_seconds < 60 * 60 * 24 {
        let hours = elapsed_seconds / (60 * 60);
        let minutes = (elapsed_seconds % (60 * 60)) / 60;
        return format!("{hours}h {minutes}m");
    }

    let days = elapsed_seconds / (60 * 60 * 24);
    let hours = (elapsed_seconds % (60 * 60 * 24)) / (60 * 60);
    format!("{days}d {hours}h")
}

#[cfg(test)]
mod tests {
    use super::format_elapsed_seconds;

    #[test]
    fn elapsed_label_keeps_seconds_for_short_running_agents() {
        assert_eq!(format_elapsed_seconds(65), "1m 5s");
        assert_eq!(format_elapsed_seconds(120), "2m");
    }
}

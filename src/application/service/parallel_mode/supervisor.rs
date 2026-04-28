use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
    ParallelModeAgentSessionDetailSnapshot, ParallelModePoolBoardSnapshot,
    ParallelModeReadinessSnapshot, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
};

use super::distributor::{ParallelModeDistributorQueueRecord, ParallelModeDistributorService};
use super::{
    PoolBoardWithContextResult, PoolRuntimeContext, build_assigned_session_detail,
    build_pool_board, default_authority_refresh_outcome, default_supervisor_notice,
    default_validation_summary, derive_supervisor_state, format_elapsed_label_from_timestamp,
    inspect_pool_board_and_context, lease_session_key, pool_operator_recovery_notice,
    reconcile_pool_board_and_context,
};

#[derive(Debug, Clone, Default)]
pub(super) struct ParallelModeSupervisorService;

impl ParallelModeSupervisorService {
    pub(super) fn new() -> Self {
        Self
    }

    pub(super) fn build_snapshot(
        &self,
        planning_authority: &dyn PlanningAuthorityPort,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
        distributor_service: &ParallelModeDistributorService,
    ) -> ParallelModeSupervisorSnapshot {
        let state = derive_supervisor_state(mode_enabled, readiness_snapshot);
        let workspace_path = readiness_snapshot
            .map(|snapshot| snapshot.workspace_path.clone())
            .unwrap_or_else(|| workspace_dir.to_string());
        let (pool, roster, detail) = match readiness_snapshot {
            Some(snapshot) if snapshot.allows_parallel_mode() => build_supervisor_views(
                inspect_pool_board_and_context(planning_authority, workspace_dir),
                mode_enabled,
            ),
            _ => (
                build_pool_board(planning_authority, workspace_dir, readiness_snapshot),
                build_placeholder_roster(mode_enabled, readiness_snapshot),
                build_supervisor_detail(readiness_snapshot),
            ),
        };
        let top_notice = supervisor_top_notice(&pool, mode_enabled, readiness_snapshot);

        ParallelModeSupervisorSnapshot::new(
            state,
            workspace_path,
            pool,
            roster,
            detail,
            distributor_service.build_snapshot(workspace_dir, mode_enabled, readiness_snapshot),
            top_notice,
        )
    }

    pub(super) fn reconcile_snapshot(
        &self,
        planning_authority: &dyn PlanningAuthorityPort,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
        distributor_service: &ParallelModeDistributorService,
    ) -> ParallelModeSupervisorSnapshot {
        let state = derive_supervisor_state(mode_enabled, readiness_snapshot);
        let workspace_path = readiness_snapshot
            .map(|snapshot| snapshot.workspace_path.clone())
            .unwrap_or_else(|| workspace_dir.to_string());
        let (pool, roster, detail) = match readiness_snapshot {
            Some(snapshot) if snapshot.allows_parallel_mode() => {
                let runtime = if mode_enabled {
                    reconcile_pool_board_and_context(planning_authority, workspace_dir)
                } else {
                    inspect_pool_board_and_context(planning_authority, workspace_dir)
                };
                build_supervisor_views(runtime, mode_enabled)
            }
            _ => (
                build_pool_board(planning_authority, workspace_dir, readiness_snapshot),
                build_placeholder_roster(mode_enabled, readiness_snapshot),
                build_supervisor_detail(readiness_snapshot),
            ),
        };
        let top_notice = supervisor_top_notice(&pool, mode_enabled, readiness_snapshot);

        ParallelModeSupervisorSnapshot::new(
            state,
            workspace_path,
            pool,
            roster,
            detail,
            distributor_service.build_snapshot(workspace_dir, mode_enabled, readiness_snapshot),
            top_notice,
        )
    }
}

fn supervisor_top_notice(
    pool: &ParallelModePoolBoardSnapshot,
    mode_enabled: bool,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> Option<String> {
    readiness_snapshot
        .and_then(|snapshot| snapshot.top_alert.clone())
        .or_else(|| pool_operator_recovery_notice(pool))
        .or_else(|| default_supervisor_notice(mode_enabled, readiness_snapshot))
}

fn build_placeholder_roster(
    mode_enabled: bool,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModeAgentRosterSnapshot {
    let empty_state = match (mode_enabled, readiness_snapshot) {
        (true, Some(snapshot)) if snapshot.allows_parallel_mode() => {
            "no agent sessions launched in this slice"
        }
        (true, Some(_)) => "readiness must recover before agent launch is allowed",
        (true, None) => "rerun readiness before agent launch is available",
        (false, Some(_)) => "parallel mode is off / agent roster is read-only",
        (false, None) => "parallel mode is off / no supervisor roster loaded",
    };

    ParallelModeAgentRosterSnapshot::new(Vec::new(), empty_state)
}

fn build_supervisor_views(
    runtime: PoolBoardWithContextResult,
    mode_enabled: bool,
) -> (
    ParallelModePoolBoardSnapshot,
    ParallelModeAgentRosterSnapshot,
    ParallelModeSupervisorDetailSnapshot,
) {
    match runtime {
        Ok((context, pool)) => (
            pool,
            build_agent_roster_from_context(&context, mode_enabled),
            build_supervisor_detail_from_context(&context, mode_enabled),
        ),
        Err(error) => {
            let (pool, detail) = *error;
            (
                pool,
                ParallelModeAgentRosterSnapshot::new(
                    Vec::new(),
                    format!("agent roster unavailable / {detail}"),
                ),
                ParallelModeSupervisorDetailSnapshot::new(
                    None,
                    format!("supervisor detail unavailable / {detail}"),
                ),
            )
        }
    }
}

fn build_agent_roster_from_context(
    context: &PoolRuntimeContext,
    mode_enabled: bool,
) -> ParallelModeAgentRosterSnapshot {
    let history = context.session_details.clone();
    let leases = sorted_active_leases(context);

    let entries = leases
        .iter()
        .map(|lease| {
            let detail = session_detail_for_lease(history.as_slice(), lease);
            build_agent_roster_entry(lease, detail.as_ref())
        })
        .collect::<Vec<_>>();
    let empty_state = if mode_enabled {
        "no agent sessions launched in this slice"
    } else {
        "parallel mode is off / agent roster is read-only"
    };

    ParallelModeAgentRosterSnapshot::new(entries, empty_state)
}

fn build_agent_roster_entry(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
) -> ParallelModeAgentRosterEntry {
    ParallelModeAgentRosterEntry::new(
        lease.agent_id.clone(),
        lease.task_title.clone(),
        lease.slot_id.clone(),
        lease.branch_name.clone(),
        roster_state_label(lease, detail),
        roster_duration_label(lease, detail),
        roster_latest_summary(lease, detail),
    )
}

fn roster_state_priority(state: ParallelModeSlotLeaseState) -> u8 {
    match state {
        ParallelModeSlotLeaseState::Running => 3,
        ParallelModeSlotLeaseState::Leased => 2,
        ParallelModeSlotLeaseState::CleanupPending => 1,
    }
}

fn roster_recency_key(lease: &ParallelModeSlotLeaseSnapshot) -> &str {
    lease
        .running_started_at
        .as_deref()
        .unwrap_or(lease.leased_at.as_str())
}

fn roster_state_label(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
) -> String {
    if let Some(detail) = detail
        && let Some(label) = active_runtime_state_override(lease, detail)
    {
        return label.to_string();
    }

    match lease.state {
        ParallelModeSlotLeaseState::Leased => "starting".to_string(),
        ParallelModeSlotLeaseState::Running => "running".to_string(),
        ParallelModeSlotLeaseState::CleanupPending => "cleanup_pending".to_string(),
    }
}

fn roster_duration_label(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
) -> String {
    if let Some(detail) = detail {
        match detail.state_label.as_str() {
            "reported_complete" => return "reported".to_string(),
            "ledger_refreshing" => return "refreshing".to_string(),
            "commit_ready" => return "official".to_string(),
            "merge_queued" => return "queued".to_string(),
            "pushing" => return "pushing".to_string(),
            "pr_pending" => return "pr pending".to_string(),
            "merge_pending" => return "merge pending".to_string(),
            "integrating" => return "integrating".to_string(),
            "failed" => return "blocked".to_string(),
            _ => {}
        }
    }

    match lease.state {
        ParallelModeSlotLeaseState::Leased => "launch pending".to_string(),
        ParallelModeSlotLeaseState::Running => lease
            .running_started_at
            .as_deref()
            .and_then(format_elapsed_label_from_timestamp)
            .unwrap_or_else(|| "active".to_string()),
        ParallelModeSlotLeaseState::CleanupPending => "complete".to_string(),
    }
}

fn roster_latest_summary(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
) -> String {
    detail
        .map(|detail| detail.latest_summary.trim())
        .filter(|summary| !summary.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match lease.state {
            ParallelModeSlotLeaseState::Leased => {
                "branch reserved and agent bootstrap in progress".to_string()
            }
            ParallelModeSlotLeaseState::Running => {
                "agent session is active in the leased slot".to_string()
            }
            ParallelModeSlotLeaseState::CleanupPending => {
                "agent session reported completion and slot cleanup is pending".to_string()
            }
        })
}

fn build_supervisor_detail(
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModeSupervisorDetailSnapshot {
    match readiness_snapshot {
        Some(_) => ParallelModeSupervisorDetailSnapshot::new(
            None,
            "readiness must recover before supervisor detail is available",
        ),
        None => ParallelModeSupervisorDetailSnapshot::new(
            None,
            "rerun readiness before supervisor detail is available",
        ),
    }
}

fn build_supervisor_detail_from_context(
    context: &PoolRuntimeContext,
    mode_enabled: bool,
) -> ParallelModeSupervisorDetailSnapshot {
    let history = context.session_details.clone();
    let queue_records = context.distributor_queue_records.clone();
    let empty_state = if mode_enabled {
        "no agent session history captured yet"
    } else {
        "parallel mode is off / supervisor detail is read-only"
    };

    ParallelModeSupervisorDetailSnapshot::new(
        selected_runtime_session_detail(context, &history, &queue_records),
        empty_state,
    )
}

fn build_live_session_detail(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<ParallelModeAgentSessionDetailSnapshot>,
) -> ParallelModeAgentSessionDetailSnapshot {
    let mut detail = detail.unwrap_or_else(|| build_assigned_session_detail(lease));
    detail.session_key = lease_session_key(lease);
    detail.agent_id = lease.agent_id.clone();
    detail.task_id = lease.task_id.clone();
    detail.task_title = lease.task_title.clone();
    detail.slot_id = lease.slot_id.clone();
    detail.worktree_path = lease.worktree_path.clone();
    detail.branch_name = lease.branch_name.clone();
    detail.lease_started_at = lease.leased_at.clone();
    detail.state_label = live_detail_state_label(lease, &detail);
    detail.completion_state_label = live_completion_state_label(lease, &detail);
    if detail.latest_summary.trim().is_empty() {
        detail.latest_summary = roster_latest_summary(lease, Some(&detail));
    }
    if detail.validation_summary.trim().is_empty() {
        detail.validation_summary = default_validation_summary().to_string();
    }
    if detail.authority_refresh_outcome.trim().is_empty() {
        detail.authority_refresh_outcome = default_authority_refresh_outcome().to_string();
    }
    if detail.distributor_outcome.is_none() {
        detail.distributor_outcome = live_distributor_outcome(lease);
    }
    if detail.updated_at.trim().is_empty() {
        detail.updated_at = live_detail_updated_at(lease).to_string();
    }
    detail
}

fn live_detail_state_label(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: &ParallelModeAgentSessionDetailSnapshot,
) -> String {
    if let Some(label) = active_runtime_state_override(lease, detail) {
        return label.to_string();
    }

    match lease.state {
        ParallelModeSlotLeaseState::Leased => {
            if detail.thread_id.is_some() || detail.state_label == "starting" {
                "starting".to_string()
            } else {
                "assigned".to_string()
            }
        }
        ParallelModeSlotLeaseState::Running => "running".to_string(),
        ParallelModeSlotLeaseState::CleanupPending => "cleanup_pending".to_string(),
    }
}

fn active_runtime_state_override<'a>(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: &'a ParallelModeAgentSessionDetailSnapshot,
) -> Option<&'a str> {
    match lease.state {
        ParallelModeSlotLeaseState::Running => match detail.state_label.as_str() {
            "reported_complete" | "ledger_refreshing" | "commit_ready" | "merge_queued"
            | "pushing" | "pr_pending" | "merge_pending" | "integrating" | "failed" => {
                Some(detail.state_label.as_str())
            }
            _ => None,
        },
        ParallelModeSlotLeaseState::CleanupPending => match detail.state_label.as_str() {
            "failed" => Some(detail.state_label.as_str()),
            _ => None,
        },
        ParallelModeSlotLeaseState::Leased => None,
    }
}

fn live_completion_state_label(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: &ParallelModeAgentSessionDetailSnapshot,
) -> String {
    if active_runtime_state_override(lease, detail).is_some() {
        return detail.completion_state_label.clone();
    }

    match lease.state {
        ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running => {
            "in_progress".to_string()
        }
        ParallelModeSlotLeaseState::CleanupPending => "merged".to_string(),
    }
}

fn live_distributor_outcome(lease: &ParallelModeSlotLeaseSnapshot) -> Option<String> {
    match lease.state {
        ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running => None,
        ParallelModeSlotLeaseState::CleanupPending => {
            Some("branch is merged into akra and the slot is awaiting cleanup".to_string())
        }
    }
}

fn live_detail_updated_at(lease: &ParallelModeSlotLeaseSnapshot) -> &str {
    lease
        .running_started_at
        .as_deref()
        .unwrap_or(lease.leased_at.as_str())
}

pub(super) fn selected_runtime_session_detail(
    context: &PoolRuntimeContext,
    history: &[ParallelModeAgentSessionDetailSnapshot],
    queue_records: &[ParallelModeDistributorQueueRecord],
) -> Option<ParallelModeAgentSessionDetailSnapshot> {
    if let Some(queue_head) = queue_records
        .iter()
        .find(|record| record.queue_state.is_active())
        && let Some(detail) =
            session_detail_for_runtime_session(context, history, &queue_head.session_key)
    {
        return Some(detail);
    }

    let active_leases = sorted_active_leases(context);

    if let Some(lease) = active_leases.first() {
        return Some(build_live_session_detail(
            lease,
            session_detail_for_lease(history, lease),
        ));
    }

    history.first().cloned()
}

fn sorted_active_leases(context: &PoolRuntimeContext) -> Vec<ParallelModeSlotLeaseSnapshot> {
    let mut leases = context.slot_leases.values().cloned().collect::<Vec<_>>();
    leases.sort_by(|left, right| {
        roster_state_priority(right.state)
            .cmp(&roster_state_priority(left.state))
            .then_with(|| roster_recency_key(right).cmp(roster_recency_key(left)))
            .then_with(|| left.slot_id.cmp(&right.slot_id))
    });
    leases
}

fn session_detail_for_lease(
    history: &[ParallelModeAgentSessionDetailSnapshot],
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Option<ParallelModeAgentSessionDetailSnapshot> {
    history
        .iter()
        .find(|detail| detail.session_key == lease_session_key(lease))
        .cloned()
}

fn session_detail_for_runtime_session(
    context: &PoolRuntimeContext,
    history: &[ParallelModeAgentSessionDetailSnapshot],
    session_key: &str,
) -> Option<ParallelModeAgentSessionDetailSnapshot> {
    let detail = history
        .iter()
        .find(|detail| detail.session_key == session_key)
        .cloned();
    if let Some(lease) = context
        .slot_leases
        .values()
        .find(|lease| lease_session_key(lease) == session_key)
    {
        return Some(build_live_session_detail(lease, detail));
    }

    detail
}

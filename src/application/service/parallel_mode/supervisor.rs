use std::collections::BTreeMap;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeAgentSessionDetailSnapshot,
    ParallelModeLiveSessionDetailDefaults, ParallelModePoolBoardSnapshot,
    ParallelModeReadinessSnapshot, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState,
};

use super::distributor::{ParallelModeDistributorQueueRecord, ParallelModeDistributorService};
use super::{
    PoolBoardWithContextResult, PoolRuntimeContext, build_pool_board,
    default_authority_refresh_outcome, default_supervisor_notice, default_validation_summary,
    format_elapsed_label_from_timestamp, inspect_pool_board_and_context, lease_session_key,
    pool_operator_recovery_notice, reconcile_pool_board_and_context,
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
        let state = ParallelModeSupervisorState::derive(mode_enabled, readiness_snapshot);
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
        let state = ParallelModeSupervisorState::derive(mode_enabled, readiness_snapshot);
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
    let leases = context.slot_leases.values().cloned().collect::<Vec<_>>();
    let duration_labels = running_duration_labels(&leases);
    ParallelModeAgentRosterSnapshot::project_from_leases(
        leases,
        &context.session_details,
        mode_enabled,
        &duration_labels,
    )
}

fn running_duration_labels(leases: &[ParallelModeSlotLeaseSnapshot]) -> BTreeMap<String, String> {
    leases
        .iter()
        .filter(|lease| lease.state == ParallelModeSlotLeaseState::Running)
        .filter_map(|lease| {
            let label = lease
                .running_started_at
                .as_deref()
                .and_then(format_elapsed_label_from_timestamp)?;
            Some((lease_session_key(lease), label))
        })
        .collect()
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

pub(super) fn selected_runtime_session_detail(
    context: &PoolRuntimeContext,
    history: &[ParallelModeAgentSessionDetailSnapshot],
    queue_records: &[ParallelModeDistributorQueueRecord],
) -> Option<ParallelModeAgentSessionDetailSnapshot> {
    let leases = context.slot_leases.values().cloned().collect::<Vec<_>>();
    let active_queue_session_key = queue_records
        .iter()
        .find(|record| record.queue_state.is_active())
        .map(|record| record.session_key.as_str());
    ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
        &leases,
        history,
        active_queue_session_key,
        live_detail_defaults(),
    )
}

fn live_detail_defaults() -> ParallelModeLiveSessionDetailDefaults<'static> {
    ParallelModeLiveSessionDetailDefaults {
        validation_summary: default_validation_summary(),
        authority_refresh_outcome: default_authority_refresh_outcome(),
    }
}

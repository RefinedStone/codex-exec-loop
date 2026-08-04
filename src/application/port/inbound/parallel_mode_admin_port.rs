use std::sync::Arc;

use crate::domain::parallel_mode::{
    ParallelModeReadinessSnapshot, ParallelModeRuntimeEventsSnapshot,
    ParallelModeSupervisorSnapshot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeAdminCommand {
    Enable,
    Dispatch,
    Refresh,
    Disable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeAdminCommandEffect {
    Enabled,
    DispatchRequested,
    EnabledInsteadOfDispatch,
    RefreshRequested,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeAdminStatusSnapshot {
    pub mode_enabled: bool,
    pub control_effect_in_flight: bool,
    pub current_epoch_id: Option<u64>,
    pub last_dispatch_withheld_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ParallelModeAdminDashboardSnapshot {
    pub planning_revision: Option<i64>,
    pub structured_task_count: Option<usize>,
    pub readiness: ParallelModeReadinessSnapshot,
    pub supervisor: ParallelModeSupervisorSnapshot,
    pub events: ParallelModeRuntimeEventsSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeAdminCommandOutcome {
    pub effect: ParallelModeAdminCommandEffect,
    pub status: ParallelModeAdminStatusSnapshot,
}

pub trait ParallelModeAdminPort: Send + Sync {
    fn load_dashboard_snapshot(
        &self,
        workspace_directory: &str,
        event_limit: usize,
    ) -> ParallelModeAdminDashboardSnapshot;

    fn load_runtime_events(
        &self,
        workspace_directory: &str,
        limit: usize,
        after_sequence: Option<i64>,
    ) -> ParallelModeRuntimeEventsSnapshot;

    fn load_control_status(&self) -> ParallelModeAdminStatusSnapshot;

    fn execute_control(
        &self,
        workspace_directory: &str,
        command: ParallelModeAdminCommand,
    ) -> ParallelModeAdminCommandOutcome;
}

impl<T> ParallelModeAdminPort for Arc<T>
where
    T: ParallelModeAdminPort + ?Sized,
{
    fn load_dashboard_snapshot(
        &self,
        workspace_directory: &str,
        event_limit: usize,
    ) -> ParallelModeAdminDashboardSnapshot {
        self.as_ref()
            .load_dashboard_snapshot(workspace_directory, event_limit)
    }

    fn load_runtime_events(
        &self,
        workspace_directory: &str,
        limit: usize,
        after_sequence: Option<i64>,
    ) -> ParallelModeRuntimeEventsSnapshot {
        self.as_ref()
            .load_runtime_events(workspace_directory, limit, after_sequence)
    }

    fn load_control_status(&self) -> ParallelModeAdminStatusSnapshot {
        self.as_ref().load_control_status()
    }

    fn execute_control(
        &self,
        workspace_directory: &str,
        command: ParallelModeAdminCommand,
    ) -> ParallelModeAdminCommandOutcome {
        self.as_ref().execute_control(workspace_directory, command)
    }
}

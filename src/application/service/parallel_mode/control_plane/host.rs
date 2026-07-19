use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use crate::domain::parallel_mode::{
    ParallelModeAutomationTrigger, ParallelModePostTurnQueueSignal,
};

#[cfg(test)]
use super::ParallelModeControlPlaneEffectId;
use super::controller::ParallelModeControlPlaneService;
use super::{
    ParallelModeControlPlaneBackgroundEvent, ParallelModeControlPlaneCommand,
    ParallelModeControlPlaneEventSink, ParallelModeControlPlanePresentationEvent,
    ParallelModePostTurnQueueContinuationOutcome, ParallelModeSupervisorInspectionState,
};

struct ParallelModeControlPlaneHost<S>
where
    S: ParallelModeControlPlaneEventSink,
{
    /*
     * R6 decision: this is intentionally a synchronous mutex-serialized facade,
     * not a mailbox-backed actor loop. Runtime behavior tests cover the current
     * ordering contract: in-flight effects gate new work, wakes are coalesced,
     * and stale completions are dropped by epoch before UI effects are emitted.
     * Durable dispatch commands remain the backpressure boundary. Do not add a
     * second queue actor unless those regressions stop covering real failures.
     */
    service: Mutex<ParallelModeControlPlaneService<S>>,
}

#[derive(Clone)]
pub struct ParallelModeControlPlaneHandle<S>
where
    S: ParallelModeControlPlaneEventSink,
{
    host: Arc<ParallelModeControlPlaneHost<S>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeControlPlaneEpochSnapshot {
    pub workspace_directory: Option<String>,
    pub current_epoch_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeControlPlanePresentationProjection {
    pub mode_enabled: bool,
    pub control_effect_in_flight: bool,
    pub supervisor_inspection_state: ParallelModeSupervisorInspectionState,
    pub last_dispatch_withheld_reason: Option<String>,
}

impl<S> ParallelModeControlPlaneHandle<S>
where
    S: ParallelModeControlPlaneEventSink,
{
    pub(crate) fn new(service: ParallelModeControlPlaneService<S>) -> Self {
        Self {
            host: Arc::new(ParallelModeControlPlaneHost {
                service: Mutex::new(service),
            }),
        }
    }

    fn service(&self) -> MutexGuard<'_, ParallelModeControlPlaneService<S>> {
        self.host
            .service
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn handle_command(
        &self,
        command: ParallelModeControlPlaneCommand,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        self.service().handle_command(command)
    }

    pub fn handle_background_event(
        &self,
        event: ParallelModeControlPlaneBackgroundEvent,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        self.service().handle_background_event(event)
    }

    pub fn continue_post_turn_queue(
        &self,
        workspace_directory: String,
        signal: Option<ParallelModePostTurnQueueSignal>,
        auto_follow_prompt_queued: bool,
        has_actionable_queue_head: bool,
    ) -> ParallelModePostTurnQueueContinuationOutcome {
        self.service().continue_post_turn_queue(
            workspace_directory,
            signal,
            auto_follow_prompt_queued,
            has_actionable_queue_head,
        )
    }

    pub fn tick(
        &self,
        now: Instant,
        workspace_directory: String,
        activity_pulse_visible: bool,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        self.service()
            .tick(now, workspace_directory, activity_pulse_visible)
    }

    pub fn mode_enabled(&self) -> bool {
        self.service().mode_enabled()
    }

    pub fn presentation_projection(&self) -> ParallelModeControlPlanePresentationProjection {
        let service = self.service();
        ParallelModeControlPlanePresentationProjection {
            mode_enabled: service.mode_enabled(),
            control_effect_in_flight: service.control_effect_in_flight(),
            supervisor_inspection_state: service.supervisor_inspection_state().clone(),
            last_dispatch_withheld_reason: service
                .last_dispatch_withheld_reason()
                .map(str::to_string),
        }
    }

    pub fn epoch_snapshot(&self) -> ParallelModeControlPlaneEpochSnapshot {
        let service = self.service();
        let store = service.store();
        ParallelModeControlPlaneEpochSnapshot {
            workspace_directory: store.workspace_directory.clone(),
            current_epoch_id: store.current_epoch_id,
        }
    }

    pub fn current_epoch_id_for_workspace(&self, workspace_directory: &str) -> Option<u64> {
        let snapshot = self.epoch_snapshot();
        (snapshot.workspace_directory.as_deref() == Some(workspace_directory))
            .then_some(snapshot.current_epoch_id)
            .flatten()
    }

    pub fn control_effect_in_flight(&self) -> bool {
        self.service().control_effect_in_flight()
    }

    pub fn supervisor_inspection_state(&self) -> ParallelModeSupervisorInspectionState {
        self.service().supervisor_inspection_state().clone()
    }

    #[cfg(test)]
    pub fn automation_epoch_is_active(&self, workspace_directory: &str, epoch_id: u64) -> bool {
        self.service()
            .automation_epoch_is_active(workspace_directory, epoch_id)
    }

    #[cfg(test)]
    pub fn supervisor_refresh_in_flight(&self) -> bool {
        self.service().supervisor_refresh_in_flight()
    }

    #[cfg(test)]
    pub fn orchestrator_wake_in_flight(&self) -> bool {
        self.service().orchestrator_wake_in_flight()
    }

    pub fn last_automation_trigger(&self) -> Option<ParallelModeAutomationTrigger> {
        self.service().last_automation_trigger()
    }

    pub fn last_dispatch_withheld_reason(&self) -> Option<String> {
        self.service()
            .last_dispatch_withheld_reason()
            .map(str::to_string)
    }

    pub fn clear_dispatch_withheld_reason(&self) {
        self.service().clear_dispatch_withheld_reason();
    }

    #[cfg(test)]
    pub fn force_mode_for_test(&self, workspace_directory: impl Into<String>, enabled: bool) {
        self.service()
            .force_mode_for_test(workspace_directory, enabled);
    }

    #[cfg(test)]
    pub fn force_initial_pool_reset_completed_for_test(&self, completed: bool) {
        self.service()
            .force_initial_pool_reset_completed_for_test(completed);
    }

    #[cfg(test)]
    pub fn force_epoch_for_test(&self, workspace_directory: impl Into<String>, epoch_id: u64) {
        self.service()
            .force_epoch_for_test(workspace_directory, epoch_id);
    }

    #[cfg(test)]
    pub fn force_supervisor_refresh_in_flight_for_test(
        &self,
        workspace_directory: impl Into<String>,
        epoch_id: u64,
    ) -> ParallelModeControlPlaneEffectId {
        self.service()
            .force_supervisor_refresh_in_flight_for_test(workspace_directory, epoch_id)
    }

    #[cfg(test)]
    pub fn force_parallel_entry_in_flight_for_test(
        &self,
        workspace_directory: impl Into<String>,
        epoch_id: u64,
    ) -> ParallelModeControlPlaneEffectId {
        self.service()
            .force_parallel_entry_in_flight_for_test(workspace_directory, epoch_id)
    }

    #[cfg(test)]
    pub fn force_readiness_snapshot_for_test(
        &self,
        readiness_snapshot: crate::domain::parallel_mode::ParallelModeReadinessSnapshot,
    ) {
        self.service()
            .force_readiness_snapshot_for_test(readiness_snapshot);
    }

    #[cfg(test)]
    pub fn readiness_snapshot_for_test(
        &self,
    ) -> Option<crate::domain::parallel_mode::ParallelModeReadinessSnapshot> {
        self.service().readiness_snapshot_for_test()
    }

    #[cfg(test)]
    pub fn supervisor_refresh_due(&self, now: Instant, activity_pulse_visible: bool) -> bool {
        self.service()
            .supervisor_refresh_due(now, activity_pulse_visible)
    }
}

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
    ParallelModePostTurnQueueContinuationTarget,
};

struct ParallelModeControlPlaneHost<S>
where
    S: ParallelModeControlPlaneEventSink,
{
    // The host keeps the mutable controller behind one application-owned gate.
    // Inbound adapters hold handles and cannot reach the raw controller/service.
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

    pub fn continue_post_turn_queue<T>(
        &self,
        workspace_directory: String,
        signal: Option<ParallelModePostTurnQueueSignal>,
        continuation_target: &mut T,
    ) -> Vec<ParallelModeControlPlanePresentationEvent>
    where
        T: ParallelModePostTurnQueueContinuationTarget,
    {
        self.service()
            .continue_post_turn_queue(workspace_directory, signal, continuation_target)
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
    pub fn supervisor_refresh_due(&self, now: Instant, activity_pulse_visible: bool) -> bool {
        self.service()
            .supervisor_refresh_due(now, activity_pulse_visible)
    }
}

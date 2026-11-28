use super::GatewayKernelClientRuntime;
use crate::core::app::{
    AppCommand, AppEvent, AppSnapshot, ConversationSnapshot, CoreInput, ParallelModeProjection,
    PlanningParallelProjection, RevisionedPlanningParallelProjection, SessionCatalogSnapshot,
};
use crate::core::native_client_port::{
    NativeClientDispatchOutcome, NativeClientEvent, NativeClientPort,
    NativeParallelDispatchOutcome, ParallelModeControlPlaneEpochSnapshot,
    ParallelModeControlPlanePresentationProjection,
};
use std::time::{Duration, Instant};

impl NativeClientPort for GatewayKernelClientRuntime {
    fn dispatch_client_event(&mut self, event: NativeClientEvent) -> NativeClientDispatchOutcome {
        let events = match event {
            NativeClientEvent::Core(input) => match *input {
                CoreInput::Command(AppCommand::RunStartupChecks {
                    workspace_directory,
                }) => self.startup_requested(workspace_directory),
                CoreInput::Command(AppCommand::LoadConversation { thread_id, .. }) => {
                    self.load_conversation(thread_id)
                }
                CoreInput::Command(AppCommand::InvalidateConversationLoad) => {
                    self.conversation.cancel();
                    self.snapshot.revision = self.snapshot.revision.saturating_add(1);
                    self.snapshot.conversation = ConversationSnapshot::Idle;
                    vec![AppEvent::ConversationChanged {
                        correlation: None,
                        snapshot: ConversationSnapshot::Idle,
                    }]
                }
                CoreInput::Command(AppCommand::LoadSessionCatalog(intent)) => {
                    // Initial sync ensures a load; explicit refresh requests are coalesced.
                    if (intent.mode == crate::core::app::SessionCatalogLoadMode::Refresh
                        || matches!(self.snapshot.session_catalog, SessionCatalogSnapshot::Idle))
                        && let Some(refresh) = &self.refresh
                    {
                        let _ = refresh.try_send(());
                    }
                    vec![AppEvent::SessionCatalogChanged(
                        self.snapshot.session_catalog.clone(),
                    )]
                }
                // The kernel owns every other decision; this read-only client does not send it yet.
                _ => Vec::new(),
            },
            NativeClientEvent::ParallelCommand(_)
            | NativeClientEvent::ParallelTick { .. }
            | NativeClientEvent::ParallelViewChanged(_)
            | NativeClientEvent::ClearParallelDispatchWithheldReason => {
                return NativeClientDispatchOutcome::Parallel(
                    NativeParallelDispatchOutcome::from_events(Vec::new()),
                );
            }
        };
        self.outcome(events)
    }

    #[cfg(test)]
    fn poll_pending_client_event(&mut self) -> Option<NativeClientDispatchOutcome> {
        let update = self.updates.try_recv().ok()?;
        let events = self.apply(update);
        Some(self.outcome(events))
    }

    fn run_host_cycle(
        &mut self,
        _now: Instant,
        wait: Duration,
        max_inputs: usize,
    ) -> Vec<NativeClientDispatchOutcome> {
        let mut outcomes = Vec::new();
        let mut next = if wait.is_zero() {
            self.updates.try_recv().ok()
        } else {
            self.updates.recv_timeout(wait).ok()
        };
        let mut applied = 0;
        while let Some(update) = next {
            let events = self.apply(update);
            if !events.is_empty() {
                outcomes.push(self.outcome(events));
            }
            applied += 1;
            next = if applied < max_inputs {
                self.updates.try_recv().ok()
            } else {
                None
            };
        }
        outcomes
    }

    fn snapshot(&self) -> AppSnapshot {
        self.snapshot.clone()
    }

    fn revisioned_planning_parallel_projection(&self) -> RevisionedPlanningParallelProjection {
        RevisionedPlanningParallelProjection {
            revision: 0,
            planning_parallel: PlanningParallelProjection::initial(),
        }
    }

    fn parallel_mode_projection(&self) -> ParallelModeProjection {
        ParallelModeProjection::default()
    }

    fn parallel_mode_enabled(&self) -> bool {
        false
    }

    fn parallel_control_plane_projection(&self) -> ParallelModeControlPlanePresentationProjection {
        ParallelModeControlPlanePresentationProjection {
            mode_enabled: false,
            control_effect_in_flight: false,
            last_dispatch_withheld_reason: None,
            global_runtime_notices: Vec::new(),
        }
    }

    fn parallel_epoch_snapshot(&self) -> ParallelModeControlPlaneEpochSnapshot {
        ParallelModeControlPlaneEpochSnapshot {
            workspace_directory: None,
            current_epoch_id: None,
        }
    }

    fn is_gateway_kernel_client(&self) -> bool {
        true
    }

    #[cfg(test)]
    fn current_parallel_epoch_id_for_workspace(&self, _workspace_directory: &str) -> Option<u64> {
        None
    }

    #[cfg(test)]
    fn dispatch_parallel_completion_for_test(
        &mut self,
        _event: crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneBackgroundEvent,
    ) -> NativeClientDispatchOutcome {
        unreachable!("the Gateway kernel client has no parallel control plane")
    }

    #[cfg(test)]
    fn begin_test_turn_submission(&mut self) -> crate::core::app::TurnSubmissionCorrelation {
        unreachable!("the Gateway kernel client does not submit turns")
    }

    #[cfg(test)]
    fn force_parallel_mode_for_test(&self, _workspace_directory: &str, _enabled: bool) {
        unreachable!("the Gateway kernel client has no parallel control plane")
    }

    #[cfg(test)]
    fn force_parallel_epoch_for_test(&self, _workspace_directory: &str, _epoch_id: u64) {
        unreachable!("the Gateway kernel client has no parallel control plane")
    }

    #[cfg(test)]
    fn parallel_automation_epoch_is_active_for_test(
        &self,
        _workspace_directory: &str,
        _epoch_id: u64,
    ) -> bool {
        false
    }

    #[cfg(test)]
    fn recv_parallel_completion_for_test(
        &self,
        _timeout: Duration,
    ) -> crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneBackgroundEvent{
        unreachable!("the Gateway kernel client has no parallel control plane")
    }
}

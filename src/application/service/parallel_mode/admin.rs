use std::sync::{Arc, Mutex, mpsc};

use crate::application::port::inbound::parallel_mode_admin_port::{
    ParallelModeAdminCommand, ParallelModeAdminCommandEffect, ParallelModeAdminCommandOutcome,
    ParallelModeAdminDashboardSnapshot, ParallelModeAdminPort, ParallelModeAdminStatusSnapshot,
};
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogRequest;
use crate::application::service::planning::PlanningApplicationProjection;
use crate::domain::parallel_mode::ParallelModeAutomationTrigger;

use super::control_plane::{
    ParallelModeControlPlaneBackgroundEvent, ParallelModeControlPlaneCommand,
    ParallelModeControlPlaneComposition, ParallelModeControlPlaneEventSink,
    ParallelModeControlPlaneHandle,
};

#[derive(Clone)]
struct ParallelModeAdminEventSink {
    tx: mpsc::Sender<ParallelModeControlPlaneBackgroundEvent>,
}

impl ParallelModeControlPlaneEventSink for ParallelModeAdminEventSink {
    fn send_control_plane_event(&self, event: ParallelModeControlPlaneBackgroundEvent) {
        let _ = self.tx.send(event);
    }
}

#[derive(Clone)]
pub struct ParallelModeAdminService {
    composition: Arc<ParallelModeControlPlaneComposition>,
    handle: ParallelModeControlPlaneHandle<ParallelModeAdminEventSink>,
    pending_events: Arc<Mutex<mpsc::Receiver<ParallelModeControlPlaneBackgroundEvent>>>,
}

impl ParallelModeAdminService {
    pub fn new(composition: Arc<ParallelModeControlPlaneComposition>) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = composition.bind_event_sink(ParallelModeAdminEventSink { tx });
        Self {
            composition,
            handle,
            pending_events: Arc::new(Mutex::new(rx)),
        }
    }

    fn drain_pending_events(&self) {
        let receiver = self
            .pending_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while let Ok(event) = receiver.try_recv() {
            let _ = self.handle.handle_background_event(event);
        }
    }

    fn status_after_drain(&self) -> ParallelModeAdminStatusSnapshot {
        self.drain_pending_events();
        let projection = self.handle.presentation_projection();
        let epoch = self.handle.epoch_snapshot();
        ParallelModeAdminStatusSnapshot {
            mode_enabled: projection.mode_enabled,
            control_effect_in_flight: projection.control_effect_in_flight,
            current_epoch_id: epoch.current_epoch_id,
            last_dispatch_withheld_reason: projection.last_dispatch_withheld_reason,
        }
    }
}

impl ParallelModeAdminPort for ParallelModeAdminService {
    fn load_dashboard_snapshot(
        &self,
        workspace_directory: &str,
        event_limit: usize,
    ) -> ParallelModeAdminDashboardSnapshot {
        let runtime_projection = self
            .composition
            .planning()
            .runtime
            .load_runtime_projection_or_invalid(workspace_directory);
        let planning_projection =
            PlanningApplicationProjection::from_runtime_projection(&runtime_projection);
        let snapshot = self.composition.inspect_dashboard_snapshot_from_projection(
            workspace_directory,
            &planning_projection,
            ParallelModeRuntimeEventLogRequest::recent(event_limit),
        );
        ParallelModeAdminDashboardSnapshot {
            planning_revision: planning_projection.planning_revision,
            structured_task_count: planning_projection
                .has_structured_queue_projection
                .then_some(planning_projection.visible_tasks.len()),
            readiness: snapshot.readiness,
            supervisor: snapshot.supervisor,
            events: snapshot.events,
        }
    }

    fn load_runtime_events(
        &self,
        workspace_directory: &str,
        limit: usize,
        after_sequence: Option<i64>,
    ) -> crate::domain::parallel_mode::ParallelModeRuntimeEventsSnapshot {
        let request = match after_sequence {
            Some(sequence) => {
                ParallelModeRuntimeEventLogRequest::recent(limit).after_sequence(sequence)
            }
            None => ParallelModeRuntimeEventLogRequest::recent(limit),
        };
        self.composition
            .build_runtime_events_snapshot(workspace_directory, request)
    }

    fn load_control_status(&self) -> ParallelModeAdminStatusSnapshot {
        self.status_after_drain()
    }

    fn execute_control(
        &self,
        workspace_directory: &str,
        command: ParallelModeAdminCommand,
    ) -> ParallelModeAdminCommandOutcome {
        self.drain_pending_events();
        let workspace_directory = workspace_directory.to_string();
        let effect = match command {
            ParallelModeAdminCommand::Enable => {
                let _ = self
                    .handle
                    .handle_command(ParallelModeControlPlaneCommand::Enable {
                        workspace_directory,
                    });
                ParallelModeAdminCommandEffect::Enabled
            }
            ParallelModeAdminCommand::Dispatch if self.handle.mode_enabled() => {
                let _ =
                    self.handle
                        .handle_command(ParallelModeControlPlaneCommand::RequestDispatch {
                            workspace_directory,
                            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
                        });
                ParallelModeAdminCommandEffect::DispatchRequested
            }
            ParallelModeAdminCommand::Dispatch => {
                let _ = self
                    .handle
                    .handle_command(ParallelModeControlPlaneCommand::Enable {
                        workspace_directory,
                    });
                ParallelModeAdminCommandEffect::EnabledInsteadOfDispatch
            }
            ParallelModeAdminCommand::Refresh => {
                let _ = self.handle.handle_command(
                    ParallelModeControlPlaneCommand::InspectSupervisor {
                        workspace_directory,
                        reconcile_pool: self.handle.mode_enabled(),
                        show_status: false,
                    },
                );
                ParallelModeAdminCommandEffect::RefreshRequested
            }
            ParallelModeAdminCommand::Disable => {
                let _ = self
                    .handle
                    .handle_command(ParallelModeControlPlaneCommand::Disable {
                        workspace_directory,
                    });
                ParallelModeAdminCommandEffect::Disabled
            }
        };
        ParallelModeAdminCommandOutcome {
            effect,
            status: self.status_after_drain(),
        }
    }
}

use std::sync::Arc;
use std::sync::mpsc;

use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogRequest;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::parallel_mode::{
    ParallelModeOrchestratorTickResult, ParallelModeOrchestratorTrigger, ParallelModeService,
};
use crate::application::service::planning::{PlanningApplicationProjection, PlanningServices};
use crate::domain::parallel_mode::{
    ParallelModeOrchestratorStateMachine, ParallelModeReadinessSnapshot,
    ParallelModeRuntimeEventsSnapshot, ParallelModeSupervisorSnapshot,
};

use super::controller::ParallelModeControlPlaneService;
use super::{
    ParallelModeControlPlaneBackgroundEvent, ParallelModeControlPlaneCommand,
    ParallelModeControlPlaneEffectRunner, ParallelModeControlPlaneEventSink,
    ParallelModeControlPlaneHandle,
};

#[derive(Clone)]
pub struct ParallelModeControlPlaneComposition {
    parallel_mode_service: ParallelModeService,
    planning: PlanningServices,
    worker_port: Arc<dyn ParallelAgentWorkerPort>,
}

pub struct ParallelModeControlPlaneDashboardSnapshot {
    pub readiness: ParallelModeReadinessSnapshot,
    pub supervisor: ParallelModeSupervisorSnapshot,
    pub events: ParallelModeRuntimeEventsSnapshot,
}

#[derive(Clone)]
struct BlockingParallelModeControlPlaneEventSink {
    tx: mpsc::Sender<ParallelModeControlPlaneBackgroundEvent>,
}

impl ParallelModeControlPlaneEventSink for BlockingParallelModeControlPlaneEventSink {
    fn send_control_plane_event(&self, event: ParallelModeControlPlaneBackgroundEvent) {
        let _ = self.tx.send(event);
    }
}

impl ParallelModeControlPlaneComposition {
    pub fn new(
        parallel_mode_service: ParallelModeService,
        planning: PlanningServices,
        worker_port: Arc<dyn ParallelAgentWorkerPort>,
    ) -> Self {
        Self {
            parallel_mode_service,
            planning,
            worker_port,
        }
    }

    pub fn parallel_mode_turn_service(&self) -> ParallelModeTurnService {
        ParallelModeTurnService::new(self.parallel_mode_service.clone())
    }

    pub fn planning(&self) -> &PlanningServices {
        &self.planning
    }

    pub fn inspect_dashboard_snapshot_from_projection(
        &self,
        workspace_directory: &str,
        planning_projection: &PlanningApplicationProjection,
        event_request: ParallelModeRuntimeEventLogRequest,
    ) -> ParallelModeControlPlaneDashboardSnapshot {
        let readiness = self
            .parallel_mode_service
            .inspect_readiness_from_planning_projection(workspace_directory, planning_projection);
        let supervisor = self.parallel_mode_service.build_supervisor_snapshot(
            workspace_directory,
            true,
            Some(&readiness),
        );
        let events = self
            .parallel_mode_service
            .build_runtime_events_snapshot(workspace_directory, event_request);
        ParallelModeControlPlaneDashboardSnapshot {
            readiness,
            supervisor,
            events,
        }
    }

    pub fn inspect_dashboard_snapshot(
        &self,
        workspace_directory: &str,
        event_request: ParallelModeRuntimeEventLogRequest,
    ) -> ParallelModeControlPlaneDashboardSnapshot {
        let runtime_snapshot = self
            .planning
            .runtime
            .load_runtime_snapshot_or_invalid(workspace_directory);
        self.inspect_dashboard_snapshot_from_projection(
            workspace_directory,
            &PlanningApplicationProjection::from_runtime_snapshot(&runtime_snapshot),
            event_request,
        )
    }

    pub fn build_runtime_events_snapshot(
        &self,
        workspace_directory: &str,
        event_request: ParallelModeRuntimeEventLogRequest,
    ) -> ParallelModeRuntimeEventsSnapshot {
        self.parallel_mode_service
            .build_runtime_events_snapshot(workspace_directory, event_request)
    }

    pub fn run_manual_orchestrator_tick(
        &self,
        workspace_directory: &str,
    ) -> Result<ParallelModeOrchestratorTickResult, String> {
        let (tx, rx) = mpsc::channel();
        let handle = self.bind_event_sink(BlockingParallelModeControlPlaneEventSink { tx });
        let workspace_directory = workspace_directory.to_string();
        let _ = handle.handle_command(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: workspace_directory.clone(),
        });
        let _ = handle.handle_command(ParallelModeControlPlaneCommand::RunOrchestratorTick {
            workspace_directory: workspace_directory.clone(),
            signature: "manual-dispatch".to_string(),
        });

        loop {
            let event = rx
                .recv()
                .map_err(|error| format!("manual control-plane tick did not complete: {error}"))?;
            if let ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
                blocked,
                notices,
                ..
            } = event
            {
                return Ok(ParallelModeOrchestratorTickResult {
                    trigger: ParallelModeOrchestratorTrigger::ManualDispatch,
                    state: ParallelModeOrchestratorStateMachine::tick_state(blocked),
                    blocked,
                    notices,
                });
            }
            let _ = handle.handle_background_event(event);
        }
    }

    pub fn bind_event_sink<S>(&self, event_sink: S) -> ParallelModeControlPlaneHandle<S>
    where
        S: ParallelModeControlPlaneEventSink,
    {
        let service =
            ParallelModeControlPlaneService::new(ParallelModeControlPlaneEffectRunner::new(
                self.parallel_mode_service.clone(),
                self.planning.clone(),
                self.worker_port.clone(),
                ParallelModeTurnService::new(self.parallel_mode_service.clone()),
                event_sink,
            ));
        ParallelModeControlPlaneHandle::new(service)
    }
}

use std::sync::Arc;

use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::PlanningServices;

use super::{
    ParallelModeControlPlaneEffectRunner, ParallelModeControlPlaneEventSink,
    ParallelModeControlPlaneService,
};

#[derive(Clone)]
pub struct ParallelModeControlPlaneComposition {
    parallel_mode_service: ParallelModeService,
    planning: PlanningServices,
    worker_port: Arc<dyn ParallelAgentWorkerPort>,
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

    pub fn parallel_mode_service(&self) -> &ParallelModeService {
        &self.parallel_mode_service
    }

    pub fn planning(&self) -> &PlanningServices {
        &self.planning
    }

    pub fn bind_event_sink<S>(&self, event_sink: S) -> ParallelModeControlPlaneService<S>
    where
        S: ParallelModeControlPlaneEventSink,
    {
        ParallelModeControlPlaneService::new(ParallelModeControlPlaneEffectRunner::new(
            self.parallel_mode_service.clone(),
            self.planning.clone(),
            self.worker_port.clone(),
            ParallelModeTurnService::new(self.parallel_mode_service.clone()),
            event_sink,
        ))
    }
}

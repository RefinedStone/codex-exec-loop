use crate::application::service::planning::PlanningRuntimeProjection;
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningParallelProjection {
    pub planning_runtime: Box<PlanningRuntimeProjection>,
    pub parallel_mode: ParallelModeProjection,
}

impl PlanningParallelProjection {
    pub fn initial() -> Self {
        Self {
            planning_runtime: Box::new(PlanningRuntimeProjection::uninitialized()),
            parallel_mode: ParallelModeProjection::default(),
        }
    }

    pub fn apply_planning_runtime_projection(
        &mut self,
        projection: Box<PlanningRuntimeProjection>,
    ) -> bool {
        if self.planning_runtime == projection {
            return false;
        }
        self.planning_runtime = projection;
        true
    }

    pub fn apply_parallel_readiness_snapshot(
        &mut self,
        snapshot: Option<Box<ParallelModeReadinessSnapshot>>,
    ) -> bool {
        if self.parallel_mode.readiness == snapshot {
            return false;
        }
        self.parallel_mode.readiness = snapshot;
        true
    }

    pub fn apply_parallel_supervisor_snapshot(
        &mut self,
        snapshot: Option<Box<ParallelModeSupervisorSnapshot>>,
    ) -> bool {
        if self.parallel_mode.supervisor == snapshot {
            return false;
        }
        self.parallel_mode.supervisor = snapshot;
        true
    }
}

impl Default for PlanningParallelProjection {
    fn default() -> Self {
        Self::initial()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParallelModeProjection {
    pub readiness: Option<Box<ParallelModeReadinessSnapshot>>,
    pub supervisor: Option<Box<ParallelModeSupervisorSnapshot>>,
}

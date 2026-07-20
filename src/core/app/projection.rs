use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};
use crate::domain::planning::RuntimeProjection;

/*
 * High-frequency inbound projections often need the planning/parallel facts and
 * the Core revision, but not startup diagnostics, the session catalog, or the
 * full conversation transcript. Keeping that coherent slice as one owned value
 * avoids rebuilding a full AppSnapshot while preserving single-sample
 * semantics for consumers.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionedPlanningParallelProjection {
    pub revision: u64,
    pub planning_parallel: PlanningParallelProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningParallelProjection {
    pub planning_runtime_workspace_directory: Option<String>,
    pub planning_runtime: Box<RuntimeProjection>,
    pub parallel_mode: ParallelModeProjection,
}

impl PlanningParallelProjection {
    pub fn initial() -> Self {
        Self {
            planning_runtime_workspace_directory: None,
            planning_runtime: Box::new(RuntimeProjection::uninitialized()),
            parallel_mode: ParallelModeProjection::default(),
        }
    }

    pub fn apply_planning_runtime_projection(
        &mut self,
        workspace_directory: String,
        projection: Box<RuntimeProjection>,
    ) -> bool {
        if self.planning_runtime_workspace_directory.as_ref() == Some(&workspace_directory)
            && self.planning_runtime == projection
        {
            return false;
        }
        self.planning_runtime_workspace_directory = Some(workspace_directory);
        self.planning_runtime = projection;
        true
    }

    pub fn clear_planning_runtime_projection(&mut self) -> bool {
        if self.planning_runtime_workspace_directory.is_none()
            && *self.planning_runtime == RuntimeProjection::uninitialized()
        {
            return false;
        }
        self.planning_runtime_workspace_directory = None;
        *self.planning_runtime = RuntimeProjection::uninitialized();
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

#[cfg(test)]
mod tests {
    use super::PlanningParallelProjection;
    use crate::domain::planning::RuntimeProjection;

    #[test]
    fn default_projection_matches_initial_uninitialized_state() {
        assert_eq!(
            PlanningParallelProjection::default(),
            PlanningParallelProjection::initial()
        );
        assert_eq!(
            *PlanningParallelProjection::default().planning_runtime,
            RuntimeProjection::uninitialized()
        );
        assert!(
            PlanningParallelProjection::default()
                .planning_runtime_workspace_directory
                .is_none()
        );
    }

    #[test]
    fn projection_identity_includes_its_workspace() {
        let mut projection = PlanningParallelProjection::default();
        let runtime = Box::new(RuntimeProjection::invalid("blocked"));

        assert!(
            projection.apply_planning_runtime_projection("/tmp/root".to_string(), runtime.clone())
        );
        assert!(
            !projection.apply_planning_runtime_projection("/tmp/root".to_string(), runtime.clone())
        );
        assert!(projection.apply_planning_runtime_projection("/tmp/slot".to_string(), runtime));
        assert_eq!(
            projection.planning_runtime_workspace_directory.as_deref(),
            Some("/tmp/slot")
        );

        assert!(projection.clear_planning_runtime_projection());
        assert!(!projection.clear_planning_runtime_projection());
        assert_eq!(projection, PlanningParallelProjection::initial());
    }

    #[test]
    fn applying_same_empty_parallel_supervisor_snapshot_is_idempotent() {
        let mut projection = PlanningParallelProjection::default();

        assert!(!projection.apply_parallel_supervisor_snapshot(None));
        assert!(projection.parallel_mode.supervisor.is_none());
    }
}

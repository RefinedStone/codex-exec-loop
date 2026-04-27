use anyhow::Result;

use super::projection::{map_directions_summary, map_doctor_report, map_runtime_snapshot};
use super::{PlanningAdminFacadeService, PlanningAdminOverview, PlanningAdminRuntimeSummary};

impl PlanningAdminFacadeService {
    pub fn load_overview(&self) -> Result<PlanningAdminOverview> {
        let doctor = self
            .planning
            .workspace
            .inspect_workspace(self.workspace_dir.as_str());
        let directions = self
            .planning
            .workspace
            .load_summary(self.workspace_dir.as_str())
            .ok()
            .map(map_directions_summary);

        Ok(PlanningAdminOverview {
            workspace_dir: self.workspace_dir.clone(),
            doctor: map_doctor_report(&doctor),
            runtime: self.load_runtime_summary()?,
            directions,
        })
    }

    pub fn load_runtime_summary(&self) -> Result<PlanningAdminRuntimeSummary> {
        let runtime = self
            .planning
            .runtime
            .load_runtime_snapshot_or_invalid(self.workspace_dir.as_str());
        Ok(map_runtime_snapshot(&runtime))
    }
}

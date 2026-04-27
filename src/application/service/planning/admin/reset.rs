use anyhow::Result;

use crate::application::service::planning::PlanningResetTarget;

use super::projection::map_doctor_report;
use super::{PlanningAdminFacadeService, PlanningAdminResetOutcome};

impl PlanningAdminFacadeService {
    pub fn reset_workspace(
        &self,
        target: PlanningResetTarget,
    ) -> Result<PlanningAdminResetOutcome> {
        let result = self
            .planning
            .workspace
            .reset_workspace(self.workspace_dir.as_str(), target)?;
        let doctor = self
            .planning
            .workspace
            .inspect_workspace(self.workspace_dir.as_str());
        Ok(PlanningAdminResetOutcome {
            target: result.target.label().to_string(),
            rewritten_paths: result.rewritten_paths,
            removed_paths: result.removed_paths,
            doctor: map_doctor_report(&doctor),
        })
    }
}

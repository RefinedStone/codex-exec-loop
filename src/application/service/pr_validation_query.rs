use std::sync::Arc;

use anyhow::{Result, bail};

use crate::application::port::inbound::pr_validation_query_port::{
    PrValidationQueryPort, PrValidationStatusRequest,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::PrValidationOperatorSummary;

pub struct PrValidationQueryService {
    workspace_dir: String,
    planning_authority: Arc<dyn PlanningAuthorityPort>,
}

impl PrValidationQueryService {
    pub fn new(
        workspace_dir: impl Into<String>,
        planning_authority: Arc<dyn PlanningAuthorityPort>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            planning_authority,
        }
    }
}

impl PrValidationQueryPort for PrValidationQueryService {
    fn status_for_pr(
        &self,
        request: PrValidationStatusRequest,
    ) -> Result<Option<PrValidationOperatorSummary>> {
        if request.pull_request_number == 0 {
            bail!("pull request number must be positive");
        }
        self.planning_authority
            .load_runtime_pr_validation_record_for_pr(
                &self.workspace_dir,
                request.pull_request_number,
            )
            .map(|record| record.map(|record| record.operator_summary()))
    }
}

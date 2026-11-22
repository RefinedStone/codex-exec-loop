use std::sync::Arc;

use anyhow::{Context, Result, bail};

use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkOrderRepositoryPort, PlanningWorkPermitDecision, PlanningWorkPermitStage,
};
use crate::application::service::planning::task_mutation::work_order::{
    PlanningSupplementalWorkRequest, PlanningWorkOrderService,
};
use crate::domain::planning::PriorityQueueService;

use super::{PlanningIdempotentTaskAdmission, PlanningQueueUseCases, PlanningTaskCreateInput};

impl PlanningQueueUseCases {
    pub fn with_work_order_repository(
        mut self,
        repository: Arc<dyn PlanningWorkOrderRepositoryPort>,
    ) -> Self {
        self.work_orders = Some(repository);
        self
    }

    /// A recovery successor reuses its first record's admission identity. Ordinary records
    /// return None and retain their existing key unchanged.
    pub fn validation_remediation_origin(
        &self,
        workspace: &str,
        validation_record_key: &str,
    ) -> Result<Option<String>> {
        match &self.work_orders {
            Some(repository) => repository
                .load_work_order_validation_recovery_origin(workspace, validation_record_key),
            None => Ok(None),
        }
    }

    pub fn admit_validation_remediation_once(
        &self,
        workspace: &str,
        validation_record_key: &str,
        admission_key: &str,
        input: PlanningTaskCreateInput,
    ) -> Result<PlanningIdempotentTaskAdmission> {
        if let Some(repository) = &self.work_orders {
            let source = repository
                .load_work_order_validation_source(workspace, validation_record_key)?
                .context(
                    "validation source is unavailable; restore its correlation before remediation",
                )?;
            if let Some(work_order_id) = source.work_order_id {
                let admission =
                    PlanningWorkOrderService::new(repository.clone(), PriorityQueueService::new())
                        .admit_supplemental_task_once(PlanningSupplementalWorkRequest {
                            workspace_directory: workspace.to_owned(),
                            work_order_id,
                            admission_key: admission_key.to_owned(),
                            input,
                        })?;
                return Ok(PlanningIdempotentTaskAdmission {
                    task_id: admission.task_id,
                    created: admission.created,
                });
            }
            // This persisted legacy source cannot later be rebound to a work order.
            return self
                .task_mutation
                .admit_system_task_once(workspace, admission_key, input);
        }
        // Older/fake composition may omit the new port. Only a still-present source queue
        // and an explicit Legacy permit decision can establish compatibility.
        let projections = self.authority.load_runtime_projections(workspace)?;
        let mut sources = projections
            .distributor_queue_records
            .iter()
            .filter(|record| record.queue_item_id == validation_record_key);
        let source = sources
            .next()
            .context("validation source is unavailable without work-order storage")?;
        if sources.next().is_some() {
            bail!("validation source is ambiguous");
        }
        if self.authority.check_work_order_permit_for_task(
            workspace,
            &source.task_id,
            PlanningWorkPermitStage::Dispatch,
            None,
        )? != PlanningWorkPermitDecision::Legacy
        {
            bail!("work-order remediation requires atomic supplemental admission storage");
        }
        self.task_mutation
            .admit_system_task_once(workspace, admission_key, input)
    }
}

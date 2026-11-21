mod supplemental;
use std::sync::Arc;
pub use supplemental::PlanningSupplementalWorkRequest;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityMutationAudit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkOrderLinkResult, PlanningWorkOrderRecord, PlanningWorkOrderRegistration,
    PlanningWorkOrderRegistrationResult, PlanningWorkOrderRepositoryPort, validate_work_order_id,
    validate_work_order_identity, validate_work_order_task_id,
};
use crate::domain::planning::{OriginSessionKind, PriorityQueueService, TaskMutationProvenance};

use super::{
    MAX_COLLISION_SUFFIX_ATTEMPTS, MAX_REVISION_CONFLICT_RETRIES, PlanningTaskCreateInput,
    PlanningTaskMutationService, PlanningTaskMutationSource, TaskMutationAuditContext,
    task_id_exists,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkOrderRequest {
    pub workspace_directory: String,
    pub work_order_id: String,
    pub origin: String,
    pub input: PlanningTaskCreateInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkOrderAdmission {
    pub work_order: PlanningWorkOrderRecord,
    /// Historical original identity, including after an operator removes or resets the task.
    pub task_id: String,
    pub created: bool,
}

/// Creates ordinary planning tasks while keeping their immutable work-order identity in the
/// same authority transaction. No issue-module types, execution policy, or receipt cache enter here.
#[derive(Clone)]
pub struct PlanningWorkOrderService {
    repository: Arc<dyn PlanningWorkOrderRepositoryPort>,
    task_mutation: PlanningTaskMutationService,
}

impl PlanningWorkOrderService {
    pub fn new(
        repository: Arc<dyn PlanningWorkOrderRepositoryPort>,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        let task_repository: Arc<dyn PlanningTaskRepositoryPort> = repository.clone();
        Self {
            repository,
            task_mutation: PlanningTaskMutationService::new(
                task_repository,
                priority_queue_service,
            ),
        }
    }

    pub fn register_work_order(
        &self,
        request: PlanningWorkOrderRequest,
    ) -> Result<PlanningWorkOrderAdmission> {
        self.register_work_order_at(request, Utc::now())
    }

    fn register_work_order_at(
        &self,
        request: PlanningWorkOrderRequest,
        generated_at: DateTime<Utc>,
    ) -> Result<PlanningWorkOrderAdmission> {
        validate_work_order_identity(&request.work_order_id, &request.origin)?;
        let provenance = TaskMutationProvenance::new(OriginSessionKind::System);
        let mut revision_conflicts = 0;
        let mut collisions = 0;
        let mut collision_suffix = None;

        loop {
            // Do this before loading authority or validating task input. A replay must survive
            // provenance edits, receipt pruning, and deliberate removal of the original task.
            if let Some(existing) = self.existing_admission(&request)? {
                return Ok(existing);
            }

            let context = match self
                .task_mutation
                .load_context(&request.workspace_directory)
            {
                Ok(context) => context,
                Err(error) => {
                    // Another writer may have registered and then reset authority after our read.
                    if let Some(existing) = self.existing_admission(&request)? {
                        return Ok(existing);
                    }
                    return Err(error);
                }
            };
            let task = self.task_mutation.build_task(
                &request.input,
                TaskMutationAuditContext {
                    source: PlanningTaskMutationSource::System,
                    legacy_source_turn_id: None,
                    provenance: &provenance,
                },
                &context.directions,
                generated_at,
                collision_suffix,
            )?;
            let task_id = task.id.clone();
            if task_id_exists(&context.task_authority, &task_id) {
                collisions += 1;
                if collisions >= MAX_COLLISION_SUFFIX_ATTEMPTS {
                    bail!("work order could not allocate a unique planning task id");
                }
                collision_suffix = self
                    .task_mutation
                    .task_id_policy
                    .next_collision_suffix(collision_suffix);
                continue;
            }

            let mut candidate = context.task_authority;
            candidate.tasks.push(task);
            let queue_projection = self
                .task_mutation
                .validate_and_project(&context.directions, &candidate)?;
            let result = self.repository.register_work_order(
                &request.workspace_directory,
                PlanningWorkOrderRegistration {
                    work_order_id: &request.work_order_id,
                    origin: &request.origin,
                    initial_task_id: &task_id,
                    observed_planning_revision: context.task_planning_revision,
                    task_authority: &candidate,
                    queue_projection: &queue_projection,
                    audit: PlanningTaskAuthorityMutationAudit {
                        task_ids: std::slice::from_ref(&task_id),
                        legacy_source_turn_id: None,
                        provenance: &provenance,
                    },
                },
            )?;
            match result {
                PlanningWorkOrderRegistrationResult::Registered { work_order, .. } => {
                    return admission(&request, work_order, true);
                }
                PlanningWorkOrderRegistrationResult::Existing { work_order } => {
                    return admission(&request, work_order, false);
                }
                PlanningWorkOrderRegistrationResult::RevisionConflict { .. } => {
                    revision_conflicts += 1;
                    if revision_conflicts > MAX_REVISION_CONFLICT_RETRIES {
                        bail!("work order could not commit because planning state kept changing");
                    }
                }
                PlanningWorkOrderRegistrationResult::TaskIdConflict {
                    task_id: rejected_id,
                } => {
                    if rejected_id != task_id {
                        bail!("work order repository returned a different task identity conflict");
                    }
                    collisions += 1;
                    if collisions >= MAX_COLLISION_SUFFIX_ATTEMPTS {
                        bail!("work order could not allocate a unique planning task id");
                    }
                    collision_suffix = self
                        .task_mutation
                        .task_id_policy
                        .next_collision_suffix(collision_suffix);
                }
            }
        }
    }

    fn existing_admission(
        &self,
        request: &PlanningWorkOrderRequest,
    ) -> Result<Option<PlanningWorkOrderAdmission>> {
        self.repository
            .load_work_order(&request.workspace_directory, &request.work_order_id)?
            .map(|work_order| admission(request, work_order, false))
            .transpose()
    }

    pub fn load_work_order(
        &self,
        workspace_directory: &str,
        work_order_id: &str,
    ) -> Result<Option<PlanningWorkOrderRecord>> {
        validate_work_order_id(work_order_id)?;
        validated_record(
            self.repository
                .load_work_order(workspace_directory, work_order_id)?,
        )
    }

    pub fn load_work_order_for_task(
        &self,
        workspace_directory: &str,
        task_id: &str,
    ) -> Result<Option<PlanningWorkOrderRecord>> {
        validate_work_order_task_id(task_id)?;
        validated_record(
            self.repository
                .load_work_order_for_task(workspace_directory, task_id)?,
        )
    }

    pub fn link_supplemental_task(
        &self,
        workspace_directory: &str,
        work_order_id: &str,
        task_id: &str,
    ) -> Result<PlanningWorkOrderLinkResult> {
        validate_work_order_id(work_order_id)?;
        validate_work_order_task_id(task_id)?;
        let result =
            self.repository
                .link_supplemental_task(workspace_directory, work_order_id, task_id)?;
        result.work_order.validate()?;
        Ok(result)
    }
}

fn admission(
    request: &PlanningWorkOrderRequest,
    work_order: PlanningWorkOrderRecord,
    created: bool,
) -> Result<PlanningWorkOrderAdmission> {
    work_order.validate()?;
    if work_order.work_order_id != request.work_order_id {
        bail!("work order repository returned a different work order identity");
    }
    work_order.ensure_origin(&request.origin)?;
    Ok(PlanningWorkOrderAdmission {
        task_id: work_order.initial_task_id()?.to_string(),
        work_order,
        created,
    })
}

fn validated_record(
    work_order: Option<PlanningWorkOrderRecord>,
) -> Result<Option<PlanningWorkOrderRecord>> {
    if let Some(record) = work_order.as_ref() {
        record.validate()?;
    }
    Ok(work_order)
}

#[cfg(test)]
mod tests;

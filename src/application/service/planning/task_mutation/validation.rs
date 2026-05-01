use std::collections::HashSet;

use anyhow::{Context, Result};

use crate::domain::planning::{
    DirectionCatalogDocument, PlanningSemanticValidationService, PlanningValidationReport,
    PriorityQueueProjection, TaskAuthorityDocument,
};

use super::PlanningTaskMutationService;
use super::helpers::{reject_task_validation_errors, validate_priorities, validate_task_reference};

impl PlanningTaskMutationService {
    pub(super) fn validate_and_project(
        &self,
        directions: &DirectionCatalogDocument,
        task_authority: &TaskAuthorityDocument,
    ) -> Result<PriorityQueueProjection> {
        let mut report = PlanningValidationReport::new();
        PlanningSemanticValidationService::new().validate(
            Some(directions),
            Some(task_authority),
            &mut report,
        );
        reject_task_validation_errors(&report)?;
        self.validate_task_links(task_authority)?;
        validate_priorities(task_authority)?;
        self.priority_queue_service
            .build_projection(directions, task_authority)
            .context("failed to rebuild planning queue projection")
    }

    fn validate_task_links(&self, task_authority: &TaskAuthorityDocument) -> Result<()> {
        let task_ids = task_authority
            .tasks
            .iter()
            .map(|task| task.id.trim().to_string())
            .collect::<HashSet<_>>();
        for task in &task_authority.tasks {
            let task_id = task.id.trim();
            for dependency_id in &task.depends_on {
                validate_task_reference("dependency", task_id, dependency_id, &task_ids)?;
            }
            for blocker_id in &task.blocked_by {
                validate_task_reference("blocker", task_id, blocker_id, &task_ids)?;
            }
        }
        Ok(())
    }
}

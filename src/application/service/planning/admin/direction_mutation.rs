use std::collections::BTreeSet;

use anyhow::{Result, bail};

use super::documents::{
    DEFAULT_DIRECTION_ID, direction_from_request, ensure_default_direction, normalized_required_id,
    remove_task_references,
};
use super::{
    PlanningAdminDirectionDeleteRequest, PlanningAdminDirectionMutationRequest,
    PlanningAdminFacadeService,
};
use crate::domain::planning::TaskAuthorityDocument;

pub(super) struct PlanningAdminDirectionMutationService<'a> {
    facade: &'a PlanningAdminFacadeService,
}

#[derive(Debug, Clone)]
pub(super) enum PlanningAdminDirectionMutationCommand {
    Upsert(PlanningAdminDirectionMutationRequest),
    Delete(PlanningAdminDirectionDeleteRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningAdminDirectionMutationOutcome {
    pub(super) direction_id: String,
    pub(super) updated: bool,
    pub(super) deleted: bool,
    pub(super) removed_task_count: usize,
}

impl<'a> PlanningAdminDirectionMutationService<'a> {
    pub(super) fn new(facade: &'a PlanningAdminFacadeService) -> Self {
        Self { facade }
    }

    pub(super) fn apply(
        &self,
        command: PlanningAdminDirectionMutationCommand,
    ) -> Result<PlanningAdminDirectionMutationOutcome> {
        match command {
            PlanningAdminDirectionMutationCommand::Upsert(request) => self.upsert(request),
            PlanningAdminDirectionMutationCommand::Delete(request) => self.delete(request),
        }
    }

    fn upsert(
        &self,
        request: PlanningAdminDirectionMutationRequest,
    ) -> Result<PlanningAdminDirectionMutationOutcome> {
        let mut documents = self.facade.load_operator_planning_documents()?;
        let direction = direction_from_request(request, &documents.directions)?;
        let direction_id = direction.id.clone();
        let updated = if let Some(existing) = documents
            .directions
            .directions
            .iter_mut()
            .find(|existing| existing.id.trim() == direction_id)
        {
            *existing = direction;
            true
        } else {
            documents.directions.directions.push(direction);
            false
        };
        self.facade.commit_operator_planning_documents(documents)?;

        Ok(PlanningAdminDirectionMutationOutcome {
            direction_id,
            updated,
            deleted: false,
            removed_task_count: 0,
        })
    }

    fn delete(
        &self,
        request: PlanningAdminDirectionDeleteRequest,
    ) -> Result<PlanningAdminDirectionMutationOutcome> {
        let direction_id = normalized_required_id(&request.id, "direction id")?.to_string();
        let mut documents = self.facade.load_operator_planning_documents()?;
        if direction_id == DEFAULT_DIRECTION_ID {
            ensure_default_direction(&mut documents.directions)?;
            self.facade.commit_operator_planning_documents(documents)?;
            return Ok(PlanningAdminDirectionMutationOutcome {
                direction_id,
                updated: false,
                deleted: false,
                removed_task_count: 0,
            });
        }

        let original_count = documents.directions.directions.len();
        documents
            .directions
            .directions
            .retain(|direction| direction.id.trim() != direction_id);
        if documents.directions.directions.len() == original_count {
            bail!("direction `{direction_id}` was not found");
        }

        let removed_task_ids =
            remove_tasks_for_direction(&mut documents.task_authority, &direction_id);
        remove_task_references(&mut documents.task_authority, &removed_task_ids);

        let removed_task_count = removed_task_ids.len();
        ensure_default_direction(&mut documents.directions)?;
        self.facade.commit_operator_planning_documents(documents)?;

        Ok(PlanningAdminDirectionMutationOutcome {
            direction_id,
            updated: false,
            deleted: true,
            removed_task_count,
        })
    }
}

fn remove_tasks_for_direction(
    task_authority: &mut TaskAuthorityDocument,
    direction_id: &str,
) -> BTreeSet<String> {
    let mut removed_task_ids = BTreeSet::new();
    task_authority.tasks.retain(|task| {
        if task.direction_id.trim() == direction_id {
            removed_task_ids.insert(task.id.trim().to_string());
            return false;
        }
        true
    });
    removed_task_ids
}

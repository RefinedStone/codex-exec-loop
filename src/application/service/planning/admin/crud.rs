use std::collections::BTreeSet;

use anyhow::{Result, bail};

use super::documents::{
    DEFAULT_DIRECTION_ID, default_direction_id, direction_from_request, ensure_default_direction,
    ensure_direction_exists, normalized_required_id, remove_task_references, task_from_request,
};
use super::projection::map_management_view;
use super::{
    PlanningAdminCrudOutcome, PlanningAdminDirectionDeleteRequest,
    PlanningAdminDirectionMutationRequest, PlanningAdminFacadeService, PlanningAdminManagementView,
    PlanningAdminTaskDeleteRequest, PlanningAdminTaskMutationRequest,
};

impl PlanningAdminFacadeService {
    pub fn load_management_view(&self) -> Result<PlanningAdminManagementView> {
        let documents = self.load_admin_documents()?;
        Ok(map_management_view(
            &documents.directions,
            &documents.task_ledger,
            default_direction_id(&documents.directions)?,
        ))
    }

    pub fn upsert_direction(
        &self,
        request: PlanningAdminDirectionMutationRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        let mut documents = self.load_admin_documents()?;
        let direction = direction_from_request(request, &documents.directions)?;
        let id = direction.id.clone();
        let updated = if let Some(existing) = documents
            .directions
            .directions
            .iter_mut()
            .find(|existing| existing.id.trim() == id)
        {
            *existing = direction;
            true
        } else {
            documents.directions.directions.push(direction);
            false
        };
        self.commit_admin_documents(documents)?;
        let management = self.load_management_view()?;
        Ok(PlanningAdminCrudOutcome {
            notice: if updated {
                format!("direction `{id}` updated")
            } else {
                format!("direction `{id}` added")
            },
            management,
        })
    }

    pub fn delete_direction(
        &self,
        request: PlanningAdminDirectionDeleteRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        let direction_id = normalized_required_id(&request.id, "direction id")?;
        let mut documents = self.load_admin_documents()?;
        if direction_id == DEFAULT_DIRECTION_ID {
            ensure_default_direction(&mut documents.directions)?;
            self.commit_admin_documents(documents)?;
            let management = self.load_management_view()?;
            return Ok(PlanningAdminCrudOutcome {
                notice: format!("default direction `{DEFAULT_DIRECTION_ID}` is retained"),
                management,
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

        let removed_task_ids = documents
            .task_ledger
            .tasks
            .iter()
            .filter(|task| task.direction_id.trim() == direction_id)
            .map(|task| task.id.trim().to_string())
            .collect::<BTreeSet<_>>();
        documents
            .task_ledger
            .tasks
            .retain(|task| task.direction_id.trim() != direction_id);
        remove_task_references(&mut documents.task_ledger, &removed_task_ids);

        let removed_task_count = removed_task_ids.len();
        ensure_default_direction(&mut documents.directions)?;
        self.commit_admin_documents(documents)?;
        let management = self.load_management_view()?;
        Ok(PlanningAdminCrudOutcome {
            notice: format!(
                "direction `{direction_id}` deleted with {removed_task_count} child tasks"
            ),
            management,
        })
    }

    pub fn upsert_task(
        &self,
        request: PlanningAdminTaskMutationRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        let mut documents = self.load_admin_documents()?;
        ensure_default_direction(&mut documents.directions)?;
        let default_direction_id = default_direction_id(&documents.directions)?;
        let task = task_from_request(request, &documents.task_ledger, default_direction_id)?;
        ensure_direction_exists(&documents.directions, &task.direction_id)?;
        let task_id = task.id.clone();
        let updated = if let Some(existing) = documents
            .task_ledger
            .tasks
            .iter_mut()
            .find(|existing| existing.id.trim() == task_id)
        {
            *existing = task;
            true
        } else {
            documents.task_ledger.tasks.push(task);
            false
        };
        self.commit_admin_documents(documents)?;
        let management = self.load_management_view()?;
        Ok(PlanningAdminCrudOutcome {
            notice: if updated {
                format!("task `{task_id}` updated")
            } else {
                format!("task `{task_id}` added")
            },
            management,
        })
    }

    pub fn delete_task(
        &self,
        request: PlanningAdminTaskDeleteRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        let task_id = normalized_required_id(&request.id, "task id")?;
        let mut documents = self.load_admin_documents()?;
        let original_count = documents.task_ledger.tasks.len();
        documents
            .task_ledger
            .tasks
            .retain(|task| task.id.trim() != task_id);
        if documents.task_ledger.tasks.len() == original_count {
            bail!("task `{task_id}` was not found");
        }
        remove_task_references(
            &mut documents.task_ledger,
            &BTreeSet::from([task_id.to_string()]),
        );
        self.commit_admin_documents(documents)?;
        let management = self.load_management_view()?;
        Ok(PlanningAdminCrudOutcome {
            notice: format!("task `{task_id}` deleted"),
            management,
        })
    }
}

use std::collections::BTreeSet;

use anyhow::{Result, bail};

use super::documents::{
    DEFAULT_DIRECTION_ID, default_direction_id, direction_from_request, ensure_default_direction,
    normalized_required_id, remove_task_references,
};
use super::projection::map_management_view;
use super::{
    PlanningAdminCrudOutcome, PlanningAdminDirectionDeleteRequest,
    PlanningAdminDirectionMutationRequest, PlanningAdminFacadeService, PlanningAdminManagementView,
    PlanningAdminTaskDeleteRequest, PlanningAdminTaskMutationRequest,
};
use crate::application::service::planning::task_mutation::{
    PlanningTaskCreateInput, PlanningTaskMutationCommand, PlanningTaskMutationRequest,
    PlanningTaskMutationSource, PlanningTaskUpdateInput,
};
use crate::domain::planning::TaskStatus;

impl PlanningAdminFacadeService {
    pub fn load_management_view(&self) -> Result<PlanningAdminManagementView> {
        let documents = self.load_admin_documents()?;
        Ok(map_management_view(
            &documents.directions,
            &documents.task_authority,
            default_direction_id(&documents.directions)?,
        ))
    }

    pub fn upsert_direction(
        &self,
        request: PlanningAdminDirectionMutationRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        let mut documents = self.load_admin_documents()?;
        let direction = direction_from_request(request, &documents.directions)?;
        let id = direction.id.trim().to_string();
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
            .task_authority
            .tasks
            .iter()
            .filter(|task| task.direction_id.trim() == direction_id)
            .map(|task| task.id.trim().to_string())
            .collect::<BTreeSet<_>>();
        documents
            .task_authority
            .tasks
            .retain(|task| task.direction_id.trim() != direction_id);
        remove_task_references(&mut documents.task_authority, &removed_task_ids);

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
        self.ensure_default_authority()?;
        let updated = !request.id.trim().is_empty();
        let command = task_command_from_request(request)?;
        let commit = self
            .task_mutation_service
            .apply_commands(PlanningTaskMutationRequest {
                workspace_directory: self.workspace_dir.clone(),
                source: PlanningTaskMutationSource::User,
                source_turn_id: None,
                commands: vec![command],
            })?;
        let task_id = commit
            .committed_task_ids
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
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
        let original_count = documents.task_authority.tasks.len();
        documents
            .task_authority
            .tasks
            .retain(|task| task.id.trim() != task_id);
        if documents.task_authority.tasks.len() == original_count {
            bail!("task `{task_id}` was not found");
        }
        remove_task_references(
            &mut documents.task_authority,
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

fn task_command_from_request(
    request: PlanningAdminTaskMutationRequest,
) -> Result<PlanningTaskMutationCommand> {
    let task_id = request.id.trim().to_string();
    if task_id.is_empty() {
        return Ok(PlanningTaskMutationCommand::CreateTask(
            PlanningTaskCreateInput {
                direction_id: optional_id(request.direction_id, "direction id")?,
                direction_relation_note: None,
                title: required_text(&request.title, "task title")?.to_string(),
                description: optional_text(request.description),
                status: optional_task_status(&request.status)?,
                base_priority: optional_i32(&request.base_priority, "base priority")?,
                dynamic_priority_delta: optional_i32(
                    &request.dynamic_priority_delta,
                    "dynamic priority delta",
                )?,
                priority_reason: optional_text(request.priority_reason),
                depends_on: split_references(&request.depends_on_text),
                blocked_by: split_references(&request.blocked_by_text),
            },
        ));
    }

    Ok(PlanningTaskMutationCommand::UpdateTask(
        PlanningTaskUpdateInput {
            task_id: normalized_required_id(&task_id, "task id")?.to_string(),
            direction_id: optional_id(request.direction_id, "direction id")?,
            direction_relation_note: None,
            title: Some(required_text(&request.title, "task title")?.to_string()),
            description: optional_text(request.description),
            status: optional_task_status(&request.status)?,
            base_priority: optional_i32(&request.base_priority, "base priority")?,
            dynamic_priority_delta: optional_i32(
                &request.dynamic_priority_delta,
                "dynamic priority delta",
            )?,
            priority_reason: Some(request.priority_reason.trim().to_string()),
            depends_on: Some(split_references(&request.depends_on_text)),
            blocked_by: Some(split_references(&request.blocked_by_text)),
        },
    ))
}

fn optional_id(value: String, label: &str) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized_required_id(value, label)?.to_string()))
    }
}

fn required_text<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    Ok(value)
}

fn optional_text(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

fn optional_task_status(raw: &str) -> Result<Option<TaskStatus>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    Ok(Some(match raw.to_ascii_lowercase().as_str() {
        "ready" => TaskStatus::Ready,
        "blocked" => TaskStatus::Blocked,
        "in_progress" => TaskStatus::InProgress,
        "done" => TaskStatus::Done,
        "cancelled" => TaskStatus::Cancelled,
        "awaiting_user" => TaskStatus::AwaitingUser,
        "proposed" => TaskStatus::Proposed,
        other => bail!("unknown task status `{other}`"),
    }))
}

fn optional_i32(raw: &str, label: &str) -> Result<Option<i32>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    raw.parse::<i32>()
        .map(Some)
        .map_err(|error| anyhow::anyhow!("{label} must be an integer: {error}"))
}

fn split_references(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{PlanningAdminTaskMutationRequest, task_command_from_request};
    use crate::application::service::planning::task_mutation::PlanningTaskMutationCommand;
    use crate::domain::planning::TaskStatus;

    #[test]
    fn admin_task_create_maps_blank_fields_to_common_defaults() {
        let command = task_command_from_request(PlanningAdminTaskMutationRequest {
            id: String::new(),
            direction_id: String::new(),
            title: "Ship admin task bridge".to_string(),
            description: String::new(),
            status: String::new(),
            base_priority: String::new(),
            dynamic_priority_delta: String::new(),
            priority_reason: String::new(),
            depends_on_text: "task-a, task-b\n task-c".to_string(),
            blocked_by_text: String::new(),
        })
        .expect("admin request should map to common create command");

        let PlanningTaskMutationCommand::CreateTask(input) = command else {
            panic!("expected create command");
        };
        assert_eq!(input.direction_id, None);
        assert_eq!(input.description, None);
        assert_eq!(input.status, None);
        assert_eq!(input.base_priority, None);
        assert_eq!(input.dynamic_priority_delta, None);
        assert_eq!(input.depends_on, vec!["task-a", "task-b", "task-c"]);
    }

    #[test]
    fn admin_task_update_maps_to_common_update_command() {
        let command = task_command_from_request(PlanningAdminTaskMutationRequest {
            id: "task-1".to_string(),
            direction_id: "general-workstream".to_string(),
            title: "Updated task".to_string(),
            description: "Updated description".to_string(),
            status: "blocked".to_string(),
            base_priority: "90".to_string(),
            dynamic_priority_delta: "-5".to_string(),
            priority_reason: "waiting for review".to_string(),
            depends_on_text: "task-a".to_string(),
            blocked_by_text: "task-b".to_string(),
        })
        .expect("admin request should map to common update command");

        let PlanningTaskMutationCommand::UpdateTask(input) = command else {
            panic!("expected update command");
        };
        assert_eq!(input.task_id, "task-1");
        assert_eq!(input.direction_id.as_deref(), Some("general-workstream"));
        assert_eq!(input.title.as_deref(), Some("Updated task"));
        assert_eq!(input.description.as_deref(), Some("Updated description"));
        assert_eq!(input.status, Some(TaskStatus::Blocked));
        assert_eq!(input.base_priority, Some(90));
        assert_eq!(input.dynamic_priority_delta, Some(-5));
        assert_eq!(input.priority_reason.as_deref(), Some("waiting for review"));
        assert_eq!(input.depends_on, Some(vec!["task-a".to_string()]));
        assert_eq!(input.blocked_by, Some(vec!["task-b".to_string()]));
    }
}

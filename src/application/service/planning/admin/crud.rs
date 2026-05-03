use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};

use super::direction_mutation::{
    PlanningAdminDirectionMutationCommand, PlanningAdminDirectionMutationService,
};
use super::documents::{default_direction_id, normalized_required_id, remove_task_references};
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

/*
 * Admin CRUD is the operator-facing mutation bridge. Direction changes delegate
 * to the direction-specific document service, while task upserts are translated
 * into the shared PlanningTaskMutationService so admin edits obey the same
 * defaults, queue rebuilds, and validation path as runtime/model task commands.
 */
impl PlanningAdminFacadeService {
    pub fn load_management_view(&self) -> Result<PlanningAdminManagementView> {
        // Reload after every mutation from accepted authority so the response
        // cannot drift from repository state.
        let documents = self.load_operator_planning_documents()?;
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
        // Direction mutations edit the direction catalog and perform any
        // dependent task cleanup inside the direction mutation service.
        let outcome = PlanningAdminDirectionMutationService::new(self)
            .apply(PlanningAdminDirectionMutationCommand::Upsert(request))?;
        let management = self.load_management_view()?;
        Ok(PlanningAdminCrudOutcome {
            notice: if outcome.updated {
                format!("direction `{}` updated", outcome.direction_id)
            } else {
                format!("direction `{}` added", outcome.direction_id)
            },
            management,
        })
    }
    pub fn delete_direction(
        &self,
        request: PlanningAdminDirectionDeleteRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        // Deleting the default direction is intentionally reported as retained:
        // it is the fallback target for blank task creation and bootstrap repair.
        let outcome = PlanningAdminDirectionMutationService::new(self)
            .apply(PlanningAdminDirectionMutationCommand::Delete(request))?;
        let management = self.load_management_view()?;
        if !outcome.deleted {
            return Ok(PlanningAdminCrudOutcome {
                notice: format!("default direction `{}` is retained", outcome.direction_id),
                management,
            });
        }
        Ok(PlanningAdminCrudOutcome {
            notice: format!(
                "direction `{}` deleted with {} child tasks",
                outcome.direction_id, outcome.removed_task_count
            ),
            management,
        })
    }
    pub fn upsert_task(
        &self,
        request: PlanningAdminTaskMutationRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        // Task form submissions go through the common mutation service instead
        // of directly editing documents; this keeps admin, worker, and runtime
        // task semantics aligned.
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
        let task_id = commit.committed_task_ids.first().cloned().ok_or_else(|| {
            anyhow!("planning task mutation completed without returning a task id")
        })?;
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
        // Admin delete is an explicit operator maintenance action. LLM and runtime task
        // commands still cannot delete tasks; they must move work to `cancelled`.
        let task_id = normalized_required_id(&request.id, "task id")?;
        let mut documents = self.load_operator_planning_documents()?;
        let original_count = documents.task_authority.tasks.len();
        // Direct deletion is limited to this operator path, and every dangling
        // dependency/blocker reference is pruned before the document commit.
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
        self.commit_operator_planning_documents(documents)?;
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
    // Blank id means create. Nonblank id means update. The admin form uses text
    // fields, so this mapper is where strings become typed task mutation inputs.
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

    // Updates intentionally use Some(empty_string) for priority_reason and
    // empty vectors for references so an operator can clear those fields.
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
    // Optional ids are allowed to be blank, but any supplied id must still be
    // safe for graph references and route parameters.
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
    // Create inputs use None to invoke common mutation defaults.
    let value = value.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

fn optional_task_status(raw: &str) -> Result<Option<TaskStatus>> {
    // Status strings match the labels rendered by admin management views.
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
    // Reference fields support comma lists and textarea lines for fast editing.
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
        // Blank admin fields must stay absent so common create defaults, such as
        // direction fallback and priority defaults, remain centralized.
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
        // Updates carry explicit values so an admin edit can replace task state
        // without using the model-oriented command extraction path.
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

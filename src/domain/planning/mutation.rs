use std::fmt::{Display, Formatter};

use super::{TaskActor, TaskDefinition, TaskStatus};

#[derive(Default, Clone)]
// task mutation policy는 repository나 runtime을 보지 않는 순수 domain decision이다.
pub struct PlanningTaskMutationPolicy;

impl PlanningTaskMutationPolicy {
    pub fn new() -> Self {
        Self
    }

    pub fn decide_task_update(
        &self,
        current_task: &TaskDefinition,
        actor: TaskActor,
        requested_status: Option<TaskStatus>,
        description_supplied: bool,
    ) -> Result<TaskUpdateDecision, PlanningTaskMutationPolicyViolation> {
        if let Some(requested_status) = requested_status
            && current_task.status.is_terminal()
            && current_task.status != requested_status
        {
            return Err(PlanningTaskMutationPolicyViolation::TerminalStatusChange {
                task_id: current_task.id.trim().to_string(),
                current_status: current_task.status,
                requested_status,
            });
        }

        Ok(TaskUpdateDecision {
            description: decide_description_update(current_task, actor, description_supplied),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskUpdateDecision {
    pub description: TaskDescriptionUpdateDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskDescriptionUpdateDecision {
    AcceptSupplied,
    PreserveExisting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningTaskMutationPolicyViolation {
    TerminalStatusChange {
        task_id: String,
        current_status: TaskStatus,
        requested_status: TaskStatus,
    },
}

impl Display for PlanningTaskMutationPolicyViolation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TerminalStatusChange {
                task_id,
                current_status,
                requested_status,
            } => write!(
                formatter,
                "task `{task_id}` cannot change from terminal status `{}` to `{}`",
                current_status.label(),
                requested_status.label()
            ),
        }
    }
}

impl std::error::Error for PlanningTaskMutationPolicyViolation {}

fn decide_description_update(
    current_task: &TaskDefinition,
    actor: TaskActor,
    description_supplied: bool,
) -> TaskDescriptionUpdateDecision {
    if description_supplied
        && (actor == TaskActor::User || current_task.description.trim().is_empty())
    {
        return TaskDescriptionUpdateDecision::AcceptSupplied;
    }
    TaskDescriptionUpdateDecision::PreserveExisting
}

#[cfg(test)]
mod tests {
    use super::{
        PlanningTaskMutationPolicy, TaskDescriptionUpdateDecision,
        TaskDescriptionUpdateDecision::{AcceptSupplied, PreserveExisting},
    };
    use crate::domain::planning::{TaskActor, TaskDefinition, TaskMutationProvenance, TaskStatus};

    fn task(status: TaskStatus) -> TaskDefinition {
        TaskDefinition {
            id: "task-1".to_string(),
            direction_id: "direction-a".to_string(),
            direction_relation_note: "supports direction".to_string(),
            title: "Existing task".to_string(),
            description: "Existing description".to_string(),
            status,
            base_priority: 10,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            provenance: TaskMutationProvenance::default(),
            updated_at: "2026-04-29T00:00:00Z".to_string(),
        }
    }

    fn description_decision(
        actor: TaskActor,
        current_description: &str,
        description_supplied: bool,
    ) -> TaskDescriptionUpdateDecision {
        let mut task = task(TaskStatus::Ready);
        task.description = current_description.to_string();
        PlanningTaskMutationPolicy::new()
            .decide_task_update(&task, actor, None, description_supplied)
            .unwrap()
            .description
    }

    #[test]
    fn rejects_terminal_status_reclassification() {
        let task = task(TaskStatus::Done);
        let error = PlanningTaskMutationPolicy::new()
            .decide_task_update(&task, TaskActor::Worker, Some(TaskStatus::Cancelled), false)
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("cannot change from terminal status `done` to `cancelled`")
        );
    }

    #[test]
    fn permits_idempotent_terminal_status() {
        let task = task(TaskStatus::Done);
        let decision = PlanningTaskMutationPolicy::new()
            .decide_task_update(&task, TaskActor::Worker, Some(TaskStatus::Done), true)
            .unwrap();

        assert_eq!(decision.description, PreserveExisting);
    }

    #[test]
    fn keeps_description_ownership_in_domain_policy() {
        assert_eq!(
            description_decision(TaskActor::User, "Existing description", true),
            AcceptSupplied
        );
        assert_eq!(
            description_decision(TaskActor::Worker, "Existing description", true),
            PreserveExisting
        );
        assert_eq!(
            description_decision(TaskActor::Worker, "", true),
            AcceptSupplied
        );
        assert_eq!(
            description_decision(TaskActor::System, "Existing description", false),
            PreserveExisting
        );
    }
}

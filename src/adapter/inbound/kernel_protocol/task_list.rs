//! 운영자 편집에 필요한 원본 필드와 같은 snapshot의 revision이다.
use super::{PlanningTaskStatus, TaskFields};
use crate::application::port::inbound::planning_task_tool_port::PlanningTaskToolResponse;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanningTaskDetails {
    pub task_id: String,
    pub fields: TaskFields,
    pub acceptance_revision: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanningTaskList {
    pub available: bool,
    pub planning_revision: Option<i64>,
    pub tasks: Vec<PlanningTaskDetails>,
}

impl PlanningTaskList {
    pub fn unavailable() -> Self {
        Self {
            available: false,
            planning_revision: None,
            tasks: Vec::new(),
        }
    }

    pub fn from_snapshot(snapshot: PlanningTaskToolResponse) -> Self {
        Self {
            available: snapshot.ok && snapshot.committed_planning_revision.is_some(),
            planning_revision: snapshot.committed_planning_revision,
            tasks: snapshot
                .tasks
                .into_iter()
                .map(|task| PlanningTaskDetails {
                    task_id: task.id,
                    fields: TaskFields {
                        direction_id: Some(task.direction_id),
                        title: task.title,
                        description: Some(task.description),
                        acceptance: Some(task.acceptance.into_iter().map(Into::into).collect()),
                        status: Some(PlanningTaskStatus::from_core(task.status)),
                        base_priority: Some(task.base_priority),
                        dynamic_priority_delta: Some(task.dynamic_priority_delta),
                        priority_reason: Some(task.priority_reason),
                        depends_on: task.depends_on,
                        blocked_by: task.blocked_by,
                    },
                    acceptance_revision: task.acceptance_revision,
                    updated_at: task.updated_at,
                })
                .collect(),
        }
    }
}

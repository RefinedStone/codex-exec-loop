use serde::Deserialize;

use crate::domain::planning::TaskStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskCreateInput {
    pub direction_id: Option<String>,
    pub direction_relation_note: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub base_priority: Option<i32>,
    pub dynamic_priority_delta: Option<i32>,
    pub priority_reason: Option<String>,
    pub depends_on: Vec<String>,
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskUpdateInput {
    pub task_id: String,
    pub direction_id: Option<String>,
    pub direction_relation_note: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub base_priority: Option<i32>,
    pub dynamic_priority_delta: Option<i32>,
    pub priority_reason: Option<String>,
    pub depends_on: Option<Vec<String>>,
    pub blocked_by: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningTaskMutationCommand {
    CreateTask(PlanningTaskCreateInput),
    UpdateTask(PlanningTaskUpdateInput),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningTaskCommandExtraction {
    Commands(Vec<PlanningTaskMutationCommand>),
    LegacyTaskAuthorityRejected(String),
    InvalidCommands {
        error: String,
        rejected_json: Option<String>,
    },
    None,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningTaskCommandsDocument {
    planning_task_commands: PlanningTaskCommandsEnvelope,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningTaskCommandsEnvelope {
    version: u32,
    #[serde(default)]
    commands: Vec<PlanningTaskCommandEnvelope>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum PlanningTaskCommandEnvelope {
    CreateTask(PlanningTaskCreateCommand),
    UpdateTask(PlanningTaskUpdateCommand),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningTaskCreateCommand {
    direction_id: Option<String>,
    direction_relation_note: Option<String>,
    title: String,
    description: Option<String>,
    status: Option<TaskStatus>,
    base_priority: Option<i32>,
    dynamic_priority_delta: Option<i32>,
    priority_reason: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    blocked_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningTaskUpdateCommand {
    task_id: String,
    direction_id: Option<String>,
    direction_relation_note: Option<String>,
    title: Option<String>,
    description: Option<String>,
    status: Option<TaskStatus>,
    base_priority: Option<i32>,
    dynamic_priority_delta: Option<i32>,
    priority_reason: Option<String>,
    depends_on: Option<Vec<String>>,
    blocked_by: Option<Vec<String>>,
}

pub fn extract_planning_task_commands(message: &str) -> PlanningTaskCommandExtraction {
    let mut first_invalid = None;
    for candidate in candidate_json_sections(message) {
        if candidate.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) else {
            continue;
        };
        if value.get("task_authority").is_some()
            || (value.get("version").is_some() && value.get("tasks").is_some())
        {
            let rejected =
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| candidate.to_string());
            return PlanningTaskCommandExtraction::LegacyTaskAuthorityRejected(rejected);
        }
        if value.get("planning_task_commands").is_some() {
            let rejected_json =
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| candidate.to_string());
            match serde_json::from_value::<PlanningTaskCommandsDocument>(value) {
                Ok(document) => {
                    if document.planning_task_commands.version != 1 {
                        return PlanningTaskCommandExtraction::InvalidCommands {
                            error: format!(
                                "planning_task_commands version {} is not supported",
                                document.planning_task_commands.version
                            ),
                            rejected_json: Some(rejected_json),
                        };
                    }
                    return PlanningTaskCommandExtraction::Commands(
                        document
                            .planning_task_commands
                            .commands
                            .into_iter()
                            .map(PlanningTaskMutationCommand::from)
                            .collect(),
                    );
                }
                Err(error) => first_invalid = Some((error.to_string(), rejected_json)),
            }
        }
    }

    first_invalid
        .map(
            |(error, rejected_json)| PlanningTaskCommandExtraction::InvalidCommands {
                error,
                rejected_json: Some(rejected_json),
            },
        )
        .unwrap_or(PlanningTaskCommandExtraction::None)
}

impl From<PlanningTaskCommandEnvelope> for PlanningTaskMutationCommand {
    fn from(command: PlanningTaskCommandEnvelope) -> Self {
        match command {
            PlanningTaskCommandEnvelope::CreateTask(command) => {
                Self::CreateTask(PlanningTaskCreateInput {
                    direction_id: command.direction_id,
                    direction_relation_note: command.direction_relation_note,
                    title: command.title,
                    description: command.description,
                    status: command.status,
                    base_priority: command.base_priority,
                    dynamic_priority_delta: command.dynamic_priority_delta,
                    priority_reason: command.priority_reason,
                    depends_on: command.depends_on,
                    blocked_by: command.blocked_by,
                })
            }
            PlanningTaskCommandEnvelope::UpdateTask(command) => {
                Self::UpdateTask(PlanningTaskUpdateInput {
                    task_id: command.task_id,
                    direction_id: command.direction_id,
                    direction_relation_note: command.direction_relation_note,
                    title: command.title,
                    description: command.description,
                    status: command.status,
                    base_priority: command.base_priority,
                    dynamic_priority_delta: command.dynamic_priority_delta,
                    priority_reason: command.priority_reason,
                    depends_on: command.depends_on,
                    blocked_by: command.blocked_by,
                })
            }
        }
    }
}

fn candidate_json_sections(message: &str) -> Vec<&str> {
    let mut sections = Vec::new();
    let mut remainder = message;
    while let Some(start) = remainder.find("```") {
        remainder = &remainder[start + 3..];
        let body_start = remainder.find('\n').map(|index| index + 1).unwrap_or(0);
        let after_header = &remainder[body_start..];
        let Some(end) = after_header.find("```") else {
            break;
        };
        sections.push(after_header[..end].trim());
        remainder = &after_header[end + 3..];
    }
    sections.extend(balanced_json_object_sections(message));
    sections.push(message.trim());
    sections
}

fn balanced_json_object_sections(message: &str) -> Vec<&str> {
    let mut sections = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, character) in message.char_indices() {
        let Some(start_index) = start else {
            if character == '{' {
                start = Some(index);
                depth = 1;
                in_string = false;
                escaped = false;
            }
            continue;
        };

        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }

        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    sections.push(message[start_index..index + character.len_utf8()].trim());
                    start = None;
                }
            }
            _ => {}
        }
    }

    sections
}

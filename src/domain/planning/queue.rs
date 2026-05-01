use std::{collections::HashMap, fmt};

use chrono::DateTime;

use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, PriorityQueueProjection,
    PriorityQueueSkippedTask, PriorityQueueTask, TaskAuthorityDocument, TaskDefinition,
};

#[derive(Default, Clone)]
pub struct PriorityQueueService;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorityQueueBuildError {
    MultipleInProgressTasks {
        task_ids: Vec<String>,
    },
    UnknownDirection {
        task_id: String,
        direction_id: String,
    },
    MissingDependency {
        task_id: String,
        dependency_id: String,
    },
    MissingBlocker {
        task_id: String,
        blocker_id: String,
    },
    InvalidUpdatedAt {
        task_id: String,
        updated_at: String,
    },
}

impl fmt::Display for PriorityQueueBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MultipleInProgressTasks { task_ids } => write!(
                formatter,
                "task authority may contain at most one in_progress task; found {}: {}",
                task_ids.len(),
                task_ids.join(", ")
            ),
            Self::UnknownDirection {
                task_id,
                direction_id,
            } => write!(
                formatter,
                "task {task_id} references unknown direction_id {}",
                display_reference(direction_id)
            ),
            Self::MissingDependency {
                task_id,
                dependency_id,
            } => write!(
                formatter,
                "task {task_id} references unknown dependency {}",
                display_reference(dependency_id)
            ),
            Self::MissingBlocker {
                task_id,
                blocker_id,
            } => write!(
                formatter,
                "task {task_id} references unknown blocker {}",
                display_reference(blocker_id)
            ),
            Self::InvalidUpdatedAt {
                task_id,
                updated_at,
            } => write!(
                formatter,
                "task {task_id} must use RFC3339 updated_at for queue ordering, got {}",
                display_reference(updated_at)
            ),
        }
    }
}

impl std::error::Error for PriorityQueueBuildError {}

#[derive(Debug, Clone)]
struct QueueCandidate {
    readiness_rank: u8,
    combined_priority: i32,
    updated_at_epoch_millis: i64,
    task: PriorityQueueTask,
}

impl PriorityQueueService {
    pub fn new() -> Self {
        Self
    }

    pub fn build_projection(
        &self,
        directions: &DirectionCatalogDocument,
        task_authority: &TaskAuthorityDocument,
    ) -> Result<PriorityQueueProjection, PriorityQueueBuildError> {
        let direction_map = directions
            .directions
            .iter()
            .map(|direction| (direction.id.trim(), direction))
            .collect::<HashMap<_, _>>();
        let task_map = task_authority
            .tasks
            .iter()
            .map(|task| (task.id.trim(), task))
            .collect::<HashMap<_, _>>();
        let updated_at_epoch_millis_by_task_id =
            self.validate_queue_inputs(task_authority, &direction_map, &task_map)?;

        let mut candidates = Vec::new();
        let mut proposed_candidates = Vec::new();
        let mut skipped_tasks = Vec::new();

        for task in &task_authority.tasks {
            let normalized_direction_id = task.direction_id.trim();
            let direction = direction_map
                .get(normalized_direction_id)
                .expect("queue build preflight should validate direction references");

            if !direction.state.allows_queue_execution() {
                skipped_tasks.push(self.skipped_task(
                    task,
                    format!("direction {} is {}", direction.id, direction.state_label()),
                ));
                continue;
            }

            if task.status == crate::domain::planning::TaskStatus::Proposed {
                if let Some(reason) = self.unresolved_dependency_reason(task, &task_map) {
                    skipped_tasks.push(self.skipped_task(task, reason));
                    continue;
                }

                if let Some(reason) = self.unresolved_blocker_reason(task, &task_map) {
                    skipped_tasks.push(self.skipped_task(task, reason));
                    continue;
                }

                proposed_candidates.push(QueueCandidate {
                    readiness_rank: 0,
                    combined_priority: task.combined_priority(),
                    updated_at_epoch_millis: *updated_at_epoch_millis_by_task_id
                        .get(task.id.trim())
                        .expect("queue build preflight should validate updated_at"),
                    task: PriorityQueueTask {
                        rank: 0,
                        task_id: task.id.clone(),
                        direction_id: task.direction_id.clone(),
                        direction_title: direction.title.clone(),
                        task_title: task.title.clone(),
                        status: task.status,
                        combined_priority: task.combined_priority(),
                        updated_at: task.updated_at.clone(),
                        rank_reasons: build_rank_reasons(task),
                    },
                });
                continue;
            }

            let Some(readiness_rank) = task.status.queue_readiness_rank() else {
                skipped_tasks.push(self.skipped_task(
                    task,
                    format!("status {} is not executable", task.status.label()),
                ));
                continue;
            };

            if let Some(reason) = self.unresolved_dependency_reason(task, &task_map) {
                skipped_tasks.push(self.skipped_task(task, reason));
                continue;
            }

            if let Some(reason) = self.unresolved_blocker_reason(task, &task_map) {
                skipped_tasks.push(self.skipped_task(task, reason));
                continue;
            }

            candidates.push(QueueCandidate {
                readiness_rank,
                combined_priority: task.combined_priority(),
                updated_at_epoch_millis: *updated_at_epoch_millis_by_task_id
                    .get(task.id.trim())
                    .expect("queue build preflight should validate updated_at"),
                task: PriorityQueueTask {
                    rank: 0,
                    task_id: task.id.clone(),
                    direction_id: task.direction_id.clone(),
                    direction_title: direction.title.clone(),
                    task_title: task.title.clone(),
                    status: task.status,
                    combined_priority: task.combined_priority(),
                    updated_at: task.updated_at.clone(),
                    rank_reasons: build_rank_reasons(task),
                },
            });
        }

        candidates.sort_by(|left, right| {
            left.readiness_rank
                .cmp(&right.readiness_rank)
                .then_with(|| right.combined_priority.cmp(&left.combined_priority))
                .then_with(|| {
                    left.updated_at_epoch_millis
                        .cmp(&right.updated_at_epoch_millis)
                })
                .then_with(|| left.task.task_id.cmp(&right.task.task_id))
        });

        let active_tasks = candidates
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| PriorityQueueTask {
                rank: index + 1,
                ..candidate.task
            })
            .collect::<Vec<_>>();
        let next_task = active_tasks.first().cloned();

        proposed_candidates.sort_by(|left, right| {
            right
                .combined_priority
                .cmp(&left.combined_priority)
                .then_with(|| {
                    left.updated_at_epoch_millis
                        .cmp(&right.updated_at_epoch_millis)
                })
                .then_with(|| left.task.task_id.cmp(&right.task.task_id))
        });

        let proposed_tasks = proposed_candidates
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| PriorityQueueTask {
                rank: index + 1,
                ..candidate.task
            })
            .collect::<Vec<_>>();

        Ok(PriorityQueueProjection {
            next_task,
            active_tasks,
            proposed_tasks,
            skipped_tasks,
        })
    }

    fn validate_queue_inputs<'a>(
        &self,
        task_authority: &'a TaskAuthorityDocument,
        direction_map: &HashMap<&'a str, &'a DirectionDefinition>,
        task_map: &HashMap<&'a str, &'a TaskDefinition>,
    ) -> Result<HashMap<&'a str, i64>, PriorityQueueBuildError> {
        let in_progress_tasks = task_authority
            .tasks
            .iter()
            .filter(|task| task.status == crate::domain::planning::TaskStatus::InProgress)
            .collect::<Vec<_>>();
        if in_progress_tasks.len() > 1 {
            return Err(PriorityQueueBuildError::MultipleInProgressTasks {
                task_ids: in_progress_tasks
                    .into_iter()
                    .map(|task| task.id.trim().to_string())
                    .collect(),
            });
        }

        let mut updated_at_epoch_millis_by_task_id = HashMap::new();
        for task in &task_authority.tasks {
            let task_id = task.id.trim();
            if !direction_map.contains_key(task.direction_id.trim()) {
                return Err(PriorityQueueBuildError::UnknownDirection {
                    task_id: task_id.to_string(),
                    direction_id: task.direction_id.trim().to_string(),
                });
            }

            let updated_at_epoch_millis = parse_updated_at_epoch_millis(task.updated_at.as_str())
                .map_err(|_| {
                PriorityQueueBuildError::InvalidUpdatedAt {
                    task_id: task_id.to_string(),
                    updated_at: task.updated_at.clone(),
                }
            })?;
            updated_at_epoch_millis_by_task_id.insert(task_id, updated_at_epoch_millis);

            for dependency_id in &task.depends_on {
                let normalized_dependency_id = dependency_id.trim();
                if !task_map.contains_key(normalized_dependency_id) {
                    return Err(PriorityQueueBuildError::MissingDependency {
                        task_id: task_id.to_string(),
                        dependency_id: normalized_dependency_id.to_string(),
                    });
                }
            }

            for blocker_id in &task.blocked_by {
                let normalized_blocker_id = blocker_id.trim();
                if !task_map.contains_key(normalized_blocker_id) {
                    return Err(PriorityQueueBuildError::MissingBlocker {
                        task_id: task_id.to_string(),
                        blocker_id: normalized_blocker_id.to_string(),
                    });
                }
            }
        }

        Ok(updated_at_epoch_millis_by_task_id)
    }

    fn skipped_task(&self, task: &TaskDefinition, reason: String) -> PriorityQueueSkippedTask {
        PriorityQueueSkippedTask {
            task_id: task.id.clone(),
            task_title: task.title.clone(),
            direction_id: task.direction_id.clone(),
            status: task.status,
            reason,
        }
    }

    fn unresolved_dependency_reason(
        &self,
        task: &TaskDefinition,
        task_map: &HashMap<&str, &TaskDefinition>,
    ) -> Option<String> {
        let unresolved_dependencies = task
            .depends_on
            .iter()
            .filter_map(|dependency_id| {
                let normalized_dependency_id = dependency_id.trim();
                match task_map.get(normalized_dependency_id) {
                    Some(dependency) if dependency.status.is_dependency_complete() => None,
                    Some(dependency) => Some(format!(
                        "{}({})",
                        normalized_dependency_id,
                        dependency.status.label()
                    )),
                    None => {
                        unreachable!("queue build preflight should validate dependency references")
                    }
                }
            })
            .collect::<Vec<_>>();

        if unresolved_dependencies.is_empty() {
            None
        } else {
            Some(format!(
                "waiting on dependencies: {}",
                unresolved_dependencies.join(", ")
            ))
        }
    }

    fn unresolved_blocker_reason(
        &self,
        task: &TaskDefinition,
        task_map: &HashMap<&str, &TaskDefinition>,
    ) -> Option<String> {
        let unresolved_blockers = task
            .blocked_by
            .iter()
            .filter_map(|blocker_id| {
                let normalized_blocker_id = blocker_id.trim();
                match task_map.get(normalized_blocker_id) {
                    Some(blocker) if blocker.status.clears_blocker() => None,
                    Some(blocker) => Some(format!(
                        "{}({})",
                        normalized_blocker_id,
                        blocker.status.label()
                    )),
                    None => {
                        unreachable!("queue build preflight should validate blocker references")
                    }
                }
            })
            .collect::<Vec<_>>();

        if unresolved_blockers.is_empty() {
            None
        } else {
            Some(format!(
                "blocked by tasks: {}",
                unresolved_blockers.join(", ")
            ))
        }
    }
}

fn parse_updated_at_epoch_millis(updated_at: &str) -> Result<i64, chrono::ParseError> {
    DateTime::parse_from_rfc3339(updated_at).map(|timestamp| timestamp.timestamp_millis())
}

fn display_reference(value: &str) -> &str {
    if value.is_empty() { "<blank>" } else { value }
}

fn build_rank_reasons(task: &TaskDefinition) -> Vec<String> {
    let mut reasons = vec![
        format!("status={}", task.status.label()),
        format!(
            "combined_priority={} (base {} + delta {})",
            task.combined_priority(),
            task.base_priority,
            task.dynamic_priority_delta
        ),
    ];

    if !task.depends_on.is_empty() {
        reasons.push(format!("dependencies_ready={}", task.depends_on.len()));
    }
    if task.dynamic_priority_delta != 0 && !task.priority_reason.trim().is_empty() {
        reasons.push(format!("priority_reason={}", task.priority_reason.trim()));
    }

    reasons
}

trait DirectionQueueLabel {
    fn state_label(&self) -> &'static str;
}

impl DirectionQueueLabel for DirectionDefinition {
    fn state_label(&self) -> &'static str {
        match self.state {
            crate::domain::planning::DirectionState::Active => "active",
            crate::domain::planning::DirectionState::Paused => "paused",
            crate::domain::planning::DirectionState::Done => "done",
        }
    }
}

#[cfg(test)]
mod tests;

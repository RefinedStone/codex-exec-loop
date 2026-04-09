use std::collections::HashMap;

use chrono::DateTime;

use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, PriorityQueueSkippedTask, PriorityQueueSnapshot,
    PriorityQueueTask, TaskDefinition, TaskLedgerDocument,
};

#[derive(Default, Clone)]
pub struct PriorityQueueService;

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

    pub fn build_snapshot(
        &self,
        directions: &DirectionCatalogDocument,
        task_ledger: &TaskLedgerDocument,
    ) -> PriorityQueueSnapshot {
        let direction_map = directions
            .directions
            .iter()
            .map(|direction| (direction.id.as_str(), direction))
            .collect::<HashMap<_, _>>();
        let task_map = task_ledger
            .tasks
            .iter()
            .map(|task| (task.id.as_str(), task))
            .collect::<HashMap<_, _>>();

        let mut candidates = Vec::new();
        let mut skipped_tasks = Vec::new();

        for task in &task_ledger.tasks {
            let Some(direction) = direction_map.get(task.direction_id.as_str()) else {
                skipped_tasks.push(
                    self.skipped_task(task, format!("unknown direction_id {}", task.direction_id)),
                );
                continue;
            };

            if !direction.state.allows_queue_execution() {
                skipped_tasks.push(self.skipped_task(
                    task,
                    format!("direction {} is {}", direction.id, direction.state_label()),
                ));
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
                updated_at_epoch_millis: parse_updated_at_epoch_millis(task.updated_at.as_str()),
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

        PriorityQueueSnapshot {
            next_task,
            active_tasks,
            skipped_tasks,
        }
    }

    fn skipped_task(&self, task: &TaskDefinition, reason: String) -> PriorityQueueSkippedTask {
        PriorityQueueSkippedTask {
            task_id: task.id.clone(),
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
                    None => Some(format!("{normalized_dependency_id}(missing)")),
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
                    None => Some(format!("{normalized_blocker_id}(missing)")),
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

fn parse_updated_at_epoch_millis(updated_at: &str) -> i64 {
    DateTime::parse_from_rfc3339(updated_at)
        .map(|timestamp| timestamp.timestamp_millis())
        .unwrap_or(i64::MAX)
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
mod tests {
    use super::PriorityQueueService;
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, TaskActor, TaskDefinition,
        TaskLedgerDocument, TaskStatus,
    };

    fn directions(states: &[(&str, DirectionState)]) -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: 1,
            directions: states
                .iter()
                .map(|(id, state)| DirectionDefinition {
                    id: (*id).to_string(),
                    title: format!("{id} title"),
                    summary: format!("{id} summary"),
                    success_criteria: vec![format!("{id} done")],
                    scope_hints: vec![format!("{id} hint")],
                    state: *state,
                })
                .collect(),
        }
    }

    fn task(
        id: &str,
        direction_id: &str,
        status: TaskStatus,
        base_priority: i32,
        dynamic_priority_delta: i32,
        updated_at: &str,
    ) -> TaskDefinition {
        TaskDefinition {
            id: id.to_string(),
            direction_id: direction_id.to_string(),
            direction_relation_note: "fits direction".to_string(),
            title: format!("{id} title"),
            description: format!("{id} description"),
            status,
            base_priority,
            dynamic_priority_delta,
            priority_reason: if dynamic_priority_delta == 0 {
                String::new()
            } else {
                "recent result raised urgency".to_string()
            },
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            updated_at: updated_at.to_string(),
        }
    }

    #[test]
    fn prefers_in_progress_tasks_before_ready_tasks() {
        let queue_service = PriorityQueueService::new();
        let directions = directions(&[("direction-a", DirectionState::Active)]);
        let task_ledger = TaskLedgerDocument {
            version: 1,
            tasks: vec![
                task(
                    "task-ready-high",
                    "direction-a",
                    TaskStatus::Ready,
                    80,
                    0,
                    "2026-04-09T09:00:00Z",
                ),
                task(
                    "task-in-progress",
                    "direction-a",
                    TaskStatus::InProgress,
                    10,
                    0,
                    "2026-04-09T10:00:00Z",
                ),
                task(
                    "task-ready-low",
                    "direction-a",
                    TaskStatus::Ready,
                    20,
                    0,
                    "2026-04-09T08:00:00Z",
                ),
            ],
        };

        let snapshot = queue_service.build_snapshot(&directions, &task_ledger);

        assert_eq!(
            snapshot
                .next_task
                .as_ref()
                .map(|task| task.task_id.as_str()),
            Some("task-in-progress")
        );
        assert_eq!(
            snapshot
                .active_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["task-in-progress", "task-ready-high", "task-ready-low"]
        );
        assert!(
            snapshot.active_tasks[0]
                .rank_reasons
                .iter()
                .any(|reason| reason == "status=in_progress")
        );
    }

    #[test]
    fn skips_tasks_when_direction_is_inactive_or_dependencies_are_unresolved() {
        let queue_service = PriorityQueueService::new();
        let directions = directions(&[
            ("direction-a", DirectionState::Active),
            ("direction-b", DirectionState::Paused),
        ]);
        let mut waiting_on_dependency = task(
            "waiting-on-dependency",
            "direction-a",
            TaskStatus::Ready,
            40,
            0,
            "2026-04-09T09:00:00Z",
        );
        waiting_on_dependency.depends_on = vec!["dependency-open".to_string()];
        let mut blocked_by_review = task(
            "blocked-by-review",
            "direction-a",
            TaskStatus::Ready,
            30,
            0,
            "2026-04-09T09:30:00Z",
        );
        blocked_by_review.blocked_by = vec!["review-open".to_string()];

        let task_ledger = TaskLedgerDocument {
            version: 1,
            tasks: vec![
                task(
                    "dependency-open",
                    "direction-a",
                    TaskStatus::Ready,
                    90,
                    0,
                    "2026-04-09T08:00:00Z",
                ),
                waiting_on_dependency,
                task(
                    "paused-task",
                    "direction-b",
                    TaskStatus::Ready,
                    100,
                    0,
                    "2026-04-09T07:00:00Z",
                ),
                task(
                    "review-open",
                    "direction-a",
                    TaskStatus::InProgress,
                    20,
                    0,
                    "2026-04-09T10:00:00Z",
                ),
                blocked_by_review,
                task(
                    "proposed-followup",
                    "direction-a",
                    TaskStatus::Proposed,
                    10,
                    0,
                    "2026-04-09T11:00:00Z",
                ),
            ],
        };

        let snapshot = queue_service.build_snapshot(&directions, &task_ledger);

        assert_eq!(
            snapshot
                .active_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["review-open", "dependency-open"]
        );
        assert!(
            snapshot
                .skipped_tasks
                .iter()
                .any(|task| task.task_id == "paused-task" && task.reason.contains("paused"))
        );
        assert!(snapshot.skipped_tasks.iter().any(|task| {
            task.task_id == "waiting-on-dependency"
                && task.reason.contains("dependency-open(ready)")
        }));
        assert!(snapshot.skipped_tasks.iter().any(|task| {
            task.task_id == "blocked-by-review" && task.reason.contains("review-open(in_progress)")
        }));
        assert!(snapshot.skipped_tasks.iter().any(|task| {
            task.task_id == "proposed-followup"
                && task.reason.contains("status proposed is not executable")
        }));
    }

    #[test]
    fn breaks_priority_ties_with_oldest_update_time() {
        let queue_service = PriorityQueueService::new();
        let directions = directions(&[("direction-a", DirectionState::Active)]);
        let task_ledger = TaskLedgerDocument {
            version: 1,
            tasks: vec![
                task(
                    "recently-updated",
                    "direction-a",
                    TaskStatus::Ready,
                    50,
                    5,
                    "2026-04-09T12:00:00Z",
                ),
                task(
                    "older-task",
                    "direction-a",
                    TaskStatus::Ready,
                    45,
                    10,
                    "2026-04-09T08:00:00Z",
                ),
            ],
        };

        let snapshot = queue_service.build_snapshot(&directions, &task_ledger);

        assert_eq!(
            snapshot
                .active_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["older-task", "recently-updated"]
        );
        assert!(
            snapshot.active_tasks[0]
                .rank_reasons
                .iter()
                .any(|reason| reason.contains("priority_reason=recent result raised urgency"))
        );
        assert_eq!(snapshot.visible_tasks(1)[0].task_id, "older-task");
    }
}

use std::{collections::HashMap, fmt};

use chrono::DateTime;

use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, PriorityQueueProjection,
    PriorityQueueSkippedTask, PriorityQueueTask, TaskDefinition, TaskLedgerDocument,
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
                "task-ledger.json may contain at most one in_progress task; found {}: {}",
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
        task_ledger: &TaskLedgerDocument,
    ) -> Result<PriorityQueueProjection, PriorityQueueBuildError> {
        let direction_map = directions
            .directions
            .iter()
            .map(|direction| (direction.id.trim(), direction))
            .collect::<HashMap<_, _>>();
        let task_map = task_ledger
            .tasks
            .iter()
            .map(|task| (task.id.trim(), task))
            .collect::<HashMap<_, _>>();
        let updated_at_epoch_millis_by_task_id =
            self.validate_queue_inputs(task_ledger, &direction_map, &task_map)?;

        let mut candidates = Vec::new();
        let mut proposed_candidates = Vec::new();
        let mut skipped_tasks = Vec::new();

        for task in &task_ledger.tasks {
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
        task_ledger: &'a TaskLedgerDocument,
        direction_map: &HashMap<&'a str, &'a DirectionDefinition>,
        task_map: &HashMap<&'a str, &'a TaskDefinition>,
    ) -> Result<HashMap<&'a str, i64>, PriorityQueueBuildError> {
        let in_progress_tasks = task_ledger
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
        for task in &task_ledger.tasks {
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
mod tests {
    use std::collections::HashMap;

    use super::{PriorityQueueBuildError, PriorityQueueService};
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, QueueIdleConfig, TaskActor,
        TaskDefinition, TaskLedgerDocument, TaskStatus,
    };

    fn directions(states: &[(&str, DirectionState)]) -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: 1,
            queue_idle: QueueIdleConfig::default(),
            directions: states
                .iter()
                .map(|(id, state)| DirectionDefinition {
                    id: (*id).to_string(),
                    title: format!("{id} title"),
                    summary: format!("{id} summary"),
                    success_criteria: vec![format!("{id} done")],
                    scope_hints: vec![format!("{id} hint")],
                    detail_doc_path: String::new(),
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

        let snapshot = queue_service
            .build_projection(&directions, &task_ledger)
            .expect("queue projection should build");

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

        let snapshot = queue_service
            .build_projection(&directions, &task_ledger)
            .expect("queue projection should build");

        assert_eq!(
            snapshot
                .active_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["review-open", "dependency-open"]
        );
        assert_eq!(
            snapshot
                .proposed_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["proposed-followup"]
        );
        assert_eq!(snapshot.proposed_tasks[0].combined_priority, 10);
        let skipped = snapshot
            .skipped_tasks
            .iter()
            .map(|task| (task.task_id.as_str(), task.reason.as_str()))
            .collect::<HashMap<_, _>>();
        assert_eq!(skipped.len(), 3);
        assert!(skipped["paused-task"].contains("paused"));
        assert!(skipped["waiting-on-dependency"].contains("dependency-open(ready)"));
        assert!(skipped["blocked-by-review"].contains("review-open(in_progress)"));
    }

    #[test]
    fn excludes_non_promotable_proposals_from_proposed_queue() {
        let queue_service = PriorityQueueService::new();
        let directions = directions(&[
            ("direction-a", DirectionState::Active),
            ("direction-b", DirectionState::Paused),
        ]);
        let mut blocked_proposal = task(
            "blocked-proposal",
            "direction-a",
            TaskStatus::Proposed,
            70,
            5,
            "2026-04-09T09:30:00Z",
        );
        blocked_proposal.depends_on = vec!["blocking-ready".to_string()];

        let task_ledger = TaskLedgerDocument {
            version: 1,
            tasks: vec![
                task(
                    "blocking-ready",
                    "direction-a",
                    TaskStatus::Ready,
                    95,
                    0,
                    "2026-04-09T07:30:00Z",
                ),
                task(
                    "ready-proposal",
                    "direction-a",
                    TaskStatus::Proposed,
                    50,
                    10,
                    "2026-04-09T08:00:00Z",
                ),
                blocked_proposal,
                task(
                    "paused-proposal",
                    "direction-b",
                    TaskStatus::Proposed,
                    90,
                    0,
                    "2026-04-09T07:00:00Z",
                ),
            ],
        };

        let snapshot = queue_service
            .build_projection(&directions, &task_ledger)
            .expect("queue projection should build");

        assert_eq!(
            snapshot
                .proposed_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ready-proposal"]
        );
        let skipped = snapshot
            .skipped_tasks
            .iter()
            .map(|task| (task.task_id.as_str(), task.reason.as_str()))
            .collect::<HashMap<_, _>>();
        assert!(skipped["blocked-proposal"].contains("blocking-ready(ready)"));
        assert!(skipped["paused-proposal"].contains("direction direction-b is paused"));
    }

    #[test]
    fn trims_direction_and_task_ids_for_queue_resolution() {
        let queue_service = PriorityQueueService::new();
        let directions = directions(&[("direction-a", DirectionState::Active)]);
        let mut runnable_task = task(
            "  runnable-task  ",
            " direction-a ",
            TaskStatus::Ready,
            50,
            0,
            "2026-04-09T11:00:00Z",
        );
        runnable_task.depends_on = vec!["dependency-task".to_string()];
        runnable_task.blocked_by = vec![" blocker-task ".to_string()];

        let task_ledger = TaskLedgerDocument {
            version: 1,
            tasks: vec![
                task(
                    " dependency-task ",
                    "direction-a",
                    TaskStatus::Done,
                    10,
                    0,
                    "2026-04-09T09:00:00Z",
                ),
                task(
                    "blocker-task",
                    "direction-a",
                    TaskStatus::Done,
                    20,
                    0,
                    "2026-04-09T10:00:00Z",
                ),
                runnable_task,
            ],
        };

        let snapshot = queue_service
            .build_projection(&directions, &task_ledger)
            .expect("queue projection should build");

        assert_eq!(
            snapshot
                .next_task
                .as_ref()
                .map(|task| task.task_id.as_str()),
            Some("  runnable-task  ")
        );
        assert!(
            snapshot
                .active_tasks
                .iter()
                .any(|task| task.task_id == "  runnable-task  ")
        );
        assert!(
            snapshot
                .skipped_tasks
                .iter()
                .all(|task| task.task_id != "  runnable-task  ")
        );
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

        let snapshot = queue_service
            .build_projection(&directions, &task_ledger)
            .expect("queue projection should build");

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

    #[test]
    fn awaiting_user_blockers_clear_for_downstream_queue_tasks() {
        let queue_service = PriorityQueueService::new();
        let directions = directions(&[("direction-a", DirectionState::Active)]);
        let mut blocked_task = task(
            "blocked-task",
            "direction-a",
            TaskStatus::Ready,
            40,
            0,
            "2026-04-09T09:30:00Z",
        );
        blocked_task.blocked_by = vec!["user-input".to_string()];

        let task_ledger = TaskLedgerDocument {
            version: 1,
            tasks: vec![
                task(
                    "user-input",
                    "direction-a",
                    TaskStatus::AwaitingUser,
                    20,
                    0,
                    "2026-04-09T09:00:00Z",
                ),
                blocked_task,
            ],
        };

        let snapshot = queue_service
            .build_projection(&directions, &task_ledger)
            .expect("queue projection should build");

        assert_eq!(
            snapshot
                .active_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["blocked-task"]
        );
        assert!(
            snapshot
                .skipped_tasks
                .iter()
                .all(|task| task.task_id != "blocked-task")
        );
    }

    #[test]
    fn rejects_unknown_direction_references_instead_of_skipping_them() {
        let queue_service = PriorityQueueService::new();
        let directions = directions(&[("direction-a", DirectionState::Active)]);
        let task_ledger = TaskLedgerDocument {
            version: 1,
            tasks: vec![task(
                "task-1",
                "missing-direction",
                TaskStatus::Ready,
                30,
                0,
                "2026-04-09T09:00:00Z",
            )],
        };

        let error = queue_service
            .build_projection(&directions, &task_ledger)
            .expect_err("queue build should reject unknown directions");

        assert_eq!(
            error,
            PriorityQueueBuildError::UnknownDirection {
                task_id: "task-1".to_string(),
                direction_id: "missing-direction".to_string(),
            }
        );
    }

    #[test]
    fn rejects_missing_dependency_references_instead_of_skipping_them() {
        let queue_service = PriorityQueueService::new();
        let directions = directions(&[("direction-a", DirectionState::Active)]);
        let mut blocked_task = task(
            "blocked-task",
            "direction-a",
            TaskStatus::Ready,
            30,
            0,
            "2026-04-09T09:00:00Z",
        );
        blocked_task.depends_on = vec!["missing-task".to_string()];
        let task_ledger = TaskLedgerDocument {
            version: 1,
            tasks: vec![blocked_task],
        };

        let error = queue_service
            .build_projection(&directions, &task_ledger)
            .expect_err("queue build should reject missing dependency references");

        assert_eq!(
            error,
            PriorityQueueBuildError::MissingDependency {
                task_id: "blocked-task".to_string(),
                dependency_id: "missing-task".to_string(),
            }
        );
    }

    #[test]
    fn rejects_invalid_updated_at_instead_of_silently_reordering() {
        let queue_service = PriorityQueueService::new();
        let directions = directions(&[("direction-a", DirectionState::Active)]);
        let task_ledger = TaskLedgerDocument {
            version: 1,
            tasks: vec![task(
                "task-1",
                "direction-a",
                TaskStatus::Ready,
                30,
                0,
                "not-a-timestamp",
            )],
        };

        let error = queue_service
            .build_projection(&directions, &task_ledger)
            .expect_err("queue build should reject invalid updated_at values");

        assert_eq!(
            error,
            PriorityQueueBuildError::InvalidUpdatedAt {
                task_id: "task-1".to_string(),
                updated_at: "not-a-timestamp".to_string(),
            }
        );
    }

    #[test]
    fn rejects_multiple_in_progress_tasks_during_queue_build() {
        let queue_service = PriorityQueueService::new();
        let directions = directions(&[("direction-a", DirectionState::Active)]);
        let task_ledger = TaskLedgerDocument {
            version: 1,
            tasks: vec![
                task(
                    "task-1",
                    "direction-a",
                    TaskStatus::InProgress,
                    30,
                    0,
                    "2026-04-09T09:00:00Z",
                ),
                task(
                    "task-2",
                    "direction-a",
                    TaskStatus::InProgress,
                    20,
                    0,
                    "2026-04-09T09:10:00Z",
                ),
            ],
        };

        let error = queue_service
            .build_projection(&directions, &task_ledger)
            .expect_err("queue build should reject multiple in_progress tasks");

        assert_eq!(
            error,
            PriorityQueueBuildError::MultipleInProgressTasks {
                task_ids: vec!["task-1".to_string(), "task-2".to_string()],
            }
        );
    }
}

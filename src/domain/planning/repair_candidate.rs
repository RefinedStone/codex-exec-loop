use chrono::DateTime;

use super::{PriorityQueueProjection, TaskAuthorityDocument, TaskDefinition};

#[derive(Debug, Default, Clone)]
// repair candidate policy는 accepted authority와 worker candidate 사이의 순수 eligibility guard다.
pub struct PlanningRepairCandidatePolicy;

impl PlanningRepairCandidatePolicy {
    pub fn new() -> Self {
        Self
    }

    pub fn stale_candidate_failure(
        &self,
        accepted_task_authority: Option<&TaskAuthorityDocument>,
        candidate_task_authority: &TaskAuthorityDocument,
    ) -> Option<String> {
        let accepted_task_authority = accepted_task_authority?;
        for accepted_task in &accepted_task_authority.tasks {
            let task_id = accepted_task.id.trim();
            let Some(candidate_task) = find_task(candidate_task_authority, task_id) else {
                return Some(format!(
                    "planning worker task authority candidate removed accepted DB task `{task_id}`"
                ));
            };
            if accepted_task.status.is_terminal() && candidate_task.status != accepted_task.status {
                return Some(format!(
                    "planning worker task authority candidate regressed accepted DB task `{task_id}` from `{}` to `{}`",
                    accepted_task.status.label(),
                    candidate_task.status.label()
                ));
            }
            if timestamp_regressed(&candidate_task.updated_at, &accepted_task.updated_at) {
                return Some(format!(
                    "planning worker task authority candidate regressed accepted DB task `{task_id}` updated_at from `{}` to `{}`",
                    accepted_task.updated_at.trim(),
                    candidate_task.updated_at.trim()
                ));
            }
        }
        None
    }

    pub fn queue_advancement_failure(
        &self,
        previous_handoff: Option<PlanningRepairPreviousHandoff<'_>>,
        accepted_task_authority: Option<&TaskAuthorityDocument>,
        candidate_task_authority: &TaskAuthorityDocument,
        queue_projection: &PriorityQueueProjection,
    ) -> Option<String> {
        let previous_handoff = previous_handoff?;
        let queue_head = queue_projection.next_task.as_ref()?;
        if queue_head.task_id.trim() != previous_handoff.task_id.trim() {
            return None;
        }
        let accepted_task = accepted_task_authority
            .and_then(|task_authority| find_task(task_authority, previous_handoff.task_id));
        let candidate_task = find_task(candidate_task_authority, previous_handoff.task_id)?;
        match accepted_task {
            Some(accepted_task)
                if accepted_task.normalized() == candidate_task.normalized()
                    && queue_head.status.label() == previous_handoff.status_label.trim() =>
            {
                Some(format!(
                    "planning worker refresh kept previous handoff `{}` unchanged as the ready queue head",
                    previous_handoff.task_id.trim()
                ))
            }
            None if candidate_task.updated_at.trim() == previous_handoff.updated_at.trim() => {
                Some(format!(
                    "planning worker refresh returned previous handoff `{}` as the queue head without DB baseline evidence of a task update",
                    previous_handoff.task_id.trim()
                ))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningRepairPreviousHandoff<'a> {
    pub task_id: &'a str,
    pub task_title: &'a str,
    pub updated_at: &'a str,
    pub status_label: &'a str,
}

fn find_task<'a>(
    task_authority: &'a TaskAuthorityDocument,
    task_id: &str,
) -> Option<&'a TaskDefinition> {
    let task_id = task_id.trim();
    task_authority
        .tasks
        .iter()
        .find(|task| task.id.trim() == task_id)
}

fn timestamp_regressed(candidate_updated_at: &str, accepted_updated_at: &str) -> bool {
    let candidate_updated_at = candidate_updated_at.trim();
    let accepted_updated_at = accepted_updated_at.trim();
    if candidate_updated_at.is_empty() || accepted_updated_at.is_empty() {
        return false;
    }
    let Ok(candidate_updated_at) = DateTime::parse_from_rfc3339(candidate_updated_at) else {
        return false;
    };
    let Ok(accepted_updated_at) = DateTime::parse_from_rfc3339(accepted_updated_at) else {
        return false;
    };

    candidate_updated_at < accepted_updated_at
}

#[cfg(test)]
mod tests {
    use super::{PlanningRepairCandidatePolicy, PlanningRepairPreviousHandoff};
    use crate::domain::planning::{
        PLANNING_FORMAT_VERSION, PriorityQueueProjection, PriorityQueueTask, TaskActor,
        TaskAuthorityDocument, TaskDefinition, TaskStatus,
    };

    #[test]
    fn rejects_unchanged_previous_handoff_queue_head() {
        let accepted = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", TaskStatus::Ready, "2026-04-29T00:00:00Z")],
        };
        let projection = PriorityQueueProjection {
            next_task: Some(queue_task("task-1", TaskStatus::Ready)),
            active_tasks: vec![queue_task("task-1", TaskStatus::Ready)],
            proposed_tasks: Vec::new(),
            skipped_tasks: Vec::new(),
        };

        let failure = PlanningRepairCandidatePolicy::new().queue_advancement_failure(
            Some(PlanningRepairPreviousHandoff {
                task_id: "task-1",
                task_title: "Task 1",
                updated_at: "2026-04-29T00:00:00Z",
                status_label: "ready",
            }),
            Some(&accepted),
            &accepted,
            &projection,
        );

        assert_eq!(
            failure.as_deref(),
            Some(
                "planning worker refresh kept previous handoff `task-1` unchanged as the ready queue head"
            )
        );
    }

    #[test]
    fn allows_updated_same_queue_head() {
        let accepted = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", TaskStatus::Ready, "2026-04-29T00:00:00Z")],
        };
        let candidate = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", TaskStatus::Ready, "2026-04-29T00:01:00Z")],
        };
        let projection = PriorityQueueProjection {
            next_task: Some(queue_task("task-1", TaskStatus::Ready)),
            active_tasks: vec![queue_task("task-1", TaskStatus::Ready)],
            proposed_tasks: Vec::new(),
            skipped_tasks: Vec::new(),
        };

        let failure = PlanningRepairCandidatePolicy::new().queue_advancement_failure(
            Some(PlanningRepairPreviousHandoff {
                task_id: "task-1",
                task_title: "Task 1",
                updated_at: "2026-04-29T00:00:00Z",
                status_label: "ready",
            }),
            Some(&accepted),
            &candidate,
            &projection,
        );

        assert_eq!(failure, None);
    }

    #[test]
    fn rejects_accepted_db_status_regression() {
        let accepted = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task(
                "planning-prompt-assembly-remaining-surface-slice",
                TaskStatus::Done,
                "2026-04-29T03:00:32Z",
            )],
        };
        let stale_candidate = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task(
                "planning-prompt-assembly-remaining-surface-slice",
                TaskStatus::Ready,
                "2026-04-29T01:43:52Z",
            )],
        };

        let failure = PlanningRepairCandidatePolicy::new()
            .stale_candidate_failure(Some(&accepted), &stale_candidate);

        assert_eq!(
            failure.as_deref(),
            Some(
                "planning worker task authority candidate regressed accepted DB task `planning-prompt-assembly-remaining-surface-slice` from `done` to `ready`"
            )
        );
    }

    #[test]
    fn rejects_older_accepted_db_timestamp() {
        let accepted = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", TaskStatus::Ready, "2026-04-29T03:00:32Z")],
        };
        let stale_candidate = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", TaskStatus::Ready, "2026-04-29T01:43:52Z")],
        };

        let failure = PlanningRepairCandidatePolicy::new()
            .stale_candidate_failure(Some(&accepted), &stale_candidate);

        assert_eq!(
            failure.as_deref(),
            Some(
                "planning worker task authority candidate regressed accepted DB task `task-1` updated_at from `2026-04-29T03:00:32Z` to `2026-04-29T01:43:52Z`"
            )
        );
    }

    #[test]
    fn compares_rfc3339_timestamps_by_time() {
        let accepted = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task(
                "task-1",
                TaskStatus::Ready,
                "2026-04-29T03:00:32+00:00",
            )],
        };
        let candidate = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task(
                "task-1",
                TaskStatus::Ready,
                "2026-04-29T03:00:32.500Z",
            )],
        };

        let failure = PlanningRepairCandidatePolicy::new()
            .stale_candidate_failure(Some(&accepted), &candidate);

        assert_eq!(failure, None);
    }

    #[test]
    fn stale_candidate_guard_covers_absent_baseline_removed_task_and_timestamp_parse_edges() {
        let policy = PlanningRepairCandidatePolicy::new();
        let candidate = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", TaskStatus::Ready, "2026-04-29T00:00:00Z")],
        };

        assert_eq!(policy.stale_candidate_failure(None, &candidate), None);

        let accepted_removed = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-2", TaskStatus::Ready, "2026-04-29T00:00:00Z")],
        };
        assert_eq!(
            policy
                .stale_candidate_failure(Some(&accepted_removed), &candidate)
                .as_deref(),
            Some("planning worker task authority candidate removed accepted DB task `task-2`")
        );

        for (candidate_updated_at, accepted_updated_at) in [
            ("", "2026-04-29T00:00:00Z"),
            ("not-rfc3339", "2026-04-29T00:00:00Z"),
            ("2026-04-29T00:00:00Z", "not-rfc3339"),
        ] {
            let accepted = TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: vec![task("task-1", TaskStatus::Ready, accepted_updated_at)],
            };
            let candidate = TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: vec![task("task-1", TaskStatus::Ready, candidate_updated_at)],
            };

            assert_eq!(
                policy.stale_candidate_failure(Some(&accepted), &candidate),
                None
            );
        }
    }

    #[test]
    fn queue_advancement_guard_covers_noop_missing_and_no_baseline_edges() {
        let policy = PlanningRepairCandidatePolicy::new();
        let candidate = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", TaskStatus::Ready, "2026-04-29T00:00:00Z")],
        };
        let matching_handoff = Some(PlanningRepairPreviousHandoff {
            task_id: "task-1",
            task_title: "Task 1",
            updated_at: "2026-04-29T00:00:00Z",
            status_label: "ready",
        });
        let matching_projection = PriorityQueueProjection {
            next_task: Some(queue_task("task-1", TaskStatus::Ready)),
            active_tasks: Vec::new(),
            proposed_tasks: Vec::new(),
            skipped_tasks: Vec::new(),
        };

        assert_eq!(
            policy.queue_advancement_failure(None, None, &candidate, &matching_projection),
            None
        );
        assert_eq!(
            policy.queue_advancement_failure(
                matching_handoff,
                None,
                &candidate,
                &PriorityQueueProjection {
                    next_task: None,
                    active_tasks: Vec::new(),
                    proposed_tasks: Vec::new(),
                    skipped_tasks: Vec::new(),
                },
            ),
            None
        );
        assert_eq!(
            policy.queue_advancement_failure(
                matching_handoff,
                None,
                &candidate,
                &PriorityQueueProjection {
                    next_task: Some(queue_task("other-task", TaskStatus::Ready)),
                    active_tasks: Vec::new(),
                    proposed_tasks: Vec::new(),
                    skipped_tasks: Vec::new(),
                },
            ),
            None
        );
        assert_eq!(
            policy.queue_advancement_failure(
                matching_handoff,
                None,
                &TaskAuthorityDocument {
                    version: PLANNING_FORMAT_VERSION,
                    tasks: Vec::new(),
                },
                &matching_projection,
            ),
            None
        );
        assert_eq!(
            policy
                .queue_advancement_failure(matching_handoff, None, &candidate, &matching_projection)
                .as_deref(),
            Some(
                "planning worker refresh returned previous handoff `task-1` as the queue head without DB baseline evidence of a task update"
            )
        );
    }

    fn task(id: &str, status: TaskStatus, updated_at: &str) -> TaskDefinition {
        TaskDefinition {
            id: id.to_string(),
            direction_id: "direction-a".to_string(),
            direction_relation_note: "supports direction".to_string(),
            title: "Task 1".to_string(),
            description: "Do task 1".to_string(),
            status,
            base_priority: 10,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::Worker,
            last_updated_by: TaskActor::Worker,
            source_turn_id: None,
            provenance: Default::default(),
            updated_at: updated_at.to_string(),
        }
    }

    fn queue_task(id: &str, status: TaskStatus) -> PriorityQueueTask {
        PriorityQueueTask {
            rank: 1,
            task_id: id.to_string(),
            direction_id: "direction-a".to_string(),
            direction_title: "Direction A".to_string(),
            task_title: "Task 1".to_string(),
            status,
            combined_priority: 10,
            updated_at: "2026-04-29T00:00:00Z".to_string(),
            rank_reasons: vec!["status=ready".to_string()],
        }
    }
}

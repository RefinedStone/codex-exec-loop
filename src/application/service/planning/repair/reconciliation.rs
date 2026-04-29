use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskAuthoritySnapshot,
    PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning::shared::contract::{
    RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path,
};
use crate::domain::planning::PlanningWorkspaceFiles;
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    PriorityQueueProjection, TaskAuthorityDocument, TaskDefinition, TaskStatus,
};

pub use super::ledger_recovery::PlanningQueueProjectionAction;
pub use super::prompt::{
    PlanningRepairPromptHandoff, PlanningRepairRetryReason, build_planning_repair_prompt,
};
pub use super::protected_restore::PlanningProtectedFileRestoration;
use crate::application::service::planning::runtime::validation::PlanningValidationService;

#[derive(Clone)]
pub struct PlanningReconciliationService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanningExecutionSnapshot {
    pub result_output_markdown: Option<String>,
}

impl PlanningExecutionSnapshot {
    pub fn captures_path(path: &str) -> bool {
        canonical_active_planning_file_path(path).is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanningReconciliationResult {
    pub notices: Vec<String>,
    pub restored_protected_files: Vec<PlanningProtectedFileRestoration>,
    pub rejected_task_authority: bool,
    pub rejected_archive_path: Option<String>,
    pub queue_projection_action: Option<PlanningQueueProjectionAction>,
    pub repair_request: Option<PlanningRepairRequest>,
    pub auto_followup_block_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRepairRequest {
    pub failure_summary: String,
    pub validation_errors: Vec<String>,
    pub direction_authority_json: String,
    pub accepted_task_authority_json: String,
    pub accepted_queue_projection_json: String,
    pub rejected_task_authority_json: Option<String>,
    pub rejected_archive_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PlanningChangeSet {
    pub(super) result_output_changed: bool,
}

impl PlanningChangeSet {
    fn from_paths(paths: &[String]) -> Self {
        let mut change_set = Self::default();
        for path in paths {
            if let Some(RESULT_OUTPUT_FILE_PATH) = canonical_active_planning_file_path(path) {
                change_set.result_output_changed = true;
            }
        }
        change_set
    }

    fn has_relevant_changes(self) -> bool {
        self.result_output_changed
    }
}

impl PlanningReconciliationService {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self::with_task_repository(
            planning_workspace_port,
            planning_validation_service,
            priority_queue_service,
            Arc::new(crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort),
        )
    }

    pub fn with_task_repository(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_validation_service,
            priority_queue_service,
            planning_task_repository_port,
        }
    }

    pub fn load_execution_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningExecutionSnapshot> {
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;

        Ok(PlanningExecutionSnapshot {
            result_output_markdown: workspace_record.result_output_markdown,
        })
    }

    pub fn reconcile_after_turn(
        &self,
        workspace_dir: &str,
        _turn_id: &str,
        changed_planning_file_paths: &[String],
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> Result<PlanningReconciliationResult> {
        let change_set = PlanningChangeSet::from_paths(changed_planning_file_paths);
        if !change_set.has_relevant_changes() {
            return Ok(PlanningReconciliationResult::default());
        }

        let mut result = PlanningReconciliationResult::default();
        self.planning_workspace_port
            .commit_planning_workspace_files(
                workspace_dir,
                &execution_snapshot_to_workspace_record(execution_snapshot),
            )?;
        result
            .notices
            .push("planning reconciliation restored protected planning files".to_string());

        Ok(result)
    }

    pub fn commit_task_authority_candidate(
        &self,
        workspace_dir: &str,
        candidate_task_authority_json: &str,
        execution_snapshot: &PlanningExecutionSnapshot,
        previous_handoff: Option<PlanningRepairPromptHandoff<'_>>,
    ) -> Result<PlanningReconciliationResult> {
        let mut result = PlanningReconciliationResult::default();
        let direction_snapshot = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(workspace_dir)?
            .ok_or_else(|| anyhow::anyhow!("planning direction authority is unavailable"))?;
        let direction_authority_json =
            serde_json::to_string_pretty(&direction_snapshot.directions)?;
        let result_output_markdown = execution_snapshot
            .result_output_markdown
            .as_deref()
            .unwrap_or_default();
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions: &direction_snapshot.directions,
                    task_authority_json: candidate_task_authority_json,
                    result_output_markdown,
                });

        if !validation_result.is_valid() {
            let validation_errors = validation_error_summaries(&validation_result);
            let failure_summary = validation_errors
                .first()
                .cloned()
                .unwrap_or_else(|| "unknown validation failure".to_string());
            let accepted_snapshot = self
                .planning_task_repository_port
                .load_task_authority_snapshot(workspace_dir)?;
            let (accepted_task_authority_json, accepted_queue_projection_json) =
                accepted_authority_prompt_jsons(accepted_snapshot.as_ref())?;
            result.repair_request = Some(PlanningRepairRequest {
                failure_summary: failure_summary.clone(),
                validation_errors,
                direction_authority_json,
                accepted_task_authority_json,
                accepted_queue_projection_json,
                rejected_task_authority_json: Some(candidate_task_authority_json.to_string()),
                rejected_archive_path: None,
            });
            result.rejected_task_authority = true;
            result.notices.push(format!(
                "planning worker produced an invalid task authority update ({failure_summary})"
            ));
            return Ok(result);
        }

        let directions = validation_result.directions.as_ref().ok_or_else(|| {
            anyhow::anyhow!("planning validation reported success without direction authority")
        })?;
        let task_authority = validation_result.task_authority.as_ref().ok_or_else(|| {
            anyhow::anyhow!("planning validation reported success without parsed task authority")
        })?;
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_authority)
            .map_err(|error| {
                anyhow::anyhow!("planning validation passed but queue build failed: {error}")
            })?;
        let authority_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?;
        if let Some(failure_summary) = stale_candidate_guard_failure(
            authority_snapshot
                .as_ref()
                .map(|snapshot| &snapshot.task_authority),
            task_authority,
        ) {
            let (accepted_task_authority_json, accepted_queue_projection_json) =
                accepted_authority_prompt_jsons(authority_snapshot.as_ref())?;
            result.repair_request = Some(PlanningRepairRequest {
                failure_summary: failure_summary.clone(),
                validation_errors: vec![failure_summary.clone()],
                direction_authority_json,
                accepted_task_authority_json,
                accepted_queue_projection_json,
                rejected_task_authority_json: Some(candidate_task_authority_json.to_string()),
                rejected_archive_path: None,
            });
            result.rejected_task_authority = true;
            result.notices.push(format!(
                "planning worker produced a stale task authority update ({failure_summary})"
            ));
            return Ok(result);
        }
        if let Some(failure_summary) = queue_advancement_guard_failure(
            previous_handoff,
            authority_snapshot
                .as_ref()
                .map(|snapshot| &snapshot.task_authority),
            task_authority,
            &queue_projection,
        ) {
            let (accepted_task_authority_json, accepted_queue_projection_json) =
                accepted_authority_prompt_jsons(authority_snapshot.as_ref())?;
            result.repair_request = Some(PlanningRepairRequest {
                failure_summary: failure_summary.clone(),
                validation_errors: vec![failure_summary.clone()],
                direction_authority_json,
                accepted_task_authority_json,
                accepted_queue_projection_json,
                rejected_task_authority_json: Some(candidate_task_authority_json.to_string()),
                rejected_archive_path: None,
            });
            result.rejected_task_authority = true;
            result.notices.push(format!(
                "planning worker produced a non-advancing task authority update ({failure_summary})"
            ));
            return Ok(result);
        }
        match self
            .planning_task_repository_port
            .commit_task_authority_snapshot(
                workspace_dir,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: authority_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.planning_revision),
                    task_authority,
                    queue_projection: &queue_projection,
                },
            )? {
            PlanningTaskAuthorityCommitResult::Committed { .. } => {}
            PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision,
                current_planning_revision,
            } => {
                anyhow::bail!(
                    "planning db changed while committing worker task authority update (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                );
            }
        }

        result.queue_projection_action =
            Some(PlanningQueueProjectionAction::RebuiltFromAcceptedPlanning);
        result
            .notices
            .push("planning worker committed DB task authority update".to_string());
        Ok(result)
    }
}

pub(super) fn execution_snapshot_to_workspace_record(
    execution_snapshot: &PlanningExecutionSnapshot,
) -> PlanningWorkspaceLoadRecord {
    PlanningWorkspaceLoadRecord {
        result_output_markdown: execution_snapshot.result_output_markdown.clone(),
    }
}

fn accepted_authority_prompt_jsons(
    snapshot: Option<&PlanningTaskAuthoritySnapshot>,
) -> Result<(String, String)> {
    let Some(snapshot) = snapshot else {
        return Ok((String::new(), String::new()));
    };

    Ok((
        serde_json::to_string_pretty(&snapshot.task_authority)?,
        serde_json::to_string_pretty(&snapshot.queue_projection)?,
    ))
}

fn stale_candidate_guard_failure(
    accepted_task_authority: Option<&TaskAuthorityDocument>,
    candidate_task_authority: &TaskAuthorityDocument,
) -> Option<String> {
    let accepted_task_authority = accepted_task_authority?;
    for accepted_task in &accepted_task_authority.tasks {
        let task_id = accepted_task.id.trim();
        let Some(candidate_task) = find_task(candidate_task_authority, task_id) else {
            return Some(format!(
                "planner task authority candidate removed accepted DB task `{task_id}`"
            ));
        };

        if terminal_status(accepted_task.status) && candidate_task.status != accepted_task.status {
            return Some(format!(
                "planner task authority candidate regressed accepted DB task `{task_id}` from `{}` to `{}`",
                accepted_task.status.label(),
                candidate_task.status.label()
            ));
        }

        if timestamp_regressed(&candidate_task.updated_at, &accepted_task.updated_at) {
            return Some(format!(
                "planner task authority candidate regressed accepted DB task `{task_id}` updated_at from `{}` to `{}`",
                accepted_task.updated_at.trim(),
                candidate_task.updated_at.trim()
            ));
        }
    }
    None
}

fn terminal_status(status: TaskStatus) -> bool {
    matches!(status, TaskStatus::Done | TaskStatus::Cancelled)
}

fn timestamp_regressed(candidate_updated_at: &str, accepted_updated_at: &str) -> bool {
    let candidate_updated_at = candidate_updated_at.trim();
    let accepted_updated_at = accepted_updated_at.trim();
    !candidate_updated_at.is_empty()
        && !accepted_updated_at.is_empty()
        && candidate_updated_at < accepted_updated_at
}

fn validation_error_summaries(
    validation_result: &crate::domain::planning::PlanningValidationResult,
) -> Vec<String> {
    validation_result
        .report
        .issues
        .iter()
        .filter(|issue| {
            issue.severity == crate::domain::planning::PlanningValidationSeverity::Error
        })
        .map(|issue| issue.message.clone())
        .collect()
}

fn queue_advancement_guard_failure(
    previous_handoff: Option<PlanningRepairPromptHandoff<'_>>,
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
                "planner refresh kept previous handoff `{}` unchanged as the ready queue head",
                previous_handoff.task_id.trim()
            ))
        }
        None if candidate_task.updated_at.trim() == previous_handoff.updated_at.trim() => {
            Some(format!(
                "planner refresh returned previous handoff `{}` as the queue head without DB baseline evidence of a task update",
                previous_handoff.task_id.trim()
            ))
        }
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::{
        PlanningChangeSet, PlanningRepairPromptHandoff, PlanningRepairRequest,
        build_planning_repair_prompt, queue_advancement_guard_failure,
        stale_candidate_guard_failure,
    };
    use crate::domain::planning::{
        PLANNING_FORMAT_VERSION, PriorityQueueProjection, PriorityQueueTask, TaskActor,
        TaskAuthorityDocument, TaskDefinition, TaskStatus,
    };

    #[test]
    fn change_set_ignores_legacy_task_file_paths() {
        let paths = vec![
            "DB task authority".to_string(),
            ".codex-exec-loop/planning/legacy-queue-snapshot.json".to_string(),
        ];

        let change_set = PlanningChangeSet::from_paths(&paths);

        assert!(!change_set.has_relevant_changes());
    }

    #[test]
    fn repair_prompt_requests_task_authority_payload_without_file_artifacts() {
        let prompt = build_planning_repair_prompt(
            &PlanningRepairRequest {
                failure_summary: "invalid task".to_string(),
                validation_errors: vec!["task has unknown direction".to_string()],
                direction_authority_json: "{\"version\":1,\"directions\":[]}".to_string(),
                accepted_task_authority_json: "{\"version\":1,\"tasks\":[]}".to_string(),
                accepted_queue_projection_json:
                    "{\"next_task\":null,\"active_tasks\":[],\"proposed_tasks\":[],\"skipped_tasks\":[]}"
                        .to_string(),
                rejected_task_authority_json: Some("{ invalid json".to_string()),
                rejected_archive_path: None,
            },
            None,
            1,
            2,
            None,
        );

        assert!(prompt.contains("\"task_authority\""));
        assert!(prompt.contains("[accepted-db-queue-projection]"));
        assert!(prompt.contains("task-ledger.json"));
        assert!(!prompt.contains("task authority schema file"));
        assert!(!prompt.contains("queue snapshot artifact"));
    }

    #[test]
    fn queue_advancement_guard_rejects_unchanged_previous_handoff_head() {
        let accepted = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", "ready", "2026-04-29T00:00:00Z")],
        };
        let projection = PriorityQueueProjection {
            next_task: Some(queue_task("task-1", TaskStatus::Ready)),
            active_tasks: vec![queue_task("task-1", TaskStatus::Ready)],
            proposed_tasks: Vec::new(),
            skipped_tasks: Vec::new(),
        };

        let failure = queue_advancement_guard_failure(
            Some(PlanningRepairPromptHandoff {
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
                "planner refresh kept previous handoff `task-1` unchanged as the ready queue head"
            )
        );
    }

    #[test]
    fn queue_advancement_guard_allows_updated_same_head() {
        let accepted = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", "ready", "2026-04-29T00:00:00Z")],
        };
        let candidate = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", "ready", "2026-04-29T00:01:00Z")],
        };
        let projection = PriorityQueueProjection {
            next_task: Some(queue_task("task-1", TaskStatus::Ready)),
            active_tasks: vec![queue_task("task-1", TaskStatus::Ready)],
            proposed_tasks: Vec::new(),
            skipped_tasks: Vec::new(),
        };

        let failure = queue_advancement_guard_failure(
            Some(PlanningRepairPromptHandoff {
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
    fn stale_candidate_guard_rejects_accepted_db_status_regression() {
        let accepted = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![
                task(
                    "planning-prompt-assembly-remaining-surface-slice",
                    "done",
                    "2026-04-29T03:00:32Z",
                ),
                task(
                    "planning-prompt-shared-section-catalog-slice",
                    "ready",
                    "2026-04-29T03:00:32Z",
                ),
            ],
        };
        let stale_candidate = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![
                task(
                    "planning-prompt-assembly-remaining-surface-slice",
                    "ready",
                    "2026-04-29T01:43:52Z",
                ),
                task(
                    "planning-prompt-shared-section-catalog-slice",
                    "proposed",
                    "2026-04-29T01:43:52Z",
                ),
            ],
        };

        let failure = stale_candidate_guard_failure(Some(&accepted), &stale_candidate);

        assert_eq!(
            failure.as_deref(),
            Some(
                "planner task authority candidate regressed accepted DB task `planning-prompt-assembly-remaining-surface-slice` from `done` to `ready`"
            )
        );
    }

    #[test]
    fn stale_candidate_guard_rejects_older_accepted_db_timestamp() {
        let accepted = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", "ready", "2026-04-29T03:00:32Z")],
        };
        let stale_candidate = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", "ready", "2026-04-29T01:43:52Z")],
        };

        let failure = stale_candidate_guard_failure(Some(&accepted), &stale_candidate);

        assert_eq!(
            failure.as_deref(),
            Some(
                "planner task authority candidate regressed accepted DB task `task-1` updated_at from `2026-04-29T03:00:32Z` to `2026-04-29T01:43:52Z`"
            )
        );
    }

    fn task(id: &str, status: &str, updated_at: &str) -> TaskDefinition {
        TaskDefinition {
            id: id.to_string(),
            direction_id: "direction-a".to_string(),
            direction_relation_note: "supports direction".to_string(),
            title: "Task 1".to_string(),
            description: "Do task 1".to_string(),
            status: match status {
                "ready" => TaskStatus::Ready,
                "done" => TaskStatus::Done,
                "proposed" => TaskStatus::Proposed,
                _ => panic!("unexpected status"),
            },
            base_priority: 10,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::Llm,
            last_updated_by: TaskActor::Llm,
            source_turn_id: None,
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

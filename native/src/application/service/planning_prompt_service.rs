use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::domain::planning::{
    DIRECTIONS_FILE_PATH, DirectionCatalogDocument, DirectionState, PlanningWorkspaceFiles,
    PriorityQueueSnapshot, QUEUE_SNAPSHOT_FILE_PATH, RESULT_OUTPUT_FILE_PATH,
    TASK_LEDGER_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
};

use super::planning_validation_service::PlanningValidationService;
use super::priority_queue_service::PriorityQueueService;

const MAX_VISIBLE_QUEUE_TASKS: usize = 5;
const MAX_SKIPPED_QUEUE_TASKS: usize = 3;

#[derive(Clone)]
pub struct PlanningPromptService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPromptContext {
    pub prompt_fragment: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningPromptContextAvailability {
    Uninitialized,
    Ready(PlanningPromptContext),
    Blocked { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPromptContextLoadResult {
    pub availability: PlanningPromptContextAvailability,
}

impl PlanningPromptContextLoadResult {
    pub fn uninitialized() -> Self {
        Self {
            availability: PlanningPromptContextAvailability::Uninitialized,
        }
    }

    pub fn ready(prompt_context: PlanningPromptContext) -> Self {
        Self {
            availability: PlanningPromptContextAvailability::Ready(prompt_context),
        }
    }

    pub fn blocked(reason: impl Into<String>) -> Self {
        Self {
            availability: PlanningPromptContextAvailability::Blocked {
                reason: reason.into(),
            },
        }
    }

    pub fn prompt_fragment(&self) -> Option<&str> {
        match &self.availability {
            PlanningPromptContextAvailability::Ready(prompt_context) => {
                Some(prompt_context.prompt_fragment.as_str())
            }
            PlanningPromptContextAvailability::Uninitialized
            | PlanningPromptContextAvailability::Blocked { .. } => None,
        }
    }

    pub fn preview_status_label(&self) -> &'static str {
        match &self.availability {
            PlanningPromptContextAvailability::Uninitialized => "inactive",
            PlanningPromptContextAvailability::Ready(_) => "ready",
            PlanningPromptContextAvailability::Blocked { .. } => "blocked",
        }
    }

    pub fn preview_detail(&self) -> Option<&str> {
        match &self.availability {
            PlanningPromptContextAvailability::Uninitialized => None,
            PlanningPromptContextAvailability::Ready(prompt_context) => {
                Some(prompt_context.summary.as_str())
            }
            PlanningPromptContextAvailability::Blocked { reason } => Some(reason.as_str()),
        }
    }

    pub fn blocks_auto_followup(&self) -> bool {
        matches!(
            &self.availability,
            PlanningPromptContextAvailability::Blocked { .. }
        )
    }
}

impl PlanningPromptService {
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_validation_service,
            priority_queue_service,
        }
    }

    pub fn load_prompt_context(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningPromptContextLoadResult> {
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;

        if !workspace_record.has_any_files() {
            return Ok(PlanningPromptContextLoadResult::uninitialized());
        }

        let missing_paths = missing_workspace_paths(&workspace_record);
        if !missing_paths.is_empty() {
            return Ok(PlanningPromptContextLoadResult::blocked(format!(
                "planning files incomplete: missing {}",
                missing_paths.join(", ")
            )));
        }

        let workspace_files = workspace_record_to_files(&workspace_record);
        let validation_result = self
            .planning_validation_service
            .validate_workspace_files(workspace_files);
        if !validation_result.is_valid() {
            let first_error = validation_result
                .report
                .errors()
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "planning validation failed".to_string());
            return Ok(PlanningPromptContextLoadResult::blocked(format!(
                "planning validation failed: {first_error}"
            )));
        }

        let directions = validation_result
            .directions
            .expect("valid planning directions should be available");
        let task_ledger = validation_result
            .task_ledger
            .expect("valid planning task ledger should be available");
        let queue_snapshot = self
            .priority_queue_service
            .build_snapshot(&directions, &task_ledger);
        let result_output_markdown = workspace_record
            .result_output_markdown
            .as_deref()
            .expect("complete planning workspace should include result output");

        Ok(PlanningPromptContextLoadResult::ready(
            PlanningPromptContext {
                prompt_fragment: build_prompt_fragment(
                    &directions,
                    &queue_snapshot,
                    result_output_markdown,
                ),
                summary: build_queue_summary(&queue_snapshot),
            },
        ))
    }
}

fn workspace_record_to_files(
    workspace_record: &PlanningWorkspaceLoadRecord,
) -> PlanningWorkspaceFiles<'_> {
    PlanningWorkspaceFiles {
        directions_toml: workspace_record
            .directions_toml
            .as_deref()
            .expect("complete planning workspace should include directions"),
        task_ledger_json: workspace_record
            .task_ledger_json
            .as_deref()
            .expect("complete planning workspace should include task ledger"),
        task_ledger_schema_json: workspace_record
            .task_ledger_schema_json
            .as_deref()
            .expect("complete planning workspace should include task-ledger schema"),
        result_output_markdown: workspace_record
            .result_output_markdown
            .as_deref()
            .expect("complete planning workspace should include result output"),
    }
}

fn missing_workspace_paths(workspace_record: &PlanningWorkspaceLoadRecord) -> Vec<&'static str> {
    let mut missing_paths = Vec::new();
    if workspace_record.directions_toml.is_none() {
        missing_paths.push(DIRECTIONS_FILE_PATH);
    }
    if workspace_record.task_ledger_json.is_none() {
        missing_paths.push(TASK_LEDGER_FILE_PATH);
    }
    if workspace_record.task_ledger_schema_json.is_none() {
        missing_paths.push(TASK_LEDGER_SCHEMA_FILE_PATH);
    }
    if workspace_record.result_output_markdown.is_none() {
        missing_paths.push(RESULT_OUTPUT_FILE_PATH);
    }
    missing_paths
}

fn build_prompt_fragment(
    directions: &DirectionCatalogDocument,
    queue_snapshot: &PriorityQueueSnapshot,
    result_output_markdown: &str,
) -> String {
    let mut lines = vec![
        "Planning Context".to_string(),
        "".to_string(),
        "Direction Summary".to_string(),
    ];

    for direction in &directions.directions {
        lines.push(format!(
            "- {} | {} | state={}",
            direction.id.trim(),
            direction.title.trim(),
            direction_state_label(direction.state),
        ));
        lines.push(format!("  summary: {}", direction.summary.trim()));
        lines.push(format!(
            "  success_criteria: {}",
            direction.success_criteria.join(" | ")
        ));
        if !direction.scope_hints.is_empty() {
            lines.push(format!(
                "  scope_hints: {}",
                direction.scope_hints.join(" | ")
            ));
        }
    }

    lines.push(String::new());
    lines.push("Queue Summary".to_string());
    match queue_snapshot.next_task.as_ref() {
        Some(task) => {
            lines.push(format!(
                "- next_task: rank {} | {} | {} | direction={} | status={} | combined_priority={}",
                task.rank,
                task.task_id.trim(),
                task.task_title.trim(),
                task.direction_id.trim(),
                task.status.label(),
                task.combined_priority,
            ));
            lines.push(format!("  rank_reasons: {}", task.rank_reasons.join(" | ")));
        }
        None => lines.push("- next_task: none".to_string()),
    }

    if queue_snapshot.active_tasks.is_empty() {
        lines.push("- visible_tasks: none".to_string());
    } else {
        let visible_tasks = queue_snapshot.visible_tasks(MAX_VISIBLE_QUEUE_TASKS);
        lines.push(format!(
            "- visible_tasks: top {} of {}",
            visible_tasks.len(),
            queue_snapshot.active_tasks.len()
        ));
        for task in visible_tasks {
            lines.push(format!(
                "  - rank {} | {} | {} | direction={} | status={} | combined_priority={}",
                task.rank,
                task.task_id.trim(),
                task.task_title.trim(),
                task.direction_id.trim(),
                task.status.label(),
                task.combined_priority,
            ));
            lines.push(format!(
                "    rank_reasons: {}",
                task.rank_reasons.join(" | ")
            ));
        }
    }

    if !queue_snapshot.skipped_tasks.is_empty() {
        lines.push(format!(
            "- skipped_tasks: showing {} of {}",
            queue_snapshot
                .skipped_tasks
                .iter()
                .take(MAX_SKIPPED_QUEUE_TASKS)
                .count(),
            queue_snapshot.skipped_tasks.len()
        ));
        for skipped_task in queue_snapshot
            .skipped_tasks
            .iter()
            .take(MAX_SKIPPED_QUEUE_TASKS)
        {
            lines.push(format!(
                "  - {} | direction={} | status={} | reason={}",
                skipped_task.task_id.trim(),
                skipped_task.direction_id.trim(),
                skipped_task.status.label(),
                skipped_task.reason.trim(),
            ));
        }
    }

    lines.push(String::new());
    lines.push("Task Ledger Mutation Contract".to_string());
    lines.push(format!("- You may edit only `{}`.", TASK_LEDGER_FILE_PATH));
    lines.push(format!(
        "- Do not edit `{}`, `{}`, `{}`, or `{}`.",
        DIRECTIONS_FILE_PATH,
        TASK_LEDGER_SCHEMA_FILE_PATH,
        QUEUE_SNAPSHOT_FILE_PATH,
        RESULT_OUTPUT_FILE_PATH,
    ));
    lines.push(
        "- New tasks must attach to an existing `direction_id` and include `direction_relation_note`."
            .to_string(),
    );
    lines.push(
        "- Do not write unrelated tasks that cannot be connected to the existing directions."
            .to_string(),
    );
    lines.push(
        "- Keep `task-ledger.json` valid JSON that satisfies the checked-in schema.".to_string(),
    );

    lines.push(String::new());
    lines.push("Result Output Prompt".to_string());
    if !result_output_markdown.is_empty() {
        lines.push(result_output_markdown.to_string());
    }

    lines.join("\n")
}

fn build_queue_summary(queue_snapshot: &PriorityQueueSnapshot) -> String {
    match queue_snapshot.next_task.as_ref() {
        Some(task) => format!(
            "next task: rank {} / {} / {} / priority {}",
            task.rank,
            task.task_id.trim(),
            task.task_title.trim(),
            task.combined_priority,
        ),
        None => "queue idle: no executable planning task".to_string(),
    }
}

fn direction_state_label(state: DirectionState) -> &'static str {
    match state {
        DirectionState::Active => "active",
        DirectionState::Paused => "paused",
        DirectionState::Done => "done",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;

    use super::{PlanningPromptContextAvailability, PlanningPromptService};
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning_bootstrap_service::PlanningBootstrapService;
    use crate::application::service::planning_validation_service::PlanningValidationService;
    use crate::application::service::priority_queue_service::PriorityQueueService;

    #[derive(Default)]
    struct FakePlanningWorkspacePort {
        load_record: PlanningWorkspaceLoadRecord,
    }

    impl PlanningWorkspacePort for FakePlanningWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _files: &[crate::application::port::outbound::planning_workspace_port::PlanningDraftFileRecord],
        ) -> Result<
            crate::application::port::outbound::planning_workspace_port::PlanningDraftStageRecord,
        > {
            unreachable!("staging is not used in planning prompt service tests")
        }

        fn load_planning_workspace_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Ok(self.load_record.clone())
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
            _body: Option<&str>,
        ) -> Result<()> {
            unreachable!("file replacement is not used in planning prompt service tests")
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            _archive_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            unreachable!("archive writes are not used in planning prompt service tests")
        }
    }

    fn sample_service(load_record: PlanningWorkspaceLoadRecord) -> PlanningPromptService {
        PlanningPromptService::new(
            Arc::new(FakePlanningWorkspacePort { load_record }),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        )
    }

    #[test]
    fn missing_all_planning_files_keeps_prompt_context_uninitialized() {
        let result = sample_service(PlanningWorkspaceLoadRecord::default())
            .load_prompt_context("/tmp/workspace")
            .expect("planning prompt context should load");

        assert_eq!(
            result.availability,
            PlanningPromptContextAvailability::Uninitialized
        );
        assert!(!result.blocks_auto_followup());
    }

    #[test]
    fn partial_planning_workspace_blocks_auto_followup() {
        let bootstrap_artifacts = PlanningBootstrapService::new().build_artifacts();
        let result = sample_service(PlanningWorkspaceLoadRecord {
            directions_toml: Some(bootstrap_artifacts.directions_toml),
            task_ledger_json: Some(bootstrap_artifacts.task_ledger_json),
            task_ledger_schema_json: None,
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown),
        })
        .load_prompt_context("/tmp/workspace")
        .expect("planning prompt context should load");

        let PlanningPromptContextAvailability::Blocked { ref reason } = result.availability else {
            panic!("partial workspace should block auto follow-up");
        };
        assert!(reason.contains("task-ledger.schema.json"));
        assert!(result.blocks_auto_followup());
    }

    #[test]
    fn valid_planning_workspace_builds_prompt_fragment_with_queue_context() {
        let bootstrap_artifacts = PlanningBootstrapService::new().build_artifacts();
        let result = sample_service(PlanningWorkspaceLoadRecord {
            directions_toml: Some(bootstrap_artifacts.directions_toml),
            task_ledger_json: Some(
                r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "example-direction",
      "title": "Implement the next slice",
      "description": "Move the planning work forward.",
      "status": "ready",
      "base_priority": 8,
      "dynamic_priority_delta": 2,
      "priority_reason": "user requested this next",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T09:00:00Z"
    }
  ]
}"#
                .to_string(),
            ),
            task_ledger_schema_json: Some(bootstrap_artifacts.task_ledger_schema_json),
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown),
        })
        .load_prompt_context("/tmp/workspace")
        .expect("planning prompt context should load");

        let PlanningPromptContextAvailability::Ready(prompt_context) = result.availability else {
            panic!("valid workspace should produce a ready planning prompt context");
        };
        assert!(prompt_context.prompt_fragment.contains("Planning Context"));
        assert!(prompt_context.prompt_fragment.contains("Direction Summary"));
        assert!(prompt_context.prompt_fragment.contains("Queue Summary"));
        assert!(prompt_context.prompt_fragment.contains("task-1"));
        assert!(
            prompt_context
                .prompt_fragment
                .contains("Result Output Prompt")
        );
        assert!(prompt_context.summary.contains("task-1"));
    }

    #[test]
    fn invalid_planning_workspace_blocks_auto_followup_with_validation_reason() {
        let bootstrap_artifacts = PlanningBootstrapService::new().build_artifacts();
        let result = sample_service(PlanningWorkspaceLoadRecord {
            directions_toml: Some(bootstrap_artifacts.directions_toml),
            task_ledger_json: Some(
                r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1"
    }
  ]
}"#
                .to_string(),
            ),
            task_ledger_schema_json: Some(bootstrap_artifacts.task_ledger_schema_json),
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown),
        })
        .load_prompt_context("/tmp/workspace")
        .expect("planning prompt context should load");

        let PlanningPromptContextAvailability::Blocked { ref reason } = result.availability else {
            panic!("invalid planning workspace should block auto follow-up");
        };
        assert!(reason.contains("planning validation failed"));
    }
}

use std::collections::{HashMap, HashSet};

use chrono::DateTime;
use serde_json::Value;

use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningFileKind, PlanningValidationReport,
    PlanningValidationResult, PlanningWorkspaceFiles, TaskActor, TaskLedgerDocument, TaskStatus,
};

#[derive(Default, Clone)]
pub struct PlanningValidationService;

impl PlanningValidationService {
    pub fn new() -> Self {
        Self
    }

    pub fn validate_workspace_files(
        &self,
        files: PlanningWorkspaceFiles<'_>,
    ) -> PlanningValidationResult {
        let mut report = PlanningValidationReport::new();
        let directions = self.parse_direction_catalog(files.directions_toml, &mut report);
        let task_ledger = self.parse_task_ledger(files.task_ledger_json, &mut report);
        self.validate_task_ledger_schema(files.task_ledger_schema_json, &mut report);
        self.validate_result_output_markdown(files.result_output_markdown, &mut report);

        if let Some(direction_catalog) = directions.as_ref() {
            self.validate_direction_catalog(direction_catalog, &mut report);
        }
        if let Some(task_ledger_document) = task_ledger.as_ref() {
            self.validate_task_ledger(task_ledger_document, &mut report);
        }
        if let (Some(direction_catalog), Some(task_ledger_document)) =
            (directions.as_ref(), task_ledger.as_ref())
        {
            self.validate_cross_references(direction_catalog, task_ledger_document, &mut report);
        }

        PlanningValidationResult {
            directions,
            task_ledger,
            report,
        }
    }

    fn parse_direction_catalog(
        &self,
        directions_toml: &str,
        report: &mut PlanningValidationReport,
    ) -> Option<DirectionCatalogDocument> {
        match toml::from_str(directions_toml) {
            Ok(document) => Some(document),
            Err(error) => {
                report.push_error(
                    PlanningFileKind::Directions,
                    "directions_parse_failed",
                    format!("failed to parse directions.toml: {error}"),
                );
                None
            }
        }
    }

    fn parse_task_ledger(
        &self,
        task_ledger_json: &str,
        report: &mut PlanningValidationReport,
    ) -> Option<TaskLedgerDocument> {
        match serde_json::from_str(task_ledger_json) {
            Ok(document) => Some(document),
            Err(error) => {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "task_ledger_parse_failed",
                    format!("failed to parse task-ledger.json: {error}"),
                );
                None
            }
        }
    }

    fn validate_direction_catalog(
        &self,
        direction_catalog: &DirectionCatalogDocument,
        report: &mut PlanningValidationReport,
    ) {
        if direction_catalog.version != PLANNING_FORMAT_VERSION {
            report.push_error(
                PlanningFileKind::Directions,
                "unsupported_directions_version",
                format!(
                    "directions.toml version {} does not match supported version {}",
                    direction_catalog.version, PLANNING_FORMAT_VERSION
                ),
            );
        }

        if direction_catalog.directions.is_empty() {
            report.push_error(
                PlanningFileKind::Directions,
                "missing_directions",
                "directions.toml must contain at least one direction",
            );
            return;
        }

        let mut seen_ids = HashSet::new();
        for direction in &direction_catalog.directions {
            let direction_id = direction.id.trim();
            if direction_id.is_empty() {
                report.push_error(
                    PlanningFileKind::Directions,
                    "blank_direction_id",
                    "direction ids must not be blank",
                );
            } else if !seen_ids.insert(direction_id.to_string()) {
                report.push_error(
                    PlanningFileKind::Directions,
                    "duplicate_direction_id",
                    format!("duplicate direction id: {direction_id}"),
                );
            }

            if direction.title.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::Directions,
                    "blank_direction_title",
                    format!("direction {direction_id} must have a non-empty title"),
                );
            }
            if direction.summary.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::Directions,
                    "blank_direction_summary",
                    format!("direction {direction_id} must have a non-empty summary"),
                );
            }
            if direction.success_criteria.is_empty()
                || direction
                    .success_criteria
                    .iter()
                    .any(|criterion| criterion.trim().is_empty())
            {
                report.push_error(
                    PlanningFileKind::Directions,
                    "invalid_success_criteria",
                    format!(
                        "direction {direction_id} must include at least one non-empty success criterion"
                    ),
                );
            }
        }
    }

    fn validate_task_ledger(
        &self,
        task_ledger: &TaskLedgerDocument,
        report: &mut PlanningValidationReport,
    ) {
        if task_ledger.version != PLANNING_FORMAT_VERSION {
            report.push_error(
                PlanningFileKind::TaskLedger,
                "unsupported_task_ledger_version",
                format!(
                    "task-ledger.json version {} does not match supported version {}",
                    task_ledger.version, PLANNING_FORMAT_VERSION
                ),
            );
        }

        let mut seen_ids = HashSet::new();
        for task in &task_ledger.tasks {
            let task_id = task.id.trim();
            if task_id.is_empty() {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "blank_task_id",
                    "task ids must not be blank",
                );
            } else if !seen_ids.insert(task_id.to_string()) {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "duplicate_task_id",
                    format!("duplicate task id: {task_id}"),
                );
            }

            if task.direction_id.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "blank_direction_reference",
                    format!("task {task_id} must reference a direction_id"),
                );
            }
            if task.title.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "blank_task_title",
                    format!("task {task_id} must have a non-empty title"),
                );
            }
            if task.description.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "blank_task_description",
                    format!("task {task_id} must have a non-empty description"),
                );
            }
            if task.requires_relation_note() && task.direction_relation_note.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "missing_direction_relation_note",
                    format!("LLM-authored task {task_id} must include direction_relation_note"),
                );
            }
            if task.dynamic_priority_delta != 0 && task.priority_reason.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "missing_priority_reason",
                    format!(
                        "task {task_id} must include priority_reason when dynamic_priority_delta is non-zero"
                    ),
                );
            }
            if DateTime::parse_from_rfc3339(task.updated_at.as_str()).is_err() {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "invalid_updated_at",
                    format!("task {task_id} must use RFC3339 updated_at"),
                );
            }

            self.validate_task_links(task, report);
        }
    }

    fn validate_task_links(
        &self,
        task: &crate::domain::planning::TaskDefinition,
        report: &mut PlanningValidationReport,
    ) {
        let task_id = task.id.trim();
        let mut dependency_ids = HashSet::new();
        for dependency_id in &task.depends_on {
            let normalized_dependency_id = dependency_id.trim();
            if normalized_dependency_id.is_empty() {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "blank_dependency_id",
                    format!("task {task_id} contains a blank depends_on entry"),
                );
                continue;
            }
            if normalized_dependency_id == task_id {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "self_dependency",
                    format!("task {task_id} cannot depend on itself"),
                );
            }
            if !dependency_ids.insert(normalized_dependency_id.to_string()) {
                report.push_warning(
                    PlanningFileKind::TaskLedger,
                    "duplicate_dependency_id",
                    format!("task {task_id} repeats dependency id {normalized_dependency_id}"),
                );
            }
        }

        let mut blocker_ids = HashSet::new();
        for blocker_id in &task.blocked_by {
            let normalized_blocker_id = blocker_id.trim();
            if normalized_blocker_id.is_empty() {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "blank_blocker_id",
                    format!("task {task_id} contains a blank blocked_by entry"),
                );
                continue;
            }
            if !blocker_ids.insert(normalized_blocker_id.to_string()) {
                report.push_warning(
                    PlanningFileKind::TaskLedger,
                    "duplicate_blocker_id",
                    format!("task {task_id} repeats blocker id {normalized_blocker_id}"),
                );
            }
        }

        if matches!(task.status, TaskStatus::Proposed) && task.created_by == TaskActor::Llm {
            report.push_warning(
                PlanningFileKind::TaskLedger,
                "llm_proposed_task",
                format!(
                    "task {task_id} is proposed by the LLM and will stay out of normal execution until promoted"
                ),
            );
        }
    }

    fn validate_cross_references(
        &self,
        direction_catalog: &DirectionCatalogDocument,
        task_ledger: &TaskLedgerDocument,
        report: &mut PlanningValidationReport,
    ) {
        let direction_ids = direction_catalog
            .directions
            .iter()
            .map(|direction| direction.id.trim().to_string())
            .collect::<HashSet<_>>();
        let task_map = task_ledger
            .tasks
            .iter()
            .map(|task| (task.id.trim().to_string(), task))
            .collect::<HashMap<_, _>>();

        for task in &task_ledger.tasks {
            let task_id = task.id.trim();
            if !direction_ids.contains(task.direction_id.trim()) {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "missing_direction_reference",
                    format!(
                        "task {task_id} references unknown direction_id {}",
                        task.direction_id.trim()
                    ),
                );
            }
            for dependency_id in &task.depends_on {
                let normalized_dependency_id = dependency_id.trim();
                if !task_map.contains_key(normalized_dependency_id) {
                    report.push_error(
                        PlanningFileKind::TaskLedger,
                        "missing_dependency_reference",
                        format!(
                            "task {task_id} references unknown dependency {normalized_dependency_id}"
                        ),
                    );
                }
            }
            for blocker_id in &task.blocked_by {
                let normalized_blocker_id = blocker_id.trim();
                if !task_map.contains_key(normalized_blocker_id) {
                    report.push_error(
                        PlanningFileKind::TaskLedger,
                        "missing_blocker_reference",
                        format!(
                            "task {task_id} references unknown blocker {normalized_blocker_id}"
                        ),
                    );
                }
            }
        }

        if self.contains_dependency_cycle(task_ledger) {
            report.push_error(
                PlanningFileKind::TaskLedger,
                "dependency_cycle_detected",
                "task-ledger.json contains a dependency cycle",
            );
        }
    }

    fn contains_dependency_cycle(&self, task_ledger: &TaskLedgerDocument) -> bool {
        let adjacency_map = task_ledger
            .tasks
            .iter()
            .map(|task| {
                (
                    task.id.trim().to_string(),
                    task.depends_on
                        .iter()
                        .map(|dependency_id| dependency_id.trim().to_string())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<HashMap<_, _>>();

        let mut temporary_marks = HashSet::new();
        let mut permanent_marks = HashSet::new();

        adjacency_map.keys().any(|task_id| {
            self.detect_cycle(
                task_id,
                &adjacency_map,
                &mut temporary_marks,
                &mut permanent_marks,
            )
        })
    }

    fn detect_cycle(
        &self,
        task_id: &str,
        adjacency_map: &HashMap<String, Vec<String>>,
        temporary_marks: &mut HashSet<String>,
        permanent_marks: &mut HashSet<String>,
    ) -> bool {
        if permanent_marks.contains(task_id) {
            return false;
        }
        if !temporary_marks.insert(task_id.to_string()) {
            return true;
        }

        if let Some(dependencies) = adjacency_map.get(task_id) {
            for dependency_id in dependencies {
                if adjacency_map.contains_key(dependency_id)
                    && self.detect_cycle(
                        dependency_id,
                        adjacency_map,
                        temporary_marks,
                        permanent_marks,
                    )
                {
                    return true;
                }
            }
        }

        temporary_marks.remove(task_id);
        permanent_marks.insert(task_id.to_string());
        false
    }

    fn validate_task_ledger_schema(
        &self,
        task_ledger_schema_json: &str,
        report: &mut PlanningValidationReport,
    ) {
        if task_ledger_schema_json.trim().is_empty() {
            report.push_error(
                PlanningFileKind::TaskLedgerSchema,
                "blank_task_ledger_schema",
                "task-ledger.schema.json must not be blank",
            );
            return;
        }

        let parsed_schema = match serde_json::from_str::<Value>(task_ledger_schema_json) {
            Ok(value) => value,
            Err(error) => {
                report.push_error(
                    PlanningFileKind::TaskLedgerSchema,
                    "task_ledger_schema_parse_failed",
                    format!("failed to parse task-ledger.schema.json: {error}"),
                );
                return;
            }
        };

        if !parsed_schema.is_object() {
            report.push_error(
                PlanningFileKind::TaskLedgerSchema,
                "task_ledger_schema_not_object",
                "task-ledger.schema.json must be a JSON object",
            );
        }
        if parsed_schema
            .get("$schema")
            .and_then(Value::as_str)
            .is_none()
        {
            report.push_warning(
                PlanningFileKind::TaskLedgerSchema,
                "missing_schema_declaration",
                "task-ledger.schema.json should declare a $schema URI",
            );
        }
        if parsed_schema
            .get("properties")
            .and_then(Value::as_object)
            .is_none()
        {
            report.push_error(
                PlanningFileKind::TaskLedgerSchema,
                "missing_schema_properties",
                "task-ledger.schema.json must define top-level properties",
            );
        }
    }

    fn validate_result_output_markdown(
        &self,
        result_output_markdown: &str,
        report: &mut PlanningValidationReport,
    ) {
        if result_output_markdown.trim().is_empty() {
            report.push_error(
                PlanningFileKind::ResultOutput,
                "blank_result_output",
                "result-output.md must not be blank",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PlanningValidationService;
    use crate::application::service::planning_bootstrap_service::PlanningBootstrapService;
    use crate::domain::planning::{PlanningFileKind, PlanningWorkspaceFiles};

    fn bootstrap_files<'a>(
        artifacts: &'a crate::application::service::planning_bootstrap_service::PlanningBootstrapArtifacts,
    ) -> PlanningWorkspaceFiles<'a> {
        PlanningWorkspaceFiles {
            directions_toml: &artifacts.directions_toml,
            task_ledger_json: &artifacts.task_ledger_json,
            task_ledger_schema_json: &artifacts.task_ledger_schema_json,
            result_output_markdown: &artifacts.result_output_markdown,
        }
    }

    #[test]
    fn bootstrap_artifacts_validate_successfully() {
        let bootstrap_service = PlanningBootstrapService::new();
        let validation_service = PlanningValidationService::new();
        let artifacts = bootstrap_service.build_artifacts();

        let result = validation_service.validate_workspace_files(bootstrap_files(&artifacts));

        assert!(result.is_valid(), "{:?}", result.report.issues);
        assert!(result.directions.is_some());
        assert!(result.task_ledger.is_some());
    }

    #[test]
    fn rejects_unknown_direction_references() {
        let validation_service = PlanningValidationService::new();
        let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
            directions_toml: r#"version = 1

[[directions]]
id = "product-direction"
title = "Product direction"
summary = "Ship planning support."
success_criteria = ["Complete the planning slice."]
state = "active"
"#,
            task_ledger_json: r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "missing-direction",
      "direction_relation_note": "Loose relation",
      "title": "Draft follow-up work",
      "description": "Write one next task.",
      "status": "ready",
      "base_priority": 10,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:00:00Z"
    }
  ]
}"#,
            task_ledger_schema_json: r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"version":{"type":"integer"},"tasks":{"type":"array"}}}"#,
            result_output_markdown: "Summarize the result.",
        });

        assert!(!result.is_valid());
        assert!(result.report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::TaskLedger
                && issue.code == "missing_direction_reference"
        }));
    }

    #[test]
    fn rejects_llm_tasks_without_relation_notes() {
        let validation_service = PlanningValidationService::new();
        let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
            directions_toml: r#"version = 1

[[directions]]
id = "direction-a"
title = "Direction A"
summary = "Keep task updates aligned."
success_criteria = ["Only aligned tasks enter the ledger."]
state = "active"
"#,
            task_ledger_json: r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "direction-a",
      "direction_relation_note": "",
      "title": "Add a follow-up",
      "description": "LLM adds a new task.",
      "status": "proposed",
      "base_priority": 5,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": "turn-1",
      "updated_at": "2026-04-09T10:00:00Z"
    }
  ]
}"#,
            task_ledger_schema_json: r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"version":{"type":"integer"},"tasks":{"type":"array"}}}"#,
            result_output_markdown: "Summarize the result.",
        });

        assert!(!result.is_valid());
        assert!(result.report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::TaskLedger
                && issue.code == "missing_direction_relation_note"
        }));
    }

    #[test]
    fn rejects_dependency_cycles() {
        let validation_service = PlanningValidationService::new();
        let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
            directions_toml: r#"version = 1

[[directions]]
id = "direction-a"
title = "Direction A"
summary = "Keep task updates aligned."
success_criteria = ["Only aligned tasks enter the ledger."]
state = "active"
"#,
            task_ledger_json: r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "direction-a",
      "direction_relation_note": "Still under direction A",
      "title": "Task 1",
      "description": "First task.",
      "status": "ready",
      "base_priority": 10,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": ["task-2"],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:00:00Z"
    },
    {
      "id": "task-2",
      "direction_id": "direction-a",
      "direction_relation_note": "Still under direction A",
      "title": "Task 2",
      "description": "Second task.",
      "status": "ready",
      "base_priority": 9,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": ["task-1"],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:01:00Z"
    }
  ]
}"#,
            task_ledger_schema_json: r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"version":{"type":"integer"},"tasks":{"type":"array"}}}"#,
            result_output_markdown: "Summarize the result.",
        });

        assert!(!result.is_valid());
        assert!(result.report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::TaskLedger
                && issue.code == "dependency_cycle_detected"
        }));
    }

    #[test]
    fn rejects_blank_result_output_prompt() {
        let bootstrap_service = PlanningBootstrapService::new();
        let validation_service = PlanningValidationService::new();
        let artifacts = bootstrap_service.build_artifacts();
        let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
            directions_toml: &artifacts.directions_toml,
            task_ledger_json: &artifacts.task_ledger_json,
            task_ledger_schema_json: &artifacts.task_ledger_schema_json,
            result_output_markdown: "   ",
        });

        assert!(!result.is_valid());
        assert!(result.report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::ResultOutput && issue.code == "blank_result_output"
        }));
    }
}

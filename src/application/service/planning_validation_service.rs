use std::collections::{HashMap, HashSet};

use chrono::DateTime;
use jsonschema::Validator;
use serde_json::Value;

use crate::application::service::planning_contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY,
};
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningFileKind, PlanningValidationReport,
    PlanningValidationResult, PlanningWorkspaceFiles, QueueIdlePolicy, TaskActor,
    TaskLedgerDocument, TaskStatus,
};

#[derive(Default, Clone)]
pub struct PlanningValidationService;

const PLACEHOLDER_MARKERS: &[&str] = &[
    "{{", "}}", "todo", "tbd", "<replace", "[replace", "<fill", "[fill",
];

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
        let task_ledger_value = self.parse_task_ledger_value(files.task_ledger_json, &mut report);
        let task_ledger_schema =
            self.validate_task_ledger_schema(files.task_ledger_schema_json, &mut report);
        if let (Some(task_ledger_value), Some(task_ledger_schema)) =
            (task_ledger_value.as_ref(), task_ledger_schema.as_ref())
        {
            self.validate_task_ledger_against_schema(
                task_ledger_value,
                task_ledger_schema,
                &mut report,
            );
        }
        let task_ledger = task_ledger_value
            .and_then(|task_ledger_value| self.parse_task_ledger(task_ledger_value, &mut report));
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

    fn parse_task_ledger_value(
        &self,
        task_ledger_json: &str,
        report: &mut PlanningValidationReport,
    ) -> Option<Value> {
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

    fn parse_task_ledger(
        &self,
        task_ledger_value: Value,
        report: &mut PlanningValidationReport,
    ) -> Option<TaskLedgerDocument> {
        match serde_json::from_value(task_ledger_value) {
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

    pub fn validate_direction_supporting_files<F>(
        &self,
        direction_catalog: &DirectionCatalogDocument,
        mut has_file: F,
        report: &mut PlanningValidationReport,
    ) where
        F: FnMut(&str) -> bool,
    {
        for direction in &direction_catalog.directions {
            let direction_id = direction.id.trim();
            let detail_doc_path = direction.detail_doc_path.trim();
            if detail_doc_path.is_empty() {
                continue;
            }

            if !is_valid_planning_markdown_path(detail_doc_path, PLANNING_DIRECTION_DOCS_DIRECTORY)
            {
                report.push_error(
                    PlanningFileKind::Directions,
                    "invalid_detail_doc_path",
                    format!(
                        "direction {direction_id} detail_doc_path must point to a markdown file under {PLANNING_DIRECTION_DOCS_DIRECTORY}"
                    ),
                );
                continue;
            }

            if !has_file(detail_doc_path) {
                report.push_error(
                    PlanningFileKind::Directions,
                    "missing_detail_doc_file",
                    format!(
                        "direction {direction_id} detail_doc_path does not exist: {detail_doc_path}"
                    ),
                );
            }
        }

        let prompt_path = direction_catalog.queue_idle.prompt_path.trim();
        if direction_catalog.queue_idle.policy == QueueIdlePolicy::ReviewAndEnqueue
            && prompt_path.is_empty()
        {
            report.push_error(
                PlanningFileKind::Directions,
                "missing_queue_idle_prompt_path",
                format!(
                    "queue_idle.policy=review_and_enqueue requires queue_idle.prompt_path; default path: {DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH}"
                ),
            );
            return;
        }

        if prompt_path.is_empty() {
            return;
        }

        if !is_valid_planning_markdown_path(prompt_path, PLANNING_PROMPTS_DIRECTORY) {
            report.push_error(
                PlanningFileKind::Directions,
                "invalid_queue_idle_prompt_path",
                format!(
                    "queue_idle.prompt_path must point to a markdown file under {PLANNING_PROMPTS_DIRECTORY}"
                ),
            );
            return;
        }

        if !has_file(prompt_path) {
            report.push_error(
                PlanningFileKind::Directions,
                "missing_queue_idle_prompt_file",
                format!("queue_idle.prompt_path does not exist: {prompt_path}"),
            );
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
            if normalized_blocker_id == task_id {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "self_blocker",
                    format!("task {task_id} cannot block itself"),
                );
            }
        }

        let mut conflict_ids = HashSet::new();
        for dependency_id in &task.depends_on {
            let normalized_dependency_id = dependency_id.trim();
            if normalized_dependency_id.is_empty() {
                continue;
            }
            if blocker_ids.contains(normalized_dependency_id)
                && conflict_ids.insert(normalized_dependency_id.to_string())
            {
                report.push_error(
                    PlanningFileKind::TaskLedger,
                    "dependency_blocker_conflict",
                    format!(
                        "task {task_id} cannot list {normalized_dependency_id} in both depends_on and blocked_by"
                    ),
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

        self.validate_task_semantics(task_ledger, &task_map, report);

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

    fn validate_task_semantics(
        &self,
        task_ledger: &TaskLedgerDocument,
        task_map: &HashMap<String, &crate::domain::planning::TaskDefinition>,
        report: &mut PlanningValidationReport,
    ) {
        let mut in_progress_task_ids = Vec::new();

        for task in &task_ledger.tasks {
            let task_id = task.id.trim();
            if task.status == TaskStatus::InProgress {
                in_progress_task_ids.push(task_id);
            }
            if task.status != TaskStatus::Done {
                continue;
            }

            for dependency_id in &task.depends_on {
                let normalized_dependency_id = dependency_id.trim();
                if let Some(dependency) = task_map.get(normalized_dependency_id)
                    && !dependency.status.is_dependency_complete()
                {
                    report.push_error(
                        PlanningFileKind::TaskLedger,
                        "done_task_unresolved_dependency",
                        format!(
                            "done task {task_id} cannot depend on incomplete task {normalized_dependency_id} ({})",
                            dependency.status.label()
                        ),
                    );
                }
            }

            for blocker_id in &task.blocked_by {
                let normalized_blocker_id = blocker_id.trim();
                if let Some(blocker) = task_map.get(normalized_blocker_id)
                    && !blocker.status.clears_blocker()
                {
                    report.push_error(
                        PlanningFileKind::TaskLedger,
                        "done_task_unresolved_blocker",
                        format!(
                            "done task {task_id} cannot remain blocked by task {normalized_blocker_id} ({})",
                            blocker.status.label()
                        ),
                    );
                }
            }
        }

        if in_progress_task_ids.len() > 1 {
            report.push_error(
                PlanningFileKind::TaskLedger,
                "multiple_in_progress_tasks",
                format!(
                    "task-ledger.json may contain at most one in_progress task; found {}: {}",
                    in_progress_task_ids.len(),
                    in_progress_task_ids.join(", ")
                ),
            );
        }
    }

    fn validate_task_ledger_schema(
        &self,
        task_ledger_schema_json: &str,
        report: &mut PlanningValidationReport,
    ) -> Option<Validator> {
        if task_ledger_schema_json.trim().is_empty() {
            report.push_error(
                PlanningFileKind::TaskLedgerSchema,
                "blank_task_ledger_schema",
                "task-ledger.schema.json must not be blank",
            );
            return None;
        }

        let parsed_schema = match serde_json::from_str::<Value>(task_ledger_schema_json) {
            Ok(value) => value,
            Err(error) => {
                report.push_error(
                    PlanningFileKind::TaskLedgerSchema,
                    "task_ledger_schema_parse_failed",
                    format!("failed to parse task-ledger.schema.json: {error}"),
                );
                return None;
            }
        };

        let mut schema_is_usable = true;
        if !parsed_schema.is_object() {
            report.push_error(
                PlanningFileKind::TaskLedgerSchema,
                "task_ledger_schema_not_object",
                "task-ledger.schema.json must be a JSON object",
            );
            schema_is_usable = false;
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
            schema_is_usable = false;
        }

        if !schema_is_usable {
            return None;
        }

        match jsonschema::validator_for(&parsed_schema) {
            Ok(validator) => Some(validator),
            Err(error) => {
                report.push_error(
                    PlanningFileKind::TaskLedgerSchema,
                    "invalid_task_ledger_schema",
                    format!("task-ledger.schema.json is not a valid JSON schema: {error}"),
                );
                None
            }
        }
    }

    fn validate_task_ledger_against_schema(
        &self,
        task_ledger_value: &Value,
        validator: &Validator,
        report: &mut PlanningValidationReport,
    ) {
        for error in validator.iter_errors(task_ledger_value) {
            let instance_path = display_json_location(error.instance_path().to_string());
            report.push_error(
                PlanningFileKind::TaskLedger,
                "task_ledger_schema_violation",
                format!("task-ledger.json failed schema validation at {instance_path}: {error}"),
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
            return;
        }

        let non_empty_lines = result_output_markdown
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some((index + 1, trimmed))
                }
            })
            .collect::<Vec<_>>();

        let Some((_, first_line)) = non_empty_lines.first() else {
            return;
        };
        if !first_line.starts_with('#') {
            report.push_error(
                PlanningFileKind::ResultOutput,
                "missing_result_output_heading",
                "result-output.md must start with a markdown heading",
            );
        }

        if non_empty_lines
            .iter()
            .skip(1)
            .all(|(_, line)| line.starts_with('#'))
        {
            report.push_error(
                PlanningFileKind::ResultOutput,
                "missing_result_output_instructions",
                "result-output.md must include at least one instruction line after the heading",
            );
        }

        for (line_number, line) in non_empty_lines {
            if let Some(marker) = placeholder_marker(line) {
                report.push_warning(
                    PlanningFileKind::ResultOutput,
                    "result_output_contains_placeholder",
                    format!(
                        "result-output.md contains unresolved placeholder marker {marker:?} on line {line_number}"
                    ),
                );
            }
        }
    }
}

fn is_valid_planning_markdown_path(path: &str, required_prefix: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains("../")
        || normalized.contains("/..")
    {
        return false;
    }

    let Some(suffix) = normalized.strip_prefix(required_prefix) else {
        return false;
    };

    suffix.starts_with('/') && suffix.len() > 1 && normalized.ends_with(".md")
}

fn display_json_location(path: String) -> String {
    if path.is_empty() || path == "." {
        "root".to_string()
    } else {
        path
    }
}

fn placeholder_marker(line: &str) -> Option<&'static str> {
    let normalized = line.to_ascii_lowercase();
    PLACEHOLDER_MARKERS
        .iter()
        .copied()
        .find(|marker| normalized.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::PlanningValidationService;
    use crate::application::service::planning_bootstrap_service::{
        PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::domain::planning::{PlanningFileKind, PlanningWorkspaceFiles};

    fn valid_result_output_markdown() -> &'static str {
        r#"# Result Output Prompt

- Summarize the work you actually completed in this turn.
- Mention task-ledger updates when they changed.
"#
    }

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
        let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);

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
            result_output_markdown: valid_result_output_markdown(),
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
            result_output_markdown: valid_result_output_markdown(),
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
            result_output_markdown: valid_result_output_markdown(),
        });

        assert!(!result.is_valid());
        assert!(result.report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::TaskLedger
                && issue.code == "dependency_cycle_detected"
        }));
    }

    #[test]
    fn rejects_task_ledgers_that_fail_json_schema() {
        let bootstrap_service = PlanningBootstrapService::new();
        let validation_service = PlanningValidationService::new();
        let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);

        let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
            directions_toml: &artifacts.directions_toml,
            task_ledger_json: r#"{
  "version": 1
}"#,
            task_ledger_schema_json: &artifacts.task_ledger_schema_json,
            result_output_markdown: valid_result_output_markdown(),
        });

        assert!(!result.is_valid());
        assert!(result.report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::TaskLedger
                && issue.code == "task_ledger_schema_violation"
        }));
    }

    #[test]
    fn rejects_unknown_task_ledger_fields() {
        let bootstrap_service = PlanningBootstrapService::new();
        let validation_service = PlanningValidationService::new();
        let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);

        let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
            directions_toml: &artifacts.directions_toml,
            task_ledger_json: r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "example-direction",
      "direction_relation_note": "",
      "title": "Task 1",
      "description": "Keep schema and serde aligned.",
      "status": "ready",
      "base_priority": 10,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:00:00Z",
      "unexpected_field": true
    }
  ]
}"#,
            task_ledger_schema_json: &artifacts.task_ledger_schema_json,
            result_output_markdown: valid_result_output_markdown(),
        });

        assert!(!result.is_valid());
        assert!(result.report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::TaskLedger
                && issue.code == "task_ledger_parse_failed"
        }));
    }

    #[test]
    fn rejects_conflicting_done_relationships_and_multiple_in_progress_tasks() {
        let bootstrap_service = PlanningBootstrapService::new();
        let validation_service = PlanningValidationService::new();
        let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);

        let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
            directions_toml: &artifacts.directions_toml,
            task_ledger_json: r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "example-direction",
      "direction_relation_note": "",
      "title": "Task 1",
      "description": "Still running.",
      "status": "in_progress",
      "base_priority": 10,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:00:00Z"
    },
    {
      "id": "task-2",
      "direction_id": "example-direction",
      "direction_relation_note": "",
      "title": "Task 2",
      "description": "Also marked active.",
      "status": "in_progress",
      "base_priority": 9,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:01:00Z"
    },
    {
      "id": "task-3",
      "direction_id": "example-direction",
      "direction_relation_note": "",
      "title": "Task 3",
      "description": "Claims to be done too early.",
      "status": "done",
      "base_priority": 8,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": ["task-1"],
      "blocked_by": ["task-1"],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:02:00Z"
    }
  ]
}"#,
            task_ledger_schema_json: &artifacts.task_ledger_schema_json,
            result_output_markdown: valid_result_output_markdown(),
        });

        assert!(!result.is_valid());
        assert!(result.report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::TaskLedger
                && issue.code == "dependency_blocker_conflict"
        }));
        assert!(result.report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::TaskLedger
                && issue.code == "done_task_unresolved_dependency"
        }));
        assert!(result.report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::TaskLedger
                && issue.code == "done_task_unresolved_blocker"
        }));
        assert!(result.report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::TaskLedger
                && issue.code == "multiple_in_progress_tasks"
        }));
    }

    #[test]
    fn rejects_result_output_without_heading() {
        let bootstrap_service = PlanningBootstrapService::new();
        let validation_service = PlanningValidationService::new();
        let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
            directions_toml: &artifacts.directions_toml,
            task_ledger_json: &artifacts.task_ledger_json,
            task_ledger_schema_json: &artifacts.task_ledger_schema_json,
            result_output_markdown: "Summarize the completed work.",
        });

        assert!(!result.is_valid());
        assert!(result.report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::ResultOutput
                && issue.code == "missing_result_output_heading"
        }));
    }

    #[test]
    fn warns_on_result_output_placeholders() {
        let bootstrap_service = PlanningBootstrapService::new();
        let validation_service = PlanningValidationService::new();
        let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
            directions_toml: &artifacts.directions_toml,
            task_ledger_json: &artifacts.task_ledger_json,
            task_ledger_schema_json: &artifacts.task_ledger_schema_json,
            result_output_markdown: r#"# Result Output Prompt

- TODO: replace this guidance before relying on it.
"#,
        });

        assert!(result.is_valid(), "{:?}", result.report.issues);
        assert!(result.report.issues.iter().any(|issue| {
            issue.file_kind == PlanningFileKind::ResultOutput
                && issue.code == "result_output_contains_placeholder"
        }));
    }

    #[test]
    fn rejects_blank_result_output_prompt() {
        let bootstrap_service = PlanningBootstrapService::new();
        let validation_service = PlanningValidationService::new();
        let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);
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

    #[test]
    fn rejects_detail_doc_paths_that_only_match_prefix_textually() {
        let validation_service = PlanningValidationService::new();
        let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
            directions_toml: r#"version = 1

[[directions]]
id = "direction-a"
title = "Direction A"
summary = "Keep details in a scoped markdown file."
success_criteria = ["Use a detail doc path inside the directions directory."]
detail_doc_path = ".codex-exec-loop/planning/directions_backup/direction-a.md"
state = "active"
"#,
            task_ledger_json: r#"{"version":1,"tasks":[]}"#,
            task_ledger_schema_json: r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"version":{"type":"integer"},"tasks":{"type":"array"}}}"#,
            result_output_markdown: valid_result_output_markdown(),
        });

        assert!(result.is_valid(), "{:?}", result.report.issues);
        let mut report = result.report;
        let directions = result
            .directions
            .expect("directions should parse for supporting file validation");
        validation_service.validate_direction_supporting_files(&directions, |_| true, &mut report);
        assert!(report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::Directions
                && issue.code == "invalid_detail_doc_path"
        }));
    }

    #[test]
    fn rejects_queue_idle_prompt_paths_that_only_match_prefix_textually() {
        let validation_service = PlanningValidationService::new();
        let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
            directions_toml: r#"version = 1

[queue_idle]
policy = "review_and_enqueue"
prompt_path = ".codex-exec-loop/planning/prompts_backup/queue-idle-review.md"

[[directions]]
id = "direction-a"
title = "Direction A"
summary = "Keep details in a scoped markdown file."
success_criteria = ["Use a queue-idle prompt inside the prompts directory."]
state = "active"
"#,
            task_ledger_json: r#"{"version":1,"tasks":[]}"#,
            task_ledger_schema_json: r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"version":{"type":"integer"},"tasks":{"type":"array"}}}"#,
            result_output_markdown: valid_result_output_markdown(),
        });

        assert!(result.is_valid(), "{:?}", result.report.issues);
        let mut report = result.report;
        let directions = result
            .directions
            .expect("directions should parse for supporting file validation");
        validation_service.validate_direction_supporting_files(&directions, |_| true, &mut report);
        assert!(report.errors().iter().any(|issue| {
            issue.file_kind == PlanningFileKind::Directions
                && issue.code == "invalid_queue_idle_prompt_path"
        }));
    }
}

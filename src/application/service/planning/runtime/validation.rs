use serde_json::Value;

use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY,
};
use crate::application::service::planning::shared::planning_paths::is_valid_planning_markdown_path;
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningFileKind, PlanningSemanticValidationService,
    PlanningValidationReport, PlanningValidationResult, PlanningWorkspaceFiles, QueueIdlePolicy,
    TaskAuthorityDocument,
};

/*
 * PlanningValidationService is the shared application gate before draft, repair, reset, or promotion
 * output is treated as accepted planning authority.
 * It owns validation that needs workspace contracts: JSON syntax and serde shape for task authority,
 * result-output markdown structure, and supporting-file path rules. The domain semantic validator still
 * owns cross-document meaning, such as task/direction relationships, so every planning entrypoint reports
 * errors through one PlanningValidationReport contract.
 */
#[derive(Default, Clone)]
pub struct PlanningValidationService;

// Template markers in result-output.md are warnings, not hard blockers, because an operator may still be editing copy.
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
        /*
         * Validation collects as many independent issues as possible in one pass.
         * Parse failures are added to the report instead of short-circuiting the whole flow; later phases
         * receive only documents that were successfully parsed, which lets editors and doctor reports show
         * syntax, structure, markdown, and semantic problems together without feeding invalid authority into
         * the domain validator.
         */
        let mut report = PlanningValidationReport::new();
        let directions = Some(files.directions.clone());
        let task_authority_value =
            self.parse_task_authority_value(files.task_authority_json, &mut report);
        let task_authority = task_authority_value.and_then(|task_authority_value| {
            self.parse_task_authority(task_authority_value, &mut report)
        });
        self.validate_result_output_markdown(files.result_output_markdown, &mut report);
        // Semantic validation is intentionally last so it works from parsed domain documents only.
        PlanningSemanticValidationService::new().validate(
            directions.as_ref(),
            task_authority.as_ref(),
            &mut report,
        );

        PlanningValidationResult {
            directions,
            task_authority,
            report,
        }
    }

    fn parse_task_authority_value(
        &self,
        task_authority_json: &str,
        report: &mut PlanningValidationReport,
    ) -> Option<Value> {
        // First parse isolates raw JSON syntax problems before any domain-schema assumptions are applied.
        match serde_json::from_str(task_authority_json) {
            Ok(document) => Some(document),
            Err(error) => {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "task_authority_parse_failed",
                    format!("failed to parse task authority: {error}"),
                );
                None
            }
        }
    }

    fn parse_task_authority(
        &self,
        task_authority_value: Value,
        report: &mut PlanningValidationReport,
    ) -> Option<TaskAuthorityDocument> {
        // Second parse lowers syntactically valid JSON into the versioned task-authority domain contract.
        match serde_json::from_value(task_authority_value) {
            Ok(document) => Some(document),
            Err(error) => {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "task_authority_parse_failed",
                    format!("failed to parse task authority: {error}"),
                );
                None
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
        /*
         * detail_doc_path is the runtime prompt assembly link from a direction to its expanded instructions.
         * This check verifies both the planning-docs sandbox rule and existence so admin authoring cannot
         * promote a direction catalog that points outside the workspace contract or to a missing file.
         */
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

        /*
         * review_and_enqueue makes the queue-idle prompt part of the runtime contract.
         * Without a prompt path, the hidden worker has no operator-approved instruction source for deriving
         * follow-up proposals, so the otherwise optional prompt mapping becomes mandatory.
         */
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

        // Prompt files use their own sandbox so worker prompt assembly cannot read arbitrary workspace files.
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

    fn validate_result_output_markdown(
        &self,
        result_output_markdown: &str,
        report: &mut PlanningValidationReport,
    ) {
        /*
         * result-output.md is the runtime-facing instruction file for completed task summaries.
         * A blank file leaves the worker without an output contract, so this is an error even when
         * directions and task authority parse successfully.
         */
        if result_output_markdown.trim().is_empty() {
            report.push_error(
                PlanningFileKind::ResultOutput,
                "blank_result_output",
                "result-output.md must not be blank",
            );
            return;
        }

        // Preserve line numbers for placeholder warnings while ignoring blank lines for document-shape checks.
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
        // Requiring a heading keeps admin preview and prompt fragments aligned on the same section boundary.
        if !first_line.starts_with('#') {
            report.push_error(
                PlanningFileKind::ResultOutput,
                "missing_result_output_heading",
                "result-output.md must start with a markdown heading",
            );
        }

        // A heading-only file is valid markdown but not a useful worker instruction contract.
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

        // Placeholder markers remain warnings so operators can decide whether example copy is intentional.
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

fn placeholder_marker(line: &str) -> Option<&'static str> {
    // Case-insensitive matching collapses TODO/todo/TBD variants into one warning code.
    let normalized = line.to_ascii_lowercase();
    PLACEHOLDER_MARKERS
        .iter()
        .copied()
        .find(|marker| normalized.contains(marker))
}

#[cfg(test)]
mod tests;

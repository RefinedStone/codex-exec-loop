use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::application::service::prompt_component::PromptDocumentBuilder;

const PLANNING_TASK_COMMANDS_OUTPUT_CONTRACT: &str = "Final answer must include exactly one fenced JSON object: `{\"planning_task_commands\":{\"version\":1,\"commands\":[...]}}`.";
const MAX_WORKER_DIRECTION_AUTHORITY_CHARS: usize = 4_000;
const MAX_WORKER_TASK_AUTHORITY_CHARS: usize = 4_000;
const MAX_WORKER_QUEUE_PROJECTION_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlanningPromptHandoff<'a> {
    pub(crate) task_id: &'a str,
    pub(crate) task_title: &'a str,
    pub(crate) updated_at: &'a str,
    pub(crate) status_label: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanningWorkerAuthorityPromptContext {
    pub(crate) status_lines: Vec<String>,
    pub(crate) direction_authority_json: Option<String>,
    pub(crate) task_authority_json: Option<String>,
    pub(crate) queue_projection_json: Option<String>,
}

pub(crate) fn worker_role_lines() -> Vec<String> {
    vec![
        "session=planning-only".to_string(),
        "protected_files=`result-output.md`, direction detail docs, queue-idle review prompt"
            .to_string(),
        "Use only the accepted DB authority sections as planning authority.".to_string(),
    ]
}

pub(crate) fn add_worker_authority_context_sections(
    builder: PromptDocumentBuilder,
    authority_context: &PlanningWorkerAuthorityPromptContext,
) -> PromptDocumentBuilder {
    builder
        .lines("db-authority", authority_context.status_lines.clone())
        .optional_code_block(
            "accepted-db-direction-authority",
            "json",
            truncate_optional_prompt_section(
                authority_context.direction_authority_json.as_deref(),
                MAX_WORKER_DIRECTION_AUTHORITY_CHARS,
            )
            .as_deref(),
        )
        .optional_code_block(
            "accepted-db-task-authority",
            "json",
            truncate_optional_prompt_section(
                authority_context.task_authority_json.as_deref(),
                MAX_WORKER_TASK_AUTHORITY_CHARS,
            )
            .as_deref(),
        )
        .optional_code_block(
            "db-queue-projection",
            "json",
            truncate_optional_prompt_section(
                authority_context.queue_projection_json.as_deref(),
                MAX_WORKER_QUEUE_PROJECTION_CHARS,
            )
            .as_deref(),
        )
}

pub(crate) fn worker_task_authority_output_contract() -> Vec<String> {
    vec![
        PLANNING_TASK_COMMANDS_OUTPUT_CONTRACT.to_string(),
        "`commands` may contain only `create_task` or `update_task` operations.".to_string(),
        "Do not return `task_authority` or a full task ledger document.".to_string(),
        "Do not include fields controlled by the application: `id`, `created_by`, `last_updated_by`, `updated_at`, or `source_turn_id`."
            .to_string(),
        "Use `status=cancelled` to cancel work; do not emit delete operations.".to_string(),
        "End with a short natural-language summary of the task command changes.".to_string(),
    ]
}

pub(crate) fn repair_task_authority_output_contract() -> Vec<String> {
    vec![
        PLANNING_TASK_COMMANDS_OUTPUT_CONTRACT.to_string(),
        "`commands` must be the smallest create/update set needed to resolve the validation errors."
            .to_string(),
        "Do not return `task_authority` or a full task ledger document.".to_string(),
        "Do not include fields controlled by the application: `id`, `created_by`, `last_updated_by`, `updated_at`, or `source_turn_id`."
            .to_string(),
        "Resolve every validation error listed below.".to_string(),
    ]
}

pub(crate) fn runtime_task_authority_contract_rules() -> Vec<String> {
    vec![
        format!("Do not edit `{}`.", RESULT_OUTPUT_FILE_PATH),
        "New tasks must attach to an existing `direction_id` and include `direction_relation_note`."
            .to_string(),
        "Do not write unrelated tasks that cannot be connected to existing directions."
            .to_string(),
        "Task catalog mutations must go through `planning_task_commands`; queue validation refreshes prompt state."
            .to_string(),
        "Use accepted DB authority as the only planning source of truth.".to_string(),
    ]
}

pub(crate) fn repair_constraints() -> Vec<String> {
    vec![
        "Do not edit planning files in this turn; return corrected planning task commands as JSON only."
            .to_string(),
        format!("Do not edit `{}`.", RESULT_OUTPUT_FILE_PATH),
        "Use the last accepted DB snapshot as the current task authority baseline.".to_string(),
        "Do not add unrelated work outside the existing direction frame.".to_string(),
    ]
}

pub(crate) fn worker_previous_handoff_lines(
    previous_handoff: Option<PlanningPromptHandoff<'_>>,
) -> Vec<String> {
    previous_handoff.map_or_else(Vec::new, |task| {
        let mut lines = handoff_common_lines(task);
        lines.extend([
            "Do not select this task again as unchanged `ready` queue head.".to_string(),
            "If complete, mark `done`; if still active, update the task from latest evidence."
                .to_string(),
            "If follow-up work split out, update the existing task or add a new task.".to_string(),
        ]);
        lines
    })
}

pub(crate) fn repair_previous_handoff_lines(
    previous_handoff: Option<PlanningPromptHandoff<'_>>,
) -> Vec<String> {
    previous_handoff.map_or_else(Vec::new, |previous_handoff| {
        let mut lines = handoff_common_lines(previous_handoff);
        lines.push("If this task stays active, the ledger must show what changed.".to_string());
        lines
    })
}

fn handoff_common_lines(handoff: PlanningPromptHandoff<'_>) -> Vec<String> {
    vec![
        format!("task_id={}", handoff.task_id),
        format!("title={}", handoff.task_title),
        format!("updated_at={}", handoff.updated_at),
        format!("status={}", handoff.status_label),
    ]
}

fn truncate_optional_prompt_section(body: Option<&str>, max_chars: usize) -> Option<String> {
    body.map(|body| truncate_prompt_section(body, max_chars))
        .filter(|body| !body.trim().is_empty())
}

pub(crate) fn truncate_prompt_section(body: &str, max_chars: usize) -> String {
    let body = body.trim();
    if body.chars().count() <= max_chars {
        return body.to_string();
    }

    let truncated = body.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n... [truncated]")
}

#[cfg(test)]
mod tests {
    use super::{
        PlanningPromptHandoff, PlanningWorkerAuthorityPromptContext,
        add_worker_authority_context_sections, repair_constraints, repair_previous_handoff_lines,
        runtime_task_authority_contract_rules, worker_previous_handoff_lines, worker_role_lines,
        worker_task_authority_output_contract,
    };
    use crate::application::service::prompt_component::PromptDocument;

    #[test]
    fn shared_contract_sections_keep_db_authority_source_of_truth() {
        let runtime_rules = runtime_task_authority_contract_rules().join("\n");
        let repair_rules = repair_constraints().join("\n");

        assert!(runtime_rules.contains("accepted DB authority"));
        assert!(repair_rules.contains("last accepted DB snapshot"));
    }

    #[test]
    fn shared_output_contract_uses_required_task_command_payload() {
        let contract = worker_task_authority_output_contract().join("\n");

        assert!(contract.contains("\"planning_task_commands\""));
        assert!(contract.contains("create_task"));
        assert!(contract.contains("Do not return `task_authority`"));
    }

    #[test]
    fn shared_handoff_sections_have_worker_and_repair_variants() {
        let handoff = PlanningPromptHandoff {
            task_id: "task-1",
            task_title: "Task 1",
            updated_at: "2026-04-29T00:00:00Z",
            status_label: "ready",
        };

        let worker_lines = worker_previous_handoff_lines(Some(handoff));
        let repair_lines = repair_previous_handoff_lines(Some(handoff));

        assert!(
            worker_lines
                .iter()
                .any(|line| line.contains("unchanged `ready` queue head"))
        );
        assert!(
            repair_lines
                .iter()
                .any(|line| line.contains("ledger must show what changed"))
        );
    }

    #[test]
    fn worker_role_leaves_source_of_truth_to_db_authority_section() {
        let role = worker_role_lines().join("\n");

        assert!(!role.contains("source_of_truth="));
    }

    #[test]
    fn worker_authority_sections_truncate_large_json_payloads() {
        let authority_context = PlanningWorkerAuthorityPromptContext {
            status_lines: vec!["source_of_truth=accepted DB authority only".to_string()],
            direction_authority_json: Some("x".repeat(4_100)),
            task_authority_json: Some("y".repeat(4_100)),
            queue_projection_json: Some("z".repeat(2_100)),
        };

        let prompt = add_worker_authority_context_sections(
            PromptDocument::builder("task"),
            &authority_context,
        )
        .build()
        .render();

        assert!(prompt.contains("... [truncated]"));
        assert!(!prompt.contains(&"x".repeat(4_100)));
        assert!(!prompt.contains(&"y".repeat(4_100)));
        assert!(!prompt.contains(&"z".repeat(2_100)));
    }
}

use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::application::service::planning::task_tool::planning_task_tool_contract_json;
use crate::application::service::prompt_component::PromptDocumentBuilder;

/*
 * Shared prompt sections keep worker refresh and repair prompts on the same DB-authority contract.
 * The concrete prompt builders decide role, validation, and context order; this module supplies repeated
 * sections for source-of-truth framing, planning-task command output, planning-tool usage, previous-handoff
 * handling, and bounded authority excerpts.
 */

// The final-answer envelope must match the task tool parser, not the older full-authority JSON shape.
const PLANNING_TASK_COMMANDS_OUTPUT_CONTRACT: &str = "Final answer must include exactly one fenced JSON object: `{\"planning_task_commands\":{\"version\":1,\"commands\":[...]}}`.";
const PLANNING_TASK_COMMANDS_SHAPE_RULE: &str = "Each command must be a flat object with a required top-level `op` field, for example `{\"op\":\"create_task\",\"title\":\"...\"}` or `{\"op\":\"update_task\",\"task_id\":\"...\"}`.";
const PLANNING_TASK_COMMANDS_WRAPPER_RULE: &str =
    "Do not wrap commands as `{\"create_task\":{...}}` or `{\"update_task\":{...}}`.";

// Authority excerpts are advisory prompt context; caps keep large DB snapshots from dominating the worker prompt.
const MAX_WORKER_DIRECTION_AUTHORITY_CHARS: usize = 4_000;
const MAX_WORKER_TASK_AUTHORITY_CHARS: usize = 4_000;
const MAX_WORKER_QUEUE_PROJECTION_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Refresh prompts advance normal queue state; repair prompts fix a rejected candidate under stricter rules.
pub(crate) enum PlanningTaskMutationPromptMode {
    Refresh,
    Repair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * Previous handoff is the last task the worker was responsible for.
 * Refresh and repair prompts both include it so the next response either records progress on that task or
 * explicitly changes the queue instead of returning the same unchanged head.
 */
pub(crate) struct PlanningPromptHandoff<'a> {
    pub(crate) task_id: &'a str,
    pub(crate) task_title: &'a str,
    pub(crate) updated_at: &'a str,
    pub(crate) status_label: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Serialized DB authority context that worker prompts render as named prompt sections.
pub(crate) struct PlanningWorkerAuthorityPromptContext {
    pub(crate) status_lines: Vec<String>,
    pub(crate) direction_authority_json: Option<String>,
    pub(crate) task_authority_json: Option<String>,
    pub(crate) queue_projection_json: Option<String>,
}

// Role lines state protected files and defer source-of-truth detail to the db-authority section.
pub(crate) fn worker_role_lines() -> Vec<String> {
    vec![
        "session=planning-only".to_string(),
        "protected_files=`result-output.md`, direction detail docs, queue-idle review prompt"
            .to_string(),
        "Use only the accepted DB authority sections as planning authority.".to_string(),
    ]
}

/*
 * Add accepted DB authority excerpts to a worker prompt.
 * Each block is optional and independently truncated so missing authority surfaces as status copy while present
 * authority remains inspectable without pushing the mutation rules out of the model context.
 */
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

// Normal worker output may create or update tasks, but must prefer already-applied planning-tool mutations.
pub(crate) fn worker_task_authority_output_contract() -> Vec<String> {
    vec![
        PLANNING_TASK_COMMANDS_OUTPUT_CONTRACT.to_string(),
        PLANNING_TASK_COMMANDS_SHAPE_RULE.to_string(),
        PLANNING_TASK_COMMANDS_WRAPPER_RULE.to_string(),
        "`commands` may contain only `create_task` or `update_task` operations.".to_string(),
        "If `planning-task-tool` has already applied a mutation successfully, return an empty `commands` array so the host does not apply it twice."
            .to_string(),
        "Do not return `task_authority` or a full task ledger document.".to_string(),
        "Do not include fields controlled by the application: `id`, `created_by`, `last_updated_by`, `updated_at`, or `source_turn_id`."
            .to_string(),
        "Use `status=cancelled` to cancel work; do not emit delete operations.".to_string(),
        "End with a short natural-language summary of the task command changes.".to_string(),
    ]
}

// Repair output is narrower: fix listed validation failures with the smallest safe mutation set.
pub(crate) fn repair_task_authority_output_contract() -> Vec<String> {
    vec![
        PLANNING_TASK_COMMANDS_OUTPUT_CONTRACT.to_string(),
        PLANNING_TASK_COMMANDS_SHAPE_RULE.to_string(),
        PLANNING_TASK_COMMANDS_WRAPPER_RULE.to_string(),
        "`commands` must be the smallest create/update set needed to resolve the validation errors."
            .to_string(),
        "Priority repairs must keep `base_priority + dynamic_priority_delta` within `0-100` inclusive."
            .to_string(),
        "When `dynamic_priority_delta != 0`, include `priority_reason` or preserve the existing non-empty reason."
            .to_string(),
        "Do not use an empty `commands` array to resolve a listed validation error when a rejected candidate command or task id is present, unless the mutation was already applied via `akra planning-tool`."
            .to_string(),
        "When the rejected candidate used wrapped commands, preserve the same task intent and rewrite it into the flat `op` command shape."
            .to_string(),
        "Do not return `task_authority` or a full task ledger document.".to_string(),
        "Do not include fields controlled by the application: `id`, `created_by`, `last_updated_by`, `updated_at`, or `source_turn_id`."
            .to_string(),
        "Resolve every validation error listed below.".to_string(),
    ]
}

/*
 * Shared mutation block inserted into refresh and repair prompts.
 * It teaches the worker to use the host planning-tool first, then use the final JSON envelope only as a
 * fallback; the final-response contract is mode-specific because repair has stricter validation recovery rules.
 */
pub(crate) fn add_planning_task_mutation_sections(
    builder: PromptDocumentBuilder,
    mode: PlanningTaskMutationPromptMode,
) -> PromptDocumentBuilder {
    builder
        .bullets(
            "mutation-workflow",
            planning_task_mutation_workflow_rules(mode),
        )
        .text(
            "planning-task-tool-contract",
            planning_task_tool_contract_json(),
        )
        .code_block(
            "planning-task-tool-examples",
            "bash",
            planning_task_tool_examples(mode),
        )
        .bullets(
            "final-response-contract",
            match mode {
                PlanningTaskMutationPromptMode::Refresh => worker_task_authority_output_contract(),
                PlanningTaskMutationPromptMode::Repair => repair_task_authority_output_contract(),
            },
        )
}

// Workflow rules prevent double-application when the worker already applied the mutation via the CLI.
fn planning_task_mutation_workflow_rules(mode: PlanningTaskMutationPromptMode) -> Vec<String> {
    let mut rules = vec![
        "First inspect accepted task state with `akra planning-tool run .` and `op=list_tasks` before deciding create vs update."
            .to_string(),
        "Use `akra planning-tool run .` with `apply=true` for every create_task or update_task mutation whenever the CLI is available."
            .to_string(),
        "After a successful planning-tool mutation, the final `planning_task_commands` envelope must use `commands: []` to prevent double application."
            .to_string(),
        "Use non-empty final `planning_task_commands` only as a fallback when the planning-tool CLI cannot be used or returns an error."
            .to_string(),
        "Never both apply a mutation with planning-tool and repeat the same mutation in the final envelope."
            .to_string(),
    ];
    if mode == PlanningTaskMutationPromptMode::Repair {
        rules.extend([
            "In repair mode, empty final commands are valid only after `akra planning-tool` has successfully applied the repair; otherwise keep correcting the repair payload."
                .to_string(),
            "If planning-tool rejects the repair mutation, correct the payload and retry within this turn before falling back to final commands."
                .to_string(),
        ]);
    }
    rules
}

// Examples are intentionally shell snippets because the worker can paste them directly in a planning turn.
fn planning_task_tool_examples(mode: PlanningTaskMutationPromptMode) -> &'static str {
    match mode {
        PlanningTaskMutationPromptMode::Refresh => {
            r#"printf '%s\n' '{"version":1,"op":"list_tasks","status":["ready","in_progress","blocked","proposed"],"limit":20}' | akra planning-tool run .
printf '%s\n' '{"version":1,"op":"update_task","apply":true,"task_id":"task-123","status":"done","priority_reason":"Completed by latest accepted work."}' | akra planning-tool run ."#
        }
        PlanningTaskMutationPromptMode::Repair => {
            r#"printf '%s\n' '{"version":1,"op":"list_tasks","limit":20}' | akra planning-tool run .
printf '%s\n' '{"version":1,"op":"update_task","apply":true,"task_id":"task-123","dynamic_priority_delta":-10,"priority_reason":"Adjusted so combined priority stays within 0-100 inclusive."}' | akra planning-tool run ."#
        }
    }
}

// Runtime prompt rules apply to ordinary planning workers that may discover or close queue work.
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

// Repair prompts forbid file edits because they exist to correct DB task commands, not workspace copy.
pub(crate) fn repair_constraints() -> Vec<String> {
    vec![
        "Do not edit planning files in this turn; return corrected planning task commands as JSON only."
            .to_string(),
        format!("Do not edit `{}`.", RESULT_OUTPUT_FILE_PATH),
        "Use the last accepted DB snapshot as the current task authority baseline.".to_string(),
        "Do not add unrelated work outside the existing direction frame.".to_string(),
    ]
}

// Worker refresh gets stronger instructions: do not return an unchanged ready head as progress.
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

// Repair handoff only needs to prove that the still-active task has changed in the corrected ledger.
pub(crate) fn repair_previous_handoff_lines(
    previous_handoff: Option<PlanningPromptHandoff<'_>>,
) -> Vec<String> {
    previous_handoff.map_or_else(Vec::new, |previous_handoff| {
        let mut lines = handoff_common_lines(previous_handoff);
        lines.push("If this task stays active, the ledger must show what changed.".to_string());
        lines
    })
}

// Common handoff lines stay key=value to make prompt comparisons easy in tests and snapshots.
fn handoff_common_lines(handoff: PlanningPromptHandoff<'_>) -> Vec<String> {
    vec![
        format!("task_id={}", handoff.task_id),
        format!("title={}", handoff.task_title),
        format!("updated_at={}", handoff.updated_at),
        format!("status={}", handoff.status_label),
    ]
}

// Optional sections disappear when absent or trimmed empty, avoiding blank code blocks in generated prompts.
fn truncate_optional_prompt_section(body: Option<&str>, max_chars: usize) -> Option<String> {
    body.map(|body| truncate_prompt_section(body, max_chars))
        .filter(|body| !body.trim().is_empty())
}

// Character-based truncation preserves UTF-8 boundaries and appends an explicit marker for operator context.
pub(crate) fn truncate_prompt_section(body: &str, max_chars: usize) -> String {
    let body = body.trim();
    if body.chars().count() <= max_chars {
        return body.to_string();
    }
    let truncated = body.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n... [truncated]")
}

#[cfg(test)]
// Tests pin wording-sensitive prompt contracts because downstream workers parse these instructions behaviorally.
mod tests {
    use super::{
        PlanningPromptHandoff, PlanningTaskMutationPromptMode,
        PlanningWorkerAuthorityPromptContext, add_planning_task_mutation_sections,
        add_worker_authority_context_sections, repair_constraints, repair_previous_handoff_lines,
        repair_task_authority_output_contract, runtime_task_authority_contract_rules,
        worker_previous_handoff_lines, worker_role_lines, worker_task_authority_output_contract,
    };
    use crate::application::service::prompt_component::PromptDocument;

    // Runtime and repair prompts must both name DB authority as the accepted baseline.
    #[test]
    fn shared_contract_sections_keep_db_authority_source_of_truth() {
        let runtime_rules = runtime_task_authority_contract_rules().join("\n");
        let repair_rules = repair_constraints().join("\n");

        assert!(runtime_rules.contains("accepted DB authority"));
        assert!(repair_rules.contains("last accepted DB snapshot"));
    }

    // The output contract must stay aligned with the parser for planning_task_commands.
    #[test]
    fn shared_output_contract_uses_required_task_command_payload() {
        let contract = worker_task_authority_output_contract().join("\n");

        assert!(contract.contains("\"planning_task_commands\""));
        assert!(contract.contains("create_task"));
        assert!(contract.contains("\"op\":\"create_task\""));
        assert!(contract.contains("Do not wrap commands"));
        assert!(contract.contains("planning-task-tool"));
        assert!(contract.contains("does not apply it twice"));
        assert!(contract.contains("Do not return `task_authority`"));
    }

    // Prompt sections make planning-tool application primary and final JSON a fallback.
    #[test]
    fn shared_mutation_sections_make_tool_use_primary() {
        let prompt = add_planning_task_mutation_sections(
            PromptDocument::builder("task"),
            PlanningTaskMutationPromptMode::Refresh,
        )
        .build()
        .render();

        assert!(prompt.contains("[mutation-workflow]"));
        assert!(prompt.contains("First inspect accepted task state"));
        assert!(prompt.contains("Use `akra planning-tool run .`"));
        assert!(prompt.contains("commands: []"));
        assert!(prompt.contains("only as a fallback"));
        assert!(prompt.contains("[planning-task-tool-contract]"));
        assert!(prompt.contains("[planning-task-tool-examples]"));
        assert!(prompt.contains("[final-response-contract]"));
    }

    // Repair-specific contract protects priority bounds and rejects empty no-op fixes for known errors.
    #[test]
    fn repair_contract_names_priority_and_empty_command_guards() {
        let contract = repair_task_authority_output_contract().join("\n");

        assert!(contract.contains("base_priority + dynamic_priority_delta"));
        assert!(contract.contains("within `0-100` inclusive"));
        assert!(contract.contains("priority_reason"));
        assert!(contract.contains("empty `commands` array"));
    }

    // Refresh and repair use the same handoff fields but apply different anti-loop language.
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

    // Source-of-truth detail belongs in db-authority context, not duplicated in the role header.
    #[test]
    fn worker_role_leaves_source_of_truth_to_db_authority_section() {
        let role = worker_role_lines().join("\n");

        assert!(!role.contains("source_of_truth="));
    }

    // Large authority JSON should be available as context but bounded to keep mutation rules visible.
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

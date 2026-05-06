use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::application::service::planning::task_tool::planning_task_tool_contract_json;
use crate::application::service::prompt_component::PromptDocumentBuilder;

/*
 * worker refresh prompt와 repair prompt가 같은 DB-authority contract를 공유하게 하는 공통
 * prompt section 모듈이다. 구체적인 prompt builder는 role, validation, context 순서를 결정하고,
 * 이 모듈은 source-of-truth framing, planning-task command output, planning-tool 사용법,
 * previous-handoff 처리, bounded authority excerpt처럼 반복되는 문구를 공급한다.
 */

// final-answer envelope는 task tool parser와 맞아야 한다. 예전 full-authority JSON shape를
// 허용하면 worker가 DB authority 전체 교체를 시도할 수 있다.
const PLANNING_TASK_COMMANDS_OUTPUT_CONTRACT: &str = "Final answer must include exactly one fenced JSON object: `{\"planning_task_commands\":{\"version\":1,\"commands\":[...]}}`.";
const PLANNING_TASK_COMMANDS_SHAPE_RULE: &str = "Each command must be a flat object with a required top-level `op` field, for example `{\"op\":\"create_task\",\"title\":\"...\"}` or `{\"op\":\"update_task\",\"task_id\":\"...\"}`.";
const PLANNING_TASK_COMMANDS_WRAPPER_RULE: &str =
    "Do not wrap commands as `{\"create_task\":{...}}` or `{\"update_task\":{...}}`.";

// authority excerpt는 advisory prompt context다. cap은 큰 DB snapshot이 worker prompt를 지배해
// mutation rule과 repair instruction을 밀어내지 못하게 한다.
const MAX_WORKER_DIRECTION_AUTHORITY_CHARS: usize = 4_000;
const MAX_WORKER_TASK_AUTHORITY_CHARS: usize = 4_000;
const MAX_WORKER_QUEUE_PROJECTION_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// refresh prompt는 정상 queue state를 전진시키고, repair prompt는 거부된 candidate를 더 엄격한 규칙으로 고친다.
pub(crate) enum PlanningTaskMutationPromptMode {
    Refresh,
    Repair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * previous handoff는 직전 worker가 책임졌던 마지막 task다. refresh와 repair prompt가 모두 이를
 * 포함하는 이유는 다음 응답이 그 task의 진행을 기록하거나 queue를 명시적으로 바꾸게 하기 위해서다.
 * 같은 unchanged head를 다시 반환하는 loop를 prompt level에서 줄인다.
 */
pub(crate) struct PlanningPromptHandoff<'a> {
    pub(crate) task_id: &'a str,
    pub(crate) task_title: &'a str,
    pub(crate) updated_at: &'a str,
    pub(crate) status_label: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// worker prompt가 named section으로 렌더링할 serialized DB authority context다.
pub(crate) struct PlanningWorkerAuthorityPromptContext {
    pub(crate) status_lines: Vec<String>,
    pub(crate) direction_authority_json: Option<String>,
    pub(crate) task_authority_json: Option<String>,
    pub(crate) queue_projection_json: Option<String>,
}

// role line은 protected file만 선언하고 source-of-truth 세부사항은 db-authority section에 맡긴다.
pub(crate) fn worker_role_lines() -> Vec<String> {
    vec![
        "session=planning-only".to_string(),
        "protected_files=`result-output.md`, direction detail docs, queue-idle review prompt"
            .to_string(),
        "Use only the accepted DB authority sections as planning authority.".to_string(),
    ]
}

/*
 * accepted DB authority excerpt를 worker prompt에 추가한다. 각 block은 optional이고 독립적으로
 * truncate된다. authority가 없으면 status copy로 표면화되고, 존재하는 authority는 mutation rule을
 * model context 밖으로 밀어내지 않는 범위에서만 inspection 가능하게 한다.
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

// 일반 worker output은 task create/update를 할 수 있지만, 이미 planning-tool로 적용한 mutation을 우선해야 한다.
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
        "For `update_task`, omit `description`; existing non-empty descriptions are preserved by the host."
            .to_string(),
        "Use `status=cancelled` to cancel work; do not emit delete operations.".to_string(),
        "End with a short natural-language summary of the task command changes.".to_string(),
    ]
}

// repair output은 더 좁다. listed validation failure를 해결하는 가장 작은 safe mutation set만 허용한다.
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
        "For `update_task`, omit `description`; existing non-empty descriptions are preserved by the host."
            .to_string(),
        "Resolve every validation error listed below.".to_string(),
    ]
}

/*
 * refresh/repair prompt에 공통 삽입되는 mutation block이다. worker에게 host planning-tool을 먼저
 * 쓰고 final JSON envelope는 fallback으로만 쓰라고 가르친다. repair는 validation recovery rule이
 * 더 엄격하므로 final-response contract만 mode별로 갈라진다.
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

// workflow rule은 worker가 CLI로 이미 mutation을 적용한 뒤 final envelope에서 같은 변경을 다시 내는
// double-application을 막는다.
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

// example은 의도적으로 shell snippet이다. worker가 planning turn 안에서 바로 붙여 실행할 수 있어야 한다.
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

// runtime prompt rule은 queue work를 발견하거나 닫는 일반 planning worker에게 적용된다.
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

// repair prompt는 file edit을 금지한다. 이 turn의 목적은 workspace copy가 아니라 DB task command를 고치는 것이다.
pub(crate) fn repair_constraints() -> Vec<String> {
    vec![
        "Do not edit planning files in this turn; return corrected planning task commands as JSON only."
            .to_string(),
        format!("Do not edit `{}`.", RESULT_OUTPUT_FILE_PATH),
        "Use the last accepted DB snapshot as the current task authority baseline.".to_string(),
        "Do not add unrelated work outside the existing direction frame.".to_string(),
    ]
}

// worker refresh는 더 강한 anti-loop 지시를 받는다. unchanged ready head를 progress처럼 반환하면 안 된다.
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

// repair handoff는 still-active task가 corrected ledger에서 실제로 바뀌었음을 증명하면 된다.
pub(crate) fn repair_previous_handoff_lines(
    previous_handoff: Option<PlanningPromptHandoff<'_>>,
) -> Vec<String> {
    previous_handoff.map_or_else(Vec::new, |previous_handoff| {
        let mut lines = handoff_common_lines(previous_handoff);
        lines.push("If this task stays active, the ledger must show what changed.".to_string());
        lines
    })
}

// 공통 handoff line은 key=value 형식을 유지한다. test와 snapshot에서 prompt 비교가 안정적이기 때문이다.
fn handoff_common_lines(handoff: PlanningPromptHandoff<'_>) -> Vec<String> {
    vec![
        format!("task_id={}", handoff.task_id),
        format!("title={}", handoff.task_title),
        format!("updated_at={}", handoff.updated_at),
        format!("status={}", handoff.status_label),
    ]
}

// optional section은 없거나 trim 후 비어 있으면 사라진다. generated prompt에 빈 code block을 만들지 않는다.
fn truncate_optional_prompt_section(body: Option<&str>, max_chars: usize) -> Option<String> {
    body.map(|body| truncate_prompt_section(body, max_chars))
        .filter(|body| !body.trim().is_empty())
}

// character 기반 truncation은 UTF-8 boundary를 보존하고, operator context를 위해 명시적 marker를 붙인다.
pub(crate) fn truncate_prompt_section(body: &str, max_chars: usize) -> String {
    let body = body.trim();
    if body.chars().count() <= max_chars {
        return body.to_string();
    }
    let truncated = body.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n... [truncated]")
}

#[cfg(test)]
// downstream worker는 이 지시문을 행동 계약으로 해석하므로, 테스트는 wording-sensitive prompt contract를 고정한다.
mod tests {
    use super::{
        PlanningPromptHandoff, PlanningTaskMutationPromptMode,
        PlanningWorkerAuthorityPromptContext, add_planning_task_mutation_sections,
        add_worker_authority_context_sections, repair_constraints, repair_previous_handoff_lines,
        repair_task_authority_output_contract, runtime_task_authority_contract_rules,
        worker_previous_handoff_lines, worker_role_lines, worker_task_authority_output_contract,
    };
    use crate::application::service::prompt_component::PromptDocument;

    // runtime과 repair prompt 모두 DB authority를 accepted baseline으로 명시해야 한다.
    #[test]
    fn shared_contract_sections_keep_db_authority_source_of_truth() {
        let runtime_rules = runtime_task_authority_contract_rules().join("\n");
        let repair_rules = repair_constraints().join("\n");

        assert!(runtime_rules.contains("accepted DB authority"));
        assert!(repair_rules.contains("last accepted DB snapshot"));
    }

    // output contract는 planning_task_commands parser와 계속 맞아야 한다.
    #[test]
    fn shared_output_contract_uses_required_task_command_payload() {
        let contract = worker_task_authority_output_contract().join("\n");

        assert!(contract.contains("\"planning_task_commands\""));
        assert!(contract.contains("create_task"));
        assert!(contract.contains("\"op\":\"create_task\""));
        assert!(contract.contains("omit `description`"));
        assert!(contract.contains("Do not wrap commands"));
        assert!(contract.contains("planning-task-tool"));
        assert!(contract.contains("does not apply it twice"));
        assert!(contract.contains("Do not return `task_authority`"));
    }

    // prompt section은 planning-tool 적용을 primary로, final JSON을 fallback으로 만든다.
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

    // repair-specific contract는 priority bound를 보호하고 알려진 error에 대한 empty no-op fix를 거부한다.
    #[test]
    fn repair_contract_names_priority_and_empty_command_guards() {
        let contract = repair_task_authority_output_contract().join("\n");

        assert!(contract.contains("base_priority + dynamic_priority_delta"));
        assert!(contract.contains("within `0-100` inclusive"));
        assert!(contract.contains("priority_reason"));
        assert!(contract.contains("empty `commands` array"));
    }

    // refresh와 repair는 같은 handoff field를 쓰지만 다른 anti-loop 문구를 적용한다.
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

    // source-of-truth detail은 role header에 중복하지 않고 db-authority context에 둔다.
    #[test]
    fn worker_role_leaves_source_of_truth_to_db_authority_section() {
        let role = worker_role_lines().join("\n");

        assert!(!role.contains("source_of_truth="));
    }

    // 큰 authority JSON은 context로 제공하되 mutation rule이 보이도록 bounded 상태여야 한다.
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

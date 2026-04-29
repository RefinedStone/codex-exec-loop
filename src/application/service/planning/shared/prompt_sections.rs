use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::application::service::prompt_component::PromptDocumentBuilder;

const LEGACY_AUTHORITY_ARTIFACTS: &str = "`task-ledger.json`, `directions.toml`, `queue.snapshot.json`, `planning-snapshot.json`, and `.codex-exec-loop/runtime/exports/*`";

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
        "source_of_truth=DB direction authority + DB task authority + DB queue projection"
            .to_string(),
        "protected_files=`result-output.md`, direction detail docs, queue-idle review prompt"
            .to_string(),
        "Do not read or infer planning authority from stale legacy/export artifacts.".to_string(),
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
            authority_context.direction_authority_json.as_deref(),
        )
        .optional_code_block(
            "accepted-db-task-authority",
            "json",
            authority_context.task_authority_json.as_deref(),
        )
        .optional_code_block(
            "db-queue-projection",
            "json",
            authority_context.queue_projection_json.as_deref(),
        )
}

pub(crate) fn worker_task_authority_output_contract() -> Vec<String> {
    vec![
        "Final answer must include exactly one fenced JSON object: `{\"task_authority\": {...}}`."
            .to_string(),
        "`task_authority` is the full updated task ledger document.".to_string(),
        "End with a short natural-language summary of the ledger changes.".to_string(),
    ]
}

pub(crate) fn repair_task_authority_output_contract() -> Vec<String> {
    vec![
        "Final answer must include exactly one fenced JSON object: `{\"task_authority\": {...}}`."
            .to_string(),
        "`task_authority` must be the full updated task authority document.".to_string(),
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
        "Task catalog mutations must go through the runtime task authority flow; queue validation refreshes prompt state."
            .to_string(),
        "Ignore stale legacy/export artifacts (`task-ledger.json`, `directions.toml`, `queue.snapshot.json`, `planning-snapshot.json`, `.codex-exec-loop/runtime/exports/*`); DB authority is the only planning source of truth."
            .to_string(),
    ]
}

pub(crate) fn repair_constraints() -> Vec<String> {
    vec![
        "Do not edit planning files in this turn; return the corrected ledger as JSON only."
            .to_string(),
        format!("Do not edit `{}`.", RESULT_OUTPUT_FILE_PATH),
        "Use the last accepted DB snapshot as the current task authority baseline.".to_string(),
        format!("Ignore stale legacy/export artifacts such as {LEGACY_AUTHORITY_ARTIFACTS}."),
        "Do not add unrelated work outside the existing direction frame.".to_string(),
    ]
}

pub(crate) fn worker_previous_handoff_lines(
    previous_handoff: Option<PlanningPromptHandoff<'_>>,
) -> Vec<String> {
    previous_handoff.map_or_else(Vec::new, |task| {
        vec![
            format!("task_id={}", task.task_id),
            format!("title={}", task.task_title),
            format!("updated_at={}", task.updated_at),
            format!("status={}", task.status_label),
            "Do not select this task again as unchanged `ready` queue head.".to_string(),
            "If complete, mark `done`; if still active, update the task from latest evidence."
                .to_string(),
            "If follow-up work split out, update the existing task or add a new task.".to_string(),
        ]
    })
}

pub(crate) fn repair_previous_handoff_lines(
    previous_handoff: Option<PlanningPromptHandoff<'_>>,
) -> Vec<String> {
    previous_handoff.map_or_else(Vec::new, |previous_handoff| {
        vec![
            format!("task_id={}", previous_handoff.task_id),
            format!("title={}", previous_handoff.task_title),
            format!("updated_at={}", previous_handoff.updated_at),
            format!("status={}", previous_handoff.status_label),
            "If this task stays active, the ledger must show what changed.".to_string(),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::{
        PlanningPromptHandoff, repair_constraints, repair_previous_handoff_lines,
        runtime_task_authority_contract_rules, worker_previous_handoff_lines,
        worker_task_authority_output_contract,
    };

    #[test]
    fn shared_contract_sections_keep_legacy_ignore_language() {
        let runtime_rules = runtime_task_authority_contract_rules().join("\n");
        let repair_rules = repair_constraints().join("\n");

        assert!(runtime_rules.contains("DB authority is the only planning source of truth"));
        assert!(runtime_rules.contains("task-ledger.json"));
        assert!(repair_rules.contains("directions.toml"));
        assert!(repair_rules.contains(".codex-exec-loop/runtime/exports/*"));
    }

    #[test]
    fn shared_output_contract_uses_required_task_authority_payload() {
        let contract = worker_task_authority_output_contract().join("\n");

        assert!(contract.contains("\"task_authority\""));
        assert!(contract.contains("full updated task ledger document"));
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
}

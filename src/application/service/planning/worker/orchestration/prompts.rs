use crate::application::service::planning::runtime::facade::PlanningTaskHandoff;
use crate::application::service::planning::shared::prompt_sections::{
    PlanningPromptHandoff, PlanningTaskMutationPromptMode, PlanningWorkerAuthorityPromptContext,
    add_planning_task_mutation_sections, add_worker_authority_context_sections,
    worker_previous_handoff_lines, worker_role_lines,
};
use crate::application::service::prompt_component::PromptDocument;
use crate::domain::planning::PlanningOfficialCompletionRefreshContract;

/*
 * 이 builder들은 model-backed planning worker가 받는 prompt contract를 정의한다.
 * 모든 variant는 같은 DB authority section과 mutation schema에서 시작하고, orchestration path별로
 * 최신 main-session evidence를 어떻게 해석할지 알려 주는 policy만 덧붙인다.
 */
pub(super) fn build_planning_queue_refresh_prompt(
    latest_user_message: Option<&str>,
    latest_main_reply: &str,
    previous_handoff_task: Option<&PlanningTaskHandoff>,
    authority_context: &PlanningWorkerAuthorityPromptContext,
) -> String {
    // queue refresh는 일반 post-turn 경로다. accepted DB authority를 먼저 놓고, model-visible tool syntax를
    // 그 다음에 둔 뒤, volatile chat evidence는 더 강한 source 뒤에만 붙인다.
    let builder = add_worker_authority_context_sections(
        PromptDocument::builder("planning-worker-refresh").lines("role", worker_role_lines()),
        authority_context,
    );
    add_planning_task_mutation_sections(builder, PlanningTaskMutationPromptMode::Refresh)
        .bullets("refresh-policy", queue_refresh_policy_rules())
        .bullets("queue-advancement", queue_advancement_rules())
        .optional_text("latest-operator-request", latest_user_message)
        .lines(
            "previous-handoff",
            worker_previous_handoff_lines(previous_handoff_task.map(worker_handoff)),
        )
        .text("main-session-latest-reply", latest_main_reply)
        .build()
        .render()
}
pub(super) fn build_planning_queue_idle_derive_prompt(
    latest_user_message: Option<&str>,
    latest_main_reply: &str,
    previous_handoff_task: Option<&PlanningTaskHandoff>,
    queue_idle_prompt_markdown: &str,
    authority_context: &PlanningWorkerAuthorityPromptContext,
) -> String {
    // queue-idle review는 active task가 없을 때 실행된다. worker를 의도적으로 evaluator로 framing해,
    // 자신감 있는 main-session answer가 missing validation이나 충족되지 않은 DB success criteria를 덮지 못하게 한다.
    let builder = add_worker_authority_context_sections(
        PromptDocument::builder("planning-worker-queue-idle-review")
            .lines("role", worker_role_lines()),
        authority_context,
    );
    add_planning_task_mutation_sections(builder, PlanningTaskMutationPromptMode::Refresh)
        .bullets("idle-review-policy", queue_idle_review_policy_rules())
        .optional_text("latest-operator-request", latest_user_message)
        .lines(
            "previous-handoff",
            worker_previous_handoff_lines(previous_handoff_task.map(worker_handoff)),
        )
        .text("queue-idle-review-prompt", queue_idle_prompt_markdown)
        .text("main-session-latest-reply", latest_main_reply)
        .bullets(
            "final-queue-idle-decision-rules",
            queue_idle_final_decision_rules(),
        )
        .build()
        .render()
}
pub(super) fn build_planning_official_completion_prompt(
    latest_user_message: Option<&str>,
    latest_main_reply: &str,
    previous_handoff_task: Option<&PlanningTaskHandoff>,
    contract: &PlanningOfficialCompletionRefreshContract,
    authority_context: &PlanningWorkerAuthorityPromptContext,
) -> String {
    // official completion은 generic queue sweep이 아니라 ledger refresh다. serialized payload는 completion report이고,
    // worktree_path는 provenance로만 남아야 하며 planning-tool workspace가 되면 안 된다.
    let serialized_contract = serialize_official_completion_refresh_contract(contract);
    let contract_block = format!("```json\n{serialized_contract}\n```");
    let builder = add_worker_authority_context_sections(
        PromptDocument::builder("planning-worker-official-completion")
            .lines("role", worker_role_lines()),
        authority_context,
    );
    add_planning_task_mutation_sections(builder, PlanningTaskMutationPromptMode::Refresh)
        .bullets("completion-policy", official_completion_policy_rules())
        .bullets("queue-advancement", queue_advancement_rules())
        .optional_text("latest-operator-request", latest_user_message)
        .lines(
            "previous-handoff",
            worker_previous_handoff_lines(previous_handoff_task.map(worker_handoff)),
        )
        .text("completion-refresh-contract", &contract_block)
        .text("main-session-latest-reply", latest_main_reply)
        .build()
        .render()
}
fn serialize_official_completion_refresh_contract(
    contract: &PlanningOfficialCompletionRefreshContract,
) -> String {
    // 이 shape의 owner는 domain contract다. prompt rendering은 model-facing payload를 테스트로 고정할 수 있도록
    // stable JSON envelope만 제공한다.
    serde_json::to_string_pretty(&contract)
        .expect("official completion refresh contract should serialize")
}

/* refresh policy는 worker가 매 turn을 빈 slate처럼 취급하지 못하게 하는 최소 기억 장치다. */
fn queue_refresh_policy_rules() -> Vec<String> {
    vec![
        "Use planning context, latest operator request, and latest main-session reply together."
            .to_string(),
        "If the latest reply names next steps, follow-up work, gaps, or a numbered checklist, treat that as the strongest follow-up signal."
            .to_string(),
        "Update existing matching tasks/proposals instead of creating duplicates.".to_string(),
        "Keep only executable work in `ready`, `blocked`, or `in_progress`; keep operator-choice candidates as `proposed`."
            .to_string(),
        "If proposals exist and the next executable step is clear, promote one top proposal to `ready` and keep the rest proposed."
            .to_string(),
        "If part of a task is complete, narrow the existing task to remaining work or split completed and follow-up slices."
            .to_string(),
    ]
}

/*
 * queue-idle policy는 refresh policy보다 더 엄격하다.
 * 빈 queue는 쉽게 "완료"처럼 보이기 때문이다. worker는 reply를 accepted direction criteria와 비교하고,
 * evidence가 아직 불완전하면 좁은 follow-up 하나를 만들어야 한다.
 */
fn queue_idle_review_policy_rules() -> Vec<String> {
    vec![
        "The queue is empty; act as a post-turn evaluator, not a TODO extractor for the main-session."
            .to_string(),
        "`main-session-latest-reply` is evidence only; it is not completion authority and must not override DB direction goals or success criteria."
            .to_string(),
        "Compare the latest operator request and main-session result against DB direction success criteria, detail docs, accepted task authority, and DB queue projection."
            .to_string(),
        "Create or update a task when criteria remain unmet, validation is missing, or the next execution slice is clear, even if the main reply has no explicit TODO list."
            .to_string(),
        "Put only the single clearest immediate follow-up in `ready` or `in_progress`; keep alternatives as `proposed`."
            .to_string(),
        "If no useful work remains, keep the queue empty and summarize why.".to_string(),
    ]
}

fn queue_idle_final_decision_rules() -> Vec<String> {
    // 이 rule들은 operator prompt와 main reply 뒤에 배치된다. persisted direction text 안의 오래된 queue-idle 문구보다 우선한다.
    vec![
        "These rules are the final authority for the queue-idle decision, even if older direction copy or queue-idle prompt text says otherwise."
            .to_string(),
        "Ignore legacy wording that treats file-backed planning authority or answer-implied completion as the completion test; accepted DB authority and independent evaluator judgment win."
            .to_string(),
        "Do not leave `commands` empty solely because the main reply says the work is complete, merged, tested, or validated."
            .to_string(),
        "If the latest operator request asked for nontrivial code, DB, runtime, or planning behavior changes and accepted DB task authority is empty or has no matching completed task, create one narrow follow-up task for independent review, verification, or hardening unless the supplied DB authority itself proves no work remains."
            .to_string(),
        "The follow-up task should check the implementation against the original request and any risks visible in the main reply; it must not re-run the entire project or duplicate completed work."
            .to_string(),
    ]
}

fn official_completion_policy_rules() -> Vec<String> {
    // worker는 parallel-agent report를 official DB ledger로 되돌려 맞춘다. provenance field는 출처를 설명할 뿐
    // ledger task의 의미를 바꾸지 않는다.
    vec![
        "Completion payload is an unofficial agent report until this ledger refresh succeeds."
            .to_string(),
        "Match by `task_id` and `task_title`; decide whether the ledger task is `done`, `blocked`, or still active with updates."
            .to_string(),
        "Process the supplied contract as the single official ledger update input for this refresh order."
            .to_string(),
        "`commit_sha`, `branch_name`, and `worktree_path` are provenance; reflect task meaning in the ledger."
            .to_string(),
        "Do not create follow-up tasks whose only remaining work is delivery, push, PR creation, merge, rebase, or worktree cleanup; Akra distributor owns that after official completion."
            .to_string(),
        "If validation failed or did not run, decide whether to create a blocked or remediation task."
            .to_string(),
    ]
}

fn queue_advancement_rules() -> Vec<String> {
    // refresh는 executable queue head를 바꾸거나 current task를 새 evidence로 좁힐 때만 의미 있는 진행으로 본다.
    vec![
        "Do not repeat the same queue head unchanged.".to_string(),
        "If the same task remains queue head, update status, title, priority_reason, priority, blockers, dependencies, or relation note from the latest evidence."
            .to_string(),
        "Do not rewrite an existing non-empty task description as part of automatic task updates."
            .to_string(),
        "Adding only blocked/proposed tasks is not queue advancement.".to_string(),
    ]
}

fn worker_handoff(task: &PlanningTaskHandoff) -> PlanningPromptHandoff<'_> {
    // prompt section에는 compact handoff identity만 필요하다. direction_id나 combined_priority 같은 scheduling field는 제외한다.
    PlanningPromptHandoff {
        task_id: task.task_id.as_str(),
        task_title: task.task_title.as_str(),
        updated_at: task.updated_at.as_str(),
        status_label: task.status_label.as_str(),
    }
}
#[cfg(test)]
mod tests {
    use super::{
        build_planning_official_completion_prompt, build_planning_queue_idle_derive_prompt,
        build_planning_queue_refresh_prompt,
    };
    use crate::application::service::planning::runtime::facade::PlanningTaskHandoff;
    use crate::application::service::planning::shared::prompt_sections::PlanningWorkerAuthorityPromptContext;
    use crate::domain::planning::{
        PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
    };

    #[test]
    fn refresh_prompt_embeds_db_authority_contract() {
        // 이 test는 shared worker contract를 고정한다. DB authority section, planning-tool syntax,
        // raw authority 반환 금지 guard가 한 prompt에 함께 있어야 한다.
        let authority_context = PlanningWorkerAuthorityPromptContext {
            status_lines: vec![
                "source_of_truth=accepted DB direction authority, accepted DB task authority, and DB queue projection below".to_string(),
                "direction_revision=7".to_string(),
                "task_revision=8".to_string(),
            ],
            direction_authority_json: Some("{\"version\":1,\"directions\":[]}".to_string()),
            task_authority_json: Some("{\"version\":1,\"tasks\":[]}".to_string()),
            queue_projection_json: Some(
                "{\"next_task\":null,\"active_tasks\":[],\"proposed_tasks\":[],\"skipped_tasks\":[]}"
                    .to_string(),
            ),
        };
        let prompt = build_planning_queue_refresh_prompt(
            Some("latest user"),
            "latest reply",
            Some(&PlanningTaskHandoff {
                task_id: "task-1".to_string(),
                task_title: "Task 1".to_string(),
                direction_id: "direction-a".to_string(),
                combined_priority: 10,
                updated_at: "2026-04-29T00:00:00Z".to_string(),
                status_label: "ready".to_string(),
            }),
            &authority_context,
        );

        assert!(prompt.contains("[accepted-db-direction-authority]"));
        assert!(prompt.contains("{\"version\":1,\"directions\":[]}"));
        assert!(prompt.contains("[accepted-db-task-authority]"));
        assert!(prompt.contains("{\"version\":1,\"tasks\":[]}"));
        assert!(prompt.contains("[db-queue-projection]"));
        assert!(prompt.contains("\"planning_task_commands\""));
        assert!(prompt.contains("[planning-task-tool-contract]"));
        assert!(prompt.contains("akra planning-tool run ."));
        assert!(prompt.contains("do not use payload.worktree_path"));
        assert!(prompt.contains("Do not return `task_authority`"));
        assert!(prompt.contains("Use only the accepted DB authority sections"));
    }

    #[test]
    fn queue_idle_prompt_renders_evaluator_policy() {
        // queue-idle prompt는 evaluator rule을 main reply 뒤에 유지해야 한다.
        // 그래야 stale하거나 낙관적인 completion language를 override할 수 있다.
        let authority_context = PlanningWorkerAuthorityPromptContext {
            status_lines: vec![
                "source_of_truth=accepted DB direction authority, accepted DB task authority, and DB queue projection below".to_string(),
                "direction_revision=7".to_string(),
                "task_revision=8".to_string(),
            ],
            direction_authority_json: Some(
                "{\"version\":1,\"directions\":[{\"id\":\"direction-a\",\"success_criteria\":[\"validated\"]}]}".to_string(),
            ),
            task_authority_json: Some("{\"version\":1,\"tasks\":[]}".to_string()),
            queue_projection_json: Some(
                "{\"next_task\":null,\"active_tasks\":[],\"proposed_tasks\":[],\"skipped_tasks\":[]}"
                    .to_string(),
            ),
        };
        let prompt = build_planning_queue_idle_derive_prompt(
            Some("latest user"),
            "Implemented everything.",
            None,
            "Queue idle operator prompt",
            &authority_context,
        );

        assert!(prompt.contains("post-turn evaluator"));
        assert!(prompt.contains("not a TODO extractor"));
        assert!(prompt.contains("not completion authority"));
        assert!(prompt.contains("success criteria"));
        assert!(prompt.contains("even if the main reply has no explicit TODO list"));
        assert!(prompt.contains("final-queue-idle-decision-rules"));
        assert!(prompt.contains(
            "Do not leave `commands` empty solely because the main reply says the work is complete"
        ));
        assert!(prompt.contains("create one narrow follow-up task for independent review"));
        assert!(prompt.contains("[main-session-latest-reply]"));
        assert!(
            prompt.find("[final-queue-idle-decision-rules]")
                > prompt.find("[main-session-latest-reply]")
        );
    }

    #[test]
    fn official_completion_prompt_keeps_parallel_worktree_out_of_tool_workspace() {
        // parallel completion payload에는 source worktree가 들어 있지만, planning-tool command는 여전히 official app workspace에서 실행된다.
        let authority_context = PlanningWorkerAuthorityPromptContext {
            status_lines: vec![
                "source_of_truth=accepted DB direction authority, accepted DB task authority, and DB queue projection below".to_string(),
                "direction_revision=7".to_string(),
                "task_revision=8".to_string(),
            ],
            direction_authority_json: Some("{\"version\":1,\"directions\":[]}".to_string()),
            task_authority_json: Some("{\"version\":1,\"tasks\":[]}".to_string()),
            queue_projection_json: Some(
                "{\"next_task\":null,\"active_tasks\":[],\"proposed_tasks\":[],\"skipped_tasks\":[]}"
                    .to_string(),
            ),
        };
        let contract = PlanningOfficialCompletionRefreshContract::new(
            "turn-1",
            11,
            PlanningOfficialCompletionRefreshPayload::new(
                "agent-1",
                "task-1",
                "Task 1",
                "akra-agent/slot-1/task-1",
                "/tmp/parallel-worktree",
                "abc123",
                "validated",
                "completed",
                Some("done".to_string()),
                None,
                "2026-04-29T00:00:00Z",
            ),
        );
        let prompt = build_planning_official_completion_prompt(
            Some("latest user"),
            "latest reply",
            None,
            &contract,
            &authority_context,
        );

        assert!(prompt.contains("[planning-task-tool-contract]"));
        assert!(prompt.contains("akra planning-tool run ."));
        assert!(prompt.contains("do not use payload.worktree_path"));
        assert!(prompt.contains("/tmp/parallel-worktree"));
        assert!(prompt.contains("worktree_path` are provenance"));
        assert!(prompt.contains("Akra distributor owns that after official completion"));
    }
}

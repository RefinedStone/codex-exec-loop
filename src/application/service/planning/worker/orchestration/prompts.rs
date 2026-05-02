// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::service::planning::runtime::facade::PlanningTaskHandoff;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::service::planning::shared::prompt_sections::{
    PlanningPromptHandoff, PlanningTaskMutationPromptMode, PlanningWorkerAuthorityPromptContext,
    add_planning_task_mutation_sections, add_worker_authority_context_sections,
    worker_previous_handoff_lines, worker_role_lines,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::service::prompt_component::PromptDocument;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::planning::PlanningOfficialCompletionRefreshContract;

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_planning_queue_refresh_prompt(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    latest_user_message: Option<&str>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    latest_main_reply: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    previous_handoff_task: Option<&PlanningTaskHandoff>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    authority_context: &PlanningWorkerAuthorityPromptContext,
) -> String {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let builder = add_worker_authority_context_sections(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        PromptDocument::builder("planning-worker-refresh").lines("role", worker_role_lines()),
        authority_context,
    );
    add_planning_task_mutation_sections(builder, PlanningTaskMutationPromptMode::Refresh)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .bullets("refresh-policy", queue_refresh_policy_rules())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .bullets("queue-advancement", queue_advancement_rules())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .optional_text("latest-operator-request", latest_user_message)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .lines(
            "previous-handoff",
            worker_previous_handoff_lines(previous_handoff_task.map(worker_handoff)),
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .text("main-session-latest-reply", latest_main_reply)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .build()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .render()
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_planning_queue_idle_derive_prompt(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    latest_user_message: Option<&str>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    latest_main_reply: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    previous_handoff_task: Option<&PlanningTaskHandoff>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    queue_idle_prompt_markdown: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    authority_context: &PlanningWorkerAuthorityPromptContext,
) -> String {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let builder = add_worker_authority_context_sections(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        PromptDocument::builder("planning-worker-queue-idle-review")
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .lines("role", worker_role_lines()),
        authority_context,
    );
    add_planning_task_mutation_sections(builder, PlanningTaskMutationPromptMode::Refresh)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .bullets("idle-review-policy", queue_idle_review_policy_rules())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .optional_text("latest-operator-request", latest_user_message)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .lines(
            "previous-handoff",
            worker_previous_handoff_lines(previous_handoff_task.map(worker_handoff)),
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .text("queue-idle-review-prompt", queue_idle_prompt_markdown)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .text("main-session-latest-reply", latest_main_reply)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .bullets(
            "final-queue-idle-decision-rules",
            queue_idle_final_decision_rules(),
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .build()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .render()
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_planning_official_completion_prompt(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    latest_user_message: Option<&str>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    latest_main_reply: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    previous_handoff_task: Option<&PlanningTaskHandoff>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    contract: &PlanningOfficialCompletionRefreshContract,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    authority_context: &PlanningWorkerAuthorityPromptContext,
) -> String {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let serialized_contract = serialize_official_completion_refresh_contract(contract);
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let contract_block = format!("```json\n{serialized_contract}\n```");

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let builder = add_worker_authority_context_sections(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        PromptDocument::builder("planning-worker-official-completion")
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .lines("role", worker_role_lines()),
        authority_context,
    );
    add_planning_task_mutation_sections(builder, PlanningTaskMutationPromptMode::Refresh)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .bullets("completion-policy", official_completion_policy_rules())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .bullets("queue-advancement", queue_advancement_rules())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .optional_text("latest-operator-request", latest_user_message)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .lines(
            "previous-handoff",
            worker_previous_handoff_lines(previous_handoff_task.map(worker_handoff)),
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .text("completion-refresh-contract", &contract_block)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .text("main-session-latest-reply", latest_main_reply)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .build()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .render()
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn serialize_official_completion_refresh_contract(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    contract: &PlanningOfficialCompletionRefreshContract,
) -> String {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    serde_json::to_string_pretty(&contract)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("official completion refresh contract should serialize")
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn queue_refresh_policy_rules() -> Vec<String> {
    vec![
        "Use planning context, latest operator request, and latest main-session reply together."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "If the latest reply names next steps, follow-up work, gaps, or a numbered checklist, treat that as the strongest follow-up signal."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "Update existing matching tasks/proposals instead of creating duplicates.".to_string(),
        "Keep only executable work in `ready`, `blocked`, or `in_progress`; keep operator-choice candidates as `proposed`."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "If proposals exist and the next executable step is clear, promote one top proposal to `ready` and keep the rest proposed."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "If part of a task is complete, narrow the existing task to remaining work or split completed and follow-up slices."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
    ]
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn queue_idle_review_policy_rules() -> Vec<String> {
    vec![
        "The queue is empty; act as a post-turn evaluator, not a TODO extractor for the main session."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "`main-session-latest-reply` is evidence only; it is not completion authority and must not override DB direction goals or success criteria."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "Compare the latest operator request and main-session result against DB direction success criteria, detail docs, accepted task authority, and DB queue projection."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "Create or update a task when criteria remain unmet, validation is missing, or the next execution slice is clear, even if the main reply has no explicit TODO list."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "Put only the single clearest immediate follow-up in `ready` or `in_progress`; keep alternatives as `proposed`."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "If no useful work remains, keep the queue empty and summarize why.".to_string(),
    ]
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn queue_idle_final_decision_rules() -> Vec<String> {
    vec![
        "These rules are the final authority for the queue-idle decision, even if older direction copy or queue-idle prompt text says otherwise."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "Ignore legacy wording that treats file-backed planning authority or answer-implied completion as the completion test; accepted DB authority and independent evaluator judgment win."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "Do not leave `commands` empty solely because the main reply says the work is complete, merged, tested, or validated."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "If the latest operator request asked for nontrivial code, DB, runtime, or planning behavior changes and accepted DB task authority is empty or has no matching completed task, create one narrow follow-up task for independent review, verification, or hardening unless the supplied DB authority itself proves no work remains."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "The follow-up task should check the implementation against the original request and any risks visible in the main reply; it must not re-run the entire project or duplicate completed work."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
    ]
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn official_completion_policy_rules() -> Vec<String> {
    vec![
        "Completion payload is an unofficial agent report until this ledger refresh succeeds."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "Match by `task_id` and `task_title`; decide whether the ledger task is `done`, `blocked`, or still active with updates."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "Process the supplied contract as the single official ledger update input for this refresh order."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "`commit_sha`, `branch_name`, and `worktree_path` are provenance; reflect task meaning in the ledger."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "If validation failed or did not run, decide whether to create a blocked or remediation task."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
    ]
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn queue_advancement_rules() -> Vec<String> {
    vec![
        "Do not repeat the same queue head unchanged.".to_string(),
        "If the same task remains queue head, update scope, description, priority_reason, title, status, or updated_at from the latest evidence."
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
        "Adding only blocked/proposed tasks is not queue advancement.".to_string(),
    ]
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn worker_handoff(task: &PlanningTaskHandoff) -> PlanningPromptHandoff<'_> {
    PlanningPromptHandoff {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        task_id: task.task_id.as_str(),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        task_title: task.task_title.as_str(),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        updated_at: task.updated_at.as_str(),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        status_label: task.status_label.as_str(),
    }
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[cfg(test)]
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod tests {
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use super::{
        build_planning_official_completion_prompt, build_planning_queue_idle_derive_prompt,
        build_planning_queue_refresh_prompt,
    };
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use crate::application::service::planning::runtime::facade::PlanningTaskHandoff;
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use crate::application::service::planning::shared::prompt_sections::PlanningWorkerAuthorityPromptContext;
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use crate::domain::planning::{
        PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
    };

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn refresh_prompt_embeds_db_authority_contract() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let authority_context = PlanningWorkerAuthorityPromptContext {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            status_lines: vec![
                "source_of_truth=accepted DB direction authority, accepted DB task authority, and DB queue projection below".to_string(),
                "direction_revision=7".to_string(),
                "task_revision=8".to_string(),
            ],
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            direction_authority_json: Some("{\"version\":1,\"directions\":[]}".to_string()),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            task_authority_json: Some("{\"version\":1,\"tasks\":[]}".to_string()),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            queue_projection_json: Some(
                "{\"next_task\":null,\"active_tasks\":[],\"proposed_tasks\":[],\"skipped_tasks\":[]}"
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .to_string(),
            ),
        };

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let prompt = build_planning_queue_refresh_prompt(
            Some("latest user"),
            "latest reply",
            Some(&PlanningTaskHandoff {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                task_id: "task-1".to_string(),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                task_title: "Task 1".to_string(),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                direction_id: "direction-a".to_string(),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                combined_priority: 10,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                updated_at: "2026-04-29T00:00:00Z".to_string(),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
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

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn queue_idle_prompt_renders_evaluator_policy() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let authority_context = PlanningWorkerAuthorityPromptContext {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            status_lines: vec![
                "source_of_truth=accepted DB direction authority, accepted DB task authority, and DB queue projection below".to_string(),
                "direction_revision=7".to_string(),
                "task_revision=8".to_string(),
            ],
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            direction_authority_json: Some(
                "{\"version\":1,\"directions\":[{\"id\":\"direction-a\",\"success_criteria\":[\"validated\"]}]}".to_string(),
            ),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            task_authority_json: Some("{\"version\":1,\"tasks\":[]}".to_string()),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            queue_projection_json: Some(
                "{\"next_task\":null,\"active_tasks\":[],\"proposed_tasks\":[],\"skipped_tasks\":[]}"
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .to_string(),
            ),
        };

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn official_completion_prompt_keeps_parallel_worktree_out_of_tool_workspace() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let authority_context = PlanningWorkerAuthorityPromptContext {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            status_lines: vec![
                "source_of_truth=accepted DB direction authority, accepted DB task authority, and DB queue projection below".to_string(),
                "direction_revision=7".to_string(),
                "task_revision=8".to_string(),
            ],
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            direction_authority_json: Some("{\"version\":1,\"directions\":[]}".to_string()),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            task_authority_json: Some("{\"version\":1,\"tasks\":[]}".to_string()),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            queue_projection_json: Some(
                "{\"next_task\":null,\"active_tasks\":[],\"proposed_tasks\":[],\"skipped_tasks\":[]}"
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .to_string(),
            ),
        };
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let contract = PlanningOfficialCompletionRefreshContract::new(
            "turn-1",
            11,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
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

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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
    }
}

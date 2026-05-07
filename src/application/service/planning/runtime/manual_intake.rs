use crate::application::service::planning::runtime::facade::{
    PlanningRuntimeFacadeService, PlanningTaskHandoff,
};
use crate::application::service::planning::runtime::intake::{
    PlanningTaskIntakeRequest, PlanningTaskIntakeService,
};
use crate::diagnostics::event_log;
use crate::domain::planning::{
    OriginSessionKind, PriorityQueueTask, TaskDefinition, TaskMutationProvenance,
};
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq)]
// manual prompt hidden intake가 소비하는 원본 입력이다. main-session prompt를 만들기 전에 task authority 여부를 결정한다.
pub struct ManualPromptIntakeRequest {
    pub workspace_directory: String,
    pub raw_prompt: String,
    // Legacy task lookup key. Provider-neutral audit identity lives in `provenance`.
    pub legacy_source_turn_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub parent_turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// main-session에 넘길 최종 실행 입력이다. visible transcript에는 `transcript_text`만 남기고, prompt는 app-server로만 간다.
pub struct ManualPromptMainHandoff {
    pub prompt: String,
    pub transcript_text: String,
    pub task: Option<PlanningTaskHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// manual intake outcome은 task authority 변경 여부를 명시한다. Failed도 값으로 내려 TUI가 main turn 시작을 막을 수 있다.
pub enum ManualPromptIntakeOutcome {
    NoTaskNeeded(ManualPromptMainHandoff),
    TaskCommitted {
        committed_task_id: String,
        committed_planning_revision: i64,
        handoff: ManualPromptMainHandoff,
    },
    TaskUpdated {
        updated_task_id: String,
        committed_planning_revision: i64,
        handoff: ManualPromptMainHandoff,
    },
    Rejected {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone)]
/*
 * ManualPromptIntakeService는 수동 prompt와 main-session 사이의 hidden preflight다.
 * task 생성/queue 선택은 여기서 끝내고, main-session에는 작은 handoff 또는 즉답 prompt만 전달한다.
 */
pub struct ManualPromptIntakeService {
    task_intake: PlanningTaskIntakeService,
    runtime_facade: PlanningRuntimeFacadeService,
}

impl ManualPromptIntakeService {
    pub fn new(
        task_intake: PlanningTaskIntakeService,
        runtime_facade: PlanningRuntimeFacadeService,
    ) -> Self {
        Self {
            task_intake,
            runtime_facade,
        }
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub fn prepare_manual_turn(
        &self,
        request: ManualPromptIntakeRequest,
    ) -> ManualPromptIntakeOutcome {
        let transcript_text = request.raw_prompt.trim().to_string();
        if transcript_text.is_empty() {
            return ManualPromptIntakeOutcome::Rejected {
                reason: "manual prompt is empty".to_string(),
            };
        }

        event_log::emit_lazy("manual_intake_started", || {
            json!({
                "workspace_directory": &request.workspace_directory,
                "prompt_chars": transcript_text.chars().count(),
            })
        });

        if prompt_can_use_direct_response(&transcript_text) {
            return match self.runtime_facade.build_manual_prompt(&transcript_text) {
                Some(prompt) => {
                    event_log::emit_lazy("manual_intake_no_task_needed", || {
                        json!({
                            "workspace_directory": &request.workspace_directory,
                            "prompt_chars": transcript_text.chars().count(),
                        })
                    });
                    ManualPromptIntakeOutcome::NoTaskNeeded(ManualPromptMainHandoff {
                        prompt,
                        transcript_text,
                        task: None,
                    })
                }
                None => ManualPromptIntakeOutcome::Rejected {
                    reason: "manual prompt is empty".to_string(),
                },
            };
        }

        let outcome = self.commit_prompt_as_task(&request, &transcript_text);
        match &outcome {
            ManualPromptIntakeOutcome::TaskCommitted {
                committed_task_id,
                committed_planning_revision,
                handoff,
            } => event_log::emit_lazy("manual_intake_committed", || {
                json!({
                    "workspace_directory": &request.workspace_directory,
                    "task_id": committed_task_id,
                    "planning_revision": committed_planning_revision,
                    "handoff_task_id": handoff.task.as_ref().map(|task| task.task_id.as_str()),
                })
            }),
            ManualPromptIntakeOutcome::Failed { reason } => {
                event_log::emit_lazy("manual_intake_failed", || {
                    json!({
                        "workspace_directory": &request.workspace_directory,
                        "reason": reason,
                    })
                });
            }
            _ => {}
        }
        outcome
    }

    fn commit_prompt_as_task(
        &self,
        request: &ManualPromptIntakeRequest,
        transcript_text: &str,
    ) -> ManualPromptIntakeOutcome {
        let proposal = match self
            .task_intake
            .prepare_task_intake(PlanningTaskIntakeRequest {
                workspace_directory: request.workspace_directory.clone(),
                raw_prompt: transcript_text.to_string(),
                legacy_source_turn_id: request.legacy_source_turn_id.clone(),
                provenance: TaskMutationProvenance::new(OriginSessionKind::ManualIntake)
                    .with_parent(
                        request.parent_thread_id.clone(),
                        request.parent_turn_id.clone(),
                    ),
                requested_direction_id: None,
                observed_planning_revision: None,
            }) {
            Ok(proposal) => proposal,
            Err(error) => {
                return ManualPromptIntakeOutcome::Failed {
                    reason: format!("manual intake prepare failed: {error}"),
                };
            }
        };
        let commit = match self.task_intake.commit_task_intake(&proposal) {
            Ok(commit) => commit,
            Err(error) => {
                return ManualPromptIntakeOutcome::Failed {
                    reason: format!("manual intake commit failed: {error}"),
                };
            }
        };
        let handoff = self.runtime_facade.build_manual_intake_task_handoff(
            &proposal.draft.task,
            &proposal.draft.direction_title,
            commit.queue_head.as_ref(),
            transcript_text,
        );
        ManualPromptIntakeOutcome::TaskCommitted {
            committed_task_id: commit.committed_task_id,
            committed_planning_revision: commit.committed_planning_revision,
            handoff: ManualPromptMainHandoff {
                prompt: handoff.prompt,
                transcript_text: transcript_text.to_string(),
                task: Some(handoff.task),
            },
        }
    }
}

fn prompt_can_use_direct_response(prompt: &str) -> bool {
    let normalized = prompt.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return true;
    }
    let compact = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let has_action_intent = [
        "implement",
        "fix",
        "add",
        "change",
        "create",
        "update",
        "delete",
        "write",
        "build",
        "test",
        "refactor",
        "review",
        "구현",
        "수정",
        "추가",
        "변경",
        "삭제",
        "작성",
        "테스트",
        "리뷰",
    ]
    .iter()
    .any(|keyword| compact.contains(keyword));
    if has_action_intent {
        return false;
    }
    if matches!(
        compact.as_str(),
        "hi" | "hello"
            | "hey"
            | "안녕"
            | "안녕하세요"
            | "thanks"
            | "thank you"
            | "고마워"
            | "감사합니다"
    ) {
        return true;
    }
    let question_prefix = [
        "what ",
        "how ",
        "why ",
        "when ",
        "where ",
        "who ",
        "is ",
        "are ",
        "can ",
        "does ",
        "do ",
        "뭐",
        "무엇",
        "어떻게",
        "왜",
        "언제",
        "어디",
        "누구",
    ]
    .iter()
    .any(|prefix| compact.starts_with(prefix));
    question_prefix || compact.ends_with('?')
}

pub(super) fn manual_intake_handoff_from_task(
    task: &TaskDefinition,
    _direction_title: &str,
) -> PlanningTaskHandoff {
    PlanningTaskHandoff {
        task_id: task.id.trim().to_string(),
        task_title: task.title.trim().to_string(),
        direction_id: task.direction_id.trim().to_string(),
        combined_priority: task.combined_priority(),
        updated_at: task.updated_at.trim().to_string(),
        status_label: task.status.label().to_string(),
    }
}

pub(super) fn manual_intake_handoff_from_queue_task(
    task: &PriorityQueueTask,
) -> PlanningTaskHandoff {
    PlanningTaskHandoff {
        task_id: task.task_id.trim().to_string(),
        task_title: task.task_title.trim().to_string(),
        direction_id: task.direction_id.trim().to_string(),
        combined_priority: task.combined_priority,
        updated_at: task.updated_at.trim().to_string(),
        status_label: task.status.label().to_string(),
    }
}

pub(super) fn manual_intake_task_prompt(
    task: &TaskDefinition,
    direction_title: &str,
    original_prompt: &str,
) -> String {
    crate::application::service::prompt_component::PromptDocument::builder(
        "manual-intake-task-handoff",
    )
    .lines(
        "task",
        vec![
            "intent=Execute the task prepared by hidden ManualPromptIntake.".to_string(),
            format!("task_id={}", task.id.trim()),
            format!("title={}", task.title.trim()),
            format!("direction={}", direction_title.trim()),
            format!("direction_id={}", task.direction_id.trim()),
            format!("status={}", task.status.label()),
            format!("combined_priority={}", task.combined_priority()),
        ],
    )
    .text("description", &task.description)
    .text("original-user-prompt", original_prompt)
    .bullets(
        "rules",
        vec![
            "Perform only this handoff and the user's concrete feedback.".to_string(),
            "Do not mutate task authority or planning queue state directly.".to_string(),
            "When finished, summarize what changed and any follow-up suggestion.".to_string(),
        ],
    )
    .build()
    .render()
}

#[cfg(test)]
mod tests {
    use super::prompt_can_use_direct_response;

    #[test]
    fn direct_response_classifier_keeps_greetings_and_questions_out_of_task_authority() {
        assert!(prompt_can_use_direct_response("안녕하세요"));
        assert!(prompt_can_use_direct_response("How does the queue work?"));
        assert!(!prompt_can_use_direct_response(
            "fix the manual prompt boundary"
        ));
        assert!(!prompt_can_use_direct_response("hidden intake를 구현해줘"));
    }
}

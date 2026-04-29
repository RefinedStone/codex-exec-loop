use crate::application::service::planning::repair::reconciliation::{
    PlanningExecutionSnapshot, PlanningReconciliationResult, PlanningReconciliationService,
};
use crate::application::service::planning::runtime::policy::{
    PlanningAutoFollowBlockReason, PlanningAutoFollowPolicyDecision, PlanningAutoFollowPromptMode,
    PlanningRuntimePolicyService,
};
use crate::application::service::planning::runtime::prompt::{
    PlanningPromptService, PlanningRuntimeSnapshot,
};
use crate::application::service::planning::shared::auto_follow_copy::BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT;
use crate::application::service::prompt_component::PromptDocument;
use crate::application::service::turn_prompt_assembly_service::{
    ManualPromptAssemblyRequest, TurnPromptAssemblyService,
};
use crate::domain::planning::PriorityQueueTask;
use anyhow::Result;

pub use crate::application::service::planning::runtime::policy::{
    PlanningRuntimeRepairAttempt, PlanningRuntimeStatusProjection,
    PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
    PlanningRuntimeSummaryRequest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeAutoFollowRequest<'a> {
    pub stop_keyword: &'a str,
    pub last_message: &'a str,
    pub snapshot: &'a PlanningRuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimePreviewRequest<'a> {
    pub stop_keyword: &'a str,
    pub last_message: Option<&'a str>,
    pub snapshot: &'a PlanningRuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningRuntimeAutoFollowDecision {
    QueuePrompt(PlanningRuntimeQueuedAutoFollowPrompt),
    Blocked(PlanningAutoFollowBlockReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeQueuedAutoFollowPrompt {
    pub prompt: String,
    pub transcript_text: String,
    pub handoff_task: Option<PlanningTaskHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeRenderedPreview {
    pub rendered_prompt: String,
    pub planning_status_line: String,
    pub planning_detail_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningMainSessionHandoff {
    pub prompt: String,
    pub transcript_text: String,
    pub task: PlanningTaskHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskHandoff {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub combined_priority: i32,
    pub updated_at: String,
    pub status_label: String,
}

#[derive(Clone)]
pub struct PlanningRuntimeFacadeService {
    planning_prompt_service: PlanningPromptService,
    planning_reconciliation_service: PlanningReconciliationService,
    planning_runtime_policy_service: PlanningRuntimePolicyService,
    turn_prompt_assembly_service: TurnPromptAssemblyService,
}

impl PlanningRuntimeFacadeService {
    pub fn new(
        planning_prompt_service: PlanningPromptService,
        planning_reconciliation_service: PlanningReconciliationService,
        planning_runtime_policy_service: PlanningRuntimePolicyService,
        turn_prompt_assembly_service: TurnPromptAssemblyService,
    ) -> Self {
        Self {
            planning_prompt_service,
            planning_reconciliation_service,
            planning_runtime_policy_service,
            turn_prompt_assembly_service,
        }
    }

    pub fn load_runtime_snapshot_or_invalid(
        &self,
        workspace_directory: &str,
    ) -> PlanningRuntimeSnapshot {
        self.planning_prompt_service
            .load_runtime_snapshot(workspace_directory)
            .unwrap_or_else(|error| {
                PlanningRuntimeSnapshot::invalid(format!(
                    "failed to load planning workspace: {error}"
                ))
            })
    }

    pub fn build_manual_prompt(
        &self,
        operator_prompt: &str,
        _snapshot: &PlanningRuntimeSnapshot,
    ) -> Option<String> {
        self.turn_prompt_assembly_service
            .build_manual_prompt(ManualPromptAssemblyRequest {
                operator_prompt,
                planning_prompt_fragment: None,
            })
    }

    pub fn build_builtin_next_task_handoff(
        &self,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> Option<PlanningMainSessionHandoff> {
        let queue_head = snapshot.queue_head()?;
        Some(PlanningMainSessionHandoff {
            prompt: render_builtin_next_task_handoff_prompt(queue_head),
            transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
            task: PlanningTaskHandoff {
                task_id: queue_head.task_id.trim().to_string(),
                task_title: queue_head.task_title.trim().to_string(),
                direction_id: queue_head.direction_id.trim().to_string(),
                combined_priority: queue_head.combined_priority,
                updated_at: queue_head.updated_at.trim().to_string(),
                status_label: queue_head.status.label().to_string(),
            },
        })
    }

    pub fn builtin_next_task_preview_prompt(&self, snapshot: &PlanningRuntimeSnapshot) -> String {
        self.build_builtin_next_task_handoff(snapshot)
            .map(|handoff| handoff.prompt)
            .unwrap_or_else(|| {
                match snapshot.queue_idle_policy() {
                    crate::domain::planning::QueueIdlePolicy::Stop => {
                        "The current planning queue has no actionable head and queue-idle policy is stop, so internal continuation will end after the current turn.".to_string()
                    }
                    crate::domain::planning::QueueIdlePolicy::ReviewAndEnqueue => {
                        "A queue-manager planning worker reviews the direction goals after the current turn and re-enqueues follow-up work only when a justified actionable task exists.".to_string()
                    }
                }
            })
    }

    pub fn decide_auto_followup(
        &self,
        request: PlanningRuntimeAutoFollowRequest<'_>,
    ) -> PlanningRuntimeAutoFollowDecision {
        match self
            .planning_runtime_policy_service
            .decide_auto_follow(request.snapshot)
        {
            PlanningAutoFollowPolicyDecision::Blocked(block_reason) => {
                PlanningRuntimeAutoFollowDecision::Blocked(block_reason)
            }
            PlanningAutoFollowPolicyDecision::QueuePrompt(
                PlanningAutoFollowPromptMode::ContinueQueuedTask,
            ) => match self.build_builtin_next_task_handoff(request.snapshot) {
                Some(handoff) => PlanningRuntimeAutoFollowDecision::QueuePrompt(
                    PlanningRuntimeQueuedAutoFollowPrompt {
                        prompt: handoff.prompt,
                        transcript_text: handoff.transcript_text,
                        handoff_task: Some(handoff.task),
                    },
                ),
                None => PlanningRuntimeAutoFollowDecision::Blocked(
                    PlanningAutoFollowBlockReason::ActionableQueueRequired,
                ),
            },
        }
    }

    pub fn build_auto_follow_preview(
        &self,
        request: PlanningRuntimePreviewRequest<'_>,
    ) -> PlanningRuntimeRenderedPreview {
        let policy_decision = self
            .planning_runtime_policy_service
            .decide_auto_follow(request.snapshot);
        let planning_view = self
            .planning_runtime_policy_service
            .build_preview_view_for_decision(policy_decision, request.snapshot);
        let rendered_prompt = self.builtin_next_task_preview_prompt(request.snapshot);
        PlanningRuntimeRenderedPreview {
            rendered_prompt,
            planning_status_line: format!("planning: {}", planning_view.status_label),
            planning_detail_line: planning_view
                .detail
                .map(|detail| format!("planning detail: {detail}")),
        }
    }

    pub fn build_summary_line(
        &self,
        request: PlanningRuntimeSummaryLineRequest<'_>,
    ) -> Option<String> {
        self.planning_runtime_policy_service
            .build_summary_line(request)
    }

    pub fn build_followup_status_projection(
        &self,
        request: PlanningRuntimeStatusProjectionRequest<'_>,
    ) -> PlanningRuntimeStatusProjection {
        self.planning_runtime_policy_service
            .build_status_projection(request)
    }

    pub fn load_execution_snapshot(
        &self,
        workspace_directory: &str,
    ) -> Result<PlanningExecutionSnapshot> {
        self.planning_reconciliation_service
            .load_execution_snapshot(workspace_directory)
    }

    pub fn reconcile_after_turn(
        &self,
        workspace_directory: &str,
        turn_id: &str,
        changed_planning_file_paths: &[String],
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> Result<PlanningReconciliationResult> {
        self.planning_reconciliation_service.reconcile_after_turn(
            workspace_directory,
            turn_id,
            changed_planning_file_paths,
            execution_snapshot,
        )
    }

    pub fn commit_task_authority_candidate(
        &self,
        workspace_directory: &str,
        candidate_task_authority_json: &str,
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> Result<PlanningReconciliationResult> {
        self.planning_reconciliation_service
            .commit_task_authority_candidate(
                workspace_directory,
                candidate_task_authority_json,
                execution_snapshot,
            )
    }
}

fn render_builtin_next_task_handoff_prompt(queue_head: &PriorityQueueTask) -> String {
    let rank_reason = queue_head
        .rank_reasons
        .iter()
        .find(|reason| !reason.trim().is_empty())
        .map(String::as_str)
        .unwrap_or("this is the highest-priority actionable task");

    PromptDocument::builder("queued-task-handoff")
        .lines(
            "task",
            vec![
                "intent=Continue the next highest-priority task.".to_string(),
                format!("title={}", queue_head.task_title.trim()),
                format!("direction={}", queue_head.direction_title.trim()),
                format!("rank={}", queue_head.rank),
                format!("combined_priority={}", queue_head.combined_priority),
                format!("why_now={}", rank_reason.trim()),
            ],
        )
        .bullets(
            "rules",
            vec![
                "Work from the current repository state and focus only on this task.".to_string(),
                "Treat `.codex-exec-loop/planning` and planning control files as internal runtime state unless the user explicitly requested planning maintenance or the task strictly requires it."
                    .to_string(),
                "Do not describe planning queue refresh logic in commentary or final answer."
                    .to_string(),
                "When finished, summarize what changed and what remains.".to_string(),
            ],
        )
        .build()
        .render()
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_compiles_after_task_authority_file_removal() {
        assert!(std::env::current_dir().is_ok());
    }
}

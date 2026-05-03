/*
 * Runtime facade is the application-layer boundary used by TUI and app-server turn execution.
 * It does not reimplement planning rules; it sequences snapshot loading, auto-follow policy decisions,
 * prompt assembly, and post-turn reconciliation into return shapes that inbound adapters can consume without
 * knowing the internal service graph.
 */
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
    MainSessionPromptAssemblyRequest, ManualPromptAssemblyRequest, SubSessionPromptAssemblyRequest,
    TurnPromptAssemblyService,
};
use crate::domain::planning::PriorityQueueTask;
use anyhow::Result;

// Re-export policy view models through the facade so callers keep one runtime import surface.
pub use crate::application::service::planning::runtime::policy::{
    PlanningRuntimeRepairAttempt, PlanningRuntimeStatusProjection,
    PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
    PlanningRuntimeSummaryRequest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
// Auto-follow decisions need the last message and stop keyword for policy, plus a preloaded snapshot for speed.
pub struct PlanningRuntimeAutoFollowRequest<'a> {
    pub stop_keyword: &'a str,
    pub last_message: &'a str,
    pub snapshot: &'a PlanningRuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Preview rendering is read-only and may run with no last message when the UI only needs planning status.
pub struct PlanningRuntimePreviewRequest<'a> {
    pub stop_keyword: &'a str,
    pub last_message: Option<&'a str>,
    pub snapshot: &'a PlanningRuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Policy output after facade materializes any required queued-task prompt.
pub enum PlanningRuntimeAutoFollowDecision {
    QueuePrompt(PlanningRuntimeQueuedAutoFollowPrompt),
    Blocked(PlanningAutoFollowBlockReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Queued auto-follow contains both executable prompt text and transcript metadata for the visible session.
pub struct PlanningRuntimeQueuedAutoFollowPrompt {
    pub prompt: String,
    pub transcript_text: String,
    pub handoff_task: Option<PlanningTaskHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Preview bundle pairs the actual prompt preview with policy-derived status copy.
pub struct PlanningRuntimeRenderedPreview {
    pub rendered_prompt: String,
    pub planning_status_line: String,
    pub planning_detail_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Main-session handoff is appended to the operator-visible conversation and therefore carries transcript copy.
pub struct PlanningMainSessionHandoff {
    pub prompt: String,
    pub transcript_text: String,
    pub task: PlanningTaskHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Sub-session handoff starts hidden work; it needs a prompt and task identity but no visible transcript marker.
pub struct PlanningSubSessionHandoff {
    pub prompt: String,
    pub task: PlanningTaskHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Compact task metadata shared by auto-follow, UI status, and repair handoff code.
pub struct PlanningTaskHandoff {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub combined_priority: i32,
    pub updated_at: String,
    pub status_label: String,
}

#[derive(Clone)]
/*
 * Facade owns the ordering between runtime services.
 * Prompt service creates immutable snapshots, policy service turns snapshots into status/follow decisions,
 * turn prompt assembly wraps task instructions into main/sub-session prompts, and reconciliation restores
 * protected planning state after a turn.
 */
pub struct PlanningRuntimeFacadeService {
    planning_prompt_service: PlanningPromptService,
    planning_reconciliation_service: PlanningReconciliationService,
    planning_runtime_policy_service: PlanningRuntimePolicyService,
    turn_prompt_assembly_service: TurnPromptAssemblyService,
}
impl PlanningRuntimeFacadeService {
    // Composition injects concrete services once, keeping adapters away from runtime wiring details.
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

    /*
     * TUI rendering must not panic when planning files fail to load.
     * Collapsing loader errors into an invalid snapshot lets the same policy/status path display a blocked
     * planning state and failure reason.
     */
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

    // Manual prompts are user-authored prompts plus the current planning fragment, if the snapshot is ready.
    pub fn build_manual_prompt(
        &self,
        operator_prompt: &str,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> Option<String> {
        self.turn_prompt_assembly_service
            .build_manual_prompt(ManualPromptAssemblyRequest {
                operator_prompt,
                planning_prompt_fragment: snapshot.prompt_fragment(),
            })
    }

    /*
     * Convert the current queue head into a main-session prompt.
     * A missing queue head returns None instead of fabricating work; policy maps that condition to an actionable
     * queue-required block when auto-follow was otherwise allowed.
     */
    pub fn build_builtin_next_task_handoff(
        &self,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> Option<PlanningMainSessionHandoff> {
        let queue_head = snapshot.queue_head()?;
        Some(self.build_task_handoff_with_planning_fragment(queue_head, snapshot.prompt_fragment()))
    }

    // Public helper for callers that already have a task and do not need to include a planning fragment.
    pub fn build_task_handoff(&self, task: &PriorityQueueTask) -> PlanningMainSessionHandoff {
        self.build_task_handoff_with_planning_fragment(task, None)
    }

    /*
     * Hidden sub-sessions receive only the queued task handoff prompt.
     * They intentionally omit the planning prompt fragment because orchestration-specific workers render their
     * own authority context through worker prompt builders.
     */
    pub fn build_sub_session_task_handoff(
        &self,
        task: &PriorityQueueTask,
    ) -> PlanningSubSessionHandoff {
        let task_prompt = render_builtin_next_task_handoff_prompt(task);
        let prompt = self
            .turn_prompt_assembly_service
            .build_sub_session_prompt(SubSessionPromptAssemblyRequest {
                handoff_prompt: &task_prompt,
            })
            .expect("queued sub session handoff prompt should not be empty");

        PlanningSubSessionHandoff {
            prompt,
            task: planning_task_handoff_from_queue_task(task),
        }
    }

    /*
     * Main session handoff is visible to the operator and can include the current planning fragment.
     * The transcript marker records that the runtime queued a built-in continuation without leaking the full
     * internal queue prompt into chat history.
     */
    fn build_task_handoff_with_planning_fragment(
        &self,
        task: &PriorityQueueTask,
        planning_prompt_fragment: Option<&str>,
    ) -> PlanningMainSessionHandoff {
        let task_prompt = render_builtin_next_task_handoff_prompt(task);
        let prompt = self
            .turn_prompt_assembly_service
            .build_main_session_prompt(MainSessionPromptAssemblyRequest {
                user_prompt: &task_prompt,
                planning_prompt_fragment,
            })
            .expect("queued task handoff prompt should not be empty");

        PlanningMainSessionHandoff {
            prompt,
            transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
            task: planning_task_handoff_from_queue_task(task),
        }
    }

    // Preview uses the same prompt builder as execution, with queue-idle explanatory copy when no task exists.
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

    /*
     * Decide auto-follow and materialize the executable prompt only for the allowed queued-task mode.
     * The extra queue-head check protects against stale snapshots or future policy changes that claim work is
     * actionable without providing a concrete task.
     */
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

    // Build the read-only prompt/status preview shown before auto-follow is submitted.
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

    // Summary/status helpers are delegated so facade callers do not import policy service directly.
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

    // Capture protected planning files before a turn so reconciliation can restore them afterward if needed.
    pub fn load_execution_snapshot(
        &self,
        workspace_directory: &str,
    ) -> Result<PlanningExecutionSnapshot> {
        self.planning_reconciliation_service
            .load_execution_snapshot(workspace_directory)
    }

    // Reconciliation remains behind the facade because adapters only know changed paths and the pre-turn snapshot.
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
}

// Normalize queue task fields once before sharing handoff metadata with UI/reconciliation code.
fn planning_task_handoff_from_queue_task(task: &PriorityQueueTask) -> PlanningTaskHandoff {
    PlanningTaskHandoff {
        task_id: task.task_id.trim().to_string(),
        task_title: task.task_title.trim().to_string(),
        direction_id: task.direction_id.trim().to_string(),
        combined_priority: task.combined_priority,
        updated_at: task.updated_at.trim().to_string(),
        status_label: task.status.label().to_string(),
    }
}

/*
 * Render a domain queue task into the instruction document sent to Codex.
 * The task section explains what to continue and why it is first in the queue; the rules section keeps the
 * worker focused on repository work instead of exposing planning queue maintenance unless explicitly requested.
 */
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

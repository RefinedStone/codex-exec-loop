use crate::application::service::planning::{
    PlanningRuntimeProjection, PlanningTurnExecutionSnapshotCapture,
};
use crate::application::service::post_turn_evaluation::{
    PostTurnAutoFollowSkipReason,
    PostTurnContinuationAction as ApplicationPostTurnContinuationAction, PostTurnEvaluationContext,
    PostTurnEvaluationExecution, PostTurnEvaluationOutcome as ApplicationPostTurnEvaluationOutcome,
    PostTurnEvaluationProvenance as ApplicationPostTurnEvaluationProvenance,
};
use crate::core::app::{AppCommand, CoreInput, PostTurnEvaluationCorrelation};

use super::super::conversation_model::PlanningRepairState;
use super::super::conversation_runtime::{
    PostTurnContinuationAction, PostTurnEvaluationOutcome, PostTurnEvaluationProvenance,
    PostTurnQueuedPrompt,
};
use super::super::post_turn_continuation::PostTurnEvaluationCompletionPayload;
use super::super::{
    AutoFollowSkipReason, AutoFollowSnapshotPresentation, ConversationState, ConversationViewModel,
    NativeTuiApp,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PostTurnEvaluationRequest {
    pub workspace_directory: String,
    pub completed_turn_id: String,
    pub changed_planning_file_paths: Vec<String>,
    pub execution_snapshot_capture: Option<PlanningTurnExecutionSnapshotCapture>,
}

impl NativeTuiApp {
    pub(super) fn execute_post_turn_evaluation(&mut self, request: PostTurnEvaluationRequest) {
        let Some(context) = self.ready_post_turn_evaluation_context() else {
            return;
        };
        let request = application_post_turn_request(request, context);
        self.dispatch_client_event(CoreInput::Command(AppCommand::EvaluatePostTurn(Box::new(
            request,
        ))));
    }

    fn ready_post_turn_evaluation_context(&self) -> Option<PostTurnEvaluationContext> {
        let current_runtime_projection = self.planning_runtime_projection_snapshot();
        let planning_workspace_directory = self.planning_workspace_directory();
        let parallel_automation_epoch_id = self
            .runtime
            .client_runtime
            .current_parallel_epoch_id_for_workspace(&planning_workspace_directory);
        match &self.conversation.lifecycle.conversation_state {
            ConversationState::Ready(conversation) => Some(post_turn_context_from_conversation(
                conversation.as_ref(),
                &planning_workspace_directory,
                current_runtime_projection,
                self.parallel_mode_enabled(),
                parallel_automation_epoch_id,
            )),
            ConversationState::Loading | ConversationState::Failed(_) => None,
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn apply_post_turn_evaluation_execution(
        &mut self,
        correlation: PostTurnEvaluationCorrelation,
        execution: PostTurnEvaluationExecution,
        route_resolution: crate::core::app::PostTurnRouteResolution,
    ) {
        let workspace_directory = execution.runtime_projection_workspace_directory;
        self.apply_post_turn_evaluation_completion_payload(PostTurnEvaluationCompletionPayload {
            correlation,
            evaluation: Box::new(tui_post_turn_evaluation_outcome(
                execution.evaluation,
                workspace_directory,
            )),
            planning_worker_panel_state: execution.planning_worker_panel_state,
            route_resolution,
        });
    }
}

fn application_post_turn_request(
    request: PostTurnEvaluationRequest,
    context: PostTurnEvaluationContext,
) -> crate::application::service::post_turn_evaluation::PostTurnEvaluationRequest {
    crate::application::service::post_turn_evaluation::PostTurnEvaluationRequest {
        context,
        workspace_directory: request.workspace_directory,
        completed_turn_id: request.completed_turn_id,
        changed_planning_file_paths: request.changed_planning_file_paths,
        execution_snapshot_capture: request.execution_snapshot_capture,
        // Historical panel detail is Core-owned. This placeholder only keeps
        // the application request contract stable until Core admits the exact
        // lifecycle and replaces it with its accepted history snapshot.
        planning_worker_panel_state: Default::default(),
        // Core replaces this compatibility placeholder with its own captured
        // permit during admission. No continuation gate is owned by the TUI.
        continuation_permit: crate::domain::planning::PostTurnContinuationGate::default().capture(),
    }
}

fn post_turn_context_from_conversation(
    conversation: &ConversationViewModel,
    planning_workspace_directory: &str,
    current_runtime_projection: PlanningRuntimeProjection,
    parallel_mode_enabled: bool,
    parallel_automation_epoch_id: Option<u64>,
) -> PostTurnEvaluationContext {
    let latest_main_reply = conversation.latest_agent_message_text().map(str::to_string);

    PostTurnEvaluationContext {
        thread_id: conversation.thread_id.clone(),
        planning_workspace_directory: planning_workspace_directory.to_string(),
        latest_user_message: conversation.latest_user_message_text().map(str::to_string),
        latest_main_reply,
        // Core replaces this compatibility placeholder from the exact
        // ConversationRuntimeSnapshot before admitting the worker.
        previous_handoff_task: None,
        current_runtime_projection,
        // These policy-shaped fields are compatibility placeholders only.
        // Core replaces every one from ConversationRuntimeAuthority before it
        // admits an evaluation effect, so the TUI cannot become a second
        // auto-follow/post-turn policy writer.
        parallel_mode_enabled,
        parallel_automation_epoch_id,
        planning_settlement_paused: false,
        continuation_paused: false,
        can_queue_next: false,
        stop_keyword: String::new(),
        stop_keyword_matched: false,
        no_file_changes_stop_matched: false,
        mode_label: conversation.auto_follow_state().mode_label().to_string(),
    }
}

fn tui_post_turn_evaluation_outcome(
    outcome: ApplicationPostTurnEvaluationOutcome,
    workspace_directory: String,
) -> PostTurnEvaluationOutcome {
    let has_actionable_queue_head = outcome.runtime_projection.has_actionable_queue_head();
    PostTurnEvaluationOutcome {
        provenance: tui_post_turn_evaluation_provenance(outcome.provenance)
            .with_runtime_projection_routing(workspace_directory, has_actionable_queue_head),
        planning_repair_state: outcome
            .planning_repair_state
            .map(|state| PlanningRepairState {
                attempts_used: state.attempts_used,
                max_attempts: state.max_attempts,
                latest_request: state.latest_request,
            }),
        runtime_notices: outcome.runtime_notices,
        action: tui_post_turn_action(outcome.action),
        operator_alerts: outcome.operator_alerts,
    }
}

fn tui_post_turn_evaluation_provenance(
    provenance: ApplicationPostTurnEvaluationProvenance,
) -> PostTurnEvaluationProvenance {
    PostTurnEvaluationProvenance::new(provenance.completed_turn_id)
        .with_handoff_task(provenance.handoff_task)
        .with_parallel_queue_signal(provenance.parallel_queue_signal)
        .with_queue_mutation_receipt(provenance.queue_mutation_receipt)
}

fn tui_post_turn_action(
    action: ApplicationPostTurnContinuationAction,
) -> PostTurnContinuationAction {
    match action {
        ApplicationPostTurnContinuationAction::QueueAutoPrompt(prompt) => {
            PostTurnContinuationAction::QueueAutoPrompt(Box::new(PostTurnQueuedPrompt {
                prompt: prompt.prompt,
                mode_label: prompt.mode_label,
                transcript_text: prompt.transcript_text,
            }))
        }
        ApplicationPostTurnContinuationAction::SkipAutoFollow { reason } => {
            PostTurnContinuationAction::SkipAutoFollow {
                reason: tui_auto_follow_skip_reason(reason),
            }
        }
    }
}

fn tui_auto_follow_skip_reason(reason: PostTurnAutoFollowSkipReason) -> AutoFollowSkipReason {
    match reason {
        PostTurnAutoFollowSkipReason::PostTurnContinuationPaused => {
            AutoFollowSkipReason::PostTurnContinuationPaused
        }
        PostTurnAutoFollowSkipReason::LimitReached => AutoFollowSkipReason::LimitReached,
        PostTurnAutoFollowSkipReason::NoAgentReply => AutoFollowSkipReason::NoAgentReply,
        PostTurnAutoFollowSkipReason::StopKeywordMatched => {
            AutoFollowSkipReason::StopKeywordMatched
        }
        PostTurnAutoFollowSkipReason::NoFileChanges => AutoFollowSkipReason::NoFileChanges,
        PostTurnAutoFollowSkipReason::PlanningBlocked => AutoFollowSkipReason::PlanningBlocked,
        PostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop => {
            AutoFollowSkipReason::PlanningQueueIdlePolicyStop
        }
        PostTurnAutoFollowSkipReason::PlanningQueueHeadRequired => {
            AutoFollowSkipReason::PlanningQueueHeadRequired
        }
        PostTurnAutoFollowSkipReason::PlanningQueueDrained => {
            AutoFollowSkipReason::PlanningQueueDrained
        }
        PostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead => {
            AutoFollowSkipReason::PlanningRepeatedQueueHead
        }
        PostTurnAutoFollowSkipReason::ParallelSessionCompleted => {
            AutoFollowSkipReason::ParallelSessionCompleted
        }
        PostTurnAutoFollowSkipReason::PostTurnEvaluationTimedOut => {
            AutoFollowSkipReason::PostTurnEvaluationTimedOut
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update_auto_follow(
        conversation: &mut ConversationViewModel,
        update: impl FnOnce(&mut crate::core::app::AutoFollowAuthoritySnapshot),
    ) {
        let mut runtime = conversation.runtime_snapshot().clone();
        update(&mut runtime.auto_follow);
        conversation.apply_runtime_snapshot(runtime);
    }

    fn request() -> PostTurnEvaluationRequest {
        PostTurnEvaluationRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            completed_turn_id: "turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
            execution_snapshot_capture: None,
        }
    }

    fn assert_neutral_policy_placeholders(context: &PostTurnEvaluationContext) {
        assert!(!context.planning_settlement_paused);
        assert!(!context.continuation_paused);
        assert!(!context.can_queue_next);
        assert!(context.stop_keyword.is_empty());
        assert!(!context.stop_keyword_matched);
        assert!(!context.no_file_changes_stop_matched);
    }

    #[test]
    fn auto_follow_policy_fields_remain_neutral_tui_placeholders() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/workspace".to_string());

        let disabled = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            false,
            None,
        );
        assert_neutral_policy_placeholders(&disabled);

        update_auto_follow(&mut conversation, |auto_follow| {
            auto_follow.max_auto_turns = 3;
            auto_follow.completed_auto_turns = 0;
            auto_follow.continuation_paused = false;
        });
        let enabled = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            false,
            None,
        );
        assert_neutral_policy_placeholders(&enabled);

        update_auto_follow(&mut conversation, |auto_follow| {
            auto_follow.phase = crate::core::app::AutoFollowPhase::Idle;
            auto_follow.completed_auto_turns = 0;
            auto_follow.continuation_paused = true;
        });
        let stopped = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            true,
            None,
        );
        assert_neutral_policy_placeholders(&stopped);

        update_auto_follow(&mut conversation, |auto_follow| {
            auto_follow.max_auto_turns = 3;
            auto_follow.completed_auto_turns = 0;
            auto_follow.continuation_paused = false;
        });
        let rearmed = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            false,
            None,
        );
        assert_neutral_policy_placeholders(&rearmed);
    }

    #[test]
    fn explicit_parallel_mode_projects_only_external_control_plane_facts() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/workspace".to_string());

        let parallel = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            true,
            Some(7),
        );
        assert!(parallel.parallel_mode_enabled);
        assert_eq!(parallel.parallel_automation_epoch_id, Some(7));
        assert_neutral_policy_placeholders(&parallel);
        assert!(!conversation.auto_follow_state().is_enabled());

        update_auto_follow(&mut conversation, |auto_follow| {
            auto_follow.continuation_paused = true;
            auto_follow.parallel_rearmed_after_stop = false;
        });
        let stopped = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            true,
            None,
        );
        assert!(stopped.parallel_mode_enabled);
        assert_eq!(stopped.parallel_automation_epoch_id, None);
        assert_neutral_policy_placeholders(&stopped);

        update_auto_follow(&mut conversation, |auto_follow| {
            auto_follow.parallel_rearmed_after_stop = true;
        });
        let parallel_rearmed = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            true,
            Some(8),
        );
        assert!(parallel_rearmed.parallel_mode_enabled);
        assert_eq!(parallel_rearmed.parallel_automation_epoch_id, Some(8));
        assert_neutral_policy_placeholders(&parallel_rearmed);
        assert!(
            conversation
                .auto_follow_state()
                .post_turn_continuation_paused()
        );
        assert!(!conversation.auto_follow_state().can_queue_next());
    }

    #[test]
    fn post_turn_context_projects_raw_reply_but_not_derived_stop_policy() {
        use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

        let mut conversation = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "work complete.\nAUTO_STOP!",
            None,
            None,
        ));
        update_auto_follow(&mut conversation, |auto_follow| {
            auto_follow.stop_on_no_file_changes = true;
        });

        let default_keyword = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            false,
            None,
        );

        assert_eq!(
            default_keyword.latest_main_reply.as_deref(),
            Some("work complete.\nAUTO_STOP!")
        );
        assert_neutral_policy_placeholders(&default_keyword);

        update_auto_follow(&mut conversation, |auto_follow| {
            auto_follow.stop_keyword = "DONE".to_string();
        });
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "done!",
            None,
            None,
        ));
        conversation
            .turn_activity
            .last_completed_turn_file_change_count = 2;

        let custom_keyword = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            false,
            None,
        );

        assert_eq!(custom_keyword.latest_main_reply.as_deref(), Some("done!"));
        assert_neutral_policy_placeholders(&custom_keyword);
    }

    #[test]
    fn resumed_cross_workspace_turn_preserves_the_conversation_planning_workspace() {
        let mut request = request();
        request.workspace_directory = "/tmp/active-turn-worktree".to_string();
        let mut conversation =
            ConversationViewModel::new_draft("/tmp/shell-draft-workspace".to_string());
        conversation.thread_id = "resumed-thread".to_string();
        conversation.cwd = "/tmp/resumed-session-workspace".to_string();
        conversation.draft_workspace_directory = "/tmp/shell-draft-workspace".to_string();

        let context = post_turn_context_from_conversation(
            &conversation,
            "/tmp/resumed-session-workspace",
            PlanningRuntimeProjection::uninitialized(),
            false,
            None,
        );

        assert_eq!(context.thread_id, "resumed-thread");
        assert_eq!(context.planning_workspace_directory, conversation.cwd);
        assert_ne!(
            context.planning_workspace_directory,
            request.workspace_directory
        );
        assert_ne!(
            context.planning_workspace_directory,
            conversation.draft_workspace_directory
        );
    }
}

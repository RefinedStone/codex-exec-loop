use crate::application::service::planning::{
    PlanningRuntimeProjection, PlanningTurnExecutionSnapshotCapture,
};
use crate::application::service::post_turn_evaluation::{
    PostTurnAutoFollowSkipReason,
    PostTurnContinuationAction as ApplicationPostTurnContinuationAction, PostTurnEvaluationContext,
    PostTurnEvaluationExecution, PostTurnEvaluationOutcome as ApplicationPostTurnEvaluationOutcome,
    PostTurnEvaluationProvenance as ApplicationPostTurnEvaluationProvenance,
};
use crate::core::app::{AppCommand, CoreInput};

use super::super::conversation_model::PlanningRepairState;
use super::super::conversation_runtime::{
    PostTurnContinuationAction, PostTurnEvaluationOutcome, PostTurnEvaluationProvenance,
    PostTurnQueuedPrompt,
};
use super::super::post_turn_continuation::PostTurnEvaluationCompletionPayload;
use super::super::{AutoFollowSkipReason, ConversationState, ConversationViewModel, NativeTuiApp};

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
        let request = application_post_turn_request(
            request,
            context,
            self.planning.post_turn_continuation_gate.capture(),
        );
        self.dispatch_client_event(CoreInput::Command(AppCommand::EvaluatePostTurn(Box::new(
            request,
        ))));
    }

    fn ready_post_turn_evaluation_context(&self) -> Option<PostTurnEvaluationContext> {
        let current_runtime_projection = self.planning_runtime_projection_snapshot();
        let planning_workspace_directory = self.planning_workspace_directory();
        let parallel_automation_epoch_id = self
            .runtime
            .parallel_mode_control_plane
            .current_epoch_id_for_workspace(&planning_workspace_directory);
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
        execution: PostTurnEvaluationExecution,
    ) {
        let workspace_directory = execution.runtime_projection_workspace_directory;
        self.apply_post_turn_evaluation_completion_payload(PostTurnEvaluationCompletionPayload {
            evaluation: Box::new(tui_post_turn_evaluation_outcome(
                execution.evaluation,
                workspace_directory,
            )),
            planning_worker_panel_state: execution.planning_worker_panel_state,
        });
    }
}

fn application_post_turn_request(
    request: PostTurnEvaluationRequest,
    context: PostTurnEvaluationContext,
    continuation_permit: crate::domain::planning::PostTurnContinuationPermit,
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
        continuation_permit,
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
    let stop_keyword_matched = latest_main_reply
        .as_deref()
        .map(|message| {
            conversation
                .auto_follow_state
                .stop_rules
                .stop_keyword
                .matches(message)
        })
        .unwrap_or(false);
    let no_file_changes_stop_matched = conversation
        .auto_follow_state
        .stop_rules
        .should_stop_on_no_file_changes(
            conversation
                .turn_activity
                .last_completed_file_change_count(),
        );

    let operator_stopped = conversation
        .auto_follow_state
        .post_turn_continuation_paused();
    let parallel_continuation_enabled = parallel_mode_enabled
        && conversation
            .auto_follow_state
            .parallel_post_turn_continuation_allowed();

    PostTurnEvaluationContext {
        thread_id: conversation.thread_id.clone(),
        planning_workspace_directory: planning_workspace_directory.to_string(),
        latest_user_message: conversation.latest_user_message_text().map(str::to_string),
        latest_main_reply,
        previous_handoff_task: conversation.last_planning_task_handoff().cloned(),
        current_runtime_projection,
        // Single-session continuation remains off by default. Explicit parallel
        // mode is an independent automation opt-in, while `:stop` remains a
        // sticky kill switch for both paths.
        parallel_mode_enabled,
        parallel_automation_epoch_id,
        planning_settlement_paused: operator_stopped && !parallel_continuation_enabled,
        continuation_paused: (!conversation.auto_follow_state.is_enabled() || operator_stopped)
            && !parallel_continuation_enabled,
        can_queue_next: conversation.auto_follow_state.can_queue_next()
            || parallel_continuation_enabled,
        stop_keyword: conversation
            .auto_follow_state
            .stop_keyword_value()
            .to_string(),
        stop_keyword_matched,
        no_file_changes_stop_matched,
        mode_label: conversation.auto_follow_state.mode_label().to_string(),
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

    fn request() -> PostTurnEvaluationRequest {
        PostTurnEvaluationRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            completed_turn_id: "turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
            execution_snapshot_capture: None,
        }
    }

    #[test]
    fn auto_follow_off_allows_settlement_while_explicit_stop_pauses_it() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/workspace".to_string());

        let disabled = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            false,
            None,
        );
        assert!(disabled.continuation_paused);
        assert!(!disabled.planning_settlement_paused);
        assert!(!disabled.can_queue_next);

        conversation.auto_follow_state.set_max_auto_turns(3);
        let enabled = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            false,
            None,
        );
        assert!(!enabled.continuation_paused);
        assert!(!enabled.planning_settlement_paused);
        assert!(enabled.can_queue_next);

        conversation
            .auto_follow_state
            .pause_post_turn_continuation();
        conversation.auto_follow_state.reset_for_manual_turn();
        let stopped = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            true,
            None,
        );
        assert!(stopped.continuation_paused);
        assert!(stopped.planning_settlement_paused);
        assert!(!stopped.can_queue_next);

        conversation.auto_follow_state.set_max_auto_turns(3);
        let rearmed = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            false,
            None,
        );
        assert!(!rearmed.continuation_paused);
        assert!(!rearmed.planning_settlement_paused);
        assert!(rearmed.can_queue_next);
    }

    #[test]
    fn explicit_parallel_mode_enables_only_parallel_post_turn_continuation() {
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
        assert!(!parallel.continuation_paused);
        assert!(parallel.can_queue_next);
        assert!(!conversation.auto_follow_state.is_enabled());

        conversation
            .auto_follow_state
            .pause_post_turn_continuation();
        let stopped = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            true,
            None,
        );
        assert!(stopped.continuation_paused);
        assert!(!stopped.can_queue_next);

        conversation.rearm_parallel_post_turn_continuation();
        let parallel_rearmed = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            true,
            Some(8),
        );
        assert!(!parallel_rearmed.continuation_paused);
        assert!(parallel_rearmed.can_queue_next);
        assert!(
            conversation
                .auto_follow_state
                .post_turn_continuation_paused()
        );
        assert!(!conversation.auto_follow_state.can_queue_next());
    }

    #[test]
    fn post_turn_context_projects_keyword_and_file_change_stop_rules() {
        use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

        let mut conversation = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "work complete.\nAUTO_STOP!",
            None,
            None,
        ));
        conversation
            .auto_follow_state
            .stop_rules
            .stop_on_no_file_changes = true;

        let default_keyword = post_turn_context_from_conversation(
            &conversation,
            "/tmp/workspace",
            PlanningRuntimeProjection::uninitialized(),
            false,
            None,
        );

        assert_eq!(default_keyword.stop_keyword, "AUTO_STOP");
        assert!(default_keyword.stop_keyword_matched);
        assert!(default_keyword.no_file_changes_stop_matched);

        conversation.auto_follow_state.stop_rules.stop_keyword.value = "DONE".to_string();
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

        assert_eq!(custom_keyword.stop_keyword, "DONE");
        assert!(custom_keyword.stop_keyword_matched);
        assert!(!custom_keyword.no_file_changes_stop_matched);
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

use crate::application::service::post_turn_decision::{
    PostTurnAutoPromptRoute, PostTurnAutoPromptRouteRequest, PostTurnAutoPromptSuppressionReason,
    decide_post_turn_auto_prompt_route,
};
use crate::core::app::TurnStreamUpdate;
use crate::domain::parallel_mode::ParallelModePostTurnQueueSignal;

use super::conversation_runtime::{
    PostTurnEvaluationOutcome, conversation_runtime_auto_prompt_queued,
    suppress_conversation_runtime_auto_prompt,
};
use super::{
    ConversationRuntimeEffect, ConversationRuntimeEvent, ConversationState, NativeTuiApp,
    PlanningWorkerPanelState,
};

pub(super) struct PostTurnEvaluationCompletionPayload {
    pub(super) evaluation: Box<PostTurnEvaluationOutcome>,
    pub(super) planning_worker_panel_state: PlanningWorkerPanelState,
}

pub(super) struct PostTurnContinuationRoutingContext {
    route_after_reduction: bool,
    parallel_mode_post_turn_queue_signal: Option<ParallelModePostTurnQueueSignal>,
}

impl NativeTuiApp {
    pub(super) fn apply_post_turn_evaluation_completion_payload(
        &mut self,
        result: PostTurnEvaluationCompletionPayload,
    ) -> bool {
        self.planning_worker_panel_state = result.planning_worker_panel_state;
        self.invalidate_parallel_mode_supervisor_snapshot();
        self.dispatch_conversation_runtime(ConversationRuntimeEvent::PostTurnEvaluationCompleted {
            evaluation: result.evaluation,
        });
        true
    }

    pub(super) fn post_turn_continuation_context(
        &self,
        event: &ConversationRuntimeEvent,
    ) -> PostTurnContinuationRoutingContext {
        let failed_stream_snapshot = matches!(
            event,
            ConversationRuntimeEvent::StreamSnapshotApplied(snapshot)
                if matches!(&snapshot.update, TurnStreamUpdate::Failed { .. })
        );
        PostTurnContinuationRoutingContext {
            route_after_reduction: matches!(
                event,
                ConversationRuntimeEvent::PostTurnEvaluationCompleted { .. }
            ) || failed_stream_snapshot,
            parallel_mode_post_turn_queue_signal: self.parallel_mode_post_turn_queue_signal(event),
        }
    }

    pub(super) fn route_post_turn_continuation_effects(
        &mut self,
        context: PostTurnContinuationRoutingContext,
        effects: &mut Vec<ConversationRuntimeEffect>,
    ) {
        if !context.route_after_reduction {
            return;
        }
        let queued_auto_prompt_available = conversation_runtime_auto_prompt_queued(effects);
        let pending_task_intake_executed = self.execute_pending_task_intake_command_if_ready();
        let parallel_dispatch_consumed_auto_prompt = if pending_task_intake_executed {
            false
        } else {
            self.apply_parallel_mode_post_turn_queue_continuation(
                queued_auto_prompt_available,
                context.parallel_mode_post_turn_queue_signal,
            )
        };
        let route = decide_post_turn_auto_prompt_route(PostTurnAutoPromptRouteRequest {
            queued_auto_prompt_available,
            pending_task_intake_executed,
            parallel_dispatch_consumed_auto_prompt,
        });
        self.apply_post_turn_auto_prompt_route(route, effects);
    }

    fn apply_post_turn_auto_prompt_route(
        &mut self,
        route: PostTurnAutoPromptRoute,
        effects: &mut Vec<ConversationRuntimeEffect>,
    ) {
        if route.should_suppress_prompt() {
            suppress_conversation_runtime_auto_prompt(effects);
        }
        if matches!(
            route,
            PostTurnAutoPromptRoute::Suppress(
                PostTurnAutoPromptSuppressionReason::ParallelDispatch
            )
        ) {
            self.record_auto_follow_parallel_dispatch();
        }
    }

    fn record_auto_follow_parallel_dispatch(&mut self) {
        if let ConversationState::Ready(conversation) = &mut self.conversation_state {
            conversation.record_auto_follow_parallel_dispatch();
        }
    }
}

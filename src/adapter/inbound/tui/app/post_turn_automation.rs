use crate::application::service::post_turn_decision::{
    PostTurnAutoPromptRoute, PostTurnAutoPromptRouteRequest, PostTurnAutoPromptSuppressionReason,
    decide_post_turn_auto_prompt_route,
};
use crate::core::app::TurnStreamUpdate;
use crate::domain::parallel_mode::ParallelModePostTurnQueueSignal;

use super::conversation_runtime::{
    ConversationPostTurnEvaluation, conversation_runtime_auto_prompt_queued,
    suppress_conversation_runtime_auto_prompt,
};
use super::{
    ConversationRuntimeEffect, ConversationRuntimeEvent, ConversationState, NativeTuiApp,
    PlanningWorkerPanelState,
};

pub(super) struct PostTurnAutomationBackgroundResult {
    pub(super) thread_id: String,
    pub(super) completed_turn_id: String,
    pub(super) evaluation: Box<ConversationPostTurnEvaluation>,
    pub(super) planning_worker_panel_state: PlanningWorkerPanelState,
}

pub(super) struct ConversationRuntimeAutomationContext {
    route_after_reduction: bool,
    parallel_mode_post_turn_queue_signal: Option<ParallelModePostTurnQueueSignal>,
}

impl NativeTuiApp {
    pub(super) fn enqueue_post_turn_automation_result(
        &mut self,
        result: PostTurnAutomationBackgroundResult,
    ) {
        self.pending_post_turn_automation_results.push(result);
    }

    pub(super) fn route_pending_post_turn_automation_result(
        &mut self,
        thread_id: &str,
        completed_turn_id: &str,
    ) -> bool {
        let Some(result_index) =
            self.pending_post_turn_automation_results
                .iter()
                .position(|result| {
                    result.thread_id == thread_id && result.completed_turn_id == completed_turn_id
                })
        else {
            return false;
        };
        let result = self
            .pending_post_turn_automation_results
            .remove(result_index);
        self.route_post_turn_automation_result(result)
    }

    pub(super) fn route_post_turn_automation_result(
        &mut self,
        result: PostTurnAutomationBackgroundResult,
    ) -> bool {
        if !self.should_apply_post_turn_evaluation(&result.thread_id, &result.completed_turn_id) {
            return false;
        }
        self.record_post_turn_evaluation_applied(&result.completed_turn_id);
        self.planning_worker_panel_state = result.planning_worker_panel_state;
        self.invalidate_parallel_mode_supervisor_snapshot();
        self.dispatch_conversation_runtime(ConversationRuntimeEvent::PostTurnAutomationEvaluated {
            evaluation: result.evaluation,
        });
        true
    }

    pub(super) fn conversation_runtime_automation_context(
        &self,
        event: &ConversationRuntimeEvent,
    ) -> ConversationRuntimeAutomationContext {
        let failed_stream_snapshot = matches!(
            event,
            ConversationRuntimeEvent::StreamSnapshotApplied(snapshot)
                if matches!(&snapshot.update, TurnStreamUpdate::Failed { .. })
        );
        ConversationRuntimeAutomationContext {
            route_after_reduction: matches!(
                event,
                ConversationRuntimeEvent::PostTurnAutomationEvaluated { .. }
            ) || failed_stream_snapshot,
            parallel_mode_post_turn_queue_signal: self.parallel_mode_post_turn_queue_signal(event),
        }
    }

    pub(super) fn route_conversation_runtime_automation_effects(
        &mut self,
        context: ConversationRuntimeAutomationContext,
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

    fn should_apply_post_turn_evaluation(&self, thread_id: &str, completed_turn_id: &str) -> bool {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.accepts_post_turn_evaluation(thread_id, completed_turn_id)
            }
            ConversationState::Loading | ConversationState::Failed(_) => false,
        }
    }

    fn record_post_turn_evaluation_applied(&mut self, completed_turn_id: &str) {
        if let ConversationState::Ready(conversation) = &mut self.conversation_state {
            conversation.record_post_turn_evaluation_applied(completed_turn_id);
        }
    }

    fn record_auto_follow_parallel_dispatch(&mut self) {
        if let ConversationState::Ready(conversation) = &mut self.conversation_state {
            conversation.record_auto_follow_parallel_dispatch();
        }
    }
}

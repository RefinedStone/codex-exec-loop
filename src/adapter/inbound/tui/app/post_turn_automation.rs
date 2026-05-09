use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::parallel_mode::control_plane::ParallelModePostTurnQueueContinuationTarget;
use crate::domain::parallel_mode::ParallelModePostTurnQueueSignal;

use super::conversation_runtime::ConversationPostTurnEvaluation;
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

pub(super) struct TuiPostTurnQueueContinuationTarget<'a> {
    effects: &'a mut Vec<ConversationRuntimeEffect>,
    conversation_state: &'a mut ConversationState,
}

impl<'a> TuiPostTurnQueueContinuationTarget<'a> {
    pub(super) fn new(
        effects: &'a mut Vec<ConversationRuntimeEffect>,
        conversation_state: &'a mut ConversationState,
    ) -> Self {
        Self {
            effects,
            conversation_state,
        }
    }
}

impl ParallelModePostTurnQueueContinuationTarget for TuiPostTurnQueueContinuationTarget<'_> {
    fn auto_follow_prompt_queued(&self) -> bool {
        self.effects
            .iter()
            .any(|effect| matches!(effect, ConversationRuntimeEffect::QueueAutoPrompt { .. }))
    }

    fn consume_auto_follow_prompt(&mut self) {
        self.effects
            .retain(|effect| !matches!(effect, ConversationRuntimeEffect::QueueAutoPrompt { .. }));
    }

    fn record_auto_follow_parallel_dispatch(&mut self) {
        if let ConversationState::Ready(conversation) = self.conversation_state {
            conversation.record_auto_follow_parallel_dispatch();
        }
    }
}

impl NativeTuiApp {
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
        ConversationRuntimeAutomationContext {
            route_after_reduction: matches!(
                event,
                ConversationRuntimeEvent::PostTurnAutomationEvaluated { .. }
                    | ConversationRuntimeEvent::StreamUpdated(
                        ConversationStreamEvent::Failed { .. }
                    )
            ),
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
        // A task-intake command can become executable only after the stream or
        // post-turn evaluation settles planning state. When it fires, suppress the
        // reducer's generic auto prompt to avoid double-submitting.
        if self.execute_pending_task_intake_command_if_ready() {
            effects.retain(|effect| {
                !matches!(effect, ConversationRuntimeEffect::QueueAutoPrompt { .. })
            });
            return;
        }
        self.apply_parallel_mode_post_turn_queue_continuation(
            effects,
            context.parallel_mode_post_turn_queue_signal,
        );
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
}

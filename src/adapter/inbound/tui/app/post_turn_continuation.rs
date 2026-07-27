use crate::core::app::{PostTurnEvaluationCorrelation, PostTurnRouteResolution};

use super::conversation_runtime::PostTurnEvaluationOutcome;
use super::{ConversationRuntimeEvent, NativeTuiApp, PlanningWorkerPanelState};

pub(super) struct PostTurnEvaluationCompletionPayload {
    pub(super) correlation: PostTurnEvaluationCorrelation,
    pub(super) evaluation: Box<PostTurnEvaluationOutcome>,
    pub(super) planning_worker_panel_state: PlanningWorkerPanelState,
    pub(super) route_resolution: PostTurnRouteResolution,
}

impl NativeTuiApp {
    pub(super) fn apply_post_turn_evaluation_completion_payload(
        &mut self,
        result: PostTurnEvaluationCompletionPayload,
    ) -> bool {
        self.planning
            .planning_worker_panel_state
            .apply_completed(result.planning_worker_panel_state);
        self.invalidate_parallel_mode_supervisor_snapshot();
        self.dispatch_conversation_runtime(ConversationRuntimeEvent::PostTurnEvaluationCompleted {
            correlation: result.correlation,
            evaluation: result.evaluation,
            route_resolution: result.route_resolution,
        });
        true
    }
}

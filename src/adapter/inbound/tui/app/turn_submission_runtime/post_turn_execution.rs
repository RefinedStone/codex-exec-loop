use crate::core::app::PostTurnEvaluationCorrelation;
use crate::domain::planning::{
    PostTurnAutoFollowSkipReason,
    PostTurnContinuationAction as ApplicationPostTurnContinuationAction,
    PostTurnExecution as PostTurnEvaluationExecution,
    PostTurnOutcome as ApplicationPostTurnEvaluationOutcome,
    PostTurnProvenance as ApplicationPostTurnEvaluationProvenance,
};

use super::super::conversation_model::PlanningRepairState;
use super::super::conversation_runtime::{
    PostTurnContinuationAction, PostTurnEvaluationOutcome, PostTurnEvaluationProvenance,
    PostTurnQueuedPrompt,
};
use super::super::post_turn_continuation::PostTurnEvaluationCompletionPayload;
use super::super::{AutoFollowSkipReason, NativeTuiApp};

impl NativeTuiApp {
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
                mode_label: prompt.mode_label,
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

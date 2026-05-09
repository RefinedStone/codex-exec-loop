use super::orchestrator::{
    ParallelModeOrchestratorStateMachine, ParallelModePostTurnQueueDecision,
    ParallelModePostTurnQueueSignal,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeDispatchReadinessDecision {
    Ready,
    Deferred { reason: &'static str },
}

impl ParallelModeDispatchReadinessDecision {
    pub fn deferred_reason(self) -> Option<&'static str> {
        match self {
            Self::Ready => None,
            Self::Deferred { reason } => Some(reason),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelModeControlPlaneAggregate;

impl ParallelModeControlPlaneAggregate {
    pub fn dispatch_readiness(
        projection_ready: bool,
        control_effect_in_flight: bool,
    ) -> ParallelModeDispatchReadinessDecision {
        if !projection_ready || control_effect_in_flight {
            return ParallelModeDispatchReadinessDecision::Deferred {
                reason: "entry loading or control-plane refresh is still in progress",
            };
        }

        ParallelModeDispatchReadinessDecision::Ready
    }

    pub fn post_turn_queue_continuation(
        parallel_mode_enabled: bool,
        signal: Option<ParallelModePostTurnQueueSignal>,
        has_actionable_queue_head: bool,
    ) -> ParallelModePostTurnQueueDecision {
        ParallelModeOrchestratorStateMachine::post_turn_queue_continuation(
            parallel_mode_enabled,
            signal,
            has_actionable_queue_head,
        )
    }

    pub fn command_targets_current_epoch(
        command_workspace: &str,
        command_epoch_id: u64,
        current_workspace: Option<&str>,
        current_epoch_id: Option<u64>,
    ) -> bool {
        current_workspace == Some(command_workspace) && current_epoch_id == Some(command_epoch_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::parallel_mode::ParallelModeAutomationTrigger;

    #[test]
    fn dispatch_readiness_defers_until_projection_is_ready_and_idle() {
        assert!(matches!(
            ParallelModeControlPlaneAggregate::dispatch_readiness(false, false),
            ParallelModeDispatchReadinessDecision::Deferred { .. }
        ));
        assert!(matches!(
            ParallelModeControlPlaneAggregate::dispatch_readiness(true, true),
            ParallelModeDispatchReadinessDecision::Deferred { .. }
        ));
        assert_eq!(
            ParallelModeControlPlaneAggregate::dispatch_readiness(true, false),
            ParallelModeDispatchReadinessDecision::Ready
        );
    }

    #[test]
    fn post_turn_continuation_keeps_policy_in_domain() {
        let decision = ParallelModeControlPlaneAggregate::post_turn_queue_continuation(
            true,
            Some(ParallelModePostTurnQueueSignal::AutoFollowQueued),
            false,
        );

        assert_eq!(
            decision.dispatch_trigger(),
            Some(ParallelModeAutomationTrigger::MainTurnPostEvaluation)
        );
        assert!(decision.should_consume_auto_follow_prompt());
    }

    #[test]
    fn stale_epoch_detection_is_workspace_scoped() {
        assert!(
            ParallelModeControlPlaneAggregate::command_targets_current_epoch(
                "/repo",
                7,
                Some("/repo"),
                Some(7)
            )
        );
        assert!(
            !ParallelModeControlPlaneAggregate::command_targets_current_epoch(
                "/repo",
                7,
                Some("/other"),
                Some(7)
            )
        );
    }
}

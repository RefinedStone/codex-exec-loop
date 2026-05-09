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
pub struct ParallelModeEnableEntryDecision {
    pub mode_was_enabled: bool,
    pub initial_pool_reset_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeEffectStartDecision {
    StartNow,
    QueueUntilIdle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeProjectionReadyContinuation {
    RefreshSupervisor,
    DrainPendingWake,
    PollPendingDispatchWake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeOrchestratorTickDecision {
    Start,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelModeControlPlaneAggregate;

impl ParallelModeControlPlaneAggregate {
    pub fn mode_enabled_for_workspace(
        mode_enabled: bool,
        current_workspace: Option<&str>,
        workspace: &str,
    ) -> bool {
        mode_enabled && current_workspace == Some(workspace)
    }

    pub fn enable_entry(
        mode_enabled: bool,
        current_workspace: Option<&str>,
        workspace: &str,
        initial_pool_reset_completed: bool,
    ) -> ParallelModeEnableEntryDecision {
        let mode_was_enabled =
            Self::mode_enabled_for_workspace(mode_enabled, current_workspace, workspace);
        ParallelModeEnableEntryDecision {
            mode_was_enabled,
            initial_pool_reset_required: !mode_was_enabled && !initial_pool_reset_completed,
        }
    }

    pub fn effect_start_decision(
        control_effect_in_flight: bool,
    ) -> ParallelModeEffectStartDecision {
        if control_effect_in_flight {
            return ParallelModeEffectStartDecision::QueueUntilIdle;
        }

        ParallelModeEffectStartDecision::StartNow
    }

    pub fn projection_ready_continuation(
        pending_supervisor_refresh: bool,
        pending_orchestrator_wake: bool,
    ) -> ParallelModeProjectionReadyContinuation {
        if pending_supervisor_refresh {
            return ParallelModeProjectionReadyContinuation::RefreshSupervisor;
        }
        if pending_orchestrator_wake {
            return ParallelModeProjectionReadyContinuation::DrainPendingWake;
        }

        ParallelModeProjectionReadyContinuation::PollPendingDispatchWake
    }

    pub fn orchestrator_tick_decision(
        control_effect_in_flight: bool,
        last_signature: Option<&str>,
        requested_signature: &str,
    ) -> ParallelModeOrchestratorTickDecision {
        if control_effect_in_flight || last_signature == Some(requested_signature) {
            return ParallelModeOrchestratorTickDecision::Skip;
        }

        ParallelModeOrchestratorTickDecision::Start
    }

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

    pub fn current_epoch_for_workspace(
        command_workspace: &str,
        current_workspace: Option<&str>,
        current_epoch_id: Option<u64>,
    ) -> Option<u64> {
        match (current_workspace, current_epoch_id) {
            (Some(current_workspace), Some(epoch_id)) if current_workspace == command_workspace => {
                Some(epoch_id)
            }
            _ => None,
        }
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
    fn enable_entry_requires_initial_reset_only_for_new_mode_entry() {
        assert_eq!(
            ParallelModeControlPlaneAggregate::enable_entry(false, None, "/repo", false),
            ParallelModeEnableEntryDecision {
                mode_was_enabled: false,
                initial_pool_reset_required: true,
            }
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::enable_entry(true, Some("/repo"), "/repo", false),
            ParallelModeEnableEntryDecision {
                mode_was_enabled: true,
                initial_pool_reset_required: false,
            }
        );
    }

    #[test]
    fn projection_ready_prioritizes_refresh_then_pending_wake_then_poll() {
        assert_eq!(
            ParallelModeControlPlaneAggregate::projection_ready_continuation(true, true),
            ParallelModeProjectionReadyContinuation::RefreshSupervisor
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::projection_ready_continuation(false, true),
            ParallelModeProjectionReadyContinuation::DrainPendingWake
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::projection_ready_continuation(false, false),
            ParallelModeProjectionReadyContinuation::PollPendingDispatchWake
        );
    }

    #[test]
    fn orchestrator_tick_skips_duplicate_or_busy_tick() {
        assert_eq!(
            ParallelModeControlPlaneAggregate::orchestrator_tick_decision(true, None, "sig"),
            ParallelModeOrchestratorTickDecision::Skip
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::orchestrator_tick_decision(
                false,
                Some("sig"),
                "sig"
            ),
            ParallelModeOrchestratorTickDecision::Skip
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::orchestrator_tick_decision(
                false,
                Some("old"),
                "sig"
            ),
            ParallelModeOrchestratorTickDecision::Start
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

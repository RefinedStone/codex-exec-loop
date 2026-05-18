use serde::{Deserialize, Serialize};

use super::orchestrator::{
    ParallelModeAutomationTrigger, ParallelModeOrchestratorStateMachine,
    ParallelModePostTurnQueueDecision, ParallelModePostTurnQueueSignal,
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
pub struct ParallelModeSupervisorInspectionDecision {
    pub mode_enabled: bool,
    pub reconcile_pool: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeControlPlaneEffectCompletionFollowUp {
    RefreshSupervisor,
    DrainPendingWake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeEntryCompletionDecision {
    CloseEpoch,
    RefreshSupervisor,
    DispatchInitialQueue,
    ProjectionReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeModeCompletionDecision {
    KeepEpochOpen,
    CloseEpoch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeTickCompletionDecision {
    RefreshSupervisorOnly,
    RefreshSupervisorAndQueueCapacityDispatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModePendingDispatchPollDecision {
    Ready,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModePendingDispatchWakeDecision {
    StartWake,
    RunFollowUpTick,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModeControlPlaneWorkerEventKind {
    #[serde(rename = "worker_completed")]
    Completed,
    #[serde(rename = "worker_launch_failed")]
    LaunchFailed,
    #[serde(rename = "worker_stream_failed")]
    StreamFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeControlPlaneWorkerEvent {
    pub workspace_directory: String,
    pub epoch_id: u64,
    pub task_id: String,
    pub task_title: String,
    pub kind: ParallelModeControlPlaneWorkerEventKind,
    pub notices: Vec<String>,
}

impl ParallelModeControlPlaneWorkerEvent {
    pub fn new(
        workspace_directory: impl Into<String>,
        epoch_id: u64,
        task_id: impl Into<String>,
        task_title: impl Into<String>,
        kind: ParallelModeControlPlaneWorkerEventKind,
        notices: Vec<String>,
    ) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            epoch_id,
            task_id: task_id.into(),
            task_title: task_title.into(),
            kind,
            notices,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelModeControlPlaneWorkerEventDecision {
    pub stale_drop_reason: Option<&'static str>,
    pub refresh_supervisor: bool,
    pub wake_trigger: Option<ParallelModeAutomationTrigger>,
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

    pub fn supervisor_inspection(
        mode_enabled: bool,
        current_workspace: Option<&str>,
        workspace: &str,
        reconcile_pool: bool,
    ) -> ParallelModeSupervisorInspectionDecision {
        let mode_enabled =
            Self::mode_enabled_for_workspace(mode_enabled, current_workspace, workspace);
        ParallelModeSupervisorInspectionDecision {
            mode_enabled,
            reconcile_pool: reconcile_pool && mode_enabled,
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

    pub fn effect_completion_follow_up(
        pending_supervisor_refresh: bool,
    ) -> ParallelModeControlPlaneEffectCompletionFollowUp {
        if pending_supervisor_refresh {
            return ParallelModeControlPlaneEffectCompletionFollowUp::RefreshSupervisor;
        }

        ParallelModeControlPlaneEffectCompletionFollowUp::DrainPendingWake
    }

    pub fn entry_completion(
        mode_enabled: bool,
        mode_was_enabled: bool,
        pending_supervisor_refresh: bool,
        has_actionable_queue_head: bool,
    ) -> ParallelModeEntryCompletionDecision {
        if !mode_enabled {
            return ParallelModeEntryCompletionDecision::CloseEpoch;
        }
        if pending_supervisor_refresh {
            return ParallelModeEntryCompletionDecision::RefreshSupervisor;
        }
        if !mode_was_enabled && has_actionable_queue_head {
            return ParallelModeEntryCompletionDecision::DispatchInitialQueue;
        }

        ParallelModeEntryCompletionDecision::ProjectionReady
    }

    pub fn mode_completion(mode_enabled: bool) -> ParallelModeModeCompletionDecision {
        if mode_enabled {
            return ParallelModeModeCompletionDecision::KeepEpochOpen;
        }

        ParallelModeModeCompletionDecision::CloseEpoch
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

    pub fn tick_completion(blocked: bool) -> ParallelModeTickCompletionDecision {
        if blocked {
            return ParallelModeTickCompletionDecision::RefreshSupervisorOnly;
        }

        ParallelModeTickCompletionDecision::RefreshSupervisorAndQueueCapacityDispatch
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

    pub fn pending_dispatch_poll_readiness(
        mode_enabled: bool,
        current_epoch_open: bool,
        projection_ready: bool,
        control_effect_in_flight: bool,
    ) -> ParallelModePendingDispatchPollDecision {
        if !mode_enabled || !current_epoch_open || !projection_ready || control_effect_in_flight {
            return ParallelModePendingDispatchPollDecision::Skip;
        }

        ParallelModePendingDispatchPollDecision::Ready
    }

    pub fn pending_dispatch_wake_decision(
        has_wake: bool,
        has_follow_up_tick: bool,
    ) -> ParallelModePendingDispatchWakeDecision {
        if has_wake {
            return ParallelModePendingDispatchWakeDecision::StartWake;
        }
        if has_follow_up_tick {
            return ParallelModePendingDispatchWakeDecision::RunFollowUpTick;
        }

        ParallelModePendingDispatchWakeDecision::Idle
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

    pub fn worker_event_decision(
        event_workspace: &str,
        event_epoch_id: u64,
        event_kind: ParallelModeControlPlaneWorkerEventKind,
        current_workspace: Option<&str>,
        current_epoch_id: Option<u64>,
        has_actionable_queue_head: bool,
    ) -> ParallelModeControlPlaneWorkerEventDecision {
        if current_workspace != Some(event_workspace) {
            return ParallelModeControlPlaneWorkerEventDecision {
                stale_drop_reason: Some("worker event targets a different workspace"),
                refresh_supervisor: false,
                wake_trigger: None,
            };
        }
        if current_epoch_id != Some(event_epoch_id) {
            return ParallelModeControlPlaneWorkerEventDecision {
                stale_drop_reason: Some("worker event belongs to a stale epoch"),
                refresh_supervisor: false,
                wake_trigger: None,
            };
        }

        ParallelModeControlPlaneWorkerEventDecision {
            stale_drop_reason: None,
            refresh_supervisor: true,
            wake_trigger: (event_kind == ParallelModeControlPlaneWorkerEventKind::Completed
                && has_actionable_queue_head)
                .then_some(ParallelModeAutomationTrigger::ParallelOfficialCompletion),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::parallel_mode::ParallelModeAutomationTrigger;

    #[test]
    fn dispatch_readiness_defers_until_projection_is_ready_and_idle() {
        assert_eq!(
            ParallelModeControlPlaneAggregate::dispatch_readiness(false, false),
            ParallelModeDispatchReadinessDecision::Deferred {
                reason: "entry loading or control-plane refresh is still in progress"
            }
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::dispatch_readiness(true, true),
            ParallelModeDispatchReadinessDecision::Deferred {
                reason: "entry loading or control-plane refresh is still in progress"
            }
        );
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
    fn effect_completion_follow_up_prioritizes_pending_supervisor_refresh() {
        assert_eq!(
            ParallelModeControlPlaneAggregate::effect_completion_follow_up(true),
            ParallelModeControlPlaneEffectCompletionFollowUp::RefreshSupervisor
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::effect_completion_follow_up(false),
            ParallelModeControlPlaneEffectCompletionFollowUp::DrainPendingWake
        );
    }

    #[test]
    fn entry_completion_keeps_projection_and_initial_dispatch_policy_in_domain() {
        assert_eq!(
            ParallelModeControlPlaneAggregate::entry_completion(false, false, false, true),
            ParallelModeEntryCompletionDecision::CloseEpoch
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::entry_completion(true, false, true, true),
            ParallelModeEntryCompletionDecision::RefreshSupervisor
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::entry_completion(true, false, false, true),
            ParallelModeEntryCompletionDecision::DispatchInitialQueue
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::entry_completion(true, true, false, true),
            ParallelModeEntryCompletionDecision::ProjectionReady
        );
    }

    #[test]
    fn mode_completion_closes_epoch_only_when_mode_is_disabled() {
        assert_eq!(
            ParallelModeControlPlaneAggregate::mode_completion(true),
            ParallelModeModeCompletionDecision::KeepEpochOpen
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::mode_completion(false),
            ParallelModeModeCompletionDecision::CloseEpoch
        );
    }

    #[test]
    fn tick_completion_queues_capacity_dispatch_only_when_retry_is_not_blocked() {
        assert_eq!(
            ParallelModeControlPlaneAggregate::tick_completion(true),
            ParallelModeTickCompletionDecision::RefreshSupervisorOnly
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::tick_completion(false),
            ParallelModeTickCompletionDecision::RefreshSupervisorAndQueueCapacityDispatch
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
    fn pending_dispatch_poll_requires_enabled_ready_open_epoch() {
        assert_eq!(
            ParallelModeControlPlaneAggregate::pending_dispatch_poll_readiness(
                true, true, true, false
            ),
            ParallelModePendingDispatchPollDecision::Ready
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::pending_dispatch_poll_readiness(
                true, true, false, false
            ),
            ParallelModePendingDispatchPollDecision::Skip
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::pending_dispatch_poll_readiness(
                true, true, true, true
            ),
            ParallelModePendingDispatchPollDecision::Skip
        );
    }

    #[test]
    fn worker_event_decision_drops_events_for_different_workspace_before_epoch_checks() {
        assert_eq!(
            ParallelModeControlPlaneAggregate::worker_event_decision(
                "/repo",
                7,
                ParallelModeControlPlaneWorkerEventKind::Completed,
                Some("/other"),
                Some(7),
                true,
            ),
            ParallelModeControlPlaneWorkerEventDecision {
                stale_drop_reason: Some("worker event targets a different workspace"),
                refresh_supervisor: false,
                wake_trigger: None,
            }
        );
    }

    #[test]
    fn worker_event_decision_is_epoch_scoped_and_wakes_only_completed_queue_work() {
        assert_eq!(
            ParallelModeControlPlaneAggregate::worker_event_decision(
                "/repo",
                7,
                ParallelModeControlPlaneWorkerEventKind::Completed,
                Some("/repo"),
                Some(7),
                true,
            ),
            ParallelModeControlPlaneWorkerEventDecision {
                stale_drop_reason: None,
                refresh_supervisor: true,
                wake_trigger: Some(ParallelModeAutomationTrigger::ParallelOfficialCompletion),
            }
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::worker_event_decision(
                "/repo",
                7,
                ParallelModeControlPlaneWorkerEventKind::LaunchFailed,
                Some("/repo"),
                Some(7),
                true,
            )
            .wake_trigger,
            None
        );
        assert_eq!(
            ParallelModeControlPlaneAggregate::worker_event_decision(
                "/repo",
                7,
                ParallelModeControlPlaneWorkerEventKind::Completed,
                Some("/repo"),
                Some(8),
                true,
            )
            .stale_drop_reason,
            Some("worker event belongs to a stale epoch")
        );
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

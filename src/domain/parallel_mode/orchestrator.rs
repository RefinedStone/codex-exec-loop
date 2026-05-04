// orchestrator state machine은 parallel mode의 제어 결정을 표시 문자열이나
// 파일 존재 여부에서 분리한다. application layer는 여기서 나온 action만 실행하고,
// planning task authority 자체를 reset 대상으로 삼지 않는다.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModePoolResetScope {
    PoolOnly,
}

impl ParallelModePoolResetScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::PoolOnly => "pool_only",
        }
    }

    pub fn status_detail(self) -> &'static str {
        match self {
            Self::PoolOnly => "pool-only reset; planning tasks preserved",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeOrchestratorState {
    Off,
    ReadinessBlocked,
    PoolResetting,
    Dispatching,
    Supervising,
    IntegrationBlocked,
}

impl ParallelModeOrchestratorState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::ReadinessBlocked => "readiness_blocked",
            Self::PoolResetting => "pool_resetting",
            Self::Dispatching => "dispatching",
            Self::Supervising => "supervising",
            Self::IntegrationBlocked => "integration_blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelModeEntryPlan {
    pub state: ParallelModeOrchestratorState,
    pub reset_scope: Option<ParallelModePoolResetScope>,
}

impl ParallelModeEntryPlan {
    fn new(
        state: ParallelModeOrchestratorState,
        reset_scope: Option<ParallelModePoolResetScope>,
    ) -> Self {
        Self { state, reset_scope }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModeDispatchBlockReason {
    RuntimeAlreadyOwnsTask,
    StartupFailedUntilTaskChanges,
}

impl ParallelModeDispatchBlockReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::RuntimeAlreadyOwnsTask => "runtime_already_owns_task",
            Self::StartupFailedUntilTaskChanges => "startup_failed_until_task_changes",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeTaskDispatchBlockSnapshot {
    pub task_id: String,
    pub task_updated_at: String,
    pub blocked_at: String,
    pub reason: ParallelModeDispatchBlockReason,
}

impl ParallelModeTaskDispatchBlockSnapshot {
    pub fn new(
        task_id: impl Into<String>,
        task_updated_at: impl Into<String>,
        blocked_at: impl Into<String>,
        reason: ParallelModeDispatchBlockReason,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            task_updated_at: task_updated_at.into(),
            blocked_at: blocked_at.into(),
            reason,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelModeDispatchEligibility {
    pub block_reason: Option<ParallelModeDispatchBlockReason>,
}

impl ParallelModeDispatchEligibility {
    fn dispatchable() -> Self {
        Self { block_reason: None }
    }

    fn blocked(reason: ParallelModeDispatchBlockReason) -> Self {
        Self {
            block_reason: Some(reason),
        }
    }

    pub fn is_dispatchable(self) -> bool {
        self.block_reason.is_none()
    }
}

pub struct ParallelModeOrchestratorStateMachine;

impl ParallelModeOrchestratorStateMachine {
    pub fn plan_parallel_entry(
        mode_was_enabled: bool,
        readiness_allows_parallel_mode: bool,
    ) -> ParallelModeEntryPlan {
        if !readiness_allows_parallel_mode {
            return ParallelModeEntryPlan::new(
                ParallelModeOrchestratorState::ReadinessBlocked,
                None,
            );
        }

        if mode_was_enabled {
            return ParallelModeEntryPlan::new(ParallelModeOrchestratorState::Dispatching, None);
        }

        ParallelModeEntryPlan::new(
            ParallelModeOrchestratorState::PoolResetting,
            Some(ParallelModePoolResetScope::PoolOnly),
        )
    }

    pub fn dispatch_eligibility(
        runtime_already_owns_task: bool,
        startup_failed_at_epoch_millis: Option<i64>,
        task_updated_at_epoch_millis: Option<i64>,
    ) -> ParallelModeDispatchEligibility {
        if runtime_already_owns_task {
            return ParallelModeDispatchEligibility::blocked(
                ParallelModeDispatchBlockReason::RuntimeAlreadyOwnsTask,
            );
        }

        if let (Some(failed_at), Some(task_updated_at)) =
            (startup_failed_at_epoch_millis, task_updated_at_epoch_millis)
            && failed_at >= task_updated_at
        {
            return ParallelModeDispatchEligibility::blocked(
                ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
            );
        }

        ParallelModeDispatchEligibility::dispatchable()
    }

    pub fn tick_state(integration_worktree_blocked: bool) -> ParallelModeOrchestratorState {
        if integration_worktree_blocked {
            ParallelModeOrchestratorState::IntegrationBlocked
        } else {
            ParallelModeOrchestratorState::Supervising
        }
    }
}

// orchestrator state machine은 parallel mode의 제어 결정을 표시 문자열이나
// 파일 존재 여부에서 분리한다. application layer는 여기서 나온 action만 실행하고,
// planning task authority 자체를 reset 대상으로 삼지 않는다.
use super::pool_reset::ParallelModePoolResetPolicy;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModeAutomationTrigger {
    MainTurnPostEvaluation,
    ParallelOfficialCompletion,
    TaskIntakeAfterEpoch,
}

impl ParallelModeAutomationTrigger {
    pub fn label(self) -> &'static str {
        match self {
            Self::MainTurnPostEvaluation => "main_turn_post_evaluation",
            Self::ParallelOfficialCompletion => "parallel_official_completion",
            Self::TaskIntakeAfterEpoch => "task_intake_after_epoch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModePostTurnQueueSignal {
    AutoFollowQueued,
    ParallelCompletionFinalized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModePostTurnQueueDecision {
    NoDispatch,
    Dispatch {
        trigger: ParallelModeAutomationTrigger,
        consume_auto_follow_prompt: bool,
    },
}

impl ParallelModePostTurnQueueDecision {
    pub fn dispatch_trigger(self) -> Option<ParallelModeAutomationTrigger> {
        match self {
            Self::NoDispatch => None,
            Self::Dispatch { trigger, .. } => Some(trigger),
        }
    }

    pub fn should_consume_auto_follow_prompt(self) -> bool {
        match self {
            Self::NoDispatch => false,
            Self::Dispatch {
                consume_auto_follow_prompt,
                ..
            } => consume_auto_follow_prompt,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeRuntimeEvent {
    AutoFollowQueued,
    ParallelCompletionFinalized,
    TaskIntakeCommitted,
    SlotCapacityAvailable,
    ModeDisabled,
}

impl ParallelModeRuntimeEvent {
    fn dispatch_trigger(self) -> Option<ParallelModeAutomationTrigger> {
        match self {
            Self::AutoFollowQueued => Some(ParallelModeAutomationTrigger::MainTurnPostEvaluation),
            Self::ParallelCompletionFinalized => {
                Some(ParallelModeAutomationTrigger::ParallelOfficialCompletion)
            }
            Self::TaskIntakeCommitted | Self::SlotCapacityAvailable => {
                Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch)
            }
            Self::ModeDisabled => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModeDispatchCommandKind {
    DispatchReadyQueue,
}

impl ParallelModeDispatchCommandKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::DispatchReadyQueue => "dispatch_ready_queue",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "dispatch_ready_queue" => Some(Self::DispatchReadyQueue),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModeDispatchCommandState {
    Pending,
    Running,
    Completed,
    Blocked,
    Canceled,
}

impl ParallelModeDispatchCommandState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
            Self::Canceled => "canceled",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "pending" => Some(Self::Pending),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "blocked" => Some(Self::Blocked),
            "canceled" => Some(Self::Canceled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeDispatchCommandSnapshot {
    pub command_id: String,
    pub kind: ParallelModeDispatchCommandKind,
    pub trigger: ParallelModeAutomationTrigger,
    pub state: ParallelModeDispatchCommandState,
    #[serde(default)]
    pub queue_head_signature: Option<String>,
    #[serde(default)]
    pub epoch_id: Option<u64>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub owner_token: Option<String>,
    #[serde(default)]
    pub status_detail: Option<String>,
}

impl ParallelModeDispatchCommandSnapshot {
    pub fn dispatch_ready_queue(
        trigger: ParallelModeAutomationTrigger,
        queue_head_signature: Option<String>,
        epoch_id: Option<u64>,
        timestamp: impl Into<String>,
    ) -> Self {
        let timestamp = timestamp.into();
        Self {
            command_id: dispatch_ready_queue_command_id(queue_head_signature.as_deref()),
            kind: ParallelModeDispatchCommandKind::DispatchReadyQueue,
            trigger,
            state: ParallelModeDispatchCommandState::Pending,
            queue_head_signature,
            epoch_id,
            created_at: timestamp.clone(),
            updated_at: timestamp,
            owner_token: None,
            status_detail: None,
        }
    }

    pub fn mark_running(&mut self, owner_token: impl Into<String>, timestamp: impl Into<String>) {
        self.state = ParallelModeDispatchCommandState::Running;
        self.owner_token = Some(owner_token.into());
        self.updated_at = timestamp.into();
    }

    pub fn mark_completed(
        &mut self,
        status_detail: impl Into<String>,
        timestamp: impl Into<String>,
    ) {
        self.state = ParallelModeDispatchCommandState::Completed;
        self.owner_token = None;
        self.status_detail = Some(status_detail.into());
        self.updated_at = timestamp.into();
    }

    pub fn mark_blocked(&mut self, status_detail: impl Into<String>, timestamp: impl Into<String>) {
        self.state = ParallelModeDispatchCommandState::Blocked;
        self.owner_token = None;
        self.status_detail = Some(status_detail.into());
        self.updated_at = timestamp.into();
    }

    pub fn mark_canceled(
        &mut self,
        status_detail: impl Into<String>,
        timestamp: impl Into<String>,
    ) {
        self.state = ParallelModeDispatchCommandState::Canceled;
        self.owner_token = None;
        self.status_detail = Some(status_detail.into());
        self.updated_at = timestamp.into();
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            ParallelModeDispatchCommandState::Completed
                | ParallelModeDispatchCommandState::Blocked
                | ParallelModeDispatchCommandState::Canceled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeDispatchOutcome {
    pub trigger: ParallelModeAutomationTrigger,
    pub workspace_directory: String,
    pub epoch_id: u64,
    pub idle_slot_count: usize,
    pub candidate_task_ids: Vec<String>,
    pub launched_task_ids: Vec<String>,
    pub blocked_reason: Option<String>,
    pub status_copy_input: String,
}

impl ParallelModeDispatchOutcome {
    pub fn new(
        trigger: ParallelModeAutomationTrigger,
        workspace_directory: impl Into<String>,
        epoch_id: u64,
    ) -> Self {
        Self {
            trigger,
            workspace_directory: workspace_directory.into(),
            epoch_id,
            idle_slot_count: 0,
            candidate_task_ids: Vec::new(),
            launched_task_ids: Vec::new(),
            blocked_reason: None,
            status_copy_input: String::new(),
        }
    }

    pub fn status_detail(&self) -> String {
        if !self.status_copy_input.trim().is_empty() {
            return self.status_copy_input.clone();
        }
        if let Some(reason) = self.blocked_reason.as_deref() {
            return format!("auto dispatch blocked / {reason}");
        }
        format!("auto dispatched {} worker(s)", self.launched_task_ids.len())
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelModeControlPlaneEntryDecision {
    pub plan: ParallelModeEntryPlan,
    pub reset_policy: Option<ParallelModePoolResetPolicy>,
}

impl ParallelModeControlPlaneEntryDecision {
    fn new(plan: ParallelModeEntryPlan, reset_policy: Option<ParallelModePoolResetPolicy>) -> Self {
        Self { plan, reset_policy }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParallelModeDispatchTaskCandidate {
    pub(crate) task_id: String,
    pub(crate) task_updated_at_epoch_millis: Option<i64>,
}

impl ParallelModeDispatchTaskCandidate {
    pub(crate) fn new(
        task_id: impl Into<String>,
        task_updated_at_epoch_millis: Option<i64>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            task_updated_at_epoch_millis,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParallelModeDispatchCandidateSelection {
    pub(crate) idle_slot_count: usize,
    pub(crate) requested_count: usize,
    pub(crate) dispatch_capacity: usize,
    pub(crate) excluded_task_ids: Vec<String>,
    pub(crate) selected_task_ids: Vec<String>,
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
            return ParallelModeEntryPlan::new(ParallelModeOrchestratorState::Supervising, None);
        }

        ParallelModeEntryPlan::new(
            ParallelModeOrchestratorState::PoolResetting,
            Some(ParallelModePoolResetScope::PoolOnly),
        )
    }

    pub fn decide_parallel_entry(
        mode_was_enabled: bool,
        readiness_allows_parallel_mode: bool,
        initial_pool_reset_required: bool,
    ) -> ParallelModeControlPlaneEntryDecision {
        let plan = Self::plan_parallel_entry(mode_was_enabled, readiness_allows_parallel_mode);
        let reset_policy = match plan.reset_scope {
            Some(ParallelModePoolResetScope::PoolOnly) if initial_pool_reset_required => {
                Some(ParallelModePoolResetPolicy::ForceDisposable)
            }
            Some(ParallelModePoolResetScope::PoolOnly) => {
                Some(ParallelModePoolResetPolicy::ProtectLive)
            }
            None => None,
        };
        ParallelModeControlPlaneEntryDecision::new(plan, reset_policy)
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

    pub(crate) fn select_dispatch_candidates(
        idle_slot_count: usize,
        requested_count: usize,
        runtime_owned_task_ids: impl IntoIterator<Item = String>,
        failed_start_blockers: &std::collections::BTreeMap<String, i64>,
        active_tasks: impl IntoIterator<Item = ParallelModeDispatchTaskCandidate>,
    ) -> ParallelModeDispatchCandidateSelection {
        let dispatch_capacity = requested_count.min(idle_slot_count);
        let runtime_owned_task_ids = runtime_owned_task_ids
            .into_iter()
            .map(|task_id| task_id.trim().to_string())
            .filter(|task_id| !task_id.is_empty())
            .collect::<std::collections::BTreeSet<_>>();
        let mut excluded_task_ids = runtime_owned_task_ids.clone();
        let mut selected_task_ids = Vec::new();

        for task in active_tasks {
            if selected_task_ids.len() >= dispatch_capacity {
                break;
            }
            let task_id = task.task_id.trim();
            let eligibility = Self::dispatch_eligibility(
                runtime_owned_task_ids.contains(task_id),
                failed_start_blockers.get(task_id).copied(),
                task.task_updated_at_epoch_millis,
            );
            if !eligibility.is_dispatchable() {
                excluded_task_ids.insert(task_id.to_string());
                continue;
            }
            selected_task_ids.push(task_id.to_string());
        }

        ParallelModeDispatchCandidateSelection {
            idle_slot_count,
            requested_count,
            dispatch_capacity,
            excluded_task_ids: excluded_task_ids.into_iter().collect(),
            selected_task_ids,
        }
    }

    pub fn post_turn_queue_continuation(
        parallel_mode_enabled: bool,
        signal: Option<ParallelModePostTurnQueueSignal>,
        has_actionable_queue_head: bool,
    ) -> ParallelModePostTurnQueueDecision {
        if !parallel_mode_enabled {
            return ParallelModePostTurnQueueDecision::NoDispatch;
        }

        match signal {
            Some(ParallelModePostTurnQueueSignal::AutoFollowQueued) => {
                ParallelModePostTurnQueueDecision::Dispatch {
                    trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
                    consume_auto_follow_prompt: true,
                }
            }
            Some(ParallelModePostTurnQueueSignal::ParallelCompletionFinalized)
                if has_actionable_queue_head =>
            {
                ParallelModePostTurnQueueDecision::Dispatch {
                    trigger: ParallelModeAutomationTrigger::ParallelOfficialCompletion,
                    consume_auto_follow_prompt: false,
                }
            }
            Some(ParallelModePostTurnQueueSignal::ParallelCompletionFinalized) | None => {
                ParallelModePostTurnQueueDecision::NoDispatch
            }
        }
    }

    pub fn runtime_dispatch_commands(
        parallel_mode_enabled: bool,
        event: ParallelModeRuntimeEvent,
        has_actionable_queue_head: bool,
        queue_head_signature: Option<String>,
        epoch_id: Option<u64>,
        timestamp: impl Into<String>,
    ) -> Vec<ParallelModeDispatchCommandSnapshot> {
        if !parallel_mode_enabled || event == ParallelModeRuntimeEvent::ModeDisabled {
            return Vec::new();
        }
        if !has_actionable_queue_head && event != ParallelModeRuntimeEvent::AutoFollowQueued {
            return Vec::new();
        }
        let Some(trigger) = event.dispatch_trigger() else {
            return Vec::new();
        };
        vec![ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
            trigger,
            queue_head_signature,
            epoch_id,
            timestamp,
        )]
    }

    pub fn tick_state(integration_worktree_blocked: bool) -> ParallelModeOrchestratorState {
        if integration_worktree_blocked {
            ParallelModeOrchestratorState::IntegrationBlocked
        } else {
            ParallelModeOrchestratorState::Supervising
        }
    }
}

fn dispatch_ready_queue_command_id(queue_head_signature: Option<&str>) -> String {
    let signature = queue_head_signature
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    format!("dispatch-ready-queue-{signature}")
}

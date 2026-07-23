use crate::domain::parallel_mode::{
    ParallelModeAutomationTrigger, ParallelModeControlPlaneAggregate,
    ParallelModeControlPlaneEffectCompletionFollowUp, ParallelModeControlPlaneWorkerEvent,
    ParallelModeControlPlaneWorkerEventKind, ParallelModeEffectStartDecision,
    ParallelModeEntryCompletionDecision, ParallelModeModeCompletionDecision,
    ParallelModeOrchestratorTickDecision, ParallelModePendingDispatchWakeDecision,
    ParallelModePostTurnQueueSignal, ParallelModeProjectionReadyContinuation,
    ParallelModeTickCompletionDecision,
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const MAX_UNSETTLED_DISPATCH_CLEANUPS: usize = 64;
const CANCEL_DISPATCH_COMMAND_IDENTITY: &str = "cancel_runtime_dispatch_commands";

mod composition;
mod controller;
mod effect_runner;
mod host;

pub use composition::{
    ParallelModeControlPlaneComposition, ParallelModeControlPlaneDashboardSnapshot,
};
pub use controller::{
    ParallelModeControlPlanePresentationEvent, ParallelModePostTurnQueueContinuationOutcome,
};
#[cfg(test)]
pub(crate) use effect_runner::parallel_mode_distributor_tick_signature;
pub use effect_runner::{
    ParallelModeControlPlaneBackgroundEvent, ParallelModeControlPlaneEffectRunner,
    ParallelModeControlPlaneEventSink, ParallelModeControlPlaneLoadingStage,
    ParallelModeSupervisorInspectionSnapshot,
};
pub use host::{
    ParallelModeControlPlaneEpochSnapshot, ParallelModeControlPlaneHandle,
    ParallelModeControlPlanePresentationProjection,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModeControlPlaneEffectKind {
    EnterParallelMode,
    RefreshSupervisor,
    RunOrchestrator,
    RunOrchestratorTick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeControlPlaneEffectId {
    pub sequence: u64,
    pub kind: ParallelModeControlPlaneEffectKind,
}

impl ParallelModeControlPlaneEffectId {
    fn new(sequence: u64, kind: ParallelModeControlPlaneEffectKind) -> Self {
        Self { sequence, kind }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeSupervisorInspectionCorrelation {
    pub operation_id: u64,
    pub workspace_directory: String,
    pub epoch_id: Option<u64>,
}

impl ParallelModeSupervisorInspectionCorrelation {
    fn new(operation_id: u64, workspace_directory: String, epoch_id: Option<u64>) -> Self {
        Self {
            operation_id,
            workspace_directory,
            epoch_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModePendingDispatchPollCorrelation {
    pub operation_id: u64,
    pub workspace_directory: String,
    pub epoch_id: u64,
}

impl ParallelModePendingDispatchPollCorrelation {
    fn new(operation_id: u64, workspace_directory: String, epoch_id: u64) -> Self {
        Self {
            operation_id,
            workspace_directory,
            epoch_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeDispatchMutationCorrelation {
    pub operation_id: u64,
    pub workspace_directory: String,
    pub epoch_id: u64,
}

impl ParallelModeDispatchMutationCorrelation {
    fn new(operation_id: u64, workspace_directory: String, epoch_id: u64) -> Self {
        Self {
            operation_id,
            workspace_directory,
            epoch_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeDispatchCleanupCorrelation {
    pub operation_id: u64,
    pub workspace_directory: String,
    pub epoch_id: u64,
    pub command_identity: String,
}

impl ParallelModeDispatchCleanupCorrelation {
    fn from_mutation(correlation: &ParallelModeDispatchMutationCorrelation) -> Self {
        Self {
            operation_id: correlation.operation_id,
            workspace_directory: correlation.workspace_directory.clone(),
            epoch_id: correlation.epoch_id,
            command_identity: CANCEL_DISPATCH_COMMAND_IDENTITY.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeGlobalRuntimeNoticeProjection {
    pub cleanup_correlation: ParallelModeDispatchCleanupCorrelation,
    pub notice: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mutation", rename_all = "snake_case")]
pub enum ParallelModeDispatchMutation {
    EnqueueSlotCapacity,
    EnqueueForTrigger {
        trigger: ParallelModeAutomationTrigger,
        reason: String,
    },
    Cancel {
        reason: String,
    },
    RetryCancel {
        original_cleanup: ParallelModeDispatchCleanupCorrelation,
    },
}

impl ParallelModeDispatchMutation {
    fn is_enqueue(&self) -> bool {
        matches!(
            self,
            Self::EnqueueSlotCapacity | Self::EnqueueForTrigger { .. }
        )
    }

    fn canonical_enqueue_trigger(&self) -> Option<ParallelModeAutomationTrigger> {
        match self {
            Self::EnqueueSlotCapacity => Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
            Self::EnqueueForTrigger { trigger, .. } => Some(*trigger),
            Self::Cancel { .. } | Self::RetryCancel { .. } => None,
        }
    }

    fn has_same_semantic_intent(&self, other: &Self) -> bool {
        if let (Some(left), Some(right)) = (
            self.canonical_enqueue_trigger(),
            other.canonical_enqueue_trigger(),
        ) {
            return left == right;
        }
        match (self, other) {
            (Self::Cancel { .. }, Self::Cancel { .. }) => true,
            (
                Self::RetryCancel {
                    original_cleanup: left,
                },
                Self::RetryCancel {
                    original_cleanup: right,
                },
            ) => left == right,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParallelModeSupervisorInspectionState {
    Idle,
    Loading {
        correlation: ParallelModeSupervisorInspectionCorrelation,
        show_status: bool,
    },
    Ready {
        correlation: ParallelModeSupervisorInspectionCorrelation,
    },
    Failed {
        correlation: ParallelModeSupervisorInspectionCorrelation,
        error: String,
    },
}

impl ParallelModeSupervisorInspectionState {
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeControlPlaneWake {
    pub workspace_directory: String,
    pub trigger: ParallelModeAutomationTrigger,
    pub epoch_id: u64,
    pub enqueue_trigger: Option<ParallelModeAutomationTrigger>,
}

impl ParallelModeControlPlaneWake {
    pub fn new(
        workspace_directory: impl Into<String>,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
        enqueue_trigger: Option<ParallelModeAutomationTrigger>,
    ) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            trigger,
            epoch_id,
            enqueue_trigger,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ParallelModeControlPlaneCommand {
    OpenEpoch {
        workspace_directory: String,
    },
    Enable {
        workspace_directory: String,
    },
    Disable {
        workspace_directory: String,
    },
    InspectSupervisor {
        workspace_directory: String,
        reconcile_pool: bool,
        show_status: bool,
    },
    SupervisorInspectionCompleted {
        correlation: ParallelModeSupervisorInspectionCorrelation,
        succeeded: bool,
    },
    RefreshSupervisor {
        workspace_directory: String,
    },
    WakeOrchestrator(ParallelModeControlPlaneWake),
    RunOrchestratorTick {
        workspace_directory: String,
        signature: String,
    },
    RequestDispatch {
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
    },
    RequestDispatchForEpoch {
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
    },
    ContinuePostTurnQueue {
        workspace_directory: String,
        signal: Option<ParallelModePostTurnQueueSignal>,
        auto_follow_prompt_queued: bool,
        has_actionable_queue_head: bool,
    },
    PollPendingDispatchWake {
        workspace_directory: String,
        follow_up_tick_signature: Option<String>,
    },
    PendingDispatchWakePolled {
        correlation: ParallelModePendingDispatchPollCorrelation,
        result: Result<Option<ParallelModeControlPlaneWake>, String>,
    },
    DispatchMutationCompleted {
        correlation: ParallelModeDispatchMutationCorrelation,
        result: Result<usize, String>,
    },
    RetryUnsettledDispatchCleanup {
        original_cleanup: ParallelModeDispatchCleanupCorrelation,
    },
    WorkerCompleted {
        workspace_directory: String,
        epoch_id: u64,
        trigger: ParallelModeAutomationTrigger,
    },
    WorkerEventReceived {
        event: ParallelModeControlPlaneWorkerEvent,
        has_actionable_queue_head: bool,
    },
    EffectCompleted {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
    },
    EntryCompleted {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        mode_enabled: bool,
        mode_was_enabled: bool,
        initial_pool_reset_completed: bool,
        has_actionable_queue_head: bool,
        follow_up_tick_signature: Option<String>,
    },
    SupervisorSnapshotRefreshCompleted {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        follow_up_tick_signature: Option<String>,
    },
    OrchestratorWakeCompleted {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        mode_enabled: bool,
        follow_up_tick_signature: Option<String>,
    },
    OrchestratorTickCompleted {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        blocked: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ParallelModeControlPlaneEvent {
    EpochOpened {
        workspace_directory: String,
        epoch_id: u64,
    },
    EpochClosed {
        workspace_directory: String,
        epoch_id: u64,
    },
    EffectStarted {
        effect_id: ParallelModeControlPlaneEffectId,
    },
    ModeEnabled {
        workspace_directory: String,
        epoch_id: u64,
    },
    ModeDisabled {
        workspace_directory: String,
    },
    SupervisorInspectionStarted {
        correlation: ParallelModeSupervisorInspectionCorrelation,
        show_status: bool,
    },
    SupervisorInspectionCompleted {
        correlation: ParallelModeSupervisorInspectionCorrelation,
        succeeded: bool,
        projection_current: bool,
    },
    SupervisorRefreshQueued,
    OrchestratorWakeQueued {
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
    },
    OrchestratorWakeDequeued {
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
    },
    DispatchWithheld {
        trigger: Option<ParallelModeAutomationTrigger>,
        reason: String,
    },
    DispatchCommandQueued {
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
        inserted_count: usize,
        reason: String,
    },
    DispatchCommandsCancelled {
        workspace_directory: String,
        epoch_id: u64,
        cancelled_count: usize,
    },
    DispatchMutationFailed {
        operation_id: u64,
        workspace_directory: String,
        epoch_id: u64,
        trigger: Option<ParallelModeAutomationTrigger>,
        reason: String,
        presentation_current: bool,
        cleanup_correlation: Option<ParallelModeDispatchCleanupCorrelation>,
    },
    DispatchCleanupSettled {
        original_cleanup: ParallelModeDispatchCleanupCorrelation,
        retry_operation_id: u64,
    },
    PostTurnAutoFollowPromptConsumed,
    PostTurnDispatchRequested {
        workspace_directory: String,
        epoch_id: u64,
    },
    ConversationRuntimeNotice {
        workspace_directory: String,
        notice: String,
    },
    WorkerCompleted {
        workspace_directory: String,
        epoch_id: u64,
        task_id: String,
    },
    WorkerLaunchFailed {
        workspace_directory: String,
        epoch_id: u64,
        task_id: String,
    },
    WorkerStreamFailed {
        workspace_directory: String,
        epoch_id: u64,
        task_id: String,
    },
    EffectCompleted {
        effect_id: ParallelModeControlPlaneEffectId,
    },
    StaleCommandDropped {
        workspace_directory: String,
        epoch_id: u64,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum ParallelModeControlPlaneEffect {
    EnterParallelMode {
        effect_id: ParallelModeControlPlaneEffectId,
        workspace_directory: String,
        epoch_id: u64,
        mode_was_enabled: bool,
        initial_pool_reset_required: bool,
    },
    RefreshSupervisor {
        effect_id: ParallelModeControlPlaneEffectId,
        workspace_directory: String,
        epoch_id: u64,
    },
    InspectSupervisor {
        correlation: ParallelModeSupervisorInspectionCorrelation,
        mode_enabled: bool,
        reconcile_pool: bool,
    },
    RunOrchestrator {
        effect_id: ParallelModeControlPlaneEffectId,
        wake: ParallelModeControlPlaneWake,
    },
    RunOrchestratorTick {
        effect_id: ParallelModeControlPlaneEffectId,
        workspace_directory: String,
        epoch_id: u64,
        signature: String,
    },
    PollPendingDispatchWake {
        correlation: ParallelModePendingDispatchPollCorrelation,
    },
    MutateDispatchCommands {
        correlation: ParallelModeDispatchMutationCorrelation,
        mutation: ParallelModeDispatchMutation,
    },
}

impl ParallelModeControlPlaneEffect {
    pub fn effect_id(&self) -> Option<ParallelModeControlPlaneEffectId> {
        match self {
            Self::EnterParallelMode { effect_id, .. }
            | Self::RefreshSupervisor { effect_id, .. }
            | Self::RunOrchestrator { effect_id, .. }
            | Self::RunOrchestratorTick { effect_id, .. } => Some(*effect_id),
            Self::InspectSupervisor { .. }
            | Self::PollPendingDispatchWake { .. }
            | Self::MutateDispatchCommands { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeControlPlaneRuntimeOutcome {
    pub events: Vec<ParallelModeControlPlaneEvent>,
    pub effects: Vec<ParallelModeControlPlaneEffect>,
}

impl ParallelModeControlPlaneRuntimeOutcome {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            effects: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeControlPlaneRuntimeStore {
    /*
     * This store is process-lifetime control-plane memory, not recoverable state.
     * Restart loss is acceptable because durable work lives in PlanningAuthorityPort
     * projections: dispatch commands, slot leases, session detail, task dispatch
     * blocks, distributor queue records, official-refresh claims, runtime events,
     * planning authority, and task provenance. A fresh process must reopen an
     * epoch explicitly, then read those durable rows before scheduling effects.
     */
    workspace_directory: Option<String>,
    mode_enabled: bool,
    initial_pool_reset_completed: bool,
    current_epoch_id: Option<u64>,
    next_epoch_id: u64,
    parallel_entry_in_flight: Option<ParallelModeControlPlaneEffectId>,
    supervisor_refresh_in_flight: Option<ParallelModeControlPlaneEffectId>,
    orchestrator_wake_in_flight: Option<ParallelModeControlPlaneEffectId>,
    orchestrator_tick_in_flight: Option<ParallelModeControlPlaneEffectId>,
    projection_ready: bool,
    pending_supervisor_refresh: bool,
    pending_orchestrator_wake: Option<ParallelModeControlPlaneWake>,
    pending_parallel_entry: Option<ParallelModePendingEntry>,
    last_orchestrator_tick_signature: Option<String>,
    supervisor_inspection_in_flight: Option<ParallelModeSupervisorInspectionInFlight>,
    pending_supervisor_inspection: Option<ParallelModeSupervisorInspectionIntent>,
    next_supervisor_inspection_operation_id: u64,
    pending_dispatch_poll_in_flight: Option<ParallelModePendingDispatchPollInFlight>,
    pending_dispatch_poll: Option<ParallelModePendingDispatchPollIntent>,
    next_pending_dispatch_poll_operation_id: u64,
    dispatch_mutation_in_flight: Option<ParallelModeDispatchMutationInFlight>,
    pending_dispatch_mutations: VecDeque<ParallelModeDispatchMutationIntent>,
    unsettled_dispatch_cleanups: VecDeque<ParallelModeUnsettledDispatchCleanup>,
    next_dispatch_mutation_operation_id: u64,
    next_effect_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelModePendingEntry {
    workspace_directory: String,
    epoch_id: u64,
    mode_was_enabled: bool,
    initial_pool_reset_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelModeSupervisorInspectionIntent {
    workspace_directory: String,
    reconcile_pool: bool,
    show_status: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelModeSupervisorInspectionInFlight {
    correlation: ParallelModeSupervisorInspectionCorrelation,
    mode_enabled: bool,
    reconcile_pool: bool,
    show_status: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelModePendingDispatchPollInFlight {
    correlation: ParallelModePendingDispatchPollCorrelation,
    follow_up_tick_signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelModePendingDispatchPollIntent {
    workspace_directory: String,
    epoch_id: u64,
    follow_up_tick_signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelModeDispatchMutationIntent {
    workspace_directory: String,
    epoch_id: u64,
    mutation: ParallelModeDispatchMutation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelModeDispatchMutationInFlight {
    correlation: ParallelModeDispatchMutationCorrelation,
    mutation: ParallelModeDispatchMutation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelModeUnsettledDispatchCleanup {
    correlation: ParallelModeDispatchCleanupCorrelation,
    error: String,
}

impl ParallelModeUnsettledDispatchCleanup {
    fn global_runtime_notice_projection(&self) -> ParallelModeGlobalRuntimeNoticeProjection {
        let correlation = &self.correlation;
        ParallelModeGlobalRuntimeNoticeProjection {
            cleanup_correlation: correlation.clone(),
            notice: format!(
                "parallel mode: dispatch cleanup remains unsettled / workspace: {} / epoch: {} / operation: {} / command: {} / {} / retry this exact cleanup correlation",
                correlation.workspace_directory,
                correlation.epoch_id,
                correlation.operation_id,
                correlation.command_identity,
                self.error,
            ),
        }
    }
}

struct ParallelModeEntryCompletion {
    workspace_directory: String,
    epoch_id: u64,
    effect_id: ParallelModeControlPlaneEffectId,
    mode_enabled: bool,
    mode_was_enabled: bool,
    initial_pool_reset_completed: bool,
    has_actionable_queue_head: bool,
    follow_up_tick_signature: Option<String>,
}

impl Default for ParallelModeControlPlaneRuntimeStore {
    fn default() -> Self {
        Self {
            workspace_directory: None,
            mode_enabled: false,
            initial_pool_reset_completed: false,
            current_epoch_id: None,
            next_epoch_id: 1,
            parallel_entry_in_flight: None,
            supervisor_refresh_in_flight: None,
            orchestrator_wake_in_flight: None,
            orchestrator_tick_in_flight: None,
            projection_ready: false,
            pending_supervisor_refresh: false,
            pending_orchestrator_wake: None,
            pending_parallel_entry: None,
            last_orchestrator_tick_signature: None,
            supervisor_inspection_in_flight: None,
            pending_supervisor_inspection: None,
            next_supervisor_inspection_operation_id: 1,
            pending_dispatch_poll_in_flight: None,
            pending_dispatch_poll: None,
            next_pending_dispatch_poll_operation_id: 1,
            dispatch_mutation_in_flight: None,
            pending_dispatch_mutations: VecDeque::new(),
            unsettled_dispatch_cleanups: VecDeque::new(),
            next_dispatch_mutation_operation_id: 1,
            next_effect_sequence: 1,
        }
    }
}

impl ParallelModeControlPlaneRuntimeStore {
    fn effect_is_in_flight(&self, effect_id: ParallelModeControlPlaneEffectId) -> bool {
        match effect_id.kind {
            ParallelModeControlPlaneEffectKind::EnterParallelMode => {
                self.parallel_entry_in_flight == Some(effect_id)
            }
            ParallelModeControlPlaneEffectKind::RefreshSupervisor => {
                self.supervisor_refresh_in_flight == Some(effect_id)
            }
            ParallelModeControlPlaneEffectKind::RunOrchestrator => {
                self.orchestrator_wake_in_flight == Some(effect_id)
            }
            ParallelModeControlPlaneEffectKind::RunOrchestratorTick => {
                self.orchestrator_tick_in_flight == Some(effect_id)
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ParallelModeControlPlaneRuntime {
    store: ParallelModeControlPlaneRuntimeStore,
}

impl ParallelModeControlPlaneRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn store(&self) -> &ParallelModeControlPlaneRuntimeStore {
        &self.store
    }

    fn oldest_unsettled_dispatch_cleanup(&self) -> Option<ParallelModeDispatchCleanupCorrelation> {
        self.store
            .unsettled_dispatch_cleanups
            .front()
            .map(|cleanup| cleanup.correlation.clone())
    }

    fn global_runtime_notice_projection(&self) -> Vec<ParallelModeGlobalRuntimeNoticeProjection> {
        self.store
            .unsettled_dispatch_cleanups
            .iter()
            .map(ParallelModeUnsettledDispatchCleanup::global_runtime_notice_projection)
            .collect()
    }

    pub fn reset_orchestrator_tick_signature(&mut self) {
        self.store.last_orchestrator_tick_signature = None;
    }

    #[cfg(test)]
    pub fn force_epoch_for_test(&mut self, workspace_directory: impl Into<String>, epoch_id: u64) {
        self.store.workspace_directory = Some(workspace_directory.into());
        self.store.mode_enabled = true;
        self.store.current_epoch_id = Some(epoch_id);
        self.store.next_epoch_id = self.store.next_epoch_id.max(epoch_id.saturating_add(1));
    }

    pub fn mode_enabled(&self) -> bool {
        self.store.mode_enabled
    }

    #[cfg(test)]
    pub fn force_mode_for_test(&mut self, workspace_directory: impl Into<String>, enabled: bool) {
        let workspace_directory = workspace_directory.into();
        if enabled {
            self.ensure_epoch(
                workspace_directory,
                &mut ParallelModeControlPlaneRuntimeOutcome::new(),
            );
            self.store.mode_enabled = true;
            self.store.projection_ready = true;
        } else {
            self.store.mode_enabled = false;
            self.store.workspace_directory = None;
            self.store.current_epoch_id = None;
            self.clear_process_effect_state();
        }
    }

    #[cfg(test)]
    pub fn force_initial_pool_reset_completed_for_test(&mut self, completed: bool) {
        self.store.initial_pool_reset_completed = completed;
    }

    #[cfg(test)]
    pub fn force_supervisor_refresh_in_flight_for_test(
        &mut self,
        workspace_directory: impl Into<String>,
        epoch_id: u64,
    ) -> ParallelModeControlPlaneEffectId {
        self.force_epoch_for_test(workspace_directory, epoch_id);
        let effect_id = self.next_effect_id(ParallelModeControlPlaneEffectKind::RefreshSupervisor);
        self.store.supervisor_refresh_in_flight = Some(effect_id);
        effect_id
    }

    #[cfg(test)]
    pub fn force_parallel_entry_in_flight_for_test(
        &mut self,
        workspace_directory: impl Into<String>,
        epoch_id: u64,
    ) -> ParallelModeControlPlaneEffectId {
        self.force_epoch_for_test(workspace_directory, epoch_id);
        self.store.projection_ready = false;
        let effect_id = self.next_effect_id(ParallelModeControlPlaneEffectKind::EnterParallelMode);
        self.store.parallel_entry_in_flight = Some(effect_id);
        effect_id
    }

    pub fn handle(
        &mut self,
        command: ParallelModeControlPlaneCommand,
    ) -> ParallelModeControlPlaneRuntimeOutcome {
        let mut outcome = ParallelModeControlPlaneRuntimeOutcome::new();
        match command {
            ParallelModeControlPlaneCommand::OpenEpoch {
                workspace_directory,
            } => self.open_epoch(workspace_directory, &mut outcome),
            ParallelModeControlPlaneCommand::Enable {
                workspace_directory,
            } => self.enable(workspace_directory, &mut outcome),
            ParallelModeControlPlaneCommand::Disable {
                workspace_directory,
            } => self.disable(workspace_directory, &mut outcome),
            ParallelModeControlPlaneCommand::InspectSupervisor {
                workspace_directory,
                reconcile_pool,
                show_status,
            } => self.inspect_supervisor(
                workspace_directory,
                reconcile_pool,
                show_status,
                &mut outcome,
            ),
            ParallelModeControlPlaneCommand::SupervisorInspectionCompleted {
                correlation,
                succeeded,
            } => self.supervisor_inspection_completed(correlation, succeeded, &mut outcome),
            ParallelModeControlPlaneCommand::RefreshSupervisor {
                workspace_directory,
            } => self.refresh_supervisor(workspace_directory, &mut outcome),
            ParallelModeControlPlaneCommand::WakeOrchestrator(wake) => {
                self.wake_orchestrator(wake, &mut outcome)
            }
            ParallelModeControlPlaneCommand::RunOrchestratorTick {
                workspace_directory,
                signature,
            } => self.run_orchestrator_tick(workspace_directory, signature, &mut outcome),
            ParallelModeControlPlaneCommand::RequestDispatch {
                workspace_directory,
                trigger,
            } => self.request_dispatch(workspace_directory, trigger, None, &mut outcome),
            ParallelModeControlPlaneCommand::RequestDispatchForEpoch {
                workspace_directory,
                trigger,
                epoch_id,
            } => self.request_dispatch(workspace_directory, trigger, Some(epoch_id), &mut outcome),
            ParallelModeControlPlaneCommand::ContinuePostTurnQueue {
                workspace_directory,
                signal,
                auto_follow_prompt_queued,
                has_actionable_queue_head,
            } => self.continue_post_turn_queue(
                workspace_directory,
                signal,
                auto_follow_prompt_queued,
                has_actionable_queue_head,
                &mut outcome,
            ),
            ParallelModeControlPlaneCommand::PollPendingDispatchWake {
                workspace_directory,
                follow_up_tick_signature,
            } => self.poll_pending_dispatch_wake(
                workspace_directory,
                follow_up_tick_signature,
                &mut outcome,
            ),
            ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
                correlation,
                result,
            } => self.pending_dispatch_wake_polled(correlation, result, &mut outcome),
            ParallelModeControlPlaneCommand::DispatchMutationCompleted {
                correlation,
                result,
            } => self.dispatch_mutation_completed(correlation, result, &mut outcome),
            ParallelModeControlPlaneCommand::RetryUnsettledDispatchCleanup { original_cleanup } => {
                self.retry_unsettled_dispatch_cleanup(original_cleanup, &mut outcome)
            }
            ParallelModeControlPlaneCommand::WorkerCompleted {
                workspace_directory,
                epoch_id,
                trigger,
            } => self.wake_orchestrator(
                ParallelModeControlPlaneWake::new(workspace_directory, trigger, epoch_id, None),
                &mut outcome,
            ),
            ParallelModeControlPlaneCommand::WorkerEventReceived {
                event,
                has_actionable_queue_head,
            } => self.worker_event_received(event, has_actionable_queue_head, &mut outcome),
            ParallelModeControlPlaneCommand::EffectCompleted {
                workspace_directory,
                epoch_id,
                effect_id,
            } => self.effect_completed(workspace_directory, epoch_id, effect_id, &mut outcome),
            ParallelModeControlPlaneCommand::EntryCompleted {
                workspace_directory,
                epoch_id,
                effect_id,
                mode_enabled,
                mode_was_enabled,
                initial_pool_reset_completed,
                has_actionable_queue_head,
                follow_up_tick_signature,
            } => self.entry_completed(
                ParallelModeEntryCompletion {
                    workspace_directory,
                    epoch_id,
                    effect_id,
                    mode_enabled,
                    mode_was_enabled,
                    initial_pool_reset_completed,
                    has_actionable_queue_head,
                    follow_up_tick_signature,
                },
                &mut outcome,
            ),
            ParallelModeControlPlaneCommand::SupervisorSnapshotRefreshCompleted {
                workspace_directory,
                epoch_id,
                effect_id,
                follow_up_tick_signature,
            } => self.supervisor_snapshot_refresh_completed(
                workspace_directory,
                epoch_id,
                effect_id,
                follow_up_tick_signature,
                &mut outcome,
            ),
            ParallelModeControlPlaneCommand::OrchestratorWakeCompleted {
                workspace_directory,
                epoch_id,
                effect_id,
                mode_enabled,
                follow_up_tick_signature,
            } => self.orchestrator_wake_completed(
                workspace_directory,
                epoch_id,
                effect_id,
                mode_enabled,
                follow_up_tick_signature,
                &mut outcome,
            ),
            ParallelModeControlPlaneCommand::OrchestratorTickCompleted {
                workspace_directory,
                epoch_id,
                effect_id,
                blocked,
            } => self.orchestrator_tick_completed(
                workspace_directory,
                epoch_id,
                effect_id,
                blocked,
                &mut outcome,
            ),
        }
        if !self.start_pending_parallel_entry_if_idle(&mut outcome) {
            self.start_pending_supervisor_inspection_if_idle(&mut outcome);
        }
        outcome
    }

    fn open_epoch(
        &mut self,
        workspace_directory: String,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        self.ensure_epoch(workspace_directory, outcome);
    }

    fn enable(
        &mut self,
        workspace_directory: String,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let entry_decision = ParallelModeControlPlaneAggregate::enable_entry(
            self.store.mode_enabled,
            self.store.workspace_directory.as_deref(),
            &workspace_directory,
            self.store.initial_pool_reset_completed,
        );
        let epoch_id = self.ensure_epoch(workspace_directory.clone(), outcome);
        self.store.mode_enabled = true;
        self.store.workspace_directory = Some(workspace_directory.clone());
        self.store.last_orchestrator_tick_signature = None;
        self.store.projection_ready = false;
        outcome
            .events
            .push(ParallelModeControlPlaneEvent::ModeEnabled {
                workspace_directory: workspace_directory.clone(),
                epoch_id,
            });
        self.start_parallel_entry(
            workspace_directory,
            epoch_id,
            entry_decision.mode_was_enabled,
            entry_decision.initial_pool_reset_required,
            outcome,
        );
    }

    fn disable(
        &mut self,
        workspace_directory: String,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if let Some(current_workspace) = self.store.workspace_directory.as_deref()
            && current_workspace != workspace_directory
        {
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::StaleCommandDropped {
                    workspace_directory,
                    epoch_id: 0,
                    reason: "disable command targets a different workspace".to_string(),
                });
            return;
        }
        let closed_epoch_id = self.store.current_epoch_id.take();
        if let Some(epoch_id) = closed_epoch_id {
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::EpochClosed {
                    workspace_directory: workspace_directory.clone(),
                    epoch_id,
                });
        }
        self.store.mode_enabled = false;
        self.store.workspace_directory = None;
        self.store.projection_ready = false;
        self.clear_process_effect_state();
        outcome
            .events
            .push(ParallelModeControlPlaneEvent::ModeDisabled {
                workspace_directory: workspace_directory.clone(),
            });
        if let Some(epoch_id) = closed_epoch_id {
            self.schedule_dispatch_mutation(
                workspace_directory,
                epoch_id,
                ParallelModeDispatchMutation::Cancel {
                    reason: "parallel mode disabled".to_string(),
                },
                outcome,
            );
        }
    }

    fn inspect_supervisor(
        &mut self,
        workspace_directory: String,
        reconcile_pool: bool,
        show_status: bool,
        _outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let decision = ParallelModeControlPlaneAggregate::supervisor_inspection(
            self.store.mode_enabled,
            self.store.workspace_directory.as_deref(),
            &workspace_directory,
            reconcile_pool,
        );
        let epoch_id = self.supervisor_inspection_epoch_for_workspace(&workspace_directory);
        if let Some(in_flight) = self.store.supervisor_inspection_in_flight.as_ref() {
            let same_context = in_flight.correlation.workspace_directory == workspace_directory
                && in_flight.correlation.epoch_id == epoch_id;
            let requests_stronger_inspection = decision.reconcile_pool && !in_flight.reconcile_pool
                || show_status && !in_flight.show_status;
            if same_context && !requests_stronger_inspection {
                return;
            }
        }

        let intent = ParallelModeSupervisorInspectionIntent {
            workspace_directory,
            reconcile_pool,
            show_status,
        };
        if let Some(pending) = self.store.pending_supervisor_inspection.as_mut()
            && pending.workspace_directory == intent.workspace_directory
        {
            pending.reconcile_pool |= intent.reconcile_pool;
            pending.show_status |= intent.show_status;
        } else {
            self.store.pending_supervisor_inspection = Some(intent);
        }
    }

    fn supervisor_inspection_completed(
        &mut self,
        correlation: ParallelModeSupervisorInspectionCorrelation,
        succeeded: bool,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let Some(in_flight) = self
            .store
            .supervisor_inspection_in_flight
            .as_ref()
            .filter(|in_flight| in_flight.correlation == correlation)
        else {
            self.stale_command(
                correlation.workspace_directory,
                correlation.epoch_id.unwrap_or(0),
                "unknown parallel supervisor inspection",
                outcome,
            );
            return;
        };
        let projection_current = in_flight.mode_enabled
            == ParallelModeControlPlaneAggregate::mode_enabled_for_workspace(
                self.store.mode_enabled,
                self.store.workspace_directory.as_deref(),
                &correlation.workspace_directory,
            );
        if !self.supervisor_inspection_context_is_current(&correlation) {
            self.store.supervisor_inspection_in_flight = None;
            self.store.pending_supervisor_inspection = None;
            self.stale_command(
                correlation.workspace_directory,
                correlation.epoch_id.unwrap_or(0),
                "parallel supervisor inspection context is stale",
                outcome,
            );
            return;
        }
        self.store.supervisor_inspection_in_flight = None;
        outcome.events.push(
            ParallelModeControlPlaneEvent::SupervisorInspectionCompleted {
                correlation,
                succeeded,
                projection_current,
            },
        );
        if let (Some(workspace_directory), Some(epoch_id)) = (
            self.store.workspace_directory.clone(),
            self.store.current_epoch_id,
        ) {
            self.continue_after_effect_completed(workspace_directory, epoch_id, outcome);
        }
    }

    fn refresh_supervisor(
        &mut self,
        workspace_directory: String,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let Some(epoch_id) = self.current_epoch_for_workspace(&workspace_directory, outcome) else {
            return;
        };
        self.start_or_queue_supervisor_refresh(workspace_directory, epoch_id, outcome);
    }

    fn wake_orchestrator(
        &mut self,
        wake: ParallelModeControlPlaneWake,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if !self.command_epoch_is_current(&wake.workspace_directory, wake.epoch_id, outcome) {
            return;
        }
        match ParallelModeControlPlaneAggregate::effect_start_decision(self.has_in_flight_effect())
        {
            ParallelModeEffectStartDecision::StartNow => {
                self.start_orchestrator_wake(wake, outcome)
            }
            ParallelModeEffectStartDecision::QueueUntilIdle => {
                self.store.pending_orchestrator_wake = Some(wake.clone());
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::OrchestratorWakeQueued {
                        trigger: wake.trigger,
                        epoch_id: wake.epoch_id,
                    });
            }
        }
    }

    fn effect_completed(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let unknown_reason = unknown_effect_reason(effect_id.kind);
        if !self.finish_effect(
            &workspace_directory,
            epoch_id,
            effect_id,
            effect_id.kind,
            unknown_reason,
            outcome,
        ) {
            return;
        }
        self.continue_after_effect_completed(workspace_directory, epoch_id, outcome);
    }

    fn entry_completed(
        &mut self,
        completion: ParallelModeEntryCompletion,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let ParallelModeEntryCompletion {
            workspace_directory,
            epoch_id,
            effect_id,
            mode_enabled,
            mode_was_enabled,
            initial_pool_reset_completed,
            has_actionable_queue_head,
            follow_up_tick_signature,
        } = completion;
        if !self.command_epoch_is_current(&workspace_directory, epoch_id, outcome) {
            return;
        }
        if self.store.parallel_entry_in_flight != Some(effect_id) {
            self.stale_command(
                workspace_directory,
                epoch_id,
                "unknown parallel entry",
                outcome,
            );
            return;
        }

        self.store.parallel_entry_in_flight = None;
        self.store.mode_enabled = mode_enabled;
        if initial_pool_reset_completed {
            self.store.initial_pool_reset_completed = true;
        }
        outcome
            .events
            .push(ParallelModeControlPlaneEvent::EffectCompleted { effect_id });

        match ParallelModeControlPlaneAggregate::entry_completion(
            mode_enabled,
            mode_was_enabled,
            self.store.pending_supervisor_refresh,
            has_actionable_queue_head,
        ) {
            ParallelModeEntryCompletionDecision::CloseEpoch => {
                self.store.current_epoch_id = None;
                self.store.workspace_directory = None;
                self.store.projection_ready = false;
                self.clear_process_effect_state();
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::EpochClosed {
                        workspace_directory: workspace_directory.clone(),
                        epoch_id,
                    });
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::ModeDisabled {
                        workspace_directory,
                    });
                return;
            }
            ParallelModeEntryCompletionDecision::RefreshSupervisor => {
                self.store.pending_supervisor_refresh = false;
                self.start_supervisor_refresh(workspace_directory, epoch_id, outcome);
                return;
            }
            ParallelModeEntryCompletionDecision::DispatchInitialQueue => {
                self.store.projection_ready = true;
                self.start_orchestrator_wake(
                    ParallelModeControlPlaneWake::new(
                        workspace_directory,
                        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
                        epoch_id,
                        Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
                    ),
                    outcome,
                );
                return;
            }
            ParallelModeEntryCompletionDecision::ProjectionReady => {
                self.store.projection_ready = true;
            }
        }
        self.schedule_after_projection_ready(
            workspace_directory,
            epoch_id,
            follow_up_tick_signature,
            outcome,
        );
    }

    fn supervisor_snapshot_refresh_completed(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        follow_up_tick_signature: Option<String>,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if !self.finish_effect(
            &workspace_directory,
            epoch_id,
            effect_id,
            ParallelModeControlPlaneEffectKind::RefreshSupervisor,
            "unknown supervisor refresh",
            outcome,
        ) {
            return;
        }
        self.store.projection_ready = true;
        self.schedule_after_projection_ready(
            workspace_directory,
            epoch_id,
            follow_up_tick_signature,
            outcome,
        );
    }

    fn orchestrator_wake_completed(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        mode_enabled: bool,
        follow_up_tick_signature: Option<String>,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if !self.finish_effect(
            &workspace_directory,
            epoch_id,
            effect_id,
            ParallelModeControlPlaneEffectKind::RunOrchestrator,
            "unknown orchestrator wake",
            outcome,
        ) {
            return;
        }
        self.store.mode_enabled = mode_enabled;
        if ParallelModeControlPlaneAggregate::mode_completion(mode_enabled)
            == ParallelModeModeCompletionDecision::CloseEpoch
        {
            self.store.current_epoch_id = None;
            self.store.workspace_directory = None;
            self.store.projection_ready = false;
            self.clear_process_effect_state();
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::EpochClosed {
                    workspace_directory: workspace_directory.clone(),
                    epoch_id,
                });
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::ModeDisabled {
                    workspace_directory,
                });
            return;
        }
        self.store.projection_ready = true;
        self.schedule_after_projection_ready(
            workspace_directory,
            epoch_id,
            follow_up_tick_signature,
            outcome,
        );
    }

    fn orchestrator_tick_completed(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        blocked: bool,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if !self.finish_effect(
            &workspace_directory,
            epoch_id,
            effect_id,
            ParallelModeControlPlaneEffectKind::RunOrchestratorTick,
            "unknown orchestrator tick",
            outcome,
        ) {
            return;
        }
        self.store.projection_ready = false;
        self.start_or_queue_supervisor_refresh(workspace_directory.clone(), epoch_id, outcome);
        if ParallelModeControlPlaneAggregate::tick_completion(blocked)
            == ParallelModeTickCompletionDecision::RefreshSupervisorAndQueueCapacityDispatch
        {
            self.schedule_dispatch_mutation(
                workspace_directory,
                epoch_id,
                ParallelModeDispatchMutation::EnqueueSlotCapacity,
                outcome,
            );
        }
    }

    fn worker_event_received(
        &mut self,
        event: ParallelModeControlPlaneWorkerEvent,
        has_actionable_queue_head: bool,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let decision = ParallelModeControlPlaneAggregate::worker_event_decision(
            &event.workspace_directory,
            event.epoch_id,
            event.kind,
            self.store.workspace_directory.as_deref(),
            self.store.current_epoch_id,
            has_actionable_queue_head,
        );
        if let Some(reason) = decision.stale_drop_reason {
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::StaleCommandDropped {
                    workspace_directory: event.workspace_directory,
                    epoch_id: event.epoch_id,
                    reason: reason.to_string(),
                });
            return;
        }
        outcome
            .events
            .push(worker_event_to_control_plane_event(&event));
        let workspace_directory = event.workspace_directory.clone();
        for notice in event.notices {
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::ConversationRuntimeNotice {
                    workspace_directory: workspace_directory.clone(),
                    notice,
                });
        }
        if decision.refresh_supervisor {
            self.start_or_queue_supervisor_refresh(
                event.workspace_directory.clone(),
                event.epoch_id,
                outcome,
            );
        }
        if let Some(trigger) = decision.wake_trigger {
            self.wake_orchestrator(
                ParallelModeControlPlaneWake::new(
                    event.workspace_directory,
                    trigger,
                    event.epoch_id,
                    Some(trigger),
                ),
                outcome,
            );
        }
    }

    fn start_parallel_entry(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        mode_was_enabled: bool,
        initial_pool_reset_required: bool,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        match ParallelModeControlPlaneAggregate::effect_start_decision(self.has_in_flight_effect())
        {
            ParallelModeEffectStartDecision::StartNow => {
                let effect_id =
                    self.next_effect_id(ParallelModeControlPlaneEffectKind::EnterParallelMode);
                self.store.parallel_entry_in_flight = Some(effect_id);
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::EffectStarted { effect_id });
                outcome
                    .effects
                    .push(ParallelModeControlPlaneEffect::EnterParallelMode {
                        effect_id,
                        workspace_directory,
                        epoch_id,
                        mode_was_enabled,
                        initial_pool_reset_required,
                    });
            }
            ParallelModeEffectStartDecision::QueueUntilIdle => {
                if self.store.supervisor_inspection_in_flight.is_some()
                    || self.store.pending_supervisor_inspection.is_some()
                    || self.store.dispatch_mutation_in_flight.is_some()
                    || !self.store.pending_dispatch_mutations.is_empty()
                {
                    let pending_entry = ParallelModePendingEntry {
                        workspace_directory,
                        epoch_id,
                        mode_was_enabled,
                        initial_pool_reset_required,
                    };
                    if let Some(pending) = self.store.pending_parallel_entry.as_mut()
                        && pending.workspace_directory == pending_entry.workspace_directory
                        && pending.epoch_id == pending_entry.epoch_id
                    {
                        pending.mode_was_enabled &= pending_entry.mode_was_enabled;
                        pending.initial_pool_reset_required |=
                            pending_entry.initial_pool_reset_required;
                    } else {
                        self.store.pending_parallel_entry = Some(pending_entry);
                    }
                } else {
                    outcome
                        .events
                        .push(ParallelModeControlPlaneEvent::SupervisorRefreshQueued);
                    self.store.pending_supervisor_refresh = true;
                }
            }
        }
    }

    fn start_or_queue_supervisor_refresh(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        match ParallelModeControlPlaneAggregate::effect_start_decision(self.has_in_flight_effect())
        {
            ParallelModeEffectStartDecision::StartNow => {
                self.start_supervisor_refresh(workspace_directory, epoch_id, outcome);
            }
            ParallelModeEffectStartDecision::QueueUntilIdle => {
                self.store.pending_supervisor_refresh = true;
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::SupervisorRefreshQueued);
            }
        }
    }

    fn start_supervisor_refresh(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let effect_id = self.next_effect_id(ParallelModeControlPlaneEffectKind::RefreshSupervisor);
        self.store.supervisor_refresh_in_flight = Some(effect_id);
        self.store.projection_ready = false;
        outcome
            .events
            .push(ParallelModeControlPlaneEvent::EffectStarted { effect_id });
        outcome
            .effects
            .push(ParallelModeControlPlaneEffect::RefreshSupervisor {
                effect_id,
                workspace_directory,
                epoch_id,
            });
    }

    fn start_orchestrator_wake(
        &mut self,
        wake: ParallelModeControlPlaneWake,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let effect_id = self.next_effect_id(ParallelModeControlPlaneEffectKind::RunOrchestrator);
        self.store.orchestrator_wake_in_flight = Some(effect_id);
        outcome
            .events
            .push(ParallelModeControlPlaneEvent::EffectStarted { effect_id });
        outcome
            .effects
            .push(ParallelModeControlPlaneEffect::RunOrchestrator { effect_id, wake });
    }

    fn run_orchestrator_tick(
        &mut self,
        workspace_directory: String,
        signature: String,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let Some(epoch_id) = self.current_epoch_for_workspace(&workspace_directory, outcome) else {
            return;
        };
        if ParallelModeControlPlaneAggregate::orchestrator_tick_decision(
            self.has_in_flight_effect(),
            self.store.last_orchestrator_tick_signature.as_deref(),
            &signature,
        ) == ParallelModeOrchestratorTickDecision::Skip
        {
            return;
        }

        let effect_id =
            self.next_effect_id(ParallelModeControlPlaneEffectKind::RunOrchestratorTick);
        self.store.orchestrator_tick_in_flight = Some(effect_id);
        self.store.last_orchestrator_tick_signature = Some(signature.clone());
        outcome
            .events
            .push(ParallelModeControlPlaneEvent::EffectStarted { effect_id });
        outcome
            .effects
            .push(ParallelModeControlPlaneEffect::RunOrchestratorTick {
                effect_id,
                workspace_directory,
                epoch_id,
                signature,
            });
    }

    fn request_dispatch(
        &mut self,
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
        expected_epoch_id: Option<u64>,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let Some(epoch_id) = self.current_epoch_for_workspace(&workspace_directory, outcome) else {
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::DispatchWithheld {
                    trigger: Some(trigger),
                    reason: "automation epoch is not open".to_string(),
                });
            return;
        };
        if let Some(expected_epoch_id) = expected_epoch_id
            && expected_epoch_id != epoch_id
        {
            self.stale_command(
                workspace_directory,
                expected_epoch_id,
                "stale automation epoch",
                outcome,
            );
            return;
        }
        if let Some(reason) = ParallelModeControlPlaneAggregate::dispatch_readiness(
            self.store.projection_ready,
            self.has_in_flight_effect(),
        )
        .deferred_reason()
        {
            self.schedule_dispatch_mutation(
                workspace_directory,
                epoch_id,
                ParallelModeDispatchMutation::EnqueueForTrigger {
                    trigger,
                    reason: reason.to_string(),
                },
                outcome,
            );
            return;
        }
        self.start_orchestrator_wake(
            ParallelModeControlPlaneWake::new(
                workspace_directory,
                trigger,
                epoch_id,
                Some(trigger),
            ),
            outcome,
        );
    }

    fn continue_post_turn_queue(
        &mut self,
        workspace_directory: String,
        signal: Option<ParallelModePostTurnQueueSignal>,
        auto_follow_prompt_queued: bool,
        has_actionable_queue_head: bool,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let signal = if auto_follow_prompt_queued {
            Some(ParallelModePostTurnQueueSignal::AutoFollowQueued)
        } else {
            signal
        };
        let mode_enabled = ParallelModeControlPlaneAggregate::mode_enabled_for_workspace(
            self.store.mode_enabled,
            self.store.workspace_directory.as_deref(),
            &workspace_directory,
        );
        let decision = ParallelModeControlPlaneAggregate::post_turn_queue_continuation(
            mode_enabled,
            signal,
            has_actionable_queue_head,
        );
        let Some(trigger) = decision.dispatch_trigger() else {
            return;
        };

        if decision.should_consume_auto_follow_prompt() {
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::PostTurnAutoFollowPromptConsumed);
        }
        let epoch_id = self.ensure_epoch(workspace_directory.clone(), outcome);
        outcome
            .events
            .push(ParallelModeControlPlaneEvent::PostTurnDispatchRequested {
                workspace_directory: workspace_directory.clone(),
                epoch_id,
            });
        self.request_dispatch(workspace_directory, trigger, Some(epoch_id), outcome);
    }

    fn poll_pending_dispatch_wake(
        &mut self,
        workspace_directory: String,
        follow_up_tick_signature: Option<String>,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let Some(epoch_id) = self.current_epoch_for_workspace(&workspace_directory, outcome) else {
            return;
        };
        if let Some(in_flight) = self.store.pending_dispatch_poll_in_flight.as_mut() {
            if in_flight.correlation.workspace_directory == workspace_directory
                && in_flight.correlation.epoch_id == epoch_id
                && follow_up_tick_signature.is_some()
            {
                in_flight.follow_up_tick_signature = follow_up_tick_signature;
            }
            return;
        }
        if self.has_in_flight_effect() {
            self.queue_pending_dispatch_poll(
                workspace_directory,
                epoch_id,
                follow_up_tick_signature,
            );
            return;
        }
        self.start_pending_dispatch_poll(
            workspace_directory,
            epoch_id,
            follow_up_tick_signature,
            outcome,
        );
    }

    fn pending_dispatch_wake_polled(
        &mut self,
        correlation: ParallelModePendingDispatchPollCorrelation,
        result: Result<Option<ParallelModeControlPlaneWake>, String>,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let Some(in_flight) = self
            .store
            .pending_dispatch_poll_in_flight
            .as_ref()
            .filter(|in_flight| in_flight.correlation == correlation)
            .cloned()
        else {
            self.stale_command(
                correlation.workspace_directory,
                correlation.epoch_id,
                "unknown pending dispatch poll",
                outcome,
            );
            return;
        };
        if !self.command_epoch_is_current(
            &correlation.workspace_directory,
            correlation.epoch_id,
            outcome,
        ) {
            self.store.pending_dispatch_poll_in_flight = None;
            return;
        }
        self.store.pending_dispatch_poll_in_flight = None;
        let (wake, error) = match result {
            Ok(wake) => (wake, None),
            Err(error) => (None, Some(error)),
        };
        if let Some(error) = error {
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::DispatchWithheld {
                    trigger: Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
                    reason: format!("pending dispatch command poll failed: {error}"),
                });
        }
        let workspace_directory = correlation.workspace_directory;
        let epoch_id = correlation.epoch_id;
        let has_queued_continuation = self.store.pending_parallel_entry.is_some()
            || self.store.pending_supervisor_refresh
            || self.store.pending_orchestrator_wake.is_some();
        if has_queued_continuation {
            if self.store.pending_orchestrator_wake.is_none() {
                self.store.pending_orchestrator_wake = wake;
            }
            self.continue_after_effect_completed(workspace_directory, epoch_id, outcome);
            return;
        }
        match ParallelModeControlPlaneAggregate::pending_dispatch_wake_decision(
            wake.is_some(),
            in_flight.follow_up_tick_signature.is_some(),
        ) {
            ParallelModePendingDispatchWakeDecision::StartWake => {
                if let Some(wake) = wake {
                    self.wake_orchestrator(wake, outcome);
                }
            }
            ParallelModePendingDispatchWakeDecision::RunFollowUpTick => {
                if let Some(signature) = in_flight.follow_up_tick_signature {
                    self.run_orchestrator_tick(workspace_directory.clone(), signature, outcome);
                }
            }
            ParallelModePendingDispatchWakeDecision::Idle => {}
        }
        if !self.has_in_flight_effect() {
            self.continue_after_effect_completed(workspace_directory, epoch_id, outcome);
        }
    }

    fn schedule_dispatch_mutation(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        mutation: ParallelModeDispatchMutation,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if self
            .store
            .dispatch_mutation_in_flight
            .as_ref()
            .is_some_and(|in_flight| {
                in_flight.correlation.workspace_directory == workspace_directory
                    && in_flight.correlation.epoch_id == epoch_id
                    && in_flight.mutation.has_same_semantic_intent(&mutation)
            })
        {
            return;
        }

        let intent = ParallelModeDispatchMutationIntent {
            workspace_directory,
            epoch_id,
            mutation,
        };
        if intent.mutation.is_enqueue()
            && self.store.pending_dispatch_mutations.iter().any(|pending| {
                pending.workspace_directory == intent.workspace_directory
                    && pending.epoch_id == intent.epoch_id
                    && pending.mutation.has_same_semantic_intent(&intent.mutation)
            })
        {
            return;
        }
        if self.store.dispatch_mutation_in_flight.is_some() {
            if intent.mutation.is_enqueue() {
                self.store.pending_dispatch_mutations.push_back(intent);
            } else {
                self.store.pending_dispatch_mutations.retain(|pending| {
                    pending.workspace_directory != intent.workspace_directory
                        || pending.epoch_id != intent.epoch_id
                        || !pending.mutation.is_enqueue()
                });
                if self.store.pending_dispatch_mutations.iter().any(|pending| {
                    pending.workspace_directory == intent.workspace_directory
                        && pending.epoch_id == intent.epoch_id
                        && pending.mutation.has_same_semantic_intent(&intent.mutation)
                }) {
                    return;
                }
                let insert_at = self
                    .store
                    .pending_dispatch_mutations
                    .iter()
                    .position(|pending| pending.mutation.is_enqueue())
                    .unwrap_or(self.store.pending_dispatch_mutations.len());
                self.store
                    .pending_dispatch_mutations
                    .insert(insert_at, intent);
            }
            return;
        }
        self.start_dispatch_mutation(intent, outcome);
    }

    fn retry_unsettled_dispatch_cleanup(
        &mut self,
        original_cleanup: ParallelModeDispatchCleanupCorrelation,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if !self
            .store
            .unsettled_dispatch_cleanups
            .iter()
            .any(|cleanup| cleanup.correlation == original_cleanup)
        {
            self.stale_command(
                original_cleanup.workspace_directory,
                original_cleanup.epoch_id,
                "unknown unsettled dispatch cleanup",
                outcome,
            );
            return;
        }
        self.schedule_dispatch_mutation(
            original_cleanup.workspace_directory.clone(),
            original_cleanup.epoch_id,
            ParallelModeDispatchMutation::RetryCancel { original_cleanup },
            outcome,
        );
    }

    fn start_dispatch_mutation(
        &mut self,
        intent: ParallelModeDispatchMutationIntent,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let operation_id = self.store.next_dispatch_mutation_operation_id;
        self.store.next_dispatch_mutation_operation_id = operation_id
            .checked_add(1)
            .expect("parallel dispatch mutation operation id exhausted");
        let correlation = ParallelModeDispatchMutationCorrelation::new(
            operation_id,
            intent.workspace_directory,
            intent.epoch_id,
        );
        self.store.dispatch_mutation_in_flight = Some(ParallelModeDispatchMutationInFlight {
            correlation: correlation.clone(),
            mutation: intent.mutation.clone(),
        });
        outcome
            .effects
            .push(ParallelModeControlPlaneEffect::MutateDispatchCommands {
                correlation,
                mutation: intent.mutation,
            });
    }

    fn dispatch_mutation_completed(
        &mut self,
        correlation: ParallelModeDispatchMutationCorrelation,
        result: Result<usize, String>,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let Some(in_flight) = self
            .store
            .dispatch_mutation_in_flight
            .as_ref()
            .filter(|in_flight| in_flight.correlation == correlation)
            .cloned()
        else {
            self.stale_command(
                correlation.workspace_directory,
                correlation.epoch_id,
                "unknown dispatch mutation",
                outcome,
            );
            return;
        };
        self.store.dispatch_mutation_in_flight = None;

        let targets_current_epoch =
            ParallelModeControlPlaneAggregate::command_targets_current_epoch(
                &correlation.workspace_directory,
                correlation.epoch_id,
                self.store.workspace_directory.as_deref(),
                self.store.current_epoch_id,
            );
        if in_flight.mutation.is_enqueue() && !targets_current_epoch {
            self.stale_command(
                correlation.workspace_directory.clone(),
                correlation.epoch_id,
                "dispatch mutation belongs to a stale epoch",
                outcome,
            );
        } else {
            self.record_dispatch_mutation_result(
                &correlation,
                &in_flight.mutation,
                result,
                targets_current_epoch,
                outcome,
            );
        }

        if self.start_next_dispatch_mutation(outcome) || self.has_in_flight_effect() {
            return;
        }
        self.continue_current_context_after_effect_completed(outcome);
    }

    fn record_dispatch_mutation_result(
        &mut self,
        correlation: &ParallelModeDispatchMutationCorrelation,
        mutation: &ParallelModeDispatchMutation,
        result: Result<usize, String>,
        targets_current_epoch: bool,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        match (mutation, result) {
            (ParallelModeDispatchMutation::EnqueueSlotCapacity, Ok(_)) => {}
            (ParallelModeDispatchMutation::EnqueueForTrigger { trigger, .. }, Ok(0)) => outcome
                .events
                .push(ParallelModeControlPlaneEvent::DispatchWithheld {
                    trigger: Some(*trigger),
                    reason: "orchestrator wake already queued".to_string(),
                }),
            (
                ParallelModeDispatchMutation::EnqueueForTrigger { trigger, reason },
                Ok(inserted_count),
            ) => outcome
                .events
                .push(ParallelModeControlPlaneEvent::DispatchCommandQueued {
                    workspace_directory: correlation.workspace_directory.clone(),
                    trigger: *trigger,
                    inserted_count,
                    reason: reason.clone(),
                }),
            (ParallelModeDispatchMutation::Cancel { .. }, Ok(cancelled_count)) => outcome
                .events
                .push(ParallelModeControlPlaneEvent::DispatchCommandsCancelled {
                    workspace_directory: correlation.workspace_directory.clone(),
                    epoch_id: correlation.epoch_id,
                    cancelled_count,
                }),
            (
                ParallelModeDispatchMutation::RetryCancel { original_cleanup },
                Ok(cancelled_count),
            ) => {
                self.store
                    .unsettled_dispatch_cleanups
                    .retain(|cleanup| cleanup.correlation != *original_cleanup);
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::DispatchCommandsCancelled {
                        workspace_directory: correlation.workspace_directory.clone(),
                        epoch_id: correlation.epoch_id,
                        cancelled_count,
                    });
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::DispatchCleanupSettled {
                        original_cleanup: original_cleanup.clone(),
                        retry_operation_id: correlation.operation_id,
                    });
            }
            (mutation, Err(error)) => {
                let (trigger, reason, cleanup_correlation) = match mutation {
                    ParallelModeDispatchMutation::EnqueueSlotCapacity => (
                        Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
                        format!("slot-capacity dispatch queue failed: {error}"),
                        None,
                    ),
                    ParallelModeDispatchMutation::EnqueueForTrigger { trigger, .. } => (
                        Some(*trigger),
                        format!("orchestrator wake queue failed: {error}"),
                        None,
                    ),
                    ParallelModeDispatchMutation::Cancel { .. } => {
                        let cleanup =
                            ParallelModeDispatchCleanupCorrelation::from_mutation(correlation);
                        (
                            None,
                            format!("dispatch command cancellation failed: {error}"),
                            Some(cleanup),
                        )
                    }
                    ParallelModeDispatchMutation::RetryCancel { original_cleanup } => (
                        None,
                        format!("dispatch command cancellation retry failed: {error}"),
                        Some(original_cleanup.clone()),
                    ),
                };
                if let Some(cleanup_correlation) = cleanup_correlation.as_ref() {
                    self.record_unsettled_dispatch_cleanup(
                        cleanup_correlation.clone(),
                        reason.clone(),
                    );
                }
                let presentation_current = targets_current_epoch
                    || (cleanup_correlation.is_some()
                        && self.store.current_epoch_id.is_none()
                        && self.store.workspace_directory.is_none());
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::DispatchMutationFailed {
                        operation_id: correlation.operation_id,
                        workspace_directory: correlation.workspace_directory.clone(),
                        epoch_id: correlation.epoch_id,
                        trigger,
                        reason,
                        presentation_current,
                        cleanup_correlation,
                    });
            }
        }
    }

    fn record_unsettled_dispatch_cleanup(
        &mut self,
        correlation: ParallelModeDispatchCleanupCorrelation,
        error: String,
    ) {
        if let Some(cleanup) = self
            .store
            .unsettled_dispatch_cleanups
            .iter_mut()
            .find(|cleanup| cleanup.correlation == correlation)
        {
            cleanup.error = error;
            return;
        }
        if self.store.unsettled_dispatch_cleanups.len() == MAX_UNSETTLED_DISPATCH_CLEANUPS {
            self.store.unsettled_dispatch_cleanups.pop_front();
        }
        self.store
            .unsettled_dispatch_cleanups
            .push_back(ParallelModeUnsettledDispatchCleanup { correlation, error });
    }

    fn start_next_dispatch_mutation(
        &mut self,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) -> bool {
        while let Some(intent) = self.store.pending_dispatch_mutations.pop_front() {
            if intent.mutation.is_enqueue()
                && !ParallelModeControlPlaneAggregate::command_targets_current_epoch(
                    &intent.workspace_directory,
                    intent.epoch_id,
                    self.store.workspace_directory.as_deref(),
                    self.store.current_epoch_id,
                )
            {
                self.stale_command(
                    intent.workspace_directory,
                    intent.epoch_id,
                    "queued dispatch mutation belongs to a stale epoch",
                    outcome,
                );
                continue;
            }
            self.start_dispatch_mutation(intent, outcome);
            return true;
        }
        false
    }

    fn schedule_after_projection_ready(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        follow_up_tick_signature: Option<String>,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        match ParallelModeControlPlaneAggregate::projection_ready_continuation(
            self.store.pending_supervisor_refresh,
            self.store.pending_orchestrator_wake.is_some(),
        ) {
            ParallelModeProjectionReadyContinuation::RefreshSupervisor => {
                self.store.pending_supervisor_refresh = false;
                self.start_supervisor_refresh(workspace_directory, epoch_id, outcome);
                return;
            }
            ParallelModeProjectionReadyContinuation::DrainPendingWake => {
                if self.drain_pending_orchestrator_wake(outcome) {
                    return;
                }
            }
            ParallelModeProjectionReadyContinuation::PollPendingDispatchWake => {}
        }
        if self.store.pending_dispatch_poll.is_some() {
            self.queue_pending_dispatch_poll(
                workspace_directory,
                epoch_id,
                follow_up_tick_signature,
            );
            self.drain_pending_dispatch_poll(outcome);
            return;
        }
        self.start_pending_dispatch_poll(
            workspace_directory,
            epoch_id,
            follow_up_tick_signature,
            outcome,
        );
    }

    fn start_pending_dispatch_poll(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        follow_up_tick_signature: Option<String>,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if self.has_in_flight_effect() {
            self.queue_pending_dispatch_poll(
                workspace_directory,
                epoch_id,
                follow_up_tick_signature,
            );
            return;
        }
        let operation_id = self.store.next_pending_dispatch_poll_operation_id;
        self.store.next_pending_dispatch_poll_operation_id = operation_id
            .checked_add(1)
            .expect("pending dispatch poll operation id exhausted");
        let correlation = ParallelModePendingDispatchPollCorrelation::new(
            operation_id,
            workspace_directory,
            epoch_id,
        );
        self.store.pending_dispatch_poll_in_flight =
            Some(ParallelModePendingDispatchPollInFlight {
                correlation: correlation.clone(),
                follow_up_tick_signature,
            });
        outcome
            .effects
            .push(ParallelModeControlPlaneEffect::PollPendingDispatchWake { correlation });
    }

    fn queue_pending_dispatch_poll(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        follow_up_tick_signature: Option<String>,
    ) {
        if let Some(pending) = self.store.pending_dispatch_poll.as_mut()
            && pending.workspace_directory == workspace_directory
            && pending.epoch_id == epoch_id
        {
            if follow_up_tick_signature.is_some() {
                pending.follow_up_tick_signature = follow_up_tick_signature;
            }
            return;
        }
        self.store.pending_dispatch_poll = Some(ParallelModePendingDispatchPollIntent {
            workspace_directory,
            epoch_id,
            follow_up_tick_signature,
        });
    }

    fn drain_pending_dispatch_poll(
        &mut self,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) -> bool {
        if self.has_in_flight_effect() {
            return false;
        }
        let Some(intent) = self.store.pending_dispatch_poll.take() else {
            return false;
        };
        if !ParallelModeControlPlaneAggregate::command_targets_current_epoch(
            &intent.workspace_directory,
            intent.epoch_id,
            self.store.workspace_directory.as_deref(),
            self.store.current_epoch_id,
        ) {
            self.stale_command(
                intent.workspace_directory,
                intent.epoch_id,
                "pending dispatch poll belongs to a stale epoch",
                outcome,
            );
            return false;
        }
        self.start_pending_dispatch_poll(
            intent.workspace_directory,
            intent.epoch_id,
            intent.follow_up_tick_signature,
            outcome,
        );
        true
    }

    fn drain_pending_orchestrator_wake(
        &mut self,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) -> bool {
        if self.has_in_flight_effect() {
            return false;
        }
        let Some(wake) = self.store.pending_orchestrator_wake.take() else {
            return false;
        };
        if !self.wake_epoch_is_current(&wake) {
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::StaleCommandDropped {
                    workspace_directory: wake.workspace_directory,
                    epoch_id: wake.epoch_id,
                    reason: "pending wake belongs to a stale epoch".to_string(),
                });
            return false;
        }
        outcome
            .events
            .push(ParallelModeControlPlaneEvent::OrchestratorWakeDequeued {
                trigger: wake.trigger,
                epoch_id: wake.epoch_id,
            });
        self.start_orchestrator_wake(wake, outcome);
        true
    }

    fn continue_after_effect_completed(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if self.start_pending_parallel_entry_if_idle(outcome) {
            return;
        }
        match ParallelModeControlPlaneAggregate::effect_completion_follow_up(
            self.store.pending_supervisor_refresh,
        ) {
            ParallelModeControlPlaneEffectCompletionFollowUp::RefreshSupervisor => {
                self.store.pending_supervisor_refresh = false;
                self.start_supervisor_refresh(workspace_directory, epoch_id, outcome);
            }
            ParallelModeControlPlaneEffectCompletionFollowUp::DrainPendingWake => {
                if !self.drain_pending_orchestrator_wake(outcome) {
                    self.drain_pending_dispatch_poll(outcome);
                }
            }
        }
    }

    fn continue_current_context_after_effect_completed(
        &mut self,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let (Some(workspace_directory), Some(epoch_id)) = (
            self.store.workspace_directory.clone(),
            self.store.current_epoch_id,
        ) else {
            return;
        };
        self.continue_after_effect_completed(workspace_directory, epoch_id, outcome);
    }

    fn finish_effect(
        &mut self,
        workspace_directory: &str,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        expected_kind: ParallelModeControlPlaneEffectKind,
        unknown_reason: &str,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) -> bool {
        if !self.command_epoch_is_current(workspace_directory, epoch_id, outcome) {
            return false;
        }
        if effect_id.kind != expected_kind {
            self.stale_command(
                workspace_directory.to_string(),
                epoch_id,
                unknown_reason,
                outcome,
            );
            return false;
        }
        let current = match expected_kind {
            ParallelModeControlPlaneEffectKind::EnterParallelMode => {
                self.store.parallel_entry_in_flight
            }
            ParallelModeControlPlaneEffectKind::RefreshSupervisor => {
                self.store.supervisor_refresh_in_flight
            }
            ParallelModeControlPlaneEffectKind::RunOrchestrator => {
                self.store.orchestrator_wake_in_flight
            }
            ParallelModeControlPlaneEffectKind::RunOrchestratorTick => {
                self.store.orchestrator_tick_in_flight
            }
        };
        if current != Some(effect_id) {
            self.stale_command(
                workspace_directory.to_string(),
                epoch_id,
                unknown_reason,
                outcome,
            );
            return false;
        }
        match expected_kind {
            ParallelModeControlPlaneEffectKind::EnterParallelMode => {
                self.store.parallel_entry_in_flight = None;
            }
            ParallelModeControlPlaneEffectKind::RefreshSupervisor => {
                self.store.supervisor_refresh_in_flight = None;
            }
            ParallelModeControlPlaneEffectKind::RunOrchestrator => {
                self.store.orchestrator_wake_in_flight = None;
            }
            ParallelModeControlPlaneEffectKind::RunOrchestratorTick => {
                self.store.orchestrator_tick_in_flight = None;
            }
        }
        outcome
            .events
            .push(ParallelModeControlPlaneEvent::EffectCompleted { effect_id });
        true
    }

    fn ensure_epoch(
        &mut self,
        workspace_directory: String,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) -> u64 {
        match (
            self.store.workspace_directory.as_deref(),
            self.store.current_epoch_id,
        ) {
            (Some(current_workspace), Some(epoch_id))
                if current_workspace == workspace_directory =>
            {
                epoch_id
            }
            _ => {
                if let (Some(previous_workspace), Some(previous_epoch_id)) = (
                    self.store.workspace_directory.clone(),
                    self.store.current_epoch_id,
                ) {
                    outcome
                        .events
                        .push(ParallelModeControlPlaneEvent::EpochClosed {
                            workspace_directory: previous_workspace.clone(),
                            epoch_id: previous_epoch_id,
                        });
                    self.schedule_dispatch_mutation(
                        previous_workspace,
                        previous_epoch_id,
                        ParallelModeDispatchMutation::Cancel {
                            reason: "parallel workspace superseded".to_string(),
                        },
                        outcome,
                    );
                }
                let epoch_id = self.store.next_epoch_id;
                self.store.next_epoch_id = self.store.next_epoch_id.saturating_add(1);
                self.store.mode_enabled = false;
                self.store.initial_pool_reset_completed = false;
                self.clear_process_effect_state();
                self.store.workspace_directory = Some(workspace_directory.clone());
                self.store.current_epoch_id = Some(epoch_id);
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::EpochOpened {
                        workspace_directory,
                        epoch_id,
                    });
                epoch_id
            }
        }
    }

    fn has_in_flight_effect(&self) -> bool {
        self.store.parallel_entry_in_flight.is_some()
            || self.store.supervisor_refresh_in_flight.is_some()
            || self.store.orchestrator_wake_in_flight.is_some()
            || self.store.orchestrator_tick_in_flight.is_some()
            || self.store.supervisor_inspection_in_flight.is_some()
            || self.store.pending_dispatch_poll_in_flight.is_some()
            || self.store.dispatch_mutation_in_flight.is_some()
    }

    fn clear_process_effect_state(&mut self) {
        self.store.parallel_entry_in_flight = None;
        self.store.supervisor_refresh_in_flight = None;
        self.store.orchestrator_wake_in_flight = None;
        self.store.orchestrator_tick_in_flight = None;
        self.store.supervisor_inspection_in_flight = None;
        self.store.pending_supervisor_inspection = None;
        self.store.pending_dispatch_poll_in_flight = None;
        self.store.pending_dispatch_poll = None;
        self.store.pending_parallel_entry = None;
        self.store.projection_ready = false;
        self.store.pending_supervisor_refresh = false;
        self.store.pending_orchestrator_wake = None;
        self.store.last_orchestrator_tick_signature = None;
    }

    fn supervisor_inspection_context_is_current(
        &self,
        correlation: &ParallelModeSupervisorInspectionCorrelation,
    ) -> bool {
        self.supervisor_inspection_epoch_for_workspace(&correlation.workspace_directory)
            == correlation.epoch_id
    }

    fn supervisor_inspection_epoch_for_workspace(&self, workspace_directory: &str) -> Option<u64> {
        (self.store.workspace_directory.as_deref() == Some(workspace_directory))
            .then_some(self.store.current_epoch_id)
            .flatten()
    }

    fn start_pending_parallel_entry_if_idle(
        &mut self,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) -> bool {
        if self.has_in_flight_effect() {
            return false;
        }
        let Some(entry) = self.store.pending_parallel_entry.take() else {
            return false;
        };
        if self.store.workspace_directory.as_deref() != Some(&entry.workspace_directory)
            || self.store.current_epoch_id != Some(entry.epoch_id)
        {
            self.stale_command(
                entry.workspace_directory,
                entry.epoch_id,
                "pending parallel entry belongs to a stale epoch",
                outcome,
            );
            return false;
        }
        self.start_parallel_entry(
            entry.workspace_directory,
            entry.epoch_id,
            entry.mode_was_enabled,
            entry.initial_pool_reset_required,
            outcome,
        );
        true
    }

    fn start_pending_supervisor_inspection_if_idle(
        &mut self,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if self.has_in_flight_effect() {
            return;
        }
        let Some(intent) = self.store.pending_supervisor_inspection.take() else {
            return;
        };
        let decision = ParallelModeControlPlaneAggregate::supervisor_inspection(
            self.store.mode_enabled,
            self.store.workspace_directory.as_deref(),
            &intent.workspace_directory,
            intent.reconcile_pool,
        );
        let operation_id = self.store.next_supervisor_inspection_operation_id;
        self.store.next_supervisor_inspection_operation_id = operation_id
            .checked_add(1)
            .expect("parallel supervisor inspection operation id exhausted");
        let epoch_id = self.supervisor_inspection_epoch_for_workspace(&intent.workspace_directory);
        let correlation = ParallelModeSupervisorInspectionCorrelation::new(
            operation_id,
            intent.workspace_directory,
            epoch_id,
        );
        self.store.supervisor_inspection_in_flight =
            Some(ParallelModeSupervisorInspectionInFlight {
                correlation: correlation.clone(),
                mode_enabled: decision.mode_enabled,
                reconcile_pool: decision.reconcile_pool,
                show_status: intent.show_status,
            });
        outcome
            .events
            .push(ParallelModeControlPlaneEvent::SupervisorInspectionStarted {
                correlation: correlation.clone(),
                show_status: intent.show_status,
            });
        outcome
            .effects
            .push(ParallelModeControlPlaneEffect::InspectSupervisor {
                correlation,
                mode_enabled: decision.mode_enabled,
                reconcile_pool: decision.reconcile_pool,
            });
    }

    fn next_effect_id(
        &mut self,
        kind: ParallelModeControlPlaneEffectKind,
    ) -> ParallelModeControlPlaneEffectId {
        let effect_id =
            ParallelModeControlPlaneEffectId::new(self.store.next_effect_sequence, kind);
        self.store.next_effect_sequence = self.store.next_effect_sequence.saturating_add(1);
        effect_id
    }

    fn current_epoch_for_workspace(
        &mut self,
        workspace_directory: &str,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) -> Option<u64> {
        match ParallelModeControlPlaneAggregate::current_epoch_for_workspace(
            workspace_directory,
            self.store.workspace_directory.as_deref(),
            self.store.current_epoch_id,
        ) {
            Some(epoch_id) => Some(epoch_id),
            None => {
                self.stale_command(
                    workspace_directory.to_string(),
                    0,
                    "parallel automation epoch is not open for workspace",
                    outcome,
                );
                None
            }
        }
    }

    fn command_epoch_is_current(
        &mut self,
        workspace_directory: &str,
        epoch_id: u64,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) -> bool {
        if ParallelModeControlPlaneAggregate::command_targets_current_epoch(
            workspace_directory,
            epoch_id,
            self.store.workspace_directory.as_deref(),
            self.store.current_epoch_id,
        ) {
            return true;
        }
        self.stale_command(
            workspace_directory.to_string(),
            epoch_id,
            "parallel automation epoch is stale",
            outcome,
        );
        false
    }

    fn wake_epoch_is_current(&self, wake: &ParallelModeControlPlaneWake) -> bool {
        ParallelModeControlPlaneAggregate::command_targets_current_epoch(
            &wake.workspace_directory,
            wake.epoch_id,
            self.store.workspace_directory.as_deref(),
            self.store.current_epoch_id,
        )
    }

    fn stale_command(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        reason: &str,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        outcome
            .events
            .push(ParallelModeControlPlaneEvent::StaleCommandDropped {
                workspace_directory,
                epoch_id,
                reason: reason.to_string(),
            });
    }
}

fn unknown_effect_reason(kind: ParallelModeControlPlaneEffectKind) -> &'static str {
    match kind {
        ParallelModeControlPlaneEffectKind::EnterParallelMode => "unknown parallel entry",
        ParallelModeControlPlaneEffectKind::RefreshSupervisor => "unknown supervisor refresh",
        ParallelModeControlPlaneEffectKind::RunOrchestrator => "unknown orchestrator wake",
        ParallelModeControlPlaneEffectKind::RunOrchestratorTick => "unknown orchestrator tick",
    }
}

fn worker_event_to_control_plane_event(
    event: &ParallelModeControlPlaneWorkerEvent,
) -> ParallelModeControlPlaneEvent {
    match event.kind {
        ParallelModeControlPlaneWorkerEventKind::Completed => {
            ParallelModeControlPlaneEvent::WorkerCompleted {
                workspace_directory: event.workspace_directory.clone(),
                epoch_id: event.epoch_id,
                task_id: event.task_id.clone(),
            }
        }
        ParallelModeControlPlaneWorkerEventKind::LaunchFailed => {
            ParallelModeControlPlaneEvent::WorkerLaunchFailed {
                workspace_directory: event.workspace_directory.clone(),
                epoch_id: event.epoch_id,
                task_id: event.task_id.clone(),
            }
        }
        ParallelModeControlPlaneWorkerEventKind::StreamFailed => {
            ParallelModeControlPlaneEvent::WorkerStreamFailed {
                workspace_directory: event.workspace_directory.clone(),
                epoch_id: event.epoch_id,
                task_id: event.task_id.clone(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enable(workspace_directory: &str) -> ParallelModeControlPlaneCommand {
        ParallelModeControlPlaneCommand::Enable {
            workspace_directory: workspace_directory.to_string(),
        }
    }

    fn wake(workspace_directory: &str, epoch_id: u64) -> ParallelModeControlPlaneCommand {
        ParallelModeControlPlaneCommand::WakeOrchestrator(ParallelModeControlPlaneWake::new(
            workspace_directory,
            ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id,
            Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
        ))
    }

    fn tick(workspace_directory: &str, signature: &str) -> ParallelModeControlPlaneCommand {
        ParallelModeControlPlaneCommand::RunOrchestratorTick {
            workspace_directory: workspace_directory.to_string(),
            signature: signature.to_string(),
        }
    }

    fn completed(
        workspace_directory: &str,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
    ) -> ParallelModeControlPlaneCommand {
        ParallelModeControlPlaneCommand::EffectCompleted {
            workspace_directory: workspace_directory.to_string(),
            epoch_id,
            effect_id,
        }
    }

    fn only_effect(
        outcome: &ParallelModeControlPlaneRuntimeOutcome,
    ) -> ParallelModeControlPlaneEffect {
        assert_eq!(outcome.effects.len(), 1);
        outcome.effects[0].clone()
    }

    #[test]
    fn control_plane_command_serialization_round_trips() {
        let command = wake("/repo", 7);
        let json = serde_json::to_string(&command).expect("command should serialize");
        let decoded: ParallelModeControlPlaneCommand =
            serde_json::from_str(&json).expect("command should deserialize");

        assert_eq!(decoded, command);
    }

    #[test]
    fn open_epoch_does_not_start_refresh_effect() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();

        let opened = runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: "/repo".to_string(),
        });

        assert_eq!(
            opened.events,
            vec![ParallelModeControlPlaneEvent::EpochOpened {
                workspace_directory: "/repo".to_string(),
                epoch_id: 1,
            }]
        );
        assert!(opened.effects.is_empty());
        assert_eq!(runtime.store().current_epoch_id, Some(1));
        assert!(runtime.store().supervisor_refresh_in_flight.is_none());
    }

    #[test]
    fn opening_different_workspace_clears_previous_process_effect_state() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        let enabled = runtime.handle(enable("/repo"));
        let refresh_id = only_effect(&enabled)
            .effect_id()
            .expect("refresh effect should have id");

        let opened = runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: "/other".to_string(),
        });

        assert!(matches!(
            opened.events.as_slice(),
            [
                ParallelModeControlPlaneEvent::EpochClosed {
                    workspace_directory: previous_workspace,
                    epoch_id: 1,
                },
                ParallelModeControlPlaneEvent::EpochOpened {
                    workspace_directory,
                    epoch_id: 2,
                }
            ] if previous_workspace == "/repo" && workspace_directory == "/other"
        ));
        assert!(matches!(
            opened.effects.as_slice(),
            [ParallelModeControlPlaneEffect::MutateDispatchCommands {
                correlation,
                mutation: ParallelModeDispatchMutation::Cancel { reason },
            }] if correlation.workspace_directory == "/repo"
                && correlation.epoch_id == 1
                && reason == "parallel workspace superseded"
        ));
        assert!(runtime.store().supervisor_refresh_in_flight.is_none());
        assert_eq!(runtime.store().current_epoch_id, Some(2));

        let stale = runtime.handle(completed("/repo", 1, refresh_id));
        assert!(matches!(
            stale.events.as_slice(),
            [ParallelModeControlPlaneEvent::StaleCommandDropped { epoch_id: 1, .. }]
        ));
    }

    #[test]
    fn opening_new_workspace_resets_mode_and_initial_reset_state() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        runtime.force_mode_for_test("/repo", true);
        runtime.force_initial_pool_reset_completed_for_test(true);

        let opened = runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: "/other".to_string(),
        });
        assert!(matches!(
            opened.events.as_slice(),
            [
                ParallelModeControlPlaneEvent::EpochClosed {
                    workspace_directory: previous_workspace,
                    epoch_id: 1,
                },
                ParallelModeControlPlaneEvent::EpochOpened {
                    workspace_directory,
                    epoch_id: 2,
                }
            ] if previous_workspace == "/repo" && workspace_directory == "/other"
        ));
        assert!(matches!(
            opened.effects.as_slice(),
            [ParallelModeControlPlaneEffect::MutateDispatchCommands {
                correlation,
                mutation: ParallelModeDispatchMutation::Cancel { .. },
            }] if correlation.workspace_directory == "/repo" && correlation.epoch_id == 1
        ));
        assert!(!runtime.store().mode_enabled);
        assert!(!runtime.store().initial_pool_reset_completed);
        assert_eq!(runtime.store().current_epoch_id, Some(2));

        let cancel_correlation = match opened.effects.as_slice() {
            [
                ParallelModeControlPlaneEffect::MutateDispatchCommands {
                    correlation,
                    mutation: ParallelModeDispatchMutation::Cancel { .. },
                },
            ] => correlation.clone(),
            effects => panic!("expected one superseded-workspace cancellation, got {effects:?}"),
        };
        let cancelled =
            runtime.handle(ParallelModeControlPlaneCommand::DispatchMutationCompleted {
                correlation: cancel_correlation,
                result: Ok(0),
            });
        assert!(cancelled.effects.is_empty());

        let enabled = runtime.handle(enable("/other"));
        let entry = only_effect(&enabled);
        assert!(matches!(
            &entry,
            ParallelModeControlPlaneEffect::EnterParallelMode {
                mode_was_enabled: false,
                initial_pool_reset_required: true,
                ..
            }
        ));
    }

    #[test]
    fn force_mode_for_test_false_clears_enabled_state() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        runtime.force_mode_for_test("/repo", true);
        runtime.force_mode_for_test("/repo", false);

        assert!(!runtime.store().mode_enabled);
        assert_eq!(runtime.store().workspace_directory, None);
        assert_eq!(runtime.store().current_epoch_id, None);
    }

    #[test]
    fn enable_opens_epoch_and_orders_entry_before_orchestrator_wake() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        let enabled = runtime.handle(enable("/repo"));
        let entry = only_effect(&enabled);
        assert!(matches!(
            enabled.events.as_slice(),
            [
                ParallelModeControlPlaneEvent::EpochOpened { epoch_id: 1, .. },
                ParallelModeControlPlaneEvent::ModeEnabled { epoch_id: 1, .. },
                ParallelModeControlPlaneEvent::EffectStarted { .. }
            ]
        ));
        assert!(matches!(
            &entry,
            ParallelModeControlPlaneEffect::EnterParallelMode {
                mode_was_enabled: false,
                initial_pool_reset_required: true,
                ..
            }
        ));

        let queued_wake = runtime.handle(wake("/repo", 1));
        assert!(queued_wake.effects.is_empty());
        assert_eq!(
            queued_wake.events,
            vec![ParallelModeControlPlaneEvent::OrchestratorWakeQueued {
                trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
                epoch_id: 1,
            }]
        );

        let entry_id = entry.effect_id().expect("entry effect should have id");
        let completed = runtime.handle(completed("/repo", 1, entry_id));
        assert!(matches!(
            completed.events.as_slice(),
            [
                ParallelModeControlPlaneEvent::EffectCompleted { .. },
                ParallelModeControlPlaneEvent::OrchestratorWakeDequeued { epoch_id: 1, .. },
                ParallelModeControlPlaneEvent::EffectStarted { .. }
            ]
        ));
        assert!(matches!(
            completed.effects.as_slice(),
            [ParallelModeControlPlaneEffect::RunOrchestrator { .. }]
        ));
    }

    #[test]
    fn orchestrator_wake_coalesces_while_effect_is_in_flight() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        let enabled = runtime.handle(enable("/repo"));
        let refresh_id = only_effect(&enabled)
            .effect_id()
            .expect("refresh effect should have id");
        assert!(runtime.handle(wake("/repo", 1)).effects.is_empty());
        let run_after_refresh = runtime.handle(completed("/repo", 1, refresh_id));
        let first_run_id = only_effect(&run_after_refresh)
            .effect_id()
            .expect("run effect should have id");

        assert!(runtime.handle(wake("/repo", 1)).effects.is_empty());
        assert!(runtime.handle(wake("/repo", 1)).effects.is_empty());

        let completed = runtime.handle(completed("/repo", 1, first_run_id));
        let run_effects = completed
            .effects
            .iter()
            .filter(|effect| {
                effect.effect_id().is_some_and(|effect_id| {
                    effect_id.kind == ParallelModeControlPlaneEffectKind::RunOrchestrator
                })
            })
            .count();
        assert_eq!(run_effects, 1);
        assert!(runtime.store().pending_orchestrator_wake.is_none());
    }

    #[test]
    fn synchronous_mutex_facade_covers_ordering_backpressure_and_stale_completion() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        let enabled = runtime.handle(enable("/repo"));
        let entry_id = only_effect(&enabled)
            .effect_id()
            .expect("entry effect should have id");

        for _ in 0..3 {
            assert!(runtime.handle(wake("/repo", 1)).effects.is_empty());
        }

        let run_after_entry = runtime.handle(completed("/repo", 1, entry_id));
        let run_id = only_effect(&run_after_entry)
            .effect_id()
            .expect("run effect should have id");

        for _ in 0..3 {
            assert!(runtime.handle(wake("/repo", 1)).effects.is_empty());
        }

        let disabled = runtime.handle(ParallelModeControlPlaneCommand::Disable {
            workspace_directory: "/repo".to_string(),
        });
        assert!(matches!(
            disabled.effects.as_slice(),
            [ParallelModeControlPlaneEffect::MutateDispatchCommands {
                mutation: ParallelModeDispatchMutation::Cancel { .. },
                ..
            }]
        ));

        let stale_completion = runtime.handle(completed("/repo", 1, run_id));
        assert!(stale_completion.effects.is_empty());
        assert!(matches!(
            stale_completion.events.as_slice(),
            [ParallelModeControlPlaneEvent::StaleCommandDropped { epoch_id: 1, .. }]
        ));
    }

    #[test]
    fn orchestrator_tick_tracks_signature_and_drains_queued_wake() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: "/repo".to_string(),
        });

        let tick_started = runtime.handle(tick("/repo", "sig-1"));
        let tick_id = only_effect(&tick_started)
            .effect_id()
            .expect("tick effect should have id");
        assert!(matches!(
            tick_started.effects.as_slice(),
            [ParallelModeControlPlaneEffect::RunOrchestratorTick { .. }]
        ));

        let duplicate = runtime.handle(tick("/repo", "sig-1"));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());

        let queued_wake = runtime.handle(wake("/repo", 1));
        assert_eq!(
            queued_wake.events,
            vec![ParallelModeControlPlaneEvent::OrchestratorWakeQueued {
                trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
                epoch_id: 1,
            }]
        );
        assert!(queued_wake.effects.is_empty());

        let completed = runtime.handle(completed("/repo", 1, tick_id));
        assert!(matches!(
            completed.events.as_slice(),
            [
                ParallelModeControlPlaneEvent::EffectCompleted { .. },
                ParallelModeControlPlaneEvent::OrchestratorWakeDequeued { .. },
                ParallelModeControlPlaneEvent::EffectStarted { .. }
            ]
        ));
        assert!(matches!(
            completed.effects.as_slice(),
            [ParallelModeControlPlaneEffect::RunOrchestrator { .. }]
        ));
    }

    #[test]
    fn stale_epoch_completion_is_dropped_after_disable() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        let enabled = runtime.handle(enable("/repo"));
        let refresh_id = only_effect(&enabled)
            .effect_id()
            .expect("refresh effect should have id");

        let disabled = runtime.handle(ParallelModeControlPlaneCommand::Disable {
            workspace_directory: "/repo".to_string(),
        });
        assert!(matches!(
            disabled.effects.as_slice(),
            [ParallelModeControlPlaneEffect::MutateDispatchCommands {
                mutation: ParallelModeDispatchMutation::Cancel { .. },
                ..
            }]
        ));
        assert_eq!(runtime.store().current_epoch_id, None);

        let stale = runtime.handle(completed("/repo", 1, refresh_id));
        assert!(stale.effects.is_empty());
        assert!(matches!(
            stale.events.as_slice(),
            [ParallelModeControlPlaneEvent::StaleCommandDropped { epoch_id: 1, .. }]
        ));
    }

    #[test]
    fn entry_completion_starts_ready_queue_dispatch_in_runtime() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        let enabled = runtime.handle(enable("/repo"));
        let entry_id = only_effect(&enabled)
            .effect_id()
            .expect("entry effect should have id");

        let completed = runtime.handle(ParallelModeControlPlaneCommand::EntryCompleted {
            workspace_directory: "/repo".to_string(),
            epoch_id: 1,
            effect_id: entry_id,
            mode_enabled: true,
            mode_was_enabled: false,
            initial_pool_reset_completed: true,
            has_actionable_queue_head: true,
            follow_up_tick_signature: Some("tick-sig".to_string()),
        });

        assert!(matches!(
            completed.events.as_slice(),
            [
                ParallelModeControlPlaneEvent::EffectCompleted { .. },
                ParallelModeControlPlaneEvent::EffectStarted { .. }
            ]
        ));
        assert!(matches!(
            completed.effects.as_slice(),
            [ParallelModeControlPlaneEffect::RunOrchestrator { wake, .. }]
                if wake.trigger == ParallelModeAutomationTrigger::TaskIntakeAfterEpoch
        ));
    }

    #[test]
    fn projection_completion_polls_pending_wake_before_tick_follow_up() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: "/repo".to_string(),
        });
        let refresh = runtime.handle(ParallelModeControlPlaneCommand::RefreshSupervisor {
            workspace_directory: "/repo".to_string(),
        });
        let refresh_id = only_effect(&refresh)
            .effect_id()
            .expect("refresh effect should have id");

        let completed = runtime.handle(
            ParallelModeControlPlaneCommand::SupervisorSnapshotRefreshCompleted {
                workspace_directory: "/repo".to_string(),
                epoch_id: 1,
                effect_id: refresh_id,
                follow_up_tick_signature: Some("tick-sig".to_string()),
            },
        );

        assert!(matches!(
            completed.effects.as_slice(),
            [ParallelModeControlPlaneEffect::PollPendingDispatchWake { correlation }]
                if correlation.workspace_directory == "/repo" && correlation.epoch_id == 1
        ));
        assert_eq!(
            runtime
                .store
                .pending_dispatch_poll_in_flight
                .as_ref()
                .and_then(|in_flight| in_flight.follow_up_tick_signature.as_deref()),
            Some("tick-sig")
        );
    }

    #[test]
    fn dispatch_request_queues_durable_command_when_projection_is_not_ready() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: "/repo".to_string(),
        });

        let requested = runtime.handle(ParallelModeControlPlaneCommand::RequestDispatch {
            workspace_directory: "/repo".to_string(),
            trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
        });

        assert!(matches!(
            requested.effects.as_slice(),
            [ParallelModeControlPlaneEffect::MutateDispatchCommands {
                correlation: ParallelModeDispatchMutationCorrelation { epoch_id: 1, .. },
                mutation: ParallelModeDispatchMutation::EnqueueForTrigger {
                    trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
                    ..
                },
            }]
        ));
    }

    #[test]
    fn duplicate_enqueue_coalesces_and_closed_epoch_cancels_preempt_pending_enqueue() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: "/repo-a".to_string(),
        });
        let first = runtime.handle(ParallelModeControlPlaneCommand::RequestDispatch {
            workspace_directory: "/repo-a".to_string(),
            trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
        });
        let first_correlation = match first.effects.as_slice() {
            [ParallelModeControlPlaneEffect::MutateDispatchCommands { correlation, .. }] => {
                correlation.clone()
            }
            effects => panic!("expected one initial enqueue, got {effects:?}"),
        };

        for _ in 0..32 {
            assert!(
                runtime
                    .handle(ParallelModeControlPlaneCommand::RequestDispatch {
                        workspace_directory: "/repo-a".to_string(),
                        trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
                    })
                    .effects
                    .is_empty()
            );
        }
        assert!(
            runtime.store.pending_dispatch_mutations.is_empty(),
            "duplicates must coalesce with the in-flight semantic intent"
        );

        runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: "/repo-b".to_string(),
        });
        runtime.handle(ParallelModeControlPlaneCommand::RequestDispatch {
            workspace_directory: "/repo-b".to_string(),
            trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
        });
        runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: "/repo-c".to_string(),
        });
        runtime.handle(ParallelModeControlPlaneCommand::RequestDispatch {
            workspace_directory: "/repo-c".to_string(),
            trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
        });

        assert_eq!(runtime.store.pending_dispatch_mutations.len(), 3);
        assert!(matches!(
            runtime.store.pending_dispatch_mutations.front(),
            Some(ParallelModeDispatchMutationIntent {
                workspace_directory,
                mutation: ParallelModeDispatchMutation::Cancel { .. },
                ..
            }) if workspace_directory == "/repo-a"
        ));
        assert!(matches!(
            runtime.store.pending_dispatch_mutations.get(1),
            Some(ParallelModeDispatchMutationIntent {
                workspace_directory,
                mutation: ParallelModeDispatchMutation::Cancel { .. },
                ..
            }) if workspace_directory == "/repo-b"
        ));
        assert!(matches!(
            runtime.store.pending_dispatch_mutations.get(2),
            Some(ParallelModeDispatchMutationIntent {
                workspace_directory,
                mutation: ParallelModeDispatchMutation::EnqueueForTrigger { .. },
                ..
            }) if workspace_directory == "/repo-c"
        ));

        let first_cancel = only_effect(&runtime.handle(
            ParallelModeControlPlaneCommand::DispatchMutationCompleted {
                correlation: first_correlation,
                result: Ok(1),
            },
        ))
        .clone();
        let first_cancel_correlation = match first_cancel {
            ParallelModeControlPlaneEffect::MutateDispatchCommands {
                correlation,
                mutation: ParallelModeDispatchMutation::Cancel { .. },
            } if correlation.workspace_directory == "/repo-a" => correlation,
            effect => panic!("expected repo-a cleanup first, got {effect:?}"),
        };
        let second_cancel = only_effect(&runtime.handle(
            ParallelModeControlPlaneCommand::DispatchMutationCompleted {
                correlation: first_cancel_correlation,
                result: Ok(0),
            },
        ))
        .clone();
        let second_cancel_correlation = match second_cancel {
            ParallelModeControlPlaneEffect::MutateDispatchCommands {
                correlation,
                mutation: ParallelModeDispatchMutation::Cancel { .. },
            } if correlation.workspace_directory == "/repo-b" => correlation,
            effect => panic!("expected repo-b cleanup second, got {effect:?}"),
        };
        assert!(matches!(
            only_effect(&runtime.handle(
                ParallelModeControlPlaneCommand::DispatchMutationCompleted {
                    correlation: second_cancel_correlation,
                    result: Ok(0),
                },
            )),
            ParallelModeControlPlaneEffect::MutateDispatchCommands {
                correlation,
                mutation: ParallelModeDispatchMutation::EnqueueForTrigger { .. },
            } if correlation.workspace_directory == "/repo-c"
        ));
    }

    #[test]
    fn dispatch_mutation_rejects_forged_duplicate_and_aba_completions() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: "/repo-a".to_string(),
        });
        let started = runtime.handle(ParallelModeControlPlaneCommand::RequestDispatch {
            workspace_directory: "/repo-a".to_string(),
            trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
        });
        let first = match started.effects.as_slice() {
            [
                ParallelModeControlPlaneEffect::MutateDispatchCommands {
                    correlation,
                    mutation: ParallelModeDispatchMutation::EnqueueForTrigger { .. },
                },
            ] => correlation.clone(),
            effects => panic!("expected one dispatch enqueue, got {effects:?}"),
        };

        assert!(
            runtime
                .handle(ParallelModeControlPlaneCommand::OpenEpoch {
                    workspace_directory: "/repo-b".to_string(),
                })
                .effects
                .is_empty(),
            "repo-a cancellation must queue behind its in-flight enqueue"
        );
        assert!(
            runtime
                .handle(ParallelModeControlPlaneCommand::OpenEpoch {
                    workspace_directory: "/repo-a".to_string(),
                })
                .effects
                .is_empty(),
            "repo-b cancellation must preserve FIFO ordering during ABA"
        );
        assert_eq!(runtime.store().current_epoch_id, Some(3));

        let mut forged = first.clone();
        forged.operation_id = forged.operation_id.saturating_add(10);
        let forged_completion =
            runtime.handle(ParallelModeControlPlaneCommand::DispatchMutationCompleted {
                correlation: forged,
                result: Ok(1),
            });
        assert!(matches!(
            forged_completion.events.as_slice(),
            [ParallelModeControlPlaneEvent::StaleCommandDropped { reason, .. }]
                if reason == "unknown dispatch mutation"
        ));
        assert_eq!(
            runtime
                .store
                .dispatch_mutation_in_flight
                .as_ref()
                .map(|in_flight| &in_flight.correlation),
            Some(&first)
        );

        let stale_enqueue =
            runtime.handle(ParallelModeControlPlaneCommand::DispatchMutationCompleted {
                correlation: first.clone(),
                result: Ok(1),
            });
        assert!(matches!(
            stale_enqueue.events.as_slice(),
            [ParallelModeControlPlaneEvent::StaleCommandDropped { reason, .. }]
                if reason == "dispatch mutation belongs to a stale epoch"
        ));
        let first_cancel = match stale_enqueue.effects.as_slice() {
            [
                ParallelModeControlPlaneEffect::MutateDispatchCommands {
                    correlation,
                    mutation: ParallelModeDispatchMutation::Cancel { .. },
                },
            ] => correlation.clone(),
            effects => panic!("expected repo-a cancellation after stale enqueue, got {effects:?}"),
        };
        assert!(first_cancel.operation_id > first.operation_id);

        let duplicate =
            runtime.handle(ParallelModeControlPlaneCommand::DispatchMutationCompleted {
                correlation: first,
                result: Ok(1),
            });
        assert!(matches!(
            duplicate.events.as_slice(),
            [ParallelModeControlPlaneEvent::StaleCommandDropped { reason, .. }]
                if reason == "unknown dispatch mutation"
        ));
        assert_eq!(
            runtime
                .store
                .dispatch_mutation_in_flight
                .as_ref()
                .map(|in_flight| &in_flight.correlation),
            Some(&first_cancel)
        );

        let stale_cancel_failure =
            runtime.handle(ParallelModeControlPlaneCommand::DispatchMutationCompleted {
                correlation: first_cancel,
                result: Err("sqlite busy".to_string()),
            });
        assert!(matches!(
            stale_cancel_failure.events.as_slice(),
            [ParallelModeControlPlaneEvent::DispatchMutationFailed {
                workspace_directory,
                epoch_id: 1,
                presentation_current: false,
                ..
            }] if workspace_directory == "/repo-a"
        ));
        assert!(matches!(
            stale_cancel_failure.effects.as_slice(),
            [ParallelModeControlPlaneEffect::MutateDispatchCommands {
                mutation: ParallelModeDispatchMutation::Cancel { .. },
                ..
            }]
        ));
    }

    #[test]
    fn dispatch_mutation_completion_preserves_refresh_wake_then_poll_priority() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: "/repo".to_string(),
        });
        let enqueue = runtime.handle(ParallelModeControlPlaneCommand::RequestDispatch {
            workspace_directory: "/repo".to_string(),
            trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
        });
        let enqueue_correlation = match enqueue.effects.as_slice() {
            [ParallelModeControlPlaneEffect::MutateDispatchCommands { correlation, .. }] => {
                correlation.clone()
            }
            effects => panic!("expected one dispatch enqueue, got {effects:?}"),
        };
        assert!(
            runtime
                .handle(ParallelModeControlPlaneCommand::RefreshSupervisor {
                    workspace_directory: "/repo".to_string(),
                })
                .effects
                .is_empty()
        );
        assert!(runtime.handle(wake("/repo", 1)).effects.is_empty());
        assert!(
            runtime
                .handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
                    workspace_directory: "/repo".to_string(),
                    follow_up_tick_signature: Some("queued-tick".to_string()),
                })
                .effects
                .is_empty()
        );

        let refreshed =
            runtime.handle(ParallelModeControlPlaneCommand::DispatchMutationCompleted {
                correlation: enqueue_correlation,
                result: Ok(1),
            });
        let refresh_id = only_effect(&refreshed)
            .effect_id()
            .expect("supervisor refresh should run first");
        assert_eq!(
            refresh_id.kind,
            ParallelModeControlPlaneEffectKind::RefreshSupervisor
        );

        let woke = runtime.handle(
            ParallelModeControlPlaneCommand::SupervisorSnapshotRefreshCompleted {
                workspace_directory: "/repo".to_string(),
                epoch_id: 1,
                effect_id: refresh_id,
                follow_up_tick_signature: None,
            },
        );
        let wake_id = only_effect(&woke)
            .effect_id()
            .expect("queued wake should run after refresh");
        assert_eq!(
            wake_id.kind,
            ParallelModeControlPlaneEffectKind::RunOrchestrator
        );

        let polled = runtime.handle(ParallelModeControlPlaneCommand::OrchestratorWakeCompleted {
            workspace_directory: "/repo".to_string(),
            epoch_id: 1,
            effect_id: wake_id,
            mode_enabled: true,
            follow_up_tick_signature: None,
        });
        assert!(matches!(
            polled.effects.as_slice(),
            [ParallelModeControlPlaneEffect::PollPendingDispatchWake { correlation }]
                if correlation.workspace_directory == "/repo" && correlation.epoch_id == 1
        ));
        assert_eq!(
            runtime
                .store
                .pending_dispatch_poll_in_flight
                .as_ref()
                .and_then(|in_flight| in_flight.follow_up_tick_signature.as_deref()),
            Some("queued-tick")
        );
        assert!(runtime.store.pending_dispatch_poll.is_none());
    }

    #[test]
    fn post_turn_queue_continuation_opens_epoch_and_requests_dispatch() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        runtime.force_mode_for_test("/repo", true);

        let requested = runtime.handle(ParallelModeControlPlaneCommand::ContinuePostTurnQueue {
            workspace_directory: "/repo".to_string(),
            signal: Some(ParallelModePostTurnQueueSignal::AutoFollowQueued),
            auto_follow_prompt_queued: true,
            has_actionable_queue_head: false,
        });

        assert!(
            requested
                .events
                .contains(&ParallelModeControlPlaneEvent::PostTurnAutoFollowPromptConsumed)
        );
        assert!(requested.events.iter().any(|event| {
            matches!(
                event,
                ParallelModeControlPlaneEvent::PostTurnDispatchRequested { epoch_id: 1, .. }
            )
        }));
        assert!(matches!(
            requested.effects.as_slice(),
            [ParallelModeControlPlaneEffect::RunOrchestrator { wake, .. }]
                if wake.trigger == ParallelModeAutomationTrigger::MainTurnPostEvaluation
        ));
    }

    #[test]
    fn pending_dispatch_poll_runs_tick_when_no_wake_is_pending() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory: "/repo".to_string(),
        });
        let started = runtime.handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
            workspace_directory: "/repo".to_string(),
            follow_up_tick_signature: Some("tick-sig".to_string()),
        });
        let correlation = match only_effect(&started) {
            ParallelModeControlPlaneEffect::PollPendingDispatchWake { correlation } => correlation,
            effect => panic!("expected pending dispatch poll effect, got {effect:?}"),
        };

        let polled = runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
            correlation,
            result: Ok(None),
        });

        assert!(matches!(
            polled.effects.as_slice(),
            [ParallelModeControlPlaneEffect::RunOrchestratorTick { signature, .. }]
                if signature == "tick-sig"
        ));
    }

    #[test]
    fn worker_completed_event_refreshes_projection_and_wakes_when_queue_has_work() {
        let event = ParallelModeControlPlaneWorkerEvent::new(
            "/repo",
            9,
            "task-1",
            "Task One",
            ParallelModeControlPlaneWorkerEventKind::Completed,
            vec!["official completion refreshed".to_string()],
        );

        let decision = ParallelModeControlPlaneAggregate::worker_event_decision(
            &event.workspace_directory,
            event.epoch_id,
            event.kind,
            Some("/repo"),
            Some(9),
            true,
        );

        assert_eq!(
            worker_event_to_control_plane_event(&event),
            ParallelModeControlPlaneEvent::WorkerCompleted {
                workspace_directory: "/repo".to_string(),
                epoch_id: 9,
                task_id: "task-1".to_string(),
            }
        );
        assert!(decision.stale_drop_reason.is_none());
        assert_eq!(event.notices, vec!["official completion refreshed"]);
        assert!(decision.refresh_supervisor);
        assert_eq!(
            decision.wake_trigger,
            Some(ParallelModeAutomationTrigger::ParallelOfficialCompletion)
        );
    }

    #[test]
    fn worker_failure_event_refreshes_projection_without_waking_dispatch() {
        let event = ParallelModeControlPlaneWorkerEvent::new(
            "/repo",
            9,
            "task-1",
            "Task One",
            ParallelModeControlPlaneWorkerEventKind::LaunchFailed,
            vec!["launch failed".to_string()],
        );

        let decision = ParallelModeControlPlaneAggregate::worker_event_decision(
            &event.workspace_directory,
            event.epoch_id,
            event.kind,
            Some("/repo"),
            Some(9),
            true,
        );

        assert_eq!(
            worker_event_to_control_plane_event(&event),
            ParallelModeControlPlaneEvent::WorkerLaunchFailed {
                workspace_directory: "/repo".to_string(),
                epoch_id: 9,
                task_id: "task-1".to_string(),
            }
        );
        assert_eq!(event.notices, vec!["launch failed"]);
        assert!(decision.refresh_supervisor);
        assert!(decision.wake_trigger.is_none());
    }

    #[test]
    fn stale_worker_event_is_dropped_before_ui_effects() {
        let event = ParallelModeControlPlaneWorkerEvent::new(
            "/repo",
            9,
            "task-1",
            "Task One",
            ParallelModeControlPlaneWorkerEventKind::Completed,
            vec!["late completion".to_string()],
        );

        let decision = ParallelModeControlPlaneAggregate::worker_event_decision(
            &event.workspace_directory,
            event.epoch_id,
            event.kind,
            Some("/repo"),
            Some(10),
            true,
        );

        assert_eq!(
            decision.stale_drop_reason,
            Some("worker event belongs to a stale epoch")
        );
        assert!(!decision.refresh_supervisor);
        assert!(decision.wake_trigger.is_none());
    }
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

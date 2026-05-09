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
};
pub use host::{ParallelModeControlPlaneEpochSnapshot, ParallelModeControlPlaneHandle};

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
        workspace_directory: String,
        epoch_id: u64,
        wake: Option<ParallelModeControlPlaneWake>,
        error: Option<String>,
        follow_up_tick_signature: Option<String>,
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
        trigger: ParallelModeAutomationTrigger,
        inserted_count: usize,
    },
    PostTurnAutoFollowPromptConsumed,
    PostTurnDispatchRequested {
        workspace_directory: String,
        epoch_id: u64,
    },
    ConversationRuntimeNotice {
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
        workspace_directory: String,
        mode_enabled: bool,
        reconcile_pool: bool,
        show_status: bool,
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
        workspace_directory: String,
        epoch_id: u64,
        follow_up_tick_signature: Option<String>,
    },
    EnqueueSlotCapacityDispatch {
        workspace_directory: String,
        epoch_id: u64,
    },
    EnqueueDispatchForTrigger {
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
        reason: String,
    },
    CancelDispatchCommands {
        workspace_directory: String,
        reason: String,
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
            | Self::EnqueueSlotCapacityDispatch { .. }
            | Self::EnqueueDispatchForTrigger { .. }
            | Self::CancelDispatchCommands { .. } => None,
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
    last_orchestrator_tick_signature: Option<String>,
    next_effect_sequence: u64,
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
            last_orchestrator_tick_signature: None,
            next_effect_sequence: 1,
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
        self.store.mode_enabled = enabled;
        if enabled {
            self.ensure_epoch(
                workspace_directory,
                &mut ParallelModeControlPlaneRuntimeOutcome::new(),
            );
            self.store.projection_ready = true;
        } else {
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
                workspace_directory,
                epoch_id,
                wake,
                error,
                follow_up_tick_signature,
            } => self.pending_dispatch_wake_polled(
                workspace_directory,
                epoch_id,
                wake,
                error,
                follow_up_tick_signature,
                &mut outcome,
            ),
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
        if let Some(epoch_id) = self.store.current_epoch_id.take() {
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
        outcome
            .effects
            .push(ParallelModeControlPlaneEffect::CancelDispatchCommands {
                workspace_directory,
                reason: "parallel mode disabled".to_string(),
            });
    }

    fn inspect_supervisor(
        &mut self,
        workspace_directory: String,
        reconcile_pool: bool,
        show_status: bool,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let decision = ParallelModeControlPlaneAggregate::supervisor_inspection(
            self.store.mode_enabled,
            self.store.workspace_directory.as_deref(),
            &workspace_directory,
            reconcile_pool,
        );
        outcome
            .effects
            .push(ParallelModeControlPlaneEffect::InspectSupervisor {
                workspace_directory,
                mode_enabled: decision.mode_enabled,
                reconcile_pool: decision.reconcile_pool,
                show_status,
            });
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
            outcome.effects.push(
                ParallelModeControlPlaneEffect::EnqueueSlotCapacityDispatch {
                    workspace_directory,
                    epoch_id,
                },
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
        for notice in event.notices {
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::ConversationRuntimeNotice { notice });
        }
        if decision.refresh_supervisor {
            let Some(workspace_directory) = self.store.workspace_directory.clone() else {
                return;
            };
            let Some(epoch_id) = self.store.current_epoch_id else {
                return;
            };
            self.start_or_queue_supervisor_refresh(workspace_directory, epoch_id, outcome);
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
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::SupervisorRefreshQueued);
                self.store.pending_supervisor_refresh = true;
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
            outcome
                .effects
                .push(ParallelModeControlPlaneEffect::EnqueueDispatchForTrigger {
                    workspace_directory,
                    trigger,
                    epoch_id,
                    reason: reason.to_string(),
                });
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
        if self.has_in_flight_effect() {
            return;
        }
        outcome
            .effects
            .push(ParallelModeControlPlaneEffect::PollPendingDispatchWake {
                workspace_directory,
                epoch_id,
                follow_up_tick_signature,
            });
    }

    fn pending_dispatch_wake_polled(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        wake: Option<ParallelModeControlPlaneWake>,
        error: Option<String>,
        follow_up_tick_signature: Option<String>,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if !self.command_epoch_is_current(&workspace_directory, epoch_id, outcome) {
            return;
        }
        if let Some(error) = error {
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::DispatchWithheld {
                    trigger: Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
                    reason: format!("pending dispatch command poll failed: {error}"),
                });
        }
        match ParallelModeControlPlaneAggregate::pending_dispatch_wake_decision(
            wake.is_some(),
            follow_up_tick_signature.is_some(),
        ) {
            ParallelModePendingDispatchWakeDecision::StartWake => {
                if let Some(wake) = wake {
                    self.wake_orchestrator(wake, outcome);
                }
            }
            ParallelModePendingDispatchWakeDecision::RunFollowUpTick => {
                if let Some(signature) = follow_up_tick_signature {
                    self.run_orchestrator_tick(workspace_directory, signature, outcome);
                }
            }
            ParallelModePendingDispatchWakeDecision::Idle => {}
        }
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
        outcome
            .effects
            .push(ParallelModeControlPlaneEffect::PollPendingDispatchWake {
                workspace_directory,
                epoch_id,
                follow_up_tick_signature,
            });
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
        match ParallelModeControlPlaneAggregate::effect_completion_follow_up(
            self.store.pending_supervisor_refresh,
        ) {
            ParallelModeControlPlaneEffectCompletionFollowUp::RefreshSupervisor => {
                self.store.pending_supervisor_refresh = false;
                self.start_supervisor_refresh(workspace_directory, epoch_id, outcome);
            }
            ParallelModeControlPlaneEffectCompletionFollowUp::DrainPendingWake => {
                self.drain_pending_orchestrator_wake(outcome);
            }
        }
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
                let epoch_id = self.store.next_epoch_id;
                self.store.next_epoch_id = self.store.next_epoch_id.saturating_add(1);
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
    }

    fn clear_process_effect_state(&mut self) {
        self.store.parallel_entry_in_flight = None;
        self.store.supervisor_refresh_in_flight = None;
        self.store.orchestrator_wake_in_flight = None;
        self.store.orchestrator_tick_in_flight = None;
        self.store.projection_ready = false;
        self.store.pending_supervisor_refresh = false;
        self.store.pending_orchestrator_wake = None;
        self.store.last_orchestrator_tick_signature = None;
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
            [ParallelModeControlPlaneEvent::EpochOpened {
                workspace_directory,
                epoch_id: 2,
            }] if workspace_directory == "/other"
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
                matches!(
                    effect,
                    ParallelModeControlPlaneEffect::RunOrchestrator { .. }
                )
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
            [ParallelModeControlPlaneEffect::CancelDispatchCommands { .. }]
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
            [ParallelModeControlPlaneEffect::CancelDispatchCommands { .. }]
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
            [ParallelModeControlPlaneEffect::PollPendingDispatchWake {
                follow_up_tick_signature: Some(signature),
                ..
            }] if signature == "tick-sig"
        ));
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
            [ParallelModeControlPlaneEffect::EnqueueDispatchForTrigger {
                trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
                epoch_id: 1,
                ..
            }]
        ));
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

        assert!(requested.events.iter().any(|event| {
            matches!(
                event,
                ParallelModeControlPlaneEvent::PostTurnAutoFollowPromptConsumed
            )
        }));
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

        let polled = runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
            workspace_directory: "/repo".to_string(),
            epoch_id: 1,
            wake: None,
            error: None,
            follow_up_tick_signature: Some("tick-sig".to_string()),
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

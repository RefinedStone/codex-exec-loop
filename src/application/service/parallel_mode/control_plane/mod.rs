use crate::domain::parallel_mode::ParallelModeAutomationTrigger;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModeControlPlaneEffectKind {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModeControlPlaneWorkerEventKind {
    WorkerCompleted,
    WorkerLaunchFailed,
    WorkerStreamFailed,
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

    pub fn reduce(
        self,
        current_workspace_directory: &str,
        current_epoch_id: Option<u64>,
        has_actionable_queue_head: bool,
    ) -> ParallelModeControlPlaneWorkerEventOutcome {
        if current_workspace_directory != self.workspace_directory {
            return ParallelModeControlPlaneWorkerEventOutcome::stale(
                self,
                "worker event targets a different workspace",
            );
        }
        if current_epoch_id != Some(self.epoch_id) {
            return ParallelModeControlPlaneWorkerEventOutcome::stale(
                self,
                "worker event belongs to a stale epoch",
            );
        }

        let event = match self.kind {
            ParallelModeControlPlaneWorkerEventKind::WorkerCompleted => {
                ParallelModeControlPlaneEvent::WorkerCompleted {
                    workspace_directory: self.workspace_directory.clone(),
                    epoch_id: self.epoch_id,
                    task_id: self.task_id.clone(),
                }
            }
            ParallelModeControlPlaneWorkerEventKind::WorkerLaunchFailed => {
                ParallelModeControlPlaneEvent::WorkerLaunchFailed {
                    workspace_directory: self.workspace_directory.clone(),
                    epoch_id: self.epoch_id,
                    task_id: self.task_id.clone(),
                }
            }
            ParallelModeControlPlaneWorkerEventKind::WorkerStreamFailed => {
                ParallelModeControlPlaneEvent::WorkerStreamFailed {
                    workspace_directory: self.workspace_directory.clone(),
                    epoch_id: self.epoch_id,
                    task_id: self.task_id.clone(),
                }
            }
        };
        let wake = (self.kind == ParallelModeControlPlaneWorkerEventKind::WorkerCompleted
            && has_actionable_queue_head)
            .then(|| {
                ParallelModeControlPlaneWake::new(
                    self.workspace_directory.clone(),
                    ParallelModeAutomationTrigger::ParallelOfficialCompletion,
                    self.epoch_id,
                    Some(ParallelModeAutomationTrigger::ParallelOfficialCompletion),
                )
            });
        ParallelModeControlPlaneWorkerEventOutcome {
            event,
            stale_drop_reason: None,
            notices: self.notices,
            refresh_supervisor: true,
            wake,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeControlPlaneWorkerEventOutcome {
    pub event: ParallelModeControlPlaneEvent,
    pub stale_drop_reason: Option<String>,
    pub notices: Vec<String>,
    pub refresh_supervisor: bool,
    pub wake: Option<ParallelModeControlPlaneWake>,
}

impl ParallelModeControlPlaneWorkerEventOutcome {
    fn stale(event: ParallelModeControlPlaneWorkerEvent, reason: &str) -> Self {
        Self {
            event: ParallelModeControlPlaneEvent::StaleCommandDropped {
                workspace_directory: event.workspace_directory,
                epoch_id: event.epoch_id,
                reason: reason.to_string(),
            },
            stale_drop_reason: Some(reason.to_string()),
            notices: Vec::new(),
            refresh_supervisor: false,
            wake: None,
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
    RefreshSupervisor {
        workspace_directory: String,
    },
    WakeOrchestrator(ParallelModeControlPlaneWake),
    RunOrchestratorTick {
        workspace_directory: String,
        signature: String,
    },
    WorkerCompleted {
        workspace_directory: String,
        epoch_id: u64,
        trigger: ParallelModeAutomationTrigger,
    },
    EffectCompleted {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
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
    SupervisorRefreshQueued,
    OrchestratorWakeQueued {
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
    },
    OrchestratorWakeDequeued {
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
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
    RefreshSupervisor {
        effect_id: ParallelModeControlPlaneEffectId,
        workspace_directory: String,
        epoch_id: u64,
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
    CancelDispatchCommands {
        workspace_directory: String,
        reason: String,
    },
}

impl ParallelModeControlPlaneEffect {
    pub fn effect_id(&self) -> Option<ParallelModeControlPlaneEffectId> {
        match self {
            Self::RefreshSupervisor { effect_id, .. }
            | Self::RunOrchestrator { effect_id, .. }
            | Self::RunOrchestratorTick { effect_id, .. } => Some(*effect_id),
            Self::CancelDispatchCommands { .. } => None,
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
    pub workspace_directory: Option<String>,
    pub current_epoch_id: Option<u64>,
    pub next_epoch_id: u64,
    pub supervisor_refresh_in_flight: Option<ParallelModeControlPlaneEffectId>,
    pub orchestrator_wake_in_flight: Option<ParallelModeControlPlaneEffectId>,
    pub orchestrator_tick_in_flight: Option<ParallelModeControlPlaneEffectId>,
    pub pending_supervisor_refresh: bool,
    pub pending_orchestrator_wake: Option<ParallelModeControlPlaneWake>,
    pub last_orchestrator_tick_signature: Option<String>,
    next_effect_sequence: u64,
}

impl Default for ParallelModeControlPlaneRuntimeStore {
    fn default() -> Self {
        Self {
            workspace_directory: None,
            current_epoch_id: None,
            next_epoch_id: 1,
            supervisor_refresh_in_flight: None,
            orchestrator_wake_in_flight: None,
            orchestrator_tick_in_flight: None,
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
        self.store.current_epoch_id = Some(epoch_id);
        self.store.next_epoch_id = self.store.next_epoch_id.max(epoch_id.saturating_add(1));
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
            ParallelModeControlPlaneCommand::WorkerCompleted {
                workspace_directory,
                epoch_id,
                trigger,
            } => self.wake_orchestrator(
                ParallelModeControlPlaneWake::new(workspace_directory, trigger, epoch_id, None),
                &mut outcome,
            ),
            ParallelModeControlPlaneCommand::EffectCompleted {
                workspace_directory,
                epoch_id,
                effect_id,
            } => self.effect_completed(workspace_directory, epoch_id, effect_id, &mut outcome),
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
        let epoch_id = self.ensure_epoch(workspace_directory.clone(), outcome);
        self.start_or_queue_supervisor_refresh(workspace_directory, epoch_id, outcome);
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
        self.store.workspace_directory = None;
        self.clear_process_effect_state();
        outcome
            .effects
            .push(ParallelModeControlPlaneEffect::CancelDispatchCommands {
                workspace_directory,
                reason: "parallel mode disabled".to_string(),
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
        if self.has_in_flight_effect() {
            self.store.pending_orchestrator_wake = Some(wake.clone());
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::OrchestratorWakeQueued {
                    trigger: wake.trigger,
                    epoch_id: wake.epoch_id,
                });
            return;
        }
        self.start_orchestrator_wake(wake, outcome);
    }

    fn effect_completed(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if !self.command_epoch_is_current(&workspace_directory, epoch_id, outcome) {
            return;
        }
        match effect_id.kind {
            ParallelModeControlPlaneEffectKind::RefreshSupervisor => {
                if self.store.supervisor_refresh_in_flight != Some(effect_id) {
                    self.stale_command(
                        workspace_directory,
                        epoch_id,
                        "unknown supervisor refresh",
                        outcome,
                    );
                    return;
                }
                self.store.supervisor_refresh_in_flight = None;
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::EffectCompleted { effect_id });
                if self.store.pending_supervisor_refresh {
                    self.store.pending_supervisor_refresh = false;
                    self.start_supervisor_refresh(workspace_directory, epoch_id, outcome);
                    return;
                }
                self.drain_pending_orchestrator_wake(outcome);
            }
            ParallelModeControlPlaneEffectKind::RunOrchestrator => {
                if self.store.orchestrator_wake_in_flight != Some(effect_id) {
                    self.stale_command(
                        workspace_directory,
                        epoch_id,
                        "unknown orchestrator wake",
                        outcome,
                    );
                    return;
                }
                self.store.orchestrator_wake_in_flight = None;
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::EffectCompleted { effect_id });
                if self.store.pending_supervisor_refresh {
                    self.store.pending_supervisor_refresh = false;
                    self.start_supervisor_refresh(workspace_directory, epoch_id, outcome);
                    return;
                }
                self.drain_pending_orchestrator_wake(outcome);
            }
            ParallelModeControlPlaneEffectKind::RunOrchestratorTick => {
                if self.store.orchestrator_tick_in_flight != Some(effect_id) {
                    self.stale_command(
                        workspace_directory,
                        epoch_id,
                        "unknown orchestrator tick",
                        outcome,
                    );
                    return;
                }
                self.store.orchestrator_tick_in_flight = None;
                outcome
                    .events
                    .push(ParallelModeControlPlaneEvent::EffectCompleted { effect_id });
                if self.store.pending_supervisor_refresh {
                    self.store.pending_supervisor_refresh = false;
                    self.start_supervisor_refresh(workspace_directory, epoch_id, outcome);
                    return;
                }
                self.drain_pending_orchestrator_wake(outcome);
            }
        }
    }

    fn start_or_queue_supervisor_refresh(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if self.has_in_flight_effect() {
            self.store.pending_supervisor_refresh = true;
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::SupervisorRefreshQueued);
            return;
        }
        self.start_supervisor_refresh(workspace_directory, epoch_id, outcome);
    }

    fn start_supervisor_refresh(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        let effect_id = self.next_effect_id(ParallelModeControlPlaneEffectKind::RefreshSupervisor);
        self.store.supervisor_refresh_in_flight = Some(effect_id);
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
        if self.has_in_flight_effect() {
            return;
        }
        if self.store.last_orchestrator_tick_signature.as_deref() == Some(signature.as_str()) {
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

    fn drain_pending_orchestrator_wake(
        &mut self,
        outcome: &mut ParallelModeControlPlaneRuntimeOutcome,
    ) {
        if self.has_in_flight_effect() {
            return;
        }
        let Some(wake) = self.store.pending_orchestrator_wake.take() else {
            return;
        };
        if !self.wake_epoch_is_current(&wake) {
            outcome
                .events
                .push(ParallelModeControlPlaneEvent::StaleCommandDropped {
                    workspace_directory: wake.workspace_directory,
                    epoch_id: wake.epoch_id,
                    reason: "pending wake belongs to a stale epoch".to_string(),
                });
            return;
        }
        outcome
            .events
            .push(ParallelModeControlPlaneEvent::OrchestratorWakeDequeued {
                trigger: wake.trigger,
                epoch_id: wake.epoch_id,
            });
        self.start_orchestrator_wake(wake, outcome);
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
        self.store.supervisor_refresh_in_flight.is_some()
            || self.store.orchestrator_wake_in_flight.is_some()
            || self.store.orchestrator_tick_in_flight.is_some()
    }

    fn clear_process_effect_state(&mut self) {
        self.store.supervisor_refresh_in_flight = None;
        self.store.orchestrator_wake_in_flight = None;
        self.store.orchestrator_tick_in_flight = None;
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
        match (
            self.store.workspace_directory.as_deref(),
            self.store.current_epoch_id,
        ) {
            (Some(current_workspace), Some(epoch_id))
                if current_workspace == workspace_directory =>
            {
                Some(epoch_id)
            }
            _ => {
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
        if self.store.workspace_directory.as_deref() == Some(workspace_directory)
            && self.store.current_epoch_id == Some(epoch_id)
        {
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
        self.store.workspace_directory.as_deref() == Some(wake.workspace_directory.as_str())
            && self.store.current_epoch_id == Some(wake.epoch_id)
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
    fn enable_opens_epoch_and_orders_refresh_before_orchestrator_wake() {
        let mut runtime = ParallelModeControlPlaneRuntime::new();
        let enabled = runtime.handle(enable("/repo"));
        let refresh = only_effect(&enabled);
        assert!(matches!(
            enabled.events.as_slice(),
            [
                ParallelModeControlPlaneEvent::EpochOpened { epoch_id: 1, .. },
                ParallelModeControlPlaneEvent::EffectStarted { .. }
            ]
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

        let refresh_id = refresh.effect_id().expect("refresh effect should have id");
        let completed = runtime.handle(completed("/repo", 1, refresh_id));
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
    fn worker_completed_event_refreshes_projection_and_wakes_when_queue_has_work() {
        let event = ParallelModeControlPlaneWorkerEvent::new(
            "/repo",
            9,
            "task-1",
            "Task One",
            ParallelModeControlPlaneWorkerEventKind::WorkerCompleted,
            vec!["official completion refreshed".to_string()],
        );

        let outcome = event.reduce("/repo", Some(9), true);

        assert_eq!(
            outcome.event,
            ParallelModeControlPlaneEvent::WorkerCompleted {
                workspace_directory: "/repo".to_string(),
                epoch_id: 9,
                task_id: "task-1".to_string(),
            }
        );
        assert!(outcome.stale_drop_reason.is_none());
        assert_eq!(outcome.notices, vec!["official completion refreshed"]);
        assert!(outcome.refresh_supervisor);
        assert_eq!(
            outcome.wake,
            Some(ParallelModeControlPlaneWake::new(
                "/repo",
                ParallelModeAutomationTrigger::ParallelOfficialCompletion,
                9,
                Some(ParallelModeAutomationTrigger::ParallelOfficialCompletion),
            ))
        );
    }

    #[test]
    fn worker_failure_event_refreshes_projection_without_waking_dispatch() {
        let event = ParallelModeControlPlaneWorkerEvent::new(
            "/repo",
            9,
            "task-1",
            "Task One",
            ParallelModeControlPlaneWorkerEventKind::WorkerLaunchFailed,
            vec!["launch failed".to_string()],
        );

        let outcome = event.reduce("/repo", Some(9), true);

        assert_eq!(
            outcome.event,
            ParallelModeControlPlaneEvent::WorkerLaunchFailed {
                workspace_directory: "/repo".to_string(),
                epoch_id: 9,
                task_id: "task-1".to_string(),
            }
        );
        assert_eq!(outcome.notices, vec!["launch failed"]);
        assert!(outcome.refresh_supervisor);
        assert!(outcome.wake.is_none());
    }

    #[test]
    fn stale_worker_event_is_dropped_before_ui_effects() {
        let event = ParallelModeControlPlaneWorkerEvent::new(
            "/repo",
            9,
            "task-1",
            "Task One",
            ParallelModeControlPlaneWorkerEventKind::WorkerCompleted,
            vec!["late completion".to_string()],
        );

        let outcome = event.reduce("/repo", Some(10), true);

        assert_eq!(
            outcome.event,
            ParallelModeControlPlaneEvent::StaleCommandDropped {
                workspace_directory: "/repo".to_string(),
                epoch_id: 9,
                reason: "worker event belongs to a stale epoch".to_string(),
            }
        );
        assert!(outcome.stale_drop_reason.is_some());
        assert!(outcome.notices.is_empty());
        assert!(!outcome.refresh_supervisor);
        assert!(outcome.wake.is_none());
    }
}

use std::time::{Duration, Instant};

use crate::application::service::parallel_mode::control_plane::effect_runner::{
    ParallelModeControlPlaneBackgroundEvent, ParallelModeControlPlaneEffectRunner,
    ParallelModeControlPlaneEventSink, ParallelModeControlPlaneLoadingStage,
};
use crate::diagnostics::event_log;
use crate::domain::parallel_mode::{
    ParallelModeAutomationTrigger, ParallelModeDispatchOutcome,
    ParallelModeOrchestratorStateMachine, ParallelModePostTurnQueueDecision,
    ParallelModePostTurnQueueSignal, ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot,
};

use super::{
    ParallelModeControlPlaneCommand, ParallelModeControlPlaneEffect,
    ParallelModeControlPlaneEffectId, ParallelModeControlPlaneEvent,
    ParallelModeControlPlaneRuntime, ParallelModeControlPlaneRuntimeOutcome,
    ParallelModeControlPlaneRuntimeStore, ParallelModeControlPlaneWorkerEvent,
};

const CONTROL_PLANE_TICK_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub enum ParallelModeControlPlanePresentationEvent {
    EnterProgress {
        workspace_directory: String,
        readiness_snapshot: Option<ParallelModeReadinessSnapshot>,
        loading_stage: ParallelModeControlPlaneLoadingStage,
        status_text: String,
    },
    ReadinessSnapshotChanged {
        workspace_directory: String,
        snapshot: ParallelModeReadinessSnapshot,
    },
    SupervisorSnapshotChanged {
        workspace_directory: String,
        snapshot: Box<ParallelModeSupervisorSnapshot>,
    },
    StatusShown {
        status_text: String,
    },
    ConversationRuntimeNotice {
        notice: String,
    },
    PlanningRuntimeRefreshRequested {
        workspace_directory: String,
    },
    ModeDisabled {
        workspace_directory: String,
    },
}

pub struct ParallelModeControlPlaneController<S>
where
    S: ParallelModeControlPlaneEventSink,
{
    runtime: ParallelModeControlPlaneRuntime,
    effect_runner: ParallelModeControlPlaneEffectRunner<S>,
    readiness_snapshot: Option<ParallelModeReadinessSnapshot>,
    last_automation_trigger: Option<ParallelModeAutomationTrigger>,
    last_dispatch_withheld_reason: Option<String>,
    last_supervisor_refresh_at: Option<Instant>,
    last_orchestrator_wake_poll_at: Option<Instant>,
}

impl<S> ParallelModeControlPlaneController<S>
where
    S: ParallelModeControlPlaneEventSink,
{
    pub fn new(effect_runner: ParallelModeControlPlaneEffectRunner<S>) -> Self {
        Self {
            runtime: ParallelModeControlPlaneRuntime::new(),
            effect_runner,
            readiness_snapshot: None,
            last_automation_trigger: None,
            last_dispatch_withheld_reason: None,
            last_supervisor_refresh_at: None,
            last_orchestrator_wake_poll_at: None,
        }
    }

    pub fn store(&self) -> &ParallelModeControlPlaneRuntimeStore {
        self.runtime.store()
    }

    pub fn mode_enabled(&self) -> bool {
        self.runtime.mode_enabled()
    }

    pub fn current_epoch_id(&self) -> Option<u64> {
        self.runtime.store().current_epoch_id
    }

    pub fn supervisor_refresh_in_flight(&self) -> bool {
        self.runtime.store().supervisor_refresh_in_flight.is_some()
    }

    pub fn orchestrator_wake_in_flight(&self) -> bool {
        self.runtime.store().orchestrator_wake_in_flight.is_some()
    }

    pub fn orchestrator_tick_in_flight(&self) -> bool {
        self.runtime.store().orchestrator_tick_in_flight.is_some()
    }

    pub fn control_effect_in_flight(&self) -> bool {
        self.runtime.store().parallel_entry_in_flight.is_some()
            || self.supervisor_refresh_in_flight()
            || self.orchestrator_wake_in_flight()
            || self.orchestrator_tick_in_flight()
    }

    pub fn last_automation_trigger(&self) -> Option<ParallelModeAutomationTrigger> {
        self.last_automation_trigger
    }

    pub fn last_dispatch_withheld_reason(&self) -> Option<&str> {
        self.last_dispatch_withheld_reason.as_deref()
    }

    pub fn clear_dispatch_withheld_reason(&mut self) {
        self.last_dispatch_withheld_reason = None;
    }

    pub fn set_readiness_snapshot(&mut self, snapshot: ParallelModeReadinessSnapshot) {
        self.readiness_snapshot = Some(snapshot);
    }

    pub fn reset_orchestrator_tick_signature(&mut self) {
        self.runtime.reset_orchestrator_tick_signature();
    }

    pub fn decide_post_turn_queue_continuation(
        &self,
        signal: Option<ParallelModePostTurnQueueSignal>,
        has_actionable_queue_head: bool,
    ) -> ParallelModePostTurnQueueDecision {
        ParallelModeOrchestratorStateMachine::post_turn_queue_continuation(
            self.mode_enabled(),
            signal,
            has_actionable_queue_head,
        )
    }

    pub fn handle_command(
        &mut self,
        command: ParallelModeControlPlaneCommand,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        let outcome = self.runtime.handle(command);
        self.drain_outcome(outcome)
    }

    pub fn handle_background_event(
        &mut self,
        event: ParallelModeControlPlaneBackgroundEvent,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        match event {
            ParallelModeControlPlaneBackgroundEvent::EnterProgress {
                workspace_directory,
                readiness_snapshot,
                loading_stage,
                status_text,
            } => self.enter_progress(
                workspace_directory,
                readiness_snapshot,
                loading_stage,
                status_text,
            ),
            ParallelModeControlPlaneBackgroundEvent::Entered {
                workspace_directory,
                epoch_id,
                effect_id,
                mode_was_enabled,
                readiness_snapshot,
                supervisor_snapshot,
                status_text,
                initial_pool_reset_completed,
                has_actionable_queue_head,
                orchestrator_tick_signature,
            } => self.entry_completed(ParallelModeEntryResult {
                workspace_directory,
                epoch_id,
                effect_id,
                mode_was_enabled,
                readiness_snapshot,
                supervisor_snapshot: *supervisor_snapshot,
                status_text,
                initial_pool_reset_completed,
                has_actionable_queue_head,
                follow_up_tick_signature: orchestrator_tick_signature,
            }),
            ParallelModeControlPlaneBackgroundEvent::SupervisorSnapshotRefreshed {
                workspace_directory,
                epoch_id,
                effect_id,
                supervisor_snapshot,
                orchestrator_tick_signature,
            } => self.supervisor_snapshot_refreshed(
                workspace_directory,
                epoch_id,
                effect_id,
                *supervisor_snapshot,
                orchestrator_tick_signature,
            ),
            ParallelModeControlPlaneBackgroundEvent::OrchestratorWakeCompleted {
                workspace_directory,
                effect_id,
                readiness_snapshot,
                supervisor_snapshot,
                outcome,
                orchestrator_tick_signature,
            } => self.orchestrator_wake_completed(
                workspace_directory,
                effect_id,
                readiness_snapshot,
                *supervisor_snapshot,
                outcome,
                orchestrator_tick_signature,
            ),
            ParallelModeControlPlaneBackgroundEvent::WorkerEvent {
                event,
                has_actionable_queue_head,
            } => self.worker_event_received(event, has_actionable_queue_head),
            ParallelModeControlPlaneBackgroundEvent::ConversationRuntimeNotice(notice) => {
                vec![
                    ParallelModeControlPlanePresentationEvent::ConversationRuntimeNotice { notice },
                ]
            }
            ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
                workspace_directory,
                epoch_id,
                effect_id,
                blocked,
                notices,
            } => self.orchestrator_tick_completed(
                workspace_directory,
                epoch_id,
                effect_id,
                blocked,
                notices,
            ),
        }
    }

    pub fn tick(
        &mut self,
        now: Instant,
        workspace_directory: String,
        activity_pulse_visible: bool,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        let mut events = Vec::new();
        if self.supervisor_refresh_due(now, activity_pulse_visible) {
            self.last_supervisor_refresh_at = Some(now);
            events.extend(self.handle_command(
                ParallelModeControlPlaneCommand::RefreshSupervisor {
                    workspace_directory: workspace_directory.clone(),
                },
            ));
        }
        if self.orchestrator_wake_poll_due(now) {
            self.last_orchestrator_wake_poll_at = Some(now);
            events.extend(self.poll_pending_dispatch_wake(workspace_directory, None));
        }
        events
    }

    pub fn supervisor_refresh_due(&self, now: Instant, activity_pulse_visible: bool) -> bool {
        if self.control_effect_in_flight() {
            return false;
        }
        if !activity_pulse_visible {
            return false;
        }

        self.last_supervisor_refresh_at.is_none_or(|last_refresh| {
            now.duration_since(last_refresh) >= CONTROL_PLANE_TICK_INTERVAL
        })
    }

    pub fn orchestrator_wake_poll_due(&self, now: Instant) -> bool {
        if self.control_effect_in_flight() {
            return false;
        }

        self.last_orchestrator_wake_poll_at
            .is_none_or(|last_poll| now.duration_since(last_poll) >= CONTROL_PLANE_TICK_INTERVAL)
    }

    pub fn poll_pending_dispatch_wake(
        &mut self,
        workspace_directory: String,
        follow_up_tick_signature: Option<String>,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        if !self.mode_enabled() || self.current_epoch_id().is_none() {
            return Vec::new();
        }
        if !self.runtime.store().projection_ready || self.control_effect_in_flight() {
            return Vec::new();
        }
        self.handle_command(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
            workspace_directory,
            follow_up_tick_signature,
        })
    }

    #[cfg(test)]
    pub fn force_mode_for_test(&mut self, workspace_directory: impl Into<String>, enabled: bool) {
        self.runtime
            .force_mode_for_test(workspace_directory, enabled);
    }

    #[cfg(test)]
    pub fn force_initial_pool_reset_completed_for_test(&mut self, completed: bool) {
        self.runtime
            .force_initial_pool_reset_completed_for_test(completed);
    }

    #[cfg(test)]
    pub fn force_epoch_for_test(&mut self, workspace_directory: impl Into<String>, epoch_id: u64) {
        self.runtime
            .force_epoch_for_test(workspace_directory, epoch_id);
    }

    #[cfg(test)]
    pub fn force_supervisor_refresh_in_flight_for_test(
        &mut self,
        workspace_directory: impl Into<String>,
        epoch_id: u64,
    ) -> ParallelModeControlPlaneEffectId {
        self.runtime
            .force_supervisor_refresh_in_flight_for_test(workspace_directory, epoch_id)
    }

    fn enter_progress(
        &mut self,
        workspace_directory: String,
        readiness_snapshot: Option<ParallelModeReadinessSnapshot>,
        loading_stage: ParallelModeControlPlaneLoadingStage,
        status_text: String,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        if !self.event_targets_active_workspace(&workspace_directory) {
            return Vec::new();
        }
        if let Some(readiness_snapshot) = readiness_snapshot.as_ref() {
            self.readiness_snapshot = Some(readiness_snapshot.clone());
        }
        vec![ParallelModeControlPlanePresentationEvent::EnterProgress {
            workspace_directory,
            readiness_snapshot,
            loading_stage,
            status_text,
        }]
    }

    fn entry_completed(
        &mut self,
        result: ParallelModeEntryResult,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        let ParallelModeEntryResult {
            workspace_directory,
            epoch_id,
            effect_id,
            mode_was_enabled,
            readiness_snapshot,
            supervisor_snapshot,
            status_text,
            initial_pool_reset_completed,
            has_actionable_queue_head,
            follow_up_tick_signature,
        } = result;

        if !self.event_targets_active_workspace(&workspace_directory) {
            let outcome = self
                .runtime
                .handle(ParallelModeControlPlaneCommand::EntryCompleted {
                    workspace_directory,
                    epoch_id,
                    effect_id,
                    mode_enabled: false,
                    mode_was_enabled,
                    initial_pool_reset_completed: false,
                    has_actionable_queue_head: false,
                    follow_up_tick_signature: None,
                });
            return self.drain_outcome(outcome);
        }

        let mode_enabled = readiness_snapshot.allows_parallel_mode();
        let outcome = self
            .runtime
            .handle(ParallelModeControlPlaneCommand::EntryCompleted {
                workspace_directory: workspace_directory.clone(),
                epoch_id,
                effect_id,
                mode_enabled,
                mode_was_enabled,
                initial_pool_reset_completed,
                has_actionable_queue_head,
                follow_up_tick_signature,
            });
        if !outcome_effect_completed(&outcome, effect_id) {
            return self.drain_outcome(outcome);
        }

        self.readiness_snapshot = Some(readiness_snapshot.clone());
        let mut events = vec![
            ParallelModeControlPlanePresentationEvent::ReadinessSnapshotChanged {
                workspace_directory: workspace_directory.clone(),
                snapshot: readiness_snapshot,
            },
            ParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged {
                workspace_directory: workspace_directory.clone(),
                snapshot: Box::new(supervisor_snapshot),
            },
            ParallelModeControlPlanePresentationEvent::PlanningRuntimeRefreshRequested {
                workspace_directory,
            },
            ParallelModeControlPlanePresentationEvent::StatusShown { status_text },
        ];
        events.extend(self.drain_outcome(outcome));
        events
    }

    fn supervisor_snapshot_refreshed(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        supervisor_snapshot: ParallelModeSupervisorSnapshot,
        follow_up_tick_signature: Option<String>,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        if !self.event_targets_active_workspace(&workspace_directory) {
            let outcome = self
                .runtime
                .handle(ParallelModeControlPlaneCommand::Disable {
                    workspace_directory,
                });
            return self.drain_outcome(outcome);
        }

        let outcome = self.runtime.handle(
            ParallelModeControlPlaneCommand::SupervisorSnapshotRefreshCompleted {
                workspace_directory: workspace_directory.clone(),
                epoch_id,
                effect_id,
                follow_up_tick_signature,
            },
        );
        if !outcome_effect_completed(&outcome, effect_id) {
            return self.drain_outcome(outcome);
        }

        let mut events = vec![
            ParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged {
                workspace_directory,
                snapshot: Box::new(supervisor_snapshot),
            },
        ];
        events.extend(self.drain_outcome(outcome));
        events
    }

    fn orchestrator_wake_completed(
        &mut self,
        workspace_directory: String,
        effect_id: ParallelModeControlPlaneEffectId,
        readiness_snapshot: ParallelModeReadinessSnapshot,
        supervisor_snapshot: ParallelModeSupervisorSnapshot,
        outcome: ParallelModeDispatchOutcome,
        follow_up_tick_signature: Option<String>,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        if !self.event_targets_active_workspace(&workspace_directory) {
            let runtime_outcome = self
                .runtime
                .handle(ParallelModeControlPlaneCommand::Disable {
                    workspace_directory,
                });
            return self.drain_outcome(runtime_outcome);
        }

        let mode_enabled = readiness_snapshot.allows_parallel_mode();
        let runtime_outcome =
            self.runtime
                .handle(ParallelModeControlPlaneCommand::OrchestratorWakeCompleted {
                    workspace_directory: workspace_directory.clone(),
                    epoch_id: outcome.epoch_id,
                    effect_id,
                    mode_enabled,
                    follow_up_tick_signature,
                });
        if !outcome_effect_completed(&runtime_outcome, effect_id) {
            return self.drain_outcome(runtime_outcome);
        }

        self.readiness_snapshot = Some(readiness_snapshot.clone());
        self.record_dispatch_completed(&workspace_directory, &outcome);
        let status_text = format!(
            "parallel mode: dispatch refreshed / trigger: {} / {}",
            outcome.trigger.label(),
            outcome.status_detail()
        );
        let mut events = vec![
            ParallelModeControlPlanePresentationEvent::ReadinessSnapshotChanged {
                workspace_directory: workspace_directory.clone(),
                snapshot: readiness_snapshot,
            },
            ParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged {
                workspace_directory: workspace_directory.clone(),
                snapshot: Box::new(supervisor_snapshot),
            },
            ParallelModeControlPlanePresentationEvent::PlanningRuntimeRefreshRequested {
                workspace_directory,
            },
            ParallelModeControlPlanePresentationEvent::StatusShown { status_text },
        ];
        events.extend(self.drain_outcome(runtime_outcome));
        events
    }

    fn worker_event_received(
        &mut self,
        event: ParallelModeControlPlaneWorkerEvent,
        has_actionable_queue_head: bool,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        let outcome = self
            .runtime
            .handle(ParallelModeControlPlaneCommand::WorkerEventReceived {
                event,
                has_actionable_queue_head,
            });
        self.drain_outcome(outcome)
    }

    fn orchestrator_tick_completed(
        &mut self,
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        blocked: bool,
        notices: Vec<String>,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        if !self.event_targets_active_workspace(&workspace_directory) {
            let outcome = self
                .runtime
                .handle(ParallelModeControlPlaneCommand::Disable {
                    workspace_directory,
                });
            return self.drain_outcome(outcome);
        }

        let outcome =
            self.runtime
                .handle(ParallelModeControlPlaneCommand::OrchestratorTickCompleted {
                    workspace_directory: workspace_directory.clone(),
                    epoch_id,
                    effect_id,
                    blocked,
                });
        if !outcome_effect_completed(&outcome, effect_id) {
            return self.drain_outcome(outcome);
        }

        let notice_count = notices.len();
        let status_text = if blocked {
            format!("parallel mode: distributor retry blocked / notices: {notice_count}")
        } else {
            format!("parallel mode: distributor retry completed / notices: {notice_count}")
        };
        let mut events: Vec<_> =
            notices
                .into_iter()
                .map(|notice| {
                    ParallelModeControlPlanePresentationEvent::ConversationRuntimeNotice { notice }
                })
                .collect();
        events.push(
            ParallelModeControlPlanePresentationEvent::PlanningRuntimeRefreshRequested {
                workspace_directory,
            },
        );
        events.push(ParallelModeControlPlanePresentationEvent::StatusShown { status_text });
        events.extend(self.drain_outcome(outcome));
        events
    }

    fn drain_outcome(
        &mut self,
        outcome: ParallelModeControlPlaneRuntimeOutcome,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        let mut presentation_events = self.reduce_runtime_events(outcome.events);
        for effect in outcome.effects {
            presentation_events.extend(self.run_effect(effect));
        }
        presentation_events
    }

    fn reduce_runtime_events(
        &mut self,
        events: Vec<ParallelModeControlPlaneEvent>,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        let mut presentation_events = Vec::new();
        for event in events {
            match event {
                ParallelModeControlPlaneEvent::StaleCommandDropped {
                    workspace_directory,
                    epoch_id,
                    reason,
                } => {
                    event_log::emit_lazy("parallel_control_plane_stale_command_dropped", || {
                        serde_json::json!({
                            "workspace": workspace_directory,
                            "epoch_id": epoch_id,
                            "reason": reason,
                        })
                    });
                }
                ParallelModeControlPlaneEvent::EffectStarted { effect_id } => {
                    event_log::emit_lazy("parallel_control_plane_effect_started", || {
                        serde_json::json!({
                            "sequence": effect_id.sequence,
                            "kind": effect_id.kind,
                        })
                    });
                }
                ParallelModeControlPlaneEvent::EffectCompleted { effect_id } => {
                    event_log::emit_lazy("parallel_control_plane_effect_completed", || {
                        serde_json::json!({
                            "sequence": effect_id.sequence,
                            "kind": effect_id.kind,
                        })
                    });
                }
                ParallelModeControlPlaneEvent::DispatchWithheld { trigger, reason } => {
                    self.record_dispatch_withheld(trigger, &reason);
                    presentation_events.push(
                        ParallelModeControlPlanePresentationEvent::StatusShown {
                            status_text: format!("parallel mode: dispatch withheld / {reason}"),
                        },
                    );
                }
                ParallelModeControlPlaneEvent::DispatchCommandQueued {
                    trigger,
                    inserted_count,
                } => {
                    self.last_automation_trigger = Some(trigger);
                    self.last_dispatch_withheld_reason = None;
                    event_log::emit_lazy("parallel_orchestrator_wake_queued", || {
                        serde_json::json!({
                            "trigger": trigger.label(),
                            "workspace": self.runtime.store().workspace_directory.as_deref(),
                            "epoch_id": self.current_epoch_id(),
                            "inserted_count": inserted_count,
                            "reason": "application control-plane effect",
                        })
                    });
                }
                ParallelModeControlPlaneEvent::ConversationRuntimeNotice { notice } => {
                    presentation_events.push(
                        ParallelModeControlPlanePresentationEvent::ConversationRuntimeNotice {
                            notice,
                        },
                    );
                }
                ParallelModeControlPlaneEvent::ModeDisabled {
                    workspace_directory,
                } => {
                    self.readiness_snapshot = None;
                    presentation_events.push(
                        ParallelModeControlPlanePresentationEvent::ModeDisabled {
                            workspace_directory,
                        },
                    );
                }
                _ => {}
            }
        }
        presentation_events
    }

    fn run_effect(
        &mut self,
        effect: ParallelModeControlPlaneEffect,
    ) -> Vec<ParallelModeControlPlanePresentationEvent> {
        match effect {
            ParallelModeControlPlaneEffect::EnterParallelMode {
                effect_id,
                workspace_directory,
                epoch_id,
                mode_was_enabled,
                initial_pool_reset_required,
            } => {
                self.effect_runner.spawn_entry(
                    workspace_directory,
                    epoch_id,
                    effect_id,
                    mode_was_enabled,
                    initial_pool_reset_required,
                );
                Vec::new()
            }
            ParallelModeControlPlaneEffect::RefreshSupervisor {
                effect_id,
                workspace_directory,
                epoch_id,
            } => {
                let Some(readiness_snapshot) = self.readiness_snapshot.clone() else {
                    let outcome = self.runtime.handle(
                        ParallelModeControlPlaneCommand::SupervisorSnapshotRefreshCompleted {
                            workspace_directory,
                            epoch_id,
                            effect_id,
                            follow_up_tick_signature: None,
                        },
                    );
                    return self.drain_outcome(outcome);
                };
                self.effect_runner.spawn_supervisor_snapshot_refresh(
                    workspace_directory,
                    readiness_snapshot,
                    self.mode_enabled(),
                    epoch_id,
                    effect_id,
                );
                Vec::new()
            }
            ParallelModeControlPlaneEffect::RunOrchestrator { effect_id, wake } => {
                if wake.enqueue_trigger.is_some() || self.last_automation_trigger.is_none() {
                    self.last_automation_trigger = Some(wake.trigger);
                }
                self.last_dispatch_withheld_reason = None;
                event_log::emit_lazy("parallel_dispatch_requested", || {
                    serde_json::json!({
                        "trigger": wake.trigger.label(),
                        "workspace": &wake.workspace_directory,
                        "epoch_id": wake.epoch_id,
                        "effect_sequence": effect_id.sequence,
                    })
                });
                self.effect_runner.spawn_orchestrator_wake(
                    wake.workspace_directory,
                    wake.trigger,
                    wake.epoch_id,
                    wake.enqueue_trigger,
                    effect_id,
                );
                Vec::new()
            }
            ParallelModeControlPlaneEffect::RunOrchestratorTick {
                effect_id,
                workspace_directory,
                epoch_id,
                signature,
            } => {
                self.effect_runner.spawn_orchestrator_tick(
                    workspace_directory,
                    signature,
                    epoch_id,
                    effect_id,
                );
                Vec::new()
            }
            ParallelModeControlPlaneEffect::PollPendingDispatchWake {
                workspace_directory,
                epoch_id,
                follow_up_tick_signature,
            } => {
                let (wake, error) = match self
                    .effect_runner
                    .pending_dispatch_wake(&workspace_directory, epoch_id)
                {
                    Ok(wake) => (wake, None),
                    Err(error) => (None, Some(error)),
                };
                let outcome = self.runtime.handle(
                    ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
                        workspace_directory,
                        epoch_id,
                        wake,
                        error,
                        follow_up_tick_signature,
                    },
                );
                self.drain_outcome(outcome)
            }
            ParallelModeControlPlaneEffect::EnqueueSlotCapacityDispatch {
                workspace_directory,
                epoch_id,
            } => match self
                .effect_runner
                .enqueue_slot_capacity_dispatch(&workspace_directory, epoch_id)
            {
                Ok(_) => Vec::new(),
                Err(error) => self.drain_outcome(ParallelModeControlPlaneRuntimeOutcome {
                    events: vec![ParallelModeControlPlaneEvent::DispatchWithheld {
                        trigger: Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
                        reason: format!("slot-capacity dispatch queue failed: {error}"),
                    }],
                    effects: Vec::new(),
                }),
            },
            ParallelModeControlPlaneEffect::EnqueueDispatchForTrigger {
                workspace_directory,
                trigger,
                epoch_id,
                reason,
            } => match self.effect_runner.enqueue_dispatch_for_trigger(
                &workspace_directory,
                trigger,
                epoch_id,
            ) {
                Ok(0) => self.drain_outcome(ParallelModeControlPlaneRuntimeOutcome {
                    events: vec![ParallelModeControlPlaneEvent::DispatchWithheld {
                        trigger: Some(trigger),
                        reason: "orchestrator wake already queued".to_string(),
                    }],
                    effects: Vec::new(),
                }),
                Ok(inserted_count) => {
                    let mut events = self.drain_outcome(ParallelModeControlPlaneRuntimeOutcome {
                        events: vec![ParallelModeControlPlaneEvent::DispatchCommandQueued {
                            trigger,
                            inserted_count,
                        }],
                        effects: Vec::new(),
                    });
                    events.push(ParallelModeControlPlanePresentationEvent::StatusShown {
                        status_text: format!("parallel mode: dispatch deferred / {reason}"),
                    });
                    events
                }
                Err(error) => self.drain_outcome(ParallelModeControlPlaneRuntimeOutcome {
                    events: vec![ParallelModeControlPlaneEvent::DispatchWithheld {
                        trigger: Some(trigger),
                        reason: format!("orchestrator wake queue failed: {error}"),
                    }],
                    effects: Vec::new(),
                }),
            },
            ParallelModeControlPlaneEffect::CancelDispatchCommands {
                workspace_directory,
                reason,
            } => {
                self.effect_runner
                    .cancel_dispatch_commands(&workspace_directory, &reason);
                Vec::new()
            }
        }
    }

    fn event_targets_active_workspace(&self, workspace_directory: &str) -> bool {
        self.mode_enabled()
            && self.runtime.store().workspace_directory.as_deref() == Some(workspace_directory)
    }

    fn record_dispatch_withheld(
        &mut self,
        trigger: Option<ParallelModeAutomationTrigger>,
        reason: &str,
    ) {
        self.last_automation_trigger = trigger;
        self.last_dispatch_withheld_reason = Some(reason.to_string());
        event_log::emit_lazy("parallel_dispatch_blocked", || {
            serde_json::json!({
                "trigger": trigger.map(|value| value.label()),
                "workspace": self.runtime.store().workspace_directory.as_deref(),
                "epoch_id": self.current_epoch_id(),
                "blocked_reason": reason,
            })
        });
    }

    fn record_dispatch_completed(
        &mut self,
        workspace_directory: &str,
        outcome: &ParallelModeDispatchOutcome,
    ) {
        if outcome.trigger != ParallelModeAutomationTrigger::TaskIntakeAfterEpoch
            || self.last_automation_trigger.is_none()
            || self.last_automation_trigger
                == Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch)
        {
            self.last_automation_trigger = Some(outcome.trigger);
        }
        self.last_dispatch_withheld_reason = outcome.blocked_reason.clone();
        event_log::emit_lazy("parallel_dispatch_completed", || {
            serde_json::json!({
                "trigger": outcome.trigger.label(),
                "workspace": workspace_directory,
                "epoch_id": outcome.epoch_id,
                "idle_slot_count": outcome.idle_slot_count,
                "task_ids": &outcome.candidate_task_ids,
                "launched_count": outcome.launched_task_ids.len(),
                "blocked_reason": &outcome.blocked_reason,
            })
        });
    }
}

struct ParallelModeEntryResult {
    workspace_directory: String,
    epoch_id: u64,
    effect_id: ParallelModeControlPlaneEffectId,
    mode_was_enabled: bool,
    readiness_snapshot: ParallelModeReadinessSnapshot,
    supervisor_snapshot: ParallelModeSupervisorSnapshot,
    status_text: String,
    initial_pool_reset_completed: bool,
    has_actionable_queue_head: bool,
    follow_up_tick_signature: Option<String>,
}

fn outcome_effect_completed(
    outcome: &ParallelModeControlPlaneRuntimeOutcome,
    effect_id: ParallelModeControlPlaneEffectId,
) -> bool {
    outcome.events.iter().any(|event| {
        matches!(
            event,
            ParallelModeControlPlaneEvent::EffectCompleted {
                effect_id: completed,
            } if *completed == effect_id
        )
    })
}

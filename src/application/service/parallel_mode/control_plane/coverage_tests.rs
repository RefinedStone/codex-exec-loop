use super::*;

const WORKSPACE: &str = "/repo";

fn open_epoch(runtime: &mut ParallelModeControlPlaneRuntime) {
    runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
        workspace_directory: WORKSPACE.to_string(),
    });
}

fn only_effect_id(
    outcome: &ParallelModeControlPlaneRuntimeOutcome,
) -> ParallelModeControlPlaneEffectId {
    outcome
        .effects
        .first()
        .and_then(ParallelModeControlPlaneEffect::effect_id)
        .expect("outcome should contain one identified effect")
}

fn wake(epoch_id: u64) -> ParallelModeControlPlaneWake {
    ParallelModeControlPlaneWake::new(
        WORKSPACE,
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        epoch_id,
        Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
    )
}

fn worker_event(
    kind: ParallelModeControlPlaneWorkerEventKind,
    notices: Vec<String>,
) -> ParallelModeControlPlaneWorkerEvent {
    ParallelModeControlPlaneWorkerEvent::new(WORKSPACE, 1, "task-1", "Task One", kind, notices)
}

#[test]
fn utility_effect_ids_inspection_and_reset_tick_signature_cover_process_edges() {
    assert_eq!(
        ParallelModeControlPlaneEffect::InspectSupervisor {
            workspace_directory: WORKSPACE.to_string(),
            mode_enabled: false,
            reconcile_pool: false,
            show_status: true,
        }
        .effect_id(),
        None
    );
    assert_eq!(
        ParallelModeControlPlaneEffect::PollPendingDispatchWake {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            follow_up_tick_signature: None,
        }
        .effect_id(),
        None
    );
    assert_eq!(
        ParallelModeControlPlaneEffect::EnqueueSlotCapacityDispatch {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
        }
        .effect_id(),
        None
    );
    assert_eq!(
        ParallelModeControlPlaneEffect::EnqueueDispatchForTrigger {
            workspace_directory: WORKSPACE.to_string(),
            trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
            epoch_id: 1,
            reason: "deferred".to_string(),
        }
        .effect_id(),
        None
    );
    assert_eq!(
        ParallelModeControlPlaneEffect::CancelDispatchCommands {
            workspace_directory: WORKSPACE.to_string(),
            reason: "disabled".to_string(),
        }
        .effect_id(),
        None
    );

    let mut runtime = ParallelModeControlPlaneRuntime::new();
    let inspected = runtime.handle(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: WORKSPACE.to_string(),
        reconcile_pool: true,
        show_status: true,
    });
    assert_eq!(
        inspected.effects,
        vec![ParallelModeControlPlaneEffect::InspectSupervisor {
            workspace_directory: WORKSPACE.to_string(),
            mode_enabled: false,
            reconcile_pool: false,
            show_status: true,
        }]
    );

    open_epoch(&mut runtime);
    let tick_started = runtime.handle(ParallelModeControlPlaneCommand::RunOrchestratorTick {
        workspace_directory: WORKSPACE.to_string(),
        signature: "sig-1".to_string(),
    });
    assert_eq!(
        only_effect_id(&tick_started).kind,
        ParallelModeControlPlaneEffectKind::RunOrchestratorTick
    );
    assert_eq!(
        runtime.store().last_orchestrator_tick_signature.as_deref(),
        Some("sig-1")
    );
    runtime.reset_orchestrator_tick_signature();
    assert!(runtime.store().last_orchestrator_tick_signature.is_none());
}

#[test]
fn stale_workspace_and_epoch_commands_return_without_scheduling_effects() {
    let mut runtime = ParallelModeControlPlaneRuntime::new();
    runtime.force_epoch_for_test(WORKSPACE, 7);

    let disabled = runtime.handle(ParallelModeControlPlaneCommand::Disable {
        workspace_directory: "/other".to_string(),
    });
    assert_eq!(
        disabled.events,
        vec![ParallelModeControlPlaneEvent::StaleCommandDropped {
            workspace_directory: "/other".to_string(),
            epoch_id: 0,
            reason: "disable command targets a different workspace".to_string(),
        }]
    );

    let stale_wake = runtime.handle(ParallelModeControlPlaneCommand::WakeOrchestrator(
        ParallelModeControlPlaneWake::new(
            WORKSPACE,
            ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            99,
            None,
        ),
    ));
    assert!(matches!(
        stale_wake.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { epoch_id: 99, .. }]
    ));
    assert!(stale_wake.effects.is_empty());

    let missing_tick = runtime.handle(ParallelModeControlPlaneCommand::RunOrchestratorTick {
        workspace_directory: "/other".to_string(),
        signature: "sig".to_string(),
    });
    assert!(matches!(
        missing_tick.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped {
            workspace_directory,
            epoch_id: 0,
            ..
        }] if workspace_directory == "/other"
    ));

    let mut closed = ParallelModeControlPlaneRuntime::new();
    let withheld = closed.handle(ParallelModeControlPlaneCommand::RequestDispatch {
        workspace_directory: WORKSPACE.to_string(),
        trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
    });
    assert_eq!(
        withheld.events,
        vec![
            ParallelModeControlPlaneEvent::StaleCommandDropped {
                workspace_directory: WORKSPACE.to_string(),
                epoch_id: 0,
                reason: "parallel automation epoch is not open for workspace".to_string(),
            },
            ParallelModeControlPlaneEvent::DispatchWithheld {
                trigger: Some(ParallelModeAutomationTrigger::MainTurnPostEvaluation),
                reason: "automation epoch is not open".to_string(),
            },
        ]
    );

    let poll_without_epoch =
        closed.handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
            workspace_directory: WORKSPACE.to_string(),
            follow_up_tick_signature: None,
        });
    assert!(matches!(
        poll_without_epoch.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { epoch_id: 0, .. }]
    ));
}

#[test]
fn entry_completion_branches_close_refresh_and_reject_unknown_entries() {
    let mut unknown_runtime = ParallelModeControlPlaneRuntime::new();
    let enabled = unknown_runtime.handle(ParallelModeControlPlaneCommand::Enable {
        workspace_directory: WORKSPACE.to_string(),
    });
    assert_eq!(
        only_effect_id(&enabled).kind,
        ParallelModeControlPlaneEffectKind::EnterParallelMode
    );
    let unknown_entry = unknown_runtime.handle(ParallelModeControlPlaneCommand::EntryCompleted {
        workspace_directory: WORKSPACE.to_string(),
        epoch_id: 1,
        effect_id: ParallelModeControlPlaneEffectId::new(
            999,
            ParallelModeControlPlaneEffectKind::EnterParallelMode,
        ),
        mode_enabled: true,
        mode_was_enabled: false,
        initial_pool_reset_completed: true,
        has_actionable_queue_head: false,
        follow_up_tick_signature: None,
    });
    assert_eq!(
        unknown_entry.events,
        vec![ParallelModeControlPlaneEvent::StaleCommandDropped {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            reason: "unknown parallel entry".to_string(),
        }]
    );

    let mut close_runtime = ParallelModeControlPlaneRuntime::new();
    let close_enabled = close_runtime.handle(ParallelModeControlPlaneCommand::Enable {
        workspace_directory: WORKSPACE.to_string(),
    });
    let close_id = only_effect_id(&close_enabled);
    let closed = close_runtime.handle(ParallelModeControlPlaneCommand::EntryCompleted {
        workspace_directory: WORKSPACE.to_string(),
        epoch_id: 1,
        effect_id: close_id,
        mode_enabled: false,
        mode_was_enabled: false,
        initial_pool_reset_completed: true,
        has_actionable_queue_head: true,
        follow_up_tick_signature: None,
    });
    assert!(matches!(
        closed.events.as_slice(),
        [
            ParallelModeControlPlaneEvent::EffectCompleted { .. },
            ParallelModeControlPlaneEvent::EpochClosed { epoch_id: 1, .. },
            ParallelModeControlPlaneEvent::ModeDisabled { .. }
        ]
    ));
    assert_eq!(close_runtime.store().current_epoch_id, None);

    let mut refresh_runtime = ParallelModeControlPlaneRuntime::new();
    let first_enable = refresh_runtime.handle(ParallelModeControlPlaneCommand::Enable {
        workspace_directory: WORKSPACE.to_string(),
    });
    let first_entry_id = only_effect_id(&first_enable);
    let second_enable = refresh_runtime.handle(ParallelModeControlPlaneCommand::Enable {
        workspace_directory: WORKSPACE.to_string(),
    });
    assert_eq!(
        second_enable.events,
        vec![
            ParallelModeControlPlaneEvent::ModeEnabled {
                workspace_directory: WORKSPACE.to_string(),
                epoch_id: 1,
            },
            ParallelModeControlPlaneEvent::SupervisorRefreshQueued,
        ]
    );
    let refreshed = refresh_runtime.handle(ParallelModeControlPlaneCommand::EntryCompleted {
        workspace_directory: WORKSPACE.to_string(),
        epoch_id: 1,
        effect_id: first_entry_id,
        mode_enabled: true,
        mode_was_enabled: true,
        initial_pool_reset_completed: false,
        has_actionable_queue_head: false,
        follow_up_tick_signature: None,
    });
    assert!(matches!(
        refreshed.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RefreshSupervisor { epoch_id: 1, .. }]
    ));
}

#[test]
fn completion_commands_cover_specific_wake_refresh_tick_and_worker_arms() {
    let mut worker_runtime = ParallelModeControlPlaneRuntime::new();
    worker_runtime.force_mode_for_test(WORKSPACE, true);
    let worker_completed =
        worker_runtime.handle(ParallelModeControlPlaneCommand::WorkerCompleted {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            trigger: ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        });
    assert!(matches!(
        worker_completed.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RunOrchestrator { wake, .. }]
            if wake.trigger == ParallelModeAutomationTrigger::ParallelOfficialCompletion
    ));

    let mut wake_runtime = ParallelModeControlPlaneRuntime::new();
    wake_runtime.force_mode_for_test(WORKSPACE, true);
    let requested = wake_runtime.handle(ParallelModeControlPlaneCommand::RequestDispatch {
        workspace_directory: WORKSPACE.to_string(),
        trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
    });
    let wake_id = only_effect_id(&requested);
    let closed = wake_runtime.handle(ParallelModeControlPlaneCommand::OrchestratorWakeCompleted {
        workspace_directory: WORKSPACE.to_string(),
        epoch_id: 1,
        effect_id: wake_id,
        mode_enabled: false,
        follow_up_tick_signature: Some("ignored".to_string()),
    });
    assert!(matches!(
        closed.events.as_slice(),
        [
            ParallelModeControlPlaneEvent::EffectCompleted { .. },
            ParallelModeControlPlaneEvent::EpochClosed { .. },
            ParallelModeControlPlaneEvent::ModeDisabled { .. }
        ]
    ));

    let mut refresh_runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut refresh_runtime);
    let refresh = refresh_runtime.handle(ParallelModeControlPlaneCommand::RefreshSupervisor {
        workspace_directory: WORKSPACE.to_string(),
    });
    let wrong_kind = refresh_runtime.handle(
        ParallelModeControlPlaneCommand::SupervisorSnapshotRefreshCompleted {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: ParallelModeControlPlaneEffectId::new(
                77,
                ParallelModeControlPlaneEffectKind::RunOrchestrator,
            ),
            follow_up_tick_signature: None,
        },
    );
    assert!(matches!(
        wrong_kind.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { reason, .. }]
            if reason == "unknown supervisor refresh"
    ));
    let refresh_id = only_effect_id(&refresh);
    let refreshed = refresh_runtime.handle(
        ParallelModeControlPlaneCommand::SupervisorSnapshotRefreshCompleted {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: refresh_id,
            follow_up_tick_signature: None,
        },
    );
    assert!(matches!(
        refreshed.effects.as_slice(),
        [ParallelModeControlPlaneEffect::PollPendingDispatchWake { epoch_id: 1, .. }]
    ));

    let mut tick_runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut tick_runtime);
    let tick = tick_runtime.handle(ParallelModeControlPlaneCommand::RunOrchestratorTick {
        workspace_directory: WORKSPACE.to_string(),
        signature: "tick".to_string(),
    });
    let tick_id = only_effect_id(&tick);
    let tick_completed =
        tick_runtime.handle(ParallelModeControlPlaneCommand::OrchestratorTickCompleted {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: tick_id,
            blocked: false,
        });
    assert!(matches!(
        tick_completed.effects.as_slice(),
        [
            ParallelModeControlPlaneEffect::RefreshSupervisor { epoch_id: 1, .. },
            ParallelModeControlPlaneEffect::EnqueueSlotCapacityDispatch { epoch_id: 1, .. }
        ]
    ));

    let stale_tick =
        tick_runtime.handle(ParallelModeControlPlaneCommand::OrchestratorTickCompleted {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: tick_id,
            blocked: true,
        });
    assert!(matches!(
        stale_tick.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { reason, .. }]
            if reason == "unknown orchestrator tick"
    ));
}

#[test]
fn pending_dispatch_poll_and_request_edges_cover_ready_stale_busy_and_error_paths() {
    let mut ready_runtime = ParallelModeControlPlaneRuntime::new();
    ready_runtime.force_mode_for_test(WORKSPACE, true);
    let ready = ready_runtime.handle(ParallelModeControlPlaneCommand::RequestDispatch {
        workspace_directory: WORKSPACE.to_string(),
        trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
    });
    assert!(matches!(
        ready.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RunOrchestrator { wake, .. }]
            if wake.trigger == ParallelModeAutomationTrigger::MainTurnPostEvaluation
    ));

    let mut stale_runtime = ParallelModeControlPlaneRuntime::new();
    stale_runtime.force_mode_for_test(WORKSPACE, true);
    let stale_epoch =
        stale_runtime.handle(ParallelModeControlPlaneCommand::RequestDispatchForEpoch {
            workspace_directory: WORKSPACE.to_string(),
            trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
            epoch_id: 99,
        });
    assert!(matches!(
        stale_epoch.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { epoch_id: 99, .. }]
    ));

    let mut busy_runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut busy_runtime);
    let refresh = busy_runtime.handle(ParallelModeControlPlaneCommand::RefreshSupervisor {
        workspace_directory: WORKSPACE.to_string(),
    });
    let busy_poll = busy_runtime.handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
        workspace_directory: WORKSPACE.to_string(),
        follow_up_tick_signature: Some("tick".to_string()),
    });
    assert!(busy_poll.effects.is_empty());
    assert!(busy_poll.events.is_empty());
    let refresh_id = only_effect_id(&refresh);
    let polled_after_refresh = busy_runtime.handle(
        ParallelModeControlPlaneCommand::SupervisorSnapshotRefreshCompleted {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: refresh_id,
            follow_up_tick_signature: Some("tick".to_string()),
        },
    );
    assert!(matches!(
        polled_after_refresh.effects.as_slice(),
        [ParallelModeControlPlaneEffect::PollPendingDispatchWake {
            follow_up_tick_signature: Some(signature),
            ..
        }] if signature == "tick"
    ));

    let mut polled_runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut polled_runtime);
    let error = polled_runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
        workspace_directory: WORKSPACE.to_string(),
        epoch_id: 1,
        wake: None,
        error: Some("sqlite busy".to_string()),
        follow_up_tick_signature: None,
    });
    assert_eq!(
        error.events,
        vec![ParallelModeControlPlaneEvent::DispatchWithheld {
            trigger: Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
            reason: "pending dispatch command poll failed: sqlite busy".to_string(),
        }]
    );

    let wake_polled =
        polled_runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            wake: Some(wake(1)),
            error: None,
            follow_up_tick_signature: Some("unused".to_string()),
        });
    assert!(matches!(
        wake_polled.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RunOrchestrator { .. }]
    ));

    let stale_polled =
        polled_runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 99,
            wake: None,
            error: None,
            follow_up_tick_signature: None,
        });
    assert!(matches!(
        stale_polled.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { epoch_id: 99, .. }]
    ));
}

#[test]
fn projection_ready_and_effect_completion_drain_or_refresh_pending_work() {
    let mut drain_runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut drain_runtime);
    let refresh = drain_runtime.handle(ParallelModeControlPlaneCommand::RefreshSupervisor {
        workspace_directory: WORKSPACE.to_string(),
    });
    let refresh_id = only_effect_id(&refresh);
    let queued = drain_runtime.handle(ParallelModeControlPlaneCommand::WakeOrchestrator(wake(1)));
    assert!(matches!(
        queued.events.as_slice(),
        [ParallelModeControlPlaneEvent::OrchestratorWakeQueued { epoch_id: 1, .. }]
    ));
    let drained = drain_runtime.handle(
        ParallelModeControlPlaneCommand::SupervisorSnapshotRefreshCompleted {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: refresh_id,
            follow_up_tick_signature: Some("ignored".to_string()),
        },
    );
    assert!(matches!(
        drained.events.as_slice(),
        [
            ParallelModeControlPlaneEvent::EffectCompleted { .. },
            ParallelModeControlPlaneEvent::OrchestratorWakeDequeued { .. },
            ParallelModeControlPlaneEvent::EffectStarted { .. }
        ]
    ));

    let mut stale_wake_runtime = ParallelModeControlPlaneRuntime::new();
    stale_wake_runtime.force_epoch_for_test(WORKSPACE, 1);
    stale_wake_runtime.store.pending_orchestrator_wake = Some(ParallelModeControlPlaneWake::new(
        WORKSPACE,
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        2,
        None,
    ));
    let mut stale_outcome = ParallelModeControlPlaneRuntimeOutcome::new();
    assert!(!stale_wake_runtime.drain_pending_orchestrator_wake(&mut stale_outcome));
    assert!(matches!(
        stale_outcome.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { epoch_id: 2, .. }]
    ));

    let mut busy_drain_runtime = ParallelModeControlPlaneRuntime::new();
    let busy_id = busy_drain_runtime.force_supervisor_refresh_in_flight_for_test(WORKSPACE, 1);
    busy_drain_runtime.store.pending_orchestrator_wake = Some(wake(1));
    let mut busy_outcome = ParallelModeControlPlaneRuntimeOutcome::new();
    assert!(!busy_drain_runtime.drain_pending_orchestrator_wake(&mut busy_outcome));
    assert!(busy_drain_runtime.store.pending_orchestrator_wake.is_some());
    let after_busy = busy_drain_runtime.handle(ParallelModeControlPlaneCommand::EffectCompleted {
        workspace_directory: WORKSPACE.to_string(),
        epoch_id: 1,
        effect_id: busy_id,
    });
    assert!(matches!(
        after_busy.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RunOrchestrator { .. }]
    ));

    let mut no_pending_runtime = ParallelModeControlPlaneRuntime::new();
    no_pending_runtime.force_epoch_for_test(WORKSPACE, 1);
    let mut no_pending_outcome = ParallelModeControlPlaneRuntimeOutcome::new();
    assert!(!no_pending_runtime.drain_pending_orchestrator_wake(&mut no_pending_outcome));
    assert!(no_pending_outcome.events.is_empty());

    let mut busy_schedule_runtime = ParallelModeControlPlaneRuntime::new();
    busy_schedule_runtime.force_supervisor_refresh_in_flight_for_test(WORKSPACE, 1);
    busy_schedule_runtime.store.pending_orchestrator_wake = Some(wake(1));
    let mut busy_schedule_outcome = ParallelModeControlPlaneRuntimeOutcome::new();
    busy_schedule_runtime.schedule_after_projection_ready(
        WORKSPACE.to_string(),
        1,
        Some("tick-after-busy".to_string()),
        &mut busy_schedule_outcome,
    );
    assert!(matches!(
        busy_schedule_outcome.effects.as_slice(),
        [ParallelModeControlPlaneEffect::PollPendingDispatchWake {
            follow_up_tick_signature: Some(signature),
            ..
        }] if signature == "tick-after-busy"
    ));

    let mut refresh_follow_up_runtime = ParallelModeControlPlaneRuntime::new();
    refresh_follow_up_runtime.force_mode_for_test(WORKSPACE, true);
    let wake_started =
        refresh_follow_up_runtime.handle(ParallelModeControlPlaneCommand::RequestDispatch {
            workspace_directory: WORKSPACE.to_string(),
            trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
        });
    let wake_id = only_effect_id(&wake_started);
    refresh_follow_up_runtime.store.pending_supervisor_refresh = true;
    let follow_up =
        refresh_follow_up_runtime.handle(ParallelModeControlPlaneCommand::EffectCompleted {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: wake_id,
        });
    assert!(matches!(
        follow_up.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RefreshSupervisor { epoch_id: 1, .. }]
    ));
}

#[test]
fn worker_event_received_maps_notices_stream_failures_refreshes_and_wakes() {
    let mut runtime = ParallelModeControlPlaneRuntime::new();
    runtime.force_mode_for_test(WORKSPACE, true);

    let completed = runtime.handle(ParallelModeControlPlaneCommand::WorkerEventReceived {
        event: worker_event(
            ParallelModeControlPlaneWorkerEventKind::Completed,
            vec!["official completion refreshed".to_string()],
        ),
        has_actionable_queue_head: true,
    });
    assert!(matches!(
        completed.events.as_slice(),
        [
            ParallelModeControlPlaneEvent::WorkerCompleted { .. },
            ParallelModeControlPlaneEvent::ConversationRuntimeNotice { notice },
            ParallelModeControlPlaneEvent::EffectStarted { .. },
            ParallelModeControlPlaneEvent::OrchestratorWakeQueued { .. }
        ] if notice == "official completion refreshed"
    ));
    assert!(matches!(
        completed.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RefreshSupervisor { .. }]
    ));

    let mut stream_runtime = ParallelModeControlPlaneRuntime::new();
    stream_runtime.force_mode_for_test(WORKSPACE, true);
    let stream_failed =
        stream_runtime.handle(ParallelModeControlPlaneCommand::WorkerEventReceived {
            event: worker_event(
                ParallelModeControlPlaneWorkerEventKind::StreamFailed,
                vec!["stream failed".to_string()],
            ),
            has_actionable_queue_head: true,
        });
    assert!(matches!(
        stream_failed.events.as_slice(),
        [
            ParallelModeControlPlaneEvent::WorkerStreamFailed { task_id, .. },
            ParallelModeControlPlaneEvent::ConversationRuntimeNotice { notice },
            ParallelModeControlPlaneEvent::EffectStarted { .. }
        ] if task_id == "task-1" && notice == "stream failed"
    ));
    assert!(matches!(
        worker_event_to_control_plane_event(&worker_event(
            ParallelModeControlPlaneWorkerEventKind::StreamFailed,
            Vec::new(),
        )),
        ParallelModeControlPlaneEvent::WorkerStreamFailed { .. }
    ));
}

#[test]
fn unknown_effect_completion_reasons_cover_refresh_and_specific_kind_mismatches() {
    let mut generic_runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut generic_runtime);
    let unknown_refresh =
        generic_runtime.handle(ParallelModeControlPlaneCommand::EffectCompleted {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: ParallelModeControlPlaneEffectId::new(
                42,
                ParallelModeControlPlaneEffectKind::RefreshSupervisor,
            ),
        });
    assert_eq!(
        unknown_refresh.events,
        vec![ParallelModeControlPlaneEvent::StaleCommandDropped {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            reason: "unknown supervisor refresh".to_string(),
        }]
    );

    let mut wake_runtime = ParallelModeControlPlaneRuntime::new();
    wake_runtime.force_mode_for_test(WORKSPACE, true);
    let started = wake_runtime.handle(ParallelModeControlPlaneCommand::RequestDispatch {
        workspace_directory: WORKSPACE.to_string(),
        trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
    });
    assert_eq!(
        only_effect_id(&started).kind,
        ParallelModeControlPlaneEffectKind::RunOrchestrator
    );
    let wrong_kind =
        wake_runtime.handle(ParallelModeControlPlaneCommand::OrchestratorWakeCompleted {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: ParallelModeControlPlaneEffectId::new(
                1,
                ParallelModeControlPlaneEffectKind::RefreshSupervisor,
            ),
            mode_enabled: true,
            follow_up_tick_signature: None,
        });
    assert!(matches!(
        wrong_kind.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { reason, .. }]
            if reason == "unknown orchestrator wake"
    ));
}

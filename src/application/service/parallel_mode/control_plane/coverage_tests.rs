use super::*;

use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
};
use crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort;
use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::PlanningServices;
use crate::diagnostics::trace_event_log::AKRA_EVENT_TARGET;
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModeDispatchOutcome, ParallelModeReadinessSnapshot, ParallelModeReadinessState,
    ParallelModeSupervisorSnapshot,
};
use std::sync::{Arc, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

const WORKSPACE: &str = "/repo";

#[derive(Clone)]
struct CapturingControlPlaneEventSink {
    tx: mpsc::Sender<ParallelModeControlPlaneBackgroundEvent>,
}

impl ParallelModeControlPlaneEventSink for CapturingControlPlaneEventSink {
    fn send_control_plane_event(&self, event: ParallelModeControlPlaneBackgroundEvent) {
        let _ = self.tx.send(event);
    }
}

struct ReadyGithubAutomationPort;

impl GithubAutomationPort for ReadyGithubAutomationPort {
    fn inspect_capabilities(&self, _repo_root: &str) -> GithubAutomationCapabilities {
        GithubAutomationCapabilities::new(
            ready_capability(ParallelModeCapabilityKey::PushRemote),
            ready_capability(ParallelModeCapabilityKey::GhBinary),
            ready_capability(ParallelModeCapabilityKey::GhAuth),
        )
    }

    fn push_branch(
        &self,
        _repo_root: &str,
        _branch_name: &str,
        _force_with_lease: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn ensure_pull_request(
        &self,
        _repo_root: &str,
        base_branch: &str,
        head_branch: &str,
        _title: &str,
        _body: &str,
    ) -> anyhow::Result<GithubAutomationPullRequest> {
        Ok(GithubAutomationPullRequest::new(
            1,
            "https://github.example/pr/1",
            "open",
            base_branch,
            head_branch,
            false,
        ))
    }

    fn inspect_pull_request(
        &self,
        _repo_root: &str,
        pr_number: u64,
    ) -> anyhow::Result<GithubAutomationPullRequest> {
        Ok(GithubAutomationPullRequest::new(
            pr_number,
            "https://github.example/pr/1",
            "open",
            "prerelease",
            "akra-agent/slot-1/task",
            false,
        ))
    }

    fn push_integration_branch(&self, _repo_root: &str, _branch_name: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn close_pull_request(&self, _repo_root: &str, _pr_number: u64) -> anyhow::Result<()> {
        Ok(())
    }
}

fn ready_capability(key: ParallelModeCapabilityKey) -> ParallelModeCapabilitySnapshot {
    ParallelModeCapabilitySnapshot::new(key, ParallelModeCapabilityState::Ready, "ready", None)
}

fn unique_workspace(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    format!("/tmp/akra-control-plane-coverage-{label}-{nanos}")
}

fn test_parallel_mode_service(
    authority: Arc<SqlitePlanningAuthorityAdapter>,
) -> ParallelModeService {
    ParallelModeService::new(
        authority,
        Arc::new(ReadyGithubAutomationPort),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    )
}

fn test_planning_services(authority: Arc<SqlitePlanningAuthorityAdapter>) -> PlanningServices {
    PlanningServices::from_ports(
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        authority.clone(),
        authority,
        Arc::new(NoopPlanningWorkerPort),
    )
}

fn test_control_plane_handle() -> (
    ParallelModeControlPlaneHandle<CapturingControlPlaneEventSink>,
    mpsc::Receiver<ParallelModeControlPlaneBackgroundEvent>,
) {
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let parallel_mode_service = test_parallel_mode_service(authority.clone());
    let planning = test_planning_services(authority);
    let (tx, rx) = mpsc::channel();
    let effect_runner = ParallelModeControlPlaneEffectRunner::new(
        parallel_mode_service.clone(),
        planning,
        Arc::new(NoopParallelAgentWorkerPort),
        ParallelModeTurnService::new(parallel_mode_service),
        CapturingControlPlaneEventSink { tx },
    );
    let service = super::controller::ParallelModeControlPlaneService::new(effect_runner);
    (ParallelModeControlPlaneHandle::new(service), rx)
}

fn ready_readiness(workspace_directory: &str) -> ParallelModeReadinessSnapshot {
    ParallelModeReadinessSnapshot::new(
        workspace_directory,
        ParallelModeReadinessState::Ready,
        Vec::new(),
        None,
    )
}

fn supervisor_snapshot(workspace_directory: &str) -> ParallelModeSupervisorSnapshot {
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let service = test_parallel_mode_service(authority);
    let readiness = ready_readiness(workspace_directory);
    service.build_supervisor_snapshot(workspace_directory, true, Some(&readiness))
}

fn recv_background_event(
    rx: &mpsc::Receiver<ParallelModeControlPlaneBackgroundEvent>,
) -> ParallelModeControlPlaneBackgroundEvent {
    rx.recv_timeout(Duration::from_secs(5))
        .expect("control plane background event should be sent")
}

fn recv_orchestrator_wake_completed(
    rx: &mpsc::Receiver<ParallelModeControlPlaneBackgroundEvent>,
) -> ParallelModeControlPlaneBackgroundEvent {
    for _ in 0..8 {
        let event = recv_background_event(rx);
        if matches!(
            event,
            ParallelModeControlPlaneBackgroundEvent::OrchestratorWakeCompleted { .. }
        ) {
            return event;
        }
    }
    panic!("orchestrator wake completion should be sent");
}

fn recv_entered_event(
    rx: &mpsc::Receiver<ParallelModeControlPlaneBackgroundEvent>,
) -> ParallelModeControlPlaneBackgroundEvent {
    for _ in 0..8 {
        let event = recv_background_event(rx);
        if matches!(
            event,
            ParallelModeControlPlaneBackgroundEvent::Entered { .. }
        ) {
            return event;
        }
    }
    panic!("parallel entry completion should be sent");
}

fn with_akra_event_trace<T>(body: impl FnOnce() -> T) -> T {
    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::new(format!("{AKRA_EVENT_TARGET}=debug")))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::sink));
    tracing::subscriber::with_default(subscriber, body)
}

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

#[test]
fn controller_background_events_map_direct_notices_and_ignore_stale_completions() {
    let (handle, _rx) = test_control_plane_handle();
    handle.force_mode_for_test(WORKSPACE, true);

    let notice = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::ConversationRuntimeNotice(
            "runtime notice".to_string(),
        ),
    );
    assert!(matches!(
        notice.as_slice(),
        [ParallelModeControlPlanePresentationEvent::ConversationRuntimeNotice { notice }]
            if notice == "runtime notice"
    ));

    let inactive_progress =
        handle.handle_background_event(ParallelModeControlPlaneBackgroundEvent::EnterProgress {
            workspace_directory: "/other".to_string(),
            readiness_snapshot: Some(ready_readiness("/other")),
            loading_stage: ParallelModeControlPlaneLoadingStage::ReconcilingPool,
            status_text: "ignored".to_string(),
        });
    assert!(inactive_progress.is_empty());

    let active_progress =
        handle.handle_background_event(ParallelModeControlPlaneBackgroundEvent::EnterProgress {
            workspace_directory: WORKSPACE.to_string(),
            readiness_snapshot: None,
            loading_stage: ParallelModeControlPlaneLoadingStage::ReconcilingPool,
            status_text: "working".to_string(),
        });
    assert!(matches!(
        active_progress.as_slice(),
        [ParallelModeControlPlanePresentationEvent::EnterProgress {
            readiness_snapshot: None,
            status_text,
            ..
        }] if status_text == "working"
    ));

    let unknown_entry =
        handle.handle_background_event(ParallelModeControlPlaneBackgroundEvent::Entered {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: ParallelModeControlPlaneEffectId::new(
                900,
                ParallelModeControlPlaneEffectKind::EnterParallelMode,
            ),
            mode_was_enabled: false,
            readiness_snapshot: ready_readiness(WORKSPACE),
            supervisor_snapshot: Box::new(supervisor_snapshot(WORKSPACE)),
            status_text: "entered".to_string(),
            initial_pool_reset_completed: true,
            has_actionable_queue_head: false,
            orchestrator_tick_signature: None,
        });
    assert!(unknown_entry.is_empty());

    let stale_refresh = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::SupervisorSnapshotRefreshed {
            workspace_directory: "/other".to_string(),
            epoch_id: 1,
            effect_id: ParallelModeControlPlaneEffectId::new(
                901,
                ParallelModeControlPlaneEffectKind::RefreshSupervisor,
            ),
            supervisor_snapshot: Box::new(supervisor_snapshot("/other")),
            orchestrator_tick_signature: None,
        },
    );
    assert!(stale_refresh.is_empty());

    let unknown_refresh = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::SupervisorSnapshotRefreshed {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: ParallelModeControlPlaneEffectId::new(
                902,
                ParallelModeControlPlaneEffectKind::RefreshSupervisor,
            ),
            supervisor_snapshot: Box::new(supervisor_snapshot(WORKSPACE)),
            orchestrator_tick_signature: None,
        },
    );
    assert!(unknown_refresh.is_empty());

    let stale_wake = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::OrchestratorWakeCompleted {
            workspace_directory: "/other".to_string(),
            effect_id: ParallelModeControlPlaneEffectId::new(
                903,
                ParallelModeControlPlaneEffectKind::RunOrchestrator,
            ),
            readiness_snapshot: ready_readiness("/other"),
            supervisor_snapshot: Box::new(supervisor_snapshot("/other")),
            outcome: ParallelModeDispatchOutcome::new(
                ParallelModeAutomationTrigger::MainTurnPostEvaluation,
                "/other",
                1,
            ),
            orchestrator_tick_signature: None,
        },
    );
    assert!(stale_wake.is_empty());

    let unknown_wake = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::OrchestratorWakeCompleted {
            workspace_directory: WORKSPACE.to_string(),
            effect_id: ParallelModeControlPlaneEffectId::new(
                904,
                ParallelModeControlPlaneEffectKind::RunOrchestrator,
            ),
            readiness_snapshot: ready_readiness(WORKSPACE),
            supervisor_snapshot: Box::new(supervisor_snapshot(WORKSPACE)),
            outcome: ParallelModeDispatchOutcome::new(
                ParallelModeAutomationTrigger::MainTurnPostEvaluation,
                WORKSPACE,
                1,
            ),
            orchestrator_tick_signature: None,
        },
    );
    assert!(unknown_wake.is_empty());
}

#[test]
fn controller_orchestrator_tick_completion_covers_retry_status_paths() {
    let (unblocked_handle, unblocked_rx) = test_control_plane_handle();
    unblocked_handle.force_epoch_for_test(WORKSPACE, 1);
    let started =
        unblocked_handle.handle_command(ParallelModeControlPlaneCommand::RunOrchestratorTick {
            workspace_directory: WORKSPACE.to_string(),
            signature: "retry-1".to_string(),
        });
    assert!(started.is_empty());
    let tick_event = recv_background_event(&unblocked_rx);
    let (workspace_directory, epoch_id, effect_id) = match tick_event {
        ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
            workspace_directory,
            epoch_id,
            effect_id,
            ..
        } => (workspace_directory, epoch_id, effect_id),
        other => panic!("expected orchestrator tick completion, got {other:?}"),
    };
    let completed = unblocked_handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
            workspace_directory,
            epoch_id,
            effect_id,
            blocked: false,
            notices: vec!["retry completed".to_string()],
        },
    );
    assert!(completed.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::ConversationRuntimeNotice { notice }
            if notice == "retry completed"
    )));
    assert!(completed.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::PlanningRuntimeRefreshRequested {
            workspace_directory
        } if workspace_directory == WORKSPACE
    )));
    assert!(completed.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text }
            if status_text == "parallel mode: distributor retry completed / notices: 1"
    )));

    let stale_tick = unblocked_handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
            workspace_directory: "/other".to_string(),
            epoch_id: 1,
            effect_id: ParallelModeControlPlaneEffectId::new(
                905,
                ParallelModeControlPlaneEffectKind::RunOrchestratorTick,
            ),
            blocked: false,
            notices: Vec::new(),
        },
    );
    assert!(stale_tick.is_empty());

    let unknown_tick = unblocked_handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: ParallelModeControlPlaneEffectId::new(
                906,
                ParallelModeControlPlaneEffectKind::RunOrchestratorTick,
            ),
            blocked: false,
            notices: Vec::new(),
        },
    );
    assert!(unknown_tick.is_empty());

    let (blocked_handle, blocked_rx) = test_control_plane_handle();
    blocked_handle.force_epoch_for_test(WORKSPACE, 7);
    let started =
        blocked_handle.handle_command(ParallelModeControlPlaneCommand::RunOrchestratorTick {
            workspace_directory: WORKSPACE.to_string(),
            signature: "retry-2".to_string(),
        });
    assert!(started.is_empty());
    let tick_event = recv_background_event(&blocked_rx);
    let (workspace_directory, epoch_id, effect_id) = match tick_event {
        ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
            workspace_directory,
            epoch_id,
            effect_id,
            ..
        } => (workspace_directory, epoch_id, effect_id),
        other => panic!("expected orchestrator tick completion, got {other:?}"),
    };
    let blocked = blocked_handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
            workspace_directory,
            epoch_id,
            effect_id,
            blocked: true,
            notices: vec!["retry blocked".to_string()],
        },
    );
    assert!(blocked.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text }
            if status_text == "parallel mode: distributor retry blocked / notices: 1"
    )));
}

#[test]
fn controller_dispatch_wake_completion_records_traceable_dispatch_state() {
    let (handle, rx) = test_control_plane_handle();
    let workspace = unique_workspace("dispatch-wake");
    handle.force_mode_for_test(&workspace, true);

    let started = with_akra_event_trace(|| {
        handle.handle_command(ParallelModeControlPlaneCommand::RequestDispatch {
            workspace_directory: workspace.clone(),
            trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
        })
    });
    assert!(started.is_empty());

    let wake_completed = recv_orchestrator_wake_completed(&rx);
    let presented = with_akra_event_trace(|| handle.handle_background_event(wake_completed));
    assert!(presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::ReadinessSnapshotChanged { .. }
    )));
    assert!(presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged { .. }
    )));
    assert!(presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::PlanningRuntimeRefreshRequested {
            workspace_directory
        } if workspace_directory == &workspace
    )));
    assert!(presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text }
            if status_text.starts_with("parallel mode: dispatch refreshed / trigger: ")
    )));
    assert_eq!(
        handle.last_automation_trigger(),
        Some(ParallelModeAutomationTrigger::MainTurnPostEvaluation)
    );
}

#[test]
fn controller_refresh_supervisor_uses_cached_readiness_and_applies_refreshed_snapshot() {
    let (handle, rx) = test_control_plane_handle();
    handle.force_mode_for_test(WORKSPACE, true);
    let progress =
        handle.handle_background_event(ParallelModeControlPlaneBackgroundEvent::EnterProgress {
            workspace_directory: WORKSPACE.to_string(),
            readiness_snapshot: Some(ready_readiness(WORKSPACE)),
            loading_stage: ParallelModeControlPlaneLoadingStage::ReconcilingPool,
            status_text: "warming up".to_string(),
        });
    assert!(matches!(
        progress.as_slice(),
        [ParallelModeControlPlanePresentationEvent::EnterProgress { .. }]
    ));

    let started = handle.handle_command(ParallelModeControlPlaneCommand::RefreshSupervisor {
        workspace_directory: WORKSPACE.to_string(),
    });
    assert!(started.is_empty());

    let refreshed = recv_background_event(&rx);
    assert!(matches!(
        refreshed,
        ParallelModeControlPlaneBackgroundEvent::SupervisorSnapshotRefreshed { .. }
    ));
    let presented = handle.handle_background_event(refreshed);
    assert!(presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged { .. }
    )));
}

#[test]
fn controller_pending_dispatch_poll_runs_follow_up_tick_when_queue_is_empty() {
    let (handle, rx) = test_control_plane_handle();
    let workspace = unique_workspace("pending-poll");
    handle.force_epoch_for_test(&workspace, 1);

    let started = handle.handle_command(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
        workspace_directory: workspace,
        follow_up_tick_signature: Some("empty-queue-tick".to_string()),
    });
    assert!(started.is_empty());

    let tick_event = recv_background_event(&rx);
    let (workspace_directory, epoch_id, effect_id) = match tick_event {
        ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
            workspace_directory,
            epoch_id,
            effect_id,
            ..
        } => (workspace_directory, epoch_id, effect_id),
        other => panic!("expected follow-up orchestrator tick completion, got {other:?}"),
    };
    let completed = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
            workspace_directory,
            epoch_id,
            effect_id,
            blocked: false,
            notices: Vec::new(),
        },
    );
    assert!(completed.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text }
            if status_text == "parallel mode: distributor retry completed / notices: 0"
    )));
}

#[test]
fn controller_deferred_dispatch_without_projection_records_traceable_queue_state() {
    let (handle, _rx) = test_control_plane_handle();
    let workspace = unique_workspace("deferred-dispatch");
    handle.force_epoch_for_test(&workspace, 1);

    let presented = with_akra_event_trace(|| {
        handle.handle_command(ParallelModeControlPlaneCommand::RequestDispatch {
            workspace_directory: workspace,
            trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
        })
    });
    assert!(presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text }
            if status_text
                == "parallel mode: dispatch deferred / entry loading or control-plane refresh is still in progress"
    )));
    assert_eq!(handle.last_dispatch_withheld_reason().as_deref(), None);
    assert_eq!(
        handle.last_automation_trigger(),
        Some(ParallelModeAutomationTrigger::MainTurnPostEvaluation)
    );
}

#[test]
fn effect_runner_spawns_traceable_refresh_tick_and_blocked_entry_events() {
    let (refresh_handle, refresh_rx) = test_control_plane_handle();
    refresh_handle.force_mode_for_test(WORKSPACE, true);
    let progress = refresh_handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::EnterProgress {
            workspace_directory: WORKSPACE.to_string(),
            readiness_snapshot: Some(ready_readiness(WORKSPACE)),
            loading_stage: ParallelModeControlPlaneLoadingStage::ReconcilingPool,
            status_text: "cached readiness".to_string(),
        },
    );
    assert!(matches!(
        progress.as_slice(),
        [ParallelModeControlPlanePresentationEvent::EnterProgress { .. }]
    ));
    assert!(
        refresh_handle
            .handle_command(ParallelModeControlPlaneCommand::RefreshSupervisor {
                workspace_directory: WORKSPACE.to_string(),
            })
            .is_empty()
    );
    assert!(matches!(
        recv_background_event(&refresh_rx),
        ParallelModeControlPlaneBackgroundEvent::SupervisorSnapshotRefreshed { .. }
    ));

    let (tick_handle, tick_rx) = test_control_plane_handle();
    let tick_workspace = unique_workspace("trace-tick");
    tick_handle.force_epoch_for_test(&tick_workspace, 1);
    assert!(
        tick_handle
            .handle_command(ParallelModeControlPlaneCommand::RunOrchestratorTick {
                workspace_directory: tick_workspace.clone(),
                signature: "traceable-tick".to_string(),
            })
            .is_empty()
    );
    assert!(matches!(
        recv_background_event(&tick_rx),
        ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
            workspace_directory,
            ..
        } if workspace_directory == tick_workspace
    ));

    let (entry_handle, entry_rx) = test_control_plane_handle();
    let entry_workspace = unique_workspace("blocked-entry");
    assert!(
        entry_handle
            .handle_command(ParallelModeControlPlaneCommand::Enable {
                workspace_directory: entry_workspace.clone(),
            })
            .is_empty()
    );
    let entered = recv_entered_event(&entry_rx);
    assert!(matches!(
        entered,
        ParallelModeControlPlaneBackgroundEvent::Entered {
            workspace_directory,
            readiness_snapshot,
            status_text,
            initial_pool_reset_completed: false,
            ..
        } if workspace_directory == entry_workspace
            && !readiness_snapshot.allows_parallel_mode()
            && status_text.starts_with("parallel mode: blocked / readiness:")
    ));
}

#[test]
fn controller_inspect_supervisor_reconciles_pool_when_requested() {
    let (handle, _rx) = test_control_plane_handle();
    let workspace = unique_workspace("inspect-reconcile");
    handle.force_mode_for_test(&workspace, true);

    let presented = handle.handle_command(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: workspace.clone(),
        reconcile_pool: true,
        show_status: true,
    });

    assert!(presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::ReadinessSnapshotChanged {
            workspace_directory,
            ..
        } if workspace_directory == &workspace
    )));
    assert!(presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged {
            workspace_directory,
            ..
        } if workspace_directory == &workspace
    )));
    assert!(presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text }
            if status_text.starts_with("parallel readiness refreshed / state:")
    )));
}

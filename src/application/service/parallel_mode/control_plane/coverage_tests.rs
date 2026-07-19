use super::*;

use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
};
use crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort;
use crate::application::port::outbound::planning_authority_port::{
    NoopPlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
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
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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

    fn remote_branch_names_for_prefix_for_delivery_target(
        &self,
        _repo_root: &str,
        _push_remote: &str,
        _credential_redacted_push_url: &str,
        _branch_prefix: &str,
    ) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
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

    fn push_integration_branch(
        &self,
        _repo_root: &str,
        _branch_name: &str,
        _expected_old_commit_sha: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn close_pull_request(&self, _repo_root: &str, _pr_number: u64) -> anyhow::Result<()> {
        Ok(())
    }
}

struct GatedGithubAutomationPort {
    entered_tx: mpsc::Sender<()>,
    release_rx: Mutex<mpsc::Receiver<()>>,
}

impl GithubAutomationPort for GatedGithubAutomationPort {
    fn inspect_capabilities(&self, repo_root: &str) -> GithubAutomationCapabilities {
        let _ = self.entered_tx.send(());
        let _ = self
            .release_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recv_timeout(Duration::from_secs(2));
        ReadyGithubAutomationPort.inspect_capabilities(repo_root)
    }

    fn remote_branch_names_for_prefix_for_delivery_target(
        &self,
        repo_root: &str,
        push_remote: &str,
        credential_redacted_push_url: &str,
        branch_prefix: &str,
    ) -> anyhow::Result<Vec<String>> {
        ReadyGithubAutomationPort.remote_branch_names_for_prefix_for_delivery_target(
            repo_root,
            push_remote,
            credential_redacted_push_url,
            branch_prefix,
        )
    }

    fn push_branch(
        &self,
        repo_root: &str,
        branch_name: &str,
        force_with_lease: bool,
    ) -> anyhow::Result<()> {
        ReadyGithubAutomationPort.push_branch(repo_root, branch_name, force_with_lease)
    }

    fn ensure_pull_request(
        &self,
        repo_root: &str,
        base_branch: &str,
        head_branch: &str,
        title: &str,
        body: &str,
    ) -> anyhow::Result<GithubAutomationPullRequest> {
        ReadyGithubAutomationPort.ensure_pull_request(
            repo_root,
            base_branch,
            head_branch,
            title,
            body,
        )
    }

    fn inspect_pull_request(
        &self,
        repo_root: &str,
        pr_number: u64,
    ) -> anyhow::Result<GithubAutomationPullRequest> {
        ReadyGithubAutomationPort.inspect_pull_request(repo_root, pr_number)
    }

    fn push_integration_branch(
        &self,
        repo_root: &str,
        branch_name: &str,
        expected_old_commit_sha: &str,
    ) -> anyhow::Result<()> {
        ReadyGithubAutomationPort.push_integration_branch(
            repo_root,
            branch_name,
            expected_old_commit_sha,
        )
    }

    fn close_pull_request(&self, repo_root: &str, pr_number: u64) -> anyhow::Result<()> {
        ReadyGithubAutomationPort.close_pull_request(repo_root, pr_number)
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
    test_parallel_mode_service_with_github(authority, Arc::new(ReadyGithubAutomationPort))
}

fn test_parallel_mode_service_with_github(
    authority: Arc<SqlitePlanningAuthorityAdapter>,
    github_automation: Arc<dyn GithubAutomationPort>,
) -> ParallelModeService {
    ParallelModeService::new(
        authority,
        github_automation,
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
    test_control_plane_handle_with_github(Arc::new(ReadyGithubAutomationPort))
}

fn test_control_plane_handle_with_github(
    github_automation: Arc<dyn GithubAutomationPort>,
) -> (
    ParallelModeControlPlaneHandle<CapturingControlPlaneEventSink>,
    mpsc::Receiver<ParallelModeControlPlaneBackgroundEvent>,
) {
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let parallel_mode_service =
        test_parallel_mode_service_with_github(authority.clone(), github_automation);
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

fn test_control_plane_handle_with_noop_authority(
    authority: Arc<NoopPlanningAuthorityPort>,
) -> (
    ParallelModeControlPlaneHandle<CapturingControlPlaneEventSink>,
    mpsc::Receiver<ParallelModeControlPlaneBackgroundEvent>,
) {
    let parallel_mode_service = ParallelModeService::new(
        authority.clone(),
        Arc::new(ReadyGithubAutomationPort),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    let planning = PlanningServices::from_ports(
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        authority,
        Arc::new(NoopPlanningTaskRepositoryPort),
        Arc::new(NoopPlanningWorkerPort),
    );
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

#[test]
fn disabling_parallel_mode_cancels_the_active_automation_epoch() {
    let (handle, _rx) = test_control_plane_handle();
    let workspace = unique_workspace("cancel-epoch");

    let _ = handle.handle_command(ParallelModeControlPlaneCommand::OpenEpoch {
        workspace_directory: workspace.clone(),
    });
    let epoch_id = handle
        .current_epoch_id_for_workspace(&workspace)
        .expect("opening an epoch should assign an id");
    assert!(handle.automation_epoch_is_active(&workspace, epoch_id));

    let _ = handle.handle_command(ParallelModeControlPlaneCommand::Disable {
        workspace_directory: workspace.clone(),
    });

    assert!(!handle.automation_epoch_is_active(&workspace, epoch_id));
}

#[test]
fn opening_another_workspace_cancels_the_superseded_automation_epoch() {
    let (handle, _rx) = test_control_plane_handle();
    let first_workspace = unique_workspace("first-epoch");
    let second_workspace = unique_workspace("second-epoch");

    let first_effect = handle.force_parallel_entry_in_flight_for_test(first_workspace.clone(), 1);
    let first_epoch_id = handle
        .current_epoch_id_for_workspace(&first_workspace)
        .expect("first workspace should own an epoch");
    assert!(handle.automation_epoch_is_active(&first_workspace, first_epoch_id));
    let _ =
        handle.handle_background_event(ParallelModeControlPlaneBackgroundEvent::EnterProgress {
            workspace_directory: first_workspace.clone(),
            epoch_id: first_epoch_id,
            effect_id: first_effect,
            readiness_snapshot: Some(ready_readiness(&first_workspace)),
            loading_stage: ParallelModeControlPlaneLoadingStage::ReconcilingPool,
            status_text: "first workspace progress".to_string(),
        });

    let _ = handle.handle_command(ParallelModeControlPlaneCommand::OpenEpoch {
        workspace_directory: second_workspace.clone(),
    });
    let second_epoch_id = handle
        .current_epoch_id_for_workspace(&second_workspace)
        .expect("second workspace should own an epoch");

    assert!(!handle.automation_epoch_is_active(&first_workspace, first_epoch_id));
    assert!(handle.automation_epoch_is_active(&second_workspace, second_epoch_id));
    assert!(
        handle.readiness_snapshot_for_test().is_none(),
        "the new workspace must not inherit the old readiness cache"
    );
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

fn loading_inspection_correlation(
    handle: &ParallelModeControlPlaneHandle<CapturingControlPlaneEventSink>,
) -> ParallelModeSupervisorInspectionCorrelation {
    match handle.supervisor_inspection_state() {
        ParallelModeSupervisorInspectionState::Loading { correlation, .. } => correlation,
        state => panic!("expected loading supervisor inspection, got {state:?}"),
    }
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

fn only_pending_dispatch_poll_correlation(
    outcome: &ParallelModeControlPlaneRuntimeOutcome,
) -> ParallelModePendingDispatchPollCorrelation {
    match outcome.effects.as_slice() {
        [ParallelModeControlPlaneEffect::PollPendingDispatchWake { correlation }] => {
            correlation.clone()
        }
        effects => panic!("expected one pending dispatch poll effect, got {effects:?}"),
    }
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
            correlation: ParallelModeSupervisorInspectionCorrelation::new(
                1,
                WORKSPACE.to_string(),
                None,
            ),
            mode_enabled: false,
            reconcile_pool: false,
        }
        .effect_id(),
        None
    );
    assert_eq!(
        ParallelModeControlPlaneEffect::PollPendingDispatchWake {
            correlation: ParallelModePendingDispatchPollCorrelation::new(
                1,
                WORKSPACE.to_string(),
                1,
            ),
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
            correlation: ParallelModeSupervisorInspectionCorrelation::new(
                1,
                WORKSPACE.to_string(),
                None,
            ),
            mode_enabled: false,
            reconcile_pool: false,
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
fn inspection_queues_exact_parallel_entry_instead_of_downgrading_enable_to_refresh() {
    let mut runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut runtime);
    let inspected = runtime.handle(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: WORKSPACE.to_string(),
        reconcile_pool: false,
        show_status: false,
    });
    let correlation = match inspected.effects.as_slice() {
        [ParallelModeControlPlaneEffect::InspectSupervisor { correlation, .. }] => {
            correlation.clone()
        }
        effects => panic!("inspection should start immediately, got {effects:?}"),
    };

    let enabled = runtime.handle(ParallelModeControlPlaneCommand::Enable {
        workspace_directory: WORKSPACE.to_string(),
    });
    assert!(enabled.effects.is_empty());
    assert!(enabled.events.iter().any(|event| matches!(
        event,
        ParallelModeControlPlaneEvent::ModeEnabled { epoch_id: 1, .. }
    )));
    assert!(!enabled.events.iter().any(|event| matches!(
        event,
        ParallelModeControlPlaneEvent::SupervisorRefreshQueued
    )));
    let enabled_again = runtime.handle(ParallelModeControlPlaneCommand::Enable {
        workspace_directory: WORKSPACE.to_string(),
    });
    assert!(enabled_again.effects.is_empty());
    assert!(!enabled_again.events.iter().any(|event| matches!(
        event,
        ParallelModeControlPlaneEvent::SupervisorRefreshQueued
    )));

    let completed = runtime.handle(
        ParallelModeControlPlaneCommand::SupervisorInspectionCompleted {
            correlation,
            succeeded: true,
        },
    );
    assert!(matches!(
        completed.effects.as_slice(),
        [ParallelModeControlPlaneEffect::EnterParallelMode {
            epoch_id: 1,
            mode_was_enabled: false,
            initial_pool_reset_required: true,
            ..
        }]
    ));
    assert!(completed.events.iter().any(|event| matches!(
        event,
        ParallelModeControlPlaneEvent::SupervisorInspectionCompleted {
            projection_current: false,
            ..
        }
    )));
    assert!(completed.effects.iter().all(|effect| !matches!(
        effect,
        ParallelModeControlPlaneEffect::RefreshSupervisor { .. }
    )));
}

#[test]
fn stronger_inspection_intent_runs_after_passive_inspection_completes() {
    let mut runtime = ParallelModeControlPlaneRuntime::new();
    runtime.force_mode_for_test(WORKSPACE, true);
    let passive = runtime.handle(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: WORKSPACE.to_string(),
        reconcile_pool: false,
        show_status: false,
    });
    let passive_correlation = match passive.effects.as_slice() {
        [
            ParallelModeControlPlaneEffect::InspectSupervisor {
                correlation,
                reconcile_pool: false,
                ..
            },
        ] => correlation.clone(),
        effects => panic!("passive inspection should start, got {effects:?}"),
    };

    let stronger = runtime.handle(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: WORKSPACE.to_string(),
        reconcile_pool: true,
        show_status: true,
    });
    assert!(stronger.events.is_empty());
    assert!(stronger.effects.is_empty());

    let completed = runtime.handle(
        ParallelModeControlPlaneCommand::SupervisorInspectionCompleted {
            correlation: passive_correlation.clone(),
            succeeded: true,
        },
    );
    let stronger_correlation = match completed.effects.as_slice() {
        [
            ParallelModeControlPlaneEffect::InspectSupervisor {
                correlation,
                mode_enabled: true,
                reconcile_pool: true,
            },
        ] => correlation,
        effects => panic!("stronger inspection should run next, got {effects:?}"),
    };
    assert!(stronger_correlation.operation_id > passive_correlation.operation_id);
    assert!(completed.events.iter().any(|event| matches!(
        event,
        ParallelModeControlPlaneEvent::SupervisorInspectionStarted {
            correlation,
            show_status: true,
        } if correlation == stronger_correlation
    )));
}

#[test]
fn inspection_epoch_binding_and_completion_validate_the_exact_workspace() {
    let mut passive_runtime = ParallelModeControlPlaneRuntime::new();
    passive_runtime.force_epoch_for_test("/active", 7);
    let passive = passive_runtime.handle(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: "/inspected".to_string(),
        reconcile_pool: false,
        show_status: false,
    });
    let passive_correlation = match passive.effects.as_slice() {
        [ParallelModeControlPlaneEffect::InspectSupervisor { correlation, .. }] => correlation,
        effects => panic!("passive workspace inspection should start, got {effects:?}"),
    };
    assert_eq!(passive_correlation.epoch_id, None);

    let mut active_runtime = ParallelModeControlPlaneRuntime::new();
    active_runtime.force_epoch_for_test("/first", 7);
    let active = active_runtime.handle(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: "/first".to_string(),
        reconcile_pool: false,
        show_status: false,
    });
    let active_correlation = match active.effects.as_slice() {
        [ParallelModeControlPlaneEffect::InspectSupervisor { correlation, .. }] => {
            correlation.clone()
        }
        effects => panic!("active workspace inspection should start, got {effects:?}"),
    };
    assert_eq!(active_correlation.epoch_id, Some(7));

    active_runtime.force_epoch_for_test("/second", 7);
    let stale = active_runtime.handle(
        ParallelModeControlPlaneCommand::SupervisorInspectionCompleted {
            correlation: active_correlation,
            succeeded: true,
        },
    );
    assert!(matches!(
        stale.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { reason, .. }]
            if reason == "parallel supervisor inspection context is stale"
    ));
}

#[test]
fn unchanged_recovery_signature_dedupes_until_external_state_changes_in_same_epoch() {
    let mut runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut runtime);
    let dirty_signature = "head:none|recovery:slot-2|source:untracked|head:abc";
    let started = runtime.handle(ParallelModeControlPlaneCommand::RunOrchestratorTick {
        workspace_directory: WORKSPACE.to_string(),
        signature: dirty_signature.to_string(),
    });
    let tick_id = only_effect_id(&started);
    let completed = runtime.handle(ParallelModeControlPlaneCommand::OrchestratorTickCompleted {
        workspace_directory: WORKSPACE.to_string(),
        epoch_id: 1,
        effect_id: tick_id,
        blocked: true,
    });
    let refresh_id = completed
        .effects
        .iter()
        .find_map(ParallelModeControlPlaneEffect::effect_id)
        .expect("blocked recovery should refresh the supervisor");
    let refreshed = runtime.handle(
        ParallelModeControlPlaneCommand::SupervisorSnapshotRefreshCompleted {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: refresh_id,
            follow_up_tick_signature: Some(dirty_signature.to_string()),
        },
    );
    assert!(refreshed.effects.iter().all(|effect| !matches!(
        effect,
        ParallelModeControlPlaneEffect::RunOrchestratorTick { .. }
    )));
    let poll_correlation = only_pending_dispatch_poll_correlation(&refreshed);
    let polled = runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
        correlation: poll_correlation,
        result: Ok(None),
    });
    assert!(polled.effects.iter().all(|effect| !matches!(
        effect,
        ParallelModeControlPlaneEffect::RunOrchestratorTick { .. }
    )));
    let unchanged = runtime.handle(ParallelModeControlPlaneCommand::RunOrchestratorTick {
        workspace_directory: WORKSPACE.to_string(),
        signature: dirty_signature.to_string(),
    });
    assert!(unchanged.effects.is_empty());

    let repaired = runtime.handle(ParallelModeControlPlaneCommand::RunOrchestratorTick {
        workspace_directory: WORKSPACE.to_string(),
        signature: "head:none|recovery:slot-2|source:clean|head:def".to_string(),
    });
    assert!(matches!(
        repaired.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RunOrchestratorTick { epoch_id: 1, .. }]
    ));
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
        [ParallelModeControlPlaneEffect::PollPendingDispatchWake { correlation }]
            if correlation.epoch_id == 1
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
        [ParallelModeControlPlaneEffect::PollPendingDispatchWake { correlation }]
            if correlation.workspace_directory == WORKSPACE && correlation.epoch_id == 1
    ));
    assert_eq!(
        busy_runtime
            .store
            .pending_dispatch_poll_in_flight
            .as_ref()
            .and_then(|in_flight| in_flight.follow_up_tick_signature.as_deref()),
        Some("tick")
    );

    let mut polled_runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut polled_runtime);
    let started_error =
        polled_runtime.handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
            workspace_directory: WORKSPACE.to_string(),
            follow_up_tick_signature: None,
        });
    let error_correlation = only_pending_dispatch_poll_correlation(&started_error);
    let error = polled_runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
        correlation: error_correlation,
        result: Err("sqlite busy".to_string()),
    });
    assert_eq!(
        error.events,
        vec![ParallelModeControlPlaneEvent::DispatchWithheld {
            trigger: Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
            reason: "pending dispatch command poll failed: sqlite busy".to_string(),
        }]
    );

    let started_wake =
        polled_runtime.handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
            workspace_directory: WORKSPACE.to_string(),
            follow_up_tick_signature: Some("unused".to_string()),
        });
    let wake_correlation = only_pending_dispatch_poll_correlation(&started_wake);
    let wake_polled =
        polled_runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
            correlation: wake_correlation,
            result: Ok(Some(wake(1))),
        });
    assert!(matches!(
        wake_polled.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RunOrchestrator { .. }]
    ));

    let stale_polled =
        polled_runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
            correlation: ParallelModePendingDispatchPollCorrelation::new(
                99,
                WORKSPACE.to_string(),
                99,
            ),
            result: Ok(None),
        });
    assert!(matches!(
        stale_polled.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { epoch_id: 99, .. }]
    ));
}

#[test]
fn pending_dispatch_poll_coalesces_ticks_and_keeps_the_stronger_follow_up() {
    let mut runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut runtime);
    let started = runtime.handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
        workspace_directory: WORKSPACE.to_string(),
        follow_up_tick_signature: None,
    });
    let correlation = only_pending_dispatch_poll_correlation(&started);

    let coalesced = runtime.handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
        workspace_directory: WORKSPACE.to_string(),
        follow_up_tick_signature: Some("coalesced-tick".to_string()),
    });
    assert!(coalesced.effects.is_empty());
    assert_eq!(
        runtime
            .store
            .pending_dispatch_poll_in_flight
            .as_ref()
            .map(|in_flight| &in_flight.correlation),
        Some(&correlation)
    );

    let completed = runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
        correlation,
        result: Ok(None),
    });
    assert!(matches!(
        completed.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RunOrchestratorTick { signature, .. }]
            if signature == "coalesced-tick"
    ));
}

#[test]
fn deduped_poll_follow_up_still_drains_work_queued_while_authority_was_loading() {
    let mut runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut runtime);
    runtime.store.last_orchestrator_tick_signature = Some("same-tick".to_string());
    let started = runtime.handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
        workspace_directory: WORKSPACE.to_string(),
        follow_up_tick_signature: Some("same-tick".to_string()),
    });
    let correlation = only_pending_dispatch_poll_correlation(&started);
    runtime.store.pending_supervisor_refresh = true;

    let completed = runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
        correlation,
        result: Ok(None),
    });

    assert!(matches!(
        completed.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RefreshSupervisor { .. }]
    ));
}

#[test]
fn new_poll_follow_up_yields_to_queued_refresh_before_queued_wake() {
    let mut runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut runtime);
    let started = runtime.handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
        workspace_directory: WORKSPACE.to_string(),
        follow_up_tick_signature: Some("new-tick".to_string()),
    });
    let correlation = only_pending_dispatch_poll_correlation(&started);
    runtime.store.pending_supervisor_refresh = true;
    runtime.store.pending_orchestrator_wake = Some(ParallelModeControlPlaneWake::new(
        WORKSPACE,
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        1,
        None,
    ));

    let completed = runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
        correlation,
        result: Ok(None),
    });

    assert!(matches!(
        completed.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RefreshSupervisor { .. }]
    ));
    assert_eq!(
        runtime
            .store
            .pending_orchestrator_wake
            .as_ref()
            .map(|wake| wake.trigger),
        Some(ParallelModeAutomationTrigger::ParallelOfficialCompletion)
    );
    assert!(
        completed.effects.iter().all(|effect| !matches!(
            effect,
            ParallelModeControlPlaneEffect::RunOrchestratorTick { .. }
        )),
        "the captured tick signature is stale once a newer refresh is queued"
    );
}

#[test]
fn new_poll_follow_up_yields_to_a_wake_queued_while_authority_was_loading() {
    let mut runtime = ParallelModeControlPlaneRuntime::new();
    open_epoch(&mut runtime);
    let started = runtime.handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
        workspace_directory: WORKSPACE.to_string(),
        follow_up_tick_signature: Some("new-tick".to_string()),
    });
    let correlation = only_pending_dispatch_poll_correlation(&started);
    runtime.store.pending_orchestrator_wake = Some(ParallelModeControlPlaneWake::new(
        WORKSPACE,
        ParallelModeAutomationTrigger::MainTurnPostEvaluation,
        1,
        None,
    ));

    let completed = runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
        correlation,
        result: Ok(None),
    });

    assert!(matches!(
        completed.effects.as_slice(),
        [ParallelModeControlPlaneEffect::RunOrchestrator { wake, .. }]
            if wake.trigger == ParallelModeAutomationTrigger::MainTurnPostEvaluation
    ));
    assert!(
        completed.effects.iter().all(|effect| !matches!(
            effect,
            ParallelModeControlPlaneEffect::RunOrchestratorTick { .. }
        )),
        "a wake accepted during the poll must run before the captured follow-up tick"
    );
}

#[test]
fn pending_dispatch_poll_rejects_disable_workspace_switch_duplicate_and_aba_completions() {
    let mut runtime = ParallelModeControlPlaneRuntime::new();
    runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
        workspace_directory: "/first".to_string(),
    });
    let first = runtime.handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
        workspace_directory: "/first".to_string(),
        follow_up_tick_signature: None,
    });
    let stale = only_pending_dispatch_poll_correlation(&first);

    runtime.handle(ParallelModeControlPlaneCommand::Disable {
        workspace_directory: "/first".to_string(),
    });
    runtime.handle(ParallelModeControlPlaneCommand::OpenEpoch {
        workspace_directory: "/second".to_string(),
    });
    let second = runtime.handle(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
        workspace_directory: "/second".to_string(),
        follow_up_tick_signature: None,
    });
    let current = only_pending_dispatch_poll_correlation(&second);
    assert!(current.operation_id > stale.operation_id);
    assert!(current.epoch_id > stale.epoch_id);

    let stale_completion =
        runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
            correlation: stale,
            result: Ok(None),
        });
    assert!(matches!(
        stale_completion.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { reason, .. }]
            if reason == "unknown pending dispatch poll"
    ));
    assert_eq!(
        runtime
            .store
            .pending_dispatch_poll_in_flight
            .as_ref()
            .map(|in_flight| &in_flight.correlation),
        Some(&current)
    );

    let mut forged = current.clone();
    forged.operation_id = forged.operation_id.saturating_add(1);
    let forged_completion =
        runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
            correlation: forged,
            result: Ok(None),
        });
    assert!(matches!(
        forged_completion.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { reason, .. }]
            if reason == "unknown pending dispatch poll"
    ));
    assert_eq!(
        runtime
            .store
            .pending_dispatch_poll_in_flight
            .as_ref()
            .map(|in_flight| &in_flight.correlation),
        Some(&current)
    );

    let accepted = runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
        correlation: current.clone(),
        result: Ok(None),
    });
    assert!(accepted.effects.is_empty());
    assert!(runtime.store.pending_dispatch_poll_in_flight.is_none());
    let duplicate = runtime.handle(ParallelModeControlPlaneCommand::PendingDispatchWakePolled {
        correlation: current,
        result: Ok(None),
    });
    assert!(matches!(
        duplicate.events.as_slice(),
        [ParallelModeControlPlaneEvent::StaleCommandDropped { reason, .. }]
            if reason == "unknown pending dispatch poll"
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
    assert!(matches!(busy_schedule_outcome.effects.as_slice(), []));

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
            ParallelModeControlPlaneEvent::ConversationRuntimeNotice { notice, .. },
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
            ParallelModeControlPlaneEvent::ConversationRuntimeNotice { notice, .. },
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
    let entry_effect_id = handle.force_parallel_entry_in_flight_for_test(WORKSPACE, 1);

    let notice = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::ConversationRuntimeNotice {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: entry_effect_id,
            notice: "runtime notice".to_string(),
        },
    );
    assert!(matches!(
        notice.as_slice(),
        [ParallelModeControlPlanePresentationEvent::ConversationRuntimeNotice { notice, .. }]
            if notice == "runtime notice"
    ));

    let inactive_progress =
        handle.handle_background_event(ParallelModeControlPlaneBackgroundEvent::EnterProgress {
            workspace_directory: "/other".to_string(),
            epoch_id: 1,
            effect_id: entry_effect_id,
            readiness_snapshot: Some(ready_readiness("/other")),
            loading_stage: ParallelModeControlPlaneLoadingStage::ReconcilingPool,
            status_text: "ignored".to_string(),
        });
    assert!(inactive_progress.is_empty());

    let active_progress =
        handle.handle_background_event(ParallelModeControlPlaneBackgroundEvent::EnterProgress {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: entry_effect_id,
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
fn same_workspace_reenable_rejects_prior_epoch_entry_progress() {
    let (handle, _rx) = test_control_plane_handle();
    let first_effect = handle.force_parallel_entry_in_flight_for_test(WORKSPACE, 1);
    let _ = handle.handle_command(ParallelModeControlPlaneCommand::Disable {
        workspace_directory: WORKSPACE.to_string(),
    });
    let second_effect = handle.force_parallel_entry_in_flight_for_test(WORKSPACE, 2);
    assert_ne!(first_effect, second_effect);

    let stale =
        handle.handle_background_event(ParallelModeControlPlaneBackgroundEvent::EnterProgress {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: first_effect,
            readiness_snapshot: Some(ready_readiness(WORKSPACE)),
            loading_stage: ParallelModeControlPlaneLoadingStage::ReconcilingPool,
            status_text: "stale epoch one progress".to_string(),
        });
    assert!(stale.is_empty());
    let stale_notice = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::ConversationRuntimeNotice {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 1,
            effect_id: first_effect,
            notice: "stale epoch one notice".to_string(),
        },
    );
    assert!(stale_notice.is_empty());

    let current =
        handle.handle_background_event(ParallelModeControlPlaneBackgroundEvent::EnterProgress {
            workspace_directory: WORKSPACE.to_string(),
            epoch_id: 2,
            effect_id: second_effect,
            readiness_snapshot: Some(ready_readiness(WORKSPACE)),
            loading_stage: ParallelModeControlPlaneLoadingStage::ReconcilingPool,
            status_text: "current epoch two progress".to_string(),
        });
    assert!(matches!(
        current.as_slice(),
        [ParallelModeControlPlanePresentationEvent::EnterProgress {
            epoch_id: 2,
            effect_id,
            status_text,
            ..
        }] if *effect_id == second_effect && status_text == "current epoch two progress"
    ));
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
        ParallelModeControlPlanePresentationEvent::ConversationRuntimeNotice { notice, .. }
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
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text, .. }
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
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text, .. }
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
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text, .. }
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
    handle.force_readiness_snapshot_for_test(ready_readiness(WORKSPACE));

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

    let poll_event = recv_background_event(&rx);
    assert!(matches!(
        poll_event,
        ParallelModeControlPlaneBackgroundEvent::PendingDispatchWakePolled { .. }
    ));
    assert!(
        handle.handle_background_event(poll_event).is_empty(),
        "an empty poll should only schedule the correlated follow-up tick"
    );
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
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text, .. }
            if status_text == "parallel mode: distributor retry completed / notices: 0"
    )));
}

#[test]
fn pending_dispatch_poll_returns_before_a_gated_six_hundred_millisecond_authority_read() {
    let shared_projection = Arc::new(Mutex::new(
        PlanningAuthorityRuntimeProjectionSnapshot::default(),
    ));
    let authority = Arc::new(
        NoopPlanningAuthorityPort::default()
            .with_shared_runtime_projection(shared_projection.clone()),
    );
    let (handle, rx) = test_control_plane_handle_with_noop_authority(authority);
    let workspace = unique_workspace("pending-poll-nonblocking");
    handle.force_epoch_for_test(&workspace, 1);

    let (gate_entered_tx, gate_entered_rx) = mpsc::channel();
    let gate = shared_projection.clone();
    let gate_thread = std::thread::spawn(move || {
        let _guard = gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        gate_entered_tx
            .send(())
            .expect("authority gate should report acquisition");
        std::thread::sleep(Duration::from_millis(600));
    });
    gate_entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("authority gate should be held before dispatch");

    let started_at = Instant::now();
    let presented =
        handle.handle_command(ParallelModeControlPlaneCommand::PollPendingDispatchWake {
            workspace_directory: workspace.clone(),
            follow_up_tick_signature: None,
        });
    let command_elapsed = started_at.elapsed();

    assert!(
        command_elapsed < Duration::from_millis(300),
        "pending dispatch poll held the control-plane mutex for {command_elapsed:?}"
    );
    assert!(presented.is_empty());
    assert!(
        handle.control_effect_in_flight(),
        "correlation must be installed before the worker is dispatched"
    );
    let snapshot_started_at = Instant::now();
    assert_eq!(
        handle
            .current_epoch_id_for_workspace(&workspace)
            .expect("active epoch should remain readable"),
        1
    );
    assert!(
        snapshot_started_at.elapsed() < Duration::from_millis(300),
        "the async authority read must not retain the control-plane mutex"
    );

    gate_thread
        .join()
        .expect("authority gate thread should complete");
    let completed = recv_background_event(&rx);
    assert!(matches!(
        &completed,
        ParallelModeControlPlaneBackgroundEvent::PendingDispatchWakePolled {
            correlation,
            result: Ok(None),
        } if correlation.workspace_directory == workspace
            && correlation.epoch_id == 1
            && correlation.operation_id == 1
    ));
    assert!(handle.handle_background_event(completed).is_empty());
    assert!(!handle.control_effect_in_flight());
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
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text, .. }
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
    refresh_handle.force_readiness_snapshot_for_test(ready_readiness(WORKSPACE));
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
    let (handle, rx) = test_control_plane_handle();
    let workspace = unique_workspace("inspect-reconcile");
    handle.force_mode_for_test(&workspace, true);

    let loading = handle.handle_command(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: workspace.clone(),
        reconcile_pool: true,
        show_status: true,
    });

    assert!(matches!(
        handle.supervisor_inspection_state(),
        ParallelModeSupervisorInspectionState::Loading { .. }
    ));
    assert!(loading.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text, .. }
            if status_text.starts_with("parallel readiness refresh: loading")
    )));
    let completed = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("inspection worker should report completion");
    let presented = handle.handle_background_event(completed);
    assert!(matches!(
        handle.supervisor_inspection_state(),
        ParallelModeSupervisorInspectionState::Ready { .. }
    ));

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
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text, .. }
            if status_text.starts_with("parallel readiness refreshed / state:")
    )));
}

#[test]
fn enabling_during_passive_inspection_preserves_projection_until_entry_completes() {
    let (handle, _rx) = test_control_plane_handle();
    let workspace = unique_workspace("inspect-enable-transition");
    let previous_readiness = ready_readiness(&workspace);
    let _ = handle.handle_command(ParallelModeControlPlaneCommand::OpenEpoch {
        workspace_directory: workspace.clone(),
    });
    handle.force_readiness_snapshot_for_test(previous_readiness.clone());
    let _ = handle.handle_command(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: workspace.clone(),
        reconcile_pool: false,
        show_status: true,
    });
    let correlation = loading_inspection_correlation(&handle);
    let _ = handle.handle_command(ParallelModeControlPlaneCommand::Enable {
        workspace_directory: workspace.clone(),
    });

    let presented = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::SupervisorInspectionCompleted {
            correlation,
            result: Ok(ParallelModeSupervisorInspectionSnapshot {
                readiness_snapshot: ready_readiness(&workspace),
                supervisor_snapshot: Box::new(supervisor_snapshot(&workspace)),
            }),
        },
    );

    assert_eq!(
        handle.readiness_snapshot_for_test(),
        Some(previous_readiness)
    );
    assert!(matches!(
        handle.supervisor_inspection_state(),
        ParallelModeSupervisorInspectionState::Idle
    ));
    assert!(handle.control_effect_in_flight());
    assert!(!presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::ReadinessSnapshotChanged { .. }
            | ParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged { .. }
    )));
    assert!(!presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text, .. }
            if status_text.starts_with("parallel readiness refreshed / state:")
    )));
}

#[test]
fn epochless_passive_inspection_does_not_replace_active_readiness_cache() {
    let (handle, _rx) = test_control_plane_handle();
    let inspected_workspace = unique_workspace("epochless-inspection");
    let active_readiness = ready_readiness("/active-readiness-cache");
    handle.force_readiness_snapshot_for_test(active_readiness.clone());
    let _ = handle.handle_command(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: inspected_workspace.clone(),
        reconcile_pool: false,
        show_status: false,
    });
    let correlation = loading_inspection_correlation(&handle);
    assert_eq!(correlation.epoch_id, None);

    let presented = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::SupervisorInspectionCompleted {
            correlation,
            result: Ok(ParallelModeSupervisorInspectionSnapshot {
                readiness_snapshot: ready_readiness(&inspected_workspace),
                supervisor_snapshot: Box::new(supervisor_snapshot(&inspected_workspace)),
            }),
        },
    );

    assert_eq!(handle.readiness_snapshot_for_test(), Some(active_readiness));
    assert!(presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::ReadinessSnapshotChanged {
            workspace_directory,
            ..
        } if workspace_directory == &inspected_workspace
    )));
}

#[test]
fn supervisor_inspection_returns_loading_before_the_io_worker_is_released() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let github = Arc::new(GatedGithubAutomationPort {
        entered_tx,
        release_rx: Mutex::new(release_rx),
    });
    let (handle, background_rx) = test_control_plane_handle_with_github(github);
    let workspace = env!("CARGO_MANIFEST_DIR").to_string();

    let started_at = Instant::now();
    let loading = handle.handle_command(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: workspace,
        reconcile_pool: false,
        show_status: true,
    });
    let command_elapsed = started_at.elapsed();

    assert!(
        command_elapsed < Duration::from_millis(750),
        "inspection command held the control-plane mutex for {command_elapsed:?}"
    );
    assert!(handle.control_effect_in_flight());
    assert!(matches!(
        handle.supervisor_inspection_state(),
        ParallelModeSupervisorInspectionState::Loading { .. }
    ));
    assert!(loading.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text, .. }
            if status_text.starts_with("parallel readiness refresh: loading")
    )));
    let correlation = loading_inspection_correlation(&handle);
    assert!(
        handle
            .handle_command(ParallelModeControlPlaneCommand::InspectSupervisor {
                workspace_directory: correlation.workspace_directory.clone(),
                reconcile_pool: false,
                show_status: true,
            })
            .is_empty(),
        "a duplicate inspection must be gated while the worker is in flight"
    );
    assert_eq!(loading_inspection_correlation(&handle), correlation);
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("inspection worker should reach the gated GitHub capability read");
    release_tx
        .send(())
        .expect("inspection worker release should be delivered");
    let completed = recv_background_event(&background_rx);
    let presented = handle.handle_background_event(completed);
    assert!(matches!(
        handle.supervisor_inspection_state(),
        ParallelModeSupervisorInspectionState::Ready { .. }
    ));
    assert!(presented.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged { .. }
    )));
}

#[test]
fn disabling_before_inspection_io_is_released_rejects_reconciliation_permit() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let github = Arc::new(GatedGithubAutomationPort {
        entered_tx,
        release_rx: Mutex::new(release_rx),
    });
    let (handle, background_rx) = test_control_plane_handle_with_github(github);
    let workspace = env!("CARGO_MANIFEST_DIR").to_string();
    handle.force_mode_for_test(&workspace, true);
    let epoch_id = handle
        .current_epoch_id_for_workspace(&workspace)
        .expect("forced mode should activate an epoch");

    let _ = handle.handle_command(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: workspace.clone(),
        reconcile_pool: true,
        show_status: true,
    });
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("inspection worker should reach the gated capability read");

    let disable_started_at = Instant::now();
    let _ = handle.handle_command(ParallelModeControlPlaneCommand::Disable {
        workspace_directory: workspace.clone(),
    });
    assert!(
        disable_started_at.elapsed() < Duration::from_millis(750),
        "epoch cancellation must not wait for the inspection I/O worker"
    );
    assert!(!handle.automation_epoch_is_active(&workspace, epoch_id));

    release_tx
        .send(())
        .expect("inspection worker release should be delivered");
    let completed = recv_background_event(&background_rx);
    assert!(matches!(
        &completed,
        ParallelModeControlPlaneBackgroundEvent::SupervisorInspectionCompleted {
            result: Err(error),
            ..
        } if error.contains("inactive epoch")
    ));
    assert!(handle.handle_background_event(completed).is_empty());
}

#[test]
fn failed_supervisor_inspection_preserves_projections_and_can_retry() {
    let (handle, _rx) = test_control_plane_handle();
    let workspace = unique_workspace("inspect-failure");
    let previous_readiness = ready_readiness(&workspace);
    handle.force_mode_for_test(&workspace, true);
    handle.force_readiness_snapshot_for_test(previous_readiness.clone());
    let _ = handle.handle_command(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: workspace.clone(),
        reconcile_pool: false,
        show_status: true,
    });
    let first = loading_inspection_correlation(&handle);

    let failed = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::SupervisorInspectionCompleted {
            correlation: first.clone(),
            result: Err("gated inspection failed".to_string()),
        },
    );

    assert_eq!(
        handle.readiness_snapshot_for_test(),
        Some(previous_readiness)
    );
    assert!(matches!(
        handle.supervisor_inspection_state(),
        ParallelModeSupervisorInspectionState::Failed {
            correlation,
            error,
        } if correlation == first && error == "gated inspection failed"
    ));
    assert!(failed.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text, .. }
            if status_text.contains("press Ctrl+R to retry")
    )));
    assert!(!failed.iter().any(|event| matches!(
        event,
        ParallelModeControlPlanePresentationEvent::ReadinessSnapshotChanged { .. }
            | ParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged { .. }
    )));

    let _ = handle.handle_command(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: workspace,
        reconcile_pool: false,
        show_status: true,
    });
    let retry = loading_inspection_correlation(&handle);
    assert!(retry.operation_id > first.operation_id);
}

#[test]
fn supervisor_inspection_rejects_stale_duplicate_workspace_and_epoch_completions() {
    let (handle, _rx) = test_control_plane_handle();
    let workspace = unique_workspace("inspect-stale");
    handle.force_epoch_for_test(&workspace, 7);
    let _ = handle.handle_command(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: workspace.clone(),
        reconcile_pool: false,
        show_status: false,
    });
    let stale = loading_inspection_correlation(&handle);
    assert_eq!(stale.epoch_id, Some(7));

    let _ = handle.handle_command(ParallelModeControlPlaneCommand::Disable {
        workspace_directory: workspace.clone(),
    });
    handle.force_epoch_for_test(&workspace, 8);
    let _ = handle.handle_command(ParallelModeControlPlaneCommand::InspectSupervisor {
        workspace_directory: workspace.clone(),
        reconcile_pool: false,
        show_status: false,
    });
    let current = loading_inspection_correlation(&handle);
    assert_eq!(current.epoch_id, Some(8));
    assert!(current.operation_id > stale.operation_id);

    let stale_events = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::SupervisorInspectionCompleted {
            correlation: stale,
            result: Err("stale epoch".to_string()),
        },
    );
    assert!(stale_events.is_empty());
    assert_eq!(loading_inspection_correlation(&handle), current);

    let mut forged = current.clone();
    forged.workspace_directory = unique_workspace("forged-inspect-workspace");
    let forged_events = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::SupervisorInspectionCompleted {
            correlation: forged,
            result: Err("forged workspace".to_string()),
        },
    );
    assert!(forged_events.is_empty());
    assert_eq!(loading_inspection_correlation(&handle), current);

    let failed = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::SupervisorInspectionCompleted {
            correlation: current.clone(),
            result: Err("current failure".to_string()),
        },
    );
    assert!(!failed.is_empty());
    let duplicate = handle.handle_background_event(
        ParallelModeControlPlaneBackgroundEvent::SupervisorInspectionCompleted {
            correlation: current,
            result: Err("duplicate".to_string()),
        },
    );
    assert!(duplicate.is_empty());
    assert!(matches!(
        handle.supervisor_inspection_state(),
        ParallelModeSupervisorInspectionState::Failed { error, .. }
            if error == "current failure"
    ));
}

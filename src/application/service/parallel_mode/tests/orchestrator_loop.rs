use super::*;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::parallel_agent_worker_port::{
    ParallelAgentWorkerPort, ParallelAgentWorkerStreamRequest,
};
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::{
    ParallelModeRuntimeEventLogPort, ParallelModeRuntimeEventLogRequest,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
    PlanningAuthorityOfficialRefreshRecoveryStatus, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_worker_port::{
    NoopPlanningWorkerPort, PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::parallel_agent_profile::{
    ParallelAgentProfile, ParallelAgentProfileConfig, save_parallel_agent_profile_config,
};
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::parallel_mode::{
    ParallelModeDispatchOrchestratorTickRequest, ParallelModeOrchestratorLoopEvent,
};
use crate::application::service::planning::{PlanningServices, PlanningTaskIntakeRequest};
use crate::diagnostics::trace_event_log::AKRA_EVENT_TARGET;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAutomationTrigger,
    ParallelModeControlPlaneWorkerEventKind, ParallelModeDispatchCommandSnapshot,
    ParallelModeDispatchCommandState, ParallelModePoolResetReport, ParallelModeRuntimeEvent,
    ParallelModeRuntimeEventsSnapshot, ParallelModeSlotLeaseRequest, ParallelModeSlotLeaseSnapshot,
    ParallelModeTaskDispatchBlockSnapshot,
};
use crate::domain::planning::{PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex, mpsc};
use std::thread;
use std::time::Duration;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

fn with_akra_event_trace<T>(body: impl FnOnce() -> T) -> T {
    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::new(format!("{AKRA_EVENT_TARGET}=debug")))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::sink));
    tracing::subscriber::with_default(subscriber, body)
}

#[derive(Debug, Default)]
struct CountingParallelAgentWorkerPort {
    launch_count: AtomicUsize,
}

impl CountingParallelAgentWorkerPort {
    fn launch_count(&self) -> usize {
        self.launch_count.load(Ordering::SeqCst)
    }
}

impl ParallelAgentWorkerPort for CountingParallelAgentWorkerPort {
    fn run_isolated_new_thread_stream(
        &self,
        _request: ParallelAgentWorkerStreamRequest<'_>,
        event_sender: mpsc::Sender<ConversationStreamEvent>,
    ) -> anyhow::Result<()> {
        self.launch_count.fetch_add(1, Ordering::SeqCst);
        let _ = event_sender.send(ConversationStreamEvent::Failed {
            message: "test worker stops after launch".to_string(),
        });
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedParallelWorkerLaunch {
    cwd: String,
    service_name: String,
}

#[derive(Debug)]
struct HoldingParallelAgentWorkerPort {
    launch_count: AtomicUsize,
    launches: Mutex<Vec<CapturedParallelWorkerLaunch>>,
    release_rx: Mutex<mpsc::Receiver<()>>,
}

impl HoldingParallelAgentWorkerPort {
    fn new(release_rx: mpsc::Receiver<()>) -> Self {
        Self {
            launch_count: AtomicUsize::new(0),
            launches: Mutex::new(Vec::new()),
            release_rx: Mutex::new(release_rx),
        }
    }

    fn launch_count(&self) -> usize {
        self.launch_count.load(Ordering::SeqCst)
    }

    fn launches(&self) -> Vec<CapturedParallelWorkerLaunch> {
        self.launches
            .lock()
            .expect("captured launches mutex should not be poisoned")
            .clone()
    }
}

impl ParallelAgentWorkerPort for HoldingParallelAgentWorkerPort {
    fn run_isolated_new_thread_stream(
        &self,
        request: ParallelAgentWorkerStreamRequest<'_>,
        _event_sender: mpsc::Sender<ConversationStreamEvent>,
    ) -> anyhow::Result<()> {
        self.launches
            .lock()
            .expect("captured launches mutex should not be poisoned")
            .push(CapturedParallelWorkerLaunch {
                cwd: request.cwd.to_string(),
                service_name: request.service_name.to_string(),
            });
        self.launch_count.fetch_add(1, Ordering::SeqCst);
        let _ = self
            .release_rx
            .lock()
            .expect("release receiver mutex should not be poisoned")
            .recv_timeout(Duration::from_secs(5));
        Ok(())
    }
}

#[derive(Debug, Default)]
struct CompletingParallelAgentWorkerPort {
    launch_count: AtomicUsize,
}

impl CompletingParallelAgentWorkerPort {
    fn launch_count(&self) -> usize {
        self.launch_count.load(Ordering::SeqCst)
    }
}

impl ParallelAgentWorkerPort for CompletingParallelAgentWorkerPort {
    fn run_isolated_new_thread_stream(
        &self,
        request: ParallelAgentWorkerStreamRequest<'_>,
        event_sender: mpsc::Sender<ConversationStreamEvent>,
    ) -> anyhow::Result<()> {
        self.launch_count.fetch_add(1, Ordering::SeqCst);
        event_sender.send(ConversationStreamEvent::ThreadPrepared {
            thread_id: "worker-thread-1".to_string(),
            title: "Completed parallel worker".to_string(),
            cwd: request.cwd.to_string(),
        })?;
        event_sender.send(ConversationStreamEvent::TurnStarted {
            turn_id: "worker-turn-1".to_string(),
        })?;
        event_sender.send(ConversationStreamEvent::AgentMessageCompleted {
            item_id: "item-final".to_string(),
            phase: Some("final_answer".to_string()),
            text: "parallel worker finished cleanly".to_string(),
        })?;
        event_sender.send(ConversationStreamEvent::TurnCompleted {
            turn_id: "worker-turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
        })?;
        Ok(())
    }
}

struct FailingPlanningWorkerPort;

impl PlanningWorkerPort for FailingPlanningWorkerPort {
    fn run_planning_session(
        &self,
        _request: PlanningWorkerRequest,
    ) -> anyhow::Result<PlanningWorkerResponse> {
        Err(anyhow::anyhow!("official refresh worker failed"))
    }
}

struct FaultyPlanningAuthorityAdapter {
    inner: Arc<SqlitePlanningAuthorityAdapter>,
    fail_enqueue: AtomicUsize,
    fail_claim: AtomicUsize,
    fail_update: AtomicUsize,
    fail_slot_lease_upsert_after: AtomicUsize,
    slot_lease_upsert_calls: AtomicUsize,
}

impl FaultyPlanningAuthorityAdapter {
    const DISABLED: usize = usize::MAX;

    fn new(inner: Arc<SqlitePlanningAuthorityAdapter>) -> Self {
        Self {
            inner,
            fail_enqueue: AtomicUsize::new(0),
            fail_claim: AtomicUsize::new(0),
            fail_update: AtomicUsize::new(0),
            fail_slot_lease_upsert_after: AtomicUsize::new(Self::DISABLED),
            slot_lease_upsert_calls: AtomicUsize::new(0),
        }
    }

    fn fail_enqueue(&self) {
        self.fail_enqueue.store(1, Ordering::SeqCst);
    }

    fn fail_claim(&self) {
        self.fail_claim.store(1, Ordering::SeqCst);
    }

    fn fail_update(&self) {
        self.fail_update.store(1, Ordering::SeqCst);
    }

    fn fail_slot_lease_upsert_after(&self, successful_writes: usize) {
        self.fail_slot_lease_upsert_after
            .store(successful_writes, Ordering::SeqCst);
        self.slot_lease_upsert_calls.store(0, Ordering::SeqCst);
    }
}

impl ParallelModeRuntimeEventLogPort for FaultyPlanningAuthorityAdapter {
    fn load_runtime_event_log(
        &self,
        workspace_dir: &str,
        request: ParallelModeRuntimeEventLogRequest,
    ) -> anyhow::Result<ParallelModeRuntimeEventsSnapshot> {
        self.inner.load_runtime_event_log(workspace_dir, request)
    }
}

impl PlanningAuthorityPort for FaultyPlanningAuthorityAdapter {
    fn resolve_authority_location(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningAuthorityLocation> {
        self.inner.resolve_authority_location(workspace_dir)
    }

    fn inspect_shadow_store(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningAuthorityShadowStoreInspection> {
        self.inner.inspect_shadow_store(workspace_dir)
    }

    fn reserve_next_official_refresh_order(&self, workspace_dir: &str) -> anyhow::Result<u64> {
        self.inner
            .reserve_next_official_refresh_order(workspace_dir)
    }

    fn acquire_official_refresh_claim(
        &self,
        workspace_dir: &str,
        refresh_order: u64,
        owner_token: &str,
    ) -> anyhow::Result<PlanningAuthorityOfficialRefreshClaimStatus> {
        self.inner
            .acquire_official_refresh_claim(workspace_dir, refresh_order, owner_token)
    }

    fn release_official_refresh_claim(
        &self,
        workspace_dir: &str,
        refresh_order: u64,
        owner_token: &str,
    ) -> anyhow::Result<()> {
        self.inner
            .release_official_refresh_claim(workspace_dir, refresh_order, owner_token)
    }

    fn abandon_next_official_refresh_order(
        &self,
        workspace_dir: &str,
        reason: &str,
    ) -> anyhow::Result<PlanningAuthorityOfficialRefreshRecoveryStatus> {
        self.inner
            .abandon_next_official_refresh_order(workspace_dir, reason)
    }

    fn try_acquire_distributor_queue_claim(
        &self,
        workspace_dir: &str,
        queue_item_id: &str,
        owner_token: &str,
    ) -> anyhow::Result<bool> {
        self.inner
            .try_acquire_distributor_queue_claim(workspace_dir, queue_item_id, owner_token)
    }

    fn release_distributor_queue_claim(
        &self,
        workspace_dir: &str,
        queue_item_id: &str,
        owner_token: &str,
    ) -> anyhow::Result<()> {
        self.inner
            .release_distributor_queue_claim(workspace_dir, queue_item_id, owner_token)
    }

    fn load_runtime_projections(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        self.inner.load_runtime_projections(workspace_dir)
    }

    fn enqueue_runtime_dispatch_command(
        &self,
        workspace_dir: &str,
        command: &ParallelModeDispatchCommandSnapshot,
    ) -> anyhow::Result<bool> {
        if self.fail_enqueue.swap(0, Ordering::SeqCst) > 0 {
            return Err(anyhow::anyhow!("scripted dispatch enqueue failure"));
        }
        self.inner
            .enqueue_runtime_dispatch_command(workspace_dir, command)
    }

    fn try_claim_next_runtime_dispatch_command(
        &self,
        workspace_dir: &str,
        owner_token: &str,
    ) -> anyhow::Result<Option<ParallelModeDispatchCommandSnapshot>> {
        if self.fail_claim.swap(0, Ordering::SeqCst) > 0 {
            return Err(anyhow::anyhow!("scripted dispatch claim failure"));
        }
        self.inner
            .try_claim_next_runtime_dispatch_command(workspace_dir, owner_token)
    }

    fn update_runtime_dispatch_command(
        &self,
        workspace_dir: &str,
        command: &ParallelModeDispatchCommandSnapshot,
    ) -> anyhow::Result<()> {
        if self.fail_update.swap(0, Ordering::SeqCst) > 0 {
            return Err(anyhow::anyhow!("scripted dispatch update failure"));
        }
        self.inner
            .update_runtime_dispatch_command(workspace_dir, command)
    }

    fn cancel_runtime_dispatch_commands(
        &self,
        workspace_dir: &str,
        reason: &str,
    ) -> anyhow::Result<usize> {
        self.inner
            .cancel_runtime_dispatch_commands(workspace_dir, reason)
    }

    fn clear_parallel_runtime_projections(
        &self,
        workspace_dir: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        self.inner
            .clear_parallel_runtime_projections(workspace_dir, reason)
    }

    fn clear_parallel_runtime_projections_for_tasks(
        &self,
        workspace_dir: &str,
        task_ids: &[String],
        reason: &str,
    ) -> anyhow::Result<()> {
        self.inner
            .clear_parallel_runtime_projections_for_tasks(workspace_dir, task_ids, reason)
    }

    fn apply_parallel_pool_reset_report(
        &self,
        workspace_dir: &str,
        report: &ParallelModePoolResetReport,
    ) -> anyhow::Result<()> {
        self.inner
            .apply_parallel_pool_reset_report(workspace_dir, report)
    }

    fn upsert_runtime_slot_lease(
        &self,
        workspace_dir: &str,
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> anyhow::Result<()> {
        let call_index = self.slot_lease_upsert_calls.fetch_add(1, Ordering::SeqCst);
        if call_index >= self.fail_slot_lease_upsert_after.load(Ordering::SeqCst) {
            return Err(anyhow::anyhow!(
                "scripted slot lease upsert failure for {}",
                lease.slot_id
            ));
        }
        self.inner.upsert_runtime_slot_lease(workspace_dir, lease)
    }

    fn remove_runtime_slot_lease(&self, workspace_dir: &str, slot_id: &str) -> anyhow::Result<()> {
        self.inner.remove_runtime_slot_lease(workspace_dir, slot_id)
    }

    fn upsert_runtime_session_detail(
        &self,
        workspace_dir: &str,
        detail: &ParallelModeAgentSessionDetailSnapshot,
    ) -> anyhow::Result<()> {
        self.inner
            .upsert_runtime_session_detail(workspace_dir, detail)
    }

    fn upsert_runtime_task_dispatch_block(
        &self,
        workspace_dir: &str,
        block: &ParallelModeTaskDispatchBlockSnapshot,
    ) -> anyhow::Result<()> {
        self.inner
            .upsert_runtime_task_dispatch_block(workspace_dir, block)
    }

    fn upsert_runtime_distributor_queue_record(
        &self,
        workspace_dir: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
    ) -> anyhow::Result<()> {
        self.inner
            .upsert_runtime_distributor_queue_record(workspace_dir, record)
    }
}

struct RepairRequestPlanningWorkerPort;

impl PlanningWorkerPort for RepairRequestPlanningWorkerPort {
    fn run_planning_session(
        &self,
        request: PlanningWorkerRequest,
    ) -> anyhow::Result<PlanningWorkerResponse> {
        Ok(PlanningWorkerResponse {
            operation: request.operation,
            thread_id: Some("repair-worker-thread".to_string()),
            turn_id: Some("repair-worker-turn".to_string()),
            final_agent_message: Some(
                r#"```json
{"planning_task_commands":{"version":1,"commands":[{"create_task":{"title":"Missing op"}}]}}
```"#
                    .to_string(),
            ),
            changed_planning_file_paths: Vec::new(),
        })
    }
}

fn bootstrap_planning_workspace(planning: &PlanningServices, workspace_dir: &str) {
    let stage_result = planning
        .workspace
        .stage_simple_mode_draft(workspace_dir)
        .expect("planning workspace should stage");
    let promote_result = planning
        .workspace
        .promote_staged_draft(workspace_dir, &stage_result.draft_name)
        .expect("planning workspace should promote");
    assert!(promote_result.promoted_file_count > 0);
}

fn commit_ready_queue_task_with_prompt(
    planning: &PlanningServices,
    workspace_dir: &str,
    raw_prompt: &str,
) {
    let proposal = planning
        .runtime
        .prepare_task_intake(PlanningTaskIntakeRequest {
            workspace_directory: workspace_dir.to_string(),
            raw_prompt: raw_prompt.to_string(),
            legacy_source_turn_id: None,
            provenance: Default::default(),
            requested_direction_id: None,
            observed_planning_revision: None,
        })
        .expect("task intake should prepare");
    planning
        .runtime
        .commit_task_intake(&proposal)
        .expect("task intake should commit");
}

fn commit_ready_queue_task(planning: &PlanningServices, workspace_dir: &str) {
    commit_ready_queue_task_with_prompt(
        planning,
        workspace_dir,
        "Only one loop should claim this dispatch task",
    );
}

fn commit_ready_queue_tasks(planning: &PlanningServices, workspace_dir: &str, count: usize) {
    for index in 1..=count {
        commit_ready_queue_task_with_prompt(
            planning,
            workspace_dir,
            &format!("Dispatch continuation task {index}"),
        );
    }
}

fn build_test_planning_services(
    authority: Arc<SqlitePlanningAuthorityAdapter>,
) -> PlanningServices {
    build_test_planning_services_with_worker(authority, Arc::new(NoopPlanningWorkerPort))
}

fn build_test_planning_services_with_worker(
    authority: Arc<SqlitePlanningAuthorityAdapter>,
    worker: Arc<dyn PlanningWorkerPort>,
) -> PlanningServices {
    PlanningServices::from_ports(
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        authority.clone(),
        authority,
        worker,
    )
}

#[test]
fn enqueue_dispatch_commands_for_trigger_maps_public_entrypoint_to_durable_command() {
    let repo = TempGitRepo::new("orchestrator-trigger-entrypoint");
    let workspace_dir = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services(authority.clone());
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);

    let service = ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    let planning_projection = planning
        .runtime
        .load_runtime_projection_or_invalid(&workspace_dir);

    assert_eq!(
        service
            .enqueue_dispatch_commands_for_trigger(
                &workspace_dir,
                ParallelModeAutomationTrigger::MainTurnPostEvaluation,
                &planning_projection,
                Some(21),
            )
            .expect("trigger entrypoint should enqueue"),
        1
    );

    let projections = authority
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(projections.dispatch_commands.len(), 1);
    assert_eq!(
        projections.dispatch_commands[0].trigger,
        ParallelModeAutomationTrigger::MainTurnPostEvaluation
    );
}

#[test]
fn dispatch_tick_reports_no_pending_durable_command_without_mutating_queue() {
    let repo = TempGitRepo::new("orchestrator-no-pending-command");
    let workspace_dir = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services(authority.clone());
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);

    let service = Arc::new(ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    ));
    let worker_port = Arc::new(CountingParallelAgentWorkerPort::default());
    let (event_sender, event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let result =
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir.clone(),
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 22,
            enqueue_trigger: None,
            planning: planning.clone(),
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        });

    assert_eq!(
        result.outcome.blocked_reason.as_deref(),
        Some("no pending durable dispatch command")
    );
    assert_eq!(worker_port.launch_count(), 0);
    assert!(
        event_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    let projections = authority
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert!(projections.dispatch_commands.is_empty());
}

#[test]
fn dispatch_tick_blocks_before_claim_when_readiness_fails() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let workspace_path = std::env::temp_dir().join(format!(
        "parallel-mode-orchestrator-readiness-blocked-{unique}"
    ));
    fs::create_dir_all(&workspace_path).expect("non-git workspace should be created");
    let workspace_dir = workspace_path.display().to_string();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services(authority.clone());

    let service = Arc::new(ParallelModeService::new(
        authority,
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    ));
    let worker_port = Arc::new(CountingParallelAgentWorkerPort::default());
    let (event_sender, event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let result =
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir,
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 23,
            enqueue_trigger: Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
            planning,
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        });

    assert!(!result.readiness_snapshot.allows_parallel_mode());
    assert!(
        result
            .outcome
            .blocked_reason
            .as_deref()
            .expect("readiness failure should block")
            .starts_with("readiness:")
    );
    assert_eq!(worker_port.launch_count(), 0);
    assert!(
        event_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    fs::remove_dir_all(workspace_path).expect("non-git workspace should be removed");
}

#[test]
fn dispatch_tick_reports_enqueue_and_claim_failures_without_launching_workers() {
    let repo = TempGitRepo::new("orchestrator-command-failures");
    let workspace_dir = repo.workspace_dir();
    let inner_authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let authority = Arc::new(FaultyPlanningAuthorityAdapter::new(inner_authority.clone()));
    let planning = build_test_planning_services(inner_authority);
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);

    let service = Arc::new(ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    ));
    let worker_port = Arc::new(CountingParallelAgentWorkerPort::default());

    authority.fail_enqueue();
    let (event_sender, event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let enqueue_result = with_akra_event_trace(|| {
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir.clone(),
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 231,
            enqueue_trigger: Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
            planning: planning.clone(),
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        })
    });
    assert_eq!(
        enqueue_result.outcome.blocked_reason.as_deref(),
        Some("no pending durable dispatch command")
    );
    assert!(
        event_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );

    authority.fail_claim();
    let (event_sender, event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let claim_result =
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir,
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 232,
            enqueue_trigger: None,
            planning,
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        });
    assert!(
        claim_result
            .outcome
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("dispatch command claim failed"))
    );
    assert!(
        event_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    assert_eq!(worker_port.launch_count(), 0);
}

#[test]
fn dispatch_tick_enqueue_trigger_claims_and_runs_new_command() {
    let repo = TempGitRepo::new("orchestrator-enqueue-trigger");
    let workspace_dir = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services(authority.clone());
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);

    let service = Arc::new(ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    ));
    let worker_port = Arc::new(CountingParallelAgentWorkerPort::default());
    let (event_sender, _event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let result =
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir.clone(),
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 24,
            enqueue_trigger: Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
            planning: planning.clone(),
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        });

    assert_eq!(result.outcome.launched_task_ids.len(), 1);
    for _ in 0..100 {
        if worker_port.launch_count() >= 1 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(worker_port.launch_count(), 1);

    let projections = authority
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(projections.dispatch_commands.len(), 1);
    assert_eq!(
        projections.dispatch_commands[0].state,
        ParallelModeDispatchCommandState::Completed
    );
}

#[test]
fn dispatch_orchestrator_logs_update_failures_after_successful_launch() {
    let repo = TempGitRepo::new("orchestrator-command-update-failure");
    let workspace_dir = repo.workspace_dir();
    let inner_authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let authority = Arc::new(FaultyPlanningAuthorityAdapter::new(inner_authority.clone()));
    let planning = build_test_planning_services(inner_authority);
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);

    let service = ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    let planning_projection = planning
        .runtime
        .load_runtime_projection_or_invalid(&workspace_dir);
    service
        .enqueue_dispatch_commands_for_event(
            &workspace_dir,
            ParallelModeRuntimeEvent::TaskIntakeCommitted,
            &planning_projection,
            Some(233),
        )
        .expect("dispatch command should enqueue before update failure");

    let service = Arc::new(service);
    let worker_port = Arc::new(CountingParallelAgentWorkerPort::default());
    let (event_sender, _event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    authority.fail_update();
    let result = with_akra_event_trace(|| {
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir,
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 233,
            enqueue_trigger: None,
            planning,
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        })
    });

    assert_eq!(result.outcome.launched_task_ids.len(), 1);
    for _ in 0..100 {
        if worker_port.launch_count() >= 1 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(worker_port.launch_count(), 1);
}

#[test]
fn dispatch_uses_task_identity_lease_when_agent_profiles_are_disabled() {
    let repo = TempGitRepo::new("orchestrator-disabled-agent-profiles");
    let workspace_dir = repo.workspace_dir();
    save_parallel_agent_profile_config(
        &workspace_dir,
        &ParallelAgentProfileConfig {
            profiles: vec![ParallelAgentProfile {
                agent_id: "disabled-agent".to_string(),
                display_name: "Disabled".to_string(),
                role: "Disabled".to_string(),
                persona_prompt: "This profile must not be selected.".to_string(),
                avatar_class: "Runner".to_string(),
                capabilities: Vec::new(),
                enabled: false,
            }],
        },
    )
    .expect("disabled agent profile config should be written");
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services(authority.clone());
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);
    let planning_projection = planning
        .runtime
        .load_runtime_projection_or_invalid(&workspace_dir);
    let queue_head = planning_projection
        .queue_head()
        .expect("ready task should become queue head");
    let fallback_lease_request = ParallelModeSlotLeaseRequest::from_task_identity(
        &queue_head.task_id,
        &queue_head.task_title,
    );

    let service = ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    service
        .enqueue_dispatch_commands_for_event(
            &workspace_dir,
            ParallelModeRuntimeEvent::TaskIntakeCommitted,
            &planning_projection,
            Some(25),
        )
        .expect("dispatch command should enqueue");

    let service = Arc::new(service);
    let (release_tx, release_rx) = mpsc::channel();
    let worker_port = Arc::new(HoldingParallelAgentWorkerPort::new(release_rx));
    let (event_sender, event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let result = with_akra_event_trace(|| {
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir.clone(),
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 25,
            enqueue_trigger: None,
            planning: planning.clone(),
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        })
    });

    assert_eq!(result.outcome.launched_task_ids.len(), 1);
    for _ in 0..100 {
        if worker_port.launch_count() >= 1 && !worker_port.launches().is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(worker_port.launch_count(), 1);
    let launch = worker_port
        .launches()
        .into_iter()
        .next()
        .expect("worker launch should be captured");
    let projections = authority
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    let lease = projections
        .slot_leases
        .values()
        .find(|lease| lease.worktree_path == launch.cwd)
        .expect("captured worker cwd should map to a held slot lease");
    assert_eq!(lease.agent_id, fallback_lease_request.agent_id);
    assert_ne!(lease.agent_id, "disabled-agent");
    assert_eq!(launch.service_name, "akra-parallel-worker");

    release_tx
        .send(())
        .expect("holding worker should be releasable");
    let worker_event = match event_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("released holding worker should report terminal event")
    {
        ParallelModeOrchestratorLoopEvent::WorkerEvent(event) => event,
        ParallelModeOrchestratorLoopEvent::ConversationRuntimeNotice(notice) => {
            panic!("unexpected runtime notice: {notice}")
        }
    };
    assert_eq!(
        worker_event.kind,
        ParallelModeControlPlaneWorkerEventKind::LaunchFailed
    );
}

#[test]
fn dispatch_orchestrator_reports_slot_lease_persistence_failures() {
    let repo = TempGitRepo::new("orchestrator-slot-lease-fails");
    let workspace_dir = repo.workspace_dir();
    let inner_authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let authority = Arc::new(FaultyPlanningAuthorityAdapter::new(inner_authority.clone()));
    let planning = build_test_planning_services(inner_authority);
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);

    let service = ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    let planning_projection = planning
        .runtime
        .load_runtime_projection_or_invalid(&workspace_dir);
    service
        .enqueue_dispatch_commands_for_event(
            &workspace_dir,
            ParallelModeRuntimeEvent::TaskIntakeCommitted,
            &planning_projection,
            Some(234),
        )
        .expect("dispatch command should enqueue");

    let service = Arc::new(service);
    let worker_port = Arc::new(CountingParallelAgentWorkerPort::default());
    let (event_sender, event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    authority.fail_slot_lease_upsert_after(0);
    let blocked = with_akra_event_trace(|| {
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir.clone(),
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 234,
            enqueue_trigger: None,
            planning: planning.clone(),
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        })
    });

    assert!(blocked.outcome.launched_task_ids.is_empty());
    assert!(
        blocked
            .outcome
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("worker launch blocked"))
    );
    assert!(
        event_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    assert_eq!(worker_port.launch_count(), 0);
}

#[test]
fn dispatch_orchestrator_keeps_partial_launch_when_later_slot_lease_fails() {
    let repo = TempGitRepo::new("orchestrator-partial-slot-lease-failure");
    let workspace_dir = repo.workspace_dir();
    let inner_authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let authority = Arc::new(FaultyPlanningAuthorityAdapter::new(inner_authority.clone()));
    let planning = build_test_planning_services(inner_authority);
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_tasks(&planning, &workspace_dir, 2);

    let service = ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    let planning_projection = planning
        .runtime
        .load_runtime_projection_or_invalid(&workspace_dir);
    service
        .enqueue_dispatch_commands_for_event(
            &workspace_dir,
            ParallelModeRuntimeEvent::TaskIntakeCommitted,
            &planning_projection,
            Some(235),
        )
        .expect("dispatch command should enqueue");

    let service = Arc::new(service);
    let worker_port = Arc::new(CountingParallelAgentWorkerPort::default());
    let (event_sender, _event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    authority.fail_slot_lease_upsert_after(1);
    let partial = with_akra_event_trace(|| {
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir,
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 235,
            enqueue_trigger: None,
            planning,
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        })
    });

    assert_eq!(partial.outcome.launched_task_ids.len(), 1);
    assert!(partial.outcome.status_copy_input.contains("blocked:"));
    for _ in 0..100 {
        if worker_port.launch_count() >= 1 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(worker_port.launch_count(), 1);
}

#[test]
fn dispatch_orchestrator_loop_claims_one_durable_command_across_two_ticks() {
    let repo = TempGitRepo::new("orchestrator-loop-claim-once");
    let workspace_dir = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services(authority.clone());
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);

    let service = ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    let planning_projection = planning
        .runtime
        .load_runtime_projection_or_invalid(&workspace_dir);
    assert!(planning_projection.has_actionable_queue_head());
    assert_eq!(
        service
            .enqueue_dispatch_commands_for_event(
                &workspace_dir,
                ParallelModeRuntimeEvent::TaskIntakeCommitted,
                &planning_projection,
                Some(1),
            )
            .expect("dispatch command should enqueue"),
        1
    );

    let service = Arc::new(service);
    let worker_port = Arc::new(CountingParallelAgentWorkerPort::default());
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let service = service.clone();
        let planning = planning.clone();
        let worker_port = worker_port.clone();
        let workspace_dir = workspace_dir.clone();
        let barrier = barrier.clone();
        handles.push(thread::spawn(move || {
            let (event_sender, _event_receiver) =
                mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
            barrier.wait();
            service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
                workspace_directory: workspace_dir,
                trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
                epoch_id: 1,
                enqueue_trigger: None,
                planning,
                worker_port,
                turn_service: ParallelModeTurnService::new((*service).clone()),
                event_sender,
            })
        }));
    }
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("loop tick should not panic"))
        .collect::<Vec<_>>();

    assert_eq!(
        results
            .iter()
            .map(|result| result.outcome.launched_task_ids.len())
            .sum::<usize>(),
        1
    );
    for _ in 0..100 {
        if worker_port.launch_count() >= 1 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(worker_port.launch_count(), 1);

    let projections = authority
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(projections.dispatch_commands.len(), 1);
    assert_eq!(
        projections.dispatch_commands[0].state,
        ParallelModeDispatchCommandState::Completed
    );
}

#[test]
fn slot_capacity_available_event_dispatches_next_ready_task_when_only_one_slot_is_idle() {
    let repo = TempGitRepo::new("orchestrator-capacity-continuation");
    let workspace_dir = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services(authority.clone());
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_tasks(&planning, &workspace_dir, DEFAULT_POOL_SIZE + 1);

    let service = ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    service
        .acquire_slot_lease(
            &workspace_dir,
            sample_lease_request(
                "already-running-1",
                "Already Running 1",
                "agent-already-running-1",
                "already-running-1",
            ),
        )
        .expect("first occupied slot should be leased");
    service
        .acquire_slot_lease(
            &workspace_dir,
            sample_lease_request(
                "already-running-2",
                "Already Running 2",
                "agent-already-running-2",
                "already-running-2",
            ),
        )
        .expect("second occupied slot should be leased");

    let planning_projection = planning
        .runtime
        .load_runtime_projection_or_invalid(&workspace_dir);
    assert!(planning_projection.has_actionable_queue_head());
    assert_eq!(
        service
            .enqueue_dispatch_commands_for_event(
                &workspace_dir,
                ParallelModeRuntimeEvent::SlotCapacityAvailable,
                &planning_projection,
                Some(9),
            )
            .expect("capacity event should enqueue dispatch command"),
        1
    );

    let service = Arc::new(service);
    let worker_port = Arc::new(CountingParallelAgentWorkerPort::default());
    let (event_sender, _event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let result =
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir.clone(),
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 9,
            enqueue_trigger: None,
            planning: planning.clone(),
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        });

    assert_eq!(result.outcome.idle_slot_count, 1);
    assert_eq!(result.outcome.launched_task_ids.len(), 1);
    for _ in 0..100 {
        if worker_port.launch_count() >= 1 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(worker_port.launch_count(), 1);

    let projections = authority
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(projections.dispatch_commands.len(), 1);
    assert_eq!(
        projections.dispatch_commands[0].state,
        ParallelModeDispatchCommandState::Completed
    );
}

#[test]
fn dispatch_orchestrator_marks_durable_command_blocked_when_all_slots_are_busy() {
    let repo = TempGitRepo::new("orchestrator-no-idle-slot");
    let workspace_dir = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services(authority.clone());
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);

    let service = ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        service
            .acquire_slot_lease(
                &workspace_dir,
                sample_lease_request(
                    &format!("busy-task-{slot_number}"),
                    &format!("Busy Task {slot_number}"),
                    &format!("busy-agent-{slot_number}"),
                    &format!("busy-task-{slot_number}"),
                ),
            )
            .expect("busy slot should be leased");
    }
    let planning_projection = planning
        .runtime
        .load_runtime_projection_or_invalid(&workspace_dir);
    service
        .enqueue_dispatch_commands_for_event(
            &workspace_dir,
            ParallelModeRuntimeEvent::TaskIntakeCommitted,
            &planning_projection,
            Some(12),
        )
        .expect("dispatch command should enqueue");

    let service = Arc::new(service);
    let worker_port = Arc::new(CountingParallelAgentWorkerPort::default());
    let (event_sender, event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let result = with_akra_event_trace(|| {
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir.clone(),
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 12,
            enqueue_trigger: None,
            planning: planning.clone(),
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        })
    });

    assert_eq!(result.outcome.idle_slot_count, 0);
    assert!(result.outcome.launched_task_ids.is_empty());
    assert_eq!(
        result.outcome.blocked_reason.as_deref(),
        Some("no idle slot is available for auto dispatch")
    );
    assert!(
        event_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    assert_eq!(worker_port.launch_count(), 0);

    let projections = authority
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(
        projections.dispatch_commands[0].state,
        ParallelModeDispatchCommandState::Blocked
    );
    assert!(
        projections.dispatch_commands[0]
            .status_detail
            .as_deref()
            .expect("blocked command should keep status detail")
            .contains("no idle slot is available")
    );
}

#[test]
fn dispatch_orchestrator_marks_stale_command_blocked_when_queue_has_no_candidates() {
    let repo = TempGitRepo::new("orchestrator-no-candidate");
    let workspace_dir = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services(authority.clone());
    bootstrap_planning_workspace(&planning, &workspace_dir);
    let command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        Some("stale-head-signature".to_string()),
        Some(13),
        "2026-05-12T00:00:00Z",
    );
    authority
        .enqueue_runtime_dispatch_command(&workspace_dir, &command)
        .expect("stale durable command should persist");

    let service = Arc::new(ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    ));
    let worker_port = Arc::new(CountingParallelAgentWorkerPort::default());
    let (event_sender, event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let result =
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir.clone(),
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 13,
            enqueue_trigger: None,
            planning: planning.clone(),
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        });

    assert!(result.outcome.launched_task_ids.is_empty());
    assert_eq!(
        result.outcome.blocked_reason.as_deref(),
        Some("no actionable queue task to auto dispatch")
    );
    assert!(
        event_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    assert_eq!(worker_port.launch_count(), 0);

    let projections = authority
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(
        projections.dispatch_commands[0].state,
        ParallelModeDispatchCommandState::Blocked
    );
}

#[test]
fn dispatch_orchestrator_reports_excluded_leased_task_when_idle_slots_remain() {
    let repo = TempGitRepo::new("orchestrator-excluded-leased-task");
    let workspace_dir = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services(authority.clone());
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);
    let planning_projection = planning
        .runtime
        .load_runtime_projection_or_invalid(&workspace_dir);
    let queue_head = planning_projection
        .queue_head()
        .expect("ready task should become queue head");

    let service = ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    service
        .acquire_slot_lease(
            &workspace_dir,
            ParallelModeSlotLeaseRequest::from_task_identity_with_agent_id(
                &queue_head.task_id,
                &queue_head.task_title,
                "agent-existing",
            ),
        )
        .expect("same queue task should already be leased");
    service
        .enqueue_dispatch_commands_for_event(
            &workspace_dir,
            ParallelModeRuntimeEvent::TaskIntakeCommitted,
            &planning_projection,
            Some(15),
        )
        .expect("dispatch command should enqueue");

    let service = Arc::new(service);
    let worker_port = Arc::new(CountingParallelAgentWorkerPort::default());
    let (event_sender, event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let result = with_akra_event_trace(|| {
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir.clone(),
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 15,
            enqueue_trigger: None,
            planning: planning.clone(),
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        })
    });

    assert!(result.outcome.launched_task_ids.is_empty());
    assert!(
        result
            .outcome
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason
                .contains("no undispatched queue task available for auto dispatch / excluded:"))
    );
    assert_eq!(worker_port.launch_count(), 0);
    assert!(
        event_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );

    let projections = authority
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(
        projections.dispatch_commands[0].state,
        ParallelModeDispatchCommandState::Blocked
    );
    assert!(
        projections.dispatch_commands[0]
            .status_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("excluded:"))
    );
}

#[test]
fn completed_worker_stream_records_official_completion_and_sends_worker_event() {
    let repo = TempGitRepo::new("orchestrator-worker-completed");
    let workspace_dir = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services(authority.clone());
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);

    let service = Arc::new(ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    ));
    let planning_projection = planning
        .runtime
        .load_runtime_projection_or_invalid(&workspace_dir);
    service
        .enqueue_dispatch_commands_for_event(
            &workspace_dir,
            ParallelModeRuntimeEvent::TaskIntakeCommitted,
            &planning_projection,
            Some(14),
        )
        .expect("dispatch command should enqueue");

    let worker_port = Arc::new(CompletingParallelAgentWorkerPort::default());
    let (event_sender, event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let result =
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir.clone(),
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 14,
            enqueue_trigger: None,
            planning: planning.clone(),
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        });

    assert_eq!(result.outcome.launched_task_ids.len(), 1);
    let worker_event = match event_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("worker completion event should arrive")
    {
        ParallelModeOrchestratorLoopEvent::WorkerEvent(event) => event,
        ParallelModeOrchestratorLoopEvent::ConversationRuntimeNotice(notice) => {
            panic!("unexpected runtime notice: {notice}")
        }
    };
    assert_eq!(
        worker_event.kind,
        ParallelModeControlPlaneWorkerEventKind::Completed
    );
    assert_eq!(worker_event.epoch_id, 14);
    assert_eq!(worker_port.launch_count(), 1);
    assert!(
        worker_event
            .notices
            .iter()
            .any(|notice| notice.contains("commit-ready result entered the distributor queue"))
    );
}

#[test]
fn completed_worker_stream_reports_failed_official_completion_refresh() {
    let repo = TempGitRepo::new("orchestrator-official-refresh-failed");
    let workspace_dir = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services_with_worker(
        authority.clone(),
        Arc::new(FailingPlanningWorkerPort),
    );
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);

    let service = Arc::new(ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    ));
    let planning_projection = planning
        .runtime
        .load_runtime_projection_or_invalid(&workspace_dir);
    service
        .enqueue_dispatch_commands_for_event(
            &workspace_dir,
            ParallelModeRuntimeEvent::TaskIntakeCommitted,
            &planning_projection,
            Some(16),
        )
        .expect("dispatch command should enqueue");

    let worker_port = Arc::new(CompletingParallelAgentWorkerPort::default());
    let (event_sender, event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let result = with_akra_event_trace(|| {
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir.clone(),
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 16,
            enqueue_trigger: None,
            planning: planning.clone(),
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        })
    });

    assert_eq!(result.outcome.launched_task_ids.len(), 1);
    let worker_event = match event_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("worker failure event should arrive")
    {
        ParallelModeOrchestratorLoopEvent::WorkerEvent(event) => event,
        ParallelModeOrchestratorLoopEvent::ConversationRuntimeNotice(notice) => {
            panic!("unexpected runtime notice: {notice}")
        }
    };
    assert_eq!(
        worker_event.kind,
        ParallelModeControlPlaneWorkerEventKind::StreamFailed
    );
    assert_eq!(worker_event.epoch_id, 16);
    assert_eq!(worker_port.launch_count(), 1);
    assert!(
        worker_event
            .notices
            .iter()
            .any(|notice| notice.contains("parallel official completion refresh failed")),
        "notices: {:?}",
        worker_event.notices
    );
}

#[test]
fn completed_worker_stream_reports_repair_request_as_stream_failure() {
    let repo = TempGitRepo::new("orchestrator-official-refresh-repair-request");
    let workspace_dir = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = build_test_planning_services_with_worker(
        authority.clone(),
        Arc::new(RepairRequestPlanningWorkerPort),
    );
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);

    let service = Arc::new(ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    ));
    let planning_projection = planning
        .runtime
        .load_runtime_projection_or_invalid(&workspace_dir);
    service
        .enqueue_dispatch_commands_for_event(
            &workspace_dir,
            ParallelModeRuntimeEvent::TaskIntakeCommitted,
            &planning_projection,
            Some(236),
        )
        .expect("dispatch command should enqueue");

    let worker_port = Arc::new(CompletingParallelAgentWorkerPort::default());
    let (event_sender, event_receiver) = mpsc::channel::<ParallelModeOrchestratorLoopEvent>();
    let result =
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir,
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 236,
            enqueue_trigger: None,
            planning,
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
        });

    assert_eq!(result.outcome.launched_task_ids.len(), 1);
    let worker_event = match event_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("worker repair event should arrive")
    {
        ParallelModeOrchestratorLoopEvent::WorkerEvent(event) => event,
        ParallelModeOrchestratorLoopEvent::ConversationRuntimeNotice(notice) => {
            panic!("unexpected runtime notice: {notice}")
        }
    };
    assert_eq!(
        worker_event.kind,
        ParallelModeControlPlaneWorkerEventKind::StreamFailed
    );
    assert_eq!(worker_event.epoch_id, 236);
    assert!(
        worker_event
            .notices
            .iter()
            .any(|notice| notice.contains("parallel official completion refresh blocked")),
        "notices: {:?}",
        worker_event.notices
    );
}

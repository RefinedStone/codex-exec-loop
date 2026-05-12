use super::*;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::parallel_agent_worker_port::{
    ParallelAgentWorkerPort, ParallelAgentWorkerStreamRequest,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::parallel_mode::{
    ParallelModeDispatchOrchestratorTickRequest, ParallelModeOrchestratorLoopEvent,
};
use crate::application::service::planning::{PlanningServices, PlanningTaskIntakeRequest};
use crate::domain::parallel_mode::{
    ParallelModeAutomationTrigger, ParallelModeControlPlaneWorkerEventKind,
    ParallelModeDispatchCommandSnapshot, ParallelModeDispatchCommandState,
    ParallelModeRuntimeEvent,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, mpsc};
use std::thread;
use std::time::Duration;

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
    PlanningServices::from_ports(
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        authority.clone(),
        authority,
        Arc::new(NoopPlanningWorkerPort),
    )
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
    let result =
        service.run_dispatch_orchestrator_tick(ParallelModeDispatchOrchestratorTickRequest {
            workspace_directory: workspace_dir.clone(),
            trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id: 12,
            enqueue_trigger: None,
            planning: planning.clone(),
            worker_port: worker_port.clone(),
            turn_service: ParallelModeTurnService::new((*service).clone()),
            event_sender,
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

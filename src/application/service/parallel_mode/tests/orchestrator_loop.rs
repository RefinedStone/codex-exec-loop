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
    ParallelModeAutomationTrigger, ParallelModeDispatchCommandState, ParallelModeRuntimeEvent,
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

fn commit_ready_queue_task(planning: &PlanningServices, workspace_dir: &str) {
    let proposal = planning
        .runtime
        .prepare_task_intake(PlanningTaskIntakeRequest {
            workspace_directory: workspace_dir.to_string(),
            raw_prompt: "Only one loop should claim this dispatch task".to_string(),
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

#[test]
fn dispatch_orchestrator_loop_claims_one_durable_command_across_two_ticks() {
    let repo = TempGitRepo::new("orchestrator-loop-claim-once");
    let workspace_dir = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = PlanningServices::from_ports(
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        authority.clone(),
        authority.clone(),
        Arc::new(NoopPlanningWorkerPort),
    );
    bootstrap_planning_workspace(&planning, &workspace_dir);
    commit_ready_queue_task(&planning, &workspace_dir);

    let service = ParallelModeService::new(
        authority.clone(),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    let planning_snapshot = planning
        .runtime
        .load_runtime_snapshot_or_invalid(&workspace_dir);
    assert!(planning_snapshot.has_actionable_queue_head());
    assert_eq!(
        service
            .enqueue_dispatch_commands_for_event(
                &workspace_dir,
                ParallelModeRuntimeEvent::TaskIntakeCommitted,
                &planning_snapshot,
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

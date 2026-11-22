use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
    PriorityQueueService, QueueIdleConfig, QueueIdlePolicy, TaskActor, TaskAuthorityDocument,
    TaskDefinition, TaskMutationProvenance, TaskStatus,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct AwaitingUserWorker {
    authority: Arc<SqlitePlanningAuthorityAdapter>,
    workspace: String,
    inner: ScriptedParallelAgentWorkerPort,
    archive_calls: Arc<AtomicUsize>,
}

impl ParallelAgentWorkerPort for AwaitingUserWorker {
    fn run_isolated_new_thread_stream(
        &self,
        request: ParallelAgentWorkerStreamRequest<'_>,
        sender: crate::application::port::conversation_stream::ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        let snapshot = self
            .authority
            .load_task_authority_snapshot(&self.workspace)?
            .unwrap();
        let mut tasks = snapshot.task_authority.clone();
        tasks.tasks[0].status = TaskStatus::AwaitingUser;
        tasks.tasks[0].updated_at = chrono::Utc::now().to_rfc3339();
        let directions = self
            .authority
            .load_direction_authority_snapshot(&self.workspace)?
            .unwrap();
        let queue = PriorityQueueService::new().build_projection(&directions.directions, &tasks)?;
        self.authority.commit_task_authority_snapshot(
            &self.workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: Some(snapshot.planning_revision),
                task_authority: &tasks,
                queue_projection: &queue,
            },
        )?;
        self.inner.run_isolated_new_thread_stream(request, sender)
    }
    fn archive_completed_isolated_thread(&self, _cwd: &str, _thread_id: &str) -> Result<()> {
        self.archive_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn awaiting_user_terminal_preserves_thread_lease_and_blocks_official_completion_and_delivery() {
    let workspace = TempGitWorkspace::new("parallel-awaiting-user-terminal");
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = test_planning_services(authority.clone());
    planning
        .workspace
        .initialize_simple_workspace(workspace.path())
        .unwrap();
    let directions = DirectionCatalogDocument {
        version: 1,
        queue_idle: QueueIdleConfig {
            policy: QueueIdlePolicy::Stop,
            prompt_path: String::new(),
        },
        directions: vec![DirectionDefinition {
            id: "direction-a".into(),
            title: "Test".into(),
            summary: "Test".into(),
            success_criteria: vec!["wait for the user".into()],
            scope_hints: vec![],
            detail_doc_path: String::new(),
            state: DirectionState::Active,
        }],
    };
    authority
        .commit_direction_authority_snapshot(
            workspace.path(),
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                directions: &directions,
                authority_mutation_owner_token: None,
            },
        )
        .unwrap();
    let tasks = TaskAuthorityDocument {
        version: 1,
        tasks: vec![TaskDefinition {
            id: "task-a".into(),
            direction_id: "direction-a".into(),
            direction_relation_note: "fixture".into(),
            title: "Waiting task".into(),
            description: "Synthetic question".into(),
            status: TaskStatus::InProgress,
            base_priority: 50,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: vec![],
            blocked_by: vec![],
            created_by: TaskActor::System,
            last_updated_by: TaskActor::System,
            source_turn_id: None,
            provenance: TaskMutationProvenance::new(OriginSessionKind::System),
            updated_at: "2026-11-21T00:00:00Z".into(),
        }],
    };
    let queue = PriorityQueueService::new()
        .build_projection(&directions, &tasks)
        .unwrap();
    authority
        .commit_task_authority_snapshot(
            workspace.path(),
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &tasks,
                queue_projection: &queue,
            },
        )
        .unwrap();
    run_git(Path::new(workspace.path()), &["add", "."]);
    run_git(
        Path::new(workspace.path()),
        &["commit", "-qm", "initialize waiting fixture"],
    );
    run_git(
        Path::new(workspace.path()),
        &["push", "-q", "origin", "prerelease"],
    );
    let parallel = test_parallel_service(authority.clone());
    parallel
        .reset_pool_on_parallel_initial_setup_report(workspace.path())
        .unwrap();
    let lease = parallel
        .acquire_slot_lease(
            workspace.path(),
            ParallelModeSlotLeaseRequest::from_task_identity("task-a", "Waiting task"),
        )
        .unwrap();
    fs::write(
        Path::new(&lease.worktree_path).join("waiting-proof.txt"),
        "keep this user-visible work\n",
    )
    .unwrap();
    let archive_calls = Arc::new(AtomicUsize::new(0));
    let worker = Arc::new(AwaitingUserWorker {
        authority: authority.clone(),
        workspace: workspace.path().into(),
        archive_calls: archive_calls.clone(),
        inner: ScriptedParallelAgentWorkerPort::new(
            vec![
                ConversationStreamEvent::ThreadPrepared {
                    thread_id: "thread-scripted".into(),
                    title: "Waiting task".into(),
                    cwd: lease.worktree_path.clone(),
                    runtime_envelope: Box::default(),
                },
                ConversationStreamEvent::TurnStarted {
                    turn_id: "turn-awaiting".into(),
                    runtime_request: Box::default(),
                },
                ConversationStreamEvent::TurnTerminal {
                    receipt: completed_receipt("turn-awaiting", vec![]),
                },
            ],
            WorkerExit::Ok,
        ),
    });
    let mut request = worker_request_with_secret_bodies();
    request.planning_workspace_directory = workspace.path().into();
    request.worktree_directory = lease.worktree_path.clone();
    request.expected_lease = lease.clone();
    let outcome = run_parallel_dispatch_worker(
        request,
        worker,
        ParallelModeTurnService::new(parallel.clone()),
        planning,
    );
    assert_eq!(
        outcome.worker_event_kind,
        ParallelModeControlPlaneWorkerEventKind::Completed,
        "waiting is a retained session, not a completion failure: {:?}",
        outcome.notices
    );
    assert_eq!(archive_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        authority
            .load_task_authority_snapshot(workspace.path())
            .unwrap()
            .unwrap()
            .task_authority
            .tasks[0]
            .status,
        TaskStatus::AwaitingUser
    );
    let projection = authority
        .load_runtime_projections(workspace.path())
        .unwrap();
    let retained = projection.slot_leases.get(&lease.slot_id).unwrap();
    assert!(retained.same_generation_as(&lease));
    assert_eq!(retained.state, ParallelModeSlotLeaseState::Running);
    let detail = projection
        .session_details
        .iter()
        .find(|detail| detail.session_key == lease.session_key())
        .unwrap();
    assert_eq!(detail.thread_id.as_deref(), Some("thread-scripted"));
    assert_eq!(detail.completion_state_label, "awaiting_user");
    assert!(projection.distributor_queue_records.is_empty());
    assert!(
        parallel
            .mark_workspace_commit_ready_for_lease(retained, "must not complete")
            .unwrap()
            .is_none()
    );
    assert!(
        parallel
            .enqueue_workspace_commit_ready_result_for_lease(retained)
            .unwrap()
            .is_none()
    );
    assert!(
        parallel
            .mark_workspace_official_completion_failed_for_lease(
                retained,
                "late completion failure"
            )
            .unwrap()
            .is_none()
    );
    assert!(
        parallel
            .mark_slot_cleanup_pending(workspace.path(), &lease.slot_id, &lease.agent_id)
            .is_err()
    );
    assert!(
        !authority
            .work_order_cleanup_allowed_for_task(workspace.path(), "task-a")
            .unwrap()
    );
    assert!(
        authority
            .worker_worktree_requires_preservation(workspace.path(), &lease.worktree_path)
            .unwrap()
    );
    assert_eq!(
        fs::read_to_string(Path::new(&lease.worktree_path).join("waiting-proof.txt")).unwrap(),
        "keep this user-visible work\n"
    );
}

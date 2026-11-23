//! Real lease/Git/SQLite boundaries with an inert verifier and a fake planning worker.
use super::*;
use crate::adapter::outbound::filesystem::{
    FilesystemPlanningWorkspaceAdapter, FilesystemTaskEvidenceArtifactAdapter,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::PlanningDirectionAuthorityCommit;
use crate::application::port::outbound::planning_worker_port::{
    PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::port::outbound::task_evidence_repository_port::{
    TaskEvidenceRepositoryPort, TaskEvidenceSnapshotQuery,
};
use crate::application::port::outbound::task_verification_runner_port::{
    PreparedTaskVerification, TaskVerificationPrepareRequest, TaskVerificationRunOutcome,
    TaskVerificationRunnerPort,
};
use crate::application::service::parallel_mode::resolve_workspace_head_sha;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::PlanningServices;
use crate::application::service::task_verification::{
    TaskVerificationDependencies, TaskVerificationHandle, TaskVerificationService, VerificationJob,
};
use crate::domain::planning::verification::{
    TaskVerificationExecutionManifest, TaskVerificationExecutionOutcome,
    TaskVerificationNotRunReason, TaskVerificationObservation, TaskVerificationReceipt,
    TaskVerificationStream,
};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

mod repository;
pub(crate) use repository::FinishRepository;

pub(crate) const TASK: &str = "task-automatic-verification";
const AT: &str = "2026-11-23T12:00:00Z";
// Real Git, private filesystem and SQLite work can be slow on clean native CI.
// This bounds fixture progress only; product deadlines and cancellation checks are separate.
pub(crate) const REAL_GIT_WAIT: Duration = Duration::from_secs(120);

pub(crate) struct Fixture {
    repo: TempGitRepo,
    pub service: ParallelModeService,
    pub authority: Arc<SqlitePlanningAuthorityAdapter>,
    pub host: Arc<TaskVerificationService>,
    pub repository: Arc<FinishRepository>,
    pub lease: ParallelModeSlotLeaseSnapshot,
    pub captured: ParallelModeSlotLeaseSnapshot,
    pub planning: PlanningServices,
    pub worker: Arc<ReceiptCheckingWorker>,
    pub runner: Arc<FakeRunner>,
    jobs: Arc<Mutex<Vec<VerificationJob>>>,
}

impl Fixture {
    pub(crate) fn new(label: &str) -> Self {
        let repo = TempGitRepo::new(label);
        let workspace = repo.workspace_dir();
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let worker = Arc::new(ReceiptCheckingWorker {
            workspace: workspace.clone(),
            calls: AtomicUsize::new(0),
        });
        // Use production's private repo-scoped planning boundary on every platform.
        // Windows deliberately rejects direct-filesystem planning mutations.
        let planning_workspace = Arc::new(
            FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(authority.clone()),
        );
        assert!(planning_workspace.uses_repo_scoped_authority(&workspace));
        let planning = PlanningServices::from_ports(
            planning_workspace,
            authority.clone(),
            authority.clone(),
            worker.clone(),
        );
        let draft = planning
            .workspace
            .stage_simple_mode_draft(&workspace)
            .unwrap();
        planning
            .workspace
            .promote_staged_draft(&workspace, &draft.draft_name)
            .unwrap();
        seed_task(&workspace, authority.as_ref());
        let artifacts = Arc::new(FilesystemTaskEvidenceArtifactAdapter::new(Arc::new(
            |workspace| {
                Ok(PathBuf::from(
                    SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?
                        .runtime_dir,
                ))
            },
        )));
        let handle = TaskVerificationHandle::default();
        let service = test_parallel_mode_service()
            .with_task_evidence(authority.clone(), artifacts.clone())
            .with_task_verification(handle.clone());
        let captured = service
            .acquire_slot_lease(
                &workspace,
                sample_lease_request(TASK, "Automatic verification", "worker-1", "auto-check"),
            )
            .unwrap();
        service
            .record_workspace_slot_thread_prepared_for_lease(&captured, "thread-scripted")
            .unwrap()
            .unwrap();
        let lease = service
            .mark_workspace_slot_running(&captured.worktree_path)
            .unwrap()
            .unwrap();
        assert_eq!(captured.state, ParallelModeSlotLeaseState::Leased);
        assert_eq!(lease.state, ParallelModeSlotLeaseState::Running);
        assert!(lease.same_generation_as(&captured));
        fs::write(
            Path::new(&lease.worktree_path).join("feature.txt"),
            "changed\n",
        )
        .unwrap();
        let repository = Arc::new(FinishRepository::new(authority.clone()));
        let jobs: Arc<Mutex<Vec<VerificationJob>>> = Arc::new(Mutex::new(Vec::new()));
        let pending = jobs.clone();
        let runner = Arc::new(FakeRunner::default());
        let host = Arc::new(
            TaskVerificationService::new(TaskVerificationDependencies {
                workspace,
                repository: repository.clone(),
                artifacts,
                runtime: Arc::new(test_parallel_runtime()),
                authority: authority.clone(),
                runner: runner.clone(),
                spawn: Arc::new(move |job, _interrupted| {
                    pending.lock().unwrap().push(job);
                    Ok(())
                }),
                wake: Arc::new(|| {}),
            })
            .unwrap(),
        );
        host.initialize_live_owner(AT).unwrap();
        handle.attach(&host).unwrap();
        Self {
            repo,
            service,
            authority,
            host,
            repository,
            lease,
            captured,
            planning,
            worker,
            runner,
            jobs,
        }
    }

    pub(crate) fn workspace(&self) -> String {
        self.repo.workspace_dir()
    }

    pub(crate) fn head(&self) -> String {
        resolve_workspace_head_sha(Path::new(&self.lease.worktree_path)).unwrap()
    }

    pub(crate) fn turn_service(&self) -> ParallelModeTurnService {
        ParallelModeTurnService::new(self.service.clone())
    }

    pub(crate) fn take_job(&self) -> VerificationJob {
        let deadline = Instant::now() + REAL_GIT_WAIT;
        loop {
            if let Some(job) = self.jobs.lock().unwrap().pop() {
                return job;
            }
            assert!(
                Instant::now() < deadline,
                "automatic verification did not reserve a job"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub(crate) fn next_refresh_order(&self) -> i64 {
        use rusqlite::OptionalExtension;
        let location = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
            &self.workspace(),
        )
        .unwrap();
        rusqlite::Connection::open(&location.authority_store_path)
            .unwrap()
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM authority_metadata WHERE key='next_official_refresh_order'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap()
            .unwrap_or(1)
    }

    pub(crate) fn receipt(&self) -> Option<TaskVerificationReceipt> {
        latest_receipt(&self.workspace())
    }

    pub(crate) fn session_detail_bytes(&self) -> Option<Vec<u8>> {
        fs::read(self.repo.session_detail_path(&self.lease.session_key())).ok()
    }

    pub(crate) fn assert_waiting_session(&self) {
        let tasks = self
            .authority
            .load_task_authority_snapshot(&self.workspace())
            .unwrap()
            .unwrap();
        assert_eq!(
            tasks
                .task_authority
                .tasks
                .iter()
                .find(|task| task.id == TASK)
                .unwrap()
                .status,
            TaskStatus::AwaitingUser,
        );
        let snapshot = self
            .authority
            .load_runtime_projections(&self.workspace())
            .unwrap();
        let detail = snapshot
            .session_details
            .iter()
            .find(|detail| detail.session_key == self.lease.session_key())
            .expect("the preserved worker has a session detail");
        assert_eq!(detail.state_label, "awaiting_user");
        assert_eq!(detail.completion_state_label, "awaiting_user");
        assert_eq!(detail.thread_id.as_deref(), Some("thread-scripted"));
        let lease = snapshot.slot_leases.get(&self.lease.slot_id).unwrap();
        assert!(lease.same_generation_as(&self.captured));
        assert_eq!(lease.state, ParallelModeSlotLeaseState::Running);
    }

    pub(crate) fn record_failed_automatic_attempt(&self, attempt: usize) {
        use crate::application::service::task_verification::AutomaticTaskVerificationOutcome;
        self.runner.exit_code.store(2, Ordering::SeqCst);
        fs::write(
            Path::new(&self.lease.worktree_path).join("feature.txt"),
            format!("changed attempt {attempt}\n"),
        )
        .unwrap();
        let turn = self.turn_service();
        let workspace = self.workspace();
        let captured = self.captured.clone();
        let completion = std::thread::spawn(move || {
            let continuation = turn.automatic_completion_continuation(&workspace, None, None);
            turn.prepare_automatic_verification_for_official_completion(
                &workspace,
                &captured,
                &format!("automatic-failed-{attempt}"),
                continuation,
            )
        });
        self.take_job()();
        assert!(self.host.poll(Instant::now() + Duration::from_secs(2)));
        assert!(matches!(
            self.join(completion).unwrap(),
            AutomaticTaskVerificationOutcome::Finished { .. }
        ));
        assert_eq!(self.next_refresh_order(), 1);
    }

    pub(crate) fn join<T>(&self, worker: std::thread::JoinHandle<T>) -> T {
        let deadline = Instant::now() + REAL_GIT_WAIT;
        while !worker.is_finished() {
            assert!(
                Instant::now() < deadline,
                "completion did not settle after host persistence"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        worker
            .join()
            .unwrap_or_else(|_| panic!("completion worker panicked"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.host.shutdown();
    }
}

fn latest_receipt(workspace: &str) -> Option<TaskVerificationReceipt> {
    SqlitePlanningAuthorityAdapter::new()
        .load_current_snapshot(workspace, &TaskEvidenceSnapshotQuery::default())
        .unwrap()
        .tasks
        .into_iter()
        .find(|task| task.task.id == TASK)
        .and_then(|task| task.host_verifications.into_iter().next())
        .and_then(|stored| stored.receipt)
}

pub(crate) struct ReceiptCheckingWorker {
    workspace: String,
    pub calls: AtomicUsize,
}

impl PlanningWorkerPort for ReceiptCheckingWorker {
    fn run_planning_session(
        &self,
        _request: PlanningWorkerRequest,
    ) -> anyhow::Result<PlanningWorkerResponse> {
        let receipt =
            latest_receipt(&self.workspace).expect("refresh requires a durable host receipt");
        receipt.validate().unwrap();
        self.calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("fake planning worker stops after checking durable handoff")
    }
}

#[derive(Default)]
pub(crate) struct FakeRunner {
    pub runs: Arc<AtomicUsize>,
    pub exit_code: AtomicI32,
}

struct Prepared {
    manifest: TaskVerificationExecutionManifest,
    runs: Arc<AtomicUsize>,
    exit_code: i32,
}

impl TaskVerificationRunnerPort for FakeRunner {
    fn prepare(
        &self,
        request: TaskVerificationPrepareRequest<'_>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Box<dyn PreparedTaskVerification>, TaskVerificationNotRunReason> {
        if cancelled() {
            return Err(TaskVerificationNotRunReason::Cancelled);
        }
        assert!(
            request.source.worktree_clean,
            "host commit precedes verification"
        );
        Ok(Box::new(Prepared {
            manifest: TaskVerificationExecutionManifest {
                recipe: request.recipe,
                base_commit_sha: request.base_commit_sha.into(),
                base_tree_sha: test_parallel_runtime()
                    .read_commit_tree(request.workspace_directory, request.base_commit_sha)
                    .map_err(|_| TaskVerificationNotRunReason::Unavailable)?,
                source: request.source.clone(),
                git_binary_sha256: "a".repeat(64),
                command_sha256: "b".repeat(64),
                environment_sha256: "c".repeat(64),
                configuration_sha256: "d".repeat(64),
            },
            runs: self.runs.clone(),
            exit_code: self.exit_code.load(Ordering::SeqCst),
        }))
    }

    fn environment_fingerprint(
        &self,
        _manifest: &TaskVerificationExecutionManifest,
    ) -> Result<String, TaskVerificationNotRunReason> {
        Ok("c".repeat(64))
    }
}

impl PreparedTaskVerification for Prepared {
    fn manifest(&self) -> &TaskVerificationExecutionManifest {
        &self.manifest
    }

    fn run(self: Box<Self>, cancelled: &dyn Fn() -> bool) -> TaskVerificationRunOutcome {
        if cancelled() {
            return TaskVerificationRunOutcome::NotRun {
                reason: TaskVerificationNotRunReason::Cancelled,
            };
        }
        self.runs.fetch_add(1, Ordering::SeqCst);
        let stdout = b"fixed host fixture result\n".to_vec();
        let stream = |bytes: &[u8]| TaskVerificationStream {
            sha256: format!("{:x}", Sha256::digest(bytes)),
            size_bytes: bytes.len() as u64,
            complete: true,
        };
        TaskVerificationRunOutcome::Executed {
            observation: Box::new(TaskVerificationObservation {
                outcome: TaskVerificationExecutionOutcome::Exited {
                    exit_code: self.exit_code,
                },
                started_at: AT.into(),
                finished_at: "2026-11-23T12:00:00.010Z".into(),
                duration_ms: 10,
                stdout: stream(&stdout),
                stderr: stream(&[]),
            }),
            stdout,
            stderr: Vec::new(),
        }
    }
}

fn seed_task(workspace: &str, authority: &SqlitePlanningAuthorityAdapter) {
    let directions = serde_json::from_value(serde_json::json!({
        "version":1, "queue_idle":{}, "directions":[{
            "id":"automatic", "title":"Automatic host fixture", "summary":"Verify before refresh",
            "success_criteria":["A durable host fact precedes refresh"], "scope_hints":[],
            "detail_doc_path":"", "state":"active"
        }]
    }))
    .unwrap();
    authority
        .commit_direction_authority_snapshot(
            workspace,
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                authority_mutation_owner_token: None,
                directions: &directions,
            },
        )
        .unwrap();
    let task: TaskDefinition = serde_json::from_value(serde_json::json!({
        "id":TASK, "direction_id":"automatic", "title":"Automatic verification",
        "description":"The explicitly selected recipe runs after host commit",
        "acceptance":[{"id":"C1", "scenario":"The fixed host recipe is recorded", "channel":"command",
            "essential":true, "verification_recipe":"git_diff_check_v1"}],
        "acceptance_revision":1, "status":"ready", "base_priority":50, "depends_on":[],
        "blocked_by":[], "created_by":"user", "last_updated_by":"user", "updated_at":AT
    })).unwrap();
    authority
        .commit_task_authority_snapshot(
            workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &TaskAuthorityDocument {
                    version: crate::domain::planning::TASK_AUTHORITY_FORMAT_VERSION,
                    tasks: vec![task],
                },
                queue_projection: &PriorityQueueProjection {
                    next_task: None,
                    active_tasks: vec![],
                    proposed_tasks: vec![],
                    skipped_tasks: vec![],
                },
            },
        )
        .unwrap();
}

#[test]
fn automatic_verification_without_an_explicit_essential_recipe_keeps_an_unbound_owner_inert() {
    use crate::application::service::task_verification::AutomaticTaskVerificationOutcome;
    use crate::domain::planning::TaskAcceptanceChannel;
    use crate::domain::planning::verification::TaskVerificationRecipe;

    let fixture = Fixture::new("auto-no-recipe-owner");
    let turn = ParallelModeTurnService::new(
        fixture
            .service
            .clone()
            .with_task_verification(TaskVerificationHandle::default()),
    );
    let initial_head = fixture.head();
    for (essential, channel, recipe) in [
        (true, TaskAcceptanceChannel::Command, None),
        (true, TaskAcceptanceChannel::Data, None),
        (
            false,
            TaskAcceptanceChannel::Command,
            Some(TaskVerificationRecipe::GitDiffCheckV1),
        ),
    ] {
        let mut snapshot = fixture
            .authority
            .load_task_authority_snapshot(&fixture.workspace())
            .unwrap()
            .unwrap();
        let task = snapshot
            .task_authority
            .tasks
            .iter_mut()
            .find(|task| task.id == TASK)
            .unwrap();
        task.acceptance_revision += 1;
        task.acceptance[0].essential = essential;
        task.acceptance[0].channel = channel;
        task.acceptance[0].verification_recipe = recipe;
        fixture
            .authority
            .commit_task_authority_snapshot(
                &fixture.workspace(),
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: Some(snapshot.planning_revision),
                    task_authority: &snapshot.task_authority,
                    queue_projection: &snapshot.queue_projection,
                },
            )
            .unwrap();
        assert_eq!(
            turn.prepare_automatic_verification_for_official_completion(
                &fixture.workspace(),
                &fixture.captured,
                "ordinary-completion",
                turn.automatic_completion_continuation(&fixture.workspace(), None, None),
            )
            .unwrap(),
            AutomaticTaskVerificationOutcome::NotApplicable
        );
        assert_eq!(
            fixture.head(),
            initial_head,
            "ordinary completion does not gain a new host commit step"
        );
        assert_eq!(fixture.next_refresh_order(), 1);
        assert!(fixture.jobs.lock().unwrap().is_empty());
        assert!(fixture.receipt().is_none());
    }
}

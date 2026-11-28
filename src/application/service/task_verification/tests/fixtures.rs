use super::*;
use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkPermitDecision;
use crate::application::port::outbound::task_verification_runner_port::{
    PreparedTaskVerification, TaskVerificationPrepareRequest,
};
use crate::domain::parallel_mode::{
    ParallelModeDeliveryTargetSnapshot, ParallelModeRepositoryVisibility,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};
use crate::domain::planning::TaskDefinition;

pub(super) const AT: &str = "2026-11-23T12:00:00Z";
pub(super) type Scheduled = (VerificationJob, VerificationJob);

pub(crate) struct Fixture {
    pub service: Arc<TaskVerificationService>,
    pub evidence: Arc<crate::application::service::task_evidence::TaskEvidenceService>,
    pub repository: Arc<Repository>,
    pub jobs: Arc<Mutex<Vec<Scheduled>>>,
    pub spawn_calls: Arc<AtomicUsize>,
    pub spawn_fails: Arc<AtomicBool>,
    pub wakes: Arc<AtomicUsize>,
    pub permit: Arc<Mutex<PlanningWorkPermitDecision>>,
    pub runner: Arc<Runner>,
}

impl Fixture {
    pub fn new() -> Self {
        Self::configured("/repo", None)
    }

    pub(super) fn with_issue_binding(
        workspace: &str,
        admission: Arc<crate::application::service::issue_binding_admission::IssueBindingAdmission>,
        store: Arc<dyn crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort>,
        work_order_id: &str,
    ) -> Self {
        Self::configured(workspace, Some((
            super::super::super::parallel_mode::binding_action::IssueBindingActionConfiguration { admission, store },
            crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkerActionIdentity {
                work_order_id: work_order_id.to_owned(),
                origin: "issue_automation".into(),
                authorization_epoch: 0,
            },
        )))
    }

    fn configured(
        workspace: &str,
        binding: Option<(
            crate::application::service::parallel_mode::binding_action::IssueBindingActionConfiguration,
            crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkerActionIdentity,
        )>,
    ) -> Self {
        let repository = Arc::new(Repository::new());
        let jobs = Arc::new(Mutex::new(Vec::new()));
        let spawn_calls = Arc::new(AtomicUsize::new(0));
        let spawn_fails = Arc::new(AtomicBool::new(false));
        let wakes = Arc::new(AtomicUsize::new(0));
        let permit = Arc::new(Mutex::new(PlanningWorkPermitDecision::Granted {
            requires_human_review: false,
        }));
        let runner = Arc::new(Runner {
            prepares: AtomicUsize::new(0),
        });
        let (artifacts, runtime) =
            crate::application::service::task_evidence::tests::fixtures::inert_verification_ports();
        let spawn: VerificationSpawn = {
            let jobs = jobs.clone();
            let calls = spawn_calls.clone();
            let fail = spawn_fails.clone();
            Arc::new(move |work, interrupted| {
                calls.fetch_add(1, Ordering::SeqCst);
                if fail.load(Ordering::SeqCst) {
                    return Err(std::io::Error::other("injected spawn failure"));
                }
                jobs.lock().unwrap().push((work, interrupted));
                Ok(())
            })
        };
        let wake: VerificationWake = {
            let wakes = wakes.clone();
            Arc::new(move || {
                wakes.fetch_add(1, Ordering::SeqCst);
            })
        };
        let authority = Arc::new(
            NoopPlanningAuthorityPort::default()
                .with_shared_work_permit_decision(permit.clone())
                .with_confirmed_worker_action_identity(
                    binding.as_ref().map(|(_, identity)| identity.clone()),
                ),
        );
        let evidence = Arc::new(
            crate::application::service::task_evidence::TaskEvidenceService::new(
                workspace.into(),
                repository.clone(),
                artifacts.clone(),
                runtime.clone(),
                authority.clone(),
            ),
        );
        let mut service = TaskVerificationService::new(TaskVerificationDependencies {
            workspace: workspace.into(),
            repository: repository.clone(),
            artifacts,
            runtime,
            authority,
            runner: runner.clone(),
            spawn,
            wake,
        })
        .unwrap();
        if let Some((configuration, _)) = binding {
            service =
                service.with_issue_binding_admission(configuration.admission, configuration.store);
        }
        let service = Arc::new(service);
        Self {
            service,
            evidence,
            repository,
            jobs,
            spawn_calls,
            spawn_fails,
            wakes,
            permit,
            runner,
        }
    }

    pub fn start(&self) -> TaskVerificationClaimResult {
        self.service.initialize_live_owner(AT).unwrap();
        self.service
            .request_verification(request(), &context())
            .unwrap()
    }

    pub fn take_job(&self) -> Scheduled {
        self.jobs.lock().unwrap().remove(0)
    }

    pub fn deliver_executed(
        &self,
        claim: &TaskVerificationClaimResult,
    ) -> TaskVerificationFinishCommit {
        let execution = claim.execution.clone().unwrap();
        self.repository
            .bind_host_verification(
                "/repo",
                &TaskVerificationBindCommit {
                    execution: execution.clone(),
                    manifest: manifest(),
                    at: AT.into(),
                },
            )
            .unwrap();
        let completion = TaskVerificationFinishCommit {
            execution,
            result: executed(),
            artifact: Some(TaskEvidenceArtifact {
                sha256: "f".repeat(64),
                size_bytes: 128,
            }),
            at: "2026-11-23T12:00:01.234Z".into(),
        };
        let (sender, receiver) = mpsc::channel();
        self.service
            .flight
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .receiver = receiver;
        sender.send(completion.clone()).unwrap();
        completion
    }
}

pub(super) fn request() -> RequestTaskVerificationRequest {
    RequestTaskVerificationRequest {
        request_id: "request-1".into(),
        task_id: "task-1".into(),
        criterion_id: "criterion 1".into(),
        scope_id: "1".repeat(64),
        expected_acceptance_revision: 1,
    }
}

pub(super) fn context() -> StoredCommandContext {
    StoredCommandContext {
        human_confirmation: None,
        caller: Some("worker".into()),
        channel: "kernel".into(),
        at: AT.into(),
    }
}

pub(super) fn snapshot() -> (TaskEvidenceStoredSnapshot, StoredTaskVerification) {
    let task: TaskDefinition = serde_json::from_value(serde_json::json!({
        "id":"task-1", "direction_id":"direction-1", "title":"Task", "description":"Check the frozen diff",
        "acceptance":[{"id":"criterion 1","scenario":"Check whitespace","channel":"command","essential":true,
            "verification_recipe":"git_diff_check_v1"}],
        "acceptance_revision":1, "status":"in_progress", "base_priority":1, "depends_on":[],
        "blocked_by":[], "created_by":"user", "last_updated_by":"user", "updated_at":AT
    })).unwrap();
    let scope = TaskEvidenceScope {
        task_id: task.id.clone(),
        scope_id: "1".repeat(64),
        acceptance_revision: 1,
        criteria_sha256: criteria_sha256(&task.acceptance).unwrap(),
    };
    let lease = ParallelModeSlotLeaseSnapshot::new(
        "slot-1",
        "task-1",
        "Task",
        "worker",
        "work/task",
        "/worker",
        ParallelModeSlotLeaseState::Running,
        AT,
        Some(AT.into()),
    )
    .with_lease_generation("3".repeat(64))
    .with_delivery_target(ParallelModeDeliveryTargetSnapshot::new(
        "origin",
        "owner/repo",
        ParallelModeRepositoryVisibility::Private,
        "prerelease",
        "a".repeat(40),
    ));
    let attempt = TaskVerificationAttempt {
        attempt_id: "2".repeat(64),
        request_id: "request-1".into(),
        scope: scope.clone(),
        criterion_id: "criterion 1".into(),
        recipe: TaskVerificationRecipe::GitDiffCheckV1,
        attempt: 1,
        slot_id: lease.slot_id.clone(),
        lease_generation: lease.lease_generation.clone().unwrap(),
        session_key: lease.session_key(),
        requested_at: AT.into(),
    };
    attempt.validate().unwrap();
    let stored = StoredTaskVerification {
        attempt,
        lease: lease.clone(),
        phase: TaskVerificationPhase::Pending,
        manifest: None,
        receipt: None,
    };
    let snapshot = TaskEvidenceStoredSnapshot {
        planning_revision: 1,
        evidence_revision: 0,
        tasks: vec![TaskEvidenceTaskSnapshot {
            reviews: vec![],
            task,
            scope,
            leases: vec![lease],
            queue_records: vec![],
            evidence: vec![],
            waivers: vec![],
            host_verifications: vec![],
        }],
        truncated: false,
        next_task_id: None,
    };
    (snapshot, stored)
}

pub(super) fn manifest() -> TaskVerificationExecutionManifest {
    TaskVerificationExecutionManifest {
        recipe: TaskVerificationRecipe::GitDiffCheckV1,
        base_commit_sha: "a".repeat(40),
        base_tree_sha: "b".repeat(40),
        source: TaskEvidenceTree {
            head_commit_sha: "c".repeat(40),
            tree_sha: "d".repeat(40),
            worktree_clean: false,
        },
        git_binary_sha256: "a".repeat(64),
        command_sha256: "b".repeat(64),
        environment_sha256: "c".repeat(64),
        configuration_sha256: "d".repeat(64),
    }
}

pub(super) fn executed() -> TaskVerificationResult {
    let empty = || TaskVerificationStream {
        sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
        size_bytes: 0,
        complete: true,
    };
    TaskVerificationResult::Executed {
        observation: Box::new(TaskVerificationObservation {
            outcome: TaskVerificationExecutionOutcome::Exited { exit_code: 0 },
            started_at: AT.into(),
            finished_at: "2026-11-23T12:00:01Z".into(),
            duration_ms: 1000,
            stdout: empty(),
            stderr: empty(),
        }),
    }
}

pub(crate) struct Runner {
    pub prepares: AtomicUsize,
}
impl TaskVerificationRunnerPort for Runner {
    fn prepare(
        &self,
        _: TaskVerificationPrepareRequest<'_>,
        _: &dyn Fn() -> bool,
    ) -> std::result::Result<Box<dyn PreparedTaskVerification>, TaskVerificationNotRunReason> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        panic!("manual completion or cancelled work must not enter Git preparation")
    }
    fn environment_fingerprint(
        &self,
        _: &TaskVerificationExecutionManifest,
    ) -> std::result::Result<String, TaskVerificationNotRunReason> {
        panic!("execution scheduling must not query an environment fingerprint")
    }
}

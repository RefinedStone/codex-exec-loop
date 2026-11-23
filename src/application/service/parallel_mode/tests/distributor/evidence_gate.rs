//! Real SQLite evidence and local bare refs surround fake GitHub PR metadata; no model runs.
use super::*;
use crate::adapter::outbound::filesystem::FilesystemTaskEvidenceArtifactAdapter;
use crate::adapter::outbound::git::task_verification::GitTaskVerificationRunnerAdapter;
use crate::application::port::inbound::stored_command_port::StoredCommandContext;
use crate::application::port::inbound::stored_command_port::issue_confirmation::VerifiedHumanConfirmation;
use crate::application::port::inbound::stored_command_port::task_evidence::{
    PrepareTaskEvidenceRequest, RecordTaskEvidenceRequest, RequestTaskVerificationRequest,
    WaiveTaskCriterionRequest,
};
use crate::application::port::inbound::task_delivery_evidence_port::{
    TaskDeliveryEvidenceDecision, TaskDeliveryEvidenceGatePort, TaskDeliveryEvidenceIntegration,
};
use crate::application::port::outbound::planning_task_repository_port::PlanningDirectionAuthorityCommit;
use crate::application::port::outbound::task_delivery_evidence_repository_port::TaskDeliveryEvidenceOperation;
use crate::application::port::outbound::task_evidence_repository_port::{
    TaskEvidenceRepositoryPort, TaskEvidenceSnapshotQuery,
};
use crate::application::service::task_delivery_evidence::{
    TaskDeliveryEvidenceDependencies, TaskDeliveryEvidenceService,
};
use crate::application::service::task_evidence::TaskEvidenceService;
use crate::application::service::task_verification::{
    TaskVerificationDependencies, TaskVerificationService, VerificationJob,
};
use crate::domain::planning::delivery_evidence::{
    TaskDeliveryCriterionState, TaskDeliveryEvidenceReason, TaskEvidenceDeliveryMode,
    TaskEvidenceDeliveryStage,
};
use crate::domain::planning::evidence::{TaskEvidenceScope, TaskEvidenceStatus};
use crate::domain::planning::verification::TaskVerificationRecipe;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
    QueueIdleConfig, TASK_AUTHORITY_FORMAT_VERSION, TaskAcceptanceChannel, TaskAcceptanceCriterion,
    TaskMutationProvenance,
};
use std::time::Instant;

const TASK: &str = "task-1";

struct GateCall {
    stage: TaskEvidenceDeliveryStage,
    effect: bool,
    allowed: bool,
    queue: PlanningAuthorityDistributorQueueRecord,
}

type GateHook = Box<dyn FnOnce(&PlanningAuthorityDistributorQueueRecord) + Send>;

struct RecordingGate {
    inner: Arc<dyn TaskDeliveryEvidenceGatePort>,
    calls: Mutex<Vec<GateCall>>,
    after: Mutex<Option<(TaskEvidenceDeliveryStage, GateHook)>>,
}

impl TaskDeliveryEvidenceGatePort for RecordingGate {
    fn evaluate(
        &self,
        workspace: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
        stage: TaskEvidenceDeliveryStage,
        integration: Option<&TaskDeliveryEvidenceIntegration>,
        operation: &TaskDeliveryEvidenceOperation,
    ) -> Result<TaskDeliveryEvidenceDecision, String> {
        let decision = self
            .inner
            .evaluate(workspace, record, stage, integration, operation)?;
        let effect = matches!(operation, TaskDeliveryEvidenceOperation::Effect { .. });
        self.calls.lock().unwrap().push(GateCall {
            stage,
            effect,
            allowed: decision.allowed,
            queue: decision.updated_queue.clone(),
        });
        let hook = {
            let mut after = self.after.lock().unwrap();
            if effect && decision.allowed && after.as_ref().is_some_and(|(at, _)| *at == stage) {
                after.take()
            } else {
                None
            }
        };
        if let Some((_, hook)) = hook {
            hook(&decision.updated_queue);
        }
        Ok(decision)
    }
}

struct Fixture {
    repo: TempGitRepo,
    authority: Arc<SqlitePlanningAuthorityAdapter>,
    github: FakeGithubAutomationPort,
    service: ParallelModeService,
    artifacts: Arc<FilesystemTaskEvidenceArtifactAdapter>,
    evidence: TaskEvidenceService,
    gate: Arc<RecordingGate>,
    lease: ParallelModeSlotLeaseSnapshot,
}

impl Fixture {
    fn new(label: &str, mode: TaskEvidenceDeliveryMode, accepted: bool) -> Self {
        let repo = TempGitRepo::new(label);
        let workspace = repo.workspace_dir();
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        seed_task(&workspace, authority.as_ref(), accepted);
        let github = FakeGithubAutomationPort::ready();
        let artifacts = Arc::new(FilesystemTaskEvidenceArtifactAdapter::new(Arc::new(
            |workspace| {
                Ok(PathBuf::from(
                    SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?
                        .runtime_dir,
                ))
            },
        )));
        let gate = Arc::new(RecordingGate {
            inner: Arc::new(TaskDeliveryEvidenceService::new(
                TaskDeliveryEvidenceDependencies {
                    workspace: workspace.clone(),
                    repository_identity: crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort::resolve_authority_location(
                        authority.as_ref(), &workspace,
                    ).unwrap().repository_identity,
                    repository: authority.clone(),
                    artifacts: artifacts.clone(),
                    runtime: Arc::new(test_parallel_runtime()),
                    authority: authority.clone(),
                    runner: Arc::new(GitTaskVerificationRunnerAdapter::new()),
                },
                Ok(mode),
            )),
            calls: Mutex::new(Vec::new()),
            after: Mutex::new(None),
        });
        let service = test_parallel_mode_service_with_github(Arc::new(github.clone()))
            .with_delivery_evidence(gate.clone());
        let assigned = service
            .acquire_slot_lease(
                &workspace,
                sample_lease_request(TASK, "Evidence delivery", "agent-1", "evidence"),
            )
            .unwrap();
        let lease = service
            .mark_workspace_slot_running(&assigned.worktree_path)
            .unwrap()
            .unwrap();
        fs::write(
            Path::new(&lease.worktree_path).join("feature.txt"),
            "verified work\n",
        )
        .unwrap();
        let evidence = TaskEvidenceService::new(
            workspace,
            authority.clone(),
            artifacts.clone(),
            Arc::new(test_parallel_runtime()),
            authority.clone(),
        );
        Self {
            repo,
            authority,
            github,
            service,
            artifacts,
            evidence,
            gate,
            lease,
        }
    }

    fn scope(&self) -> TaskEvidenceScope {
        self.authority
            .load_current_snapshot(
                &self.repo.workspace_dir(),
                &TaskEvidenceSnapshotQuery::default(),
            )
            .unwrap()
            .tasks[0]
            .scope
            .clone()
    }

    fn record_worker(&self, status: TaskEvidenceStatus) {
        let scope = self.scope();
        let prepared = self
            .evidence
            .prepare_evidence(
                PrepareTaskEvidenceRequest {
                    request_id: "worker-report".into(),
                    task_id: TASK.into(),
                    criterion_id: "C1".into(),
                    scope_id: scope.scope_id,
                    expected_acceptance_revision: scope.acceptance_revision,
                },
                &command_context(),
            )
            .unwrap();
        fs::write(
            &prepared.artifact_path,
            b"worker report: no host execution authority\n",
        )
        .unwrap();
        self.evidence
            .record_evidence(
                RecordTaskEvidenceRequest {
                    attempt_id: prepared.attempt.attempt_id,
                    status,
                    note: None,
                },
                &command_context(),
            )
            .unwrap();
    }

    fn verify_host(&self, expected_pass: bool) {
        let jobs: Arc<Mutex<Vec<VerificationJob>>> = Arc::new(Mutex::new(Vec::new()));
        let queue = jobs.clone();
        let service = TaskVerificationService::new(TaskVerificationDependencies {
            workspace: self.repo.workspace_dir(),
            repository: self.authority.clone(),
            artifacts: self.artifacts.clone(),
            runtime: Arc::new(test_parallel_runtime()),
            authority: self.authority.clone(),
            runner: Arc::new(GitTaskVerificationRunnerAdapter::new()),
            spawn: Arc::new(move |job, _recover| {
                queue.lock().unwrap().push(job);
                Ok(())
            }),
            wake: Arc::new(|| {}),
        })
        .unwrap();
        let context = command_context();
        service.initialize_live_owner(&context.at).unwrap();
        let scope = self.scope();
        let result = service
            .request_verification(
                RequestTaskVerificationRequest {
                    request_id: "host-before-delivery".into(),
                    task_id: TASK.into(),
                    criterion_id: "C1".into(),
                    scope_id: scope.scope_id,
                    expected_acceptance_revision: scope.acceptance_revision,
                },
                &context,
            )
            .unwrap();
        assert!(result.changed);
        let job = jobs.lock().unwrap().pop().unwrap();
        job();
        assert!(service.poll(Instant::now()));
        assert!(!service.has_pending());
        let snapshot = self
            .authority
            .load_current_snapshot(
                &self.repo.workspace_dir(),
                &TaskEvidenceSnapshotQuery::default(),
            )
            .unwrap();
        let receipt = snapshot.tasks[0].host_verifications[0]
            .receipt
            .as_ref()
            .unwrap();
        assert_eq!(receipt.passed(), expected_pass, "{receipt:?}");
    }

    fn waive(&self) {
        let scope = self.scope();
        let request = WaiveTaskCriterionRequest {
            request_id: "human-waiver".into(),
            task_id: TASK.into(),
            criterion_id: "C1".into(),
            scope_id: scope.scope_id,
            expected_acceptance_revision: scope.acceptance_revision,
            reason: "Explicit fixture human exception".into(),
        };
        let preview = self.evidence.prepare_waiver(&request).unwrap();
        let mut context = command_context();
        context.human_confirmation = Some(
            VerifiedHumanConfirmation::from_verified(
                preview.target,
                format!("desktop-confirmation:{}", "a".repeat(32)),
            )
            .unwrap(),
        );
        self.evidence.waive_criterion(request, &context).unwrap();
    }

    fn enqueue(&self) {
        let path = Path::new(&self.lease.worktree_path);
        run_git(path, &["add", "feature.txt"]);
        run_git(path, &["commit", "-qm", "verified worker tree"]);
        self.service
            .begin_workspace_official_completion(
                &self.lease.worktree_path,
                "evidence-delivery",
                None,
                Some("Delivery fixture result"),
                Some("A separate host receipt determines evidence"),
                None,
            )
            .unwrap();
        self.service
            .mark_workspace_official_completion_refreshing(&self.lease.worktree_path)
            .unwrap();
        self.service
            .mark_workspace_commit_ready(&self.lease.worktree_path, "fixture ready")
            .unwrap();
        self.service
            .enqueue_workspace_commit_ready_result(&self.lease.worktree_path)
            .unwrap()
            .unwrap();
    }

    fn queue(&self) -> PlanningAuthorityDistributorQueueRecord {
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&self.repo.workspace_dir())
            .unwrap()
            .distributor_queue_records
            .into_iter()
            .next()
            .unwrap()
    }

    fn process(&self) -> Vec<String> {
        self.service
            .process_distributor_queue(&self.repo.workspace_dir())
            .unwrap()
    }

    fn operations(&self) -> Vec<String> {
        self.github.operations.lock().unwrap().clone()
    }

    fn remote_ref(&self, branch: &str) -> Option<String> {
        run_command(
            "git",
            [
                "-C",
                self.repo.root.join("origin.git").to_str().unwrap(),
                "rev-parse",
                "--verify",
                &format!("refs/heads/{branch}"),
            ],
            None,
        )
    }

    fn assert_no_source_or_pr(&self) {
        assert!(
            self.operations().iter().all(|operation| {
                !operation.starts_with("push:")
                    && !operation.starts_with("ensure-pr:")
                    && !operation.starts_with("push-integration:")
            }),
            "{:?}",
            self.operations()
        );
        assert_eq!(self.remote_ref(&self.lease.branch_name), None);
        assert_eq!(
            self.remote_ref(POOL_BASELINE_BRANCH).as_deref(),
            Some(
                self.lease
                    .delivery_target
                    .as_ref()
                    .unwrap()
                    .integration_base_commit_sha
                    .as_str()
            )
        );
    }

    fn assert_blocked(&self, reason: TaskDeliveryEvidenceReason, stage: TaskEvidenceDeliveryStage) {
        let row = self.queue();
        assert_eq!(row.queue_state, ParallelModeQueueItemState::Blocked);
        assert_eq!(row.evidence_wait, Some(stage));
        let audit = row
            .evidence_observations
            .iter()
            .find(|audit| audit.stage == stage)
            .unwrap();
        assert!(audit.reasons.contains(&reason), "{audit:?}");
        assert!(!audit.allows_delivery());
    }
}

fn command_context() -> StoredCommandContext {
    StoredCommandContext {
        caller: Some("fixture-worker".into()),
        channel: "kernel".into(),
        at: "2026-11-23T12:00:00Z".into(),
        human_confirmation: None,
    }
}

fn seed_task(workspace: &str, store: &SqlitePlanningAuthorityAdapter, accepted: bool) {
    store
        .commit_direction_authority_snapshot(
            workspace,
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                authority_mutation_owner_token: None,
                directions: &DirectionCatalogDocument {
                    version: 1,
                    queue_idle: QueueIdleConfig::default(),
                    directions: vec![DirectionDefinition {
                        id: "delivery".into(),
                        title: "Delivery evidence".into(),
                        summary: "Exact host evidence".into(),
                        success_criteria: vec![
                            "Only verified or explicitly waived work is delivered".into(),
                        ],
                        scope_hints: vec![],
                        detail_doc_path: String::new(),
                        state: DirectionState::Active,
                    }],
                },
            },
        )
        .unwrap();
    let acceptance = if accepted {
        vec![TaskAcceptanceCriterion {
            id: "C1".into(),
            scenario: "The fixed whitespace recipe succeeds".into(),
            channel: TaskAcceptanceChannel::Command,
            essential: true,
            verification_recipe: Some(TaskVerificationRecipe::GitDiffCheckV1),
        }]
    } else {
        vec![]
    };
    store
        .commit_task_authority_snapshot(
            workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &TaskAuthorityDocument {
                    version: TASK_AUTHORITY_FORMAT_VERSION,
                    tasks: vec![TaskDefinition {
                        id: TASK.into(),
                        direction_id: "delivery".into(),
                        direction_relation_note: String::new(),
                        title: "Delivery evidence".into(),
                        description: "Exercise the real local delivery boundary".into(),
                        acceptance,
                        acceptance_revision: u64::from(accepted),
                        status: TaskStatus::Ready,
                        base_priority: 50,
                        dynamic_priority_delta: 0,
                        priority_reason: String::new(),
                        depends_on: vec![],
                        blocked_by: vec![],
                        created_by: TaskActor::User,
                        last_updated_by: TaskActor::User,
                        source_turn_id: None,
                        provenance: TaskMutationProvenance::new(OriginSessionKind::Main),
                        updated_at: "2026-11-23T12:00:00Z".into(),
                    }],
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

fn change_acceptance(workspace: &str) {
    let store = SqlitePlanningAuthorityAdapter::new();
    let mut snapshot = store
        .load_task_authority_snapshot(workspace)
        .unwrap()
        .unwrap();
    let task = snapshot
        .task_authority
        .tasks
        .iter_mut()
        .find(|task| task.id == TASK)
        .unwrap();
    task.acceptance_revision += 1;
    task.acceptance[0].scenario = "A newly accepted criterion invalidates old evidence".into();
    task.last_updated_by = TaskActor::User;
    store
        .commit_task_authority_snapshot(
            workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: Some(snapshot.planning_revision),
                task_authority: &snapshot.task_authority,
                queue_projection: &snapshot.queue_projection,
            },
        )
        .unwrap();
}

mod flow;

use super::*;
use crate::adapter::outbound::filesystem::FilesystemTaskEvidenceArtifactAdapter;
use crate::application::port::inbound::stored_command_port::StoredCommandContext;
use crate::application::port::inbound::stored_command_port::task_evidence::{
    PrepareTaskEvidenceRequest, RecordTaskEvidenceRequest,
};
use crate::application::port::inbound::task_evidence_query_port::TaskEvidenceQueryPort;
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::task_evidence_artifact_port::TaskEvidenceArtifactPort;
use crate::application::port::outbound::task_evidence_repository_port::{
    TaskEvidenceRepositoryPort, TaskEvidenceSnapshotQuery,
};
use crate::application::service::task_evidence::TaskEvidenceService;
use crate::domain::planning::evidence::{
    TaskCriterionEvidenceState, TaskEvidenceError, TaskEvidenceStatus,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
    QueueIdleConfig, TASK_AUTHORITY_FORMAT_VERSION, TaskAcceptanceChannel, TaskAcceptanceCriterion,
    TaskActor, TaskAuthorityDocument, TaskDefinition, TaskMutationProvenance,
};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn seed_accepted_task(
    workspace: &str,
    adapter: &SqlitePlanningAuthorityAdapter,
    task_id: &str,
) {
    seed_accepted_task_with_recipe(workspace, adapter, task_id, None);
}

pub(super) fn seed_accepted_task_with_recipe(
    workspace: &str,
    adapter: &SqlitePlanningAuthorityAdapter,
    task_id: &str,
    recipe: Option<crate::domain::planning::verification::TaskVerificationRecipe>,
) {
    let directions = DirectionCatalogDocument {
        version: 1,
        queue_idle: QueueIdleConfig::default(),
        directions: vec![DirectionDefinition {
            id: "evidence-direction".into(),
            title: "Evidence fixture".into(),
            summary: "Verify bounded evidence capture".into(),
            success_criteria: vec!["Evidence remains bound to the current worker tree".into()],
            scope_hints: vec![],
            detail_doc_path: String::new(),
            state: DirectionState::Active,
        }],
    };
    adapter
        .commit_direction_authority_snapshot(
            workspace,
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                directions: &directions,
                authority_mutation_owner_token: None,
            },
        )
        .unwrap();
    let task = TaskDefinition {
        id: task_id.into(),
        direction_id: "evidence-direction".into(),
        direction_relation_note: String::new(),
        title: "Host commit task".into(),
        description: "Exercise the real evidence persistence boundaries without a model".into(),
        acceptance: vec![TaskAcceptanceCriterion {
            id: "C1".into(),
            scenario: "The fixture result is recorded for this code tree".into(),
            channel: TaskAcceptanceChannel::Command,
            essential: true,
            verification_recipe: recipe,
        }],
        acceptance_revision: 1,
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
    };
    let queue = PriorityQueueProjection {
        next_task: None,
        active_tasks: vec![],
        proposed_tasks: vec![],
        skipped_tasks: vec![],
    };
    adapter
        .commit_task_authority_snapshot(
            workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &TaskAuthorityDocument {
                    version: TASK_AUTHORITY_FORMAT_VERSION,
                    tasks: vec![task],
                },
                queue_projection: &queue,
            },
        )
        .unwrap();
}

#[test]
fn task_evidence_real_running_worker_git_sqlite_capture_and_current_tree_projection() {
    let repo = TempGitRepo::new("task-evidence-running-end-to-end");
    let workspace = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    seed_accepted_task(&workspace, authority.as_ref(), "task-evidence-running");
    let parallel = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&parallel, &repo, "task-evidence-running");
    let resolution = crate::application::service::parallel_mode::pool::resolve_workspace_slot_lease_with_runtime(
        &test_parallel_runtime(), authority.as_ref(), &lease.worktree_path,
    ).unwrap().unwrap();
    assert_eq!(
        resolution.context.canonical_repo_root,
        repo.canonical_repo_root()
    );
    assert_ne!(
        Path::new(&resolution.context.repo_root),
        resolution.context.canonical_repo_root.as_path()
    );
    let head = resolve_workspace_head_sha(Path::new(&lease.worktree_path)).unwrap();
    let base_tree = run_command(
        "git",
        [
            "-C",
            lease.worktree_path.as_str(),
            "rev-parse",
            "HEAD^{tree}",
        ],
        None,
    )
    .unwrap();
    let code = Path::new(&lease.worktree_path).join("evidence-change.txt");
    fs::write(&code, "version one\n").unwrap();
    let artifacts = Arc::new(FilesystemTaskEvidenceArtifactAdapter::new(Arc::new(
        |workspace| {
            Ok(PathBuf::from(
                SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?
                    .runtime_dir,
            ))
        },
    )));
    let service = TaskEvidenceService::new(
        workspace.clone(),
        authority.clone(),
        artifacts,
        Arc::new(test_parallel_runtime()),
        authority.clone(),
    );
    let current = authority
        .load_current_snapshot(&workspace, &TaskEvidenceSnapshotQuery::default())
        .unwrap();
    let scope = &current.tasks[0].scope;
    let context = StoredCommandContext {
        human_confirmation: None,
        caller: Some("fixture-worker".into()),
        channel: "kernel".into(),
        at: "2026-11-23T12:00:00Z".into(),
    };
    let prepared = service
        .prepare_evidence(
            PrepareTaskEvidenceRequest {
                request_id: "real-running-attempt".into(),
                task_id: scope.task_id.clone(),
                criterion_id: "C1".into(),
                scope_id: scope.scope_id.clone(),
                expected_acceptance_revision: scope.acceptance_revision,
            },
            &context,
        )
        .unwrap();
    assert_ne!(prepared.attempt.prepared_tree.tree_sha, base_tree);
    let request = RecordTaskEvidenceRequest {
        attempt_id: prepared.attempt.attempt_id.clone(),
        status: TaskEvidenceStatus::Pass,
        note: Some("reported fixture result".into()),
    };
    let missing = service
        .record_evidence(request.clone(), &context)
        .unwrap_err();
    assert_eq!(
        missing.downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::ArtifactMissing)
    );
    assert!(service.pending_events(10).unwrap().is_empty());
    let bytes = b"fixture result: version one, reported pass\n";
    fs::write(&prepared.artifact_path, bytes).unwrap();
    let recorded = service.record_evidence(request.clone(), &context).unwrap();
    assert!(recorded.changed);
    assert_eq!(
        recorded.value.evidence.artifact.sha256,
        format!("{:x}", Sha256::digest(bytes))
    );
    assert_eq!(
        recorded.value.evidence.artifact.size_bytes,
        bytes.len() as u64
    );
    assert_eq!(
        recorded.value.evidence.recorded_tree,
        prepared.attempt.prepared_tree
    );
    assert_eq!(
        resolve_workspace_head_sha(Path::new(&lease.worktree_path)).unwrap(),
        head
    );
    let observed = service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert!(observed.unavailable.is_none());
    assert!(observed.tasks[0].unavailable.is_none());
    assert_eq!(
        observed.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Pass)
    );
    fs::write(code, "version two\n").unwrap();
    let changed = service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(
        changed.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Stale)
    );
    let replay = service.record_evidence(request, &context).unwrap();
    assert!(!replay.changed);
    assert_eq!(replay.value, recorded.value);
    assert_eq!(service.pending_events(10).unwrap().len(), 1);
}

struct ChangeCodeDuringArtifactVerification {
    inner: FilesystemTaskEvidenceArtifactAdapter,
    code: PathBuf,
    armed: AtomicBool,
}

impl TaskEvidenceArtifactPort for ChangeCodeDuringArtifactVerification {
    fn prepare_lease_incoming_root(
        &self,
        workspace: &str,
        generation: &str,
    ) -> anyhow::Result<PathBuf> {
        self.inner
            .prepare_lease_incoming_root(workspace, generation)
    }

    fn prepare_artifact(
        &self,
        workspace: &str,
        attempt: &crate::domain::planning::evidence::TaskEvidenceAttempt,
    ) -> anyhow::Result<PathBuf> {
        self.inner.prepare_artifact(workspace, attempt)
    }

    fn observe_and_seal(
        &self,
        workspace: &str,
        attempt: &crate::domain::planning::evidence::TaskEvidenceAttempt,
        tree: &crate::domain::planning::evidence::TaskEvidenceTree,
    ) -> anyhow::Result<crate::domain::planning::evidence::TaskEvidenceArtifact> {
        self.inner.observe_and_seal(workspace, attempt, tree)
    }

    fn verify_sealed(
        &self,
        workspace: &str,
        attempt: &crate::domain::planning::evidence::TaskEvidenceAttempt,
        artifact: &crate::domain::planning::evidence::TaskEvidenceArtifact,
    ) -> anyhow::Result<()> {
        self.inner.verify_sealed(workspace, attempt, artifact)?;
        if self.armed.swap(false, Ordering::SeqCst) {
            fs::write(&self.code, "version two\n")?;
        }
        Ok(())
    }
}

#[test]
fn task_evidence_running_source_change_during_sealed_verification_clears_current_pass() {
    let repo = TempGitRepo::new("task-evidence-query-running-race");
    let workspace = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    seed_accepted_task(&workspace, &authority, "task-query-race");
    let parallel = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&parallel, &repo, "task-query-race");
    let code = Path::new(&lease.worktree_path).join("evidence-change.txt");
    fs::write(&code, "version one\n").unwrap();
    let artifacts = Arc::new(ChangeCodeDuringArtifactVerification {
        inner: FilesystemTaskEvidenceArtifactAdapter::new(Arc::new(|workspace| {
            Ok(PathBuf::from(
                SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?
                    .runtime_dir,
            ))
        })),
        code,
        armed: AtomicBool::new(false),
    });
    let service = TaskEvidenceService::new(
        workspace.clone(),
        authority.clone(),
        artifacts.clone(),
        Arc::new(test_parallel_runtime()),
        authority.clone(),
    );
    let stored = authority
        .load_current_snapshot(&workspace, &TaskEvidenceSnapshotQuery::default())
        .unwrap();
    let scope = &stored.tasks[0].scope;
    let context = StoredCommandContext {
        human_confirmation: None,
        caller: Some("fixture-worker".into()),
        channel: "kernel".into(),
        at: "2026-11-23T12:00:00Z".into(),
    };
    let prepared = service
        .prepare_evidence(
            PrepareTaskEvidenceRequest {
                request_id: "query-race-attempt".into(),
                task_id: scope.task_id.clone(),
                criterion_id: "C1".into(),
                scope_id: scope.scope_id.clone(),
                expected_acceptance_revision: scope.acceptance_revision,
            },
            &context,
        )
        .unwrap();
    fs::write(
        &prepared.artifact_path,
        b"worker reported pass for version one\n",
    )
    .unwrap();
    service
        .record_evidence(
            RecordTaskEvidenceRequest {
                attempt_id: prepared.attempt.attempt_id,
                status: TaskEvidenceStatus::Pass,
                note: None,
            },
            &context,
        )
        .unwrap();

    artifacts.armed.store(true, Ordering::SeqCst);
    let raced = service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert!(raced.unavailable.is_none());
    assert_eq!(raced.tasks[0].unavailable, Some(TaskEvidenceError::Stale));
    assert_eq!(raced.tasks[0].source, None);
    assert_eq!(raced.tasks[0].criteria[0].state, None);
    assert_eq!(
        raced.tasks[0].criteria[0].evidence.as_ref().unwrap().status,
        TaskEvidenceStatus::Pass
    );
    // The immutable report remains a past fact. A subsequent stable query can now
    // explain that its tree is stale instead of retaining an unavailable result forever.
    let current = service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert!(current.tasks[0].unavailable.is_none());
    assert_eq!(
        current.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Stale)
    );
}

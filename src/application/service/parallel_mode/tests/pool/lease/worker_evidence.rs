use super::task_evidence::seed_accepted_task;
use super::*;
use crate::adapter::outbound::filesystem::FilesystemTaskEvidenceArtifactAdapter;
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::task_evidence_artifact_port::TaskEvidenceArtifactPort;
use crate::domain::planning::evidence::{
    TaskEvidenceArtifact, TaskEvidenceAttempt, TaskEvidenceTree,
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn artifacts() -> Arc<FilesystemTaskEvidenceArtifactAdapter> {
    Arc::new(FilesystemTaskEvidenceArtifactAdapter::new(Arc::new(
        |workspace| {
            Ok(PathBuf::from(
                SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?
                    .runtime_dir,
            ))
        },
    )))
}

#[test]
fn accepted_criteria_prepare_one_generation_root_for_initial_and_running_workers() {
    let repo = TempGitRepo::new("worker-evidence-generation");
    let workspace = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    seed_accepted_task(&workspace, &authority, "evidence-worker");
    let service = test_parallel_mode_service().with_task_evidence(authority, artifacts());
    let lease = service
        .acquire_slot_lease(
            &workspace,
            sample_lease_request(
                "evidence-worker",
                "Host commit task",
                "agent-host",
                "evidence",
            ),
        )
        .unwrap();
    let initial = service
        .prepare_worker_evidence_incoming(&workspace, &lease)
        .unwrap()
        .unwrap();
    assert_eq!(
        initial.file_name().unwrap().to_str(),
        lease.lease_generation.as_deref()
    );
    assert_eq!(initial.parent().unwrap().file_name().unwrap(), "incoming");
    assert!(
        !initial
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("sealed")
            .exists()
    );
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .unwrap();
    assert_eq!(
        service
            .prepare_worker_evidence_incoming(&workspace, &lease)
            .unwrap(),
        Some(initial)
    );
}

#[derive(Default)]
struct CountingArtifacts {
    prepares: AtomicUsize,
}

impl TaskEvidenceArtifactPort for CountingArtifacts {
    fn prepare_lease_incoming_root(&self, _: &str, _: &str) -> anyhow::Result<PathBuf> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("PRIVATE-ARTIFACT-PATH must not be exposed in dispatch notices")
    }
    fn prepare_artifact(&self, _: &str, _: &TaskEvidenceAttempt) -> anyhow::Result<PathBuf> {
        anyhow::bail!("unused")
    }
    fn observe_and_seal(
        &self,
        _: &str,
        _: &TaskEvidenceAttempt,
        _: &TaskEvidenceTree,
    ) -> anyhow::Result<TaskEvidenceArtifact> {
        anyhow::bail!("unused")
    }
    fn verify_sealed(
        &self,
        _: &str,
        _: &TaskEvidenceAttempt,
        _: &TaskEvidenceArtifact,
    ) -> anyhow::Result<()> {
        anyhow::bail!("unused")
    }
}

#[test]
fn empty_criteria_do_not_prepare_any_additional_worker_directory() {
    let repo = TempGitRepo::new("worker-evidence-empty");
    let workspace = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    seed_accepted_task(&workspace, &authority, "evidence-worker");
    let mut snapshot = authority
        .load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap();
    snapshot.task_authority.tasks[0].acceptance.clear();
    snapshot.task_authority.tasks[0].acceptance_revision += 1;
    authority
        .commit_task_authority_snapshot(
            &workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: Some(snapshot.planning_revision),
                task_authority: &snapshot.task_authority,
                queue_projection: &snapshot.queue_projection,
            },
        )
        .unwrap();
    let artifacts = Arc::new(CountingArtifacts::default());
    let service = test_parallel_mode_service().with_task_evidence(authority, artifacts.clone());
    let lease = acquire_running_worker_lease(&service, &repo, "evidence-worker");
    assert_eq!(
        service
            .prepare_worker_evidence_incoming(&workspace, &lease)
            .unwrap(),
        None
    );
    assert_eq!(artifacts.prepares.load(Ordering::SeqCst), 0);
}

#[test]
fn stale_or_legacy_lease_identity_never_prepares_an_incoming_directory() {
    let repo = TempGitRepo::new("worker-evidence-identity");
    let workspace = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    seed_accepted_task(&workspace, &authority, "evidence-worker");
    let artifacts = Arc::new(CountingArtifacts::default());
    let service = test_parallel_mode_service().with_task_evidence(authority, artifacts.clone());
    let mut lease = acquire_running_worker_lease(&service, &repo, "evidence-worker");
    lease.lease_generation = Some("f".repeat(64));
    assert!(
        service
            .prepare_worker_evidence_incoming(&workspace, &lease)
            .is_err()
    );
    lease.lease_generation = None;
    assert!(
        service
            .prepare_worker_evidence_incoming(&workspace, &lease)
            .is_err()
    );
    assert_eq!(artifacts.prepares.load(Ordering::SeqCst), 0);
}

#[test]
fn a_lease_from_another_canonical_repository_cannot_prepare_worker_evidence() {
    let repo = TempGitRepo::new("worker-evidence-canonical-source");
    let other = TempGitRepo::new("worker-evidence-canonical-other");
    let workspace = repo.workspace_dir();
    let other_workspace = other.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    seed_accepted_task(&workspace, &authority, "evidence-worker");
    seed_accepted_task(&other_workspace, &authority, "evidence-worker");
    let artifacts = Arc::new(CountingArtifacts::default());
    let service = test_parallel_mode_service().with_task_evidence(authority, artifacts.clone());
    let lease = acquire_running_worker_lease(&service, &repo, "evidence-worker");
    assert!(
        service
            .prepare_worker_evidence_incoming(&other_workspace, &lease)
            .is_err()
    );
    assert_eq!(artifacts.prepares.load(Ordering::SeqCst), 0);
}

#[test]
fn an_artifact_preparation_failure_preserves_the_running_lease() {
    let repo = TempGitRepo::new("worker-evidence-unavailable");
    let workspace = repo.workspace_dir();
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    seed_accepted_task(&workspace, &authority, "evidence-worker");
    let artifacts = Arc::new(CountingArtifacts::default());
    let service = test_parallel_mode_service().with_task_evidence(authority, artifacts.clone());
    let lease = acquire_running_worker_lease(&service, &repo, "evidence-worker");
    assert!(
        service
            .prepare_worker_evidence_incoming(&workspace, &lease)
            .is_err()
    );
    assert_eq!(artifacts.prepares.load(Ordering::SeqCst), 1);
    let current = service.prepare_workspace_worker_commit(&lease).unwrap_err();
    // The ordinary commit path still observes the same Running lease; it only rejects
    // this baseline because there are no result changes to commit.
    assert!(current.contains("no tree changes"), "{current}");
}

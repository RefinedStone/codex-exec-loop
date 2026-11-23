use super::*;
use crate::adapter::outbound::filesystem::FilesystemTaskEvidenceArtifactAdapter;
use crate::adapter::outbound::git::task_verification::GitTaskVerificationRunnerAdapter;
use crate::application::port::inbound::stored_command_port::StoredCommandContext;
use crate::application::port::inbound::stored_command_port::task_evidence::RequestTaskVerificationRequest;
use crate::application::port::inbound::task_evidence_query_port::TaskEvidenceQueryPort;
use crate::application::port::outbound::task_evidence_repository_port::{
    TaskEvidenceRepositoryPort, TaskEvidenceSnapshotQuery,
};
use crate::application::service::task_evidence::TaskEvidenceService;
use crate::application::service::task_verification::{
    TaskVerificationDependencies, TaskVerificationService, VerificationJob,
};
use crate::domain::planning::evidence::TaskCriterionEvidenceState;
use crate::domain::planning::verification::{
    TaskVerificationCurrentState, TaskVerificationPhase, TaskVerificationRecipe,
};
use std::sync::Mutex;
use std::time::Instant;

#[test]
fn real_running_git_sqlite_host_capture_replay_and_currentness_without_a_model() {
    for (contents, expected) in [
        (
            "clean verification text\n",
            TaskVerificationCurrentState::Passed,
        ),
        (
            "trailing whitespace  \n",
            TaskVerificationCurrentState::Failed,
        ),
    ] {
        let repo = TempGitRepo::new("host-verification-running-end-to-end");
        let workspace = repo.workspace_dir();
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        task_evidence::seed_accepted_task_with_recipe(
            &workspace,
            authority.as_ref(),
            "host-verification-running",
            Some(TaskVerificationRecipe::GitDiffCheckV1),
        );
        let parallel = test_parallel_mode_service();
        let lease = acquire_running_worker_lease(&parallel, &repo, "host-verification-running");
        let code = Path::new(&lease.worktree_path).join("host-verification.txt");
        fs::write(&code, contents).unwrap();
        let original_head = resolve_workspace_head_sha(Path::new(&lease.worktree_path)).unwrap();
        let artifacts = Arc::new(FilesystemTaskEvidenceArtifactAdapter::new(Arc::new(
            |workspace| {
                Ok(PathBuf::from(
                    SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?
                        .runtime_dir,
                ))
            },
        )));
        let runner = Arc::new(GitTaskVerificationRunnerAdapter::new());
        let jobs: Arc<Mutex<Vec<VerificationJob>>> = Arc::new(Mutex::new(Vec::new()));
        let queue = jobs.clone();
        let verification = TaskVerificationService::new(TaskVerificationDependencies {
            workspace: workspace.clone(),
            repository: authority.clone(),
            artifacts: artifacts.clone(),
            runtime: Arc::new(test_parallel_runtime()),
            authority: authority.clone(),
            runner: runner.clone(),
            spawn: Arc::new(move |work, _recover| {
                queue.lock().unwrap().push(work);
                Ok(())
            }),
            wake: Arc::new(|| {}),
        })
        .unwrap();
        let query = TaskEvidenceService::new(
            workspace.clone(),
            authority.clone(),
            artifacts,
            Arc::new(test_parallel_runtime()),
            authority.clone(),
        )
        .with_verification_runner(runner);
        let at = "2026-11-23T12:00:00Z";
        verification.initialize_live_owner(at).unwrap();
        let snapshot = authority
            .load_current_snapshot(&workspace, &TaskEvidenceSnapshotQuery::default())
            .unwrap();
        let scope = &snapshot.tasks[0].scope;
        let request = RequestTaskVerificationRequest {
            request_id: "host-verification-request".into(),
            task_id: scope.task_id.clone(),
            criterion_id: "C1".into(),
            scope_id: scope.scope_id.clone(),
            expected_acceptance_revision: scope.acceptance_revision,
        };
        let context = StoredCommandContext {
            caller: Some("fixture-worker".into()),
            channel: "kernel".into(),
            at: at.into(),
            human_confirmation: None,
        };
        let claim = verification
            .request_verification(request.clone(), &context)
            .unwrap();
        assert!(claim.changed);
        assert_eq!(claim.stored.phase, TaskVerificationPhase::Pending);
        assert_eq!(jobs.lock().unwrap().len(), 1);
        let replay = verification
            .request_verification(request.clone(), &context)
            .unwrap();
        assert!(!replay.changed);
        assert!(replay.execution.is_none());
        let job = jobs.lock().unwrap().pop().unwrap();
        job();
        assert!(verification.poll(Instant::now()));
        assert!(!verification.has_pending());
        assert_eq!(
            resolve_workspace_head_sha(Path::new(&lease.worktree_path)).unwrap(),
            original_head
        );
        let observed = query
            .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
            .unwrap();
        assert_eq!(observed.unavailable, None);
        assert_eq!(observed.tasks[0].unavailable, None);
        let row = &observed.tasks[0].criteria[0];
        assert_eq!(row.state, Some(TaskCriterionEvidenceState::Pending));
        let host = row.host_verification.as_ref().unwrap();
        assert_eq!(host.state, expected, "{host:?}");
        let receipt = host.receipt.clone().unwrap();
        let replay = verification
            .request_verification(request, &context)
            .unwrap();
        assert!(!replay.changed);
        assert!(replay.execution.is_none());
        assert!(jobs.lock().unwrap().is_empty());
        assert_eq!(query.pending_events(10).unwrap().len(), 1);

        fs::write(&code, "a changed tree after verification\n").unwrap();
        let changed = query
            .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
            .unwrap();
        let host = changed.tasks[0].criteria[0]
            .host_verification
            .as_ref()
            .unwrap();
        assert_eq!(host.state, TaskVerificationCurrentState::Stale);
        assert_eq!(host.receipt.as_ref(), Some(&receipt));
        fs::write(&code, contents).unwrap();
        let runtime =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&workspace).unwrap();
        let sealed = Path::new(&runtime.runtime_dir)
            .join("evidence")
            .join("sealed")
            .join("verification")
            .join(&claim.stored.attempt.attempt_id)
            .join("artifact");
        fs::remove_file(&sealed).unwrap();
        let missing = query
            .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
            .unwrap();
        let host = missing.tasks[0].criteria[0]
            .host_verification
            .as_ref()
            .unwrap();
        assert_eq!(host.state, TaskVerificationCurrentState::Unavailable);
        assert_eq!(host.receipt.as_ref(), Some(&receipt));
        assert!(!sealed.exists());
        assert!(jobs.lock().unwrap().is_empty());
        assert_eq!(query.pending_events(10).unwrap().len(), 1);
    }
}

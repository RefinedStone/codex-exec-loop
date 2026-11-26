//! Exercise the real running-worker query/guard path with in-memory port observations.
//! No filesystem, Git process, network, or model I/O is performed by this fixture.
use super::*;
use crate::application::port::inbound::task_evidence_query_port::TaskEvidenceSourceKind;
use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelCommandOutput, ParallelPoolMutationPermit, ParallelWorkerTreeRequest,
    ParallelWorkerTreeSnapshot,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityRuntimeProjectionSnapshot;

#[test]
fn the_final_running_tree_snapshot_cannot_leave_a_success_from_an_earlier_environment() {
    assert!(crate::configuration::current_process_config().is_none());
    for changed in [
        None,
        Some(Ok(ENVIRONMENT_B.to_owned())),
        Some(Err(TaskVerificationNotRunReason::Unavailable)),
    ] {
        let mut fixture = HostFixture::new();
        fixture.add_second_criterion();
        fixture.base.install_record(TaskEvidenceStatus::Pass);
        let stored = fixture.install_host(TaskVerificationPhase::Finished);
        fixture.add_second_host(ENVIRONMENT_A);
        let canonical = PathBuf::from("/repo");
        let pool = crate::domain::parallel_mode::default_parallel_pool_root(&canonical);
        let mut lease = stored.lease;
        lease.worktree_path = pool.join(&lease.slot_id).to_string_lossy().into_owned();
        lease.branch_name = "akra-agent/slot-1/task-1".into();
        {
            let mut snapshot = fixture.base.repository.snapshot.lock().unwrap();
            let task = &mut snapshot.tasks[0];
            task.queue_records.clear();
            task.leases = vec![lease.clone()];
            for stored in &mut task.host_verifications {
                stored.lease = lease.clone();
            }
        }
        let mut projection = PlanningAuthorityRuntimeProjectionSnapshot::default();
        projection
            .slot_leases
            .insert(lease.slot_id.clone(), lease.clone());
        let authority = NoopPlanningAuthorityPort::default()
            .with_canonical_repo_root(canonical.to_string_lossy().into_owned())
            .with_runtime_projection(projection);
        let runtime = Arc::new(RunningRuntime {
            canonical,
            pool,
            lease,
            runner: fixture.runner.clone(),
            change_on_final_snapshot: changed.clone(),
            snapshots: AtomicUsize::new(0),
            permits: AtomicUsize::new(0),
            permit_checks: Arc::new(AtomicUsize::new(0)),
            held: Arc::new(AtomicBool::new(false)),
        });
        fixture.service = TaskEvidenceService::new(
            "/repo".into(),
            fixture.repository.clone(),
            fixture.artifacts.clone(),
            runtime.clone(),
            Arc::new(authority),
        )
        .with_verification_runner(fixture.runner.clone());

        let originals: Vec<_> = fixture.base.repository.snapshot.lock().unwrap().tasks[0]
            .host_verifications
            .iter()
            .map(|stored| {
                stored
                    .receipt
                    .as_ref()
                    .map(TaskVerificationReceiptProjection::from)
            })
            .collect();
        let snapshot = fixture.query();
        assert_eq!(snapshot.unavailable, None);
        assert_eq!(snapshot.tasks[0].unavailable, None);
        assert_eq!(
            snapshot.tasks[0].source.as_ref().unwrap().kind,
            TaskEvidenceSourceKind::RunningLease
        );
        assert_eq!(
            snapshot.tasks[0].criteria[0].state,
            Some(TaskCriterionEvidenceState::Pass)
        );
        let expected = match &changed {
            None => TaskVerificationCurrentState::Passed,
            Some(Ok(_)) => TaskVerificationCurrentState::Stale,
            Some(Err(_)) => TaskVerificationCurrentState::Unavailable,
        };
        for (row, original) in snapshot.tasks[0].criteria.iter().zip(&originals) {
            let host = row.host_verification.as_ref().unwrap();
            assert_eq!(host.state, expected);
            assert_eq!(
                host.unavailable,
                if changed.as_ref().is_some_and(|value| value.is_err()) {
                    Some(TaskEvidenceError::Unavailable)
                } else {
                    None
                }
            );
            assert_eq!(&host.receipt, original);
        }
        assert_eq!(snapshot.tasks[0].criteria.len(), 2);
        assert_eq!(fixture.artifacts.reads.load(Ordering::SeqCst), 2);
        assert_eq!(runtime.snapshots.load(Ordering::SeqCst), 2);
        assert_eq!(runtime.permits.load(Ordering::SeqCst), 2);
        assert_eq!(
            runtime.permit_checks.load(Ordering::SeqCst),
            4,
            "each source observation validates the live lease before and after capture"
        );
        assert!(!runtime.held.load(Ordering::SeqCst));
        assert_eq!(fixture.repository.reads.load(Ordering::SeqCst), 2);
        assert_eq!(
            fixture.runner.reads.load(Ordering::SeqCst),
            2,
            "the last environment observation must follow the final source snapshot and second DB read"
        );
    }
}

struct RunningRuntime {
    canonical: PathBuf,
    pool: PathBuf,
    lease: ParallelModeSlotLeaseSnapshot,
    runner: Arc<Runner>,
    change_on_final_snapshot: Option<std::result::Result<String, TaskVerificationNotRunReason>>,
    snapshots: AtomicUsize,
    permits: AtomicUsize,
    permit_checks: Arc<AtomicUsize>,
    held: Arc<AtomicBool>,
}

impl RunningRuntime {
    fn known_path(&self, path: &Path) -> bool {
        path == self.canonical
            || path == self.pool
            || path == Path::new(&self.lease.worktree_path)
            || self.pool.parent() == Some(path)
            || self.pool.parent().and_then(Path::parent) == Some(path)
    }
    fn known_workspace(&self, workspace: &str) -> bool {
        Path::new(workspace) == self.canonical
            || Path::new(workspace) == Path::new(&self.lease.worktree_path)
    }
}

struct Permit {
    pool: PathBuf,
    held: Arc<AtomicBool>,
    checks: Arc<AtomicUsize>,
}

impl ParallelPoolMutationPermit for Permit {
    fn verify_pool_root(&self, root: &Path) -> std::result::Result<(), String> {
        if root != self.pool || !self.held.load(Ordering::SeqCst) {
            return Err("fixture pool ownership does not match".into());
        }
        self.checks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        assert!(self.held.swap(false, Ordering::SeqCst));
    }
}

impl ParallelModeRuntimePort for RunningRuntime {
    fn try_acquire_pool_mutation_permit(
        &self,
        pool: &Path,
    ) -> std::result::Result<Option<Box<dyn ParallelPoolMutationPermit>>, String> {
        if pool != self.pool {
            return Err("unexpected pool".into());
        }
        if self.held.swap(true, Ordering::SeqCst) {
            return Ok(None);
        }
        self.permits.fetch_add(1, Ordering::SeqCst);
        Ok(Some(Box::new(Permit {
            pool: pool.to_owned(),
            held: self.held.clone(),
            checks: self.permit_checks.clone(),
        })))
    }
    fn detect_git_repo_root(&self, workspace: &str) -> Option<String> {
        self.known_workspace(workspace)
            .then(|| workspace.to_owned())
    }
    fn ensure_git_execution_safe(&self, path: &Path) -> std::result::Result<(), String> {
        if self.known_workspace(&path.to_string_lossy()) && self.held.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err("fixture workspace is not guarded".into())
        }
    }
    fn run_command(
        &self,
        program: &str,
        args: &[&str],
        current_dir: Option<&str>,
    ) -> Option<String> {
        assert_eq!(program, "git");
        assert!(current_dir.is_none());
        assert!(self.held.load(Ordering::SeqCst));
        let ["-C", workspace, tail @ ..] = args else {
            panic!("unexpected metadata command: {args:?}");
        };
        assert!(self.known_workspace(workspace));
        match tail {
            ["config", "--get", "akra.parallelIntegrationBranch"] => Some("prerelease".into()),
            ["config", "--get", "akra.githubPushRemote"] => Some("origin".into()),
            ["rev-parse", "refs/remotes/origin/prerelease"]
            | ["rev-parse", "prerelease"]
            | ["rev-parse", "HEAD"] => Some("a".repeat(40)),
            ["rev-parse", "--abbrev-ref", "HEAD"] => Some(self.lease.branch_name.clone()),
            ["worktree", "list", "--porcelain"] => Some(format!(
                "worktree {}\nHEAD {}\nbranch refs/heads/{}\n\n",
                self.lease.worktree_path,
                "a".repeat(40),
                self.lease.branch_name,
            )),
            _ => panic!("unexpected metadata command: {args:?}"),
        }
    }
    fn run_git_command(
        &self,
        args: &[std::ffi::OsString],
        stdin: Option<&[u8]>,
    ) -> std::result::Result<ParallelCommandOutput, String> {
        // The shared cleanup identity guard retains raw inventory failures. Keep this
        // in-memory fixture on the same held-lease and known-workspace checks as metadata.
        assert!(stdin.is_none());
        let args = args
            .iter()
            .map(|arg| arg.to_str().expect("fixture arguments should be UTF-8"))
            .collect::<Vec<_>>();
        assert!(matches!(
            args.as_slice(),
            ["-C", _, "worktree", "list", "--porcelain"]
        ));
        let stdout = self
            .run_command("git", &args, None)
            .expect("fixture worktree inventory should exist");
        Ok(ParallelCommandOutput {
            exit_code: Some(0),
            stdout: stdout.into_bytes(),
            stderr: Vec::new(),
        })
    }
    fn snapshot_parallel_worker_tree(
        &self,
        request: ParallelWorkerTreeRequest<'_>,
    ) -> std::result::Result<ParallelWorkerTreeSnapshot, String> {
        assert!(self.held.load(Ordering::SeqCst));
        assert_eq!(
            Path::new(request.workspace_directory),
            Path::new(&self.lease.worktree_path)
        );
        assert_eq!(request.expected_branch_name, self.lease.branch_name);
        assert_eq!(
            request.expected_base_commit_sha,
            self.lease
                .delivery_target
                .as_ref()
                .unwrap()
                .integration_base_commit_sha
        );
        assert_eq!(request.expected_head_commit_sha, "a".repeat(40));
        let count = self.snapshots.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(count <= 2);
        if count == 2 {
            assert_eq!(
                self.runner.reads.load(Ordering::SeqCst),
                1,
                "the initial result used environment A before the last tree observation"
            );
            if let Some(next) = &self.change_on_final_snapshot {
                self.runner.set(next.clone());
            }
        }
        Ok(ParallelWorkerTreeSnapshot {
            head_commit_sha: "a".repeat(40),
            tree_sha: "c".repeat(40),
            worktree_clean: false,
        })
    }
    fn paths_match_securely(&self, left: &Path, right: &Path) -> bool {
        self.known_path(left) && self.known_path(right) && left == right
    }
    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf {
        assert!(self.known_path(path), "unexpected path: {path:?}");
        path.to_path_buf()
    }
    fn canonicalize_path(&self, path: &Path) -> std::io::Result<PathBuf> {
        assert!(self.known_path(path), "unexpected path: {path:?}");
        Ok(path.to_path_buf())
    }
    fn path_is_symlink(&self, path: &Path) -> std::io::Result<bool> {
        assert!(self.known_path(path), "unexpected path: {path:?}");
        Ok(false)
    }
    fn path_exists(&self, path: &Path) -> bool {
        self.known_path(path)
    }
    fn current_timestamp(&self) -> String {
        AT.into()
    }
    fn find_executable(&self, _: &str) -> Option<PathBuf> {
        panic!("query must not resolve an executable");
    }
    fn command_succeeds(&self, _: &str, _: &[&str]) -> bool {
        panic!("unexpected process execution");
    }
    fn run_command_with_stdin(&self, _: &str, _: &[&str], _: &str) -> Option<String> {
        panic!("unexpected process execution");
    }
    fn gh_auth_status(&self, _: Option<&str>) -> bool {
        panic!("query must not contact GitHub");
    }
    fn ensure_directory_exists(&self, _: &Path) -> std::io::Result<()> {
        panic!("query must not create directories");
    }
    fn write_runtime_mirror_atomic(&self, _: &Path, _: &Path, _: &str) -> std::io::Result<()> {
        panic!("query must not write mirrors");
    }
    fn read_runtime_mirror_optional(&self, _: &Path, _: &Path) -> std::io::Result<Option<String>> {
        panic!("authority already owns the lease snapshot");
    }
    fn read_runtime_mirror_directory(
        &self,
        _: &Path,
        _: &Path,
    ) -> std::io::Result<Vec<(PathBuf, String)>> {
        panic!("authority already owns the lease snapshot");
    }
    fn remove_runtime_mirror_file(&self, _: &Path, _: &Path) -> std::io::Result<()> {
        panic!("query must not remove mirrors");
    }
}

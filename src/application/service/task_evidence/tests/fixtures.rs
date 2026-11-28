use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use crate::domain::planning::TaskDefinition;
use crate::domain::planning::evidence::{TaskCriterionEvidence, TaskEvidenceAttempt};

pub(super) const AT: &str = "2026-11-23T00:00:00Z";

pub(super) struct Fixture {
    pub service: TaskEvidenceService,
    pub repository: Arc<Repository>,
    pub artifacts: Arc<Artifacts>,
    pub git: Arc<Git>,
}

impl Fixture {
    pub fn new() -> Self {
        let task: TaskDefinition = serde_json::from_value(serde_json::json!({
            "id":"task-1", "direction_id":"direction-1", "title":"Task", "description":"Run acceptance",
            "acceptance":[{"id":"criterion 1","scenario":"Run checks","channel":"command","essential":true}],
            "acceptance_revision":1, "status":"in_progress", "base_priority":1,
            "depends_on":[], "blocked_by":[], "created_by":"user", "last_updated_by":"user", "updated_at":AT
        })).unwrap();
        let scope = TaskEvidenceScope {
            task_id: task.id.clone(),
            scope_id: "1".repeat(64),
            acceptance_revision: 1,
            criteria_sha256: criteria_sha256(&task.acceptance).unwrap(),
        };
        let repository = Arc::new(Repository {
            snapshot: Mutex::new(TaskEvidenceStoredSnapshot {
                planning_revision: 3,
                evidence_revision: 0,
                tasks: vec![TaskEvidenceTaskSnapshot {
                    reviews: vec![],
                    task,
                    scope,
                    leases: vec![],
                    queue_records: vec![],
                    evidence: vec![],
                    waivers: vec![],
                    host_verifications: vec![],
                }],
                truncated: false,
                next_task_id: None,
            }),
            attempt: Mutex::new(None),
            reads: AtomicUsize::new(0),
            change_on_second_read: AtomicBool::new(false),
        });
        let artifacts = Arc::new(Artifacts {
            reads: AtomicUsize::new(0),
            available: AtomicBool::new(true),
            unavailable_attempt: Mutex::new(None),
        });
        let git = Arc::new(Git {
            reads: AtomicUsize::new(0),
            tree: Mutex::new("c".repeat(40)),
        });
        let service = TaskEvidenceService::new(
            "/repo".into(),
            repository.clone(),
            artifacts.clone(),
            git.clone(),
            Arc::new(NoopPlanningAuthorityPort::default()),
        );
        Self {
            service,
            repository,
            artifacts,
            git,
        }
    }

    pub fn install_record(&self, status: TaskEvidenceStatus) -> TaskEvidenceReceipt {
        let mut snapshot = self.repository.snapshot.lock().unwrap();
        let scope = snapshot.tasks[0].scope.clone();
        let tree = TaskEvidenceTree {
            head_commit_sha: "a".repeat(40),
            tree_sha: "c".repeat(40),
            worktree_clean: false,
        };
        let lease = lease();
        let attempt = TaskEvidenceAttempt {
            attempt_id: "2".repeat(64),
            request_id: "request-1".into(),
            scope,
            criterion_id: "criterion 1".into(),
            attempt: 1,
            slot_id: lease.slot_id.clone(),
            lease_generation: lease.lease_generation.clone().unwrap(),
            session_key: lease.session_key(),
            prepared_tree: tree.clone(),
            prepared_at: AT.into(),
        };
        let receipt = TaskEvidenceReceipt {
            event_id: "evidence-event-1".into(),
            evidence_revision: 1,
            evidence: TaskCriterionEvidence {
                attempt: attempt.clone(),
                status,
                artifact: TaskEvidenceArtifact {
                    sha256: "4".repeat(64),
                    size_bytes: 64,
                },
                recorded_tree: tree,
                note: Some("worker log".into()),
                by: "worker".into(),
                recorded_at: AT.into(),
            },
        };
        receipt.validate().unwrap();
        snapshot.evidence_revision = 1;
        snapshot.tasks[0].evidence = vec![receipt.clone()];
        snapshot.tasks[0].queue_records = vec![queue(&lease.session_key())];
        *self.repository.attempt.lock().unwrap() = Some(StoredTaskEvidenceAttempt {
            attempt,
            lease,
            receipt: Some(receipt.clone()),
        });
        receipt
    }
}

pub(super) fn lease() -> ParallelModeSlotLeaseSnapshot {
    ParallelModeSlotLeaseSnapshot::new(
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
}

fn queue(session: &str) -> PlanningAuthorityDistributorQueueRecord {
    serde_json::from_value(serde_json::json!({
        "queue_item_id":"queue-1", "session_key":session, "slot_id":"slot-1", "agent_id":"worker", "task_id":"task-1", "task_title":"Task",
        "delivery_target":{"push_remote":"origin","github_repository":"owner/repo","repository_visibility":"private","integration_branch":"prerelease"},
        "source_branch":"work/task", "source_base_commit_sha":"a".repeat(40), "source_commit_sha":"b".repeat(40),
        "branch_name":"work/task", "worktree_path":"/worker", "commit_sha":"b".repeat(40), "validation_summary":"reported",
        "authority_refresh_outcome":"ready", "queue_state":"queued", "integration_note":"", "enqueued_at":AT,"updated_at":AT
    })).unwrap()
}

pub(super) fn context() -> StoredCommandContext {
    StoredCommandContext {
        human_confirmation: None,
        caller: Some("worker".into()),
        channel: "kernel".into(),
        at: AT.into(),
    }
}

pub(super) fn waiver_request() -> WaiveTaskCriterionRequest {
    WaiveTaskCriterionRequest {
        request_id: "waiver-1".into(),
        task_id: "task-1".into(),
        criterion_id: "criterion 1".into(),
        scope_id: "1".repeat(64),
        expected_acceptance_revision: 1,
        reason: " manual exception ".into(),
    }
}

pub(super) struct Repository {
    pub snapshot: Mutex<TaskEvidenceStoredSnapshot>,
    attempt: Mutex<Option<StoredTaskEvidenceAttempt>>,
    reads: AtomicUsize,
    pub change_on_second_read: AtomicBool,
}

impl crate::application::port::outbound::task_verification_repository_port::TaskVerificationRepositoryPort for Repository {}

impl TaskEvidenceRepositoryPort for Repository {
    fn load_current_snapshot(
        &self,
        _: &str,
        _: &TaskEvidenceSnapshotQuery,
    ) -> Result<TaskEvidenceStoredSnapshot> {
        let mut value = self.snapshot.lock().unwrap().clone();
        let read = self.reads.fetch_add(1, Ordering::SeqCst);
        if read == 1 && self.change_on_second_read.load(Ordering::SeqCst) {
            value.evidence_revision += 1;
        }
        Ok(value)
    }
    fn load_attempt(&self, _: &str, id: &str) -> Result<Option<StoredTaskEvidenceAttempt>> {
        Ok(self
            .attempt
            .lock()
            .unwrap()
            .as_ref()
            .filter(|value| value.attempt.attempt_id == id)
            .cloned())
    }
    fn prepare_attempt(
        &self,
        _: &str,
        _: &TaskEvidencePrepareCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceAttempt>> {
        panic!("no authority write before current lease guard")
    }
    fn record_evidence(
        &self,
        _: &str,
        _: &TaskEvidenceRecordCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceReceipt>> {
        panic!("duplicate records must not write again")
    }
    fn waive_criterion(
        &self,
        _: &str,
        commit: &TaskCriterionWaiverCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskCriterionWaiverReceipt>> {
        let (reason, reason_digest) = normalize_waiver_reason(&commit.reason)?;
        let receipt = TaskCriterionWaiverReceipt {
            event_id: "waiver-event-1".into(),
            evidence_revision: 2,
            waiver: TaskCriterionWaiver {
                request_id: commit.request_id.clone(),
                scope: commit.scope.clone(),
                criterion_id: commit.criterion_id.clone(),
                reason,
                reason_digest,
                confirmation_ref: commit.confirmation_ref.clone(),
                recorded_at: commit.at.clone(),
            },
        };
        receipt.validate()?;
        let mut snapshot = self.snapshot.lock().unwrap();
        snapshot.evidence_revision = 2;
        snapshot.tasks[0].waivers = vec![receipt.clone()];
        Ok(TaskEvidenceWriteResult {
            value: receipt,
            changed: true,
        })
    }
    fn pending_events(&self, _: &str, _: usize) -> Result<Vec<TaskEvidenceEvent>> {
        Ok(vec![])
    }
    fn acknowledge_event(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
}

pub(super) struct Artifacts {
    pub reads: AtomicUsize,
    pub available: AtomicBool,
    pub unavailable_attempt: Mutex<Option<String>>,
}
impl TaskEvidenceArtifactPort for Artifacts {
    fn prepare_lease_incoming_root(&self, _: &str, _: &str) -> Result<PathBuf> {
        panic!("query must not create files")
    }
    fn prepare_artifact(&self, _: &str, _: &TaskEvidenceAttempt) -> Result<PathBuf> {
        panic!("rejected prepare must not create files")
    }
    fn observe_and_seal(
        &self,
        _: &str,
        _: &TaskEvidenceAttempt,
        _: &TaskEvidenceTree,
    ) -> Result<TaskEvidenceArtifact> {
        panic!("a replay must not recapture bytes")
    }
    fn verify_sealed(
        &self,
        _: &str,
        attempt: &TaskEvidenceAttempt,
        _: &TaskEvidenceArtifact,
    ) -> Result<()> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.available.load(Ordering::SeqCst)
            && self.unavailable_attempt.lock().unwrap().as_deref() != Some(&attempt.attempt_id)
        {
            Ok(())
        } else {
            Err(TaskEvidenceError::ArtifactMissing.into())
        }
    }
}

pub(super) struct Git {
    pub reads: AtomicUsize,
    pub tree: Mutex<String>,
}
impl ParallelModeRuntimePort for Git {
    fn read_commit_tree(&self, _: &str, sha: &str) -> std::result::Result<String, String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if sha != "b".repeat(40) {
            return Err("unknown immutable object".into());
        }
        Ok(self.tree.lock().unwrap().clone())
    }
    fn detect_git_repo_root(&self, _: &str) -> Option<String> {
        None
    }
    fn command_succeeds(&self, _: &str, _: &[&str]) -> bool {
        panic!("unexpected external command")
    }
    fn run_command(&self, _: &str, _: &[&str], _: Option<&str>) -> Option<String> {
        panic!("unexpected external command")
    }
    fn run_command_with_stdin(&self, _: &str, _: &[&str], _: &str) -> Option<String> {
        panic!("unexpected external command")
    }
    fn find_executable(&self, _: &str) -> Option<PathBuf> {
        None
    }
    fn gh_auth_status(&self, _: Option<&str>) -> bool {
        panic!("query must not contact GitHub")
    }
    fn current_timestamp(&self) -> String {
        AT.into()
    }
    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }
    fn path_exists(&self, _: &Path) -> bool {
        false
    }
    fn ensure_directory_exists(&self, _: &Path) -> std::io::Result<()> {
        panic!("query must not create directories")
    }
    fn write_runtime_mirror_atomic(&self, _: &Path, _: &Path, _: &str) -> std::io::Result<()> {
        panic!("query must not mutate mirrors")
    }
    fn read_runtime_mirror_optional(&self, _: &Path, _: &Path) -> std::io::Result<Option<String>> {
        Ok(None)
    }
    fn read_runtime_mirror_directory(
        &self,
        _: &Path,
        _: &Path,
    ) -> std::io::Result<Vec<(PathBuf, String)>> {
        Ok(vec![])
    }
    fn remove_runtime_mirror_file(&self, _: &Path, _: &Path) -> std::io::Result<()> {
        panic!("query must not delete mirrors")
    }
}

pub(crate) fn inert_verification_ports() -> (
    Arc<dyn TaskEvidenceArtifactPort>,
    Arc<dyn ParallelModeRuntimePort>,
) {
    let fixture = Fixture::new();
    (fixture.artifacts, fixture.git)
}

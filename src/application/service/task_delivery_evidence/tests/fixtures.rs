use super::*;
use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkPermitDecision;
use crate::application::port::outbound::task_delivery_evidence_repository_port::TaskDeliveryEvidenceObservationCommitResult;
use crate::application::port::outbound::task_verification_repository_port::TaskVerificationRepositoryPort;
use crate::application::port::outbound::task_verification_runner_port::{
    PreparedTaskVerification, TaskVerificationPrepareRequest,
};
use crate::domain::planning::TaskStatus;
use crate::domain::planning::evidence::*;
use crate::domain::planning::verification::*;
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(super) const AT: &str = "2026-11-23T12:00:00Z";
pub(super) type Hook = Box<dyn FnOnce() + Send>;

pub(super) struct Fixture {
    pub service: TaskDeliveryEvidenceService,
    pub repository: Arc<Repository>,
    pub runtime: Arc<Runtime>,
    pub artifacts: Arc<Artifacts>,
    pub runner: Arc<Runner>,
}

impl Fixture {
    pub fn new(mode: TaskEvidenceDeliveryMode) -> Self {
        let host = crate::application::service::task_verification::tests::Fixture::new();
        let claim = host.start();
        host.deliver_executed(&claim);
        assert!(host.service.poll(host.service.next_deadline().unwrap()));
        let stored = host
            .repository
            .load_host_verification("/repo", &claim.stored.attempt.attempt_id)
            .unwrap()
            .unwrap();
        let mut task = host.repository.snapshot.lock().unwrap().tasks[0].clone();
        let manifest = stored.manifest.as_ref().unwrap().clone();
        let mut queue: PlanningAuthorityDistributorQueueRecord = serde_json::from_value(serde_json::json!({
            "queue_item_id":"queue-1", "session_key":stored.attempt.session_key,
            "slot_id":stored.attempt.slot_id, "agent_id":"worker", "task_id":task.task.id,
            "task_title":"Task", "source_branch":"akra-agent/task-1",
            "source_base_commit_sha":manifest.base_commit_sha, "source_commit_sha":"e".repeat(40),
            "branch_name":"akra-agent/task-1", "worktree_path":"/worker", "commit_sha":"e".repeat(40),
            "validation_summary":"host result", "authority_refresh_outcome":"done",
            "queue_state":"queued", "integration_note":"queued", "enqueued_at":AT, "updated_at":AT
        })).unwrap();
        queue.evidence_binding = Some(TaskEvidenceDeliveryBinding {
            scope: task.scope.clone(),
            lease_generation: stored.attempt.lease_generation.clone(),
        });
        task.task.status = TaskStatus::Done;
        task.leases.clear();
        task.queue_records = vec![queue.clone()];
        task.host_verifications = vec![stored];
        let repository = Arc::new(Repository {
            snapshot: Mutex::new(StoredTaskDeliveryEvidence {
                planning_revision: 3,
                evidence_revision: 5,
                queue,
                task: Some(task),
                permit: PlanningWorkPermitDecision::Legacy,
                requires_preservation: false,
            }),
            reads: AtomicUsize::new(0),
            commits: AtomicUsize::new(0),
            on_second: Mutex::new(None),
            on_commit: Mutex::new(None),
            commit_error: Mutex::new(None),
            observations: Mutex::new(Vec::new()),
        });
        let runtime = Arc::new(Runtime {
            trees: Mutex::new(BTreeMap::from([
                (
                    manifest.base_commit_sha.clone(),
                    manifest.base_tree_sha.clone(),
                ),
                ("e".repeat(40), manifest.source.tree_sha.clone()),
                ("f".repeat(40), manifest.source.tree_sha.clone()),
            ])),
            reads: AtomicUsize::new(0),
        });
        let runner = Arc::new(Runner {
            current: Mutex::new(Ok(manifest.environment_sha256)),
            sequence: Mutex::new(VecDeque::new()),
            reads: AtomicUsize::new(0),
        });
        let artifacts = Arc::new(Artifacts {
            available: AtomicBool::new(true),
            on_host_read: Mutex::new(None),
            host_reads: AtomicUsize::new(0),
        });
        let service = TaskDeliveryEvidenceService::new(
            TaskDeliveryEvidenceDependencies {
                workspace: "/repo".into(),
                repository_identity: "/repo".into(),
                repository: repository.clone(),
                artifacts: artifacts.clone(),
                runtime: runtime.clone(),
                runner: runner.clone(),
                authority: Arc::new(
                    NoopPlanningAuthorityPort::default().with_canonical_repo_root("/repo"),
                ),
            },
            Ok(mode),
        );
        Self {
            service,
            repository,
            runtime,
            artifacts,
            runner,
        }
    }

    pub fn effect(&self) -> Result<TaskDeliveryEvidenceDecision, String> {
        let queue = self.repository.snapshot.lock().unwrap().queue.clone();
        self.service.evaluate(
            "/repo",
            &queue,
            TaskEvidenceDeliveryStage::Push,
            None,
            &TaskDeliveryEvidenceOperation::Effect {
                owner_token: "claim-owner".into(),
            },
        )
    }

    pub fn change(&self, change: impl FnOnce(&mut StoredTaskDeliveryEvidence)) {
        change(&mut self.repository.snapshot.lock().unwrap());
    }

    pub fn synchronize_scope(&self) {
        self.change(|snapshot| {
            let task = snapshot.task.as_mut().unwrap();
            task.scope.criteria_sha256 = criteria_sha256(&task.task.acceptance).unwrap();
            snapshot.queue.evidence_binding.as_mut().unwrap().scope = task.scope.clone();
            for host in &mut task.host_verifications {
                host.attempt.scope = task.scope.clone();
                if let Some(receipt) = &mut host.receipt {
                    receipt.attempt = host.attempt.clone();
                }
            }
        });
    }

    pub fn install_report(&self, status: TaskEvidenceStatus) {
        self.change(|snapshot| {
            let task = snapshot.task.as_mut().unwrap();
            let host = &task.host_verifications[0];
            let tree = host.manifest.as_ref().unwrap().source.clone();
            let attempt = TaskEvidenceAttempt {
                attempt_id: "6".repeat(64),
                request_id: "report".into(),
                scope: task.scope.clone(),
                criterion_id: host.attempt.criterion_id.clone(),
                attempt: 2,
                slot_id: host.attempt.slot_id.clone(),
                lease_generation: host.attempt.lease_generation.clone(),
                session_key: host.attempt.session_key.clone(),
                prepared_tree: tree.clone(),
                prepared_at: AT.into(),
            };
            let receipt = TaskEvidenceReceipt {
                event_id: "reported-event".into(),
                evidence_revision: 3,
                evidence: TaskCriterionEvidence {
                    attempt,
                    status,
                    artifact: TaskEvidenceArtifact {
                        sha256: "7".repeat(64),
                        size_bytes: 64,
                    },
                    recorded_tree: tree,
                    note: None,
                    by: "worker".into(),
                    recorded_at: AT.into(),
                },
            };
            receipt.validate().unwrap();
            task.evidence = vec![receipt];
        });
    }

    pub fn waive(&self) {
        self.change(|snapshot| {
            let task = snapshot.task.as_mut().unwrap();
            let (reason, reason_digest) = normalize_waiver_reason("Reviewed manually").unwrap();
            let receipt = TaskCriterionWaiverReceipt {
                event_id: "waiver-event".into(),
                evidence_revision: 4,
                waiver: TaskCriterionWaiver {
                    request_id: "human-waiver".into(),
                    scope: task.scope.clone(),
                    criterion_id: task.task.acceptance[0].id.clone(),
                    reason,
                    reason_digest,
                    confirmation_ref: "native-confirmation".into(),
                    recorded_at: AT.into(),
                },
            };
            receipt.validate().unwrap();
            task.waivers = vec![receipt];
        });
    }

    pub fn last(&self) -> TaskDeliveryEvidenceObservation {
        self.repository
            .observations
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .clone()
    }
}

pub(super) struct Repository {
    pub snapshot: Mutex<StoredTaskDeliveryEvidence>,
    pub reads: AtomicUsize,
    pub commits: AtomicUsize,
    pub on_second: Mutex<Option<Hook>>,
    pub on_commit: Mutex<Option<Hook>>,
    pub commit_error: Mutex<Option<TaskDeliveryEvidenceError>>,
    pub observations: Mutex<Vec<TaskDeliveryEvidenceObservation>>,
}

impl TaskDeliveryEvidenceRepositoryPort for Repository {
    fn load_delivery_evidence(
        &self,
        _: &str,
        expected: &PlanningAuthorityDistributorQueueRecord,
        _: TaskEvidenceDeliveryStage,
    ) -> anyhow::Result<StoredTaskDeliveryEvidence> {
        if self.reads.fetch_add(1, Ordering::SeqCst) == 1
            && let Some(hook) = self.on_second.lock().unwrap().take()
        {
            hook();
        }
        let snapshot = self.snapshot.lock().unwrap().clone();
        if snapshot.queue != *expected {
            return Err(TaskDeliveryEvidenceError::Conflict.into());
        }
        Ok(snapshot)
    }
    fn commit_delivery_evidence_observation(
        &self,
        _: &str,
        commit: &TaskDeliveryEvidenceObservationCommit,
    ) -> anyhow::Result<TaskDeliveryEvidenceObservationCommitResult> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.on_commit.lock().unwrap().take() {
            hook();
        }
        if let Some(error) = self.commit_error.lock().unwrap().take() {
            return Err(error.into());
        }
        let snapshot = self.snapshot.lock().unwrap();
        anyhow::ensure!(*snapshot == commit.expected, "authority CAS failed");
        commit.observation.validate()?;
        self.observations
            .lock()
            .unwrap()
            .push(commit.observation.clone());
        Ok(TaskDeliveryEvidenceObservationCommitResult {
            updated_queue: snapshot.queue.clone(),
            observation: commit.observation.clone(),
            changed: true,
            effect_allowed: matches!(
                commit.operation,
                TaskDeliveryEvidenceOperation::Effect { .. }
            ) && commit.observation.allows_delivery(),
        })
    }
}

pub(super) struct Runner {
    pub current: Mutex<Result<String, TaskVerificationNotRunReason>>,
    pub sequence: Mutex<VecDeque<Result<String, TaskVerificationNotRunReason>>>,
    pub reads: AtomicUsize,
}
impl TaskVerificationRunnerPort for Runner {
    fn prepare(
        &self,
        _: TaskVerificationPrepareRequest<'_>,
        _: &dyn Fn() -> bool,
    ) -> Result<Box<dyn PreparedTaskVerification>, TaskVerificationNotRunReason> {
        panic!("delivery evaluation must never execute or prepare a verification")
    }
    fn environment_fingerprint(
        &self,
        _: &TaskVerificationExecutionManifest,
    ) -> Result<String, TaskVerificationNotRunReason> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.sequence
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| self.current.lock().unwrap().clone())
    }
}

pub(super) struct Artifacts {
    pub available: AtomicBool,
    pub on_host_read: Mutex<Option<Hook>>,
    pub host_reads: AtomicUsize,
}
impl TaskEvidenceArtifactPort for Artifacts {
    fn prepare_lease_incoming_root(&self, _: &str, _: &str) -> anyhow::Result<PathBuf> {
        panic!("no incoming")
    }
    fn prepare_artifact(&self, _: &str, _: &TaskEvidenceAttempt) -> anyhow::Result<PathBuf> {
        panic!("no preparation")
    }
    fn observe_and_seal(
        &self,
        _: &str,
        _: &TaskEvidenceAttempt,
        _: &TaskEvidenceTree,
    ) -> anyhow::Result<TaskEvidenceArtifact> {
        panic!("no worker capture")
    }
    fn verify_sealed(
        &self,
        _: &str,
        _: &TaskEvidenceAttempt,
        _: &TaskEvidenceArtifact,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(self.available.load(Ordering::SeqCst), "missing artifact");
        Ok(())
    }
    fn verify_host_capture(
        &self,
        _: &str,
        _: &str,
        _: &TaskEvidenceArtifact,
    ) -> anyhow::Result<()> {
        self.host_reads.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.on_host_read.lock().unwrap().take() {
            hook();
        }
        anyhow::ensure!(self.available.load(Ordering::SeqCst), "missing capture");
        Ok(())
    }
}

pub(super) struct Runtime {
    pub trees: Mutex<BTreeMap<String, String>>,
    pub reads: AtomicUsize,
}
impl ParallelModeRuntimePort for Runtime {
    fn read_commit_tree(&self, _: &str, sha: &str) -> Result<String, String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.trees
            .lock()
            .unwrap()
            .get(sha)
            .cloned()
            .ok_or_else(|| "unknown commit".into())
    }
    fn paths_match_securely(&self, a: &Path, b: &Path) -> bool {
        a == b
    }
    fn detect_git_repo_root(&self, _: &str) -> Option<String> {
        None
    }
    fn command_succeeds(&self, _: &str, _: &[&str]) -> bool {
        panic!("no command")
    }
    fn run_command(&self, _: &str, _: &[&str], _: Option<&str>) -> Option<String> {
        panic!("no command")
    }
    fn run_command_with_stdin(&self, _: &str, _: &[&str], _: &str) -> Option<String> {
        panic!("no command")
    }
    fn find_executable(&self, _: &str) -> Option<PathBuf> {
        None
    }
    fn gh_auth_status(&self, _: Option<&str>) -> bool {
        panic!("no GitHub")
    }
    fn current_timestamp(&self) -> String {
        AT.into()
    }
    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf {
        path.into()
    }
    fn path_exists(&self, _: &Path) -> bool {
        false
    }
    fn ensure_directory_exists(&self, _: &Path) -> std::io::Result<()> {
        panic!("no files")
    }
    fn write_runtime_mirror_atomic(&self, _: &Path, _: &Path, _: &str) -> std::io::Result<()> {
        panic!("no writes")
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
        panic!("no removal")
    }
}

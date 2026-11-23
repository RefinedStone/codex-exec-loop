use std::collections::VecDeque;

use super::*;
use crate::application::port::outbound::task_verification_runner_port::{
    PreparedTaskVerification, TaskVerificationPrepareRequest,
};
use crate::domain::kernel_event::sample_task_verification_event;
use crate::domain::parallel_mode::{
    ParallelModeDeliveryTargetSnapshot, ParallelModeRepositoryVisibility,
};

pub(super) const ENVIRONMENT_A: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(super) const ENVIRONMENT_B: &str =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

pub(super) struct HostFixture {
    pub base: Fixture,
    pub service: TaskEvidenceService,
    pub repository: Arc<HookRepository>,
    pub artifacts: Arc<HostArtifacts>,
    pub runner: Arc<Runner>,
}

impl HostFixture {
    pub fn new() -> Self {
        let base = Fixture::new();
        select_recipe(&base);
        base.install_record(TaskEvidenceStatus::Pass);
        Self::from_base(base)
    }

    pub fn empty() -> Self {
        let base = Fixture::new();
        select_recipe(&base);
        Self::from_base(base)
    }

    fn from_base(base: Fixture) -> Self {
        let repository = Arc::new(HookRepository {
            inner: base.repository.clone(),
            reads: AtomicUsize::new(0),
            on_second: Mutex::new(None),
        });
        let artifacts = Arc::new(HostArtifacts {
            reported: base.artifacts.clone(),
            available: AtomicBool::new(true),
            reads: AtomicUsize::new(0),
        });
        let runner = Arc::new(Runner {
            value: Mutex::new(Ok(ENVIRONMENT_A.into())),
            sequence: Mutex::new(VecDeque::new()),
            reads: AtomicUsize::new(0),
        });
        let service = TaskEvidenceService::new(
            "/repo".into(),
            repository.clone(),
            artifacts.clone(),
            base.git.clone(),
            Arc::new(NoopPlanningAuthorityPort::default()),
        )
        .with_verification_runner(runner.clone());
        Self {
            base,
            service,
            repository,
            artifacts,
            runner,
        }
    }

    pub fn query(&self) -> TaskEvidencePlanningSnapshot {
        self.service
            .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
            .unwrap()
    }

    pub fn install_host(&self, phase: TaskVerificationPhase) -> StoredTaskVerification {
        let TaskEvidenceEvent::VerificationFinished { receipt } = sample_task_verification_event()
        else {
            unreachable!()
        };
        let mut receipt = *receipt;
        let mut snapshot = self.base.repository.snapshot.lock().unwrap();
        let task = &snapshot.tasks[0];
        let lease = lease().with_delivery_target(ParallelModeDeliveryTargetSnapshot::new(
            "origin",
            "owner/repo",
            ParallelModeRepositoryVisibility::Private,
            "prerelease",
            "a".repeat(40),
        ));
        receipt.attempt.scope = task.scope.clone();
        receipt.attempt.criterion_id = task.task.acceptance[0].id.clone();
        receipt.attempt.slot_id = lease.slot_id.clone();
        receipt.attempt.lease_generation = lease.lease_generation.clone().unwrap();
        receipt.attempt.session_key = lease.session_key();
        let manifest = receipt.manifest.as_mut().unwrap();
        manifest.base_commit_sha = "a".repeat(40);
        manifest.base_tree_sha = "e".repeat(40);
        manifest.source.head_commit_sha = "a".repeat(40);
        manifest.source.tree_sha = "c".repeat(40);
        manifest.environment_sha256 = ENVIRONMENT_A.into();
        receipt.evidence_revision = 2;
        receipt.validate().unwrap();
        let stored = StoredTaskVerification {
            attempt: receipt.attempt.clone(),
            lease,
            phase,
            manifest: (phase != TaskVerificationPhase::Pending)
                .then(|| receipt.manifest.clone().unwrap()),
            receipt: (phase == TaskVerificationPhase::Finished).then_some(receipt),
        };
        snapshot.evidence_revision = 2;
        snapshot.tasks[0].host_verifications = vec![stored.clone()];
        stored
    }

    pub fn edit_host(&self, change: impl FnOnce(&mut StoredTaskVerification)) {
        let mut snapshot = self.base.repository.snapshot.lock().unwrap();
        change(&mut snapshot.tasks[0].host_verifications[0]);
    }

    pub fn add_second_criterion(&self) {
        let mut snapshot = self.base.repository.snapshot.lock().unwrap();
        let task = &mut snapshot.tasks[0];
        let mut criterion = task.task.acceptance[0].clone();
        criterion.id = "criterion 2".into();
        task.task.acceptance.push(criterion);
        task.scope.criteria_sha256 = criteria_sha256(&task.task.acceptance).unwrap();
    }

    pub fn add_second_host(&self, environment: &str) {
        let mut snapshot = self.base.repository.snapshot.lock().unwrap();
        let mut second = snapshot.tasks[0].host_verifications[0].clone();
        second.attempt.attempt_id = "8".repeat(64);
        second.attempt.request_id = "host-second".into();
        second.attempt.criterion_id = "criterion 2".into();
        second.manifest.as_mut().unwrap().environment_sha256 = environment.into();
        let receipt = second.receipt.as_mut().unwrap();
        receipt.event_id = format!("task-verification-finished:{}", second.attempt.attempt_id);
        receipt.evidence_revision = 3;
        receipt.attempt = second.attempt.clone();
        receipt.manifest = second.manifest.clone();
        receipt.validate().unwrap();
        snapshot.evidence_revision = 3;
        snapshot.tasks[0].host_verifications.push(second);
    }
}

fn select_recipe(base: &Fixture) {
    let mut snapshot = base.repository.snapshot.lock().unwrap();
    let task = &mut snapshot.tasks[0];
    task.task.acceptance[0].verification_recipe = Some(TaskVerificationRecipe::GitDiffCheckV1);
    task.scope.criteria_sha256 = criteria_sha256(&task.task.acceptance).unwrap();
}

pub(super) struct HookRepository {
    inner: Arc<Repository>,
    pub reads: AtomicUsize,
    on_second: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl HookRepository {
    pub fn after_second_read(&self, hook: impl FnOnce() + Send + 'static) {
        *self.on_second.lock().unwrap() = Some(Box::new(hook));
    }
}

impl TaskVerificationRepositoryPort for HookRepository {}
impl TaskEvidenceRepositoryPort for HookRepository {
    fn load_current_snapshot(
        &self,
        workspace: &str,
        query: &TaskEvidenceSnapshotQuery,
    ) -> Result<TaskEvidenceStoredSnapshot> {
        let result = self.inner.load_current_snapshot(workspace, query)?;
        if self.reads.fetch_add(1, Ordering::SeqCst) == 1 {
            let hook = self.on_second.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        Ok(result)
    }
    fn load_attempt(&self, _: &str, _: &str) -> Result<Option<StoredTaskEvidenceAttempt>> {
        panic!("query must not load or alter an evidence reservation")
    }
    fn prepare_attempt(
        &self,
        _: &str,
        _: &TaskEvidencePrepareCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceAttempt>> {
        panic!("query must not reserve execution")
    }
    fn record_evidence(
        &self,
        _: &str,
        _: &TaskEvidenceRecordCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceReceipt>> {
        panic!("query must not record worker evidence")
    }
    fn waive_criterion(
        &self,
        _: &str,
        _: &TaskCriterionWaiverCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskCriterionWaiverReceipt>> {
        panic!("query must not grant a waiver")
    }
    fn pending_events(&self, _: &str, _: usize) -> Result<Vec<TaskEvidenceEvent>> {
        panic!("query must not consume the outbox")
    }
    fn acknowledge_event(&self, _: &str, _: &str) -> Result<()> {
        panic!("query must not acknowledge the outbox")
    }
}

pub(super) struct HostArtifacts {
    reported: Arc<Artifacts>,
    pub available: AtomicBool,
    pub reads: AtomicUsize,
}

impl TaskEvidenceArtifactPort for HostArtifacts {
    fn prepare_lease_incoming_root(&self, _: &str, _: &str) -> Result<PathBuf> {
        panic!("query must not create files")
    }
    fn prepare_artifact(&self, _: &str, _: &TaskEvidenceAttempt) -> Result<PathBuf> {
        panic!("query must not reserve files")
    }
    fn observe_and_seal(
        &self,
        _: &str,
        _: &TaskEvidenceAttempt,
        _: &TaskEvidenceTree,
    ) -> Result<TaskEvidenceArtifact> {
        panic!("query must not capture worker output")
    }
    fn verify_sealed(
        &self,
        workspace: &str,
        attempt: &TaskEvidenceAttempt,
        artifact: &TaskEvidenceArtifact,
    ) -> Result<()> {
        self.reported.verify_sealed(workspace, attempt, artifact)
    }
    fn seal_host_capture(&self, _: &str, _: &str, _: &[u8]) -> Result<TaskEvidenceArtifact> {
        panic!("query must not create host capture")
    }
    fn verify_host_capture(&self, _: &str, _: &str, _: &TaskEvidenceArtifact) -> Result<()> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.available.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(TaskEvidenceError::ArtifactMissing.into())
        }
    }
}

pub(super) struct Runner {
    value: Mutex<std::result::Result<String, TaskVerificationNotRunReason>>,
    sequence: Mutex<VecDeque<std::result::Result<String, TaskVerificationNotRunReason>>>,
    pub reads: AtomicUsize,
}

impl Runner {
    pub fn set(&self, value: std::result::Result<String, TaskVerificationNotRunReason>) {
        *self.value.lock().unwrap() = value;
    }
    pub fn sequence(&self, values: Vec<std::result::Result<String, TaskVerificationNotRunReason>>) {
        *self.sequence.lock().unwrap() = values.into();
    }
}

impl TaskVerificationRunnerPort for Runner {
    fn prepare(
        &self,
        _: TaskVerificationPrepareRequest<'_>,
        _: &dyn Fn() -> bool,
    ) -> std::result::Result<Box<dyn PreparedTaskVerification>, TaskVerificationNotRunReason> {
        panic!("query must not prepare or run a verification process")
    }
    fn environment_fingerprint(
        &self,
        _: &TaskVerificationExecutionManifest,
    ) -> std::result::Result<String, TaskVerificationNotRunReason> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.sequence
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| self.value.lock().unwrap().clone())
    }
}

use super::*;
use crate::application::port::outbound::task_evidence_repository_port::*;
use crate::application::port::outbound::task_verification_repository_port::*;
use crate::domain::planning::evidence::*;
use std::sync::atomic::AtomicBool;

pub(crate) struct FinishRepository {
    inner: Arc<SqlitePlanningAuthorityAdapter>,
    pub fail_next_finish: AtomicBool,
}

impl FinishRepository {
    pub(super) fn new(inner: Arc<SqlitePlanningAuthorityAdapter>) -> Self {
        Self {
            inner,
            fail_next_finish: AtomicBool::new(false),
        }
    }
}

impl TaskVerificationRepositoryPort for FinishRepository {
    fn claim_host_verification(
        &self,
        workspace: &str,
        commit: &TaskVerificationClaimCommit,
    ) -> anyhow::Result<TaskVerificationClaimResult> {
        self.inner.claim_host_verification(workspace, commit)
    }
    fn bind_host_verification(
        &self,
        workspace: &str,
        commit: &TaskVerificationBindCommit,
    ) -> anyhow::Result<bool> {
        self.inner.bind_host_verification(workspace, commit)
    }
    fn finish_host_verification(
        &self,
        workspace: &str,
        commit: &TaskVerificationFinishCommit,
    ) -> anyhow::Result<TaskEvidenceWriteResult<TaskVerificationReceipt>> {
        if self.fail_next_finish.swap(false, Ordering::SeqCst) {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        self.inner.finish_host_verification(workspace, commit)
    }
    fn load_host_verification(
        &self,
        workspace: &str,
        attempt_id: &str,
    ) -> anyhow::Result<Option<StoredTaskVerification>> {
        self.inner.load_host_verification(workspace, attempt_id)
    }
    fn apply_host_verification_failure_limit(
        &self,
        workspace: &str,
        commit: &TaskVerificationFailureLimitCommit,
    ) -> anyhow::Result<TaskVerificationFailureLimitResult> {
        self.inner
            .apply_host_verification_failure_limit(workspace, commit)
    }
    fn recover_incomplete_host_verifications(
        &self,
        workspace: &str,
        owner: &str,
        at: &str,
    ) -> anyhow::Result<Vec<TaskVerificationReceipt>> {
        self.inner
            .recover_incomplete_host_verifications(workspace, owner, at)
    }
}

impl TaskEvidenceRepositoryPort for FinishRepository {
    fn load_current_snapshot(
        &self,
        workspace: &str,
        query: &TaskEvidenceSnapshotQuery,
    ) -> anyhow::Result<TaskEvidenceStoredSnapshot> {
        self.inner.load_current_snapshot(workspace, query)
    }
    fn load_attempt(
        &self,
        workspace: &str,
        attempt_id: &str,
    ) -> anyhow::Result<Option<StoredTaskEvidenceAttempt>> {
        self.inner.load_attempt(workspace, attempt_id)
    }
    fn prepare_attempt(
        &self,
        workspace: &str,
        commit: &TaskEvidencePrepareCommit,
    ) -> anyhow::Result<TaskEvidenceWriteResult<TaskEvidenceAttempt>> {
        self.inner.prepare_attempt(workspace, commit)
    }
    fn record_evidence(
        &self,
        workspace: &str,
        commit: &TaskEvidenceRecordCommit,
    ) -> anyhow::Result<TaskEvidenceWriteResult<TaskEvidenceReceipt>> {
        self.inner.record_evidence(workspace, commit)
    }
    fn waive_criterion(
        &self,
        workspace: &str,
        commit: &TaskCriterionWaiverCommit,
    ) -> anyhow::Result<TaskEvidenceWriteResult<TaskCriterionWaiverReceipt>> {
        self.inner.waive_criterion(workspace, commit)
    }
    fn pending_events(
        &self,
        workspace: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<TaskEvidenceEvent>> {
        self.inner.pending_events(workspace, limit)
    }
    fn acknowledge_event(&self, workspace: &str, event_id: &str) -> anyhow::Result<()> {
        self.inner.acknowledge_event(workspace, event_id)
    }
}

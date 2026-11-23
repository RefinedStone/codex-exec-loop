use std::sync::{OnceLock, Weak};

use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;

use super::*;

/// All completion paths share one late-bound live owner. Cloning a completion
/// service does not create a verifier, recover runs, or retain the owner.
#[derive(Clone, Default)]
pub struct TaskVerificationHandle {
    owner: Arc<OnceLock<Weak<TaskVerificationService>>>,
}

impl TaskVerificationHandle {
    pub fn attach(&self, service: &Arc<TaskVerificationService>) -> Result<()> {
        if !service.initialized.load(Ordering::Acquire) || service.closed.load(Ordering::Acquire) {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        let proposed = Arc::downgrade(service);
        if let Some(current) = self.owner.get() {
            return if current.ptr_eq(&proposed) {
                Ok(())
            } else {
                Err(TaskEvidenceError::Conflict.into())
            };
        }
        self.owner
            .set(proposed)
            .map_err(|_| TaskEvidenceError::Conflict.into())
    }

    pub fn verify_automatic_completion(
        &self,
        workspace: &str,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        source_commit_sha: &str,
        completed_turn_id: &str,
        continuation: Arc<dyn TaskVerificationContinuation>,
    ) -> Result<AutomaticTaskVerificationOutcome> {
        let owner = self
            .owner
            .get()
            .and_then(Weak::upgrade)
            .ok_or(TaskEvidenceError::Unavailable)?;
        // Native shutdown closes the owner's shared execution flag before it
        // releases the repository lock, even while this bounded call holds Arc.
        owner.verify_automatic_completion(
            workspace,
            expected_lease,
            source_commit_sha,
            completed_turn_id,
            continuation,
        )
    }
}

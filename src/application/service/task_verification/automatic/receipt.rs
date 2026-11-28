use super::*;

impl TaskVerificationService {
    pub(in crate::application::service::task_verification) fn apply_automatic_failure_limit(
        &self,
        receipt: &TaskVerificationReceipt,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        continuation: &dyn TaskVerificationContinuation,
        deadline: Instant,
    ) -> Result<crate::application::port::outbound::task_verification_repository_port::TaskVerificationFailureLimitResult>{
        // shutdown waits for this same admission lock before the Native owner
        // releases the repository lock. A retained Arc cannot write afterward.
        let _flight = self
            .flight
            .lock()
            .map_err(|_| TaskEvidenceError::Unavailable)?;
        self.require_automatic_live(continuation, deadline)?;
        with_verification_commit(continuation, || {
            // The cancellation lock is already held: do not query it recursively.
            if self.closed.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Err(TaskEvidenceError::Unavailable.into());
            }
            self.dependencies
                .repository
                .apply_host_verification_failure_limit(
                    &self.dependencies.workspace,
                    &TaskVerificationFailureLimitCommit {
                        attempt_id: receipt.attempt.attempt_id.clone(),
                        expected_lease: expected_lease.clone(),
                        owner_token: self.owner_token.clone(),
                        at: utc_now(),
                    },
                )
        })
        .ok_or(TaskEvidenceError::Unavailable)?
    }

    pub(in crate::application::service::task_verification) fn wait_for_automatic_receipt(
        &self,
        claimed: &StoredTaskVerification,
        guard: &ExecutionGuard,
        deadline: Instant,
    ) -> Result<StoredTaskVerification> {
        loop {
            self.require_automatic_live(guard.continuation.as_ref(), deadline)?;
            let stored = self.dependencies.repository.load_host_verification(
                &self.dependencies.workspace,
                &claimed.attempt.attempt_id,
            )?;
            let stored = stored.ok_or(TaskEvidenceError::Missing)?;
            if stored.attempt != claimed.attempt || stored.lease != claimed.lease {
                return Err(TaskEvidenceError::Conflict.into());
            }
            if stored.phase == TaskVerificationPhase::Finished {
                let receipt = stored
                    .receipt
                    .as_ref()
                    .ok_or(TaskEvidenceError::Unavailable)?;
                receipt.validate()?;
                if receipt.attempt != claimed.attempt
                    || receipt.manifest != stored.manifest
                    || receipt
                        .manifest
                        .as_ref()
                        .is_some_and(|manifest| manifest.source != guard.source)
                {
                    return Err(TaskEvidenceError::Stale.into());
                }
                return Ok(stored);
            }
            // Native owns finish persistence and retries. A process response or
            // a channel becoming ready cannot release official refresh.
            std::thread::sleep(RECEIPT_POLL_INTERVAL);
        }
    }
}

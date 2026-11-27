//! Explicit essential recipes run before the existing official refresh. A
//! durable result is an observation, never a substitute for the delivery gate.
use sha2::{Digest, Sha256};

use crate::application::port::outbound::task_evidence_repository_port::TaskEvidenceSnapshotQuery;
use crate::application::port::outbound::task_verification_repository_port::{
    TaskVerificationFailureLimitCommit, TaskVerificationFailureLimitOutcome,
};
use crate::application::service::parallel_mode::evidence::{
    snapshot_worker_tree, with_running_worker_until,
};
use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};
use crate::domain::planning::TaskAcceptanceChannel;
use crate::domain::planning::evidence::{TaskEvidenceScope, TaskEvidenceTree, criteria_sha256};
use crate::domain::planning::verification::{TaskVerificationPhase, TaskVerificationReceipt};

use super::*;

mod receipt;

const COMPLETION_WAIT: Duration = Duration::from_secs(300);
const RECEIPT_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomaticTaskVerificationOutcome {
    NotApplicable,
    Finished {
        receipts: Vec<TaskVerificationReceipt>,
    },
    AwaitingUser {
        criterion_id: String,
    },
}

#[derive(Clone)]
pub(super) struct ExecutionGuard {
    pub(super) source: TaskEvidenceTree,
    pub(super) continuation: Arc<dyn TaskVerificationContinuation>,
    pub(super) deadline: Instant,
}

impl TaskVerificationService {
    pub fn verify_automatic_completion(
        &self,
        workspace: &str,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        source_commit_sha: &str,
        completed_turn_id: &str,
        continuation: Arc<dyn TaskVerificationContinuation>,
    ) -> Result<AutomaticTaskVerificationOutcome> {
        self.verify_automatic_completion_until(
            workspace,
            expected_lease,
            source_commit_sha,
            completed_turn_id,
            continuation,
            Instant::now() + COMPLETION_WAIT,
        )
    }

    pub(super) fn verify_automatic_completion_until(
        &self,
        workspace: &str,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        source_commit_sha: &str,
        completed_turn_id: &str,
        continuation: Arc<dyn TaskVerificationContinuation>,
        deadline: Instant,
    ) -> Result<AutomaticTaskVerificationOutcome> {
        self.require_automatic_live(continuation.as_ref(), deadline)?;
        if workspace != self.dependencies.workspace || completed_turn_id.trim().is_empty() {
            return Err(TaskEvidenceError::Invalid.into());
        }
        let snapshot = self.dependencies.repository.load_current_snapshot(
            &self.dependencies.workspace,
            &TaskEvidenceSnapshotQuery {
                task_ids: vec![expected_lease.task_id.clone()],
                after_task_id: None,
                limit: 1,
            },
        )?;
        let [task] = snapshot.tasks.as_slice() else {
            return Err(TaskEvidenceError::Missing.into());
        };
        if task.task.id != expected_lease.task_id
            || task.scope.task_id != task.task.id
            || task.scope.acceptance_revision != task.task.acceptance_revision
            || criteria_sha256(&task.task.acceptance)? != task.scope.criteria_sha256
        {
            return Err(TaskEvidenceError::ScopeChanged.into());
        }
        let criteria = task.task.acceptance.iter().filter(|criterion| {
            criterion.essential
                && criterion.channel == TaskAcceptanceChannel::Command
                && criterion.verification_recipe == Some(
                    crate::domain::planning::verification::TaskVerificationRecipe::GitDiffCheckV1,
                )
                && !task.waivers.iter().any(|waiver| {
                    waiver.waiver.scope == task.scope
                        && waiver.waiver.criterion_id == criterion.id
                        && waiver.validate().is_ok()
                })
        });
        let criteria = criteria.collect::<Vec<_>>();
        if criteria.is_empty() {
            return Ok(AutomaticTaskVerificationOutcome::NotApplicable);
        }
        let [running_lease] = task.leases.as_slice() else {
            return Err(TaskEvidenceError::LeaseChanged.into());
        };
        // Launch captures Leased and the same incarnation later becomes Running.
        // Only that state progression may be adopted; identity and frozen target
        // still belong to the original completion, never to a replacement lease.
        if running_lease.state != ParallelModeSlotLeaseState::Running
            || !running_lease.same_generation_as(expected_lease)
            || running_lease.session_key() != expected_lease.session_key()
        {
            return Err(TaskEvidenceError::LeaseChanged.into());
        }
        let expected_lease = running_lease;
        let source = self.automatic_source(expected_lease, source_commit_sha, deadline)?;
        let guard = ExecutionGuard {
            source: source.clone(),
            continuation,
            deadline,
        };
        let mut receipts = Vec::with_capacity(criteria.len());
        for criterion in criteria {
            let request = automatic_request(
                &task.scope,
                &criterion.id,
                expected_lease,
                &source,
                completed_turn_id,
            )?;
            let cancel_on_exit = CancelAutomaticRequest {
                service: self,
                request_id: request.request_id.clone(),
            };
            let claimed = loop {
                self.require_automatic_live(guard.continuation.as_ref(), deadline)?;
                let context = StoredCommandContext {
                    human_confirmation: None,
                    caller: Some("host".into()),
                    channel: "parallel_completion".into(),
                    at: utc_now(),
                };
                if let Some(claimed) = self.request_guarded(
                    request.clone(),
                    &context,
                    Some(expected_lease.clone()),
                    Some(guard.clone()),
                )? {
                    break claimed;
                }
                // Another completion owns the one workspace flight. This is
                // different from the durable per-criterion attempt quota.
                std::thread::sleep(RECEIPT_POLL_INTERVAL);
            };
            let stored = self.wait_for_automatic_receipt(&claimed.stored, &guard, deadline)?;
            authority::require_current(self.dependencies.as_ref(), &stored)?;
            let receipt = stored.receipt.ok_or(TaskEvidenceError::Unavailable)?;
            let policy = self.apply_automatic_failure_limit(
                &receipt,
                expected_lease,
                guard.continuation.as_ref(),
                deadline,
            )?;
            match policy.outcome {
                TaskVerificationFailureLimitOutcome::AwaitingUser => {
                    return Ok(AutomaticTaskVerificationOutcome::AwaitingUser {
                        criterion_id: criterion.id.clone(),
                    });
                }
                TaskVerificationFailureLimitOutcome::Superseded => {
                    return Err(TaskEvidenceError::Stale.into());
                }
                TaskVerificationFailureLimitOutcome::NotFailed
                | TaskVerificationFailureLimitOutcome::BelowLimit => {}
            }
            receipts.push(receipt);
            drop(cancel_on_exit);
        }
        self.require_automatic_live(guard.continuation.as_ref(), deadline)?;
        if self.automatic_source(expected_lease, source_commit_sha, deadline)? != source {
            return Err(TaskEvidenceError::Stale.into());
        }
        Ok(AutomaticTaskVerificationOutcome::Finished { receipts })
    }

    fn automatic_source(
        &self,
        lease: &ParallelModeSlotLeaseSnapshot,
        commit_sha: &str,
        deadline: Instant,
    ) -> Result<TaskEvidenceTree> {
        // A busy pool (allocation, evidence query) is transient. Wait for it within
        // the completion deadline instead of failing the official completion.
        let source = with_running_worker_until(
            self.dependencies.authority.as_ref(),
            self.dependencies.runtime.as_ref(),
            &self.dependencies.workspace,
            lease,
            Some(deadline),
            || snapshot_worker_tree(self.dependencies.runtime.as_ref(), lease),
        )?;
        if !source.worktree_clean || source.head_commit_sha != commit_sha {
            return Err(TaskEvidenceError::Stale.into());
        }
        source.validate()?;
        Ok(source)
    }

    fn require_automatic_live(
        &self,
        continuation: &dyn TaskVerificationContinuation,
        deadline: Instant,
    ) -> Result<()> {
        if self.closed.load(Ordering::Acquire)
            || !self.initialized.load(Ordering::Acquire)
            || !continuation.is_current()
            || Instant::now() >= deadline
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        Ok(())
    }
}

pub(super) fn automatic_request(
    scope: &TaskEvidenceScope,
    criterion_id: &str,
    lease: &ParallelModeSlotLeaseSnapshot,
    source: &TaskEvidenceTree,
    completed_turn_id: &str,
) -> Result<RequestTaskVerificationRequest> {
    let fingerprint = serde_json::to_vec(&(
        "automatic-host-verification-v1",
        scope,
        criterion_id,
        &lease.slot_id,
        &lease.lease_generation,
        lease.session_key(),
        &lease.delivery_target,
        source,
        completed_turn_id,
    ))?;
    Ok(RequestTaskVerificationRequest {
        request_id: format!("automatic-host:{:x}", Sha256::digest(fingerprint)),
        task_id: scope.task_id.clone(),
        criterion_id: criterion_id.to_owned(),
        scope_id: scope.scope_id.clone(),
        expected_acceptance_revision: scope.acceptance_revision,
    })
}

struct CancelAutomaticRequest<'a> {
    service: &'a TaskVerificationService,
    request_id: String,
}

impl Drop for CancelAutomaticRequest<'_> {
    fn drop(&mut self) {
        if let Ok(flight) = self.service.flight.lock()
            && let Some(active) = flight.as_ref()
            && active.stored.attempt.request_id == self.request_id
        {
            active.cancelled.store(true, Ordering::Release);
        }
    }
}

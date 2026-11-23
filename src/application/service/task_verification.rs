//! A live repository owner reserves once, runs outside the host loop, and persists completion
//! on that loop. Neither a query nor a transport retry starts another execution.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::Result;
use rand::RngCore;

use crate::application::port::inbound::stored_command_port::StoredCommandContext;
use crate::application::port::inbound::stored_command_port::task_evidence::RequestTaskVerificationRequest;
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::task_evidence_artifact_port::TaskEvidenceArtifactPort;
use crate::application::port::outbound::task_evidence_repository_port::TaskEvidenceRepositoryPort;
use crate::application::port::outbound::task_verification_repository_port::{
    StoredTaskVerification, TaskVerificationClaimCommit, TaskVerificationClaimResult,
    TaskVerificationExecutionKey, TaskVerificationFinishCommit,
};
use crate::application::port::outbound::task_verification_runner_port::TaskVerificationRunnerPort;
use crate::domain::planning::evidence::TaskEvidenceError;
use crate::domain::planning::verification::{TaskVerificationNotRunReason, TaskVerificationResult};

mod authority;
mod automatic;
mod continuation;
mod execution;
mod handle;
pub use automatic::AutomaticTaskVerificationOutcome;
pub use continuation::TaskVerificationContinuation;
pub(crate) use continuation::with_verification_commit;
pub use handle::TaskVerificationHandle;
#[cfg(test)]
pub(crate) mod tests;

pub type VerificationJob = Box<dyn FnOnce() + Send + 'static>;
pub type VerificationSpawn =
    Arc<dyn Fn(VerificationJob, VerificationJob) -> std::io::Result<()> + Send + Sync>;
pub type VerificationWake = Arc<dyn Fn() + Send + Sync>;

pub struct TaskVerificationDependencies {
    pub workspace: String,
    pub repository: Arc<dyn TaskEvidenceRepositoryPort>,
    pub artifacts: Arc<dyn TaskEvidenceArtifactPort>,
    pub runtime: Arc<dyn ParallelModeRuntimePort>,
    pub authority: Arc<dyn PlanningAuthorityPort>,
    pub runner: Arc<dyn TaskVerificationRunnerPort>,
    pub spawn: VerificationSpawn,
    pub wake: VerificationWake,
}

pub struct TaskVerificationService {
    dependencies: Arc<TaskVerificationDependencies>,
    owner_token: String,
    initialized: AtomicBool,
    closed: Arc<AtomicBool>,
    flight: Mutex<Option<InFlight>>,
}

struct InFlight {
    stored: StoredTaskVerification,
    execution: TaskVerificationExecutionKey,
    cancelled: Arc<AtomicBool>,
    receiver: mpsc::Receiver<TaskVerificationFinishCommit>,
    completion: Option<TaskVerificationFinishCommit>,
    next_check: Instant,
}

impl TaskVerificationService {
    /// Construction is inert. Recovery is permitted only after the repository OS lock is held.
    pub fn new(dependencies: TaskVerificationDependencies) -> Result<Self> {
        let mut random = [0_u8; 32];
        rand::rngs::OsRng
            .try_fill_bytes(&mut random)
            .map_err(|_| TaskEvidenceError::Unavailable)?;
        let owner_token = random.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            dependencies: Arc::new(dependencies),
            owner_token,
            initialized: AtomicBool::new(false),
            closed: Arc::new(AtomicBool::new(false)),
            flight: Mutex::new(None),
        })
    }

    pub fn initialize_live_owner(&self, at: &str) -> Result<()> {
        let _flight = self
            .flight
            .lock()
            .map_err(|_| TaskEvidenceError::Unavailable)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        if self.initialized.load(Ordering::Acquire) {
            return Ok(());
        }
        self.dependencies
            .repository
            .recover_incomplete_host_verifications(
                &self.dependencies.workspace,
                &self.owner_token,
                at,
            )?;
        self.initialized.store(true, Ordering::Release);
        Ok(())
    }

    pub fn request_verification(
        &self,
        request: RequestTaskVerificationRequest,
        context: &StoredCommandContext,
    ) -> Result<TaskVerificationClaimResult> {
        self.request_guarded(request, context, None, None)?
            .ok_or_else(|| TaskEvidenceError::Limit.into())
    }

    fn request_guarded(
        &self,
        request: RequestTaskVerificationRequest,
        context: &StoredCommandContext,
        expected_lease: Option<crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot>,
        automatic: Option<automatic::ExecutionGuard>,
    ) -> Result<Option<TaskVerificationClaimResult>> {
        request.validate()?;
        if !self.initialized.load(Ordering::Acquire) || self.closed.load(Ordering::Acquire) {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        let mut flight = self
            .flight
            .lock()
            .map_err(|_| TaskEvidenceError::Unavailable)?;
        if self.closed.load(Ordering::Acquire)
            || automatic.as_ref().is_some_and(|guard| {
                !guard.continuation.is_current() || Instant::now() >= guard.deadline
            })
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        // Finish retries own the same slot until their original fact is durable.
        if flight
            .as_ref()
            .is_some_and(|value| value.stored.attempt.request_id != request.request_id)
        {
            return Ok(None);
        }
        let commit = TaskVerificationClaimCommit {
            request_id: request.request_id,
            task_id: request.task_id,
            criterion_id: request.criterion_id,
            scope_id: request.scope_id,
            expected_acceptance_revision: request.expected_acceptance_revision,
            expected_lease,
            owner_token: self.owner_token.clone(),
            at: context.at.clone(),
        };
        let reserve = || {
            if self.closed.load(Ordering::Acquire)
                || automatic
                    .as_ref()
                    .is_some_and(|guard| Instant::now() >= guard.deadline)
            {
                return Err(TaskEvidenceError::Unavailable.into());
            }
            self.dependencies
                .repository
                .claim_host_verification(&self.dependencies.workspace, &commit)
        };
        let claimed = match automatic.as_ref() {
            Some(guard) => with_verification_commit(guard.continuation.as_ref(), reserve)
                .ok_or(TaskEvidenceError::Unavailable)??,
            None => reserve()?,
        };
        let Some(execution) = claimed.execution.clone() else {
            return Ok(Some(claimed));
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let dependencies = self.dependencies.clone();
        let worker_stored = claimed.stored.clone();
        let worker_execution = execution.clone();
        let worker_cancelled = cancelled.clone();
        let worker_closed = self.closed.clone();
        let worker_sender = sender.clone();
        let worker_wake = dependencies.wake.clone();
        let recovery_execution = execution.clone();
        let recovery_wake = dependencies.wake.clone();
        *flight = Some(InFlight {
            stored: claimed.stored.clone(),
            execution,
            cancelled,
            receiver,
            completion: None,
            next_check: Instant::now(),
        });
        let spawned = (self.dependencies.spawn)(
            Box::new(move || {
                let completion = execution::execute(
                    dependencies.as_ref(),
                    &worker_stored,
                    worker_execution,
                    worker_cancelled.as_ref(),
                    worker_closed.as_ref(),
                    automatic.as_ref(),
                );
                let _ = worker_sender.send(completion);
                worker_wake();
            }),
            Box::new(move || {
                let _ = sender.send(TaskVerificationFinishCommit {
                    execution: recovery_execution,
                    result: TaskVerificationResult::Interrupted {},
                    artifact: None,
                    at: utc_now(),
                });
                recovery_wake();
            }),
        );
        if spawned.is_err()
            && let Some(active) = flight.as_mut()
        {
            active.completion = Some(not_run(
                active.execution.clone(),
                TaskVerificationNotRunReason::SpawnFailed,
            ));
        }
        // Automatic completion starts outside the Native loop. Recompute its
        // authority/cancellation deadline even before a worker produces output.
        (self.dependencies.wake)();
        Ok(Some(claimed))
    }

    /// Host-loop polling never waits for a worker or repeats a completed process.
    /// The returned flag requests an evidence outbox drain after persistence.
    pub fn poll(&self, now: Instant) -> bool {
        if self.closed.load(Ordering::Acquire) {
            return false;
        }
        let Ok(mut flight) = self.flight.lock() else {
            return false;
        };
        let Some(active) = flight.as_mut() else {
            return false;
        };
        if active.completion.is_none() {
            match active.receiver.try_recv() {
                Ok(completion) => active.completion = Some(completion),
                Err(mpsc::TryRecvError::Disconnected) => {
                    active.completion = Some(TaskVerificationFinishCommit {
                        execution: active.execution.clone(),
                        result: TaskVerificationResult::Interrupted {},
                        artifact: None,
                        at: utc_now(),
                    });
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if now < active.next_check {
            return false;
        }
        active.next_check = now.checked_add(Duration::from_secs(1)).unwrap_or(now);
        if let Some(completion) = &active.completion {
            // Preserve this exact observation, including its timestamps, on transient DB errors.
            if self
                .dependencies
                .repository
                .finish_host_verification(&self.dependencies.workspace, completion)
                .is_ok()
            {
                *flight = None;
                return true;
            }
        } else if authority::require_current(self.dependencies.as_ref(), &active.stored).is_err() {
            active.cancelled.store(true, Ordering::Release);
        }
        false
    }

    pub fn has_pending(&self) -> bool {
        self.flight
            .lock()
            .map(|flight| flight.is_some())
            .unwrap_or(true)
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.flight
            .lock()
            .ok()
            .and_then(|flight| flight.as_ref().map(|active| active.next_check))
    }

    /// Revoke the live owner before the repository lock is released. Background
    /// callers may still hold an Arc, but that Arc never keeps execution authorized.
    pub fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        if let Ok(flight) = self.flight.lock()
            && let Some(active) = flight.as_ref()
        {
            active.cancelled.store(true, Ordering::Release);
        }
        (self.dependencies.wake)();
    }
}

impl Drop for TaskVerificationService {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        if let Ok(flight) = self.flight.get_mut()
            && let Some(active) = flight.as_ref()
        {
            active.cancelled.store(true, Ordering::Release);
        }
    }
}

fn not_run(
    execution: TaskVerificationExecutionKey,
    reason: TaskVerificationNotRunReason,
) -> TaskVerificationFinishCommit {
    TaskVerificationFinishCommit {
        execution,
        result: TaskVerificationResult::NotRun { reason },
        artifact: None,
        at: utc_now(),
    }
}

fn utc_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

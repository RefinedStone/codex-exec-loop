use super::*;

pub(super) type FailurePolicyHook = Arc<dyn Fn() + Send + Sync>;

struct State {
    owner: Option<String>,
    claim: Option<TaskVerificationClaimCommit>,
    execution: Option<TaskVerificationExecutionKey>,
    stored: StoredTaskVerification,
}

pub(crate) struct Repository {
    state: Mutex<State>,
    pub snapshot: Mutex<TaskEvidenceStoredSnapshot>,
    pub recoveries: AtomicUsize,
    pub claims: AtomicUsize,
    pub reads: AtomicUsize,
    pub verification_reads: AtomicUsize,
    pub failure_limit_calls: AtomicUsize,
    pub failure_limit_hook: Mutex<Option<FailurePolicyHook>>,
    pub claim_hook: Mutex<Option<FailurePolicyHook>>,
    pub finish_calls: Mutex<Vec<TaskVerificationFinishCommit>>,
    pub fail_finish_once: AtomicBool,
    pub fail_ack_once: AtomicBool,
    pub acknowledgements: AtomicUsize,
    pub outbox: Mutex<Vec<TaskEvidenceEvent>>,
}

impl Repository {
    pub fn new() -> Self {
        let (snapshot, stored) = fixtures::snapshot();
        Self {
            state: Mutex::new(State {
                owner: None,
                claim: None,
                execution: None,
                stored,
            }),
            snapshot: Mutex::new(snapshot),
            recoveries: AtomicUsize::new(0),
            claims: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            verification_reads: AtomicUsize::new(0),
            failure_limit_calls: AtomicUsize::new(0),
            failure_limit_hook: Mutex::new(None),
            claim_hook: Mutex::new(None),
            finish_calls: Mutex::new(vec![]),
            fail_finish_once: AtomicBool::new(false),
            fail_ack_once: AtomicBool::new(false),
            acknowledgements: AtomicUsize::new(0),
            outbox: Mutex::new(vec![]),
        }
    }
}

impl TaskVerificationRepositoryPort for Repository {
    fn recover_incomplete_host_verifications(
        &self,
        _: &str,
        owner: &str,
        _: &str,
    ) -> Result<Vec<TaskVerificationReceipt>> {
        self.recoveries.fetch_add(1, Ordering::SeqCst);
        self.state.lock().unwrap().owner = Some(owner.to_owned());
        Ok(vec![])
    }

    fn claim_host_verification(
        &self,
        _: &str,
        commit: &TaskVerificationClaimCommit,
    ) -> Result<TaskVerificationClaimResult> {
        self.claims.fetch_add(1, Ordering::SeqCst);
        let hook = self.claim_hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
        }
        let mut state = self.state.lock().unwrap();
        if state.owner.as_deref() != Some(commit.owner_token.as_str()) {
            return Err(TaskEvidenceError::Conflict.into());
        }
        if let Some(prior) = &state.claim {
            if prior.request_id != commit.request_id {
                return Err(TaskEvidenceError::Limit.into());
            }
            if prior.task_id != commit.task_id
                || prior.criterion_id != commit.criterion_id
                || prior.scope_id != commit.scope_id
                || prior.expected_acceptance_revision != commit.expected_acceptance_revision
            {
                return Err(TaskEvidenceError::Conflict.into());
            }
            return Ok(TaskVerificationClaimResult {
                stored: state.stored.clone(),
                execution: None,
                changed: false,
            });
        }
        state.stored.attempt.request_id = commit.request_id.clone();
        state.stored.attempt.requested_at = commit.at.clone();
        let execution = TaskVerificationExecutionKey {
            attempt_id: state.stored.attempt.attempt_id.clone(),
            owner_token: commit.owner_token.clone(),
            run_token: "4".repeat(64),
        };
        state.execution = Some(execution.clone());
        state.claim = Some(commit.clone());
        Ok(TaskVerificationClaimResult {
            stored: state.stored.clone(),
            execution: Some(execution),
            changed: true,
        })
    }

    fn bind_host_verification(&self, _: &str, commit: &TaskVerificationBindCommit) -> Result<bool> {
        let mut state = self.state.lock().unwrap();
        if state.execution.as_ref() != Some(&commit.execution) {
            return Err(TaskEvidenceError::Conflict.into());
        }
        if state.stored.phase != TaskVerificationPhase::Pending {
            return Ok(false);
        }
        state.stored.manifest = Some(commit.manifest.clone());
        state.stored.phase = TaskVerificationPhase::Running;
        Ok(true)
    }

    fn finish_host_verification(
        &self,
        _: &str,
        commit: &TaskVerificationFinishCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskVerificationReceipt>> {
        self.finish_calls.lock().unwrap().push(commit.clone());
        if self.fail_finish_once.swap(false, Ordering::SeqCst) {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        let mut state = self.state.lock().unwrap();
        if state.execution.as_ref() != Some(&commit.execution) {
            return Err(TaskEvidenceError::Conflict.into());
        }
        if let Some(receipt) = &state.stored.receipt {
            if receipt.result != commit.result || receipt.artifact != commit.artifact {
                return Err(TaskEvidenceError::Conflict.into());
            }
            return Ok(TaskEvidenceWriteResult {
                value: receipt.clone(),
                changed: false,
            });
        }
        let receipt = TaskVerificationReceipt {
            event_id: format!(
                "task-verification-finished:{}",
                state.stored.attempt.attempt_id
            ),
            evidence_revision: 1,
            attempt: state.stored.attempt.clone(),
            manifest: state.stored.manifest.clone(),
            result: commit.result.clone(),
            artifact: commit.artifact.clone(),
            finished_at: commit.at.clone(),
        };
        receipt.validate()?;
        state.stored.phase = TaskVerificationPhase::Finished;
        state.stored.receipt = Some(receipt.clone());
        self.outbox
            .lock()
            .unwrap()
            .push(TaskEvidenceEvent::VerificationFinished {
                receipt: Box::new(receipt.clone()),
            });
        Ok(TaskEvidenceWriteResult {
            value: receipt,
            changed: true,
        })
    }

    fn load_host_verification(&self, _: &str, id: &str) -> Result<Option<StoredTaskVerification>> {
        self.verification_reads.fetch_add(1, Ordering::SeqCst);
        let state = self.state.lock().unwrap();
        Ok(state
            .claim
            .as_ref()
            .filter(|_| state.stored.attempt.attempt_id == id)
            .map(|_| state.stored.clone()))
    }

    fn apply_host_verification_failure_limit(
        &self,
        _: &str,
        _: &TaskVerificationFailureLimitCommit,
    ) -> Result<TaskVerificationFailureLimitResult> {
        self.failure_limit_calls.fetch_add(1, Ordering::SeqCst);
        let hook = self.failure_limit_hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
        }
        Ok(TaskVerificationFailureLimitResult {
            outcome: TaskVerificationFailureLimitOutcome::NotFailed,
            failure_count: 0,
            planning_revision: None,
            changed: false,
        })
    }
}

impl TaskEvidenceRepositoryPort for Repository {
    fn load_current_snapshot(
        &self,
        _: &str,
        _: &TaskEvidenceSnapshotQuery,
    ) -> Result<TaskEvidenceStoredSnapshot> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.snapshot.lock().unwrap().clone())
    }
    fn load_attempt(&self, _: &str, _: &str) -> Result<Option<StoredTaskEvidenceAttempt>> {
        panic!("host scheduling must not read reported attempts")
    }
    fn prepare_attempt(
        &self,
        _: &str,
        _: &TaskEvidencePrepareCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceAttempt>> {
        panic!("host scheduling must not reserve reported attempts")
    }
    fn record_evidence(
        &self,
        _: &str,
        _: &TaskEvidenceRecordCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceReceipt>> {
        panic!("host results cannot become worker reports")
    }
    fn waive_criterion(
        &self,
        _: &str,
        _: &TaskCriterionWaiverCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskCriterionWaiverReceipt>> {
        panic!("host results cannot waive criteria")
    }
    fn pending_events(&self, _: &str, limit: usize) -> Result<Vec<TaskEvidenceEvent>> {
        Ok(self
            .outbox
            .lock()
            .unwrap()
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }
    fn acknowledge_event(&self, _: &str, id: &str) -> Result<()> {
        self.acknowledgements.fetch_add(1, Ordering::SeqCst);
        if self.fail_ack_once.swap(false, Ordering::SeqCst) {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        self.outbox
            .lock()
            .unwrap()
            .retain(|event| event.event_id() != id);
        Ok(())
    }
}

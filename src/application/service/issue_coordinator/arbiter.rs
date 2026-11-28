//! The existing coordinator tick owns one durable, bounded assessment attempt.
//! Adapter calls return promptly; the host never waits here for model completion.
use super::*;
use crate::application::port::outbound::issue_assessment_evidence_port::IssueAssessmentEvidencePort;
use crate::application::port::outbound::issue_assessment_model_port::{
    IssueAssessmentModelFailure, IssueAssessmentModelPort, IssueAssessmentModelRequest,
    IssueAssessmentModelStatus,
};
use crate::application::port::outbound::issue_automation_store_port::{
    IssueArbiterClaimRequest, IssueArbiterClaimResult, IssueArbiterCurrentContext,
    IssueArbiterFinishRequest, IssueArbiterFinishResult, IssueArbiterWaitRequest,
};
use crate::domain::issue_automation::arbiter::*;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

mod input;

#[derive(Clone, Copy)]
struct ArbiterTick<'a, 'b> {
    cancel: &'a IssueCollectionCancellation,
    base_now: u64,
    started: Instant,
    clock: &'a (dyn Fn() -> Instant + Send + Sync),
    committed: &'a Reporter<'b>,
}

impl ArbiterTick<'_, '_> {
    fn now(self) -> u64 {
        let elapsed = (self.clock)()
            .saturating_duration_since(self.started)
            .as_millis();
        self.base_now
            .saturating_add(u64::try_from(elapsed).unwrap_or(u64::MAX))
    }
}

pub(super) struct ArbiterRuntime {
    model: Arc<dyn IssueAssessmentModelPort>,
    evidence: Arc<dyn IssueAssessmentEvidencePort>,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
}

impl IssueCoordinatorService {
    pub fn with_arbiter(
        mut self,
        model: Arc<dyn IssueAssessmentModelPort>,
        evidence: Arc<dyn IssueAssessmentEvidencePort>,
    ) -> Self {
        self.arbiter = Some(ArbiterRuntime {
            model,
            evidence,
            clock: Arc::new(Instant::now),
        });
        self
    }

    #[cfg(test)]
    pub(crate) fn with_arbiter_clock(
        mut self,
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    ) -> Self {
        self.arbiter
            .as_mut()
            .expect("the arbiter is configured")
            .clock = clock;
        self
    }

    /// Both callers already hold tick_gate. In particular, no second worker can observe
    /// a just-claimed ID before the first worker has dispatched it to the same adapter.
    pub(super) fn process_arbiters(
        &self,
        cancel: &IssueCollectionCancellation,
        now: u64,
        execution_enabled: bool,
        committed: &Reporter<'_>,
    ) -> Result<()> {
        let Some(runtime) = &self.arbiter else {
            return Ok(());
        };
        let tick = ArbiterTick {
            cancel,
            base_now: now,
            started: (runtime.clock)(),
            clock: runtime.clock.as_ref(),
            committed,
        };
        let mut pending = BTreeSet::new();
        let mut after = None;
        let mut first_error = None;
        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let page = self.store.unfinished_issue_arbiter_attempts(
                &self.workspace,
                after.as_deref(),
                MAX_ISSUE_ARBITER_BATCH,
            )?;
            for attempt in &page {
                pending.insert(attempt.decision_id.clone());
                if cancel.is_cancelled() {
                    return Ok(());
                }
                if let Err(error) = self.observe_arbiter(runtime, attempt, execution_enabled, tick)
                {
                    first_error.get_or_insert(error);
                }
            }
            after = page.last().map(|attempt| attempt.attempt_id.clone());
            if page.len() < MAX_ISSUE_ARBITER_BATCH {
                break;
            }
        }
        if !execution_enabled || cancel.is_cancelled() {
            return first_error.map_or(Ok(()), Err);
        }
        let mut after = None;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let page = self.store.issue_decision_deadlines(
                &self.workspace,
                after.as_ref(),
                MAX_ISSUE_ARBITER_BATCH,
            )?;
            let ids: Vec<_> = page
                .iter()
                .filter(|entry| {
                    entry.record.request.definition().on_timeout == IssueDecisionTimeout::Arbiter
                })
                .map(|entry| entry.record.request.id().clone())
                .collect();
            let waits = if ids.is_empty() {
                BTreeSet::new()
            } else {
                self.store
                    .issue_decision_arbiter_waits(&self.workspace, &ids)?
                    .waits
                    .into_iter()
                    .map(|wait| wait.decision_id)
                    .collect()
            };
            for entry in &page {
                let request = &entry.record.request;
                if cancel.is_cancelled() {
                    break;
                }
                if request.definition().on_timeout != IssueDecisionTimeout::Arbiter
                    || !due(request.definition().due_at.as_deref(), tick.now())
                    || pending.contains(request.id())
                    || waits.contains(request.id())
                {
                    continue;
                }
                if let Err(error) = self.start_arbiter(runtime, request.id(), tick) {
                    first_error.get_or_insert(error);
                }
            }
            after = page.last().map(|entry| entry.record.request.id().clone());
            if page.len() < MAX_ISSUE_ARBITER_BATCH {
                break;
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn start_arbiter(
        &self,
        runtime: &ArbiterRuntime,
        id: &IssueDecisionId,
        tick: ArbiterTick<'_, '_>,
    ) -> Result<()> {
        let ArbiterTick {
            cancel, committed, ..
        } = tick;
        let current = self.arbiter_current(id, committed)?;
        if current.basis != current.record.request.definition().basis
            || !arbiter_eligible(&current.record.request, &current.assessment, tick.now())
        {
            return Ok(());
        }
        let admission = self
            .issue_binding_admission
            .as_ref()
            .ok_or_else(|| anyhow!("issue binding admission is unavailable"))?
            .clone();
        let binding_id = current.item.binding_id.clone();
        let pause_state = self
            .store
            .issue_binding_pause_state(&self.workspace, &binding_id)?;
        admission.register_if_absent(&pause_state)?;
        let generation = admission.capture(&binding_id)?;
        if generation != current.fence.triage.binding_run_revision {
            return Ok(());
        }
        let prepared = match self.prepare_arbiter_input(runtime, &current) {
            Ok(prepared) => prepared,
            Err(reason) => {
                let saved = cancel.commit_if_active(|| {
                    self.store.record_issue_arbiter_wait(
                        &self.workspace,
                        &IssueArbiterWaitRequest {
                            fence: current.fence,
                            current: current.assessment.clone(),
                            decision_id: id.clone(),
                            reason,
                            now_unix_ms: tick.now(),
                        },
                    )
                })?;
                if let Some(saved) = saved {
                    report_finished(saved, committed);
                }
                return Ok(());
            }
        };
        if !self.check_account_quota(
            crate::application::port::outbound::issue_account_quota_port::IssueQuotaLane::Arbiter,
            cancel,
            (committed, || Some(runtime.model.read_account_quota())),
        )? {
            return Ok(());
        }
        let claim = cancel.commit_if_active(|| {
            self.store.claim_issue_arbiter(
                &self.workspace,
                &IssueArbiterClaimRequest {
                    fence: current.fence,
                    current: current.assessment.clone(),
                    decision_id: id.clone(),
                    input_basis: prepared.basis,
                    settings: prepared.settings,
                    now_unix_ms: tick.now(),
                },
            )
        })?;
        match claim {
            Some(IssueArbiterClaimResult::Claimed(attempt)) => {
                let authorization: Arc<
                    dyn crate::application::port::agent_action::AgentActionAuthorization,
                > = Arc::new(move || {
                    let guard = admission.acquire(&binding_id, generation)?;
                    guard.validate()?;
                    Ok(Box::new(guard)
                        as Box<
                            dyn crate::application::port::agent_action::AgentActionGuard,
                        >)
                });
                let status = cancel.commit_if_active(|| {
                    let remaining_ms = attempt.deadline_at_unix_ms.saturating_sub(tick.now());
                    if remaining_ms == 0 {
                        return Ok(IssueAssessmentModelStatus::Unusable(
                            IssueAssessmentModelFailure::Uncertain,
                        ));
                    }
                    Ok(runtime
                        .model
                        .dispatch_authorized(
                            IssueAssessmentModelRequest {
                                attempt_id: attempt.attempt_id.clone(),
                                input_json: prepared.json,
                                expected_route_revision: prepared.configuration.route_revision,
                                model: prepared.configuration.model,
                                timeout: Duration::from_millis(remaining_ms),
                            },
                            authorization,
                        )
                        .unwrap_or(IssueAssessmentModelStatus::Unusable(
                            IssueAssessmentModelFailure::Unavailable,
                        )))
                })?;
                if let Some(status) = status {
                    self.finish_arbiter_status(runtime, &attempt, status, tick)?;
                }
            }
            Some(IssueArbiterClaimResult::Deferred(commit)) => {
                committed(vec![IssueCoordinatorFact::DecisionArbiterFinished(
                    commit.event,
                )]);
            }
            // A reservation from another lifetime is never a dispatch capability.
            Some(IssueArbiterClaimResult::Existing(_))
            | Some(IssueArbiterClaimResult::Resolved(_))
            | Some(IssueArbiterClaimResult::Ineligible)
            | Some(IssueArbiterClaimResult::Conflict)
            | None => {}
        }
        Ok(())
    }

    fn observe_arbiter(
        &self,
        runtime: &ArbiterRuntime,
        attempt: &IssueArbiterAttempt,
        execution_enabled: bool,
        tick: ArbiterTick<'_, '_>,
    ) -> Result<()> {
        let committed = tick.committed;
        let record = self
            .store
            .get_issue_decision(&self.workspace, &attempt.decision_id)?;
        if !execution_enabled
            || record
                .as_ref()
                .is_none_or(|record| record.request.state() != IssueDecisionState::Open)
        {
            let _ = runtime.model.cancel(&attempt.attempt_id);
            return self.finish_arbiter(
                runtime,
                attempt,
                None,
                IssueArbiterResult::Waiting(IssueArbiterWaitReason::ContextUnavailable),
                tick,
            );
        }
        let current = match self.arbiter_current(&attempt.decision_id, committed) {
            Ok(current) => current,
            Err(_) => {
                let _ = runtime.model.cancel(&attempt.attempt_id);
                return self.finish_arbiter(
                    runtime,
                    attempt,
                    None,
                    IssueArbiterResult::Waiting(IssueArbiterWaitReason::ContextUnavailable),
                    tick,
                );
            }
        };
        let context = current.context(attempt.input_basis.clone());
        if current.basis != attempt.input_basis.decision_basis
            || !arbiter_eligible(&current.record.request, &current.assessment, tick.now())
        {
            let _ = runtime.model.cancel(&attempt.attempt_id);
            return self.finish_arbiter(
                runtime,
                attempt,
                Some(context),
                IssueArbiterResult::Waiting(IssueArbiterWaitReason::ContextUnavailable),
                tick,
            );
        }
        if tick.now() >= attempt.deadline_at_unix_ms {
            let _ = runtime.model.cancel(&attempt.attempt_id);
            return self.finish_arbiter(
                runtime,
                attempt,
                Some(context),
                IssueArbiterResult::Waiting(IssueArbiterWaitReason::OutcomeUnknown),
                tick,
            );
        }
        let status = runtime.model.observe(&attempt.attempt_id).unwrap_or(
            IssueAssessmentModelStatus::Unusable(IssueAssessmentModelFailure::Unavailable),
        );
        self.finish_arbiter_status(runtime, attempt, status, tick)
    }

    fn finish_arbiter_status(
        &self,
        runtime: &ArbiterRuntime,
        attempt: &IssueArbiterAttempt,
        status: IssueAssessmentModelStatus,
        tick: ArbiterTick<'_, '_>,
    ) -> Result<()> {
        let ArbiterTick {
            cancel, committed, ..
        } = tick;
        if status == IssueAssessmentModelStatus::Pending || cancel.is_cancelled() {
            return Ok(());
        }
        let (context, result) = match status {
            IssueAssessmentModelStatus::Pending => return Ok(()),
            IssueAssessmentModelStatus::Unusable(failure) => {
                let context = self
                    .arbiter_current(&attempt.decision_id, committed)
                    .ok()
                    .map(|current| current.context(attempt.input_basis.clone()));
                (
                    context,
                    IssueArbiterResult::Waiting(failure_reason(failure)),
                )
            }
            IssueAssessmentModelStatus::Complete { json } => {
                self.assess_arbiter_reply(runtime, attempt, &json, tick)
            }
        };
        self.finish_arbiter(runtime, attempt, context, result, tick)
    }

    fn assess_arbiter_reply(
        &self,
        runtime: &ArbiterRuntime,
        attempt: &IssueArbiterAttempt,
        json: &str,
        tick: ArbiterTick<'_, '_>,
    ) -> (Option<IssueArbiterCurrentContext>, IssueArbiterResult) {
        let ArbiterTick {
            cancel, committed, ..
        } = tick;
        let unavailable = || {
            (
                None,
                IssueArbiterResult::Waiting(IssueArbiterWaitReason::ContextUnavailable),
            )
        };
        let record = match self
            .store
            .get_issue_decision(&self.workspace, &attempt.decision_id)
        {
            Ok(Some(record)) => record,
            _ => return unavailable(),
        };
        if record.request.state() != IssueDecisionState::Open {
            return unavailable();
        }
        // Observe a remote change before applying an automatic answer. Pending polls do
        // not perform this network read; submission still uses its ordinary fresh fences.
        if !self
            .fresh_resync(record.request.issue(), cancel)
            .unwrap_or(false)
        {
            return unavailable();
        }
        let current = match self.arbiter_current(&attempt.decision_id, committed) {
            Ok(current) => current,
            Err(_) => return unavailable(),
        };
        let old_context = current.context(attempt.input_basis.clone());
        if current.basis != attempt.input_basis.decision_basis
            || !arbiter_eligible(&current.record.request, &current.assessment, tick.now())
        {
            return (
                Some(old_context),
                IssueArbiterResult::Waiting(IssueArbiterWaitReason::ContextUnavailable),
            );
        }
        if tick.now() >= attempt.deadline_at_unix_ms {
            return (
                Some(old_context),
                IssueArbiterResult::Waiting(IssueArbiterWaitReason::OutcomeUnknown),
            );
        }
        let prepared = match self.prepare_arbiter_input(runtime, &current) {
            Ok(prepared) => prepared,
            Err(reason) => return (Some(old_context), IssueArbiterResult::Waiting(reason)),
        };
        let context = current.context(prepared.basis.clone());
        if prepared.basis != attempt.input_basis || prepared.settings != attempt.settings {
            return (
                Some(context),
                IssueArbiterResult::Waiting(IssueArbiterWaitReason::EvidenceChanged),
            );
        }
        let proposal = self.sanitize_arbiter_reply(&current, json);
        (
            Some(context),
            proposal
                .map(IssueArbiterResult::Proposal)
                .unwrap_or_else(IssueArbiterResult::Waiting),
        )
    }

    fn finish_arbiter(
        &self,
        runtime: &ArbiterRuntime,
        attempt: &IssueArbiterAttempt,
        current: Option<IssueArbiterCurrentContext>,
        result: IssueArbiterResult,
        tick: ArbiterTick<'_, '_>,
    ) -> Result<()> {
        let ArbiterTick {
            cancel, committed, ..
        } = tick;
        let saved = cancel.commit_if_active(|| {
            self.store.finish_issue_arbiter(
                &self.workspace,
                &IssueArbiterFinishRequest {
                    attempt_id: attempt.attempt_id.clone(),
                    expected_revision: attempt.revision,
                    current,
                    result,
                    now_unix_ms: tick.now(),
                },
            )
        })?;
        if let Some(saved) = saved {
            if matches!(saved, IssueArbiterFinishResult::Saved(_)) {
                runtime.model.forget(&attempt.attempt_id);
            }
            report_finished(saved, committed);
        }
        Ok(())
    }

    pub(super) fn next_arbiter_deadline_at(&self, now: u64) -> Result<Option<u64>> {
        let mut after = None;
        let mut next = None;
        loop {
            let page = self.store.unfinished_issue_arbiter_attempts(
                &self.workspace,
                after.as_deref(),
                MAX_ISSUE_ARBITER_BATCH,
            )?;
            for attempt in &page {
                if attempt.deadline_at_unix_ms > now {
                    next = Some(next.map_or(attempt.deadline_at_unix_ms, |prior: u64| {
                        prior.min(attempt.deadline_at_unix_ms)
                    }));
                }
            }
            after = page.last().map(|attempt| attempt.attempt_id.clone());
            if page.len() < MAX_ISSUE_ARBITER_BATCH {
                break;
            }
        }
        Ok(next)
    }
}

fn report_finished(result: IssueArbiterFinishResult, committed: &Reporter<'_>) {
    if let IssueArbiterFinishResult::Saved(commit) = result {
        committed(vec![IssueCoordinatorFact::DecisionArbiterFinished(
            commit.event,
        )]);
    }
}

fn due(at: Option<&str>, now: u64) -> bool {
    at.and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .and_then(|at| u64::try_from(at.timestamp_millis()).ok())
        .is_some_and(|at| now >= at)
}

fn failure_reason(failure: IssueAssessmentModelFailure) -> IssueArbiterWaitReason {
    match failure {
        IssueAssessmentModelFailure::InvalidReply => IssueArbiterWaitReason::InvalidResponse,
        IssueAssessmentModelFailure::Uncertain | IssueAssessmentModelFailure::Interrupted => {
            IssueArbiterWaitReason::OutcomeUnknown
        }
        IssueAssessmentModelFailure::ConfigurationChanged => {
            IssueArbiterWaitReason::EvidenceChanged
        }
        _ => IssueArbiterWaitReason::ModelUnavailable,
    }
}

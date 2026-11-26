//! 사람·명시적 정책·검증된 마감 결정의 권한을 기존 실행 경계에서 소비한다.
use super::issue_collection::{IssueCollectionCancellation, checked_resync_read};
use super::issue_decision::IssueDecisionService;
use super::issue_feedback::IssueFeedbackService;
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::domain::issue_automation::feedback::IssueFeedbackEvent;

mod arbiter;
mod auto_delivery;
mod closure;
mod delivery;
mod feedback;
mod start;
use super::issue_policy::{IssuePolicyService, issue_policy_version};
use super::issue_triage::{IssueTriageService, IssueTriagedFact};
use crate::application::port::outbound::issue_automation_store_port::{
    IssueAutomationStorePort, IssueCollectionApplyResult, IssueDecisionFence,
    IssueDecisionSaveResult, IssueTriageFence, IssueTriageSaveResult, IssueWorkAdmissionResult,
    IssueWorkSaveResult,
};
use crate::application::port::outbound::issue_source_port::{IssueResyncRequest, IssueSourcePort};
use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkExecutionPort, IssueWorkPermitRequest,
};
use crate::domain::issue_automation::collection::{IssueCollectionItem, IssueCollectionMode};
use crate::domain::issue_automation::coordinator::*;
use crate::domain::issue_automation::decision::*;
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_automation::source::{
    IssueSourceContent, IssueSourceDisclosure, IssueSourceRead,
};
use crate::domain::issue_automation::triage::{IssueTriageAssessment, issue_acceptance_criteria};
use crate::domain::issue_automation::{IssueAutomationState, IssueTransition, IssueTriageResult};
use crate::domain::issue_policy::IssueDisclosureTarget;
use crate::domain::issue_work::IssueReference;
use anyhow::{Result, anyhow, bail};
use start::{StartAuthorization, StartContext};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueCoordinatorFact {
    Triaged(IssueTriagedFact),
    FeedbackPosted(IssueFeedbackEvent),
    Decision {
        fact: IssueDecisionFact,
        revision: u64,
    },
    DecisionReminded(IssueDecisionReminder),
    DecisionArbiterFinished(crate::domain::issue_automation::arbiter::IssueArbiterFinishedEvent),
    ExecutionProgressed {
        event_id: String,
        issue: IssueReference,
        work_order_id: String,
        progress: crate::domain::issue_automation::execution::IssueExecutionProgress,
        revision: u64,
    },
    Integrated {
        event_id: String,
        issue: IssueReference,
        work_order_id: String,
        integration: crate::domain::issue_automation::execution::IssueExecutionIntegration,
        revision: u64,
    },
    ExternalClosed {
        event_id: String,
        issue: IssueReference,
        work_order_id: String,
        observation_revision: u64,
        revision: u64,
    },
    WorkSubmitted {
        event_id: String,
        issue: IssueReference,
        work_order_id: String,
        task_ids: Vec<String>,
        revision: u64,
    },
    WorkPermitGranted {
        event_id: String,
        issue: IssueReference,
        work_order_id: String,
        source_sha: String,
        gate_revision: u64,
        delivery: bool,
        revision: u64,
    },
}
type Reporter<'a> = dyn Fn(Vec<IssueCoordinatorFact>) + Send + Sync + 'a;

// 한 tick에서 같은 영속 이벤트는 한 번 보고한다. ACK하지 않은 이벤트는 다음 tick에 다시 전달한다.
fn work_event_reporter<'a>(
    committed: &'a Reporter<'a>,
) -> impl Fn(Vec<IssueCoordinatorFact>) + Send + Sync + 'a {
    let reported = Mutex::new(std::collections::BTreeSet::new());
    move |facts| {
        let fresh: Vec<_> = {
            let mut reported = reported.lock().expect("work event reporter lock poisoned");
            facts
                .into_iter()
                .filter(|fact| match fact {
                    IssueCoordinatorFact::ExecutionProgressed { event_id, .. }
                    | IssueCoordinatorFact::Integrated { event_id, .. }
                    | IssueCoordinatorFact::ExternalClosed { event_id, .. }
                    | IssueCoordinatorFact::WorkSubmitted { event_id, .. }
                    | IssueCoordinatorFact::WorkPermitGranted { event_id, .. } => {
                        reported.insert(event_id.clone())
                    }
                    IssueCoordinatorFact::FeedbackPosted(event) => {
                        reported.insert(event.event_id.clone())
                    }
                    IssueCoordinatorFact::DecisionArbiterFinished(event) => {
                        reported.insert(event.event_id.clone())
                    }
                    IssueCoordinatorFact::DecisionReminded(event) => {
                        reported.insert(event.event_id.clone())
                    }
                    IssueCoordinatorFact::Triaged(_) | IssueCoordinatorFact::Decision { .. } => {
                        true
                    }
                })
                .collect()
        };
        if !fresh.is_empty() {
            committed(fresh);
        }
    }
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
enum IssueCoordinatorGateEvent {
    Contended,
    Finished,
}

pub struct IssueCoordinatorService {
    workspace: String,
    store: Arc<dyn IssueAutomationStorePort>,
    source: Arc<dyn IssueSourcePort>,
    triage: Arc<IssueTriageService>,
    decisions: Arc<IssueDecisionService>,
    policy: IssuePolicyService,
    execution: Arc<dyn IssueWorkExecutionPort>,
    feedback: Option<Arc<IssueFeedbackService>>,
    arbiter: Option<arbiter::ArbiterRuntime>,
    publication_journal: Option<Arc<dyn IssuePublishIntentJournalPort>>,
    issue_binding_admission: Option<Arc<super::issue_binding_admission::IssueBindingAdmission>>,
    /// 한 host의 동시 tick/reconnect 호출을 직렬화한다. 실행기 상태는 소유하지 않는다.
    tick_gate: Mutex<()>,
    #[cfg(test)]
    tick_gate_events: Option<std::sync::mpsc::Sender<IssueCoordinatorGateEvent>>,
}

impl IssueCoordinatorService {
    pub fn new(
        workspace: String,
        store: Arc<dyn IssueAutomationStorePort>,
        source: Arc<dyn IssueSourcePort>,
        triage: Arc<IssueTriageService>,
        decisions: Arc<IssueDecisionService>,
        policy: IssuePolicyService,
        execution: Arc<dyn IssueWorkExecutionPort>,
    ) -> Self {
        Self {
            workspace,
            store,
            source,
            triage,
            decisions,
            policy,
            execution,
            feedback: None,
            arbiter: None,
            publication_journal: None,
            issue_binding_admission: None,
            tick_gate: Mutex::new(()),
            #[cfg(test)]
            tick_gate_events: None,
        }
    }

    /// 관찰 worker의 경합은 대기한다. tracking 중 도착한 수집 tick을 버리지 않는다.
    pub fn with_issue_binding_admission(
        mut self,
        admission: Arc<super::issue_binding_admission::IssueBindingAdmission>,
    ) -> Self {
        self.issue_binding_admission = Some(admission);
        self
    }

    fn lock_tick_gate(&self) -> Result<MutexGuard<'_, ()>> {
        let guard = match self.tick_gate.try_lock() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::Poisoned(error)) => Err(error),
            Err(TryLockError::WouldBlock) => {
                #[cfg(test)]
                if let Some(events) = &self.tick_gate_events {
                    let _ = events.send(IssueCoordinatorGateEvent::Contended);
                }
                self.tick_gate.lock()
            }
        };
        guard.map_err(|_| anyhow!("the issue coordinator tick gate is poisoned"))
    }

    pub fn tick(
        &self,
        cancel: &IssueCollectionCancellation,
        now_unix_ms: u64,
        execution_enabled: bool,
        committed: &Reporter<'_>,
    ) -> Result<()> {
        let _guard = self.lock_tick_gate()?;
        if cancel.is_cancelled() {
            return Ok(());
        }
        let report_once = work_event_reporter(committed);
        let committed: &Reporter<'_> = &report_once;
        let arbiter_error = self
            .process_arbiters(cancel, now_unix_ms, execution_enabled, committed)
            .err();
        let deadline_error = self
            .process_decision_deadlines(cancel, now_unix_ms, committed)
            .err();
        let feedback_error = self.reconcile_feedback(cancel).err();
        self.report_pending(committed)?;
        super::issue_intervention::recover_pending_controls(
            &self.workspace,
            self.store.as_ref(),
            self.execution.as_ref(),
        )?;
        let mut execution_failures = std::collections::BTreeSet::new();
        let mut first_error = execution::track(
            self,
            cancel,
            now_unix_ms,
            execution_enabled,
            committed,
            &mut execution_failures,
        )
        .err()
        .or(feedback_error)
        .or(deadline_error)
        .or(arbiter_error);
        let mut after = None;
        'batches: loop {
            if cancel.is_cancelled() {
                break;
            }
            let batch = self
                .store
                .read_triage_batch(&self.workspace, after.as_ref(), 100)?;
            if !batch
                .collection
                .as_ref()
                .is_some_and(|state| state.config.mode == IssueCollectionMode::Observe)
            {
                break;
            }
            for item in batch.items {
                if cancel.is_cancelled() {
                    break 'batches;
                }
                if item.binding_run_blocked {
                    // Durable pending is already closed. Fact/Unknown reconciliation ran above.
                    continue;
                }
                if execution_failures.contains(&issue_work_order_id(&item.issue)) {
                    // 실행 사실을 읽거나 고정하지 못한 작업에는 이 tick에서 새 허가를 내지 않는다.
                    continue;
                }
                let issue = item.issue.clone();
                let result =
                    self.reconcile(item, cancel, now_unix_ms, execution_enabled, committed);
                if let Err(error) = self.sync_issue_feedback(&issue, now_unix_ms, cancel, committed)
                {
                    first_error.get_or_insert(error);
                }
                self.report_pending(committed)?;
                if let Err(error) = result {
                    // One unreadable approval must not leave later work's permits valid.
                    first_error.get_or_insert(error);
                }
            }
            after = batch.next_after;
            if after.is_none() {
                break;
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub fn acknowledge_event(&self, event_id: &str) -> Result<()> {
        if event_id.starts_with("issue-arbiter-finished:") {
            return self
                .store
                .acknowledge_issue_arbiter_event(&self.workspace, event_id);
        }
        if event_id.starts_with("issue-decision-reminder:") {
            return self
                .store
                .acknowledge_issue_decision_reminder(&self.workspace, event_id);
        }
        if event_id.starts_with("issue-feedback:") {
            return self
                .feedback
                .as_ref()
                .ok_or_else(|| anyhow!("the issue feedback outbox is unavailable"))?
                .acknowledge_event(event_id);
        }
        self.store
            .acknowledge_issue_work_event(&self.workspace, event_id)
    }

    fn report_pending(&self, committed: &Reporter<'_>) -> Result<()> {
        committed(self.pending_facts()?);
        Ok(())
    }

    pub fn pending_facts(&self) -> Result<Vec<IssueCoordinatorFact>> {
        let mut facts: Vec<_> = self
            .store
            .pending_issue_work_events(&self.workspace)?
            .into_iter()
            .map(|event| match event.kind {
                IssueWorkEventKind::ExecutionProgressed { progress } => {
                    IssueCoordinatorFact::ExecutionProgressed {
                        event_id: event.event_id,
                        issue: event.issue,
                        work_order_id: event.work_order_id,
                        progress,
                        revision: event.revision,
                    }
                }
                IssueWorkEventKind::Integrated { integration } => {
                    IssueCoordinatorFact::Integrated {
                        event_id: event.event_id,
                        issue: event.issue,
                        work_order_id: event.work_order_id,
                        integration,
                        revision: event.revision,
                    }
                }
                IssueWorkEventKind::ExternalClosed {
                    observation_revision,
                } => IssueCoordinatorFact::ExternalClosed {
                    event_id: event.event_id,
                    issue: event.issue,
                    work_order_id: event.work_order_id,
                    observation_revision,
                    revision: event.revision,
                },
                IssueWorkEventKind::Submitted { task_ids } => IssueCoordinatorFact::WorkSubmitted {
                    event_id: event.event_id,
                    issue: event.issue,
                    work_order_id: event.work_order_id,
                    task_ids,
                    revision: event.revision,
                },
                IssueWorkEventKind::PermitGranted {
                    source_sha,
                    gate_revision,
                    delivery,
                } => IssueCoordinatorFact::WorkPermitGranted {
                    event_id: event.event_id,
                    issue: event.issue,
                    work_order_id: event.work_order_id,
                    source_sha,
                    gate_revision,
                    delivery,
                    revision: event.revision,
                },
            })
            .collect();
        if let Some(feedback) = &self.feedback {
            facts.extend(
                feedback
                    .pending_events()?
                    .into_iter()
                    .map(IssueCoordinatorFact::FeedbackPosted),
            );
        }
        facts.extend(
            self.store
                .pending_issue_decision_reminders(&self.workspace, 256)?
                .into_iter()
                .map(IssueCoordinatorFact::DecisionReminded),
        );
        facts.extend(
            self.store
                .pending_issue_arbiter_events(&self.workspace, 256)?
                .into_iter()
                .map(IssueCoordinatorFact::DecisionArbiterFinished),
        );
        Ok(facts)
    }

    /// Scheduling reads only persisted request times; processing rechecks current authority.
    pub(crate) fn next_decision_deadline_at(&self, now_unix_ms: u64) -> Result<Option<u64>> {
        let ordinary = self.decisions.next_deadline_at(now_unix_ms)?;
        let arbiter = self.next_arbiter_deadline_at(now_unix_ms)?;
        Ok(ordinary.into_iter().chain(arbiter).min())
    }

    fn process_decision_deadlines(
        &self,
        cancel: &IssueCollectionCancellation,
        now_unix_ms: u64,
        committed: &Reporter<'_>,
    ) -> Result<()> {
        self.decisions
            .process_deadlines(cancel, now_unix_ms, &|commit| {
                committed(vec![IssueCoordinatorFact::Decision {
                    fact: commit.record.request.fact(),
                    revision: commit.catalog_revision,
                }]);
            })
    }

    fn reconcile(
        &self,
        mut item: IssueCollectionItem,
        cancel: &IssueCollectionCancellation,
        now_unix_ms: u64,
        execution_enabled: bool,
        committed: &Reporter<'_>,
    ) -> Result<()> {
        if item.binding_run_blocked
            || item.tracked.is_none()
            || self
                .store
                .pending_issue_interventions(&self.workspace)?
                .iter()
                .any(|pending| pending.request.issue == item.issue)
        {
            return Ok(());
        }
        let mut work = self.store.get_issue_work(&self.workspace, &item.issue)?;
        if let Some(candidate) = work.as_mut()
            && candidate.task_ids.is_empty()
        {
            // 등록 직후의 crash는 승인 변경과 독립적으로 복구한다.
            // 원격 조회나 새 등록 없이 실행기의 영속 연결만 되찾는다.
            let task_ids = self
                .execution
                .task_ids(&self.workspace, &candidate.order.work_order_id)?;
            if !task_ids.is_empty() {
                candidate.task_ids = task_ids;
                let (fence, current_item, assessment) = self.revoke_on_current_error(
                    Some(candidate),
                    self.current(&item.issue, committed),
                )?;
                let saved = cancel.commit_if_active(|| {
                    self.store
                        .save_issue_work(&self.workspace, fence, &assessment, candidate)
                })?;
                let Some(IssueWorkSaveResult::Saved { record, .. }) = saved else {
                    return Ok(());
                };
                item = current_item;
                work = Some(*record);
            }
        }
        let Some(tracked) = item.tracked.as_ref() else {
            return Ok(());
        };
        let work_order_id = issue_work_order_id(&item.issue);
        let control = self.revoke_on_current_error(
            work.as_ref(),
            self.execution.control(&self.workspace, &work_order_id),
        )?;

        let assessment =
            self.revoke_on_current_error(work.as_ref(), self.triage.assess_current(&item))?;
        if assessment
            .reasons()
            .contains(&crate::domain::issue_automation::triage::IssueTriageReasonCode::HeldOrManual)
            && !control.held
        {
            let request = crate::domain::issue_automation::intervention::IssueInterventionRequest {
                command_id: format!(
                    "remote-hold:{}:{}:{}",
                    work_order_id,
                    control.authorization_epoch,
                    assessment.input_fingerprint()
                ),
                issue: item.issue.clone(),
                action:
                    crate::domain::issue_automation::intervention::IssueInterventionAction::Hold,
            };
            self.store
                .begin_issue_intervention(&self.workspace, &request)?;
            super::issue_intervention::recover_pending_controls(
                &self.workspace,
                self.store.as_ref(),
                self.execution.as_ref(),
            )?;
            return Ok(());
        }
        if !control.allows_automation() {
            return Ok(());
        }
        let basis =
            self.revoke_on_current_error(work.as_ref(), self.approval_basis(&assessment))?;
        if basis.authorization_epoch != control.authorization_epoch {
            return Ok(());
        }
        let decision_id = work
            .as_ref()
            .filter(|work| {
                work.current_authorization_basis().authorization_epoch
                    == control.authorization_epoch
            })
            .and_then(|work| work.current_authorization_decision_id())
            .cloned()
            .unwrap_or_else(|| start_decision_id(&item.issue, &basis));
        let current_decision = self.revoke_on_current_error(
            work.as_ref(),
            self.store.get_issue_decision(&self.workspace, &decision_id),
        )?;
        let decision = current_decision.as_ref().or_else(|| {
            item.open_decisions.iter().find(|decision| {
                if decision.request.definition().basis.authorization_epoch
                    != control.authorization_epoch
                {
                    return false;
                }
                decision.request.definition().kind == IssueDecisionKind::StartApproval
            })
        });
        let execution = if work.as_ref().is_some_and(|work| {
            work.permit_source.is_some()
                && *work.current_authorization_basis() == basis
                && ready_for_human_start(&assessment)
        }) {
            match self
                .execution
                .snapshot(&self.workspace, &issue_work_order_id(&item.issue))
            {
                Ok(snapshot) => snapshot,
                Err(_) => {
                    if let Some(work) = work.as_ref() {
                        self.block_work(
                            work,
                            IssueWorkBlockReason::SourceUnavailable,
                            cancel,
                            committed,
                        )?;
                    }
                    return Ok(());
                }
            }
        } else {
            Default::default()
        };
        let mut action = next_action(IssueCoordinatorInput {
            authorization_epoch: control.authorization_epoch,
            state: tracked.state,
            flags: tracked.flags,
            assessment: Some(&assessment),
            decision: decision.map(|stored| &stored.request),
            work: work.as_ref(),
            frozen_source: execution.frozen_source.as_deref(),
            now_unix_ms,
        });
        if action == IssueCoordinatorAction::None
            && !matches!(
                tracked.state,
                IssueAutomationState::Done
                    | IssueAutomationState::Cancelled
                    | IssueAutomationState::Failed
            )
            && let Some(work) = &work
            && work.current_authorization_basis() == &basis
            && ready_for_human_start(&assessment)
            && match work.current_start_authority() {
                IssueWorkStartAuthority::Policy => policy_start(&assessment),
                IssueWorkStartAuthority::Human => {
                    decision.is_some_and(|decision| human_start(&decision.request))
                }
                IssueWorkStartAuthority::TimeoutDefault => decision
                    .is_some_and(|decision| timeout_default_start(&assessment, &decision.request)),
                IssueWorkStartAuthority::TimeoutArbiter => decision
                    .is_some_and(|decision| timeout_arbiter_start(&assessment, &decision.request)),
            }
        {
            let floor = self.revoke_on_current_error(
                Some(work),
                self.execution
                    .permit_gate_revision(&self.workspace, &work.order.work_order_id),
            )?;
            if floor > work.permit_revision
                || (execution.frozen_source.is_none()
                    && execution
                        .dispatch_source
                        .as_ref()
                        .is_some_and(|source| work.permit_source.as_ref() != Some(source)))
            {
                // New lineage or a changed pre-dispatch base needs the normal fresh approval check.
                action = IssueCoordinatorAction::SubmitWork;
            }
        }
        match action {
            IssueCoordinatorAction::OpenDecision => {
                let mut definition = start_definition(&item.issue, basis)?;
                if assessment.requires_provider_human_start_review() {
                    definition.question = IssueDecisionSharedText::new_shared(
                        if item.issue.provider == crate::domain::issue_work::IssueProvider::Linear {
                            "Linear에서 연결된 PR 유무를 확인할 수 없습니다. 직접 확인한 뒤 이 이슈 작업을 시작할까요?"
                        } else {
                            "Jira에서 연결된 PR 유무를 확인할 수 없습니다. 직접 확인한 뒤 이 이슈 작업을 시작할까요?"
                        },
                    )?;
                    definition.options[0].effect = IssueDecisionSharedText::new_shared(
                        "진행 중인 PR이나 중복 작업이 없는지 직접 확인한 후 시작을 승인합니다. 이 확인은 현재 이슈 판의 작업 시작에만 적용되며 자동 시작이나 자동 통합을 허용하지 않습니다.",
                    )?;
                }
                if assessment.gates().iter().any(|gate| {
                    gate.gate == IssueAutomationGate::Start
                        && gate.action == crate::domain::issue_automation::policy::IssueAutomationGateAction::TimeoutDefault
                }) {
                    definition.on_timeout = IssueDecisionTimeout::Default;
                    definition.recommended = Some("start".to_owned());
                } else if assessment.gates().iter().any(|gate| {
                    gate.gate == IssueAutomationGate::Start && gate.action
                        == crate::domain::issue_automation::policy::IssueAutomationGateAction::TimeoutArbiter
                }) {
                    definition.on_timeout = IssueDecisionTimeout::Arbiter;
                    definition.recommended = Some("start".to_owned());
                }
                cancel.commit_if_active(|| {
                    let commit = self.decisions.ensure_request_at(decision_id, definition, now_unix_ms)?;
                    if commit.changed {
                        committed(vec![IssueCoordinatorFact::Decision {
                            fact: commit.record.request.fact(),
                            revision: commit.catalog_revision,
                        }]);
                    }
                    if !matches!(tracked.state, IssueAutomationState::Queued) {
                        self.transition(&item.issue, IssueTransition::AwaitDecision)?;
                    }
                    Ok(())
                })?;
            }
            IssueCoordinatorAction::ExpireDecision => {
                if let Some(stored) = decision {
                    let (fence, current, assessment) = self.revoke_on_current_error(
                        work.as_ref(),
                        self.current(&item.issue, committed),
                    )?;
                    let _ = current;
                    let resolution = stored.request.answer_agent(
                        &basis,
                        IssueDecisionAgentAnswer {
                            option: "start".into(),
                            note: None,
                            caller: Some("issue-coordinator".into()),
                            channel: "issue-coordinator".into(),
                            at: "1970-01-01T00:00:00Z".into(),
                        },
                    )?;
                    cancel.commit_if_active(|| {
                        if let IssueDecisionSaveResult::Saved(commit) =
                            self.store.save_issue_decision(
                                &self.workspace,
                                IssueDecisionFence {
                                    triage: fence,
                                    decision_revision: stored.revision,
                                },
                                &assessment,
                                &resolution.request,
                            )?
                            && commit.changed
                        {
                            committed(vec![IssueCoordinatorFact::Decision {
                                fact: commit.record.request.fact(),
                                revision: commit.catalog_revision,
                            }]);
                        }
                        Ok(())
                    })?;
                }
            }
            IssueCoordinatorAction::SubmitWork => {
                let preserve_decision = work.as_ref().is_some_and(|work| {
                    work.current_start_authority() != IssueWorkStartAuthority::Policy
                        && work.current_authorization_basis() == &basis
                });
                let authorization = if policy_start(&assessment) && !preserve_decision {
                    Some(StartAuthorization::Policy(&basis))
                } else {
                    decision.map(|decision| {
                        if timeout_default_start(&assessment, &decision.request) {
                            StartAuthorization::TimeoutDefault(&decision.request)
                        } else if timeout_arbiter_start(&assessment, &decision.request) {
                            StartAuthorization::TimeoutArbiter(&decision.request)
                        } else { StartAuthorization::Human(&decision.request) }
                    })
                };
                if let Some(authorization) = authorization {
                    self.submit(
                        &item.issue,
                        StartContext {
                            authorization,
                            now_unix_ms,
                            execution_enabled,
                        },
                        work,
                        cancel,
                        committed,
                    )?;
                }
            }
            IssueCoordinatorAction::CancelWork => {
                if let Some(mut work) = work {
                    cancel.commit_if_active(|| {
                        self.invalidate_delivery(&mut work, IssueWorkBlockReason::ApprovalChanged)
                    })?;
                }
            }
            IssueCoordinatorAction::None => {
                if let Some(mut work) = work {
                    let reason = if !execution_enabled {
                        Some(IssueWorkBlockReason::ExecutionDisabled)
                    } else if !work.delivery_granted {
                        Some(IssueWorkBlockReason::AwaitingSource)
                    } else {
                        None
                    };
                    if work.blocked_reason != reason {
                        work.blocked_reason = reason;
                        let (fence, _, assessment) = self.revoke_on_current_error(
                            Some(&work),
                            self.current(&work.order.issue, committed),
                        )?;
                        cancel.commit_if_active(|| {
                            self.store.save_issue_work(
                                &self.workspace,
                                fence,
                                &assessment,
                                &work,
                            )?;
                            Ok(())
                        })?;
                    }
                } else if ready_for_human_start(&assessment)
                    && assessment.reasons().contains(
                        &crate::domain::issue_automation::triage::IssueTriageReasonCode::BudgetExhausted,
                    )
                {
                    cancel.commit_if_active(|| {
                        self.ready(&item.issue)?;
                        self.current(&item.issue, committed)?;
                        Ok(())
                    })?;
                } else if decision
                    .is_some_and(|stored| stored.request.state() == IssueDecisionState::Answered)
                {
                    cancel.commit_if_active(|| self.ready(&item.issue))?;
                }
            }
            // 기한은 deadline 단계에서, 피드백과 완료 처리는 각 전용 경로에서 실행한다.
            IssueCoordinatorAction::Triage
            | IssueCoordinatorAction::PostFeedback
            | IssueCoordinatorAction::MarkDone => {}
        }
        Ok(())
    }

    fn submit(
        &self,
        issue: &IssueReference,
        start: StartContext<'_>,
        existing: Option<IssueWorkCheckpoint>,
        cancel: &IssueCollectionCancellation,
        committed: &Reporter<'_>,
    ) -> Result<()> {
        let StartContext {
            authorization,
            now_unix_ms,
            execution_enabled,
        } = start;
        let initial_admission = existing.is_none();
        if let Some(work) = existing.as_ref()
            && execution::submission_blocked(self, work)?
        {
            return Ok(());
        }
        // 느린 integration target fetch를 먼저 마친다. 제출 직전의 마지막 원격 읽기는 이슈다.
        let execution = self
            .execution
            .snapshot(&self.workspace, &issue_work_order_id(issue));
        // 승인 직후에도 반드시 원격을 다시 읽는다. 취소 중에는 원격을 취소할 수 없어도 저장/제출은 하지 않는다.
        if !self.revoke_on_current_error(existing.as_ref(), self.fresh_resync(issue, cancel))? {
            return Ok(());
        }
        let (fence, item, assessment) =
            self.revoke_on_current_error(existing.as_ref(), self.current(issue, committed))?;
        let basis =
            self.revoke_on_current_error(existing.as_ref(), self.approval_basis(&assessment))?;
        let control = self.revoke_on_current_error(
            existing.as_ref(),
            self.execution
                .control(&self.workspace, &issue_work_order_id(issue)),
        )?;
        if !control.allows_automation()
            || control.authorization_epoch != basis.authorization_epoch
            || !authorization.permits(&assessment, &basis)
            || item.tracked.as_ref().is_none_or(|tracked| {
                tracked.flags.held || tracked.flags.stale || tracked.flags.unknown
            })
        {
            if let Some(work) = existing {
                cancel.commit_if_active(|| {
                    self.execution
                        .revoke(&self.workspace, &work.order.work_order_id)
                })?;
                self.block_work(
                    &work,
                    IssueWorkBlockReason::ApprovalChanged,
                    cancel,
                    committed,
                )?;
            }
            return Ok(());
        }
        let mut work = match existing {
            Some(work) => work,
            None => self.build_work(&item, authorization)?,
        };
        let provider_link = self.revoke_on_current_error(
            Some(&work),
            self.triage.provider_link_for_policy(&basis.policy_version),
        )?;
        if work.order.provider_link != provider_link {
            cancel.commit_if_active(|| {
                self.execution
                    .revoke(&self.workspace, &work.order.work_order_id)
            })?;
            self.block_work(
                &work,
                IssueWorkBlockReason::ApprovalChanged,
                cancel,
                committed,
            )?;
            return Ok(());
        }
        if work.current_authorization_basis() != &basis
            || work.current_start_authority() != authorization.authority()
            || work.current_authorization_decision_id() != authorization.decision_id()
        {
            if work.order.field_hash
                != *basis
                    .field_hash
                    .as_ref()
                    .ok_or_else(|| anyhow!("the renewed approval has no issue hash"))?
            {
                bail!("changed issue fields require a new reviewed work instruction");
            }
            work.authorization = Some(IssueWorkAuthorization {
                start_authority: authorization.authority(),
                decision_id: authorization.decision_id().cloned(),
                basis: basis.clone(),
            });
            self.execution
                .revoke(&self.workspace, &work.order.work_order_id)?;
            work.delivery_assessment = None;
            work.delivery_granted = false;
        }
        // 실행 baseline을 읽을 수 없어도 승인한 작업 등록은 보존한다. 허가만 보류한다.
        let execution = execution.ok();
        let source = execution.as_ref().and_then(|snapshot| {
            snapshot
                .frozen_source
                .clone()
                .or(snapshot.dispatch_source.clone())
                .or(snapshot.saved_source.clone())
                .or_else(|| work.permit_source.clone())
        });
        let delivery = execution
            .as_ref()
            .is_some_and(|snapshot| snapshot.frozen_source.is_some());
        // source 변경만 더 높은 판을 허용한다. revoke된 동일 source를 새 판으로 재승인하지 않는다.
        let prior_source = execution
            .as_ref()
            .and_then(|snapshot| snapshot.saved_source.as_deref())
            .or(work.permit_source.as_deref());
        let prior_revision = execution
            .as_ref()
            .map(|snapshot| snapshot.saved_gate_revision)
            .unwrap_or(0)
            .max(work.permit_revision)
            .max(basis.gate_revision)
            .max(
                self.revoke_on_current_error(
                    Some(&work),
                    self.execution
                        .permit_gate_revision(&self.workspace, &work.order.work_order_id),
                )?,
            );
        let gate_revision = if prior_source
            .zip(source.as_deref())
            .is_some_and(|(old, source)| old != source)
            || execution.as_ref().is_some_and(|snapshot| {
                snapshot.saved_source.is_some()
                    && snapshot.saved_authorization_epoch != basis.authorization_epoch
            }) {
            prior_revision
                .checked_add(1)
                .filter(|revision| *revision <= i64::MAX as u64)
                .ok_or_else(|| anyhow!("the work permit revision is exhausted"))?
        } else {
            prior_revision
        };
        let mut grant_source = None;
        cancel.commit_if_active(|| {
            // 첫 intent와 예산 영수증을 함께 저장한다. 응답 유실 뒤에는 기존 intent를 복구한다.
            if initial_admission {
                let budget = self.triage.start_budget_for_policy(&basis.policy_version)?;
                match self.store.admit_issue_work(
                    &self.workspace,
                    fence,
                    &assessment,
                    &work,
                    &budget,
                    now_unix_ms,
                )? {
                    IssueWorkAdmissionResult::Saved { record, .. } => work = *record,
                    IssueWorkAdmissionResult::BudgetExhausted { .. } => {
                        self.ready(issue)?;
                        // Ready changes the tracked fence, not the approval fingerprint.
                        self.current(issue, committed)?;
                        return Ok(());
                    }
                    IssueWorkAdmissionResult::Conflict => return Ok(()),
                }
            } else {
                let IssueWorkSaveResult::Saved { record, .. } =
                    self.store
                        .save_issue_work(&self.workspace, fence, &assessment, &work)?
                else {
                    return Ok(());
                };
                work = *record;
            }
            self.execution
                .control(&self.workspace, &work.order.work_order_id)?
                .require_epoch(basis.authorization_epoch)?;
            let task_ids = match self.execution.submit(&self.workspace, &work.order) {
                Ok(task_ids) => task_ids,
                Err(_) => {
                    work.blocked_reason = Some(IssueWorkBlockReason::SubmissionUnavailable);
                    let (fence, _, assessment) = self.revoke_on_current_error(
                        Some(&work),
                        self.current(&work.order.issue, committed),
                    )?;
                    self.store
                        .save_issue_work(&self.workspace, fence, &assessment, &work)?;
                    bail!("the issue work submission was interrupted");
                }
            };
            work.task_ids = task_ids;
            let max_attempts =
                self.revoke_on_current_error(Some(&work), self.triage.execution_attempt_limit())?;
            self.revoke_on_current_error(
                Some(&work),
                self.execution.execution_limit(
                    &self.workspace,
                    &work.order.work_order_id,
                    max_attempts,
                ),
            )?;
            work.blocked_reason = if source.is_none() {
                Some(IssueWorkBlockReason::SourceUnavailable)
            } else if delivery
                && work
                    .delivery_assessment
                    .as_ref()
                    .is_some_and(|review| review.verdict.requires_human())
                && !work.delivery_granted
            {
                Some(IssueWorkBlockReason::AwaitingDeliveryApproval)
            } else if execution_enabled {
                Some(IssueWorkBlockReason::AwaitingSource)
            } else {
                Some(IssueWorkBlockReason::ExecutionDisabled)
            };
            let (fence, current_item, current, current_basis) =
                self.current_or_revoke(&work, committed)?;
            // 로컬 정책과 표시도 등록 후 다시 확인한다. 판이 바뀌면 task는 permit 없이 남는다.
            if !matches_approval(&current_item, &current, authorization, &current_basis) {
                self.execution
                    .revoke(&self.workspace, &work.order.work_order_id)?;
                work.delivery_assessment = None;
                work.delivery_granted = false;
                work.blocked_reason = Some(IssueWorkBlockReason::ApprovalChanged);
                self.store
                    .save_issue_work(&self.workspace, fence, &current, &work)?;
                return Ok(());
            }
            let IssueWorkSaveResult::Saved { record, .. } =
                self.store
                    .save_issue_work(&self.workspace, fence, &current, &work)?
            else {
                return Ok(());
            };
            work = *record;
            self.ready(issue)?;
            if self
                .store
                .get_issue(&self.workspace, issue)?
                .is_some_and(|tracked| {
                    matches!(
                        tracked.state(),
                        IssueAutomationState::Ready | IssueAutomationState::Queued
                    )
                })
            {
                self.transition(issue, IssueTransition::Queue)?;
            }
            grant_source = source;
            Ok(())
        })?;
        let Some(source) = grant_source else {
            return Ok(());
        };
        let delivery_review = if delivery {
            let Some(review) = self.prepare_delivery(
                &mut work,
                &source,
                prior_revision,
                now_unix_ms,
                cancel,
                committed,
            )?
            else {
                return Ok(());
            };
            Some(review)
        } else {
            None
        };
        let gate_revision = delivery_review
            .as_ref()
            .map_or(gate_revision, |review| review.gate_revision);
        if !self.prepare_feedback_for_grant(&work, cancel, committed)? {
            return Ok(());
        }
        cancel.commit_if_active(|| {
            let (fence, current_item, current, current_basis) =
                self.current_or_revoke(&work, committed)?;
            if !matches_approval(&current_item, &current, authorization, &current_basis) {
                self.execution
                    .revoke(&self.workspace, &work.order.work_order_id)?;
                work.delivery_assessment = None;
                work.delivery_granted = false;
                work.blocked_reason = Some(IssueWorkBlockReason::ApprovalChanged);
                self.store
                    .save_issue_work(&self.workspace, fence, &current, &work)?;
                return Ok(());
            }
            if let Some(review) = &delivery_review
                && !self.delivery_review_current(&work, review, &current)?
            {
                self.invalidate_delivery(&mut work, IssueWorkBlockReason::ApprovalChanged)?;
                return Ok(());
            }
            let delivery_authority = match delivery_review
                .as_ref()
                .map(|review| {
                    self.current_delivery_authority(&work, &current_item, &current, review)
                })
                .transpose()
            {
                Ok(authority) => authority,
                Err(error) => {
                    self.invalidate_delivery(&mut work, IssueWorkBlockReason::SourceUnavailable)?;
                    return Err(error);
                }
            };
            if let Err(error) = self.execution.grant(
                &self.workspace,
                &IssueWorkPermitRequest {
                    authorization_epoch: basis.authorization_epoch,
                    work_order_id: work.order.work_order_id.clone(),
                    approval: basis.clone(),
                    source_sha: source.clone(),
                    gate_revision,
                    delivery,
                    delivery_review: delivery_review.clone(),
                    delivery_authority: delivery_authority.clone(),
                },
            ) {
                if delivery
                    && !self
                        .delivery_grant_retryable(&work, committed)
                        .unwrap_or(false)
                {
                    self.invalidate_delivery(&mut work, IssueWorkBlockReason::SourceUnavailable)?;
                }
                return Err(error);
            }
            let (fence, current_item, current, current_basis) =
                self.current_or_revoke(&work, committed)?;
            if !matches_approval(&current_item, &current, authorization, &current_basis)
                || delivery_review.as_ref().is_some_and(|review| {
                    !self
                        .delivery_review_current(&work, review, &current)
                        .unwrap_or(false)
                })
                || delivery_review.as_ref().is_some_and(|review| {
                    self.current_delivery_authority(&work, &current_item, &current, review)
                        .ok()
                        != delivery_authority
                })
            {
                self.execution
                    .revoke(&self.workspace, &work.order.work_order_id)?;
                work.delivery_assessment = None;
                work.delivery_granted = false;
                work.blocked_reason = Some(IssueWorkBlockReason::ApprovalChanged);
                self.store
                    .save_issue_work(&self.workspace, fence, &current, &work)?;
                return Ok(());
            }
            work.permit_authorization_epoch =
                work.current_authorization_basis().authorization_epoch;
            work.permit_source = Some(source.clone());
            work.permit_revision = gate_revision;
            work.delivery_granted = delivery;
            if delivery && execution_enabled {
                work.blocked_reason = None;
            }
            match self
                .store
                .save_issue_work(&self.workspace, fence, &current, &work)
            {
                Ok(IssueWorkSaveResult::Saved { .. }) => {}
                Ok(IssueWorkSaveResult::Conflict) => {
                    self.execution
                        .revoke(&self.workspace, &work.order.work_order_id)?;
                }
                Err(error) => {
                    self.execution
                        .revoke(&self.workspace, &work.order.work_order_id)?;
                    return Err(error);
                }
            }
            Ok(())
        })?;
        Ok(())
    }

    fn current_or_revoke(
        &self,
        work: &IssueWorkCheckpoint,
        committed: &Reporter<'_>,
    ) -> Result<(
        IssueTriageFence,
        IssueCollectionItem,
        IssueTriageAssessment,
        IssueDecisionBasis,
    )> {
        let current = (|| {
            let (fence, item, assessment) = self.current(&work.order.issue, committed)?;
            let basis = self.approval_basis(&assessment)?;
            self.execution
                .control(&self.workspace, &work.order.work_order_id)?
                .require_epoch(work.current_authorization_basis().authorization_epoch)?;
            Ok((fence, item, assessment, basis))
        })();
        self.revoke_on_current_error(Some(work), current)
    }

    /// Failed current-policy or approval reads cannot leave an old capability usable.
    /// Execution responses are intentionally excluded: their durable results can be replayed.
    fn revoke_on_current_error<T>(
        &self,
        work: Option<&IssueWorkCheckpoint>,
        result: Result<T>,
    ) -> Result<T> {
        match result {
            Ok(current) => Ok(current),
            Err(error) => {
                if let Some(work) = work
                    && let Err(revoke_error) = self.invalidate_delivery(
                        &mut work.clone(),
                        IssueWorkBlockReason::ApprovalChanged,
                    )
                {
                    return Err(error.context(format!(
                        "existing issue work permits could not be revoked: {revoke_error}"
                    )));
                }
                Err(error)
            }
        }
    }

    fn block_work(
        &self,
        work: &IssueWorkCheckpoint,
        reason: IssueWorkBlockReason,
        cancel: &IssueCollectionCancellation,
        _committed: &Reporter<'_>,
    ) -> Result<()> {
        let mut candidate = work.clone();
        cancel.commit_if_active(|| self.invalidate_delivery(&mut candidate, reason))?;
        Ok(())
    }

    fn fresh_resync(
        &self,
        issue: &IssueReference,
        cancel: &IssueCollectionCancellation,
    ) -> Result<bool> {
        let Some(state) = self.store.get_collection(&self.workspace)? else {
            return Ok(false);
        };
        if state.config.mode != IssueCollectionMode::Observe || cancel.is_cancelled() {
            return Ok(false);
        }
        let publication_lease = self
            .publication_journal
            .as_ref()
            .map(|journal| journal.acquire_publication_lease(&self.workspace))
            .transpose()?;
        if let Some(lease) = &publication_lease {
            lease.verify_identity()?;
        }
        let policy = self.policy.resolve(&self.workspace)?;
        let disclosure = IssueSourceDisclosure {
            policy: *policy.disclosure_for(IssueDisclosureTarget::AiHandoff),
            policy_version: issue_policy_version(&policy),
        };
        let policy_version = disclosure.policy_version.clone();
        let read = checked_resync_read(
            issue,
            &policy_version,
            self.source.resync(IssueResyncRequest {
                issue: issue.clone(),
                disclosure,
            }),
        );
        // 미확정 자기 라벨은 새 관측 근거가 아니다. 기존 승인과 flags를 보존하되 허가는 미룬다.
        if matches!(
            &read,
            IssueSourceRead::Unknown {
                reason:
                    crate::domain::issue_automation::source::IssueSourceFailure::FeedbackUnconfirmed,
                ..
            }
        ) {
            return Ok(false);
        }
        let result = cancel.commit_if_active(|| {
            if let Some(lease) = &publication_lease {
                lease.verify_identity()?;
            }
            self.store
                .apply_issue_resync(&self.workspace, state.fence(), issue, &read)
        })?;
        Ok(matches!(
            result,
            Some(IssueCollectionApplyResult::Applied(_))
        ))
    }

    fn current(
        &self,
        issue: &IssueReference,
        committed: &Reporter<'_>,
    ) -> Result<(IssueTriageFence, IssueCollectionItem, IssueTriageAssessment)> {
        for _ in 0..3 {
            let mut after = None;
            loop {
                let batch = self
                    .store
                    .read_triage_batch(&self.workspace, after.as_ref(), 100)?;
                let Some(control) = batch
                    .collection
                    .filter(|state| state.config.mode == IssueCollectionMode::Observe)
                else {
                    bail!("issue coordination is paused");
                };
                if let Some(mut item) = batch.items.into_iter().find(|item| item.issue == *issue) {
                    let assessment = self.triage.assess_current(&item)?;
                    let mut fence = IssueTriageFence {
                        binding_run_revision: item.binding_run_revision,
                        collection_epoch: control.epoch,
                        observation_revision: item.observation_revision,
                        tracked_revision: item.tracked_revision,
                        assessment_revision: item.triage.as_ref().map(|triage| triage.revision),
                    };
                    if let IssueTriageSaveResult::Saved {
                        record,
                        binding_id,
                        catalog_revision,
                        changed,
                    } = self
                        .store
                        .save_triage(&self.workspace, fence, &assessment)?
                    {
                        if changed {
                            committed(vec![IssueCoordinatorFact::Triaged(IssueTriagedFact {
                                issue: item.issue.clone(),
                                binding_id,
                                revision: catalog_revision,
                                assessment_revision: record.revision,
                                gate_revision: record
                                    .assessment
                                    .gates()
                                    .iter()
                                    .map(|gate| gate.gate_revision)
                                    .max()
                                    .ok_or_else(|| {
                                        anyhow!("the committed issue assessment has no gates")
                                    })?,
                                policy_version: record
                                    .assessment
                                    .basis()
                                    .automation_policy_version
                                    .clone(),
                                field_hash: record.assessment.basis().field_hash.clone(),
                            })]);
                        }
                        fence.assessment_revision = Some(record.revision);
                        item.triage = Some(*record);
                        return Ok((fence, item, assessment));
                    }
                    break;
                }
                after = batch.next_after;
                if after.is_none() {
                    bail!("the tracked issue is unavailable");
                }
            }
        }
        bail!("the issue changed concurrently")
    }

    fn build_work(
        &self,
        item: &IssueCollectionItem,
        authorization: StartAuthorization<'_>,
    ) -> Result<IssueWorkCheckpoint> {
        let basis = authorization.basis();
        let Some(IssueSourceRead::Observed(observation)) = &item.current else {
            bail!("the current issue is unknown")
        };
        let IssueSourceContent::Shared(snapshot) = &observation.content else {
            bail!("the current issue handoff is withheld")
        };
        if basis.field_hash.as_ref() != Some(&snapshot.basis.field_hash) {
            bail!("the handoff is not the approved issue fields");
        }
        let acceptance = issue_acceptance_criteria(&snapshot.shared_copy.body)
            .map_err(|reason| anyhow!(reason))?;
        let criteria_adoption = match authorization {
            StartAuthorization::Human(decision)
                if decision
                    .answer()
                    .is_some_and(|answer| answer.criteria.is_some()) =>
            {
                let stored = self
                    .store
                    .get_issue_decision_commit(&self.workspace, decision.id())?
                    .ok_or_else(|| anyhow!("the confirmed criteria decision is unavailable"))?;
                if stored.record.request != *decision {
                    bail!("the criteria decision changed before work admission");
                }
                crate::domain::issue_automation::criteria::IssueWorkCriteriaAdoption::from_stored(
                    &stored.record,
                )
                .map_err(anyhow::Error::msg)?
            }
            _ => None,
        };
        Ok(IssueWorkCheckpoint {
            authorization: None,
            permit_authorization_epoch: 0,
            order: IssueWorkOrder {
                criteria_adoption,
                work_order_id: issue_work_order_id(&item.issue),
                issue: item.issue.clone(),
                field_hash: snapshot.basis.field_hash.clone(),
                title: snapshot.shared_copy.title.clone(),
                handoff: snapshot.shared_copy.clone(),
                acceptance,
                scope: self.workspace.clone(),
                // A provider suggestion only; the slot allocator still owns the real branch.
                branch_hint: observation
                    .linear
                    .as_ref()
                    .and_then(|linear| linear.branch_name.clone()),
                link_reference: observation
                    .web
                    .as_ref()
                    .map(|web| web.url.clone())
                    .or_else(|| observation.jira.as_ref().map(|jira| jira.url.clone()))
                    .or_else(|| observation.linear.as_ref().map(|linear| linear.url.clone())),
                provider_link: self
                    .triage
                    .provider_link_for_policy(&basis.policy_version)?,
                profile: None,
                priority: 80,
                attempt: 1,
            },
            start_authority: authorization.authority(),
            decision_id: authorization.decision_id().cloned(),
            approved_basis: basis.clone(),
            task_ids: Vec::new(),
            permit_source: None,
            permit_revision: 0,
            delivery_granted: false,
            delivery_assessment: None,
            delivery_revision: 0,
            blocked_reason: None,
            revision: 0,
        })
    }

    fn approval_basis(&self, assessment: &IssueTriageAssessment) -> Result<IssueDecisionBasis> {
        let mut basis =
            IssueDecisionBasis::from_assessment(assessment, IssueAutomationGate::Start, None)?;
        basis.authorization_epoch = self
            .store
            .issue_authorization_epoch(&self.workspace, &assessment.basis().issue)?;
        Ok(basis)
    }

    fn transition(&self, issue: &IssueReference, transition: IssueTransition) -> Result<()> {
        let Some(mut tracked) = self.store.get_issue(&self.workspace, issue)? else {
            return Ok(());
        };
        let revision = tracked.revision();
        tracked.transition(transition)?;
        self.store.save_issue(&self.workspace, revision, &tracked)?;
        Ok(())
    }

    fn ready(&self, issue: &IssueReference) -> Result<()> {
        let Some(tracked) = self.store.get_issue(&self.workspace, issue)? else {
            return Ok(());
        };
        if tracked.state() == IssueAutomationState::AwaitingDecision {
            self.transition(issue, IssueTransition::ResumeDecision)?;
        }
        let Some(tracked) = self.store.get_issue(&self.workspace, issue)? else {
            return Ok(());
        };
        if tracked.state() == IssueAutomationState::Discovered {
            self.transition(issue, IssueTransition::Assess(IssueTriageResult::Ready))?;
        }
        Ok(())
    }
}

fn start_definition(
    issue: &IssueReference,
    basis: IssueDecisionBasis,
) -> Result<IssueDecisionDefinition> {
    let text = |value: &str| IssueDecisionSharedText::new_shared(value);
    Ok(IssueDecisionDefinition {
        subject: IssueDecisionSubject::Issue(issue.clone()),
        gate: IssueAutomationGate::Start,
        kind: IssueDecisionKind::StartApproval,
        question: text("이 이슈 작업을 시작할까요?")?,
        options: vec![
            IssueDecisionOption {
                id: "start".into(),
                label: text("시작")?,
                effect: text(
                    "작업을 대기열에 등록하고 승인한 판의 실행 허가를 만듭니다. 통합에는 사람 리뷰가 필요합니다.",
                )?,
            },
            IssueDecisionOption {
                id: "reject".into(),
                label: text("시작하지 않음")?,
                effect: text("이 판의 작업을 등록하지 않습니다.")?,
            },
        ],
        recommended: None,
        basis,
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: None,
        asked_by: "issue-coordinator".into(),
        evidence: Vec::new(),
        delivery_review: None,
    })
}

fn matches_approval(
    item: &IssueCollectionItem,
    assessment: &IssueTriageAssessment,
    authorization: StartAuthorization<'_>,
    current_basis: &IssueDecisionBasis,
) -> bool {
    authorization.permits(assessment, current_basis)
        && item.tracked.as_ref().is_some_and(|tracked| {
            !tracked.flags.held && !tracked.flags.stale && !tracked.flags.unknown
        })
}

mod execution;

#[cfg(test)]
mod tests;

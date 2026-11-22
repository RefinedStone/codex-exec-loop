//! 저장된 승인만 소비하는 propose 조정기다. 모델 호출 없이 host tick에서 진행한다.
use super::issue_collection::{IssueCollectionCancellation, checked_resync_read};
use super::issue_decision::IssueDecisionService;
use super::issue_policy::{IssuePolicyService, issue_policy_version};
use super::issue_triage::{IssueTriageService, IssueTriagedFact};
use crate::application::port::outbound::issue_automation_store_port::{
    IssueAutomationStorePort, IssueCollectionApplyResult, IssueDecisionFence,
    IssueDecisionSaveResult, IssueTriageFence, IssueTriageSaveResult, IssueWorkSaveResult,
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
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueCoordinatorFact {
    Triaged(IssueTriagedFact),
    Decision {
        fact: IssueDecisionFact,
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

pub struct IssueCoordinatorService {
    workspace: String,
    store: Arc<dyn IssueAutomationStorePort>,
    source: Arc<dyn IssueSourcePort>,
    triage: Arc<IssueTriageService>,
    decisions: Arc<IssueDecisionService>,
    policy: IssuePolicyService,
    execution: Arc<dyn IssueWorkExecutionPort>,
    /// 한 host의 동시 tick/reconnect 호출을 직렬화한다. 실행기 상태는 소유하지 않는다.
    tick_gate: Mutex<()>,
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
            tick_gate: Mutex::new(()),
        }
    }

    pub fn tick(
        &self,
        cancel: &IssueCollectionCancellation,
        now_unix_ms: u64,
        execution_enabled: bool,
        committed: &Reporter<'_>,
    ) -> Result<()> {
        let Ok(_guard) = self.tick_gate.try_lock() else {
            return Ok(());
        };
        self.report_pending(committed)?;
        let mut first_error = None;
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
                let result =
                    self.reconcile(item, cancel, now_unix_ms, execution_enabled, committed);
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
        self.store
            .acknowledge_issue_work_event(&self.workspace, event_id)
    }

    fn report_pending(&self, committed: &Reporter<'_>) -> Result<()> {
        committed(self.pending_facts()?);
        Ok(())
    }

    pub fn pending_facts(&self) -> Result<Vec<IssueCoordinatorFact>> {
        let facts = self
            .store
            .pending_issue_work_events(&self.workspace)?
            .into_iter()
            .map(|event| match event.kind {
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
        Ok(facts)
    }

    fn reconcile(
        &self,
        mut item: IssueCollectionItem,
        cancel: &IssueCollectionCancellation,
        now_unix_ms: u64,
        execution_enabled: bool,
        committed: &Reporter<'_>,
    ) -> Result<()> {
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
                let (fence, current_item, assessment) =
                    self.current_or_revoke(candidate, committed)?;
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
        let assessment =
            self.revoke_on_current_error(work.as_ref(), self.triage.assess_current(&item))?;
        let basis = self.revoke_on_current_error(
            work.as_ref(),
            IssueDecisionBasis::from_assessment(&assessment, IssueAutomationGate::Start, None)
                .map_err(anyhow::Error::from),
        )?;
        let decision_id = work
            .as_ref()
            .map(|work| work.decision_id.clone())
            .unwrap_or_else(|| start_decision_id(&item.issue, &basis));
        let current_decision = self.revoke_on_current_error(
            work.as_ref(),
            self.store.get_issue_decision(&self.workspace, &decision_id),
        )?;
        let decision = current_decision.as_ref().or_else(|| {
            item.open_decisions.iter().find(|decision| {
                decision.request.definition().kind == IssueDecisionKind::StartApproval
            })
        });
        let execution = if work.as_ref().is_some_and(|work| {
            work.permit_source.is_some()
                && work.approved_basis == basis
                && ready_for_propose(&assessment)
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
        let action = next_action(IssueCoordinatorInput {
            state: tracked.state,
            flags: tracked.flags,
            assessment: Some(&assessment),
            decision: decision.map(|stored| &stored.request),
            work: work.as_ref(),
            frozen_source: execution.frozen_source.as_deref(),
            now_unix_ms,
        });
        match action {
            IssueCoordinatorAction::OpenDecision => {
                let definition = start_definition(&item.issue, basis)?;
                cancel.commit_if_active(|| {
                    let commit = self.decisions.ensure_request(decision_id, definition)?;
                    if commit.changed {
                        committed(vec![IssueCoordinatorFact::Decision {
                            fact: commit.record.request.fact(),
                            revision: commit.catalog_revision,
                        }]);
                    }
                    self.transition(&item.issue, IssueTransition::AwaitDecision)?;
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
                if let Some(decision) = decision {
                    self.submit(
                        &item.issue,
                        &decision.request,
                        work,
                        cancel,
                        execution_enabled,
                        committed,
                    )?;
                }
            }
            IssueCoordinatorAction::CancelWork => {
                if let Some(mut work) = work {
                    cancel.commit_if_active(|| {
                        self.execution
                            .revoke(&self.workspace, &work.order.work_order_id)?;
                        work.blocked_reason = Some(IssueWorkBlockReason::ApprovalChanged);
                        let (fence, _, assessment) = self.current(&item.issue, committed)?;
                        self.store
                            .save_issue_work(&self.workspace, fence, &assessment, &work)?;
                        Ok(())
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
                        let (fence, _, assessment) = self.current_or_revoke(&work, committed)?;
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
                } else if decision
                    .is_some_and(|stored| stored.request.state() == IssueDecisionState::Answered)
                {
                    cancel.commit_if_active(|| self.ready(&item.issue))?;
                }
            }
            // 이 slice는 expiry timer, feedback, 완료 수명주기를 실행하지 않는다.
            IssueCoordinatorAction::Triage
            | IssueCoordinatorAction::PostFeedback
            | IssueCoordinatorAction::MarkDone => {}
        }
        Ok(())
    }

    fn submit(
        &self,
        issue: &IssueReference,
        decision: &IssueDecisionRequest,
        existing: Option<IssueWorkCheckpoint>,
        cancel: &IssueCollectionCancellation,
        execution_enabled: bool,
        committed: &Reporter<'_>,
    ) -> Result<()> {
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
        let basis = self.revoke_on_current_error(
            existing.as_ref(),
            IssueDecisionBasis::from_assessment(&assessment, IssueAutomationGate::Start, None)
                .map_err(anyhow::Error::from),
        )?;
        if !ready_for_propose(&assessment)
            || decision.definition().basis != basis
            || !human_start(decision)
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
            None => self.build_work(&item, decision)?,
        };
        // 실행 baseline을 읽을 수 없어도 승인한 작업 등록은 보존한다. 허가만 보류한다.
        let execution = execution.ok();
        let source = execution.as_ref().and_then(|snapshot| {
            snapshot
                .frozen_source
                .clone()
                .or(snapshot.saved_source.clone())
                .or_else(|| work.permit_source.clone())
                .or(snapshot.dispatch_source.clone())
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
            .max(basis.gate_revision);
        let gate_revision = if prior_source
            .zip(source.as_deref())
            .is_some_and(|(old, source)| old != source)
        {
            prior_revision
                .checked_add(1)
                .filter(|revision| *revision <= i64::MAX as u64)
                .ok_or_else(|| anyhow!("the work permit revision is exhausted"))?
        } else {
            prior_revision
        };
        cancel.commit_if_active(|| {
            // intent가 실행기 등록보다 먼저 durable해진다. 응답 유실 뒤 같은 ID로 재등록한다.
            let IssueWorkSaveResult::Saved { record, .. } =
                self.store
                    .save_issue_work(&self.workspace, fence, &assessment, &work)?
            else {
                return Ok(());
            };
            work = *record;
            let task_ids = match self.execution.submit(&self.workspace, &work.order) {
                Ok(task_ids) => task_ids,
                Err(_) => {
                    work.blocked_reason = Some(IssueWorkBlockReason::SubmissionUnavailable);
                    let (fence, _, assessment) = self.current_or_revoke(&work, committed)?;
                    self.store
                        .save_issue_work(&self.workspace, fence, &assessment, &work)?;
                    bail!("the issue work submission was interrupted");
                }
            };
            work.task_ids = task_ids;
            work.blocked_reason = if source.is_none() {
                Some(IssueWorkBlockReason::SourceUnavailable)
            } else if execution_enabled {
                Some(IssueWorkBlockReason::AwaitingSource)
            } else {
                Some(IssueWorkBlockReason::ExecutionDisabled)
            };
            let (fence, current_item, current) = self.current_or_revoke(&work, committed)?;
            // 로컬 정책과 표시도 등록 후 다시 확인한다. 판이 바뀌면 task는 permit 없이 남는다.
            if !matches_approval(&current_item, &current, &basis)? {
                self.execution
                    .revoke(&self.workspace, &work.order.work_order_id)?;
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
            let Some(source) = source else {
                return Ok(());
            };
            self.execution.grant(
                &self.workspace,
                &IssueWorkPermitRequest {
                    work_order_id: work.order.work_order_id.clone(),
                    approval: basis.clone(),
                    source_sha: source.clone(),
                    gate_revision,
                    delivery,
                },
            )?;
            let (fence, current_item, current) = self.current_or_revoke(&work, committed)?;
            if !matches_approval(&current_item, &current, &basis)? {
                self.execution
                    .revoke(&self.workspace, &work.order.work_order_id)?;
                work.blocked_reason = Some(IssueWorkBlockReason::ApprovalChanged);
                self.store
                    .save_issue_work(&self.workspace, fence, &current, &work)?;
                return Ok(());
            }
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
    ) -> Result<(IssueTriageFence, IssueCollectionItem, IssueTriageAssessment)> {
        self.revoke_on_current_error(Some(work), self.current(&work.order.issue, committed))
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
                    && let Err(revoke_error) = self
                        .execution
                        .revoke(&self.workspace, &work.order.work_order_id)
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
        committed: &Reporter<'_>,
    ) -> Result<()> {
        let mut candidate = work.clone();
        candidate.blocked_reason = Some(reason);
        let (fence, _, assessment) = self.current_or_revoke(work, committed)?;
        cancel.commit_if_active(|| {
            self.store
                .save_issue_work(&self.workspace, fence, &assessment, &candidate)?;
            Ok(())
        })?;
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
        let result = cancel.commit_if_active(|| {
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
        decision: &IssueDecisionRequest,
    ) -> Result<IssueWorkCheckpoint> {
        let Some(IssueSourceRead::Observed(observation)) = &item.current else {
            bail!("the current issue is unknown")
        };
        let IssueSourceContent::Shared(snapshot) = &observation.content else {
            bail!("the current issue handoff is withheld")
        };
        if decision.definition().basis.field_hash.as_ref() != Some(&snapshot.basis.field_hash) {
            bail!("the handoff is not the approved issue fields");
        }
        let acceptance = issue_acceptance_criteria(&snapshot.shared_copy.body)
            .map_err(|reason| anyhow!(reason))?;
        Ok(IssueWorkCheckpoint {
            order: IssueWorkOrder {
                work_order_id: issue_work_order_id(&item.issue),
                issue: item.issue.clone(),
                field_hash: snapshot.basis.field_hash.clone(),
                title: snapshot.shared_copy.title.clone(),
                handoff: snapshot.shared_copy.clone(),
                acceptance,
                scope: self.workspace.clone(),
                branch_hint: None,
                link_reference: observation.web.as_ref().map(|web| web.url.clone()),
                profile: None,
                priority: 80,
                attempt: 1,
            },
            decision_id: decision.id().clone(),
            approved_basis: decision.definition().basis.clone(),
            task_ids: Vec::new(),
            permit_source: None,
            permit_revision: 0,
            delivery_granted: false,
            blocked_reason: None,
            revision: 0,
        })
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
    })
}

fn matches_approval(
    item: &IssueCollectionItem,
    assessment: &IssueTriageAssessment,
    basis: &IssueDecisionBasis,
) -> Result<bool> {
    Ok(ready_for_propose(assessment)
        && IssueDecisionBasis::from_assessment(assessment, IssueAutomationGate::Start, None)?
            == *basis
        && item.tracked.as_ref().is_some_and(|tracked| {
            !tracked.flags.held && !tracked.flags.stale && !tracked.flags.unknown
        }))
}

#[cfg(test)]
mod tests;

//! 저장된 승인만 소비하는 propose 조정기다. 모델 호출 없이 host tick에서 진행한다.
use super::issue_collection::{IssueCollectionCancellation, checked_resync_read};
use super::issue_decision::IssueDecisionService;
use super::issue_policy::{IssuePolicyService, issue_policy_version};
use super::issue_triage::IssueTriageService;
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
    Decision {
        fact: IssueDecisionFact,
        revision: u64,
    },
    WorkSubmitted {
        issue: IssueReference,
        work_order_id: String,
        task_ids: Vec<String>,
        revision: u64,
    },
    WorkPermitGranted {
        issue: IssueReference,
        work_order_id: String,
        source_sha: String,
        gate_revision: u64,
        delivery: bool,
        revision: u64,
    },
}
type Reporter = dyn Fn(Vec<IssueCoordinatorFact>) + Send + Sync;

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
        committed: &Reporter,
    ) -> Result<()> {
        let Ok(_guard) = self.tick_gate.try_lock() else {
            return Ok(());
        };
        let mut after = None;
        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let batch = self
                .store
                .read_triage_batch(&self.workspace, after.as_ref(), 100)?;
            if !batch
                .collection
                .as_ref()
                .is_some_and(|state| state.config.mode == IssueCollectionMode::Observe)
            {
                return Ok(());
            }
            for item in batch.items {
                if cancel.is_cancelled() {
                    return Ok(());
                }
                self.reconcile(item, cancel, now_unix_ms, execution_enabled, committed)?;
            }
            after = batch.next_after;
            if after.is_none() {
                return Ok(());
            }
        }
    }

    fn reconcile(
        &self,
        item: IssueCollectionItem,
        cancel: &IssueCollectionCancellation,
        now_unix_ms: u64,
        execution_enabled: bool,
        committed: &Reporter,
    ) -> Result<()> {
        let Some(tracked) = item.tracked.as_ref() else {
            return Ok(());
        };
        let work = self.store.get_issue_work(&self.workspace, &item.issue)?;
        let assessment = self.triage.assess_current(&item)?;
        let basis =
            IssueDecisionBasis::from_assessment(&assessment, IssueAutomationGate::Start, None)?;
        let decision_id = work
            .as_ref()
            .map(|work| work.decision_id.clone())
            .unwrap_or_else(|| start_decision_id(&item.issue, &basis));
        let current_decision = self
            .store
            .get_issue_decision(&self.workspace, &decision_id)?;
        let decision = current_decision.as_ref().or_else(|| {
            item.open_decisions.iter().find(|decision| {
                decision.request.definition().kind == IssueDecisionKind::StartApproval
            })
        });
        let execution = if work.is_some() {
            self.execution
                .snapshot(&self.workspace, &issue_work_order_id(&item.issue))?
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
                    let (fence, current, assessment) = self.current(&item.issue)?;
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
                        {
                            if commit.changed {
                                committed(vec![IssueCoordinatorFact::Decision {
                                    fact: commit.record.request.fact(),
                                    revision: commit.catalog_revision,
                                }]);
                            }
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
                        let (fence, _, assessment) = self.current(&item.issue)?;
                        self.store
                            .save_issue_work(&self.workspace, fence, &assessment, &work)?;
                        Ok(())
                    })?;
                }
            }
            IssueCoordinatorAction::None => {
                if let Some(mut work) = work {
                    let reason = if !execution_enabled {
                        Some(IssueWorkBlockReason::ParallelDisabled)
                    } else if !work.delivery_granted {
                        Some(IssueWorkBlockReason::AwaitingSource)
                    } else {
                        None
                    };
                    if work.blocked_reason != reason {
                        work.blocked_reason = reason;
                        let (fence, _, assessment) = self.current(&item.issue)?;
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
                    self.ready(&item.issue)?;
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
        committed: &Reporter,
    ) -> Result<()> {
        // 승인 직후에도 반드시 원격을 다시 읽는다. 취소 중에는 원격을 취소할 수 없어도 저장/제출은 하지 않는다.
        if !self.fresh_resync(issue, cancel)? {
            return Ok(());
        }
        let (fence, item, assessment) = self.current(issue)?;
        let basis =
            IssueDecisionBasis::from_assessment(&assessment, IssueAutomationGate::Start, None)?;
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
            }
            return Ok(());
        }
        let mut work = match existing {
            Some(work) => work,
            None => self.build_work(&item, decision)?,
        };
        let execution = self
            .execution
            .snapshot(&self.workspace, &work.order.work_order_id)?;
        let source = execution
            .frozen_source
            .clone()
            .or(execution.saved_source.clone())
            .or_else(|| work.permit_source.clone())
            .or(execution.dispatch_source.clone())
            .ok_or_else(|| anyhow!("the work source commit is unavailable"))?;
        let delivery = execution.frozen_source.is_some();
        // source 변경만 더 높은 판을 허용한다. revoke된 동일 source를 새 판으로 재승인하지 않는다.
        let prior_source = execution
            .saved_source
            .as_deref()
            .or(work.permit_source.as_deref());
        let prior_revision = execution
            .saved_gate_revision
            .max(work.permit_revision)
            .max(basis.gate_revision);
        let gate_revision = if prior_source.is_some_and(|old| old != source) {
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
            let task_ids = self.execution.submit(&self.workspace, &work.order)?;
            let newly_submitted = work.task_ids.is_empty();
            work.task_ids = task_ids;
            work.blocked_reason = if execution_enabled {
                Some(IssueWorkBlockReason::AwaitingSource)
            } else {
                Some(IssueWorkBlockReason::ParallelDisabled)
            };
            let (fence, _, current) = self.current(issue)?;
            // 로컬 정책도 등록 후 다시 확인한다. 판이 바뀌면 task는 permit 없이 queued로 남는다.
            if IssueDecisionBasis::from_assessment(&current, IssueAutomationGate::Start, None)?
                != basis
                || !ready_for_propose(&current)
            {
                self.execution
                    .revoke(&self.workspace, &work.order.work_order_id)?;
                return Ok(());
            }
            let IssueWorkSaveResult::Saved {
                record,
                catalog_revision,
                ..
            } = self
                .store
                .save_issue_work(&self.workspace, fence, &current, &work)?
            else {
                return Ok(());
            };
            work = *record;
            if newly_submitted {
                committed(vec![IssueCoordinatorFact::WorkSubmitted {
                    issue: issue.clone(),
                    work_order_id: work.order.work_order_id.clone(),
                    task_ids: work.task_ids.clone(),
                    revision: catalog_revision,
                }]);
            }
            self.execution.grant(
                &self.workspace,
                &IssueWorkPermitRequest {
                    work_order_id: work.order.work_order_id.clone(),
                    approval: basis,
                    source_sha: source.clone(),
                    gate_revision,
                    delivery,
                },
            )?;
            let changed = work.permit_source.as_deref() != Some(source.as_str())
                || work.delivery_granted != delivery;
            work.permit_source = Some(source.clone());
            work.permit_revision = gate_revision;
            work.delivery_granted = delivery;
            if delivery && execution_enabled {
                work.blocked_reason = None;
            }
            let (fence, _, current) = self.current(issue)?;
            if let IssueWorkSaveResult::Saved {
                catalog_revision, ..
            } = self
                .store
                .save_issue_work(&self.workspace, fence, &current, &work)?
            {
                if changed {
                    committed(vec![IssueCoordinatorFact::WorkPermitGranted {
                        issue: issue.clone(),
                        work_order_id: work.order.work_order_id.clone(),
                        source_sha: source,
                        gate_revision,
                        delivery,
                        revision: catalog_revision,
                    }]);
                }
            }
            self.ready(issue)?;
            self.transition(issue, IssueTransition::Queue)?;
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
                    if let IssueTriageSaveResult::Saved { record, .. } =
                        self.store
                            .save_triage(&self.workspace, fence, &assessment)?
                    {
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
                priority: 100,
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

#[cfg(test)]
mod tests;

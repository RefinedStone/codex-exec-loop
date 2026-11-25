//! 판이 고정된 결정 요청을 영속화한다. 답은 기록만 하며 실행 권한을 발급하지 않는다.
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};

use crate::application::port::inbound::stored_command_port::StoredCommandContext;
use crate::application::port::inbound::stored_command_port::issue_confirmation::{
    IssueConfirmationDisplay, IssueConfirmationError, IssueConfirmationPreview,
    IssueConfirmationTarget,
};
use crate::application::port::outbound::issue_automation_store_port::{
    IssueAutomationStorePort, IssueDecisionCommit, IssueDecisionFence, IssueDecisionSaveResult,
    IssueTriageFence,
};
use crate::application::port::outbound::issue_work_execution_port::IssueWorkExecutionPort;
use crate::domain::issue_automation::collection::IssueCollectionItem;
use crate::domain::issue_automation::decision::{
    IssueDecisionAgentAnswer, IssueDecisionBasis, IssueDecisionDefinition, IssueDecisionError,
    IssueDecisionId, IssueDecisionOutcome, IssueDecisionRequest, IssueDecisionSharedText,
    IssueDecisionState, MAX_ISSUE_DECISION_TEXT_BYTES, StoredIssueDecision,
};
use crate::domain::issue_masking::{IssueTextField, plan_issue_masking};
use crate::domain::issue_policy::{IssueDisclosureTarget, IssuePolicy};
use crate::domain::issue_secret_spans::secret_spans;
use crate::domain::issue_work::IssueReference;

use super::issue_policy::IssuePolicyService;
use super::issue_triage::IssueTriageService;

const DECISION_CAS_ATTEMPTS: usize = 3;

mod ask;
mod deadlines;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDecisionAnswerResult {
    pub commit: IssueDecisionCommit,
    pub outcome: IssueDecisionOutcome,
}

pub struct IssueDecisionService {
    workspace_dir: String,
    store: Arc<dyn IssueAutomationStorePort>,
    triage: Arc<IssueTriageService>,
    policy: IssuePolicyService,
    /// 에이전트 질문(`ask`)만 쓴다. 실행 중인 thread 를 확인하고 그 task 를 기다리게 한다.
    execution: Option<Arc<dyn IssueWorkExecutionPort>>,
}

impl IssueDecisionService {
    pub fn new(
        workspace_dir: String,
        store: Arc<dyn IssueAutomationStorePort>,
        triage: Arc<IssueTriageService>,
        policy: IssuePolicyService,
    ) -> Self {
        Self {
            workspace_dir,
            store,
            triage,
            policy,
            execution: None,
        }
    }

    pub fn with_execution(mut self, execution: Arc<dyn IssueWorkExecutionPort>) -> Self {
        self.execution = Some(execution);
        self
    }

    /// 한 장부 snapshot의 요청과 판을 준비한다. 새 요청이나 확인 증거를 만들지 않는다.
    pub fn prepare_confirmation(&self, id: &str, option: &str) -> Result<IssueConfirmationPreview> {
        let id =
            IssueDecisionId::new(id).map_err(|_| anyhow!("the decision identifier is invalid"))?;
        reject_secret_identifier(id.as_str())?;
        reject_secret_identifier(option)?;
        let snapshot = self
            .store
            .get_issue_decision_commit(&self.workspace_dir, &id)
            .map_err(ledger_unavailable)?
            .ok_or_else(|| anyhow!("the decision request was not found"))?;
        let request = &snapshot.record.request;
        if request.state() != IssueDecisionState::Open {
            bail!("the decision request is already closed")
        }
        let selected = request
            .definition()
            .options
            .iter()
            .find(|candidate| candidate.id == option)
            .ok_or_else(|| anyhow!("the decision option is invalid"))?;
        let policy = self
            .policy
            .resolve(&self.workspace_dir)
            .map_err(|_| policy_unavailable())?;
        let question = shared_text(request.definition().question.as_str(), &policy)?;
        let label = shared_text(selected.label.as_str(), &policy)?;
        let effect = shared_text(selected.effect.as_str(), &policy)?;
        let mut detail = format!("선택: {}\n효과: {}", label.as_str(), effect.as_str());
        if let Some(review) = delivery_confirmation_detail(request.definition()) {
            detail.push_str("\n\n");
            detail.push_str(&review);
        }
        Ok(IssueConfirmationPreview {
            target: confirmation_target(&self.workspace_dir, &snapshot.record, option),
            display: IssueConfirmationDisplay {
                message: question.as_str().to_owned(),
                detail,
                confirm_label: label.as_str().to_owned(),
            },
        })
    }

    /// 요청 생성자는 저장된 triage의 판을 사용한다. 수집이나 자동 요청 생성은 하지 않는다.
    /// 호출자는 changed인 확정 commit으로만 requested 이벤트/기록을 보낸다.
    pub fn ensure_request(
        &self,
        id: IssueDecisionId,
        definition: IssueDecisionDefinition,
    ) -> Result<IssueDecisionCommit> {
        self.ensure_request_with_time(id, definition, None)
    }

    /// 새 요청의 기한만 최초 저장 시 고정한다. 기존 요청의 기한과 판은 바꾸지 않는다.
    pub fn ensure_request_at(
        &self,
        id: IssueDecisionId,
        definition: IssueDecisionDefinition,
        now_unix_ms: u64,
    ) -> Result<IssueDecisionCommit> {
        self.ensure_request_with_time(id, definition, Some(now_unix_ms))
    }

    fn ensure_request_with_time(
        &self,
        id: IssueDecisionId,
        mut definition: IssueDecisionDefinition,
        now_unix_ms: Option<u64>,
    ) -> Result<IssueDecisionCommit> {
        if definition.subject.issue().project_scope.workspace_dir != self.workspace_dir {
            bail!("the decision belongs to a different workspace")
        }
        self.validate_work_subject(&definition)?;
        if !self.work_context_current(&definition)? {
            bail!("the work decision no longer matches its execution context")
        }
        let policy = self
            .policy
            .resolve(&self.workspace_dir)
            .map_err(|_| policy_unavailable())?;
        sanitize_definition(&mut definition, &policy)?;
        reject_secret_identifier(id.as_str())?;
        let request = IssueDecisionRequest::new(id, definition)
            .map_err(|_| anyhow!("the decision request is invalid"))?;
        for _ in 0..DECISION_CAS_ATTEMPTS {
            let (fence, item) = self.current_item(request.issue())?;
            let current = self.triage.assess_current(&item)?;
            if item
                .triage
                .as_ref()
                .is_none_or(|stored| stored.assessment != current)
            {
                bail!("the decision requires a current persisted issue assessment")
            }
            let saved = if let Some(now_unix_ms) = now_unix_ms {
                let timeouts = self
                    .triage
                    .decision_timeouts_for_policy(&current.basis().automation_policy_version)?;
                self.store.ensure_issue_decision_timed(
                    &self.workspace_dir,
                    fence,
                    &current,
                    &request,
                    now_unix_ms,
                    &timeouts,
                )
            } else {
                self.store
                    .ensure_issue_decision(&self.workspace_dir, fence, &current, &request)
            }
            .map_err(ledger_unavailable)?;
            match saved {
                IssueDecisionSaveResult::Saved(commit) => return Ok(*commit),
                IssueDecisionSaveResult::Conflict => continue,
            }
        }
        bail!("the decision basis changed concurrently")
    }

    /// caller는 표시용이다. 비직렬화된 확인 capability의 대상을 검증한 답만 human이다.
    pub fn answer(
        &self,
        decision_id: &str,
        option: &str,
        note: Option<&str>,
        context: &StoredCommandContext,
    ) -> Result<IssueDecisionAnswerResult> {
        let id = IssueDecisionId::new(decision_id)
            .map_err(|_| anyhow!("the decision identifier is invalid"))?;
        reject_secret_identifier(id.as_str())?;
        for _ in 0..DECISION_CAS_ATTEMPTS {
            let snapshot = self
                .store
                .get_issue_decision_commit(&self.workspace_dir, &id)
                .map_err(ledger_unavailable)?
                .ok_or_else(|| anyhow!("the decision request was not found"))?;
            if snapshot.record.request.state() != IssueDecisionState::Open {
                return Ok(IssueDecisionAnswerResult {
                    commit: snapshot,
                    outcome: IssueDecisionOutcome::AlreadyResolved,
                });
            }
            reject_secret_identifier(option)?;
            let stored = snapshot.record;
            let asked =
                crate::domain::issue_automation::agent_question::IssueAgentAsk::from_definition(
                    stored.request.definition(),
                );
            if asked.is_some()
                && context.caller.as_deref() == Some(stored.request.definition().asked_by.as_str())
            {
                return Err(ask::own_question_rejection());
            }
            let (fence, item) = self.current_item(stored.request.issue())?;
            let current = self.triage.assess_current(&item)?;
            let mut basis = IssueDecisionBasis::from_assessment(
                &current,
                stored.request.definition().gate,
                stored.request.definition().basis.work_order_id.clone(),
            )
            .map_err(|_| anyhow!("the current decision basis is invalid"))?;
            basis.authorization_epoch = self
                .store
                .issue_authorization_epoch(&self.workspace_dir, stored.request.issue())?;
            if stored.request.definition().is_delivery_risk_exception()
                && let Some(work) = self
                    .store
                    .get_issue_work(&self.workspace_dir, stored.request.issue())?
                && let Some(delivery) = &work.delivery_assessment
                && delivery.matches_work(&work)
                && delivery.matches_triage(&current, basis.authorization_epoch)
            {
                basis = delivery.basis.clone();
            }
            self.validate_work_subject(stored.request.definition())?;
            if !self.work_context_current(stored.request.definition())? {
                let mut obsolete = stored.request.to_record();
                obsolete.state = IssueDecisionState::Superseded;
                let obsolete = IssueDecisionRequest::hydrate(obsolete)?;
                let saved = self
                    .store
                    .save_issue_decision(
                        &self.workspace_dir,
                        IssueDecisionFence {
                            triage: fence,
                            decision_revision: stored.revision,
                        },
                        &current,
                        &obsolete,
                    )
                    .map_err(ledger_unavailable)?;
                if let IssueDecisionSaveResult::Saved(commit) = saved {
                    return Ok(IssueDecisionAnswerResult {
                        commit: *commit,
                        outcome: IssueDecisionOutcome::Superseded,
                    });
                }
                continue;
            }
            let confirmation = context.human_confirmation.as_ref();
            if let Some(confirmation) = confirmation {
                confirmation.require_target(&confirmation_target(
                    &self.workspace_dir,
                    &stored,
                    option,
                ))?;
            }
            let mut gate_action = current
                .gates()
                .iter()
                .find(|gate| gate.gate == stored.request.definition().gate)
                .ok_or_else(|| anyhow!("the current decision gate is unavailable"))?
                .action;
            let policy = self
                .policy
                .resolve(&self.workspace_dir)
                .map_err(|_| policy_unavailable())?;
            let is_worker_question = stored.request.definition().basis.work_order_id.is_some()
                && stored.request.definition().kind
                    == crate::domain::issue_automation::decision::IssueDecisionKind::AgentQuestion;
            let is_work_recovery = stored.request.definition().basis.work_order_id.is_some()
                && stored.request.definition().kind
                    == crate::domain::issue_automation::decision::IssueDecisionKind::FailureRecovery;
            let is_close_approval = stored.request.definition().kind
                == crate::domain::issue_automation::decision::IssueDecisionKind::CloseApproval;
            if is_worker_question
                || is_work_recovery
                || is_close_approval
                || stored.request.definition().is_delivery_risk_exception()
            {
                gate_action =
                    crate::domain::issue_automation::policy::IssueAutomationGateAction::Human;
            }
            let shared_note = note.map(|note| shared_text(note, &policy)).transpose()?;
            if is_worker_question
                && asked.is_none()
                && option == crate::domain::issue_automation::agent_question::WORKER_REPLY_OPTION
                && shared_note.is_none()
            {
                bail!("the worker question requires a shared answer note")
            }
            let caller = context
                .caller
                .as_deref()
                .map(|caller| {
                    shared_text(caller, &policy).map(|shared| shared.as_str().to_string())
                })
                .transpose()?;
            let resolution = stored
                .request
                .answer_for_gate(
                    &basis,
                    gate_action,
                    IssueDecisionAgentAnswer {
                        option: option.to_string(),
                        note: shared_note,
                        caller,
                        channel: context.channel.clone(),
                        at: context.at.clone(),
                    },
                    confirmation.map(|value| value.reference()),
                )
                .map_err(|error| match error {
                    IssueDecisionError::HumanConfirmationRequired => {
                        anyhow::Error::new(IssueConfirmationError::Required)
                    }
                    _ => anyhow!("the decision answer is invalid"),
                })?;
            match self
                .store
                .save_issue_decision(
                    &self.workspace_dir,
                    IssueDecisionFence {
                        triage: fence,
                        decision_revision: stored.revision,
                    },
                    &current,
                    &resolution.request,
                )
                .map_err(ledger_unavailable)?
            {
                IssueDecisionSaveResult::Saved(commit) => {
                    let outcome = if commit.changed {
                        resolution.outcome
                    } else {
                        IssueDecisionOutcome::AlreadyResolved
                    };
                    return Ok(IssueDecisionAnswerResult {
                        commit: *commit,
                        outcome,
                    });
                }
                IssueDecisionSaveResult::Conflict => continue,
            }
        }
        bail!("the decision changed concurrently")
    }

    fn validate_work_subject(&self, definition: &IssueDecisionDefinition) -> Result<()> {
        use crate::domain::issue_automation::decision::IssueDecisionKind;
        let Some(id) = &definition.basis.work_order_id else {
            return Ok(());
        };
        let work = self
            .store
            .get_issue_work(&self.workspace_dir, definition.subject.issue())?
            .ok_or_else(|| anyhow!("the work decision has no registered work"))?;
        if &work.order.work_order_id != id
            || !(matches!(
                definition.kind,
                IssueDecisionKind::AgentQuestion
                    | IssueDecisionKind::FailureRecovery
                    | IssueDecisionKind::CloseApproval
            ) || definition.is_delivery_risk_exception())
        {
            bail!("the decision does not belong to a supported work context");
        }
        Ok(())
    }

    fn work_context_current(&self, definition: &IssueDecisionDefinition) -> Result<bool> {
        use crate::domain::issue_automation::decision::IssueDecisionKind;
        let Some(id) = &definition.basis.work_order_id else {
            return Ok(true);
        };
        let Some(work) = self
            .store
            .get_issue_work(&self.workspace_dir, definition.subject.issue())?
        else {
            return Ok(false);
        };
        if &work.order.work_order_id != id {
            return Ok(false);
        }
        if definition.is_delivery_risk_exception() {
            return Ok(work.delivery_assessment.as_ref().is_some_and(|delivery| {
                delivery.matches_work(&work) && delivery.matches_decision(definition)
            }));
        }
        let Some(record) = self
            .store
            .get_issue_execution(&self.workspace_dir, definition.subject.issue())?
        else {
            return Ok(false);
        };
        if &record.work_order_id != id {
            return Ok(false);
        }
        Ok(match definition.kind {
            IssueDecisionKind::AgentQuestion => {
                crate::domain::issue_automation::agent_question::agent_question_context_matches(
                    definition, &work, &record,
                )
            }
            IssueDecisionKind::FailureRecovery => {
                record.progress.terminal.as_ref().is_some_and(|terminal| {
                    definition.evidence == [format!("terminal:{}", terminal.record_id)]
                })
            }
            IssueDecisionKind::CloseApproval => {
                crate::domain::issue_automation::closure::close_work_context_matches(
                    definition, &work, &record,
                )
            }
            _ => false,
        })
    }

    fn current_item(
        &self,
        reference: &IssueReference,
    ) -> Result<(IssueTriageFence, IssueCollectionItem)> {
        if reference.project_scope.workspace_dir != self.workspace_dir {
            bail!("the decision belongs to a different workspace")
        }
        let mut after = None;
        loop {
            let batch = self
                .store
                .read_triage_batch(&self.workspace_dir, after.as_ref(), 100)
                .map_err(ledger_unavailable)?;
            let collection = batch
                .collection
                .ok_or_else(|| anyhow!("the issue collection is unavailable"))?;
            for item in batch.items {
                if item.issue == *reference {
                    let fence = IssueTriageFence {
                        binding_run_revision: item.binding_run_revision,
                        collection_epoch: collection.epoch,
                        observation_revision: item.observation_revision,
                        tracked_revision: item.tracked_revision,
                        assessment_revision: item.triage.as_ref().map(|triage| triage.revision),
                    };
                    return Ok((fence, item));
                }
            }
            match batch.next_after {
                Some(next) => after = Some(next),
                None => bail!("the decision subject is unavailable"),
            }
        }
    }
}

fn delivery_confirmation_detail(definition: &IssueDecisionDefinition) -> Option<String> {
    use crate::domain::issue_automation::delivery::IssueDeliveryRiskReason;

    if !definition.is_delivery_risk_exception() {
        return None;
    }
    let range = definition.basis.delivery.as_ref()?;
    let review = definition.delivery_review.as_ref()?;
    let lines = review
        .changed_lines
        .map(|lines| lines.to_string())
        .unwrap_or_else(|| "바이너리 변경을 포함하여 확정할 수 없음".into());
    let reasons = review
        .reasons
        .iter()
        .map(|reason| match reason {
            IssueDeliveryRiskReason::ProtectedPath => "보호 경로 변경",
            IssueDeliveryRiskReason::DiffTooLarge => "자동 전달 크기 상한 초과",
            IssueDeliveryRiskReason::BinaryDiff => "바이너리 변경",
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "기준 커밋: {}\n전달 커밋: {}\n변경 파일: {}\n변경 줄: {}\n검토 사유: {}",
        range.base_commit_sha, range.source_commit_sha, review.changed_files, lines, reasons,
    ))
}

fn confirmation_target(
    workspace: &str,
    stored: &StoredIssueDecision,
    option: &str,
) -> IssueConfirmationTarget {
    IssueConfirmationTarget::Decision {
        workspace: workspace.to_owned(),
        decision_id: stored.request.id().clone(),
        option: option.to_owned(),
        basis: stored.request.definition().basis.clone(),
        decision_revision: stored.revision,
    }
}

fn sanitize_definition(
    definition: &mut IssueDecisionDefinition,
    policy: &IssuePolicy,
) -> Result<()> {
    definition.question = shared_text(definition.question.as_str(), policy)?;
    for option in &mut definition.options {
        reject_secret_identifier(&option.id)?;
        option.label = shared_text(option.label.as_str(), policy)?;
        option.effect = shared_text(option.effect.as_str(), policy)?;
    }
    reject_secret_identifier(&definition.asked_by)?;
    for evidence in &definition.evidence {
        reject_secret_identifier(evidence)?;
    }
    Ok(())
}

pub(crate) fn shared_text(value: &str, policy: &IssuePolicy) -> Result<IssueDecisionSharedText> {
    if value.len() > MAX_ISSUE_DECISION_TEXT_BYTES {
        bail!("the decision text exceeds its allowed length")
    }
    let plan = plan_issue_masking(
        "",
        value,
        &secret_spans(IssueTextField::Body, value),
        policy.disclosure_for(IssueDisclosureTarget::AiHandoff),
    )
    .map_err(|_| anyhow!("the decision text could not pass its disclosure policy"))?;
    IssueDecisionSharedText::new_shared(plan.shared_copy.body)
        .map_err(|_| anyhow!("the shared decision text is invalid"))
}

fn reject_secret_identifier(value: &str) -> Result<()> {
    if !secret_spans(IssueTextField::Body, value).is_empty() {
        bail!("the decision metadata contains a sensitive value")
    }
    Ok(())
}

fn ledger_unavailable(_: anyhow::Error) -> anyhow::Error {
    anyhow!("the issue decision ledger is unavailable")
}

fn policy_unavailable() -> anyhow::Error {
    anyhow!("the issue decision disclosure policy is unavailable")
}

#[cfg(test)]
pub(crate) use tests::gateway_fixture::seed_human_decision_for_gateway;

#[cfg(test)]
mod tests;

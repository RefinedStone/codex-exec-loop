//! 같은 질문의 사람 답변은 최초 durable claim에서만 전달한다.
use anyhow::{Result, anyhow, bail};
use sha2::{Digest, Sha256};

use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkAnswerDelivery, IssueWorkAnswerReceipt, IssueWorkAnswerRequest,
};
use crate::application::service::issue_collection::IssueCollectionCancellation;
use crate::application::service::issue_decision::shared_text;
use crate::domain::issue_automation::IssueAutomationState;
use crate::domain::issue_automation::agent_question::{
    IssueAgentAsk, agent_question_reply, agent_question_targets,
};
use crate::domain::issue_automation::collection::IssueCollectionItem;
use crate::domain::issue_automation::coordinator::IssueWorkCheckpoint;
use crate::domain::issue_automation::decision::{
    IssueDecisionAnswerBy, IssueDecisionBasis, IssueDecisionDefinition, IssueDecisionId,
    IssueDecisionKind, IssueDecisionOption, IssueDecisionRequest, IssueDecisionSharedText,
    IssueDecisionState, IssueDecisionSubject, IssueDecisionTimeout,
};
use crate::domain::issue_automation::execution::{
    IssueExecutionQuestion, IssueExecutionRecord, IssueExecutionState,
};
use crate::domain::issue_automation::execution_answer::{
    IssueExecutionAnswerIntent, IssueExecutionAnswerRecord, IssueExecutionAnswerStatus,
    answer_command_id, answer_hash,
};
use crate::domain::issue_automation::policy::IssueAutomationGate;

use super::super::{IssueCoordinatorFact, IssueCoordinatorService, Reporter};

/// shortcut: the store returns at most 20 closed cards for one basis. An older answered agent
/// question then falls back to the awaiting_user card; page the history if that becomes common.
const ASKED_HISTORY: u16 = 20;

pub(super) fn reconcile(
    service: &IssueCoordinatorService,
    work: &IssueWorkCheckpoint,
    record: &IssueExecutionRecord,
    cancel: &IssueCollectionCancellation,
    now_unix_ms: u64,
    execution_enabled: bool,
    committed: &Reporter<'_>,
) -> Result<()> {
    if cancel.is_cancelled() {
        return Ok(());
    }
    let Some(question) = record.question.as_ref() else {
        return Ok(());
    };
    work.validate().map_err(|reason| anyhow!(reason))?;
    record.validate().map_err(|reason| anyhow!(reason))?;
    if record.work_order_id != work.order.work_order_id
        || !work.task_ids.contains(&question.task_id)
    {
        bail!("the worker question does not belong to its issue work");
    }
    if record.progress.requires_human_recovery()
        || !matches!(
            record.progress.state,
            IssueExecutionState::Running | IssueExecutionState::InReview
        )
    {
        return Ok(());
    }
    let (item, assessment) = super::current_assessment(service, &work.order.issue, committed)?;
    let Some(tracked) = &item.tracked else {
        return Ok(());
    };
    if matches!(
        tracked.state,
        IssueAutomationState::Done | IssueAutomationState::Cancelled | IssueAutomationState::Failed
    ) || tracked.flags.held
        || tracked.flags.stale
        || tracked.flags.unknown
    {
        return Ok(());
    }
    let control = service
        .execution
        .control(&service.workspace, &work.order.work_order_id)?;
    let epoch = service
        .store
        .issue_authorization_epoch(&service.workspace, &work.order.issue)?;
    if !control.allows_automation()
        || control.authorization_epoch != epoch
        || work.current_authorization_basis().authorization_epoch != epoch
    {
        return Ok(());
    }
    let mut basis = IssueDecisionBasis::from_assessment(
        &assessment,
        IssueAutomationGate::Spec,
        Some(work.order.work_order_id.clone()),
    )?;
    basis.authorization_epoch = epoch;
    if basis.field_hash.as_ref() != Some(&work.order.field_hash) {
        return Ok(());
    }
    // An agent question opened by ask_issue_decision owns this wait; no second card opens.
    let asked = asked_decision(service, work, question, &item)?;
    let request = match asked {
        Some(request) => request,
        None => {
            let id = question_decision_id(work, question, &basis)?;
            let definition = question_definition(work, question, basis)?;
            let Some(commit) = cancel.commit_if_active(|| {
                service
                    .decisions
                    .ensure_request_at(id.clone(), definition, now_unix_ms)
            })?
            else {
                return Ok(());
            };
            if commit.record.request.id() != &id {
                // The definition deduplicates on question_id; the generated ID additionally binds
                // thread, task and generation. An old human answer cannot approve changed coordinates.
                bail!("the stored decision belongs to another worker question identity");
            }
            if commit.changed {
                committed(vec![IssueCoordinatorFact::Decision {
                    fact: commit.record.request.fact(),
                    revision: commit.catalog_revision,
                }]);
            }
            commit.record.request
        }
    };
    if !execution_enabled
        || cancel.is_cancelled()
        || request.state() != IssueDecisionState::Answered
    {
        return Ok(());
    }
    let Some(answer) = request.answer().filter(|answer| {
        answer.by == IssueDecisionAnswerBy::Human && answer.confirmation.is_some()
    }) else {
        return Ok(());
    };
    let Some(message) = agent_question_reply(request.definition(), answer) else {
        return Ok(());
    };
    // A response to Spec does not renew the original Start authorization.
    let mut start_basis =
        IssueDecisionBasis::from_assessment(&assessment, IssueAutomationGate::Start, None)?;
    start_basis.authorization_epoch = epoch;
    if &start_basis != work.current_authorization_basis() {
        return Ok(());
    }
    let policy = service
        .policy
        .resolve(&service.workspace)
        .map_err(|_| anyhow!("the current answer disclosure policy is unavailable"))?;
    let shared = shared_text(&message, &policy)?;
    if shared.as_str() != message {
        // The stored human decision approved a different disclosed payload.
        bail!("the confirmed worker answer no longer matches its disclosure policy");
    }
    let intent = IssueExecutionAnswerIntent {
        command_id: answer_command_id(&work.order.work_order_id, request.id()),
        issue: work.order.issue.clone(),
        work_order_id: work.order.work_order_id.clone(),
        decision_id: request.id().clone(),
        question: question.clone(),
        authorization_epoch: epoch,
        answer_sha256: answer_hash(shared.as_str()),
    };
    let Some(claim) = cancel.commit_if_active(|| {
        service
            .store
            .claim_issue_execution_answer(&service.workspace, &intent)
    })?
    else {
        return Ok(());
    };
    if !claim.created || cancel.is_cancelled() {
        return Ok(());
    }
    // Control may have changed after the issue-ledger claim. The execution adapter also
    // atomically rechecks ownership and the old worker generation before it queues or starts.
    service
        .execution
        .control(&service.workspace, &work.order.work_order_id)?
        .require_epoch(epoch)?;
    let outgoing = IssueWorkAnswerRequest {
        authorization_epoch: epoch,
        work_order_id: work.order.work_order_id.clone(),
        command_id: intent.command_id.clone(),
        question: question.clone(),
        message: shared.as_str().to_owned(),
    };
    match service
        .execution
        .answer_question(&service.workspace, &outgoing)
    {
        Ok(receipt) => save_receipt(service, &claim.record, &receipt, cancel),
        Err(_) => {
            tracing::warn!(command_id = %intent.command_id,
                "worker answer delivery was not confirmed; preserving Unknown without retry");
            Ok(())
        }
    }
}

pub(super) fn poll_pending_answers(
    service: &IssueCoordinatorService,
    work: &IssueWorkCheckpoint,
    cancel: &IssueCollectionCancellation,
) -> Result<()> {
    if cancel.is_cancelled() {
        return Ok(());
    }
    let pending = service
        .store
        .pending_issue_execution_answers(&service.workspace, &work.order.work_order_id)?;
    let mut first_error = None;
    for record in pending {
        if cancel.is_cancelled() {
            break;
        }
        let result = (|| {
            record.validate().map_err(|reason| anyhow!(reason))?;
            if record.intent.issue != work.order.issue
                || record.intent.work_order_id != work.order.work_order_id
                || !record.status.pending()
            {
                bail!("the pending worker answer belongs to another work");
            }
            let Some(receipt) = service
                .execution
                .question_answer_receipt(
                    &service.workspace,
                    &work.order.work_order_id,
                    &record.intent.command_id,
                )
                .map_err(|_| anyhow!("the worker answer receipt is unavailable"))?
            else {
                return Ok(());
            };
            save_receipt(service, &record, &receipt, cancel)
        })();
        if let Err(error) = result {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn save_receipt(
    service: &IssueCoordinatorService,
    record: &IssueExecutionAnswerRecord,
    receipt: &IssueWorkAnswerReceipt,
    cancel: &IssueCollectionCancellation,
) -> Result<()> {
    if receipt.command_id != record.intent.command_id
        || receipt.question_id != record.intent.question.question_id
    {
        bail!("the worker answer receipt identity does not match its claim");
    }
    let status = match &receipt.delivery {
        IssueWorkAnswerDelivery::Queued => IssueExecutionAnswerStatus::Queued,
        IssueWorkAnswerDelivery::Unknown => IssueExecutionAnswerStatus::Unknown,
        IssueWorkAnswerDelivery::Accepted {
            thread_id,
            turn_id,
            lease_generation,
        } => IssueExecutionAnswerStatus::Accepted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            lease_generation: lease_generation.clone(),
        },
        IssueWorkAnswerDelivery::RejectedBeforeStart { reason } => {
            IssueExecutionAnswerStatus::RejectedBeforeStart {
                reason: reason.clone(),
            }
        }
    };
    status
        .validate_for(&record.intent)
        .map_err(|reason| anyhow!(reason))?;
    cancel.commit_if_active(|| {
        service.store.save_issue_execution_answer(
            &service.workspace,
            &record.intent.command_id,
            record.revision,
            &status,
        )
    })?;
    Ok(())
}

/// Open agent questions come from the same catalog snapshot; answered ones from recent history.
fn asked_decision(
    service: &IssueCoordinatorService,
    work: &IssueWorkCheckpoint,
    question: &IssueExecutionQuestion,
    item: &IssueCollectionItem,
) -> Result<Option<IssueDecisionRequest>> {
    let owned = |request: &IssueDecisionRequest| {
        let definition = request.definition();
        IssueAgentAsk::from_definition(definition).is_some()
            && definition.basis.work_order_id.as_deref() == Some(work.order.work_order_id.as_str())
            && agent_question_targets(definition, question)
    };
    if let Some(open) = item
        .open_decisions
        .iter()
        .map(|decision| &decision.request)
        .find(|request| owned(request))
    {
        return Ok(Some(open.clone()));
    }
    Ok(service
        .store
        .closed_issue_decisions(&service.workspace, &work.order.issue, ASKED_HISTORY)?
        .into_iter()
        .map(|decision| decision.request)
        .find(|request| request.state() == IssueDecisionState::Answered && owned(request)))
}

fn question_decision_id(
    work: &IssueWorkCheckpoint,
    question: &IssueExecutionQuestion,
    basis: &IssueDecisionBasis,
) -> Result<IssueDecisionId> {
    let identity = serde_json::to_vec(&(
        "issue-worker-question-v1",
        &work.order.work_order_id,
        question,
        basis,
    ))?;
    Ok(IssueDecisionId::new(format!(
        "issue-question-{:x}",
        Sha256::digest(identity)
    ))?)
}

fn question_definition(
    work: &IssueWorkCheckpoint,
    question: &IssueExecutionQuestion,
    basis: IssueDecisionBasis,
) -> Result<IssueDecisionDefinition> {
    let text = |value: &str| IssueDecisionSharedText::new_shared(value);
    Ok(IssueDecisionDefinition {
        delivery_review: None,
        subject: IssueDecisionSubject::Issue(work.order.issue.clone()),
        gate: IssueAutomationGate::Spec,
        kind: IssueDecisionKind::AgentQuestion,
        question: text("작업 대화에서 질문을 확인하고 답변을 입력해 주세요.")?,
        options: vec![IssueDecisionOption {
            id: "reply".into(),
            label: text("답변 전달")?,
            effect: text("같은 작업 대화에 이 답변을 한 번 전달합니다.")?,
        }],
        recommended: None,
        basis,
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: None,
        asked_by: "issue-coordinator".into(),
        evidence: vec![format!("question:{}", question.question_id)],
    })
}

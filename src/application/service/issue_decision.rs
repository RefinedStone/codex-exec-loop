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
        }
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
        Ok(IssueConfirmationPreview {
            target: confirmation_target(&self.workspace_dir, &snapshot.record, option),
            display: IssueConfirmationDisplay {
                message: question.as_str().to_owned(),
                detail: format!("선택: {}\n효과: {}", label.as_str(), effect.as_str()),
                confirm_label: label.as_str().to_owned(),
            },
        })
    }

    /// 요청 생성자는 저장된 triage의 판을 사용한다. 수집이나 자동 요청 생성은 하지 않는다.
    /// 호출자는 changed인 확정 commit으로만 requested 이벤트/기록을 보낸다.
    pub fn ensure_request(
        &self,
        id: IssueDecisionId,
        mut definition: IssueDecisionDefinition,
    ) -> Result<IssueDecisionCommit> {
        if definition.subject.issue().project_scope.workspace_dir != self.workspace_dir {
            bail!("the decision belongs to a different workspace")
        }
        if definition.basis.work_order_id.is_some() {
            bail!("work-order decisions are not available in this version")
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
            match self
                .store
                .ensure_issue_decision(&self.workspace_dir, fence, &current, &request)
                .map_err(ledger_unavailable)?
            {
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
            let (fence, item) = self.current_item(stored.request.issue())?;
            let current = self.triage.assess_current(&item)?;
            let mut basis = IssueDecisionBasis::from_assessment(
                &current,
                stored.request.definition().gate,
                None,
            )
            .map_err(|_| anyhow!("the current decision basis is invalid"))?;
            basis.authorization_epoch = self
                .store
                .issue_authorization_epoch(&self.workspace_dir, stored.request.issue())?;
            let confirmation = context.human_confirmation.as_ref();
            if let Some(confirmation) = confirmation {
                confirmation.require_target(&confirmation_target(
                    &self.workspace_dir,
                    &stored,
                    option,
                ))?;
            }
            let gate_action = current
                .gates()
                .iter()
                .find(|gate| gate.gate == stored.request.definition().gate)
                .ok_or_else(|| anyhow!("the current decision gate is unavailable"))?
                .action;
            let policy = self
                .policy
                .resolve(&self.workspace_dir)
                .map_err(|_| policy_unavailable())?;
            let shared_note = note.map(|note| shared_text(note, &policy)).transpose()?;
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

fn shared_text(value: &str, policy: &IssuePolicy) -> Result<IssueDecisionSharedText> {
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

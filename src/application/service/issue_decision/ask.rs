//! 실행 중인 에이전트가 자기 작업 지시의 이슈에 선택형 질문을 연다 (IA8-1, #2566).
//! 대상 이슈와 답할 thread 는 호출자의 branch 를 쥔 실행 lease 와 그 작업 지시에서 찾는다. 이슈 번호는 받지 않는다.
//! 질문을 열면 그 task 는 awaiting_user 가 된다. 턴이 끝나도 thread 가 남고, 사람의 답이 그 thread 를 잇는다.
use sha2::{Digest, Sha256};

use super::*;
use crate::application::port::inbound::stored_command_port::{
    AskIssueDecisionRequest, StoredCommandRejection,
};
use crate::domain::issue_automation::IssueAutomationState;
use crate::domain::issue_automation::agent_question::{
    AGENT_QUESTION_OPTION_EFFECT, IssueAgentAsk, MAX_OPEN_AGENT_QUESTIONS_PER_ISSUE,
};
use crate::domain::issue_automation::decision::{
    IssueDecisionKind, IssueDecisionOption, IssueDecisionSubject, IssueDecisionTimeout,
};
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::kernel_spec::ReasonCode;

impl IssueDecisionService {
    /// 같은 질문의 재전송은 같은 결정을 돌려준다. 다른 질문은 이슈의 열린 질문 한도 안에서만 연다.
    pub fn ask(
        &self,
        request: AskIssueDecisionRequest,
        context: &StoredCommandContext,
    ) -> Result<IssueDecisionCommit> {
        let execution = self
            .execution
            .as_ref()
            .ok_or_else(|| anyhow!("agent questions require the issue work executor"))?;
        // The asking connection is recorded so that it cannot answer its own question.
        let asked_by = context
            .caller
            .clone()
            .ok_or_else(|| anyhow!("an agent question requires a caller connection"))?;
        reject_secret_identifier(&request.branch)?;
        let (work_order_id, ask) = execution
            .asking_worker(&self.workspace_dir, &request.branch)?
            .ok_or_else(|| anyhow!("the branch has no running issue work conversation"))?;
        let work = self
            .store
            .list_issue_work(&self.workspace_dir)
            .map_err(ledger_unavailable)?
            .into_iter()
            .find(|work| {
                work.order.work_order_id == work_order_id && work.task_ids.contains(&ask.task_id)
            })
            .ok_or_else(|| anyhow!("the asking task does not belong to registered issue work"))?;
        let epoch = self
            .store
            .issue_authorization_epoch(&self.workspace_dir, &work.order.issue)?;
        let control = execution.control(&self.workspace_dir, &work_order_id)?;
        if !control.allows_automation()
            || control.authorization_epoch != epoch
            || work.current_authorization_basis().authorization_epoch != epoch
        {
            bail!("the issue work does not allow automatic questions now");
        }
        let (_, item) = self.current_item(&work.order.issue)?;
        if item.tracked.as_ref().is_none_or(|tracked| {
            matches!(
                tracked.state,
                IssueAutomationState::Done
                    | IssueAutomationState::Cancelled
                    | IssueAutomationState::Failed
            ) || tracked.flags.held
                || tracked.flags.stale
                || tracked.flags.unknown
        }) {
            bail!("the issue is not in a state that accepts agent questions");
        }
        let current = self.triage.assess_current(&item)?;
        let mut basis = IssueDecisionBasis::from_assessment(
            &current,
            IssueAutomationGate::Spec,
            Some(work_order_id.clone()),
        )
        .map_err(|_| anyhow!("the current question basis is invalid"))?;
        basis.authorization_epoch = epoch;
        if basis.field_hash.as_ref() != Some(&work.order.field_hash) {
            bail!("the issue changed after its work order; the question was not opened");
        }
        let definition =
            self.question_definition(request, work.order.issue.clone(), basis, asked_by, &ask)?;
        let identity = serde_json::to_vec(&("issue-agent-question-v1", &definition))?;
        let id = IssueDecisionId::new(format!("issue-ask-{:x}", Sha256::digest(identity)))?;
        if let Some(existing) = self
            .store
            .get_issue_decision_commit(&self.workspace_dir, &id)
            .map_err(ledger_unavailable)?
        {
            self.await_answer(execution.as_ref(), &work_order_id, &ask)?;
            return Ok(existing);
        }
        // shortcut: the count reads the same snapshot as the ask; two concurrent asks may both
        // pass. Stored commands run one at a time in the kernel, so this holds there.
        let open = item
            .open_decisions
            .iter()
            .filter(|decision| {
                IssueAgentAsk::from_definition(decision.request.definition()).is_some()
            })
            .count();
        if open >= MAX_OPEN_AGENT_QUESTIONS_PER_ISSUE {
            return Err(anyhow::Error::new(StoredCommandRejection {
                reason: ReasonCode::IssueQuestionLimit,
                message: format!(
                    "The issue already has {open} open agent question(s). Nothing was asked."
                ),
                planning_revision: None,
            }));
        }
        // Park the task first: a failed decision write then falls back to the awaiting_user card.
        self.await_answer(execution.as_ref(), &work_order_id, &ask)?;
        self.ensure_request(id, definition)
    }

    fn question_definition(
        &self,
        request: AskIssueDecisionRequest,
        issue: IssueReference,
        basis: IssueDecisionBasis,
        asked_by: String,
        ask: &IssueAgentAsk,
    ) -> Result<IssueDecisionDefinition> {
        let policy = self
            .policy
            .resolve(&self.workspace_dir)
            .map_err(|_| policy_unavailable())?;
        let effect = shared_text(AGENT_QUESTION_OPTION_EFFECT, &policy)?;
        let options = request
            .options
            .into_iter()
            .map(|option| {
                Ok(IssueDecisionOption {
                    label: shared_text(&option.label, &policy)?,
                    id: option.id,
                    effect: effect.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(IssueDecisionDefinition {
            delivery_review: None,
            subject: IssueDecisionSubject::Issue(issue),
            gate: IssueAutomationGate::Spec,
            kind: IssueDecisionKind::AgentQuestion,
            question: shared_text(&request.question, &policy)?,
            options,
            recommended: request.recommended,
            basis,
            on_timeout: IssueDecisionTimeout::Wait,
            due_at: None,
            asked_by,
            evidence: ask.evidence(),
        })
    }

    fn await_answer(
        &self,
        execution: &dyn IssueWorkExecutionPort,
        work_order_id: &str,
        ask: &IssueAgentAsk,
    ) -> Result<()> {
        if !execution.await_question(&self.workspace_dir, work_order_id, ask)? {
            bail!("the asking task changed before it could wait for the answer");
        }
        Ok(())
    }
}

pub(super) fn own_question_rejection() -> anyhow::Error {
    anyhow::Error::new(StoredCommandRejection {
        reason: ReasonCode::OwnIssueQuestion,
        message: ReasonCode::OwnIssueQuestion.summary().to_owned(),
        planning_revision: None,
    })
}

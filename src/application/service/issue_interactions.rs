//! Read-only decision history, intervention availability, and conversation targets.
use std::sync::Arc;

use anyhow::{Result, bail};

use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_work_execution_port::IssueWorkExecutionPort;
use crate::domain::issue_automation::collection::IssueCollectionItem;
use crate::domain::issue_automation::coordinator::issue_work_order_id;
use crate::domain::issue_automation::decision::{IssueDecisionBasis, IssueDecisionState};
use crate::domain::issue_automation::interaction::{
    IssueActionCommand, IssueActionProjection, IssueControlProjection, IssueConversationTarget,
    IssueDecisionHistory, IssueDecisionResolutionCode, IssueInteractionProjection,
};
use crate::domain::issue_automation::source::{IssueSourceContent, IssueSourceRead};
use crate::domain::issue_automation::work_control::IssueWorkControl;
use crate::domain::issue_work::IssueSharedCopy;

use super::issue_policy::{IssuePolicyService, issue_policy_version};
use super::issue_triage::IssueTriageService;

const HISTORY_LIMIT: u16 = 20;

pub struct IssueInteractionsService {
    workspace: String,
    store: Arc<dyn IssueAutomationStorePort>,
    execution: Arc<dyn IssueWorkExecutionPort>,
    triage: Arc<IssueTriageService>,
    policy: IssuePolicyService,
}

impl IssueInteractionsService {
    pub fn new(
        workspace: String,
        store: Arc<dyn IssueAutomationStorePort>,
        execution: Arc<dyn IssueWorkExecutionPort>,
        triage: Arc<IssueTriageService>,
        policy: IssuePolicyService,
    ) -> Self {
        Self {
            workspace,
            store,
            execution,
            triage,
            policy,
        }
    }

    /// This is a projection, never an authorization. Commands repeat their own current checks.
    pub fn project(&self, item: &IssueCollectionItem) -> Result<IssueInteractionProjection> {
        if item.issue.project_scope.workspace_dir != self.workspace {
            bail!("the interaction belongs to another workspace");
        }
        let order_id = issue_work_order_id(&item.issue);
        let control = self.execution.control(&self.workspace, &order_id).ok();
        let pending = self
            .store
            .pending_issue_interventions(&self.workspace)?
            .iter()
            .any(|pending| pending.request.issue == item.issue);
        let has_work = self
            .store
            .get_issue_work(&self.workspace, &item.issue)?
            .is_some_and(|work| !work.task_ids.is_empty());
        let shared = self.shared_copy(item);
        let actions = IssueActionCommand::ALL
            .into_iter()
            .map(|command| action(command, control.as_ref(), has_work, pending))
            .collect();
        let epoch = self
            .store
            .issue_authorization_epoch(&self.workspace, &item.issue)?;
        let assessment = self.triage.assess_current(item).ok();
        let mut history = Vec::new();
        for stored in
            self.store
                .closed_issue_decisions(&self.workspace, &item.issue, HISTORY_LIMIT)?
        {
            let request = &stored.request;
            let resolution = match request.state() {
                IssueDecisionState::Answered => IssueDecisionResolutionCode::Answered,
                IssueDecisionState::Expired => IssueDecisionResolutionCode::Expired,
                IssueDecisionState::Withdrawn => IssueDecisionResolutionCode::Withdrawn,
                IssueDecisionState::Superseded => IssueDecisionResolutionCode::BasisChanged,
                IssueDecisionState::Open => continue,
            };
            let current_basis = assessment.as_ref().and_then(|assessment| {
                let mut basis = IssueDecisionBasis::from_assessment(
                    assessment,
                    request.definition().gate,
                    request.definition().basis.work_order_id.clone(),
                )
                .ok()?;
                basis.authorization_epoch = epoch;
                Some(basis)
            });
            let may_show_text = shared.is_some()
                && !pending
                && current_basis.as_ref() == Some(&request.definition().basis);
            history.push(IssueDecisionHistory {
                decision: request.fact(),
                request: may_show_text.then_some(stored),
                resolution,
            });
        }
        let conversation = match shared {
            Some(shared_copy) => match self
                .execution
                .conversation_thread_id(&self.workspace, &order_id)
            {
                Ok(Some(thread_id)) if valid_thread_id(&thread_id) => {
                    IssueConversationTarget::Thread { thread_id }
                }
                Ok(None) => IssueConversationTarget::Draft { shared_copy },
                Ok(Some(_)) | Err(_) => IssueConversationTarget::Unavailable,
            },
            None => IssueConversationTarget::Unavailable,
        };
        Ok(IssueInteractionProjection {
            control: control.map(|control| IssueControlProjection {
                tracked: control.tracked,
                held: control.held,
                taken_over: control.taken_over,
                cancelled: control.cancelled,
                authorization_epoch: control.authorization_epoch,
            }),
            actions,
            decision_history: history,
            conversation,
        })
    }

    /// An old shared copy is not an authorization under changed disclosure rules.
    fn shared_copy(&self, item: &IssueCollectionItem) -> Option<IssueSharedCopy> {
        let policy = self.policy.resolve(&self.workspace).ok()?;
        let current_version = issue_policy_version(&policy);
        let Some(IssueSourceRead::Observed(observation)) = &item.current else {
            return None;
        };
        let IssueSourceContent::Shared(snapshot) = &observation.content else {
            return None;
        };
        (snapshot.policy_version == current_version).then(|| snapshot.shared_copy.clone())
    }
}

fn valid_thread_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value.trim() == value
        && !value
            .chars()
            .any(|value| value.is_control() || value.is_whitespace())
}

fn action(
    command: IssueActionCommand,
    control: Option<&IssueWorkControl>,
    has_work: bool,
    pending: bool,
) -> IssueActionProjection {
    let reason = match control {
        None => Some("작업 제어 상태를 확인할 수 없습니다."),
        Some(_) if pending => Some("이전 개입 명령을 복구하는 중입니다."),
        Some(control) => match command {
            IssueActionCommand::HoldIssue if control.held => Some("이미 보류 중입니다."),
            IssueActionCommand::ReleaseIssue if !control.held => Some("보류 중인 작업이 아닙니다."),
            IssueActionCommand::SteerIssue if !has_work => Some("아직 등록된 작업이 없습니다."),
            IssueActionCommand::SteerIssue
                if control.cancelled || control.taken_over || !control.tracked =>
            {
                Some("자동 실행이 소유한 작업이 아닙니다.")
            }
            IssueActionCommand::TakeOverIssue if control.taken_over => {
                Some("이미 직접 맡은 작업입니다.")
            }
            IssueActionCommand::CancelIssueWork if control.cancelled => {
                Some("이미 취소된 작업입니다.")
            }
            IssueActionCommand::TrackIssue if control.tracked => Some("이미 추적 중입니다."),
            IssueActionCommand::UntrackIssue if !control.tracked => {
                Some("이미 추적에서 제외했습니다.")
            }
            IssueActionCommand::HoldIssue | IssueActionCommand::RetriageIssue
                if control.cancelled || !control.tracked =>
            {
                Some("취소되었거나 추적에서 제외한 작업입니다.")
            }
            IssueActionCommand::TakeOverIssue if control.cancelled => Some("취소된 작업입니다."),
            _ => None,
        },
    };
    IssueActionProjection {
        command,
        enabled: reason.is_none(),
        reason: reason.map(str::to_owned),
    }
}

#[cfg(test)]
mod tests;

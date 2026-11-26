//! Telegram 입구의 이슈 결정 알림과 답의 조립이다 (IA8-3).
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::adapter::inbound::kernel_protocol::{CommandResponse, CommandResult, KernelCommand};
use crate::adapter::outbound::db::sqlite_issue_automation_store::SqliteIssueAutomationStoreAdapter;
use crate::adapter::outbound::db::sqlite_planning_authority_adapter::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::issue_automation_policy::FilesystemIssueAutomationPolicySourceAdapter;
use crate::application::port::inbound::kernel_command_port::KernelCommandPort;
use crate::application::port::inbound::telegram_issue_port::{
    TelegramIssueAnswerOutcome, TelegramIssueAnswerPort, TelegramIssueNotificationPort,
};
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::telegram_bot_port::TelegramBotPort;
use crate::application::port::outbound::telegram_issue_notification_port::TelegramIssueDecisionLookupPort;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::telegram_issue_notification::TelegramIssueNotificationService;
use crate::domain::issue_automation::decision::{
    IssueDecisionId, IssueDecisionOutcome, IssueDecisionState,
};

use super::kernel_command_execution::{self, CALLER_TELEGRAM};

/// 알림은 커널과 같은 대표 checkout 의 기록과 장부를 읽는다. 커널이 있을 수 없는 폴더에서는 None 이다.
pub(crate) fn build_notifications(
    workspace_dir: &str,
    bot: Arc<dyn TelegramBotPort>,
) -> Option<Arc<dyn TelegramIssueNotificationPort>> {
    let workspace = super::production::repository_kernel_target(workspace_dir)
        .ok()?
        .workspace_root;
    let store = StoredIssueDecisionLookup(SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    }));
    Some(Arc::new(TelegramIssueNotificationService::new(
        workspace,
        bot,
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
        super::production::kernel_event_log_port(),
        Arc::new(store),
        IssueAutomationPolicyService::new(Arc::new(
            FilesystemIssueAutomationPolicySourceAdapter::new(),
        )),
    )))
}

pub(super) struct StoredIssueDecisionLookup(pub(super) SqliteIssueAutomationStoreAdapter);

impl TelegramIssueDecisionLookupPort for StoredIssueDecisionLookup {
    fn open_decision_options(
        &self,
        workspace_dir: &str,
        decision_id: &str,
    ) -> Result<Option<Vec<String>>> {
        let Ok(id) = IssueDecisionId::new(decision_id) else {
            return Ok(None);
        };
        Ok(self
            .0
            .get_issue_decision(workspace_dir, &id)?
            .filter(|stored| stored.request.state() == IssueDecisionState::Open)
            .map(|stored| {
                stored
                    .request
                    .definition()
                    .options
                    .iter()
                    .map(|option| option.id.clone())
                    .collect()
            }))
    }
}

/// 답마다 저장소 커널의 명령 port 를 만든다. 실행 중인 Gateway 가 있으면 그 커널이 받는다.
pub(crate) struct KernelRoutedTelegramIssueAnswer {
    workspace_dir: String,
}

impl KernelRoutedTelegramIssueAnswer {
    pub(crate) fn new(workspace_dir: String) -> Self {
        Self { workspace_dir }
    }
}

impl TelegramIssueAnswerPort for KernelRoutedTelegramIssueAnswer {
    fn answer(&self, decision_id: &str, option: &str) -> Result<TelegramIssueAnswerOutcome> {
        let port =
            kernel_command_execution::build(Path::new(&self.workspace_dir), CALLER_TELEGRAM)?;
        answer_through_kernel(port.as_ref(), decision_id, option)
    }
}

/// 사람 확인 (confirmation) 없이 보낸다. 그래서 커널은 이 답을 `by: agent` 로 기록한다.
pub(super) fn answer_through_kernel(
    port: &dyn KernelCommandPort,
    decision_id: &str,
    option: &str,
) -> Result<TelegramIssueAnswerOutcome> {
    let command = KernelCommand::AnswerIssueDecision {
        criteria: None,
        decision_id: decision_id.to_string(),
        option: option.to_string(),
        note: None,
    };
    let answer: CommandResponse = serde_json::from_value(port.command(
        command.kind(),
        serde_json::to_value(&command)?,
        None,
    )?)
    .context("the kernel answered the decision with an unknown shape")?;
    if let Some(rejection) = answer.rejection {
        return Ok(TelegramIssueAnswerOutcome::Rejected(rejection.reason));
    }
    Ok(match answer.result {
        Some(CommandResult::IssueDecisionResolved { outcome, .. }) => match outcome {
            IssueDecisionOutcome::Answered => TelegramIssueAnswerOutcome::Answered,
            IssueDecisionOutcome::Superseded => TelegramIssueAnswerOutcome::Superseded,
            IssueDecisionOutcome::AlreadyResolved => TelegramIssueAnswerOutcome::AlreadyResolved,
        },
        _ => TelegramIssueAnswerOutcome::Failed,
    })
}

#[cfg(test)]
mod tests;

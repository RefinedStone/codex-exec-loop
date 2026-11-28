//! Desktop 확인 준비는 현재 공유 장부만 읽는다. 증거의 발급·검증은 Gateway의 연결 경계가 맡는다.
use anyhow::{Result, bail};

use crate::application::port::inbound::stored_command_port::StoredCommand;
use crate::application::port::inbound::stored_command_port::issue_confirmation::IssueConfirmationPreview;

/// workspace는 Registry가 정한 대표 이슈 장부 범위다.
/// provider 읽기, 모델 호출, 상태 변경 없이 확인 창의 서버 기준을 만든다.
pub(crate) fn prepare(
    workspace: &str,
    command: &StoredCommand,
) -> Result<IssueConfirmationPreview> {
    match command {
        StoredCommand::ApplyIssueCommentRequest { key } => {
            #[cfg(not(test))]
            let collection = super::issue_collection::build(workspace);
            #[cfg(test)]
            let collection = super::issue_collection::build_for_kernel_test(workspace);
            collection
                .comment_command_service()
                .ok_or_else(|| anyhow::anyhow!("comment requests are unavailable"))?
                .prepare_confirmation(key)
        }
        StoredCommand::SetIssueBindingPaused {
            command_id,
            binding_id,
            paused: false,
            expected_revision,
        } => {
            #[cfg(not(test))]
            let collection = super::issue_collection::build(workspace);
            #[cfg(test)]
            let collection = super::issue_collection::build_for_kernel_test(workspace);
            let binding =
                crate::domain::issue_automation::IssueAutomationBindingId::new(binding_id.clone())
                    .map_err(anyhow::Error::msg)?;
            collection
                .binding_pause_service()
                .ok_or_else(|| anyhow::anyhow!("binding pause is unavailable"))?
                .prepare_confirmation(command_id, &binding, false, *expected_revision)
        }

        StoredCommand::WaiveCriterion { request } => {
            super::task_evidence::build(workspace).prepare_waiver(request)
        }
        StoredCommand::AnswerIssueDecision {
            criteria,
            decision_id,
            option,
            ..
        } => super::issue_collection::build_decisions(workspace)
            .prepare_confirmation_with_criteria(decision_id, option, criteria.as_deref()),
        StoredCommand::SetIssueAutomationMode { mode } => {
            super::issue_collection::build(workspace).prepare_mode_confirmation(*mode)
        }
        StoredCommand::SetIssueAutonomy { command_id, level } => {
            #[cfg(not(test))]
            let collection = super::issue_collection::build(workspace);
            // Registry의 kernel fixture와 같은 정책원을 사용한다.
            #[cfg(test)]
            let collection = super::issue_collection::build_for_kernel_test(workspace);
            collection
                .autonomy_service()
                .ok_or_else(|| anyhow::anyhow!("issue autonomy is unavailable"))?
                .prepare_confirmation(command_id, *level)
        }
        _ => bail!("this command does not support human confirmation"),
    }
}

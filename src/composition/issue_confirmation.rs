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
        StoredCommand::AnswerIssueDecision {
            decision_id,
            option,
            ..
        } => super::issue_collection::build_decisions(workspace)
            .prepare_confirmation(decision_id, option),
        StoredCommand::SetIssueAutomationMode { mode } => {
            super::issue_collection::build(workspace).prepare_mode_confirmation(*mode)
        }
        _ => bail!("this command does not support human confirmation"),
    }
}

//! 이슈별 개입의 전송 경계다. 안정 좌표와 재전송 식별자를 검증하고 원문은 기록에 싣지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::issue_automation::IssueSharedCopyState;
use super::{KernelCommand, KernelProtocolError, KernelQuery};
use crate::application::port::inbound::stored_command_port::StoredCommand;
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionReceipt, IssueInterventionRequest, IssueSteerDelivery,
};
use crate::domain::kernel_event::IssueEventReference;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueInterventionReceiptState {
    pub issue: IssueEventReference,
    pub command_id: String,
    pub action: String,
    pub work_order_id: Option<String>,
    pub authorization_epoch: u64,
    pub changed: bool,
    pub delivery: Option<IssueSteerDeliveryState>,
    pub handoff: Option<IssueSharedCopyState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueSteerDeliveryState {
    Queued,
    Sent,
    Unknown,
}

impl From<IssueInterventionReceipt> for IssueInterventionReceiptState {
    fn from(receipt: IssueInterventionReceipt) -> Self {
        Self {
            issue: (&receipt.issue).into(),
            command_id: receipt.command_id,
            action: receipt.action,
            work_order_id: receipt.work_order_id,
            authorization_epoch: receipt.authorization_epoch,
            changed: receipt.changed,
            delivery: receipt.delivery.map(|delivery| match delivery {
                IssueSteerDelivery::Queued => IssueSteerDeliveryState::Queued,
                IssueSteerDelivery::Sent => IssueSteerDeliveryState::Sent,
                IssueSteerDelivery::Unknown => IssueSteerDeliveryState::Unknown,
            }),
            handoff: receipt.handoff.map(|copy| IssueSharedCopyState {
                title: copy.title,
                body: copy.body,
            }),
        }
    }
}

pub(super) fn into_stored(
    issue: IssueEventReference,
    command_id: String,
    action: IssueInterventionAction,
    workspace: &str,
) -> Result<StoredCommand, KernelProtocolError> {
    if command_id.trim().is_empty()
        || command_id.len() > 256
        || command_id.chars().any(char::is_control)
    {
        return Err(KernelProtocolError::invalid_params(
            "command_id must be nonempty and at most 256 bytes without control characters",
        ));
    }
    if let IssueInterventionAction::Steer { message } = &action
        && (message.trim().is_empty() || message.len() > 64 * 1024)
    {
        return Err(KernelProtocolError::invalid_params(
            "steer_issue requires a nonempty message of at most 64 KiB",
        ));
    }
    let issue = KernelQuery::Issue { issue }
        .issue_reference(workspace)?
        .ok_or_else(|| KernelProtocolError::invalid_params("invalid issue reference"))?;
    // 취소의 confirm은 여기서 바꾸거나 버리지 않는다. 커널의 Destructive 게이트와 서비스가 각각 검사한다.
    Ok(StoredCommand::InterveneIssue {
        request: IssueInterventionRequest {
            issue,
            command_id,
            action,
        },
    })
}

pub(super) fn command_from_request(request: IssueInterventionRequest) -> KernelCommand {
    let issue = (&request.issue).into();
    let command_id = request.command_id;
    match request.action {
        IssueInterventionAction::Hold => KernelCommand::HoldIssue { issue, command_id },
        IssueInterventionAction::Release => KernelCommand::ReleaseIssue { issue, command_id },
        IssueInterventionAction::Steer { message } => KernelCommand::SteerIssue {
            issue,
            command_id,
            message,
        },
        IssueInterventionAction::TakeOver => KernelCommand::TakeOverIssue { issue, command_id },
        IssueInterventionAction::Retriage => KernelCommand::RetriageIssue { issue, command_id },
        IssueInterventionAction::Cancel { confirm } => KernelCommand::CancelIssueWork {
            issue,
            command_id,
            confirm,
        },
        IssueInterventionAction::Track => KernelCommand::TrackIssue { issue, command_id },
        IssueInterventionAction::Untrack => KernelCommand::UntrackIssue { issue, command_id },
    }
}

#[cfg(test)]
mod tests;

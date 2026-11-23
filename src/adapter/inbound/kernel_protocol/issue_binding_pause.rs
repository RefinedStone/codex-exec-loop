use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::issue_automation::binding_pause::{
    IssueBindingPauseState as DomainState, StoredIssueBindingPauseCommand,
};
use crate::domain::kernel_event::IssueBindingPauseReceiptRecord;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueBindingPauseState {
    Ready {
        binding_id: String,
        revision: u64,
        paused: bool,
        pending: Option<IssueBindingPausePendingState>,
    },
    Unavailable {
        binding_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueBindingPausePendingState {
    pub command_id: String,
    pub paused: bool,
}

impl IssueBindingPauseState {
    pub fn from_current(binding_id: &str, state: Option<&DomainState>) -> Self {
        match state.filter(|state| state.binding_id.as_str() == binding_id) {
            Some(state) => Self::Ready {
                binding_id: binding_id.to_owned(),
                revision: state.revision,
                paused: state.paused,
                pending: state
                    .pending
                    .as_ref()
                    .map(|intent| IssueBindingPausePendingState {
                        command_id: intent.command_id.clone(),
                        paused: intent.paused,
                    }),
            },
            None => Self::Unavailable {
                binding_id: binding_id.to_owned(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueBindingPauseRequestedState {
    pub command_id: String,
    pub binding_id: String,
    pub requested_paused: bool,
    pub accepted_revision: u64,
    /// None means accepted; current state is read separately and may already be newer.
    pub receipt: Option<IssueBindingPauseReceiptRecord>,
}

impl From<&StoredIssueBindingPauseCommand> for IssueBindingPauseRequestedState {
    fn from(command: &StoredIssueBindingPauseCommand) -> Self {
        Self {
            command_id: command.intent.command_id.clone(),
            binding_id: command.intent.binding_id.as_str().to_owned(),
            requested_paused: command.intent.paused,
            accepted_revision: command.accepted_revision,
            receipt: command.receipt.as_ref().map(Into::into),
        }
    }
}

#[cfg(test)]
mod tests;

pub(super) fn validate_input(
    command_id: &str,
    binding_id: &str,
    revision: u64,
) -> Result<(), super::KernelProtocolError> {
    use crate::domain::issue_automation::{
        IssueAutomationBindingId, MAX_ISSUE_AUTOMATION_REVISION,
    };
    super::issue_autonomy::validate_input(command_id)?;
    let binding = IssueAutomationBindingId::new(binding_id.to_owned())
        .map_err(|_| super::KernelProtocolError::invalid_params("invalid binding pause request"))?;
    if crate::domain::issue_automation::binding_pause::validate_binding_pause_id(&binding).is_err()
        || revision >= MAX_ISSUE_AUTOMATION_REVISION
    {
        return Err(super::KernelProtocolError::invalid_params(
            "invalid binding pause request",
        ));
    }
    Ok(())
}

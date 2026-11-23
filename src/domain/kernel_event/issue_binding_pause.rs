use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::issue_automation::binding_pause::{
    IssueBindingPauseEvent, IssueBindingPauseOutcome, IssueBindingPauseReceipt,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueBindingPauseReceiptRecord {
    pub command_id: String,
    pub binding_id: String,
    pub revision: u64,
    pub paused: bool,
    pub outcome: IssueBindingPauseOutcome,
    pub changed: bool,
}

impl From<&IssueBindingPauseReceipt> for IssueBindingPauseReceiptRecord {
    fn from(receipt: &IssueBindingPauseReceipt) -> Self {
        Self {
            command_id: receipt.command_id.clone(),
            binding_id: receipt.binding_id.as_str().to_owned(),
            revision: receipt.revision,
            paused: receipt.paused,
            outcome: receipt.outcome,
            changed: receipt.changed,
        }
    }
}

impl super::KernelEvent {
    pub fn issue_binding_pause(event: &IssueBindingPauseEvent) -> Self {
        Self::IssueBindingPauseCompleted {
            event_id: event.event_id.clone(),
            receipt: (&event.receipt).into(),
        }
    }

    pub fn issue_binding_pause_event_id(&self) -> Option<&str> {
        match self {
            Self::IssueBindingPauseCompleted { event_id, .. } => Some(event_id),
            _ => None,
        }
    }
}

#[cfg(test)]
pub(crate) fn sample_issue_binding_pause_event() -> super::KernelEvent {
    super::KernelEvent::IssueBindingPauseCompleted {
        event_id: "binding-pause-completed:pause-example".into(),
        receipt: IssueBindingPauseReceiptRecord {
            command_id: "pause-example".into(),
            binding_id: "binding-example".into(),
            revision: 3,
            paused: true,
            outcome: IssueBindingPauseOutcome::Paused,
            changed: true,
        },
    }
}

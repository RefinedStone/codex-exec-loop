use std::sync::mpsc::Sender;

use crate::domain::conversation::{ConversationApprovalReview, ConversationToolActivity};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationStreamEvent {
    AttachmentObserved {
        profile: TerminalBridgeAttachmentProfile,
    },
    ThreadPrepared {
        thread_id: String,
        title: String,
        cwd: String,
    },
    TurnStarted {
        turn_id: String,
    },
    StatusUpdated {
        text: String,
    },
    AgentMessageDelta {
        item_id: String,
        phase: Option<String>,
        delta: String,
    },
    AgentMessageCompleted {
        item_id: String,
        phase: Option<String>,
        text: String,
    },
    ToolActivity {
        activity: ConversationToolActivity,
    },
    ApprovalReviewUpdated {
        review: ConversationApprovalReview,
    },
    TurnCompleted {
        turn_id: String,
        changed_planning_file_paths: Vec<String>,
    },
    Failed {
        message: String,
    },
}

impl ConversationStreamEvent {
    pub const fn attachment_observed(profile: TerminalBridgeAttachmentProfile) -> Self {
        Self::AttachmentObserved { profile }
    }

    pub const fn codex_app_server_launch_attachment() -> Self {
        Self::attachment_observed(TerminalBridgeAttachmentProfile::codex_app_server_launch())
    }

    pub const fn codex_app_server_reattach_attachment() -> Self {
        Self::attachment_observed(TerminalBridgeAttachmentProfile::codex_app_server_reattach())
    }
}

pub(crate) fn emit_attachment_observed(
    event_sender: &Sender<ConversationStreamEvent>,
    profile: TerminalBridgeAttachmentProfile,
) {
    let _ = event_sender.send(ConversationStreamEvent::attachment_observed(profile));
}

pub(crate) fn emit_codex_app_server_launch_attachment(
    event_sender: &Sender<ConversationStreamEvent>,
) {
    emit_attachment_observed(
        event_sender,
        TerminalBridgeAttachmentProfile::codex_app_server_launch(),
    );
}

pub(crate) fn emit_codex_app_server_reattach_attachment(
    event_sender: &Sender<ConversationStreamEvent>,
) {
    emit_attachment_observed(
        event_sender,
        TerminalBridgeAttachmentProfile::codex_app_server_reattach(),
    );
}

#[cfg(test)]
mod tests {
    use super::ConversationStreamEvent;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    #[test]
    fn codex_attachment_helpers_build_expected_profiles() {
        assert_eq!(
            ConversationStreamEvent::codex_app_server_launch_attachment(),
            ConversationStreamEvent::AttachmentObserved {
                profile: TerminalBridgeAttachmentProfile::codex_app_server_launch(),
            }
        );
        assert_eq!(
            ConversationStreamEvent::codex_app_server_reattach_attachment(),
            ConversationStreamEvent::AttachmentObserved {
                profile: TerminalBridgeAttachmentProfile::codex_app_server_reattach(),
            }
        );
    }
}

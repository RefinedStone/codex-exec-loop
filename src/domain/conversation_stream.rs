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

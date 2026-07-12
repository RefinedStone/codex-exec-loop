use crate::domain::conversation::{
    ConversationApprovalRequest, ConversationApprovalResolution, ConversationApprovalReview,
    ConversationToolActivity,
};
use crate::domain::conversation_item_lifecycle::ConversationItemLifecycleObservation;
use crate::domain::conversation_progressive_activity::ConversationProgressiveActivityBatch;
use crate::domain::conversation_runtime_envelope::{
    ConversationRuntimeConfigurationRequest, ConversationRuntimeEnvelope,
    ConversationRuntimeEnvelopeObservation,
};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
use crate::domain::turn_terminal::{ConversationTurnError, ConversationTurnTerminalReceipt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationStreamEvent {
    AttachmentObserved {
        profile: TerminalBridgeAttachmentProfile,
    },
    ThreadPrepared {
        thread_id: String,
        title: String,
        cwd: String,
        runtime_envelope: Box<ConversationRuntimeEnvelope>,
    },
    TurnStarted {
        turn_id: String,
        runtime_request: Box<ConversationRuntimeConfigurationRequest>,
    },
    RuntimeEnvelopeObserved {
        observation: Box<ConversationRuntimeEnvelopeObservation>,
    },
    ItemLifecycleObserved {
        observation: Box<ConversationItemLifecycleObservation>,
    },
    ProgressiveActivityObserved {
        batch: Box<ConversationProgressiveActivityBatch>,
    },
    StatusUpdated {
        text: String,
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
    ApprovalRequested {
        request: ConversationApprovalRequest,
    },
    ApprovalResolved {
        approval_id: String,
        resolution: ConversationApprovalResolution,
    },
    TurnInterruptRequestFailed {
        message: String,
    },
    TurnRetrying {
        thread_id: String,
        turn_id: String,
        error: ConversationTurnError,
    },
    TurnTerminal {
        receipt: ConversationTurnTerminalReceipt,
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

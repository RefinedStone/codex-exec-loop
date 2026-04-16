use crate::domain::conversation::{ConversationApprovalReview, ConversationToolActivity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationStreamEvent {
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

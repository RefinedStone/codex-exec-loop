#[derive(Debug, Clone)]
pub struct ConversationSnapshot {
    pub thread_id: String,
    pub title: String,
    pub cwd: String,
    pub messages: Vec<ConversationMessage>,
    pub warnings: Vec<String>,
    pub runtime_notices: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub kind: ConversationMessageKind,
    pub text: String,
    pub debug_detail: Option<String>,
    pub phase: Option<String>,
    pub item_id: Option<String>,
    pub display_label: Option<String>,
}

impl ConversationMessage {
    pub fn new(
        kind: ConversationMessageKind,
        text: impl Into<String>,
        phase: Option<String>,
        item_id: Option<String>,
    ) -> Self {
        Self {
            kind,
            text: text.into(),
            debug_detail: None,
            phase,
            item_id,
            display_label: None,
        }
    }

    pub fn with_display_label(mut self, label: impl Into<String>) -> Self {
        self.display_label = Some(label.into());
        self
    }

    pub fn with_debug_detail(mut self, detail: impl Into<String>) -> Self {
        self.debug_detail = Some(detail.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationMessageKind {
    User,
    Agent,
    Tool,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationToolActivityKind {
    FileChange,
    CommandExecution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationToolActivity {
    pub kind: ConversationToolActivityKind,
    pub text: String,
    pub file_change_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationApprovalReviewStatus {
    InProgress,
    Approved,
    Denied,
    Aborted,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationApprovalReview {
    pub target_item_id: String,
    pub status: ConversationApprovalReviewStatus,
    pub risk_level: Option<String>,
    pub rationale: Option<String>,
}

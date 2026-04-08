#[derive(Debug, Clone)]
pub struct ConversationSnapshot {
    pub thread_id: String,
    pub title: String,
    pub cwd: String,
    pub messages: Vec<ConversationMessage>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub kind: ConversationMessageKind,
    pub text: String,
    pub phase: Option<String>,
    pub item_id: Option<String>,
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
            phase,
            item_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationMessageKind {
    User,
    Agent,
    Tool,
    Status,
}

impl ConversationMessageKind {
    pub fn label(&self, phase: Option<&str>) -> &'static str {
        match self {
            ConversationMessageKind::User => "You",
            ConversationMessageKind::Agent => match phase {
                Some("commentary") => "Codex Commentary",
                _ => "Codex",
            },
            ConversationMessageKind::Tool => "Tool",
            ConversationMessageKind::Status => "Status",
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationApprovalReviewStatus {
    InProgress,
    Approved,
    Denied,
    Aborted,
}

impl ConversationApprovalReviewStatus {
    fn summary_label(self) -> &'static str {
        match self {
            Self::InProgress => "reviewing",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Aborted => "aborted",
        }
    }

    fn status_prefix(self) -> &'static str {
        match self {
            Self::InProgress => "approval review in progress",
            Self::Approved => "approval review approved",
            Self::Denied => "approval review denied",
            Self::Aborted => "approval review aborted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationApprovalReview {
    pub target_item_id: String,
    pub status: ConversationApprovalReviewStatus,
    pub risk_level: Option<String>,
    pub rationale: Option<String>,
}

impl ConversationApprovalReview {
    pub fn summary_text(&self) -> String {
        match self
            .risk_level
            .as_deref()
            .filter(|risk| !risk.trim().is_empty())
        {
            Some(risk_level) => format!("{} {risk_level}", self.status.summary_label()),
            None => self.status.summary_label().to_string(),
        }
    }

    pub fn status_text(&self) -> String {
        let mut segments = vec![self.status.status_prefix().to_string()];

        if !self.target_item_id.trim().is_empty() {
            segments.push(format!("target: {}", self.target_item_id));
        }
        if let Some(risk_level) = self
            .risk_level
            .as_deref()
            .filter(|risk| !risk.trim().is_empty())
        {
            segments.push(format!("risk: {risk_level}"));
        }

        segments.join(" / ")
    }
}

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
    },
    Failed {
        message: String,
    },
}

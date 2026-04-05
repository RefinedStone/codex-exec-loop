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

#[derive(Debug, Clone)]
pub struct ConversationToolActivity {
    pub kind: ConversationToolActivityKind,
    pub text: String,
    pub file_change_count: usize,
}

#[derive(Debug, Clone)]
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
    TurnCompleted {
        turn_id: String,
    },
    Failed {
        message: String,
    },
}

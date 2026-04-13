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

    pub fn label(&self) -> &str {
        self.display_label
            .as_deref()
            .unwrap_or_else(|| self.kind.label(self.phase.as_deref()))
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationApprovalReviewStatus {
    InProgress,
    Approved,
    Denied,
    Aborted,
    Unknown(String),
}

impl ConversationApprovalReviewStatus {
    fn summary_label(&self) -> String {
        match self {
            Self::InProgress => "reviewing".to_string(),
            Self::Approved => "approved".to_string(),
            Self::Denied => "denied".to_string(),
            Self::Aborted => "aborted".to_string(),
            Self::Unknown(value) => humanize_protocol_status(value),
        }
    }

    fn status_prefix(&self) -> String {
        match self {
            Self::InProgress => "approval review in progress".to_string(),
            Self::Approved => "approval review approved".to_string(),
            Self::Denied => "approval review denied".to_string(),
            Self::Aborted => "approval review aborted".to_string(),
            Self::Unknown(value) => format!("approval review {}", humanize_protocol_status(value)),
        }
    }
}

fn humanize_protocol_status(value: &str) -> String {
    let mut normalized = String::new();
    let mut previous_was_separator = false;
    let mut previous_was_lower_or_digit = false;

    for ch in value.chars() {
        if ch == '-' || ch == '_' || ch.is_whitespace() {
            if !normalized.is_empty() && !previous_was_separator {
                normalized.push(' ');
            }
            previous_was_separator = true;
            previous_was_lower_or_digit = false;
            continue;
        }

        if ch.is_uppercase() && previous_was_lower_or_digit && !normalized.ends_with(' ') {
            normalized.push(' ');
        }

        normalized.extend(ch.to_lowercase());
        previous_was_separator = false;
        previous_was_lower_or_digit = ch.is_lowercase() || ch.is_ascii_digit();
    }

    normalized.trim().to_string()
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
            None => self.status.summary_label(),
        }
    }

    pub fn status_text(&self) -> String {
        let mut segments = vec![self.status.status_prefix()];

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

#[cfg(test)]
mod tests {
    use super::{ConversationApprovalReview, ConversationApprovalReviewStatus};

    #[test]
    fn unknown_approval_statuses_are_preserved_as_readable_text() {
        let review = ConversationApprovalReview {
            target_item_id: "command-1".to_string(),
            status: ConversationApprovalReviewStatus::Unknown("needsHumanReview".to_string()),
            risk_level: Some("high".to_string()),
            rationale: None,
        };

        assert_eq!(review.summary_text(), "needs human review high");
        assert_eq!(
            review.status_text(),
            "approval review needs human review / target: command-1 / risk: high"
        );
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
        changed_planning_file_paths: Vec<String>,
    },
    Failed {
        message: String,
    },
}

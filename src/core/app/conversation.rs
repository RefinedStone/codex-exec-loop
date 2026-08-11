use crate::domain::conversation::{
    ConversationSnapshot as DomainConversationSnapshot, ConversationTurnOptions,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationThreadReviewSnapshot {
    pub thread_id: String,
    pub review_id: String,
    pub review_label: String,
    pub review_state: String,
    pub review_summary: String,
    pub requested_at: String,
    pub updated_at: String,
    pub handoff_target: Option<String>,
    pub handoff_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationReadySnapshot {
    pub conversation: Box<DomainConversationSnapshot>,
    /// The exact saved option policy for this thread.  The composition layer
    /// resolves it before Core publishes the immutable snapshot so inbound
    /// adapters never read persistence ports directly.
    pub turn_options: ConversationTurnOptions,
    pub thread_id: String,
    pub title: String,
    pub workspace_directory: String,
    pub message_count: usize,
    pub warning_count: usize,
    pub runtime_notice_count: usize,
    pub thread_review: Vec<ConversationThreadReviewSnapshot>,
}

impl ConversationReadySnapshot {
    pub fn from_parts(
        conversation: DomainConversationSnapshot,
        thread_review: Vec<ConversationThreadReviewSnapshot>,
    ) -> Self {
        Self::from_parts_with_turn_options(
            conversation,
            thread_review,
            ConversationTurnOptions::app_server_default(),
        )
    }

    pub fn from_parts_with_turn_options(
        conversation: DomainConversationSnapshot,
        thread_review: Vec<ConversationThreadReviewSnapshot>,
        turn_options: ConversationTurnOptions,
    ) -> Self {
        Self {
            thread_id: conversation.thread_id.clone(),
            title: conversation.title.clone(),
            workspace_directory: conversation.cwd.clone(),
            message_count: conversation.messages.len(),
            warning_count: conversation.warnings.len(),
            runtime_notice_count: conversation.runtime_notices.len(),
            conversation: Box::new(conversation),
            turn_options,
            thread_review,
        }
    }
}

impl From<DomainConversationSnapshot> for ConversationReadySnapshot {
    fn from(conversation: DomainConversationSnapshot) -> Self {
        Self::from_parts(conversation, Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationSnapshot {
    Idle,
    Loading,
    Ready(Box<ConversationReadySnapshot>),
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConversationState {
    #[default]
    Idle,
    Loading,
    Ready(Box<ConversationReadySnapshot>),
    Failed(String),
}

impl ConversationState {
    pub fn snapshot(&self) -> ConversationSnapshot {
        match self {
            Self::Idle => ConversationSnapshot::Idle,
            Self::Loading => ConversationSnapshot::Loading,
            Self::Ready(ready) => ConversationSnapshot::Ready(ready.clone()),
            Self::Failed(message) => ConversationSnapshot::Failed {
                message: message.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

    #[test]
    fn ready_snapshot_keeps_domain_conversation_and_summary() {
        let conversation = DomainConversationSnapshot {
            thread_id: "thread-1".to_string(),
            title: "Build core runtime".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: vec![ConversationMessage::new(
                ConversationMessageKind::User,
                "hello",
                None,
                None,
            )],
            warnings: vec!["partial replay".to_string()],
            runtime_notices: vec!["reattached runtime".to_string()],
            item_lifecycle: Default::default(),
        };

        let ready = ConversationReadySnapshot::from(conversation.clone());

        assert_eq!(
            ready,
            ConversationReadySnapshot {
                conversation: Box::new(conversation),
                turn_options: ConversationTurnOptions::app_server_default(),
                thread_id: "thread-1".to_string(),
                title: "Build core runtime".to_string(),
                workspace_directory: "/tmp/workspace".to_string(),
                message_count: 1,
                warning_count: 1,
                runtime_notice_count: 1,
                thread_review: Vec::new(),
            }
        );
    }

    #[test]
    fn ready_snapshot_from_parts_keeps_thread_review() {
        let conversation = DomainConversationSnapshot {
            thread_id: "thread-1".to_string(),
            title: "Build core runtime".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        };

        let ready = ConversationReadySnapshot::from_parts(
            conversation.clone(),
            vec![ConversationThreadReviewSnapshot {
                thread_id: "thread-1".to_string(),
                review_id: "review-1".to_string(),
                review_label: "Manual review".to_string(),
                review_state: "pending".to_string(),
                review_summary: "Need operator follow-up".to_string(),
                requested_at: "2026-07-06T10:00:00Z".to_string(),
                updated_at: "2026-07-06T11:00:00Z".to_string(),
                handoff_target: Some("operator".to_string()),
                handoff_note: Some("resume in inbox".to_string()),
            }],
        );

        assert_eq!(ready.conversation, Box::new(conversation));
        assert_eq!(ready.thread_review.len(), 1);
        assert_eq!(ready.thread_review[0].review_id, "review-1");
        assert_eq!(
            ready.thread_review[0].handoff_target.as_deref(),
            Some("operator")
        );
    }

    #[test]
    fn failed_state_projects_message_snapshot() {
        assert_eq!(
            ConversationState::Failed("thread missing".to_string()).snapshot(),
            ConversationSnapshot::Failed {
                message: "thread missing".to_string(),
            }
        );
    }
}

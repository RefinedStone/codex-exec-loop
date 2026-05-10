use crate::domain::conversation::ConversationSnapshot as DomainConversationSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationReadySnapshot {
    pub conversation: Box<DomainConversationSnapshot>,
    pub thread_id: String,
    pub title: String,
    pub workspace_directory: String,
    pub message_count: usize,
    pub warning_count: usize,
    pub runtime_notice_count: usize,
}

impl From<DomainConversationSnapshot> for ConversationReadySnapshot {
    fn from(conversation: DomainConversationSnapshot) -> Self {
        Self {
            thread_id: conversation.thread_id.clone(),
            title: conversation.title.clone(),
            workspace_directory: conversation.cwd.clone(),
            message_count: conversation.messages.len(),
            warning_count: conversation.warnings.len(),
            runtime_notice_count: conversation.runtime_notices.len(),
            conversation: Box::new(conversation),
        }
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
        };

        let ready = ConversationReadySnapshot::from(conversation.clone());

        assert_eq!(
            ready,
            ConversationReadySnapshot {
                conversation: Box::new(conversation),
                thread_id: "thread-1".to_string(),
                title: "Build core runtime".to_string(),
                workspace_directory: "/tmp/workspace".to_string(),
                message_count: 1,
                warning_count: 1,
                runtime_notice_count: 1,
            }
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

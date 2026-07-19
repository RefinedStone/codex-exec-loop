use super::ConversationThreadReviewSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCenterInboxItemSnapshot {
    pub review_id: String,
    pub thread_id: String,
    pub inbox_state: String,
    pub summary: String,
    pub requested_at: String,
    pub last_activity_at: String,
    pub handoff_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCenterHistoryEntrySnapshot {
    pub review_id: String,
    pub thread_id: String,
    pub event_kind: String,
    pub summary: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCenterSnapshot {
    pub current_thread_reviews: Result<Vec<ConversationThreadReviewSnapshot>, String>,
    pub pending_inbox: Result<Vec<ReviewCenterInboxItemSnapshot>, String>,
    pub recent_history: Result<Vec<ReviewCenterHistoryEntrySnapshot>, String>,
}

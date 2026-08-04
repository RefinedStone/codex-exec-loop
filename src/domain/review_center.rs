use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewCenterThreadProjection {
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

impl ReviewCenterThreadProjection {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        thread_id: impl Into<String>,
        review_id: impl Into<String>,
        review_label: impl Into<String>,
        review_state: impl Into<String>,
        review_summary: impl Into<String>,
        requested_at: impl Into<String>,
        updated_at: impl Into<String>,
    ) -> Self {
        Self {
            thread_id: thread_id.into(),
            review_id: review_id.into(),
            review_label: review_label.into(),
            review_state: review_state.into(),
            review_summary: review_summary.into(),
            requested_at: requested_at.into(),
            updated_at: updated_at.into(),
            handoff_target: None,
            handoff_note: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewCenterInboxItem {
    pub review_id: String,
    pub thread_id: String,
    pub inbox_state: String,
    pub summary: String,
    pub requested_at: String,
    pub last_activity_at: String,
    pub handoff_target: Option<String>,
}

impl ReviewCenterInboxItem {
    pub fn new(
        review_id: impl Into<String>,
        thread_id: impl Into<String>,
        inbox_state: impl Into<String>,
        summary: impl Into<String>,
        requested_at: impl Into<String>,
        last_activity_at: impl Into<String>,
    ) -> Self {
        Self {
            review_id: review_id.into(),
            thread_id: thread_id.into(),
            inbox_state: inbox_state.into(),
            summary: summary.into(),
            requested_at: requested_at.into(),
            last_activity_at: last_activity_at.into(),
            handoff_target: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewCenterHistoryEntry {
    pub review_id: String,
    pub thread_id: String,
    pub event_kind: String,
    pub summary: String,
    pub recorded_at: String,
}

impl ReviewCenterHistoryEntry {
    pub fn new(
        review_id: impl Into<String>,
        thread_id: impl Into<String>,
        event_kind: impl Into<String>,
        summary: impl Into<String>,
        recorded_at: impl Into<String>,
    ) -> Self {
        Self {
            review_id: review_id.into(),
            thread_id: thread_id.into(),
            event_kind: event_kind.into(),
            summary: summary.into(),
            recorded_at: recorded_at.into(),
        }
    }
}

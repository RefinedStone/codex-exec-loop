use anyhow::Result;

use crate::application::service::review_center::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterThreadProjection,
};

pub trait ReviewCenterRepositoryPort: Send + Sync {
    fn load_thread_reviews(&self, thread_id: &str) -> Result<Vec<ReviewCenterThreadProjection>>;

    fn load_pending_inbox(&self) -> Result<Vec<ReviewCenterInboxItem>>;

    fn load_recent_history(&self) -> Result<Vec<ReviewCenterHistoryEntry>>;

    fn upsert_thread_review(&self, review: &ReviewCenterThreadProjection) -> Result<()>;

    fn replace_pending_inbox(&self, inbox: &[ReviewCenterInboxItem]) -> Result<()>;

    fn append_history_entry(&self, entry: &ReviewCenterHistoryEntry) -> Result<()>;
}

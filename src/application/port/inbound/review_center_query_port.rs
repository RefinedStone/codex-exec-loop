use std::sync::Arc;

use anyhow::Result;

pub use crate::domain::review_center::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterThreadProjection,
};

pub trait ReviewCenterQueryPort: Send + Sync {
    fn workspace_dir(&self) -> &str;

    fn load_thread_reviews(&self, thread_id: &str) -> Result<Vec<ReviewCenterThreadProjection>>;

    fn load_pending_inbox(&self) -> Result<Vec<ReviewCenterInboxItem>>;

    fn load_recent_history(&self) -> Result<Vec<ReviewCenterHistoryEntry>>;
}

impl<T> ReviewCenterQueryPort for Arc<T>
where
    T: ReviewCenterQueryPort + ?Sized,
{
    fn workspace_dir(&self) -> &str {
        self.as_ref().workspace_dir()
    }

    fn load_thread_reviews(&self, thread_id: &str) -> Result<Vec<ReviewCenterThreadProjection>> {
        self.as_ref().load_thread_reviews(thread_id)
    }

    fn load_pending_inbox(&self) -> Result<Vec<ReviewCenterInboxItem>> {
        self.as_ref().load_pending_inbox()
    }

    fn load_recent_history(&self) -> Result<Vec<ReviewCenterHistoryEntry>> {
        self.as_ref().load_recent_history()
    }
}

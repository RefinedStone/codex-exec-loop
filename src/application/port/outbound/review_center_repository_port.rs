pub use crate::domain::review_center::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterThreadProjection,
};
use anyhow::Result;

pub trait ReviewCenterRepositoryPort: Send + Sync {
    fn load_thread_reviews(
        &self,
        workspace_dir: &str,
        thread_id: &str,
    ) -> Result<Vec<ReviewCenterThreadProjection>>;

    fn load_pending_inbox(&self, workspace_dir: &str) -> Result<Vec<ReviewCenterInboxItem>>;

    fn load_recent_history(&self, workspace_dir: &str) -> Result<Vec<ReviewCenterHistoryEntry>>;

    fn upsert_thread_review(
        &self,
        workspace_dir: &str,
        review: &ReviewCenterThreadProjection,
    ) -> Result<()>;

    fn replace_pending_inbox(
        &self,
        workspace_dir: &str,
        inbox: &[ReviewCenterInboxItem],
    ) -> Result<()>;

    fn append_history_entry(
        &self,
        workspace_dir: &str,
        entry: &ReviewCenterHistoryEntry,
    ) -> Result<()>;
}

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

    // Store one review change: its projection, its own inbox item (upsert, or remove when None),
    // and an optional history entry. Durable adapters must do this in one atomic write so
    // concurrent writers never drop each other's inbox items; this default suits in-memory fakes.
    fn record_review_update(
        &self,
        workspace_dir: &str,
        review: &ReviewCenterThreadProjection,
        pending_item: Option<&ReviewCenterInboxItem>,
        history_entry: Option<&ReviewCenterHistoryEntry>,
    ) -> Result<()> {
        self.upsert_thread_review(workspace_dir, review)?;
        let mut inbox = self
            .load_pending_inbox(workspace_dir)?
            .into_iter()
            .filter(|item| {
                !(item.review_id == review.review_id && item.thread_id == review.thread_id)
            })
            .collect::<Vec<_>>();
        inbox.extend(pending_item.cloned());
        self.replace_pending_inbox(workspace_dir, &inbox)?;
        if let Some(entry) = history_entry {
            self.append_history_entry(workspace_dir, entry)?;
        }
        Ok(())
    }
}

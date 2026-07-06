use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterRepositoryPort,
    ReviewCenterThreadProjection,
};
#[derive(Clone)]
pub struct ReviewCenterReadService {
    workspace_dir: String,
    review_center_repository: Arc<dyn ReviewCenterRepositoryPort>,
}

impl ReviewCenterReadService {
    pub fn new(
        workspace_dir: impl Into<String>,
        review_center_repository: Arc<dyn ReviewCenterRepositoryPort>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            review_center_repository,
        }
    }

    pub fn load_thread_reviews(
        &self,
        thread_id: &str,
    ) -> Result<Vec<ReviewCenterThreadProjection>> {
        self.review_center_repository
            .load_thread_reviews(&self.workspace_dir, thread_id)
    }

    pub fn load_pending_inbox(&self) -> Result<Vec<ReviewCenterInboxItem>> {
        self.review_center_repository
            .load_pending_inbox(&self.workspace_dir)
    }

    pub fn load_recent_history(&self) -> Result<Vec<ReviewCenterHistoryEntry>> {
        self.review_center_repository
            .load_recent_history(&self.workspace_dir)
    }

    pub fn workspace_dir(&self) -> &str {
        self.workspace_dir.as_str()
    }

    pub fn load_thread_reviews_for_workspace(
        &self,
        workspace_dir: &str,
        thread_id: &str,
    ) -> Result<Vec<ReviewCenterThreadProjection>> {
        self.review_center_repository
            .load_thread_reviews(workspace_dir, thread_id)
    }

    pub fn load_pending_inbox_for_workspace(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<ReviewCenterInboxItem>> {
        self.review_center_repository
            .load_pending_inbox(workspace_dir)
    }

    pub fn load_recent_history_for_workspace(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<ReviewCenterHistoryEntry>> {
        self.review_center_repository
            .load_recent_history(workspace_dir)
    }
}

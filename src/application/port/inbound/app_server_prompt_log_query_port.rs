use std::sync::Arc;

use anyhow::Result;

pub use crate::domain::app_server_prompt_log::{
    AppServerPromptInputRecord, AppServerPromptInteractionRecord,
    AppServerPromptInteractionSnapshot, AppServerPromptOutputRecord,
};

pub trait AppServerPromptLogQueryPort: Send + Sync {
    fn is_enabled(&self) -> bool;

    fn load_recent_app_server_prompt_interactions(
        &self,
        workspace_dir: &str,
        limit: usize,
    ) -> Result<AppServerPromptInteractionSnapshot>;
}

impl<T> AppServerPromptLogQueryPort for Arc<T>
where
    T: AppServerPromptLogQueryPort + ?Sized,
{
    fn is_enabled(&self) -> bool {
        self.as_ref().is_enabled()
    }

    fn load_recent_app_server_prompt_interactions(
        &self,
        workspace_dir: &str,
        limit: usize,
    ) -> Result<AppServerPromptInteractionSnapshot> {
        self.as_ref()
            .load_recent_app_server_prompt_interactions(workspace_dir, limit)
    }
}

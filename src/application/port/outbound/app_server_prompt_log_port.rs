use anyhow::Result;

pub(crate) use crate::domain::app_server_prompt_log::{
    APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS, APP_SERVER_PROMPT_LOG_MAX_ITEMS_PER_DIRECTION,
    APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS, bounded_prompt_log_string,
};
pub use crate::domain::app_server_prompt_log::{
    AppServerPromptInputRecord, AppServerPromptInteractionRecord,
    AppServerPromptInteractionSnapshot, AppServerPromptOutputRecord,
};

pub trait AppServerPromptLogPort: Send + Sync {
    fn is_enabled(&self) -> bool {
        false
    }

    fn append_app_server_prompt_interaction(
        &self,
        workspace_dir: &str,
        record: AppServerPromptInteractionRecord,
    ) -> Result<()>;

    fn load_recent_app_server_prompt_interactions(
        &self,
        workspace_dir: &str,
        limit: usize,
    ) -> Result<AppServerPromptInteractionSnapshot>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppServerPromptLogMaintenanceMode {
    PurgeExpired,
    ClearAll,
}

pub trait AppServerPromptLogMaintenancePort: Send + Sync {
    fn maintain_app_server_prompt_logs(
        &self,
        workspace_dir: &str,
        mode: AppServerPromptLogMaintenanceMode,
    ) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct NoopAppServerPromptLogPort;

impl AppServerPromptLogPort for NoopAppServerPromptLogPort {
    fn is_enabled(&self) -> bool {
        false
    }

    fn append_app_server_prompt_interaction(
        &self,
        _workspace_dir: &str,
        _record: AppServerPromptInteractionRecord,
    ) -> Result<()> {
        Ok(())
    }

    fn load_recent_app_server_prompt_interactions(
        &self,
        _workspace_dir: &str,
        _limit: usize,
    ) -> Result<AppServerPromptInteractionSnapshot> {
        Ok(AppServerPromptInteractionSnapshot::empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_prompt_log_disables_capture_work() {
        let port = NoopAppServerPromptLogPort;
        assert!(!port.is_enabled());
    }

    #[test]
    fn prompt_log_bounds_are_utf8_safe_before_persistence() {
        let bounded = bounded_prompt_log_string(
            &"한".repeat(APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS + 1),
            APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS,
        );

        assert_eq!(
            bounded.chars().count(),
            APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS
        );
        assert!(bounded.ends_with("retention policy]"));
    }
}

use std::sync::Arc;

use anyhow::Result;

use crate::application::port::inbound::app_server_prompt_log_query_port::AppServerPromptLogQueryPort;
use crate::application::port::outbound::app_server_prompt_log_port::AppServerPromptLogPort;
use crate::domain::app_server_prompt_log::AppServerPromptInteractionSnapshot;

pub struct AppServerPromptLogQueryService {
    prompt_log_port: Arc<dyn AppServerPromptLogPort>,
}

impl AppServerPromptLogQueryService {
    pub fn new(prompt_log_port: Arc<dyn AppServerPromptLogPort>) -> Self {
        Self { prompt_log_port }
    }
}

impl AppServerPromptLogQueryPort for AppServerPromptLogQueryService {
    fn is_enabled(&self) -> bool {
        self.prompt_log_port.is_enabled()
    }

    fn load_recent_app_server_prompt_interactions(
        &self,
        workspace_dir: &str,
        limit: usize,
    ) -> Result<AppServerPromptInteractionSnapshot> {
        self.prompt_log_port
            .load_recent_app_server_prompt_interactions(workspace_dir, limit)
    }
}

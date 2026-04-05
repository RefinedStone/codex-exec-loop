use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::codex_app_server_port::CodexAppServerPort;
use crate::domain::recent_sessions::RecentSessions;

#[derive(Clone)]
pub struct SessionService {
    codex_app_server_port: Arc<dyn CodexAppServerPort>,
}

impl SessionService {
    pub fn new(codex_app_server_port: Arc<dyn CodexAppServerPort>) -> Self {
        Self {
            codex_app_server_port,
        }
    }

    pub fn load_recent_sessions(&self, limit: usize) -> Result<RecentSessions> {
        self.codex_app_server_port.load_recent_sessions(limit)
    }
}

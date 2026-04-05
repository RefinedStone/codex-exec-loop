use anyhow::Result;

use crate::domain::recent_sessions::RecentSessions;

pub trait CodexAppServerPort: Send + Sync {
    fn load_startup_context(&self) -> Result<AppServerStartupContext>;
    fn load_recent_sessions(&self, limit: usize) -> Result<RecentSessions>;
}

#[derive(Debug, Clone)]
pub struct AppServerStartupContext {
    pub initialize_detail: String,
    pub account_detail: String,
    pub account_ok: bool,
    pub warnings: Vec<String>,
}

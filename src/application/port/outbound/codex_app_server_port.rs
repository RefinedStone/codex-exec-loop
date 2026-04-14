use std::sync::mpsc::Sender;

use anyhow::Result;

use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::recent_sessions::RecentSessions;

pub trait CodexAppServerPort: Send + Sync {
    fn load_startup_context(&self) -> Result<AppServerStartupContext>;
    fn load_recent_sessions(&self, limit: usize) -> Result<RecentSessions>;
    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot>;
    fn run_new_thread_stream(
        &self,
        cwd: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()>;
    fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct AppServerStartupContext {
    pub initialize_detail: String,
    pub account_detail: String,
    pub account_ok: bool,
    pub warnings: Vec<String>,
}

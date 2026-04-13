use std::sync::mpsc::Sender;

use anyhow::Result;

use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};
use crate::domain::recent_sessions::RecentSessions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewThreadReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewThreadStreamRequest {
    pub cwd: String,
    pub prompt: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<NewThreadReasoningEffort>,
}

impl NewThreadStreamRequest {
    pub fn new(cwd: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            cwd: cwd.into(),
            prompt: prompt.into(),
            model: None,
            reasoning_effort: None,
        }
    }
}

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
    fn run_new_thread_stream_with_overrides(
        &self,
        request: NewThreadStreamRequest,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.run_new_thread_stream(&request.cwd, &request.prompt, event_sender)
    }
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

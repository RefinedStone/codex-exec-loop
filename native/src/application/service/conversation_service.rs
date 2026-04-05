use std::sync::Arc;
use std::sync::mpsc::Sender;

use anyhow::Result;

use crate::application::port::outbound::codex_app_server_port::CodexAppServerPort;
use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};

#[derive(Clone)]
pub struct ConversationService {
    codex_app_server_port: Arc<dyn CodexAppServerPort>,
}

impl ConversationService {
    pub fn new(codex_app_server_port: Arc<dyn CodexAppServerPort>) -> Self {
        Self {
            codex_app_server_port,
        }
    }

    pub fn load_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        self.codex_app_server_port
            .load_conversation_snapshot(thread_id)
    }

    pub fn run_new_thread_stream(
        &self,
        cwd: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.codex_app_server_port
            .run_new_thread_stream(cwd, prompt, event_sender)
    }

    pub fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.codex_app_server_port
            .run_turn_stream(thread_id, prompt, event_sender)
    }
}

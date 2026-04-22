use std::sync::Arc;
use std::sync::mpsc::Sender;

use anyhow::Result;

use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::domain::conversation::ConversationSnapshot;

#[derive(Clone)]
pub struct ConversationService {
    interactive_turn_runtime_port: Arc<dyn InteractiveTurnRuntimePort>,
}

impl ConversationService {
    pub fn new(interactive_turn_runtime_port: Arc<dyn InteractiveTurnRuntimePort>) -> Self {
        Self {
            interactive_turn_runtime_port,
        }
    }

    pub fn load_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        self.interactive_turn_runtime_port
            .load_conversation_snapshot(thread_id)
    }

    pub fn run_new_thread_stream(
        &self,
        cwd: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.interactive_turn_runtime_port
            .run_new_thread_stream(cwd, prompt, event_sender)
    }

    pub fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.interactive_turn_runtime_port
            .run_turn_stream(thread_id, prompt, event_sender)
    }
}

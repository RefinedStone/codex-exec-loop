use std::sync::mpsc::Sender;

use anyhow::{Result, anyhow};

use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::domain::conversation::{ConversationRuntimeControlTruth, ConversationSnapshot};

pub trait InteractiveTurnRuntimePort: Send + Sync {
    fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth;
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
    fn request_interrupt(&self, _thread_id: Option<&str>) -> Result<()> {
        Err(anyhow!("interrupt control is not exposed by this runtime"))
    }
}

use std::sync::mpsc::Sender;

use anyhow::Result;

use crate::application::service::conversation_runtime_event::ConversationStreamEvent;

pub trait ParallelAgentWorkerPort: Send + Sync {
    fn run_isolated_new_thread_stream(
        &self,
        cwd: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct NoopParallelAgentWorkerPort;

impl ParallelAgentWorkerPort for NoopParallelAgentWorkerPort {
    fn run_isolated_new_thread_stream(
        &self,
        _cwd: &str,
        _prompt: &str,
        _event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        Ok(())
    }
}

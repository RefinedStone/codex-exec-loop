use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};

use anyhow::{Result, bail};

use super::conversation::GatewayConversationClient;
use super::{ClientUpdate, GatewayKernelClientRuntime};
use crate::application::port::conversation_stream::ConversationStreamSender;
use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
use crate::domain::conversation::{
    ConversationMessage, ConversationMessageKind, ConversationRuntimeControlTruth,
    ConversationSnapshot, ConversationTurnOptions,
};
use crate::domain::turn_terminal::ConversationTurnTerminalReceipt;

struct Reader {
    history: BTreeMap<String, ConversationSnapshot>,
}

pub(crate) fn conversation(thread_id: &str) -> ConversationSnapshot {
    ConversationSnapshot {
        thread_id: thread_id.into(),
        title: format!("Saved history for {thread_id}"),
        cwd: "/repo".into(),
        messages: vec![ConversationMessage::new(
            ConversationMessageKind::User,
            "gateway-history-message",
            None,
            None,
        )],
        warnings: Vec::new(),
        runtime_notices: Vec::new(),
        item_lifecycle: Default::default(),
    }
}

pub(crate) fn client_fixture(
    workspace: &str,
    history: Vec<(String, ConversationSnapshot)>,
) -> (GatewayKernelClientRuntime, mpsc::Sender<ClientUpdate>) {
    let (sender, updates) = mpsc::channel();
    let reader = Arc::new(Reader {
        history: history.into_iter().collect(),
    });
    let conversation = GatewayConversationClient::new(reader, sender.clone()).unwrap();
    (
        GatewayKernelClientRuntime::with_updates(workspace.into(), updates, conversation),
        sender,
    )
}

impl InteractiveTurnRuntimePort for Reader {
    fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
        ConversationRuntimeControlTruth::codex_app_server()
    }

    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        match self.history.get(thread_id) {
            Some(snapshot) => Ok(snapshot.clone()),
            None => bail!("no fixture history for {thread_id}"),
        }
    }

    fn request_stop_all_sessions(&self) -> Result<()> {
        panic!("the read-only client must not stop provider sessions")
    }

    fn run_new_thread_stream(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: ConversationTurnOptions,
        _: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        panic!("the read-only client must not create model turns")
    }

    fn run_turn_stream(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: ConversationTurnOptions,
        _: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        panic!("the read-only client must not create model turns")
    }
}

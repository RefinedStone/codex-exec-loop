use std::sync::{Arc, Mutex};

use serde_json::json;

use super::{connect_with_services, kernel_request};
use crate::adapter::inbound::gateway::session::GatewayServices;
use crate::application::port::outbound::chats_port::{
    ChatConversation, ChatSummary, ChatsError, ChatsPort, ChatsRead,
};

const CHAT_ID: &str = "chat-1";

#[derive(Default)]
struct ChatsProbe {
    calls: Mutex<Vec<&'static str>>,
}

impl ChatsPort for ChatsProbe {
    fn list_chats(&self) -> Result<ChatsRead<Vec<ChatSummary>>, ChatsError> {
        self.calls.lock().unwrap().push("list");
        Ok(ChatsRead::fresh(vec![ChatSummary {
            id: CHAT_ID.into(),
            title: "A conversation".into(),
        }]))
    }

    fn read_chat(&self, id: &str) -> Result<ChatsRead<ChatConversation>, ChatsError> {
        self.calls.lock().unwrap().push("read");
        Ok(ChatsRead::fresh(ChatConversation {
            id: id.into(),
            title: "A conversation".into(),
            messages: Vec::new(),
        }))
    }
}

#[tokio::test]
async fn configured_chats_are_available_only_on_remote_sessions() {
    let chats = Arc::new(ChatsProbe::default());
    let services = GatewayServices {
        chats: Some(chats.clone()),
        ..Default::default()
    };
    let requests = [
        ("akra/chats/list", json!({})),
        ("akra/chats/read", json!({ "id": CHAT_ID })),
    ];

    let mut local = connect_with_services("/", services.clone()).await;
    for (method, params) in &requests {
        let denied = kernel_request(&mut local, method, method, params.clone()).await;
        assert_eq!(denied["error"]["code"], -32601, "{method}: {denied}");
    }
    assert!(
        chats.calls.lock().unwrap().is_empty(),
        "local requests must not invoke the chats port"
    );

    let mut remote = connect_with_services("/host/cloud", services).await;
    let [(list, list_params), (read, read_params)] = requests;
    let listed = kernel_request(&mut remote, "list", list, list_params).await;
    assert_eq!(listed["result"]["chats"][0]["id"], CHAT_ID, "{listed}");
    let opened = kernel_request(&mut remote, "read", read, read_params).await;
    assert_eq!(opened["result"]["id"], CHAT_ID, "{opened}");
    assert_eq!(*chats.calls.lock().unwrap(), ["list", "read"]);
}

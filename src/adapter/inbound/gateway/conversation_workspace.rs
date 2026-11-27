//! 로컬 준비·조회 API. thread/start 성공이나 thread 소유권을 주장하지 않는다.
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::application::port::inbound::conversation_workspace_port::ConversationWorkspacePort;
use crate::domain::conversation_workspace::{
    ConversationWorkspaceInventoryQuery, ConversationWorkspaceQuery,
    PrepareConversationWorkspaceRequest,
};

pub(super) const PREFIX: &str = "akra/conversationWorkspace/";
const PREPARE: &str = "akra/conversationWorkspace/prepare";
const INVENTORY: &str = "akra/conversationWorkspace/inventory";
const GET: &str = "akra/conversationWorkspace/get";

#[derive(Deserialize)]
struct Envelope {
    params: Value,
}

pub(super) fn spawn_request(
    service: Arc<dyn ConversationWorkspacePort>,
    id: Value,
    method: String,
    text: String,
    outbox: mpsc::Sender<Message>,
) {
    tokio::spawn(async move {
        let fallback = id.clone();
        let response =
            tokio::task::spawn_blocking(move || respond(service.as_ref(), id, &method, &text))
                .await
                .unwrap_or_else(|_| error(fallback, -32000, "workspace request stopped"));
        let _ = outbox.send(Message::text(response)).await;
    });
}

fn respond(service: &dyn ConversationWorkspacePort, id: Value, method: &str, text: &str) -> String {
    let result = match method {
        PREPARE => params::<PrepareConversationWorkspaceRequest>(text).map(|request| {
            service
                .prepare(request)
                .and_then(|record| Ok(serde_json::to_value(record)?))
        }),
        INVENTORY => params::<ConversationWorkspaceInventoryQuery>(text).map(|query| {
            service
                .inventory(query)
                .and_then(|page| Ok(serde_json::to_value(page)?))
        }),
        GET => params::<ConversationWorkspaceQuery>(text).map(|query| {
            service
                .get(query)
                .and_then(|record| Ok(serde_json::to_value(record)?))
        }),
        _ => return error(id, -32601, "unknown conversation workspace method"),
    };
    match result {
        None => error(id, -32602, "invalid conversation workspace params"),
        Some(Err(failure)) => error(id, -32000, &format!("{failure:#}")),
        Some(Ok(result)) => json!({"jsonrpc":"2.0", "id":id, "result":result}).to_string(),
    }
}

fn params<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_value(serde_json::from_str::<Envelope>(text).ok()?.params).ok()
}

fn error(id: Value, code: i64, message: &str) -> String {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message}}).to_string()
}

#[cfg(test)]
pub(super) mod tests;

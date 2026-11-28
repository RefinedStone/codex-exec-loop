use anyhow::Result;
use serde_json::{Value, json};

use super::{GET, PREPARE, respond};
use crate::application::port::inbound::conversation_workspace_port::ConversationWorkspacePort;
use crate::domain::conversation_workspace::{
    ConversationWorkspaceQuery, ConversationWorkspaceRecord, ConversationWorkspaceState,
    PrepareConversationWorkspaceRequest,
};

pub(crate) struct EchoWorkspace;

impl ConversationWorkspacePort for EchoWorkspace {
    fn prepare(
        &self,
        request: PrepareConversationWorkspaceRequest,
    ) -> Result<ConversationWorkspaceRecord> {
        Ok(ConversationWorkspaceRecord {
            request_id: request.request_id,
            repository_dir: request.cwd,
            workspace_dir: "/prepared".into(),
            branch: "prepared-branch".into(),
            base_commit: request.base_commit,
            state: ConversationWorkspaceState::Prepared,
            reason: None,
        })
    }
    fn get(
        &self,
        _query: ConversationWorkspaceQuery,
    ) -> Result<Option<ConversationWorkspaceRecord>> {
        Ok(None)
    }
}

fn response(method: &str, params: Value) -> Value {
    serde_json::from_str(&respond(
        &EchoWorkspace,
        json!(7),
        method,
        &json!({"params":params}).to_string(),
    ))
    .unwrap()
}

#[test]
fn conversation_workspace_api_has_only_prepare_and_get_and_rejects_binding_fields() {
    let request = json!({"cwd":"/repo", "requestId":"one", "baseCommit":"a".repeat(40)});
    let prepared = response(PREPARE, request.clone());
    assert_eq!(prepared["result"]["state"], "prepared");
    assert_eq!(prepared["result"]["requestId"], "one");
    assert!(prepared["result"].get("threadId").is_none());
    let mut binding = request;
    binding["threadId"] = json!("unverified-thread");
    assert_eq!(response(PREPARE, binding)["error"]["code"], -32602);
    assert_eq!(
        response("akra/conversationWorkspace/bind", json!({}))["error"]["code"],
        -32601
    );
    assert_eq!(
        response(GET, json!({"cwd":"/repo", "requestId":"one"}))["result"],
        Value::Null
    );
}
mod desktop_fixture;

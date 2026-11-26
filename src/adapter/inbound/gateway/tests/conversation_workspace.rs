use std::sync::Arc;

use serde_json::json;

use super::{connect_with_services, kernel_request};
use crate::adapter::inbound::gateway::conversation_workspace::tests::EchoWorkspace;
use crate::adapter::inbound::gateway::session::GatewayServices;

#[tokio::test]
async fn conversation_workspace_requests_are_local_and_never_reach_remote_upstream() {
    let services = GatewayServices {
        conversation_workspaces: Some(Arc::new(EchoWorkspace)),
        ..Default::default()
    };
    let mut local = connect_with_services("/", services.clone()).await;
    let params = json!({"cwd":"/repo", "requestId":"same-request", "baseCommit":"a".repeat(40)});
    let prepared = kernel_request(
        &mut local,
        "prepare",
        "akra/conversationWorkspace/prepare",
        params.clone(),
    )
    .await;
    assert_eq!(prepared["result"]["requestId"], "same-request");
    let mut remote = connect_with_services("/host/cloud", services).await;
    let refused = kernel_request(
        &mut remote,
        "prepare",
        "akra/conversationWorkspace/prepare",
        params,
    )
    .await;
    assert_eq!(refused["error"]["code"], -32601);
}

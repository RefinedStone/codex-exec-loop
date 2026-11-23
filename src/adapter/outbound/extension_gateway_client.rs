//! Extension CLI uses the same attached Gateway authority as Desktop. No local fallback.
use anyhow::{Context, Result, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

pub(crate) fn request(scope: &str, workspace: &str, call: Option<Value>) -> Result<Value> {
    let endpoint = crate::gateway_endpoint::client_endpoint()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let connected = tokio::time::timeout(
            Duration::from_secs(5),
            crate::adapter::outbound::gateway_kernel_client::connect(&endpoint),
        )
        .await
        .context("extension Gateway connection timed out")?
        .context("start akra-gateway before using extensions")?;
        let (mut socket, _) =
            tokio_tungstenite::client_async("ws://localhost/rpc", connected).await?;
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            request_over_socket(&mut socket, scope, workspace, call),
        )
        .await
        .context("extension Gateway request timed out")?;
        let _ = socket.close(None).await;
        result
    })
}
pub(crate) async fn request_over_socket<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    scope: &str,
    workspace: &str,
    call: Option<Value>,
) -> Result<Value> {
    let (method, params) = match scope {
        "local_user" => (
            "akra/kernel/extensions/attach",
            json!({"scope":"local_user","client_name":"akra-extensions-cli"}),
        ),
        "repository" => (
            "akra/kernel/attach",
            json!({"workspace":workspace,"client_name":"akra-extensions-cli","transient":true}),
        ),
        _ => bail!("scope must be local_user or repository"),
    };
    let attached = exchange(socket, 1, method, params).await?;
    let kernel = attached["kernel"]
        .as_str()
        .context("Gateway omitted extension scope")?;
    let catalog = exchange(
        socket,
        2,
        "akra/kernel/extensions/list",
        json!({"kernel":kernel}),
    )
    .await?;
    let Some(mut call) = call else {
        return Ok(catalog);
    };
    validate_catalog_call(&catalog, &call)?;
    call["kernel"] = json!(kernel);
    exchange(socket, 3, "akra/kernel/extensions/call", call).await
}
fn validate_catalog_call(catalog: &Value, call: &Value) -> Result<()> {
    let extension = call["extension"].as_str().context("missing extension ID")?;
    let version = call["version"]
        .as_u64()
        .context("missing extension version")?;
    let action = call["action"]
        .as_str()
        .context("missing extension action")?;
    let entry = catalog
        .as_array()
        .context("invalid extension catalog")?
        .iter()
        .find(|entry| entry["id"] == extension && entry["version"] == version)
        .context("extension or version is not installed")?;
    let operation = entry["operations"]
        .as_array()
        .context("extension has no registered machine schema")?
        .iter()
        .find(|operation| operation["name"] == action)
        .context("extension action is not registered")?;
    let validator = jsonschema::validator_for(&operation["input_schema"])?;
    ensure!(
        validator.is_valid(&call["input"]),
        "extension input does not match the catalog schema"
    );
    Ok(())
}
async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value> {
    socket
        .send(Message::text(
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string(),
        ))
        .await?;
    while let Some(message) = socket.next().await {
        match message? {
            Message::Text(text) => {
                let message: Value = serde_json::from_str(&text)?;
                if message["id"] != id {
                    continue;
                }
                if !message["error"].is_null() {
                    bail!("extension request rejected: {}", message["error"]);
                }
                return Ok(message["result"].clone());
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    bail!("Gateway closed before the extension response")
}

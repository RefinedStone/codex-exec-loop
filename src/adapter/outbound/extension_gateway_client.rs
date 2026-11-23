//! Extension CLI uses the same attached Gateway authority as Desktop. No local fallback.
use crate::domain::kernel_extension::ExtensionCallRejection;
use anyhow::{Context, Result, bail, ensure};
#[cfg(test)]
mod tests;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

pub(crate) fn request(
    scope: &str,
    workspace: &str,
    call: Option<Value>,
    thread_id: Option<&str>,
) -> Result<Value> {
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
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            if let Some(thread) = thread_id {
                verify_thread(&mut socket, thread).await?;
            }
            request_over_socket(&mut socket, scope, workspace, call).await
        })
        .await
        .context("extension Gateway request timed out")?;
        let _ = socket.close(None).await;
        result
    })
}
/// The upstream server must resolve the thread before scoped extension queries are admitted.
/// Resuming with excludeTurns reads metadata only and never starts or steers a model turn.
async fn verify_thread<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    thread_id: &str,
) -> Result<()> {
    ensure!(
        !thread_id.is_empty() && thread_id.len() <= 256,
        "invalid thread identifier"
    );
    exchange(socket,4,"initialize",json!({"clientInfo":{"name":"akra-extensions-cli","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
    socket
        .send(Message::text(
            json!({"jsonrpc":"2.0","method":"initialized","params":{}}).to_string(),
        ))
        .await?;
    let resumed = exchange(
        socket,
        5,
        "thread/resume",
        json!({"threadId":thread_id,"excludeTurns":true}),
    )
    .await?;
    ensure!(
        resumed.pointer("/thread/id").and_then(Value::as_str) == Some(thread_id),
        "server did not verify the requested conversation"
    );
    Ok(())
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
    let preflight = validate_catalog_call(&catalog, &call);
    call["kernel"] = json!(kernel);
    // The server owns rejection reasons even when this client's catalog check also rejects.
    // It rechecks current schemas and authority before any handler is dispatched.
    let answer = exchange(socket, 3, "akra/kernel/extensions/call", call).await?;
    if let Err(error) = preflight {
        return Err(ExtensionCallRejection {
            code: -32603,
            message: "extension catalog and call validation disagree".into(),
            data: Some(json!({
                "reason": "extension_catalog_mismatch",
                "source": "client",
                "validation": error.to_string()
            })),
        }
        .into());
    }
    Ok(answer)
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
                    let rejection: ExtensionCallRejection =
                        serde_json::from_value(message["error"].clone())
                            .context("Gateway returned an invalid extension rejection")?;
                    return Err(rejection.into());
                }
                return Ok(message["result"].clone());
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    bail!("Gateway closed before the extension response")
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    #[tokio::test]
    async fn storage_cli_thread_binding_uses_metadata_and_rejects_a_different_server_thread() {
        for matched in [true, false] {
            let (client, server) = tokio::io::duplex(16 * 1024);
            let server = tokio::spawn(async move {
                let mut socket = tokio_tungstenite::accept_async(server).await.unwrap();
                let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                    panic!("initialize")
                };
                let initialize: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(initialize["method"], "initialize");
                socket
                    .send(Message::text(
                        json!({"id":initialize["id"],"result":{}}).to_string(),
                    ))
                    .await
                    .unwrap();
                let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                    panic!("initialized")
                };
                let initialized: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(initialized["method"], "initialized");
                let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                    panic!("resume")
                };
                let resume: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(resume["method"], "thread/resume");
                assert_eq!(
                    resume["params"],
                    json!({"threadId":"verified-thread","excludeTurns":true})
                );
                socket
                    .send(Message::text(
                        json!({"id":resume["id"],"result":{"thread":{
                            "id":if matched {"verified-thread"}else{"wrong-thread"}
                        }}})
                        .to_string(),
                    ))
                    .await
                    .unwrap();
            });
            let (mut socket, _) = tokio_tungstenite::client_async("ws://localhost/rpc", client)
                .await
                .unwrap();
            assert_eq!(
                verify_thread(&mut socket, "verified-thread").await.is_ok(),
                matched
            );
            server.await.unwrap();
        }
    }
}

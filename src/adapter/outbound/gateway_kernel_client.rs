//! JSON-only Gateway transport. Protocol conversion stays in composition.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

use crate::application::port::outbound::kernel_gateway_port::{
    KernelGatewayPort, KernelGatewayReply,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

pub(crate) struct GatewayKernelClientAdapter {
    workspace: String,
    client_name: String,
}

impl GatewayKernelClientAdapter {
    pub(crate) fn new(workspace: String, client_name: String) -> Self {
        Self {
            workspace,
            client_name,
        }
    }
}

impl KernelGatewayPort for GatewayKernelClientAdapter {
    fn request(&self, method: &str, params: Value, wait: bool) -> Result<KernelGatewayReply> {
        let endpoint = crate::gateway_endpoint::client_endpoint()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let connected = tokio::time::timeout(CONNECT_TIMEOUT, async {
                let stream = connect(&endpoint).await.ok()?;
                tokio_tungstenite::client_async("ws://localhost/rpc", stream)
                    .await
                    .ok()
            })
            .await;
            let Ok(Some((mut socket, _))) = connected else {
                return Ok(KernelGatewayReply::Unavailable);
            };
            let result = tokio::time::timeout(
                REQUEST_TIMEOUT,
                request_over_socket(
                    &mut socket,
                    &self.workspace,
                    &self.client_name,
                    method,
                    params,
                    wait,
                ),
            )
            .await
            .context("the Gateway command or event wait timed out")?;
            let _ = socket.close(None).await;
            result
        })
    }
}

/// Exposed within the crate so fake-wire clients can exercise the complete transport contract.
pub(crate) async fn request_over_socket<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    workspace: &str,
    client_name: &str,
    method: &str,
    params: Value,
    wait: bool,
) -> Result<KernelGatewayReply> {
    send(
        socket,
        1,
        "akra/kernel/attach",
        json!({
            "workspace": workspace, "client_name": client_name, "transient": true
        }),
    )
    .await?;
    let mut recorded = false;
    let attached = response(socket, 1, &mut recorded).await?;
    // Attach failed before any command ran. The locked local path checks the owner.
    if attached.pointer("/error/code").and_then(Value::as_i64) == Some(-32012) {
        return Ok(KernelGatewayReply::Unavailable);
    }
    answer(attached)?;
    let mut id = 2;
    loop {
        // Attach has subscribed before the first query. Keep notifications received
        // while awaiting the response so a completion cannot be missed.
        recorded = false;
        send(socket, id, method, params.clone()).await?;
        let value = answer(response(socket, id, &mut recorded).await?)?;
        if !wait || query_finished(&value)? {
            send(socket, id + 1, "akra/kernel/detach", json!({})).await?;
            answer(response(socket, id + 1, &mut recorded).await?)?;
            return Ok(KernelGatewayReply::Answer(value));
        }
        if !recorded {
            next_record(socket).await?;
        }
        id += 1;
    }
}

async fn send<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    id: i64,
    method: &str,
    params: Value,
) -> Result<()> {
    socket
        .send(Message::text(
            json!({
                "jsonrpc": "2.0", "id": id, "method": method, "params": params
            })
            .to_string(),
        ))
        .await?;
    Ok(())
}

async fn next_json<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
) -> Result<Value> {
    while let Some(message) = socket.next().await {
        match message.context("the Gateway connection failed")? {
            Message::Text(text) => return Ok(serde_json::from_str(text.as_str())?),
            Message::Close(_) => break,
            _ => {}
        }
    }
    bail!("the Gateway closed the connection before it answered")
}

fn is_record_notification(value: &Value) -> bool {
    value.get("method").and_then(Value::as_str) == Some("akra/kernel/event")
        && matches!(
            value.pointer("/params/event/type").and_then(Value::as_str),
            Some("recorded" | "resync")
        )
}

async fn response<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    id: i64,
    recorded: &mut bool,
) -> Result<Value> {
    loop {
        let value = next_json(socket).await?;
        if value.get("id").and_then(Value::as_i64) == Some(id) && value.get("method").is_none() {
            return Ok(value);
        }
        *recorded |= is_record_notification(&value);
    }
}

async fn next_record<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
) -> Result<()> {
    loop {
        if is_record_notification(&next_json(socket).await?) {
            return Ok(());
        }
    }
}

fn answer(value: Value) -> Result<Value> {
    if let Some(error) = value.get("error") {
        bail!("Gateway kernel error: {error}");
    }
    value
        .get("result")
        .cloned()
        .context("the Gateway response has no result")
}

fn query_finished(value: &Value) -> Result<bool> {
    match value.get("type").and_then(Value::as_str) {
        Some("history") => Ok(value
            .get("records")
            .and_then(Value::as_array)
            .is_some_and(|records| !records.is_empty())),
        Some("operation") => Ok(!matches!(
            value.get("state").and_then(Value::as_str),
            Some("accepted" | "running")
        )),
        _ => bail!("--wait requires a history or operation query"),
    }
}

#[cfg(unix)]
async fn connect(endpoint: &str) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(endpoint).await
}

#[cfg(windows)]
async fn connect(
    endpoint: &str,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_end_only_at_records_or_terminal_operations() {
        assert!(!query_finished(&json!({"type":"history","records":[]})).unwrap());
        assert!(query_finished(&json!({"type":"history","records":[{}]})).unwrap());
        for state in ["accepted", "running"] {
            assert!(!query_finished(&json!({"type":"operation","state":state})).unwrap());
        }
        for state in ["done", "failed", "rejected", "lost", "unknown"] {
            assert!(query_finished(&json!({"type":"operation","state":state})).unwrap());
        }
        assert!(query_finished(&json!({"type":"planning"})).is_err());
    }

    #[test]
    fn only_recorded_and_resync_events_wake_waiters() {
        for kind in ["recorded", "resync"] {
            assert!(is_record_notification(&json!({
                "method":"akra/kernel/event","params":{"event":{"type":kind}}
            })));
        }
        assert!(!is_record_notification(&json!({
            "method":"akra/kernel/event","params":{"event":{"type":"thread_activity"}}
        })));
    }

    #[tokio::test]
    async fn event_before_query_reply_is_not_lost_and_terminal_reply_detaches() {
        use tokio_tungstenite::tungstenite::protocol::Role;
        let (client, server) = tokio::io::duplex(8192);
        let mut client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
        let mut server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
        let serving = async {
            let attach = next_json(&mut server).await.unwrap();
            assert_eq!(attach["params"]["transient"], true);
            server
                .send(Message::text(
                    json!({"jsonrpc":"2.0","id":1,"result":{}}).to_string(),
                ))
                .await
                .unwrap();
            let query = next_json(&mut server).await.unwrap();
            assert_eq!(query["id"], 2);
            server.send(Message::text(json!({
                "jsonrpc":"2.0","method":"akra/kernel/event","params":{"event":{"type":"recorded"}}
            }).to_string())).await.unwrap();
            server
                .send(Message::text(
                    json!({
                        "jsonrpc":"2.0","id":2,"result":{"type":"operation","state":"running"}
                    })
                    .to_string(),
                ))
                .await
                .unwrap();
            let query = next_json(&mut server).await.unwrap();
            assert_eq!(
                query["id"], 3,
                "the earlier notification triggers a requery without waiting"
            );
            server
                .send(Message::text(
                    json!({
                        "jsonrpc":"2.0","id":3,"result":{"type":"operation","state":"done"}
                    })
                    .to_string(),
                ))
                .await
                .unwrap();
            let detach = next_json(&mut server).await.unwrap();
            assert_eq!(detach["method"], "akra/kernel/detach");
            server
                .send(Message::text(
                    json!({"jsonrpc":"2.0","id":4,"result":{}}).to_string(),
                ))
                .await
                .unwrap();
        };
        let requesting = request_over_socket(
            &mut client,
            "/workspace",
            "test",
            "akra/kernel/query",
            json!({"query":{"type":"operation","op_id":1}}),
            true,
        );
        let (reply, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(requesting, serving)
        })
        .await
        .expect("event-driven fake wire test must finish");
        let KernelGatewayReply::Answer(answer) = reply.unwrap() else {
            panic!("Gateway exists");
        };
        assert_eq!(answer["state"], "done");
    }
}

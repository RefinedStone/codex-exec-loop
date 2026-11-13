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
use crate::domain::kernel_spec::KernelCallError;

/// The Gateway cannot attach to the kernel of the repository (`kernel_unavailable` in the spec).
const ERROR_KERNEL_UNAVAILABLE: i64 = -32012;
/// JSON-RPC: the Gateway does not know the method, or it does not accept the fields of the request.
const ERROR_METHOD_NOT_FOUND: i64 = -32601;
const ERROR_INVALID_PARAMS: i64 = -32602;

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
        /*
         * The request runs on its own thread with its own runtime. An entrance may call this port from
         * inside an async runtime (the admin server handles a reset in an async handler), and a runtime
         * cannot be started on a thread that already drives one. The caller still waits for the answer.
         */
        std::thread::scope(|scope| {
            scope
                .spawn(|| self.request_on_this_thread(method, params, wait))
                .join()
                .unwrap_or_else(|_| bail!("the Gateway request thread panicked"))
        })
    }
}

impl GatewayKernelClientAdapter {
    fn request_on_this_thread(
        &self,
        method: &str,
        params: Value,
        wait: bool,
    ) -> Result<KernelGatewayReply> {
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
                return Ok(KernelGatewayReply::NoGateway);
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
    // Attach failed before any command ran: another process runs the kernel of this repository.
    // The locked local path checks the owner.
    if attached.pointer("/error/code").and_then(Value::as_i64) == Some(ERROR_KERNEL_UNAVAILABLE) {
        return Ok(KernelGatewayReply::KernelHeldElsewhere);
    }
    /*
     * The attach of this client is valid for a Gateway of the same version. A Gateway that refuses its
     * shape is an older program (it does not know `transient`, or it has no kernel methods). Nothing ran.
     */
    if matches!(
        attached.pointer("/error/code").and_then(Value::as_i64),
        Some(ERROR_INVALID_PARAMS | ERROR_METHOD_NOT_FOUND)
    ) {
        return Ok(KernelGatewayReply::GatewayTooOld);
    }
    answer(attached)?;
    let mut id = 2;
    loop {
        // Attach has subscribed before the first query. Keep notifications received
        // while awaiting the response so a completion cannot be missed.
        recorded = false;
        send(socket, id, method, params.clone()).await?;
        let reply = response(socket, id, &mut recorded).await?;
        /*
         * The caller checked the request against its own specification before it sent it. A Gateway that
         * refuses the shape of that request does not know it: it is an older program. It refused before
         * it ran anything, so the caller may run a stored command under the repository lock.
         */
        if matches!(
            reply.pointer("/error/code").and_then(Value::as_i64),
            Some(ERROR_INVALID_PARAMS | ERROR_METHOD_NOT_FOUND)
        ) {
            return Ok(KernelGatewayReply::GatewayTooOld);
        }
        let value = answer(reply)?;
        if !wait || query_finished(&value)? {
            // The kernel already answered. A failed detach must not turn that answer into an error:
            // the caller would send the command again. Closing the socket detaches too.
            if send(socket, id + 1, "akra/kernel/detach", json!({}))
                .await
                .is_ok()
            {
                let _ = response(socket, id + 1, &mut recorded).await;
            }
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
        // Keep the code: the caller finds the next steps of the error in the specification.
        return Err(KernelCallError {
            code: error
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the Gateway answered with an error")
                .to_string(),
        }
        .into());
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

    type Wire = WebSocketStream<tokio::io::DuplexStream>;

    async fn wire() -> (Wire, Wire) {
        use tokio_tungstenite::tungstenite::protocol::Role;
        let (client, server) = tokio::io::duplex(8192);
        (
            WebSocketStream::from_raw_socket(client, Role::Client, None).await,
            WebSocketStream::from_raw_socket(server, Role::Server, None).await,
        )
    }

    async fn serve(server: &mut Wire, expected_method: &str, body: Value) {
        let request = next_json(server).await.unwrap();
        assert_eq!(request["method"], expected_method);
        let mut message = json!({"jsonrpc":"2.0","id":request["id"]});
        message
            .as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        server
            .send(Message::text(message.to_string()))
            .await
            .unwrap();
    }

    async fn command_over(client: &mut Wire) -> Result<KernelGatewayReply> {
        request_over_socket(
            client,
            "/workspace",
            "test",
            "akra/kernel/command",
            json!({"command":{"type":"stop_all"}}),
            false,
        )
        .await
    }

    #[tokio::test]
    async fn a_kernel_that_another_process_runs_is_not_a_missing_gateway() {
        // The Gateway is up. The caller must not be told to start it.
        let (mut client, mut server) = wire().await;
        let serving = serve(
            &mut server,
            "akra/kernel/attach",
            json!({"error":{"code":ERROR_KERNEL_UNAVAILABLE,"message":"held"}}),
        );
        let (reply, ()) = tokio::join!(command_over(&mut client), serving);
        assert!(matches!(
            reply.unwrap(),
            KernelGatewayReply::KernelHeldElsewhere
        ));
    }

    #[tokio::test]
    async fn a_gateway_that_refuses_the_attach_shape_is_an_older_gateway() {
        // An older Gateway keeps running after an upgrade of the CLI. It must not look like a broken request.
        for (code, message) in [
            (ERROR_INVALID_PARAMS, "unknown field `transient`"),
            (ERROR_METHOD_NOT_FOUND, "unknown method"),
        ] {
            let (mut client, mut server) = wire().await;
            let serving = serve(
                &mut server,
                "akra/kernel/attach",
                json!({"error":{"code":code,"message":message}}),
            );
            let (reply, ()) = tokio::join!(command_over(&mut client), serving);
            assert!(
                matches!(reply.unwrap(), KernelGatewayReply::GatewayTooOld),
                "{code}"
            );
        }
    }

    #[tokio::test]
    async fn a_gateway_that_does_not_know_the_command_is_an_older_gateway() {
        // plan 18 K4-9d: a Gateway of the last version attaches a transient client but refuses a new command.
        let (mut client, mut server) = wire().await;
        let serving = async {
            serve(&mut server, "akra/kernel/attach", json!({"result":{}})).await;
            serve(
                &mut server,
                "akra/kernel/command",
                json!({"error":{"code":ERROR_INVALID_PARAMS,"message":"unknown variant `create_task`"}}),
            )
            .await;
        };
        let (reply, ()) = tokio::join!(command_over(&mut client), serving);
        assert!(matches!(reply.unwrap(), KernelGatewayReply::GatewayTooOld));
    }

    #[tokio::test]
    async fn a_kernel_error_keeps_its_code_and_a_failed_detach_keeps_the_answer() {
        let (mut client, mut server) = wire().await;
        let serving = async {
            serve(&mut server, "akra/kernel/attach", json!({"result":{}})).await;
            serve(
                &mut server,
                "akra/kernel/command",
                json!({"error":{"code":-32014,"message":"key reused"}}),
            )
            .await;
        };
        let (reply, ()) = tokio::join!(command_over(&mut client), serving);
        let error = reply.err().expect("a kernel error");
        assert_eq!(
            error.downcast_ref::<KernelCallError>(),
            Some(&KernelCallError {
                code: -32014,
                message: "key reused".to_string()
            })
        );

        // The server answers the command and then closes without answering the detach.
        let (mut client, mut server) = wire().await;
        let serving = async {
            serve(&mut server, "akra/kernel/attach", json!({"result":{}})).await;
            serve(
                &mut server,
                "akra/kernel/command",
                json!({"result":{"processed":true}}),
            )
            .await;
            drop(server);
        };
        let (reply, ()) = tokio::join!(command_over(&mut client), serving);
        let KernelGatewayReply::Answer(answer) = reply.unwrap() else {
            panic!("the kernel answered");
        };
        assert_eq!(answer["processed"], true);
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

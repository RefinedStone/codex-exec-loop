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
    request_timeout: Duration,
}

impl GatewayKernelClientAdapter {
    pub(crate) fn new(workspace: String, client_name: String) -> Self {
        Self {
            workspace,
            client_name,
            request_timeout: REQUEST_TIMEOUT,
        }
    }

    /// A startup probe must not use the long-running command timeout.
    pub(crate) fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
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
            let delegation_wait =
                wait && params.pointer("/query/type").and_then(Value::as_str) == Some("delegation");
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
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
            let result = tokio::time::timeout_at(
                if delegation_wait {
                    deadline
                } else {
                    tokio::time::Instant::now() + self.request_timeout
                },
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
         * A Gateway that does not know the method or the name of the command or query is an older program.
         * It refused before it ran anything, so the caller may run a stored command under the repository
         * lock. Another refusal of the parameters (a blank value, a wrong type) is an error of the request,
         * and the caller gets its code and text.
         */
        if refuses_an_unknown_name(&reply) {
            return Ok(KernelGatewayReply::GatewayTooOld);
        }
        let value = answer(reply)?;
        if wait && params.pointer("/query/type").and_then(Value::as_str) == Some("delegation") {
            if matches!(
                value.pointer("/delegation/state").and_then(Value::as_str),
                Some("queued" | "running" | "dispatching" | "interrupted")
            ) {
                // The delegation ledger can change independently of kernel log notifications.
                // Reuse this attached connection until the first change, without issuing a command.
                let changed = tokio::time::timeout(std::time::Duration::from_secs(9), async {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        id += 1;
                        send(socket, id, method, params.clone()).await?;
                        let latest = answer(response(socket, id, &mut recorded).await?)?;
                        if latest != value {
                            return Ok::<_, anyhow::Error>(latest);
                        }
                    }
                })
                .await;
                return Ok(KernelGatewayReply::Answer(match changed {
                    Ok(result) => result?,
                    Err(_) => value,
                }));
            }
            return Ok(KernelGatewayReply::Answer(value));
        }
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

/// The current values a kernel client keeps: the query answers and the events that carry the same value.
const WATCHED_VALUES: [&str; 3] = ["sessions", "auto_follow", "thread_activity"];

const WATCH_SYNC_TIMEOUT: Duration = Duration::from_secs(15);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// An update of a client that stays attached to the Gateway kernel.
pub(crate) enum KernelValueUpdate {
    Value(Value),
    /// Attach and all three initial queries have answered.
    Synchronized,
    Disconnected(String),
}

fn forward_watch_event(
    message: &Value,
    forward: &mut impl FnMut(KernelValueUpdate) -> bool,
) -> Result<bool> {
    if message.get("method").and_then(Value::as_str) != Some("akra/kernel/event") {
        return Ok(false);
    }
    let Some(value) = message.pointer("/params/event") else {
        return Ok(false);
    };
    match value.get("type").and_then(Value::as_str) {
        Some("resync") => Ok(true),
        Some(kind) if WATCHED_VALUES.contains(&kind) => {
            if !forward(KernelValueUpdate::Value(value.clone())) {
                bail!("the kernel watcher receiver closed");
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

// Match the exact response, forwarding live events even during synchronization.
async fn watch_request<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    id: i64,
    method: &str,
    params: Value,
    forward: &mut impl FnMut(KernelValueUpdate) -> bool,
    resync: &mut bool,
) -> Result<Value> {
    send(socket, id, method, params).await?;
    loop {
        let message = next_json(socket).await?;
        if message.get("method").is_none() && message.get("id").and_then(Value::as_i64) == Some(id)
        {
            return answer(message);
        }
        *resync |= forward_watch_event(&message, forward)?;
    }
}

async fn synchronize_watch<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    id: &mut i64,
    load_sessions: bool,
    forward: &mut impl FnMut(KernelValueUpdate) -> bool,
) -> Result<bool> {
    let mut resync = false;
    if load_sessions {
        *id += 1;
        let reply = watch_request(
            socket,
            *id,
            "akra/kernel/command",
            json!({"command":{"type":"load_sessions"}}),
            forward,
            &mut resync,
        )
        .await?;
        if reply.get("processed").and_then(Value::as_bool) != Some(true)
            || reply.get("rejection").is_some_and(|v| !v.is_null())
            || reply.get("failure").is_some_and(|v| !v.is_null())
        {
            bail!("the Gateway did not accept the session catalog load");
        }
    }
    for kind in WATCHED_VALUES {
        *id += 1;
        let value = watch_request(
            socket,
            *id,
            "akra/kernel/query",
            json!({"query":{"type":kind}}),
            forward,
            &mut resync,
        )
        .await?;
        if value.get("type").and_then(Value::as_str) != Some(kind) {
            bail!("the Gateway answered the wrong kernel query");
        }
        if !forward(KernelValueUpdate::Value(value)) {
            bail!("the kernel watcher receiver closed");
        }
    }
    Ok(resync)
}

/// Read-only client: bounded handshake/initial sync/re-sync, but no idle event deadline.
async fn watch_kernel_values<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    workspace: &str,
    client_name: &str,
    refresh: &mut tokio::sync::mpsc::Receiver<()>,
    sync_timeout: Duration,
    mut forward: impl FnMut(KernelValueUpdate) -> bool,
) -> Result<()> {
    let mut id = 1;
    let mut resync = false;
    tokio::time::timeout(sync_timeout, async {
        watch_request(
            socket,
            id,
            "akra/kernel/attach",
            json!({"workspace":workspace,"client_name":client_name}),
            &mut forward,
            &mut resync,
        )
        .await?;
        resync |= synchronize_watch(socket, &mut id, true, &mut forward).await?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("Gateway kernel attach or initial synchronization timed out")??;
    if !forward(KernelValueUpdate::Synchronized) {
        return Ok(());
    }
    loop {
        let load = if resync {
            false
        } else {
            tokio::select! {
                message = next_json(socket) => {
                    resync = forward_watch_event(&message?, &mut forward)?;
                    continue;
                }
                requested = refresh.recv() => {
                    if requested.is_none() { return Ok(()); }
                    true
                }
            }
        };
        resync = tokio::time::timeout(
            sync_timeout,
            synchronize_watch(socket, &mut id, load, &mut forward),
        )
        .await
        .context("Gateway kernel synchronization timed out")??;
    }
}

/// Start the watcher and return the bounded refresh mailbox. Dropping it also
/// terminates a healthy idle watcher; the initial/sync waits have deadlines.
pub(crate) fn spawn_kernel_value_watcher(
    workspace: String,
    client_name: &'static str,
    updates: std::sync::mpsc::Sender<KernelValueUpdate>,
) -> tokio::sync::mpsc::Sender<()> {
    let (refresh, mut requests) = tokio::sync::mpsc::channel(1);
    std::thread::Builder::new()
        .name("akra-kernel-value-watcher".to_string())
        .spawn(move || {
            while !requests.is_closed() {
                let result =
                    watch_kernel_values_once(&workspace, client_name, &mut requests, |update| {
                        updates.send(update).is_ok()
                    });
                if requests.is_closed() {
                    return;
                }
                let reason = match result {
                    Ok(()) => "the Gateway closed the connection".to_string(),
                    Err(error) => format!("{error:#}"),
                };
                if updates
                    .send(KernelValueUpdate::Disconnected(reason))
                    .is_err()
                {
                    return;
                }
                std::thread::sleep(RECONNECT_DELAY);
            }
        })
        .expect("the kernel value watcher thread should start");
    refresh
}

fn watch_kernel_values_once(
    workspace: &str,
    client_name: &str,
    refresh: &mut tokio::sync::mpsc::Receiver<()>,
    forward: impl FnMut(KernelValueUpdate) -> bool,
) -> Result<()> {
    let endpoint = crate::gateway_endpoint::client_endpoint()?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let (mut socket, _) = tokio::time::timeout(CONNECT_TIMEOUT, async {
                let stream = connect(&endpoint).await?;
                Ok::<_, anyhow::Error>(
                    tokio_tungstenite::client_async("ws://localhost/rpc", stream).await?,
                )
            })
            .await
            .context("Gateway connection or WebSocket handshake timed out")??;
            watch_kernel_values(
                &mut socket,
                workspace,
                client_name,
                refresh,
                WATCH_SYNC_TIMEOUT,
                forward,
            )
            .await
        })
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

/// The reply refuses a method, a command or query name, or a field that the Gateway does not know.
fn refuses_an_unknown_name(reply: &Value) -> bool {
    match reply.pointer("/error/code").and_then(Value::as_i64) {
        Some(ERROR_METHOD_NOT_FOUND) => true,
        /*
         * serde names an unknown `type` tag and an unknown field this way. Older programs cannot change
         * this text. The caller built the request from its own types, so a Gateway of the same version
         * knows every name in it.
         */
        Some(ERROR_INVALID_PARAMS) => reply
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| {
                message.contains("unknown variant") || message.contains("unknown field")
            }),
        _ => false,
    }
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
pub(crate) async fn connect(endpoint: &str) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(endpoint).await
}

#[cfg(windows)]
pub(crate) async fn connect(
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

    async fn notify(server: &mut Wire, event: Value) {
        let message =
            json!({"jsonrpc":"2.0","method":"akra/kernel/event","params":{"event":event}});
        server
            .send(Message::text(message.to_string()))
            .await
            .unwrap();
    }

    async fn serve_watch_queries(server: &mut Wire, round: usize) {
        for expected in WATCHED_VALUES {
            let request = next_json(server).await.unwrap();
            assert_eq!(request["params"]["query"]["type"], expected);
            let result = if expected == "sessions" {
                json!({"type":expected,"round":round,"status":if round == 0 {"idle"} else {"ready"},"sessions":[]})
            } else {
                json!({"type":expected,"round":round})
            };
            server
                .send(Message::text(
                    json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string(),
                ))
                .await
                .unwrap();
        }
    }

    async fn serve_session_load(server: &mut Wire) {
        let request = next_json(server).await.unwrap();
        assert_eq!(request["method"], "akra/kernel/command");
        assert_eq!(
            request["params"]["command"],
            json!({"type":"load_sessions"})
        );
        server
            .send(Message::text(
                json!({"jsonrpc":"2.0","id":request["id"],"result":{"processed":true}}).to_string(),
            ))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_watching_client_loads_idle_sessions_refreshes_and_resyncs() {
        let (mut client, mut server) = wire().await;
        let (refresh, mut requests) = tokio::sync::mpsc::channel(1);
        let serving = async {
            let attach = next_json(&mut server).await.unwrap();
            assert_eq!(attach["params"]["client_name"], "akra-tui");
            assert_eq!(attach["params"].get("transient"), None);
            server
                .send(Message::text(
                    json!({"jsonrpc":"2.0","id":attach["id"],"result":{}}).to_string(),
                ))
                .await
                .unwrap();
            serve_session_load(&mut server).await;
            serve_watch_queries(&mut server, 0).await;
            notify(
                &mut server,
                json!({"type":"sessions","status":"ready","sessions":[]}),
            )
            .await;
            notify(&mut server, json!({"type":"planning"})).await;
            notify(&mut server, json!({"type":"resync"})).await;
            serve_watch_queries(&mut server, 1).await;
            refresh.send(()).await.unwrap();
            serve_session_load(&mut server).await;
            serve_watch_queries(&mut server, 2).await;
            drop(refresh);
        };
        let mut seen = Vec::new();
        let mut synchronized = 0;
        let watching = watch_kernel_values(
            &mut client,
            "/workspace",
            "akra-tui",
            &mut requests,
            Duration::from_secs(2),
            |update| {
                match update {
                    KernelValueUpdate::Value(value) => seen.push(value),
                    KernelValueUpdate::Synchronized => synchronized += 1,
                    KernelValueUpdate::Disconnected(_) => panic!("unexpected disconnect"),
                }
                true
            },
        );
        let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(watching, serving)
        })
        .await
        .unwrap();
        result.unwrap();
        assert_eq!(synchronized, 1);
        assert_eq!(seen.len(), 10);
        assert_eq!(seen[0]["status"], "idle");
        assert_eq!(seen[3]["status"], "ready");
        assert_eq!(seen[4]["round"], 1);
        assert_eq!(seen[7]["round"], 2);
    }

    #[tokio::test]
    async fn watching_client_times_out_when_attach_or_initial_query_is_unanswered() {
        for stall_after_attach in [false, true] {
            let (mut client, mut server) = wire().await;
            let (_refresh, mut requests) = tokio::sync::mpsc::channel(1);
            let serving = async {
                if stall_after_attach {
                    serve(&mut server, "akra/kernel/attach", json!({"result":{}})).await;
                    serve_session_load(&mut server).await;
                }
                let _ = next_json(&mut server).await.unwrap();
            };
            let watching = watch_kernel_values(
                &mut client,
                "/workspace",
                "akra-tui",
                &mut requests,
                Duration::from_millis(200),
                |update| {
                    assert!(!matches!(update, KernelValueUpdate::Synchronized));
                    true
                },
            );
            let (result, ()) = tokio::join!(watching, serving);
            assert!(format!("{:#}", result.unwrap_err()).contains("timed out"));
        }
    }

    #[tokio::test]
    async fn synchronized_idle_watcher_waits_without_timing_out_and_closes_when_dropped() {
        let (mut client, mut server) = wire().await;
        let (refresh, mut requests) = tokio::sync::mpsc::channel(1);
        let serving = async {
            serve(&mut server, "akra/kernel/attach", json!({"result":{}})).await;
            serve_session_load(&mut server).await;
            serve_watch_queries(&mut server, 0).await;
            tokio::time::sleep(Duration::from_millis(350)).await;
            drop(refresh);
        };
        let mut synchronized = false;
        let watching = watch_kernel_values(
            &mut client,
            "/workspace",
            "akra-tui",
            &mut requests,
            Duration::from_millis(200),
            |update| {
                synchronized |= matches!(update, KernelValueUpdate::Synchronized);
                true
            },
        );
        let (result, ()) = tokio::join!(watching, serving);
        result.unwrap();
        assert!(synchronized);
    }

    #[tokio::test]
    async fn delegation_wait_returns_on_state_change_without_a_second_submit() {
        let (mut client, mut server) = wire().await;
        let serving = async {
            serve(&mut server, "akra/kernel/attach", json!({"result":{}})).await;
            for state in ["queued", "running"] {
                serve(
                    &mut server,
                    "akra/kernel/query",
                    json!({
                        "result":{"type":"delegation","delegation":{"id":"job-1","state":state}}
                    }),
                )
                .await;
            }
        };
        let (reply, ()) = tokio::join!(
            request_over_socket(
                &mut client,
                "/workspace",
                "test",
                "akra/kernel/query",
                json!({"query":{"type":"delegation","id":"job-1"}}),
                true
            ),
            serving
        );
        assert!(matches!(reply.unwrap(), KernelGatewayReply::Answer(value)
            if value["delegation"]["state"] == "running"));
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
        // An older Gateway that knows the command also refuses a field that it does not know.
        assert!(refuses_an_unknown_name(&json!({
            "error":{"code":ERROR_INVALID_PARAMS,"message":"unknown field `labels`"}
        })));
    }

    #[tokio::test]
    async fn a_refused_value_of_a_known_command_keeps_its_error() {
        // The Gateway knows the command and refuses a value. That is not an older Gateway.
        let (mut client, mut server) = wire().await;
        let serving = async {
            serve(&mut server, "akra/kernel/attach", json!({"result":{}})).await;
            serve(
                &mut server,
                "akra/kernel/command",
                json!({"error":{"code":ERROR_INVALID_PARAMS,"message":"`task_id` must not be blank"}}),
            )
            .await;
        };
        let (reply, ()) = tokio::join!(command_over(&mut client), serving);
        let error = match reply {
            Err(error) => error,
            Ok(_) => panic!("a refused value is an error of the request"),
        };
        assert!(
            format!("{error:#}").contains("must not be blank"),
            "{error:#}"
        );
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

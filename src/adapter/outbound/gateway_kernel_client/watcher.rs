//! Watch Kernel values and forward them into the owning client's event channel.
use super::{CONNECT_TIMEOUT, answer, connect, next_json, send};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;

/// The current values a kernel client keeps: the query answers and the events that carry the same value.
pub(super) const WATCHED_VALUES: [&str; 3] = ["sessions", "auto_follow", "thread_activity"];

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
pub(super) async fn watch_kernel_values<S: AsyncRead + AsyncWrite + Unpin>(
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
pub(crate) fn spawn_kernel_value_watcher<T>(
    workspace: String,
    client_name: &'static str,
    updates: std::sync::mpsc::Sender<T>,
) -> Result<tokio::sync::mpsc::Sender<()>>
where
    T: From<KernelValueUpdate> + Send + 'static,
{
    let (refresh, mut requests) = tokio::sync::mpsc::channel(1);
    std::thread::Builder::new()
        .name("akra-kernel-value-watcher".to_string())
        .spawn(move || {
            while !requests.is_closed() {
                let result =
                    watch_kernel_values_once(&workspace, client_name, &mut requests, |update| {
                        updates.send(update.into()).is_ok()
                    });
                if requests.is_closed() {
                    return;
                }
                let reason = match result {
                    Ok(()) => "the Gateway closed the connection".to_string(),
                    Err(error) => format!("{error:#}"),
                };
                if updates
                    .send(KernelValueUpdate::Disconnected(reason).into())
                    .is_err()
                {
                    return;
                }
                std::thread::sleep(RECONNECT_DELAY);
            }
        })
        .context("failed to start the kernel value watcher thread")?;
    Ok(refresh)
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

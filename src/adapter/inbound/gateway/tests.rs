//! 가짜 upstream 으로 세션 중계를 시험한다. upstream 은 받은 텍스트 메시지를 `echo:` 를 붙여 되돌리는
//! WebSocket 서버이다. 실제 codex 는 실행하지 않는다.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::DuplexStream;
use tokio::sync::watch;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use super::GatewayCommand;
use super::hosts::GatewayHosts;
use super::parse_args;
use super::session::{run_session, websocket_config};
use crate::application::port::outbound::gateway_upstream_port::{
    GatewayUpstreamPort, GatewayUpstreamSnapshot, GatewayUpstreamStream,
};

const DUPLEX_BUFFER_BYTES: usize = 64 * 1024;
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

struct EchoUpstream {
    generation: watch::Sender<u64>,
    available: bool,
    opened: Mutex<u32>,
}

impl EchoUpstream {
    fn new(available: bool) -> Arc<Self> {
        Arc::new(Self {
            generation: watch::channel(0).0,
            available,
            opened: Mutex::new(0),
        })
    }

    fn restart(&self) {
        self.generation.send_modify(|generation| *generation += 1);
    }
}

impl GatewayUpstreamPort for EchoUpstream {
    fn open_session(&self) -> Result<Box<dyn GatewayUpstreamStream>> {
        if !self.available {
            bail!("upstream down")
        }
        *self.opened.lock().unwrap() += 1;
        let (gateway_side, upstream_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
        tokio::spawn(async move {
            let Ok(mut websocket) = tokio_tungstenite::accept_async_with_config(
                upstream_side,
                Some(websocket_config()),
            )
            .await
            else {
                return;
            };
            while let Some(Ok(message)) = websocket.next().await {
                match message {
                    Message::Text(text) => {
                        let reply = format!("echo:{}", text.as_str());
                        if websocket.send(Message::text(reply)).await.is_err() {
                            return;
                        }
                    }
                    Message::Close(_) => return,
                    _ => {}
                }
            }
        });
        Ok(Box::new(gateway_side))
    }

    fn snapshot(&self) -> GatewayUpstreamSnapshot {
        GatewayUpstreamSnapshot {
            codex_version: Some("codex-cli test".to_string()),
            started_at: "2026-11-05T00:00:00+00:00".to_string(),
            restarts: 0,
        }
    }

    fn generation(&self) -> watch::Receiver<u64> {
        self.generation.subscribe()
    }

    fn shutdown(&self) {}
}

async fn connect_client(upstream: Arc<EchoUpstream>) -> WebSocketStream<DuplexStream> {
    let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    tokio::spawn(async move {
        let _ = run_session(
            gateway_side,
            GatewayHosts::new(upstream, Vec::new()),
            Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default()),
            super::session::GatewayServices::default(),
            None,
        )
        .await;
    });
    let (websocket, _) = tokio_tungstenite::client_async_with_config(
        "ws://localhost/",
        client_side,
        Some(websocket_config()),
    )
    .await
    .expect("client handshake through the gateway session should succeed");
    websocket
}

async fn next_text(websocket: &mut WebSocketStream<DuplexStream>) -> String {
    loop {
        let message = tokio::time::timeout(TEST_TIMEOUT, websocket.next())
            .await
            .expect("message should arrive in time")
            .expect("stream should stay open")
            .expect("message should be valid");
        if let Message::Text(text) = message {
            return text.as_str().to_string();
        }
    }
}

async fn next_close_code(websocket: &mut WebSocketStream<DuplexStream>) -> Option<CloseCode> {
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, websocket.next())
            .await
            .expect("close should arrive in time")
        {
            Some(Ok(Message::Close(frame))) => return frame.map(|frame| frame.code),
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => return None,
        }
    }
}

#[tokio::test]
async fn app_server_messages_are_relayed_in_order() {
    let upstream = EchoUpstream::new(true);
    let mut client = connect_client(upstream.clone()).await;

    for index in 0..50 {
        let request = json!({"jsonrpc": "2.0", "id": index, "method": "thread/list"}).to_string();
        client.send(Message::text(request)).await.unwrap();
    }
    for index in 0..50 {
        let reply = next_text(&mut client).await;
        let body: Value = serde_json::from_str(reply.strip_prefix("echo:").unwrap()).unwrap();
        assert_eq!(body["id"], json!(index), "replies must keep request order");
    }
}

#[tokio::test]
async fn messages_larger_than_the_tungstenite_default_frame_are_relayed() {
    // A long thread's thread/resume response exceeded tungstenite's 16 MiB frame / 64 MiB message defaults
    // and silently closed the session (found by opening real threads in Akra Desktop).
    let upstream = EchoUpstream::new(true);
    let mut client = connect_client(upstream).await;
    let large = "x".repeat(20 << 20);
    client.send(Message::text(large.clone())).await.unwrap();
    let reply = next_text(&mut client).await;
    assert_eq!(reply.len(), "echo:".len() + large.len());
}

#[tokio::test]
async fn gateway_info_is_answered_locally_and_never_reaches_upstream() {
    let upstream = EchoUpstream::new(true);
    let mut client = connect_client(upstream.clone()).await;

    client
        .send(Message::text(
            r#"{"jsonrpc":"2.0","id":"info","method":"akra/gateway/info"}"#,
        ))
        .await
        .unwrap();
    let reply: Value = serde_json::from_str(&next_text(&mut client).await).unwrap();
    assert_eq!(reply["id"], json!("info"));
    assert_eq!(reply["result"]["codexVersion"], json!("codex-cli test"));
}

#[tokio::test]
async fn each_client_gets_its_own_upstream_connection() {
    let upstream = EchoUpstream::new(true);
    let mut first = connect_client(upstream.clone()).await;
    let mut second = connect_client(upstream.clone()).await;

    // Both clients reuse request id 1, as the app-server allows per connection.
    first
        .send(Message::text(r#"{"id":1,"method":"a"}"#))
        .await
        .unwrap();
    second
        .send(Message::text(r#"{"id":1,"method":"b"}"#))
        .await
        .unwrap();
    assert_eq!(next_text(&mut first).await, r#"echo:{"id":1,"method":"a"}"#);
    assert_eq!(
        next_text(&mut second).await,
        r#"echo:{"id":1,"method":"b"}"#
    );
    assert_eq!(*upstream.opened.lock().unwrap(), 2);
}

#[tokio::test]
async fn upstream_restart_closes_clients_with_service_restart_code() {
    let upstream = EchoUpstream::new(true);
    let mut client = connect_client(upstream.clone()).await;
    client.send(Message::text(r#"{"id":1}"#)).await.unwrap();
    next_text(&mut client).await;

    upstream.restart();
    assert_eq!(next_close_code(&mut client).await, Some(CloseCode::Restart));
}

#[tokio::test]
async fn unavailable_upstream_closes_the_client_with_try_again_later() {
    let upstream = EchoUpstream::new(false);
    let mut client = connect_client(upstream).await;
    assert_eq!(next_close_code(&mut client).await, Some(CloseCode::Again));
}

#[test]
fn arguments_accept_only_help() {
    assert_eq!(parse_args(&[]).unwrap(), GatewayCommand::Run);
    assert_eq!(
        parse_args(&["--help".to_string()]).unwrap(),
        GatewayCommand::Help
    );
    let error = parse_args(&["--port".to_string()]).unwrap_err().to_string();
    assert!(error.contains("unsupported argument: --port"), "{error}");
}

/// 실제 저장소 커널(자기 app-server, 시작 점검 없음)에 붙는 접속이다.
async fn connect_kernel_client(
    kernels: Arc<crate::composition::repository_kernel::RepositoryKernelRegistry>,
) -> WebSocketStream<DuplexStream> {
    let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    let upstream = EchoUpstream::new(true);
    tokio::spawn(async move {
        let _ = run_session(
            gateway_side,
            GatewayHosts::new(upstream, Vec::new()),
            kernels,
            super::session::GatewayServices::default(),
            None,
        )
        .await;
    });
    let (websocket, _) = tokio_tungstenite::client_async_with_config(
        "ws://localhost/",
        client_side,
        Some(websocket_config()),
    )
    .await
    .expect("client handshake through the gateway session should succeed");
    websocket
}

async fn next_json_matching(
    websocket: &mut WebSocketStream<DuplexStream>,
    matches: impl Fn(&Value) -> bool,
) -> Value {
    loop {
        let value: Value = serde_json::from_str(&next_text(websocket).await).unwrap();
        if matches(&value) {
            return value;
        }
    }
}

async fn kernel_request(
    websocket: &mut WebSocketStream<DuplexStream>,
    id: &str,
    method: &str,
    params: Value,
) -> Value {
    websocket
        .send(Message::text(
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        ))
        .await
        .unwrap();
    next_json_matching(websocket, |value| value["id"] == json!(id)).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_attaches_commands_and_receives_kernel_events() {
    // plan 18 K3-5: akra/kernel/* reaches the repository kernel through the Gateway session.
    let folder = crate::test_utils::platform_safe_temp_dir()
        .join(format!("akra-gateway-kernel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let workspace = folder.display().to_string();
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut client = connect_kernel_client(kernels.clone()).await;

    let refused = kernel_request(
        &mut client,
        "early",
        "akra/kernel/query",
        json!({"query": {"type": "auto_follow"}}),
    )
    .await;
    assert_eq!(refused["error"]["code"], json!(-32011), "{refused}");

    let attached = kernel_request(
        &mut client,
        "attach",
        "akra/kernel/attach",
        json!({"workspace": workspace, "client_name": "gateway-test"}),
    )
    .await;
    assert_eq!(
        attached["result"]["protocol_version"],
        json!(crate::adapter::inbound::kernel_protocol::PROTOCOL_VERSION),
        "{attached}"
    );
    assert!(
        attached["result"]["kernel"]
            .as_str()
            .is_some_and(|kernel| !kernel.is_empty())
    );

    // The event and the command response travel on different paths, so either may come first.
    client
        .send(Message::text(
            json!({
                "jsonrpc": "2.0",
                "id": "command",
                "method": "akra/kernel/command",
                "params": {"command": {"type": "set_auto_follow_max_turns", "value": 3}},
            })
            .to_string(),
        ))
        .await
        .unwrap();
    let (mut commanded, mut event) = (None, None);
    while commanded.is_none() || event.is_none() {
        let value: Value = serde_json::from_str(&next_text(&mut client).await).unwrap();
        if value["id"] == json!("command") {
            commanded = Some(value);
        } else if value["params"]["event"]["type"] == json!("auto_follow") {
            event = Some(value);
        }
    }
    // plan 19 Q3-6: a recorded command answers with its receipt. The operation id is a log position.
    let commanded = commanded.unwrap()["result"].clone();
    assert_eq!(commanded["processed"], json!(true), "{commanded}");
    assert_eq!(
        commanded["operation"]["state"],
        json!("accepted"),
        "{commanded}"
    );
    assert!(
        commanded["operation"]["op_id"]
            .as_i64()
            .is_some_and(|op_id| op_id > 0)
    );
    assert_eq!(
        commanded["operation"]["next"],
        json!({"query": "history", "after": commanded["operation"]["op_id"]})
    );
    assert_eq!(event.unwrap()["params"]["event"]["max_turns"], json!(3));

    // plan 18 K4-4: the operation query answers with the state and the records of that command. This command
    // ends when the kernel accepts it, so it is done. The query sees the record before the recorder wrote it.
    let operation = kernel_request(
        &mut client,
        "operation",
        "akra/kernel/query",
        json!({"query": {"type": "operation", "op_id": commanded["operation"]["op_id"]}}),
    )
    .await;
    let operation = &operation["result"];
    assert_eq!(operation["type"], json!("operation"), "{operation}");
    assert_eq!(operation["state"], json!("done"), "{operation}");
    assert_eq!(
        operation["records"][0]["event"]["kind"],
        json!("command_received"),
        "{operation}"
    );
    assert_eq!(
        operation["records"][0]["sequence"], commanded["operation"]["op_id"],
        "{operation}"
    );
    // plan 18 K4-5: the record names the connection that sent the command.
    assert_eq!(
        operation["records"][0]["event"]["caller"],
        json!("gateway-test"),
        "{operation}"
    );
    let unknown = kernel_request(
        &mut client,
        "unknown-operation",
        "akra/kernel/query",
        json!({"query": {"type": "operation", "op_id": 999_999}}),
    )
    .await;
    assert_eq!(unknown["result"]["state"], json!("unknown"), "{unknown}");

    // A resend with the same key does not run again and gets the first receipt.
    let keyed = |id: &'static str, value: u64| {
        (
            id,
            json!({
                "command": {"type": "set_auto_follow_max_turns", "value": value},
                "idempotency_key": "key-1",
            }),
        )
    };
    let (id, params) = keyed("keyed", 4);
    let first = kernel_request(&mut client, id, "akra/kernel/command", params).await;
    let (id, params) = keyed("resent", 4);
    let resent = kernel_request(&mut client, id, "akra/kernel/command", params).await;
    assert_eq!(
        first["result"]["operation"]["replayed"],
        Value::Null,
        "{first}"
    );
    assert_eq!(
        resent["result"]["operation"]["replayed"],
        json!(true),
        "{resent}"
    );
    assert_eq!(
        resent["result"]["operation"]["op_id"], first["result"]["operation"]["op_id"],
        "{resent}"
    );
    let (id, params) = keyed("reused", 5);
    let reused = kernel_request(&mut client, id, "akra/kernel/command", params).await;
    assert_eq!(reused["error"]["code"], json!(-32014), "{reused}");

    // A command that the kernel refuses at once answers `rejected`, and a resend gets the same refusal.
    let unknown_thread = |id: &'static str| {
        (
            id,
            json!({
                "command": {"type": "submit_prompt", "thread_id": "no-such-thread", "prompt": "hello"},
                "idempotency_key": "key-2",
            }),
        )
    };
    for (id, replayed) in [("refused", Value::Null), ("refused-again", json!(true))] {
        let (id, params) = unknown_thread(id);
        let refused = kernel_request(&mut client, id, "akra/kernel/command", params).await;
        let result = &refused["result"];
        assert_eq!(
            result["rejection"]["reason"],
            json!("unknown_thread"),
            "{refused}"
        );
        assert_eq!(result["operation"]["state"], json!("rejected"), "{refused}");
        assert_eq!(result["operation"]["replayed"], replayed, "{refused}");
    }

    let queried = kernel_request(
        &mut client,
        "query",
        "akra/kernel/query",
        json!({"query": {"type": "auto_follow"}}),
    )
    .await;
    assert_eq!(queried["result"]["type"], json!("auto_follow"), "{queried}");
    // The keyed command (4) ran once. The command that reused its key (5) did not run.
    assert_eq!(queried["result"]["max_turns"], json!(4), "{queried}");

    // plan 18 K3-6: the planning queue and the parallel state travel through the same session.
    let refreshed = kernel_request(
        &mut client,
        "refresh",
        "akra/kernel/command",
        json!({"command": {"type": "refresh_planning"}}),
    )
    .await;
    assert_eq!(
        refreshed["result"],
        json!({"processed": true}),
        "{refreshed}"
    );
    let planning = kernel_request(
        &mut client,
        "planning",
        "akra/kernel/query",
        json!({"query": {"type": "planning"}}),
    )
    .await;
    assert_eq!(planning["result"]["type"], json!("planning"), "{planning}");
    let parallel = kernel_request(
        &mut client,
        "parallel",
        "akra/kernel/query",
        json!({"query": {"type": "parallel"}}),
    )
    .await;
    assert_eq!(parallel["result"]["type"], json!("parallel"), "{parallel}");
    // plan 19 Q3-3: the history query goes through the host thread; this kernel has no log.
    let history = kernel_request(
        &mut client,
        "history",
        "akra/kernel/query",
        json!({"query": {"type": "history", "after": 7}}),
    )
    .await;
    assert_eq!(history["result"]["type"], json!("history"), "{history}");
    assert_eq!(history["result"]["records"], json!([]), "{history}");
    assert_eq!(history["result"]["next_after"], json!(7), "{history}");
    assert_eq!(parallel["result"]["enabled"], json!(false), "{parallel}");

    let described =
        kernel_request(&mut client, "describe", "akra/kernel/describe", json!({})).await;
    // plan 18 K4-1: `describe` answers with the whole spec. A client needs no other document.
    let spec = &described["result"];
    assert_eq!(spec["protocol_version"], json!(1), "{described}");
    let named = |list: &str, name: &str| {
        spec[list]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry["name"] == name))
    };
    assert!(named("methods", "akra/kernel/describe"));
    assert!(named("commands", "submit_prompt"));
    assert!(named("queries", "history"));
    assert!(named("events", "recorded"));
    assert!(named("records", "command_received"));
    assert!(named("reasons", "busy"));
    assert!(named("errors", "idempotency_key_reused"));
    assert!(spec["$defs"]["CommandResponse"].is_object());

    let detached = kernel_request(&mut client, "detach", "akra/kernel/detach", json!({})).await;
    assert_eq!(detached["result"], json!({}));
    let refused = kernel_request(
        &mut client,
        "late",
        "akra/kernel/command",
        json!({"command": {"type": "pause_auto_follow"}}),
    )
    .await;
    assert_eq!(refused["error"]["code"], json!(-32011), "{refused}");

    drop(client);
    drop(kernels);
    let _ = std::fs::remove_dir_all(&folder);
}

/// 커널 시험 폴더이다. 끝나면 지운다.
struct KernelTestFolder {
    path: std::path::PathBuf,
}

impl KernelTestFolder {
    fn new(label: &str) -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-gateway-kernel-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn workspace(&self) -> String {
        self.path.display().to_string()
    }
}

impl Drop for KernelTestFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

async fn attach_kernel(
    client: &mut WebSocketStream<DuplexStream>,
    id: &str,
    workspace: &str,
) -> Value {
    let attached = kernel_request(
        client,
        id,
        "akra/kernel/attach",
        json!({"workspace": workspace, "client_name": "gateway-test"}),
    )
    .await;
    assert!(attached["result"]["kernel"].is_string(), "{attached}");
    attached
}

fn set_max_turns_command(id: &str, value: u32) -> Message {
    Message::text(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "akra/kernel/command",
            "params": {"command": {"type": "set_auto_follow_max_turns", "value": value}},
        })
        .to_string(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn transient_detach_and_socket_close_wake_idle_unload() {
    for close_socket in [false, true] {
        let folder = KernelTestFolder::new(if close_socket {
            "transient-close"
        } else {
            "transient-detach"
        });
        let kernels =
            Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
        let idle_notification = super::register_idle_notification(&kernels);
        let mut client = connect_kernel_client(kernels.clone()).await;
        let attached = kernel_request(
            &mut client,
            "attach",
            "akra/kernel/attach",
            json!({"workspace": folder.workspace(), "client_name": "command-test", "transient": true}),
        ).await;
        let key = attached["result"]["kernel"]
            .as_str()
            .expect("attached")
            .to_string();
        let idle = idle_notification.0.notified();
        tokio::pin!(idle);
        idle.as_mut().enable();
        if close_socket {
            client.close(None).await.expect("close");
        } else {
            let detached =
                kernel_request(&mut client, "detach", "akra/kernel/detach", json!({})).await;
            assert_eq!(detached["result"], json!({}));
        }
        tokio::time::timeout(Duration::from_secs(10), idle)
            .await
            .expect("disconnect wakes the idle reaper");
        assert_eq!(
            kernels.unload_idle_kernels(std::time::Instant::now()),
            vec![key]
        );
        assert!(
            crate::composition::production::acquire_repository_kernel_lock(&folder.workspace())
                .is_ok()
        );
    }
}

/// 조용한 시간이 올 때까지 받은 텍스트 메시지를 모은다.
async fn texts_until_quiet(
    websocket: &mut WebSocketStream<DuplexStream>,
    quiet: Duration,
) -> Vec<Value> {
    let mut texts = Vec::new();
    while let Ok(Some(Ok(message))) = tokio::time::timeout(quiet, websocket.next()).await {
        if let Message::Text(text) = message {
            texts.push(serde_json::from_str(text.as_str()).unwrap());
        }
    }
    texts
}

const QUIET_WINDOW: Duration = Duration::from_millis(700);

#[tokio::test(flavor = "multi_thread")]
async fn a_client_close_ends_the_session_without_waiting_for_the_flush_timeout() {
    // K3-5 review: the kernel session kept an outbox sender, so the writer waited 2 s on every close.
    let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    let session = tokio::spawn(run_session(
        gateway_side,
        GatewayHosts::new(EchoUpstream::new(true), Vec::new()),
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default()),
        super::session::GatewayServices::default(),
        None,
    ));
    let (mut client, _) = tokio_tungstenite::client_async_with_config(
        "ws://localhost/",
        client_side,
        Some(websocket_config()),
    )
    .await
    .unwrap();

    client.close(None).await.unwrap();

    tokio::time::timeout(Duration::from_secs(1), session)
        .await
        .expect("the session should end before the flush timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_kernel_attach_does_not_block_app_server_forwarding() {
    // K3-5 review: the read loop awaited the kernel call, so app-server messages stopped behind it.
    let folder = KernelTestFolder::new("waiting-attach");
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let release = crate::composition::repository_kernel::tests::hold_repository_for_test(
        &kernels,
        &folder.workspace(),
    );
    let mut client = connect_kernel_client(kernels.clone()).await;

    client
        .send(Message::text(
            json!({
                "jsonrpc": "2.0",
                "id": "attach",
                "method": "akra/kernel/attach",
                "params": {"workspace": folder.workspace(), "client_name": "gateway-test"},
            })
            .to_string(),
        ))
        .await
        .unwrap();
    client
        .send(Message::text(r#"{"id":1,"method":"thread/list"}"#))
        .await
        .unwrap();
    let forwarded = tokio::time::timeout(Duration::from_secs(2), next_text(&mut client))
        .await
        .expect("app-server forwarding must not wait for the kernel attach");
    assert_eq!(forwarded, r#"echo:{"id":1,"method":"thread/list"}"#);

    drop(release);
    let attached = next_json_matching(&mut client, |value| value["id"] == json!("attach")).await;
    assert!(attached["result"]["kernel"].is_string(), "{attached}");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_kernel_event_reaches_a_client_after_its_detach_response() {
    let folder = KernelTestFolder::new("detach-events");
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut watcher = connect_kernel_client(kernels.clone()).await;
    let mut driver = connect_kernel_client(kernels.clone()).await;
    attach_kernel(&mut watcher, "attach", &folder.workspace()).await;
    attach_kernel(&mut driver, "attach", &folder.workspace()).await;

    let detached = kernel_request(&mut watcher, "detach", "akra/kernel/detach", json!({})).await;
    assert_eq!(detached["result"], json!({}));
    driver
        .send(set_max_turns_command("command", 3))
        .await
        .unwrap();
    next_json_matching(&mut driver, |value| value["id"] == json!("command")).await;

    let late = texts_until_quiet(&mut watcher, QUIET_WINDOW).await;
    assert!(
        late.is_empty(),
        "events after the detach response: {late:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attaching_again_does_not_duplicate_kernel_events() {
    let folder = KernelTestFolder::new("reattach-events");
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut client = connect_kernel_client(kernels.clone()).await;
    attach_kernel(&mut client, "first", &folder.workspace()).await;
    attach_kernel(&mut client, "second", &folder.workspace()).await;

    client
        .send(set_max_turns_command("command", 4))
        .await
        .unwrap();

    let received = texts_until_quiet(&mut client, QUIET_WINDOW).await;
    let events = received
        .iter()
        .filter(|value| value["params"]["event"]["type"] == json!("auto_follow"))
        .count();
    assert_eq!(events, 1, "{received:?}");
}

/// upstream 이 멈출 때 그 저장소의 커널 잠금이 풀려 있었는지 적는다.
struct LockProbeUpstream {
    inner: Arc<EchoUpstream>,
    workspace: String,
    kernel_lock_free_at_shutdown: Mutex<Option<bool>>,
}

impl GatewayUpstreamPort for LockProbeUpstream {
    fn open_session(&self) -> Result<Box<dyn GatewayUpstreamStream>> {
        self.inner.open_session()
    }

    fn snapshot(&self) -> GatewayUpstreamSnapshot {
        self.inner.snapshot()
    }

    fn generation(&self) -> watch::Receiver<u64> {
        self.inner.generation()
    }

    fn shutdown(&self) {
        let free =
            crate::composition::production::acquire_repository_kernel_lock(&self.workspace).is_ok();
        *self.kernel_lock_free_at_shutdown.lock().unwrap() = Some(free);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway_stop_stops_kernels_before_the_upstream_app_server() {
    // K3-5 review: a connected session kept the registry alive, so the upstream stopped first.
    let folder = KernelTestFolder::new("stop-order");
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut client = connect_kernel_client(kernels.clone()).await;
    attach_kernel(&mut client, "attach", &folder.workspace()).await;
    let upstream = Arc::new(LockProbeUpstream {
        inner: EchoUpstream::new(true),
        workspace: folder.workspace(),
        kernel_lock_free_at_shutdown: Mutex::new(None),
    });

    super::stop_backend(
        GatewayHosts::new(upstream.clone(), Vec::new()),
        kernels,
        empty_mcp_hub(),
    )
    .await;

    assert_eq!(
        *upstream.kernel_lock_free_at_shutdown.lock().unwrap(),
        Some(true),
        "the kernel stops and releases its lock before the upstream stops"
    );
}

/// MCP hub 가 필요 없는 테스트용이다. 등록된 서버가 없고 엔진도 없다.
fn empty_mcp_hub() -> Arc<super::GatewayMcpHub> {
    struct EmptyHub;
    impl crate::application::port::outbound::mcp_hub_backend_port::McpHubBackendPort for EmptyHub {
        fn registered_servers(
            &self,
        ) -> anyhow::Result<Vec<crate::domain::mcp_hub::McpHubServerSpec>> {
            Ok(Vec::new())
        }
        fn register_server(
            &self,
            _spec: &crate::domain::mcp_hub::McpHubServerSpec,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn unregister_server(&self, _name: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
        fn server_statuses(
            &self,
            _only: Option<&str>,
        ) -> anyhow::Result<Vec<crate::domain::mcp_hub::McpHubServerStatus>> {
            Ok(Vec::new())
        }
        fn call_tool(
            &self,
            _server: &str,
            _tool: &str,
            _arguments: serde_json::Value,
        ) -> anyhow::Result<serde_json::Value> {
            anyhow::bail!("no MCP hub in this test")
        }
        fn login(&self, _server: &str, _on_url: &mut dyn FnMut(&str)) -> anyhow::Result<()> {
            Ok(())
        }
    }
    Arc::new(super::GatewayMcpHub::new(String::new(), Arc::new(EmptyHub)))
}

/// plan 25 S1a: `local` 과 가짜 두 번째 호스트 `cloud` 가 있는 Gateway 에 `path` 로 붙는다.
async fn connect_to_host(
    path: &str,
    local: Arc<EchoUpstream>,
    cloud: Arc<EchoUpstream>,
    recorder: Option<Arc<super::recorder::GatewayRecorder>>,
) -> WebSocketStream<DuplexStream> {
    let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    let hosts = GatewayHosts::new(local, vec![("cloud".to_string(), cloud as _)]);
    tokio::spawn(async move {
        let _ = run_session(
            gateway_side,
            hosts,
            Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default()),
            super::session::GatewayServices::default(),
            recorder,
        )
        .await;
    });
    let (websocket, _) = tokio_tungstenite::client_async_with_config(
        format!("ws://localhost{path}"),
        client_side,
        Some(websocket_config()),
    )
    .await
    .expect("client handshake through the gateway session should succeed");
    websocket
}

#[tokio::test]
async fn the_request_path_selects_the_host_and_an_unknown_host_is_closed() {
    let local = EchoUpstream::new(true);
    let cloud = EchoUpstream::new(true);

    let mut remote = connect_to_host("/host/cloud", local.clone(), cloud.clone(), None).await;
    remote.send(Message::text(r#"{"id":1}"#)).await.unwrap();
    assert_eq!(next_text(&mut remote).await, r#"echo:{"id":1}"#);
    assert_eq!(*cloud.opened.lock().unwrap(), 1);
    assert_eq!(*local.opened.lock().unwrap(), 0);

    let info = kernel_request(&mut remote, "info", "akra/gateway/info", json!({})).await;
    assert_eq!(info["result"]["host"], json!("cloud"));
    let ids: Vec<&Value> = info["result"]["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|host| &host["id"])
        .collect();
    assert_eq!(ids, [&json!("local"), &json!("cloud")]);

    let mut unknown = connect_to_host("/host/nope", local.clone(), cloud.clone(), None).await;
    assert_eq!(next_close_code(&mut unknown).await, Some(CloseCode::Policy));
    assert_eq!(*local.opened.lock().unwrap(), 0);
}

#[tokio::test]
async fn a_remote_host_refuses_kernel_methods_and_answers_context_without_the_ledger() {
    // plan 25 §3.1 and Q-C3. No work context service is set, so an answer proves the ledger was not used.
    let mut remote = connect_to_host(
        "/host/cloud",
        EchoUpstream::new(true),
        EchoUpstream::new(true),
        None,
    )
    .await;

    let refused = kernel_request(&mut remote, "k", "akra/kernel/describe", json!({})).await;
    assert_eq!(refused["error"]["code"], json!(-32601));

    let context = kernel_request(
        &mut remote,
        "c",
        "akra/context/thread",
        json!({ "threadId": "remote-thread" }),
    )
    .await;
    assert_eq!(context["result"]["resources"], json!([]));
}

#[tokio::test]
async fn a_remote_host_session_is_not_recorded() {
    let directory = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-gateway-host-recording-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let recorder = Arc::new(super::recorder::GatewayRecorder::create(&directory).unwrap());
    let sessions = || std::fs::read_dir(&directory).unwrap().count();
    let before = sessions();

    let mut remote = connect_to_host(
        "/host/cloud",
        EchoUpstream::new(true),
        EchoUpstream::new(true),
        Some(recorder.clone()),
    )
    .await;
    remote.send(Message::text(r#"{"id":1}"#)).await.unwrap();
    next_text(&mut remote).await;
    assert_eq!(
        sessions(),
        before,
        "a remote session must not open a recording"
    );

    let mut local = connect_to_host(
        "/",
        EchoUpstream::new(true),
        EchoUpstream::new(true),
        Some(recorder),
    )
    .await;
    local.send(Message::text(r#"{"id":1}"#)).await.unwrap();
    next_text(&mut local).await;
    assert!(sessions() > before, "the local session is still recorded");
    let _ = std::fs::remove_dir_all(&directory);
}

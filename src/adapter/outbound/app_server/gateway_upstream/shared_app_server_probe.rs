//! plan 18 Q-K3-6 확인이다. Gateway upstream app-server 하나에 연결 두 개를 연다. 연결 A(커널 역할)가
//! thread 를 만들고 턴을 돌린다. 연결 B(Desktop 역할)가 그 thread 를 `thread/resume` 한 뒤 A 의 다음 턴
//! 알림을 받는지 본다. 실제 codex 턴을 두 번 돌려 사용량을 쓰므로 기본으로는 돌지 않는다. 끝나면 thread 를
//! 보관 처리한다.
//!
//! 실행: `cargo test --lib shared_app_server_probe -- --ignored --nocapture`

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use super::AppServerGatewayUpstream;
use crate::application::port::outbound::gateway_upstream_port::{
    GatewayUpstreamPort, GatewayUpstreamStream,
};

type Connection = WebSocketStream<Box<dyn GatewayUpstreamStream>>;

const TURN_TIMEOUT: Duration = Duration::from_secs(180);
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// A 가 턴을 끝낸 뒤 B 에 남은 알림을 읽는 시간이다. 알림은 이미 소켓에 쌓여 있다.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
const PROMPT: &str = "Reply with exactly the word ok. Do not run any command.";

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs real codex turns and uses Codex usage (plan 18 Q-K3-6)"]
async fn shared_app_server_probe() {
    let run_dir = std::env::temp_dir().join(format!("akq6-{}", std::process::id()));
    crate::private_directory::ensure_private_directory(&run_dir).expect("private run dir");
    let upstream = AppServerGatewayUpstream::start(&run_dir).expect("upstream should start");
    let result = probe(&upstream).await;
    upstream.shutdown();
    let _ = std::fs::remove_dir_all(&run_dir);
    let report = result.expect("probe should finish");
    println!("Q-K3-6 report: {report:#}");
    assert_eq!(
        report["resumer_saw_second_turn_completed"],
        json!(true),
        "the resumed connection did not receive the turn"
    );
}

async fn probe(upstream: &AppServerGatewayUpstream) -> Result<Value> {
    let mut runner = connect(upstream).await?;
    let mut resumer = connect(upstream).await?;
    initialize(&mut runner, "akra-q-k3-6-runner").await?;
    initialize(&mut resumer, "akra-q-k3-6-resumer").await?;

    let started = request(
        &mut runner,
        10,
        "thread/start",
        json!({
            "cwd": std::env::temp_dir().display().to_string(),
            "approvalPolicy": "never",
            "sandbox": "read-only",
        }),
    )
    .await?;
    let thread_id = started["thread"]["id"]
        .as_str()
        .context("thread/start returned no thread id")?
        .to_string();

    // 첫 턴: rollout 이 생긴다. B 는 아직 thread 를 구독하지 않았다.
    start_turn(&mut runner, 11, &thread_id).await?;
    let first_runner = collect_until_turn_completed(&mut runner, TURN_TIMEOUT).await;
    let first_resumer = collect_until_turn_completed(&mut resumer, DRAIN_TIMEOUT).await;

    // B 가 thread 에 합류한 뒤 A 가 두 번째 턴을 돌린다.
    let resumed = request(
        &mut resumer,
        20,
        "thread/resume",
        json!({ "threadId": thread_id, "excludeTurns": true }),
    )
    .await;
    let resume_error = resumed.as_ref().err().map(|error| format!("{error:#}"));
    start_turn(&mut runner, 12, &thread_id).await?;
    let second_runner = collect_until_turn_completed(&mut runner, TURN_TIMEOUT).await;
    let second_resumer = collect_until_turn_completed(&mut resumer, DRAIN_TIMEOUT).await;

    let archived = request(
        &mut runner,
        13,
        "thread/archive",
        json!({ "threadId": thread_id }),
    )
    .await
    .is_ok();
    Ok(json!({
        "thread_id": thread_id,
        "archived": archived,
        "resume_error": resume_error,
        "first_turn": { "runner": first_runner.report(), "unsubscribed_resumer": first_resumer.report() },
        "second_turn": { "runner": second_runner.report(), "resumed_resumer": second_resumer.report() },
        "resumer_saw_second_turn_completed": second_resumer.completed,
    }))
}

async fn start_turn(connection: &mut Connection, id: u64, thread_id: &str) -> Result<()> {
    request(
        connection,
        id,
        "turn/start",
        json!({
            "threadId": thread_id,
            "input": [{ "type": "text", "text": PROMPT }],
        }),
    )
    .await
    .map(|_| ())
}

async fn connect(upstream: &AppServerGatewayUpstream) -> Result<Connection> {
    let stream = upstream.open_session()?;
    let (websocket, _) = tokio_tungstenite::client_async("ws://localhost/rpc", stream)
        .await
        .context("upstream WebSocket handshake failed")?;
    Ok(websocket)
}

async fn initialize(connection: &mut Connection, name: &str) -> Result<()> {
    request(
        connection,
        1,
        "initialize",
        json!({
            "clientInfo": { "name": name, "version": "0" },
            "capabilities": { "experimentalApi": false },
        }),
    )
    .await?;
    send(
        connection,
        json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await
}

async fn send(connection: &mut Connection, message: Value) -> Result<()> {
    connection
        .send(Message::text(message.to_string()))
        .await
        .context("send failed")
}

/// 요청을 보내고 그 id 의 응답을 기다린다. 그 사이에 온 알림은 버린다.
async fn request(
    connection: &mut Connection,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value> {
    send(
        connection,
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
    )
    .await?;
    let deadline = Instant::now() + REPLY_TIMEOUT;
    while let Some(message) = next_json(connection, deadline).await {
        if message["id"] == json!(id) && message.get("method").is_none() {
            if let Some(error) = message.get("error") {
                bail!("{method} failed: {error}");
            }
            return Ok(message["result"].clone());
        }
    }
    bail!("{method} got no reply")
}

#[derive(Default)]
struct Observed {
    methods: BTreeSet<String>,
    errors: Vec<Value>,
    completed: bool,
}

impl Observed {
    fn report(&self) -> Value {
        json!({ "methods": self.methods, "errors": self.errors, "turn_completed": self.completed })
    }
}

async fn collect_until_turn_completed(connection: &mut Connection, timeout: Duration) -> Observed {
    let deadline = Instant::now() + timeout;
    let mut observed = Observed::default();
    while let Some(message) = next_json(connection, deadline).await {
        let Some(method) = message["method"].as_str() else {
            continue;
        };
        observed.methods.insert(method.to_string());
        if method == "error" {
            observed.errors.push(message["params"].clone());
        }
        if method == "turn/completed" {
            observed.completed = true;
            break;
        }
    }
    observed
}

async fn next_json(connection: &mut Connection, deadline: Instant) -> Option<Value> {
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        let message = tokio::time::timeout(left, connection.next())
            .await
            .ok()??
            .ok()?;
        if let Message::Text(text) = message
            && let Ok(value) = serde_json::from_str::<Value>(text.as_str())
        {
            return Some(value);
        }
    }
}

/// plan 18 K3-5: 커널의 app-server adapter 가 `app-server proxy` WebSocket 전송으로 Gateway upstream 에 붙는다.
/// 모델 턴은 돌리지 않는다(사용량을 쓰지 않는다). 실제 codex 와 로그인이 필요하므로 기본으로는 돌지 않는다.
#[test]
#[ignore = "needs a real codex login (plan 18 K3-5)"]
fn the_kernel_adapter_talks_to_the_upstream_through_the_proxy() {
    use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
    use crate::application::port::outbound::startup_probe_port::StartupProbePort;
    use crate::domain::recent_sessions::SessionCatalogRequest;

    let run_dir = std::env::temp_dir().join(format!("akq5-{}", std::process::id()));
    crate::private_directory::ensure_private_directory(&run_dir).expect("private run dir");
    let upstream = AppServerGatewayUpstream::start(&run_dir).expect("upstream should start");
    let adapter = super::super::CodexAppServerAdapter::new("akra-k3-5-probe", "0")
        .through_gateway_upstream(upstream.kernel_socket());

    let startup = adapter.load_startup_context();
    let catalog = adapter.load_session_catalog(SessionCatalogRequest {
        limit: 5,
        current_workspace_directory: None,
    });
    drop(adapter);
    upstream.shutdown();
    let _ = std::fs::remove_dir_all(&run_dir);

    let startup = startup.expect("startup context should load through the proxy");
    println!("initialize: {}", startup.initialize_detail);
    println!("account ok: {}", startup.account_ok);
    let catalog = catalog.expect("session catalog should load through the proxy");
    assert!(
        matches!(
            catalog,
            crate::domain::recent_sessions::SessionCatalog::Ready { .. }
        ),
        "session catalog should be ready: {catalog:?}"
    );
    println!("sessions: ready");
}

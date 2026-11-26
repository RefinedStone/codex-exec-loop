//! plan 18 K6-1: the TUI's app-server connection reaches the Gateway through an `akra` relay child.
//! A fake Gateway answers the app-server calls of a conversation (open, manual prompt, stream). No model turn runs.

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message};

const AKRA_BIN: &str = env!("CARGO_BIN_EXE_akra");
const RELAY_COMMAND: &str = "__gateway-app-server-relay";
const THREAD_ID: &str = "thread-relay-1";
const TURN_ID: &str = "turn-relay-1";

#[test]
fn relay_carries_a_conversation_and_ends_when_the_gateway_restarts_its_upstream() {
    let endpoint = test_endpoint("conversation");
    let gateway = FakeGateway::start(&endpoint);
    let mut relay = RelayChild::spawn(&endpoint);
    let (mut socket, _) = tungstenite::client::client("ws://localhost/rpc", relay.stdio())
        .expect("the WebSocket handshake reaches the Gateway through the relay");

    let initialized = call(
        &mut socket,
        1,
        "initialize",
        json!({"clientInfo": {"name": "akra"}}),
    );
    assert_eq!(initialized["result"]["userAgent"], "fake-gateway");
    let started = call(&mut socket, 2, "thread/start", json!({"cwd": "/repo"}));
    assert_eq!(started["result"]["thread"]["id"], THREAD_ID);
    let turn = call(
        &mut socket,
        3,
        "turn/start",
        json!({"threadId": THREAD_ID, "input": [{"type": "text", "text": "hello"}]}),
    );
    assert_eq!(turn["result"]["turn"]["id"], TURN_ID);
    assert_eq!(
        read_json(&mut socket)["params"]["delta"],
        "relayed reply",
        "the stream notification reaches the client"
    );
    assert_eq!(read_json(&mut socket)["method"], "turn/completed");

    // The Gateway closes a session with 1012 when its upstream app-server restarts.
    let close = loop {
        match socket.read().expect("the close frame arrives") {
            Message::Close(frame) => break frame.expect("close frame has a code"),
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("unexpected message before close: {other:?}"),
        }
    };
    assert_eq!(close.code, CloseCode::Restart);
    // Answer the close so the Gateway ends its side of the session.
    let _ = socket.flush();
    drop(socket);
    assert_eq!(
        relay.wait(),
        0,
        "the relay ends normally when the Gateway closes"
    );
    assert_eq!(
        gateway.methods(),
        ["initialize", "thread/start", "turn/start"],
        "the Gateway saw every call of the conversation"
    );
}

#[test]
fn relay_reports_a_missing_gateway() {
    let endpoint = test_endpoint("missing");
    let output = Command::new(AKRA_BIN)
        .args([RELAY_COMMAND, &endpoint])
        .stdin(Stdio::null())
        .output()
        .expect("relay runs");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot reach the Akra Gateway"),
        "stderr names the missing Gateway: {stderr}"
    );
}

fn call(
    socket: &mut tungstenite::WebSocket<RelayStdio<'_>>,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    let request = json!({"id": id, "method": method, "params": params});
    socket
        .send(Message::text(request.to_string()))
        .expect("request is sent");
    read_json(socket)
}

fn read_json(socket: &mut tungstenite::WebSocket<RelayStdio<'_>>) -> Value {
    loop {
        match socket.read().expect("a message arrives") {
            Message::Text(text) => return serde_json::from_str(&text).expect("JSON message"),
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("unexpected message: {other:?}"),
        }
    }
}

fn test_endpoint(name: &str) -> String {
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    );
    #[cfg(unix)]
    {
        std::env::temp_dir()
            .join(format!("akra-relay-{name}-{nonce}.sock"))
            .display()
            .to_string()
    }
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\akra-relay-it-{name}-{nonce}")
    }
}

struct RelayChild {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl RelayChild {
    fn spawn(endpoint: &str) -> Self {
        let mut child = Command::new(AKRA_BIN)
            .args([RELAY_COMMAND, endpoint])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("relay child starts");
        let stdin = child.stdin.take().expect("relay stdin");
        let stdout = child.stdout.take().expect("relay stdout");
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn stdio(&mut self) -> RelayStdio<'_> {
        RelayStdio {
            stdin: &mut self.stdin,
            stdout: &mut self.stdout,
        }
    }

    fn wait(&mut self) -> i32 {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().expect("relay status") {
                return status.code().expect("relay exit code");
            }
            assert!(
                std::time::Instant::now() < deadline,
                "relay did not end after the Gateway closed"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for RelayChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The client side of the relay's stdio, as one stream for the WebSocket client.
struct RelayStdio<'a> {
    stdin: &'a mut ChildStdin,
    stdout: &'a mut ChildStdout,
}

impl Read for RelayStdio<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.stdout.read(buffer)
    }
}

impl Write for RelayStdio<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.stdin.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stdin.flush()
    }
}

/// One-session fake Gateway: it answers the app-server calls itself, as the real Gateway relays them to
/// its upstream, and then closes with 1012 like an upstream restart.
struct FakeGateway {
    methods: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    _runtime: tokio::runtime::Runtime,
}

impl FakeGateway {
    fn start(endpoint: &str) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("fake Gateway runtime");
        let methods = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let listener = runtime.block_on(async { Listener::bind(endpoint) });
        let seen = methods.clone();
        runtime.spawn(async move {
            let stream = listener.accept().await;
            serve(stream, seen).await;
        });
        Self {
            methods,
            _runtime: runtime,
        }
    }

    fn methods(&self) -> Vec<String> {
        self.methods.lock().unwrap().clone()
    }
}

async fn serve<S>(stream: S, methods: std::sync::Arc<std::sync::Mutex<Vec<String>>>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut socket = tokio_tungstenite::accept_async(stream)
        .await
        .expect("fake Gateway handshake");
    while let Some(Ok(message)) = socket.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let request: Value = serde_json::from_str(&text).expect("JSON request");
        let method = request["method"].as_str().unwrap_or_default().to_string();
        methods.lock().unwrap().push(method.clone());
        let id = request["id"].clone();
        let mut replies = match method.as_str() {
            "initialize" => vec![json!({"id": id, "result": {"userAgent": "fake-gateway"}})],
            "thread/start" => vec![json!({"id": id, "result": {"thread": {"id": THREAD_ID}}})],
            "turn/start" => vec![
                json!({"id": id, "result": {"turn": {"id": TURN_ID}}}),
                json!({"method": "item/agentMessage/delta", "params": {
                    "threadId": THREAD_ID, "turnId": TURN_ID, "itemId": "agent-1", "delta": "relayed reply"
                }}),
                json!({"method": "turn/completed", "params": {
                    "threadId": THREAD_ID, "turn": {"id": TURN_ID, "items": [], "status": "completed"}
                }}),
            ],
            other => vec![json!({"id": id, "error": {"message": format!("unexpected {other}")}})],
        };
        let finished = method == "turn/start";
        for reply in replies.drain(..) {
            socket
                .send(Message::text(reply.to_string()))
                .await
                .expect("fake Gateway reply");
        }
        if finished {
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Restart,
                    reason: "codex app-server upstream restarted".into(),
                })))
                .await;
            let _ = socket.flush().await;
            // Wait for the client's close answer so the frame is not lost.
            while let Some(Ok(_)) = socket.next().await {}
            return;
        }
    }
}

#[cfg(unix)]
struct Listener(tokio::net::UnixListener);

#[cfg(unix)]
impl Listener {
    fn bind(endpoint: &str) -> Self {
        Self(tokio::net::UnixListener::bind(endpoint).expect("fake Gateway binds"))
    }

    async fn accept(self) -> tokio::net::UnixStream {
        self.0.accept().await.expect("relay connects").0
    }
}

#[cfg(windows)]
struct Listener(tokio::net::windows::named_pipe::NamedPipeServer);

#[cfg(windows)]
impl Listener {
    fn bind(endpoint: &str) -> Self {
        Self(
            tokio::net::windows::named_pipe::ServerOptions::new()
                .first_pipe_instance(true)
                .create(endpoint)
                .expect("fake Gateway binds"),
        )
    }

    async fn accept(self) -> tokio::net::windows::named_pipe::NamedPipeServer {
        self.0.connect().await.expect("relay connects");
        self.0
    }
}

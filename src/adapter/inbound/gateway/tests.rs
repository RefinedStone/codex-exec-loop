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
use super::parse_args;
use super::session::run_session;
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
            let Ok(mut websocket) = tokio_tungstenite::accept_async(upstream_side).await else {
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
        let _ = run_session(gateway_side, upstream, None).await;
    });
    let (websocket, _) = tokio_tungstenite::client_async("ws://localhost/", client_side)
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

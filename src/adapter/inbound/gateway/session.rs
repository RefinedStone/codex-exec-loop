//! 클라이언트 접속 하나를 맡는다. 클라이언트와 WebSocket 핸드셰이크를 하고, upstream 연결 하나를 열어
//! 메시지를 양방향으로 그대로 중계한다 (P-03). 방향마다 작업을 하나만 두어 순서를 지킨다. ping/pong 은
//! 구간마다 tungstenite 가 처리하고, close 는 반대쪽으로 전파한다. upstream 이 다시 시작되면 close 1012 로
//! 클라이언트 접속을 닫는다.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use super::router::{ClientTextRoute, respond_to_akra_request, route_client_text};
use crate::application::port::outbound::gateway_upstream_port::{
    GatewayUpstreamPort, GatewayUpstreamStream,
};

/// proxy 는 바이트만 전달하므로 호스트 이름은 의미가 없다. app-server 의 WebSocket 경로이다.
const UPSTREAM_WEBSOCKET_URL: &str = "ws://localhost/rpc";
const CLIENT_OUTBOX_CAPACITY: usize = 256;
const CLIENT_CLOSE_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionEnd {
    ClientClosed,
    UpstreamClosed,
    UpstreamRestarted,
}

pub(super) async fn run_session<S>(client: S, upstream: Arc<dyn GatewayUpstreamPort>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let client_websocket = tokio_tungstenite::accept_async(client)
        .await
        .context("client WebSocket handshake failed")?;
    let (mut client_sink, mut client_stream) = client_websocket.split();
    let mut generation = upstream.generation();
    generation.borrow_and_update();

    let upstream_websocket = match open_upstream(upstream.as_ref()).await {
        Ok(websocket) => websocket,
        Err(error) => {
            let _ = client_sink
                .send(close_message(
                    CloseCode::Again,
                    "codex app-server upstream is unavailable",
                ))
                .await;
            return Err(error);
        }
    };
    let (mut upstream_sink, mut upstream_stream) = upstream_websocket.split();

    // 클라이언트로 가는 메시지는 upstream 응답과 `akra/*` 응답 두 곳에서 오므로 쓰기 작업 하나로 모은다.
    let (outbox, mut outbox_receiver) = mpsc::channel::<Message>(CLIENT_OUTBOX_CAPACITY);
    let writer = tokio::spawn(async move {
        while let Some(message) = outbox_receiver.recv().await {
            let closing = matches!(message, Message::Close(_));
            if client_sink.send(message).await.is_err() || closing {
                break;
            }
        }
        let _ = client_sink.close().await;
    });

    let client_to_upstream = async {
        while let Some(Ok(message)) = client_stream.next().await {
            match message {
                Message::Text(text) => match route_client_text(text.as_str()) {
                    ClientTextRoute::Forward => {
                        if upstream_sink.send(Message::Text(text)).await.is_err() {
                            return SessionEnd::UpstreamClosed;
                        }
                    }
                    ClientTextRoute::AkraRequest { id, method } => {
                        let response = respond_to_akra_request(id, &method, &upstream.snapshot());
                        if outbox.send(Message::text(response)).await.is_err() {
                            return SessionEnd::ClientClosed;
                        }
                    }
                    ClientTextRoute::AkraNotification { method } => {
                        tracing::debug!(method, "ignored gateway notification without an id");
                    }
                },
                Message::Binary(bytes) => {
                    if upstream_sink.send(Message::Binary(bytes)).await.is_err() {
                        return SessionEnd::UpstreamClosed;
                    }
                }
                Message::Close(frame) => {
                    let _ = upstream_sink.send(Message::Close(frame)).await;
                    return SessionEnd::ClientClosed;
                }
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
        SessionEnd::ClientClosed
    };

    let upstream_to_client = async {
        while let Some(Ok(message)) = upstream_stream.next().await {
            match message {
                Message::Text(_) | Message::Binary(_) => {
                    if outbox.send(message).await.is_err() {
                        return SessionEnd::ClientClosed;
                    }
                }
                Message::Close(frame) => {
                    let _ = outbox.send(Message::Close(frame)).await;
                    return SessionEnd::UpstreamClosed;
                }
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
        let _ = outbox
            .send(close_message(
                CloseCode::Error,
                "codex app-server upstream closed",
            ))
            .await;
        SessionEnd::UpstreamClosed
    };

    let upstream_restarted = async {
        // 송신자가 사라져도(정지) 세션을 끝내야 하므로 결과와 관계없이 재시작으로 본다.
        let _ = generation.changed().await;
        SessionEnd::UpstreamRestarted
    };

    let end = tokio::select! {
        end = client_to_upstream => end,
        end = upstream_to_client => end,
        end = upstream_restarted => end,
    };
    if end == SessionEnd::UpstreamRestarted {
        let _ = outbox
            .send(close_message(
                CloseCode::Restart,
                "codex app-server upstream restarted",
            ))
            .await;
    }
    drop(outbox);
    let _ = tokio::time::timeout(CLIENT_CLOSE_FLUSH_TIMEOUT, writer).await;
    tracing::debug!(?end, "gateway session ended");
    Ok(())
}

async fn open_upstream(
    upstream: &dyn GatewayUpstreamPort,
) -> Result<WebSocketStream<Box<dyn GatewayUpstreamStream>>> {
    let stream = upstream.open_session()?;
    let (websocket, _response) = tokio_tungstenite::client_async(UPSTREAM_WEBSOCKET_URL, stream)
        .await
        .context("upstream WebSocket handshake failed")?;
    Ok(websocket)
}

fn close_message(code: CloseCode, reason: &'static str) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: reason.into(),
    }))
}

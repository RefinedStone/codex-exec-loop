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
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};

use super::chats::{CHATS_METHOD_PREFIX, spawn_request as spawn_chats_request};
use super::context::{
    CONTEXT_METHOD_PREFIX, remote_host_response as remote_context_response,
    spawn_request as spawn_context_request,
};
use super::dots::{DOTS_METHOD_PREFIX, DotWatches, spawn_request as spawn_dots_request};
use super::git::{GIT_METHOD_PREFIX, GitPorts, spawn_request as spawn_git_request};
use super::hosts::{GatewayHosts, LOCAL_HOST_ID, host_id_from_path};
use super::issue::{ISSUE_METHOD_PREFIX, spawn_request as spawn_issue_request};
use super::issue_draft::{is_issue_draft_method, spawn_request as spawn_issue_draft_request};
use super::kernel::{KERNEL_METHOD_PREFIX, KernelRequests};
use super::mcp::{GatewayMcpHub, MCP_METHOD_PREFIX, spawn_request as spawn_mcp_request};
use super::recorder::{GatewayRecorder, RecordDirection, SessionRecording};
use super::resource_version::{self, ProviderSession};
use super::router::{
    ClientTextRoute, refuse_kernel_on_remote_host, respond_to_akra_request, route_client_text,
};
use crate::application::port::inbound::git_graph_port::GitGraphPort;
use crate::application::port::inbound::git_operation_port::GitOperationPort;
use crate::application::port::inbound::issue_draft_port::IssueDraftPort;
use crate::application::port::inbound::issue_handoff_port::IssueHandoffPort;
use crate::application::port::inbound::work_context_port::WorkContextPort;
use crate::application::port::outbound::chats_port::ChatsPort;
use crate::application::port::outbound::dots_port::DotsPort;
use crate::application::port::outbound::gateway_upstream_port::{
    GatewayUpstreamPort, GatewayUpstreamStream,
};
use crate::composition::repository_kernel::RepositoryKernelRegistry;

/// proxy 는 바이트만 전달하므로 호스트 이름은 의미가 없다. app-server 의 WebSocket 경로이다.
const UPSTREAM_WEBSOCKET_URL: &str = "ws://localhost/rpc";
const CLIENT_OUTBOX_CAPACITY: usize = 256;
const CLIENT_CLOSE_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
/// `thread/resume` 응답은 긴 스레드의 모든 아이템(명령 출력 포함)을 담아 tungstenite 기본 한도(메시지 64 MiB,
/// 프레임 16 MiB)를 넘을 수 있다. 넘으면 읽기가 오류로 끝나 세션이 닫힌다.
/// shortcut: 고정 상한 256 MiB, 이보다 큰 스레드가 나오면 페이지 단위 읽기(excludeTurns, thread/items/list)로 바꾼다.
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 256 << 20;

pub(super) fn websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_WEBSOCKET_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_WEBSOCKET_MESSAGE_BYTES))
}

/// Gateway 가 직접 답하는 `akra/*` 서비스들이다. 없는 서비스의 메서드는 모르는 메서드로 답한다. 테스트는 필요한
/// 것만 채운다.
#[derive(Clone, Default)]
pub(super) struct GatewayServices {
    pub(super) mcp_hub: Option<Arc<GatewayMcpHub>>,
    pub(super) work_context: Option<Arc<dyn WorkContextPort>>,
    pub(super) resource_versions:
        Option<Arc<dyn crate::application::port::inbound::resource_version_provider_port::ResourceVersionProviderPort>>,
    /// 커밋 그래프이다 (plan 27). 이 기기의 저장소를 읽으므로 `local` 접속에만 답한다.
    pub(super) git_graph: Option<Arc<dyn GitGraphPort>>,
    /// git 작업 엔진이다 (plan 27 G3). `git_graph` 와 같은 `akra/git/*` 에서 답한다. `local` 접속에만 답한다.
    pub(super) git_operations: Option<Arc<dyn GitOperationPort>>,
    /// 이슈 인계의 공유본이다 (#2428). 이 기기의 정책 파일과 `gh` 를 쓰므로 `local` 접속에만 답한다.
    pub(super) issue_handoff: Option<Arc<dyn IssueHandoffPort>>,
    /// 이슈 초안이다 (#2426). 이 기기의 정책 파일과 `gh` 를 쓰므로 `local` 접속에만 답한다.
    pub(super) issue_drafts: Option<Arc<dyn IssueDraftPort>>,
    /// dots 가 켜졌을 때만 있다 (D-19). 없으면 `akra/dots/*` 는 모르는 method 이다.
    pub(super) dots: Option<Arc<dyn DotsPort>>,
    /// ChatGPT 개인 대화 조회이다. dots 와 같은 조건으로 켜진다 (D-19, plan 25). 없으면 `akra/chats/*` 는 모르는 method 이다.
    pub(super) chats: Option<Arc<dyn ChatsPort>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionEnd {
    ClientClosed,
    UpstreamClosed,
    UpstreamRestarted,
}

pub(super) async fn run_session<S>(
    client: S,
    hosts: GatewayHosts,
    kernels: Arc<RepositoryKernelRegistry>,
    services: GatewayServices,
    recorder: Option<Arc<GatewayRecorder>>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut request_path = String::new();
    // 오류 타입은 tungstenite 의 `Callback` 이 정한다. 이 콜백은 오류를 만들지 않는다.
    #[allow(clippy::result_large_err)]
    let read_path = |request: &Request, response: Response| {
        request_path = request.uri().path().to_string();
        Ok(response)
    };
    let client_websocket = tokio_tungstenite::accept_hdr_async_with_config(
        client,
        read_path,
        Some(websocket_config()),
    )
    .await
    .context("client WebSocket handshake failed")?;
    let (mut client_sink, mut client_stream) = client_websocket.split();
    let host_id = host_id_from_path(&request_path);
    let Some(upstream) = hosts.find(host_id) else {
        let _ = client_sink
            .send(close_message(CloseCode::Policy, "unknown gateway host"))
            .await;
        return Ok(());
    };
    let is_local = host_id == LOCAL_HOST_ID;
    // 원격 대화는 디스크에 남기지 않는다 (plan 25 §3.1).
    let recording = is_local
        .then(|| open_recording(recorder.as_deref()))
        .flatten()
        .map(Arc::new);
    let record = |direction: RecordDirection, text: &str| {
        if let Some(recording) = &recording {
            recording.record(direction, text);
        }
    };
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
    // 커널 요청은 따로 처리한다. 읽기 고리는 커널을 기다리지 않고 app-server 메시지를 계속 전달한다.
    let kernel_requests =
        is_local.then(|| KernelRequests::start(kernels, outbox.clone(), recording.clone()));
    let provider_session = if is_local {
        services
            .resource_versions
            .as_ref()
            .map(|service| ProviderSession::start(service.clone(), outbox.clone()))
            .transpose()?
    } else {
        None
    };

    let client_to_upstream = async {
        // 이 접속이 보는 dot 방들이다. 접속이 끝나면 함께 멈춘다.
        let mut dot_watches = DotWatches::default();
        while let Some(result) = client_stream.next().await {
            let message = match result {
                Ok(message) => message,
                Err(error) => {
                    tracing::warn!(error = %error, "gateway client read failed");
                    return SessionEnd::ClientClosed;
                }
            };
            match message {
                Message::Text(text) => match route_client_text(text.as_str()) {
                    ClientTextRoute::Forward => {
                        if let Some(id) = resource_version::response_id(text.as_str()) {
                            if let Some(provider) = &provider_session {
                                provider.answer(&id, text.as_str());
                            }
                            continue;
                        }
                        record(RecordDirection::Client, text.as_str());
                        if upstream_sink.send(Message::Text(text)).await.is_err() {
                            return SessionEnd::UpstreamClosed;
                        }
                    }
                    ClientTextRoute::AkraRequest { id, method } => {
                        // 이슈 초안 요청에는 가리기 전의 로컬 글이 있다. 녹화에 남기지 않는다 (#2426).
                        if !is_issue_draft_method(&method) {
                            record(RecordDirection::Client, text.as_str());
                        }
                        let mcp_hub_for_method = services
                            .mcp_hub
                            .as_ref()
                            .filter(|_| method.starts_with(MCP_METHOD_PREFIX));
                        let context_for_method = services
                            .work_context
                            .as_ref()
                            .filter(|_| method.starts_with(CONTEXT_METHOD_PREFIX));
                        let git_for_method = services
                            .git_graph
                            .as_ref()
                            .filter(|_| is_local && method.starts_with(GIT_METHOD_PREFIX));
                        let issue_for_method = services
                            .issue_handoff
                            .as_ref()
                            .filter(|_| is_local && method.starts_with(ISSUE_METHOD_PREFIX));
                        let dots_for_method = services
                            .dots
                            .as_ref()
                            .filter(|_| method.starts_with(DOTS_METHOD_PREFIX));
                        let chats_for_method = services
                            .chats
                            .as_ref()
                            .filter(|_| method.starts_with(CHATS_METHOD_PREFIX));
                        let drafts_for_method = services
                            .issue_drafts
                            .as_ref()
                            .filter(|_| is_local && is_issue_draft_method(&method));
                        let immediate = if let Some(drafts) = drafts_for_method {
                            spawn_issue_draft_request(
                                drafts.clone(),
                                id,
                                method,
                                text.as_str().to_string(),
                                outbox.clone(),
                            );
                            None
                        } else if method.starts_with(resource_version::METHOD_PREFIX) {
                            Some(match &provider_session {
                                Some(provider) => provider.request(id, &method, text.as_str()),
                                None => refuse_kernel_on_remote_host(id, &method),
                            })
                        } else if method.starts_with(KERNEL_METHOD_PREFIX) {
                            match &kernel_requests {
                                Some(kernel_requests) => kernel_requests
                                    .submit(id, method, text.as_str().to_string())
                                    .err(),
                                None => Some(refuse_kernel_on_remote_host(id, &method)),
                            }
                        } else if !is_local && method.starts_with(CONTEXT_METHOD_PREFIX) {
                            Some(remote_context_response(id))
                        } else if let Some(response) = dots_for_method.and_then(|dots| {
                            dot_watches.handle(dots.as_ref(), &id, &method, text.as_str(), &outbox)
                        }) {
                            Some(response)
                        } else if let Some(dots) = dots_for_method {
                            spawn_dots_request(
                                dots.clone(),
                                id,
                                method,
                                text.as_str().to_string(),
                                outbox.clone(),
                            );
                            None
                        } else if let Some(chats) = chats_for_method {
                            spawn_chats_request(
                                chats.clone(),
                                id,
                                method,
                                text.as_str().to_string(),
                                outbox.clone(),
                            );
                            None
                        } else if let Some(hub) = mcp_hub_for_method {
                            spawn_mcp_request(
                                hub.clone(),
                                id,
                                method,
                                text.as_str().to_string(),
                                outbox.clone(),
                                recording.clone(),
                            );
                            None
                        } else if let Some(graph) = git_for_method {
                            spawn_git_request(
                                GitPorts {
                                    graph: graph.clone(),
                                    operations: services.git_operations.clone(),
                                },
                                id,
                                method,
                                text.as_str().to_string(),
                                outbox.clone(),
                                recording.clone(),
                            );
                            None
                        } else if let Some(handoff) = issue_for_method {
                            spawn_issue_request(
                                handoff.clone(),
                                id,
                                method,
                                text.as_str().to_string(),
                                outbox.clone(),
                                recording.clone(),
                            );
                            None
                        } else if let Some(context) = context_for_method {
                            spawn_context_request(
                                context.clone(),
                                id,
                                method,
                                text.as_str().to_string(),
                                outbox.clone(),
                                recording.clone(),
                            );
                            None
                        } else {
                            Some(respond_to_akra_request(
                                id,
                                &method,
                                host_id,
                                &hosts.snapshots(),
                            ))
                        };
                        if let Some(response) = immediate {
                            record(RecordDirection::Gateway, &response);
                            if outbox.send(Message::text(response)).await.is_err() {
                                return SessionEnd::ClientClosed;
                            }
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
        while let Some(result) = upstream_stream.next().await {
            let message = match result {
                Ok(message) => message,
                Err(error) => {
                    tracing::warn!(error = %error, "gateway upstream read failed");
                    break;
                }
            };
            match message {
                Message::Text(_) | Message::Binary(_) => {
                    if let Message::Text(text) = &message {
                        record(RecordDirection::Gateway, text.as_str());
                    }
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
    drop(provider_session);
    // Kernel workers can hold additional outbox senders; stop them before flushing.
    if let Some(kernel_requests) = kernel_requests {
        kernel_requests.stop().await;
    }
    finish_client_writer(end, outbox, writer).await;
    tracing::debug!(?end, "gateway session ended");
    Ok(())
}

/// The close enqueue and the writer flush share ONE bounded shutdown deadline.
/// A blocked client's transport can fill the outbox and stall both operations.
async fn finish_client_writer(
    end: SessionEnd,
    outbox: mpsc::Sender<Message>,
    mut writer: tokio::task::JoinHandle<()>,
) {
    let shutdown = async {
        if end == SessionEnd::UpstreamRestarted {
            let _ = outbox
                .send(close_message(
                    CloseCode::Restart,
                    "codex app-server upstream restarted",
                ))
                .await;
        }
        drop(outbox);
        let _ = (&mut writer).await;
    };
    if tokio::time::timeout(CLIENT_CLOSE_FLUSH_TIMEOUT, shutdown)
        .await
        .is_err()
    {
        // Dropping a JoinHandle only detaches the task. Abort and await it to
        // release the client sink even if its write operation never completes.
        writer.abort();
        let _ = writer.await;
    }
}

fn open_recording(recorder: Option<&GatewayRecorder>) -> Option<SessionRecording> {
    match recorder?.open_session() {
        Ok(recording) => Some(recording),
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), "gateway recording disabled for session");
            None
        }
    }
}

async fn open_upstream(
    upstream: &dyn GatewayUpstreamPort,
) -> Result<WebSocketStream<Box<dyn GatewayUpstreamStream>>> {
    let stream = upstream.open_session()?;
    let (websocket, _response) = tokio_tungstenite::client_async_with_config(
        UPSTREAM_WEBSOCKET_URL,
        stream,
        Some(websocket_config()),
    )
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

#[cfg(test)]
mod writer_shutdown_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use tokio::io::AsyncWriteExt;
    use tokio::sync::{mpsc, oneshot};
    use tokio_tungstenite::tungstenite::Message;

    use super::{CLIENT_CLOSE_FLUSH_TIMEOUT, SessionEnd, finish_client_writer};

    struct WriterDropWitness(Arc<AtomicBool>);

    impl Drop for WriterDropWitness {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn full_outbox_and_blocked_client_writer_are_aborted_at_shutdown_deadline() {
        let (mut blocked_stream, _unread_client) = tokio::io::duplex(1);
        let (outbox, mut inbox) = mpsc::channel::<Message>(1);
        outbox.send(Message::text("queued")).await.unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let writer_dropped = dropped.clone();
        let (writer_started, started) = oneshot::channel();
        let writer = tokio::spawn(async move {
            let _witness = WriterDropWitness(writer_dropped);
            let _ = writer_started.send(());
            // A client which never reads holds the writer here permanently.
            let _ = blocked_stream.write_all(&vec![0u8; 4096]).await;
            let _ = inbox.recv().await;
        });
        started.await.unwrap();
        let began = Instant::now();

        finish_client_writer(SessionEnd::UpstreamRestarted, outbox, writer).await;

        assert!(
            began.elapsed() <= CLIENT_CLOSE_FLUSH_TIMEOUT + Duration::from_secs(1),
            "session shutdown must include close enqueue and writer cancellation"
        );
        assert!(
            dropped.load(Ordering::SeqCst),
            "writer task must be aborted, not detached"
        );
    }
}

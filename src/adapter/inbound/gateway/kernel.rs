//! Gateway 접속 하나의 `akra/kernel/*` 처리이다 (plan 18 K3-5). 접속은 `attach` 로 작업 공간이 속한 저장소의
//! 커널에 붙는다. 그 뒤 명령과 조회를 보내고, 커널 이벤트를 `akra/kernel/event` 알림으로 받는다. 직렬화와 core
//! 변환은 `kernel_protocol` 이 한다. 이 모듈은 전송만 한다.
//!
//! 커널 요청은 접속마다 작업 하나가 받은 차례대로 처리한다. 그래서 접속의 읽기 고리는 커널을 기다리지 않고
//! app-server 메시지(승인 답, 중단)를 계속 전달한다. 커널 호출은 블로킹이고 시간 상한이 있다. 상한을 넘으면
//! JSON-RPC 오류로 답한다. 이벤트는 접속마다 전용 스레드가 커널 핸들에서 받아 접속의 보내기 함으로 넣는다.
//! `detach` 와 다시 붙기는 그 스레드가 끝날 때까지 기다린 뒤 답한다. 그래서 떼어 낸다는 답 뒤에는 이벤트가 오지
//! 않고, 다시 붙어도 이벤트가 두 번 오지 않는다. 커널은 클라이언트가 모두 떠나도 남는다 (§3.4).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::thread::JoinHandle;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio_tungstenite::tungstenite::Message;

use super::recorder::{RecordDirection, SessionRecording};
use crate::adapter::inbound::kernel_protocol::{
    AttachRequest, AttachResponse, CommandRequest, CommandResponse, ERROR_INVALID_PARAMS,
    ERROR_KERNEL_TIMEOUT, ERROR_KERNEL_UNAVAILABLE, ERROR_NOT_ATTACHED, EventNotification,
    KernelEvent, KernelProtocolError, METHOD_ATTACH, METHOD_COMMAND, METHOD_DESCRIBE,
    METHOD_DETACH, METHOD_EVENT, METHOD_QUERY, PROTOCOL_VERSION, QueryRequest, describe,
};
use crate::composition::kernel_host::{
    KernelCallError, KernelClientHandle, KernelCommandSender, KernelHostEvent,
};
use crate::composition::repository_kernel::RepositoryKernelRegistry;

pub(super) const KERNEL_METHOD_PREFIX: &str = "akra/kernel/";
const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
/// 이벤트 스레드가 멈춤 표시를 확인하는 간격이다. `detach` 는 커널이 이벤트 채널을 닫게 하므로 보통 더 빨리 끝난다.
const EVENT_POLL: Duration = Duration::from_millis(200);
/// 보내기 함이 찼을 때 이벤트 스레드가 다시 보내 보는 간격이다. 그 사이에 멈춤 표시를 확인한다.
const EVENT_SEND_RETRY: Duration = Duration::from_millis(20);
/// 한 접속에서 처리를 기다릴 수 있는 커널 요청 수이다. 넘으면 바로 오류로 답한다.
const KERNEL_REQUEST_QUEUE_CAPACITY: usize = 64;
/// 붙기를 기다리는 최대 시간이다. 처음 붙을 때 커널을 만들며 git 과 planning 기준 데이터를 읽는다.
const KERNEL_ATTACH_TIMEOUT: Duration = Duration::from_secs(60);
/// 명령과 조회를 기다리는 최대 시간이다. 커널은 보통 한 주기(50ms) 안에 답한다.
const KERNEL_CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// 접속의 커널 요청 하나이다.
struct KernelRequest {
    id: Value,
    method: String,
    text: String,
}

/// 접속의 커널 요청 처리 작업이다. 요청을 받은 차례대로 처리하고 답을 접속의 보내기 함으로 보낸다.
pub(super) struct KernelRequests {
    sender: mpsc::Sender<KernelRequest>,
    worker: tokio::task::JoinHandle<()>,
}

impl KernelRequests {
    pub(super) fn start(
        kernels: Arc<RepositoryKernelRegistry>,
        outbox: mpsc::Sender<Message>,
        recording: Option<Arc<SessionRecording>>,
    ) -> Self {
        let (sender, requests) = mpsc::channel(KERNEL_REQUEST_QUEUE_CAPACITY);
        let session = KernelSession {
            kernels,
            outbox,
            attached: None,
        };
        let worker = tokio::spawn(serve_kernel_requests(session, requests, recording));
        Self { sender, worker }
    }

    /// 요청을 처리 차례에 넣는다. 기다리는 요청이 너무 많거나 작업이 끝났으면 바로 보낼 오류 응답을 돌려준다.
    pub(super) fn submit(&self, id: Value, method: String, text: String) -> Result<(), String> {
        let request = KernelRequest { id, method, text };
        self.sender.try_send(request).map_err(|error| {
            let (request, reason) = match error {
                TrySendError::Full(request) => (
                    request,
                    "too many kernel requests are waiting on this connection",
                ),
                TrySendError::Closed(request) => (
                    request,
                    "the kernel request handler of this connection stopped",
                ),
            };
            response_text(request.id, Err(unavailable(reason.to_string())))
        })
    }

    /// 접속이 끝날 때 부른다. 처리 중인 요청은 버린다. 붙어 있던 커널에서는 떨어진다.
    pub(super) async fn stop(self) {
        drop(self.sender);
        self.worker.abort();
        let _ = self.worker.await;
    }
}

async fn serve_kernel_requests(
    mut session: KernelSession,
    mut requests: mpsc::Receiver<KernelRequest>,
    recording: Option<Arc<SessionRecording>>,
) {
    while let Some(request) = requests.recv().await {
        let response = session
            .respond(request.id, &request.method, &request.text)
            .await;
        if let Some(recording) = &recording {
            recording.record(RecordDirection::Gateway, &response);
        }
        if session.outbox.send(Message::text(response)).await.is_err() {
            break;
        }
    }
    session.detach().await;
}

struct KernelSession {
    kernels: Arc<RepositoryKernelRegistry>,
    outbox: mpsc::Sender<Message>,
    attached: Option<AttachedKernel>,
}

struct AttachedKernel {
    workspace: String,
    commands: KernelCommandSender,
    stop_events: Arc<AtomicBool>,
    events: Option<JoinHandle<()>>,
}

impl AttachedKernel {
    /// 이벤트 스레드를 멈추게 하고 커널에서 뗀다. 스레드가 끝나기를 기다리지 않는다.
    fn signal_detach(&self) {
        self.stop_events.store(true, Ordering::SeqCst);
        self.commands.detach();
    }
}

impl Drop for AttachedKernel {
    /// 접속이 갑자기 끝나도 커널에서 떨어진다. 이벤트 스레드는 커널이 채널을 닫으면 바로 끝난다.
    fn drop(&mut self) {
        self.signal_detach();
    }
}

impl KernelSession {
    /// `akra/kernel/*` 요청 하나에 대한 JSON-RPC 응답 문자열이다.
    async fn respond(&mut self, id: Value, method: &str, text: &str) -> String {
        let params = serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|message| message.get("params").cloned())
            .unwrap_or_else(|| json!({}));
        let result = match method {
            METHOD_DESCRIBE => to_result(describe()),
            METHOD_ATTACH => self.attach(params).await,
            METHOD_COMMAND => self.command(params).await,
            METHOD_QUERY => self.query(params).await,
            METHOD_DETACH => {
                self.detach().await;
                Ok(json!({}))
            }
            _ => Err(KernelProtocolError {
                code: JSON_RPC_METHOD_NOT_FOUND,
                message: format!("unknown kernel method `{method}`"),
            }),
        };
        response_text(id, result)
    }

    async fn attach(&mut self, params: Value) -> Result<Value, KernelProtocolError> {
        let request: AttachRequest = parse_params(params)?;
        // 다시 붙으면 이전 붙음을 먼저 뗀다. 이전 이벤트 스레드가 끝난 뒤 새로 붙으므로 이벤트가 겹치지 않는다.
        self.detach().await;
        let kernels = self.kernels.clone();
        let workspace = request.workspace.clone();
        let attached = tokio::time::timeout(
            KERNEL_ATTACH_TIMEOUT,
            blocking(move || kernels.attach(&workspace)),
        )
        .await
        .map_err(|_| timed_out("attach", KERNEL_ATTACH_TIMEOUT))?;
        let (client, kernel) = attached?.map_err(|error| unavailable(format!("{error:#}")))?;
        tracing::info!(
            client_name = %request.client_name,
            client_id = client.client_id(),
            "kernel client attached"
        );
        let response = AttachResponse {
            client_id: client.client_id().to_string(),
            protocol_version: PROTOCOL_VERSION,
            kernel,
        };
        let stop_events = Arc::new(AtomicBool::new(false));
        let commands = client.commands();
        let outbox = self.outbox.clone();
        let stop = stop_events.clone();
        let events = std::thread::Builder::new()
            .name("akra-gateway-kernel-events".to_string())
            .spawn(move || forward_events(client, outbox, stop))
            .map_err(|error| unavailable(format!("failed to start the event thread: {error}")))?;
        self.attached = Some(AttachedKernel {
            workspace: request.workspace,
            commands,
            stop_events,
            events: Some(events),
        });
        to_result(response)
    }

    async fn command(&mut self, params: Value) -> Result<Value, KernelProtocolError> {
        let attached = self.attached()?;
        let request: CommandRequest = parse_params(params)?;
        let command = request.command.into_core(&attached.workspace)?;
        let commands = attached.commands.clone();
        let events =
            match blocking(move || commands.send_command(command, KERNEL_CALL_TIMEOUT)).await? {
                Ok(events) => Some(events),
                Err(KernelCallError::Stopped) => None,
                Err(KernelCallError::TimedOut) => {
                    return Err(timed_out("command", KERNEL_CALL_TIMEOUT));
                }
            };
        to_result(CommandResponse::from_core(events.as_deref()))
    }

    async fn query(&mut self, params: Value) -> Result<Value, KernelProtocolError> {
        let attached = self.attached()?;
        let request: QueryRequest = parse_params(params)?;
        let commands = attached.commands.clone();
        let view = blocking(move || commands.query(KERNEL_CALL_TIMEOUT))
            .await?
            .map_err(|error| match error {
                KernelCallError::Stopped => unavailable(error.to_string()),
                KernelCallError::TimedOut => timed_out("query", KERNEL_CALL_TIMEOUT),
            })?;
        to_result(request.query.answer(&view.snapshot, &view.thread_activity))
    }

    /// 커널에서 떨어지고, 이벤트 스레드가 끝날 때까지 기다린다. 그 뒤에는 이 접속에 커널 이벤트가 가지 않는다.
    async fn detach(&mut self) {
        let Some(mut attached) = self.attached.take() else {
            return;
        };
        attached.signal_detach();
        if let Some(events) = attached.events.take() {
            let _ = tokio::task::spawn_blocking(move || events.join()).await;
        }
    }

    fn attached(&self) -> Result<&AttachedKernel, KernelProtocolError> {
        self.attached.as_ref().ok_or_else(|| KernelProtocolError {
            code: ERROR_NOT_ATTACHED,
            message: "attach to a kernel first (akra/kernel/attach)".to_string(),
        })
    }
}

/// 커널 이벤트를 `akra/kernel/event` 알림으로 접속에 보낸다. 멈춤 표시, 커널 종료(채널 닫힘), 접속 종료 가운데
/// 하나가 오면 끝나고 핸들을 버린다. 멈춤 표시를 본 뒤에는 이벤트를 보내지 않는다.
fn forward_events(
    client: KernelClientHandle,
    outbox: mpsc::Sender<Message>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::SeqCst) {
        let event = match client.recv_event(EVENT_POLL) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let Some(event) = protocol_event(event) else {
            continue;
        };
        let notification = json!({
            "jsonrpc": "2.0",
            "method": METHOD_EVENT,
            "params": EventNotification { event },
        });
        if !send_unless_stopped(&outbox, Message::text(notification.to_string()), &stop) {
            return;
        }
    }
}

/// 보내기 함에 넣는다. 함이 차 있으면 멈춤 표시를 보며 다시 넣는다. 멈췄거나 접속이 끝났으면 `false` 이다.
fn send_unless_stopped(
    outbox: &mpsc::Sender<Message>,
    mut message: Message,
    stop: &AtomicBool,
) -> bool {
    loop {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        match outbox.try_send(message) {
            Ok(()) => return true,
            Err(TrySendError::Full(returned)) => {
                message = returned;
                std::thread::sleep(EVENT_SEND_RETRY);
            }
            Err(TrySendError::Closed(_)) => return false,
        }
    }
}

/// v0 이벤트로 바꾼다. 병렬 제어부 이벤트는 K3-6 에서 더한다.
fn protocol_event(event: KernelHostEvent) -> Option<KernelEvent> {
    match event {
        KernelHostEvent::Core(event) => KernelEvent::from_core(&event),
        KernelHostEvent::Parallel(_) => None,
        KernelHostEvent::Resync => Some(KernelEvent::Resync),
    }
}

fn response_text(id: Value, result: Result<Value, KernelProtocolError>) -> String {
    let response = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": error.code, "message": error.message },
        }),
    };
    response.to_string()
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, KernelProtocolError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| unavailable(format!("kernel call failed: {error}")))
}

fn parse_params<T: DeserializeOwned>(params: Value) -> Result<T, KernelProtocolError> {
    serde_json::from_value(params).map_err(|error| KernelProtocolError {
        code: ERROR_INVALID_PARAMS,
        message: error.to_string(),
    })
}

fn to_result(value: impl serde::Serialize) -> Result<Value, KernelProtocolError> {
    serde_json::to_value(value).map_err(|error| unavailable(error.to_string()))
}

fn unavailable(message: String) -> KernelProtocolError {
    KernelProtocolError {
        code: ERROR_KERNEL_UNAVAILABLE,
        message,
    }
}

fn timed_out(call: &str, timeout: Duration) -> KernelProtocolError {
    KernelProtocolError {
        code: ERROR_KERNEL_TIMEOUT,
        message: format!(
            "the kernel did not answer the {call} within {timeout:?}; query the kernel to see whether it ran"
        ),
    }
}

//! Gateway 접속 하나의 `akra/kernel/*` 처리이다 (plan 18 K3-5). 접속은 `attach` 로 작업 공간이 속한 저장소의
//! 커널에 붙는다. 그 뒤 명령과 조회를 보내고, 커널 이벤트를 `akra/kernel/event` 알림으로 받는다. 직렬화와 core
//! 변환은 `kernel_protocol` 이 한다. 이 모듈은 전송만 한다.
//!
//! 커널 호출은 블로킹이다(명령은 커널이 처리할 때까지 기다린다). 그래서 `spawn_blocking` 으로 부른다. 이벤트는
//! 접속마다 전용 스레드가 커널 핸들에서 받아 접속의 보내기 함으로 넣는다. 접속이 끝나거나 `detach` 하면 그 스레드가
//! 멈추고 핸들을 버려 커널에서 떨어진다. 커널은 클라이언트가 모두 떠나도 남는다 (§3.4).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::adapter::inbound::kernel_protocol::{
    AttachRequest, AttachResponse, CommandRequest, CommandResponse, ERROR_INVALID_PARAMS,
    ERROR_KERNEL_UNAVAILABLE, ERROR_NOT_ATTACHED, EventNotification, KernelEvent,
    KernelProtocolError, METHOD_ATTACH, METHOD_COMMAND, METHOD_DESCRIBE, METHOD_DETACH,
    METHOD_EVENT, METHOD_QUERY, PROTOCOL_VERSION, QueryRequest, describe,
};
use crate::composition::kernel_host::{KernelClientHandle, KernelCommandSender, KernelHostEvent};
use crate::composition::repository_kernel::RepositoryKernelRegistry;

pub(super) const KERNEL_METHOD_PREFIX: &str = "akra/kernel/";
const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
/// 이벤트 스레드가 멈춤 표시를 확인하는 간격이다. 접속이 끝난 뒤 커널에서 떨어지기까지 최대 이만큼 걸린다.
const EVENT_POLL: Duration = Duration::from_millis(200);

pub(super) struct KernelSession {
    kernels: Arc<RepositoryKernelRegistry>,
    outbox: mpsc::Sender<Message>,
    attached: Option<AttachedKernel>,
}

struct AttachedKernel {
    workspace: String,
    commands: KernelCommandSender,
    stop_events: Arc<AtomicBool>,
}

impl Drop for AttachedKernel {
    fn drop(&mut self) {
        self.stop_events.store(true, Ordering::SeqCst);
    }
}

impl KernelSession {
    pub(super) fn new(
        kernels: Arc<RepositoryKernelRegistry>,
        outbox: mpsc::Sender<Message>,
    ) -> Self {
        Self {
            kernels,
            outbox,
            attached: None,
        }
    }

    /// `akra/kernel/*` 요청 하나에 대한 JSON-RPC 응답 문자열이다.
    pub(super) async fn respond(&mut self, id: Value, method: &str, text: &str) -> String {
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
                self.attached = None;
                Ok(json!({}))
            }
            _ => Err(KernelProtocolError {
                code: JSON_RPC_METHOD_NOT_FOUND,
                message: format!("unknown kernel method `{method}`"),
            }),
        };
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

    async fn attach(&mut self, params: Value) -> Result<Value, KernelProtocolError> {
        let request: AttachRequest = parse_params(params)?;
        let kernels = self.kernels.clone();
        let workspace = request.workspace.clone();
        let (client, kernel) = blocking(move || kernels.attach(&workspace))
            .await?
            .map_err(|error| unavailable(format!("{error:#}")))?;
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
        std::thread::Builder::new()
            .name("akra-gateway-kernel-events".to_string())
            .spawn(move || forward_events(client, outbox, stop))
            .map_err(|error| unavailable(format!("failed to start the event thread: {error}")))?;
        // 다시 붙으면 이전 붙음은 버린다. 그 이벤트 스레드는 멈추고 이전 커널에서 떨어진다.
        self.attached = Some(AttachedKernel {
            workspace: request.workspace,
            commands,
            stop_events,
        });
        to_result(response)
    }

    async fn command(&mut self, params: Value) -> Result<Value, KernelProtocolError> {
        let attached = self.attached()?;
        let request: CommandRequest = parse_params(params)?;
        let command = request.command.into_core(&attached.workspace)?;
        let commands = attached.commands.clone();
        let events = blocking(move || commands.send_command(command)).await?;
        to_result(CommandResponse::from_core(events.as_deref()))
    }

    async fn query(&mut self, params: Value) -> Result<Value, KernelProtocolError> {
        let attached = self.attached()?;
        let request: QueryRequest = parse_params(params)?;
        let commands = attached.commands.clone();
        let view = blocking(move || commands.query())
            .await?
            .ok_or_else(|| unavailable("the kernel stopped".to_string()))?;
        to_result(request.query.answer(&view.snapshot, &view.thread_activity))
    }

    fn attached(&self) -> Result<&AttachedKernel, KernelProtocolError> {
        self.attached.as_ref().ok_or_else(|| KernelProtocolError {
            code: ERROR_NOT_ATTACHED,
            message: "attach to a kernel first (akra/kernel/attach)".to_string(),
        })
    }
}

/// 커널 이벤트를 `akra/kernel/event` 알림으로 접속에 보낸다. 멈춤 표시, 커널 종료, 접속 종료 가운데 하나가
/// 오면 끝나고 핸들을 버린다.
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
        if outbox
            .blocking_send(Message::text(notification.to_string()))
            .is_err()
        {
            return;
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

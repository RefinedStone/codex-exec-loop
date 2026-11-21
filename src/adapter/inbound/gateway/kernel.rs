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

use super::confirmation::ConfirmationRegistration;
use super::delegation::{self, Delegations};
use super::recorder::{RecordDirection, SessionRecording};
use crate::adapter::inbound::desktop_confirmation_bridge::{
    protocol::valid_intent, storage::random_id,
};
use crate::adapter::inbound::kernel_protocol::{
    AttachRequest, AttachResponse, CommandRejection, CommandRequest, CommandResponse,
    CommandResult, ERROR_INVALID_PARAMS, ERROR_KERNEL_TIMEOUT, ERROR_KERNEL_UNAVAILABLE,
    ERROR_METHOD_NOT_FOUND, ERROR_NOT_ATTACHED, EventNotification, IssueAutomationPolicyState,
    IssueAutomationState, IssueState, IssuesState, KernelCommandInput, KernelEvent,
    KernelProtocolError, KernelQuery, KernelQueryAnswer, METHOD_ATTACH, METHOD_COMMAND,
    METHOD_DESCRIBE, METHOD_DETACH, METHOD_EVENT, METHOD_QUERY, PROTOCOL_VERSION, ParallelSource,
    ParallelState, PlanningState, QueryRequest, describe, reason_next,
};
use crate::adapter::inbound::kernel_protocol::{
    ConfirmationPrepareRequest, ConfirmationPrepareResponse, ConfirmationRegisterRequest,
    ConfirmationRegisterResponse, ConfirmedCommandRequest, METHOD_COMMAND_CONFIRMED,
    METHOD_CONFIRMATION_PREPARE, METHOD_CONFIRMATION_REGISTER, is_confirmation_method,
};
use crate::application::port::inbound::stored_command_port::StoredCommand;
use crate::application::port::inbound::stored_command_port::issue_confirmation::{
    IssueConfirmationBinding, IssueConfirmationIntent, IssueConfirmationTarget,
    VerifiedHumanConfirmation,
};
use crate::composition::kernel_host::{
    KernelCallError, KernelClientHandle, KernelCommandOutcome, KernelCommandSender,
    KernelHostEvent, ParallelHostView,
};
use crate::composition::native_client_runtime::{KernelClientCommand, StoredCommandEnd};
use crate::composition::production::{delegation_runtime, delegation_store};
use crate::composition::repository_kernel::RepositoryKernelRegistry;
use crate::domain::issue_automation::collection::IssueCollectionQuery;
use crate::domain::kernel_event::KernelCommandDescriptor;

pub(super) const KERNEL_METHOD_PREFIX: &str = "akra/kernel/";
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
        delegations: Arc<Delegations>,
    ) -> Self {
        let (sender, requests) = mpsc::channel(KERNEL_REQUEST_QUEUE_CAPACITY);
        let session = KernelSession {
            kernels,
            outbox,
            attached: None,
            connection_id: random_id(16).unwrap_or_default(),
            confirmation: None,
            delegations,
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
        if let Some(recording) = &recording
            && !is_confirmation_method(&request.method)
            && !delegation::private_kernel_payload(&request.method, &request.text)
        {
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
    connection_id: String,
    confirmation: Option<ConfirmationRegistration>,
    delegations: Arc<Delegations>,
}

struct AttachedKernel {
    workspace: String,
    /// 이슈 장부와 커서의 project scope다. 요청한 worktree와 다를 수 있다.
    issue_workspace: String,
    /// 이 접속이 attach 때 밝힌 이름이다. 커널이 명령의 기록에 호출자로 적는다 (plan 18 K4-5).
    client_name: String,
    client_id: String,
    attachment_id: String,
    commands: KernelCommandSender,
    stop_events: Arc<AtomicBool>,
    events: Option<JoinHandle<()>>,
}

impl AttachedKernel {
    fn issue_query(
        &self,
        query: &KernelQuery,
    ) -> Result<Option<IssueCollectionQuery>, KernelProtocolError> {
        Ok(query
            .issues_range(&self.issue_workspace)?
            .map(|(after, limit)| IssueCollectionQuery {
                binding_id: None,
                after,
                limit,
            }))
    }

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
            METHOD_CONFIRMATION_REGISTER => self.register_confirmation(params).await,
            METHOD_CONFIRMATION_PREPARE => self.prepare_confirmation(params).await,
            METHOD_COMMAND_CONFIRMED => self.command_confirmed(params).await,
            METHOD_QUERY => self.query(params).await,
            METHOD_DETACH => {
                self.detach().await;
                Ok(json!({}))
            }
            _ if is_confirmation_method(method) => Err(invalid_confirmation()),
            _ => Err(KernelProtocolError {
                code: ERROR_METHOD_NOT_FOUND,
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
            blocking(move || {
                let (client, kernel, workspace) =
                    kernels.attach_with_workspace_root(&workspace, request.transient)?;
                let runtime = delegation_runtime(&workspace)?;
                Ok::<_, anyhow::Error>((client, kernel, workspace, runtime))
            }),
        )
        .await
        .map_err(|_| timed_out("attach", KERNEL_ATTACH_TIMEOUT))?;
        let (client, kernel, issue_workspace, runtime) =
            attached?.map_err(|error| unavailable(format!("{error:#}")))?;
        if let Some(store) = delegation_store(runtime.clone(), true) {
            self.delegations
                .track(runtime, store, issue_workspace.clone());
        }
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
        let client_id = response.client_id.clone();
        let attachment_id = random_id(16).unwrap_or_default();
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
            issue_workspace,
            client_name: request.client_name,
            client_id,
            attachment_id,
            commands,
            stop_events,
            events: Some(events),
        });
        to_result(response)
    }

    async fn register_confirmation(&mut self, params: Value) -> Result<Value, KernelProtocolError> {
        let request: ConfirmationRegisterRequest = parse_confirmation_params(params)?;
        let attached = self.attached()?;
        let binding = IssueConfirmationBinding {
            registration_id: request.registration_id,
            connection_id: self.connection_id.clone(),
            attachment_id: attached.attachment_id.clone(),
            client_id: attached.client_id.clone(),
            workspace: attached.issue_workspace.clone(),
        };
        // Replace only after validating a new registration. The old watch is closed on drop.
        let registration = ConfirmationRegistration::register(
            binding.clone(),
            std::path::Path::new(&request.session_file),
        )
        .await
        .map_err(|_| invalid_confirmation())?;
        self.confirmation = Some(registration);
        to_result(ConfirmationRegisterResponse { binding })
    }

    async fn prepare_confirmation(&self, params: Value) -> Result<Value, KernelProtocolError> {
        let request: ConfirmationPrepareRequest = parse_confirmation_params(params)?;
        let attached = self.attached()?;
        let registration = self
            .confirmation
            .as_ref()
            .filter(|registration| registration.available())
            .ok_or_else(invalid_confirmation)?;
        let (command, descriptor) = confirmation_command(&request.request, &attached.workspace)?;
        let workspace = attached.issue_workspace.clone();
        let preview =
            blocking(move || crate::composition::issue_confirmation::prepare(&workspace, &command))
                .await?
                .map_err(|_| invalid_confirmation())?;
        if !registration.available() {
            return Err(invalid_confirmation());
        }
        let intent = IssueConfirmationIntent {
            binding: registration.binding().clone(),
            target: preview.target,
            key: descriptor
                .idempotency_key
                .ok_or_else(invalid_confirmation)?,
            fingerprint: descriptor.fingerprint,
        };
        if !valid_intent(&intent) {
            return Err(invalid_confirmation());
        }
        to_result(ConfirmationPrepareResponse {
            intent,
            display: preview.display,
        })
    }

    async fn command_confirmed(&self, params: Value) -> Result<Value, KernelProtocolError> {
        let submission: ConfirmedCommandRequest = parse_confirmation_params(params)?;
        let attached = self.attached()?;
        let registration = self
            .confirmation
            .as_ref()
            .filter(|registration| registration.available())
            .ok_or_else(invalid_confirmation)?;
        let (command, descriptor) = confirmation_command(&submission.request, &attached.workspace)?;
        let evidence = submission.confirmation;
        if evidence.registration_id != registration.binding().registration_id
            || evidence.intent.binding != *registration.binding()
            || descriptor.idempotency_key.as_deref() != Some(evidence.intent.key.as_str())
            || descriptor.fingerprint != evidence.intent.fingerprint
            || !target_matches_command(&evidence.intent.target, &command, &attached.issue_workspace)
        {
            return Err(invalid_confirmation());
        }
        // The original persisted basis stays in the proof. Fresh triage runs in the service,
        // where a changed basis closes the decision as superseded instead of losing that result here.
        let confirmation = registration
            .consume(&evidence.proof_id, &evidence.intent)
            .await
            .map_err(|_| invalid_confirmation())?;
        self.dispatch_command(submission.request, Some(confirmation))
            .await
    }

    async fn command(&mut self, params: Value) -> Result<Value, KernelProtocolError> {
        self.dispatch_command(parse_command_params(params)?, None)
            .await
    }

    async fn delegation_store(
        &self,
        workspace: &str,
    ) -> Result<delegation::DelegationStore, KernelProtocolError> {
        let root = workspace.to_owned();
        let runtime = blocking(move || delegation_runtime(&root))
            .await?
            .map_err(|error| unavailable(error.to_string()))?;
        let store = delegation_store(runtime.clone(), false).expect("store creation enabled");
        self.delegations
            .track(runtime, store.clone(), workspace.to_owned());
        Ok(store)
    }

    async fn dispatch_command(
        &self,
        request: CommandRequest,
        confirmation: Option<VerifiedHumanConfirmation>,
    ) -> Result<Value, KernelProtocolError> {
        let attached = self.attached()?;
        // 커널은 이 설명으로 명령을 기록하고 멱등 키로 재전송을 가린다 (plan 19 Q3-6).
        let command_name = request.command.kind().name();
        let caller = Some(attached.client_name.clone()).filter(|name| !name.trim().is_empty());
        let descriptor = request
            .descriptor()?
            .map(|descriptor| KernelCommandDescriptor {
                caller,
                ..descriptor
            });
        let request_key = request.idempotency_key;
        let command = match request.command.into_input(&attached.workspace)? {
            KernelCommandInput::Core(command) => KernelClientCommand::Core(command),
            KernelCommandInput::Stored(command) => match confirmation {
                Some(confirmation) => KernelClientCommand::ConfirmedStored {
                    command: Box::new(command),
                    confirmation: Box::new(confirmation),
                },
                None => KernelClientCommand::Stored(Box::new(command)),
            },
            KernelCommandInput::SyncIssues => KernelClientCommand::SyncIssues,
            KernelCommandInput::SubmitDelegation {
                instruction,
                idempotency_key,
                session,
            } => {
                let store = self.delegation_store(&attached.issue_workspace).await?;
                let caller = if attached.client_name
                    == crate::application::service::issue_review::ISSUE_DRAFT_REVIEW_FEATURE
                {
                    "manual".to_owned()
                } else {
                    attached.client_name.clone()
                };
                let workspace = attached.issue_workspace.clone();
                let digest = if session.selected_url().is_some() {
                    self.delegations
                        .account_id()
                        .await
                        .as_deref()
                        .map(delegation::account_fingerprint)
                } else {
                    None
                };
                let outcome = delegation::db(store, move |store| {
                    let route =
                        crate::composition::production::delegation_route(&workspace, "manual");
                    store.submit_session(
                        &idempotency_key,
                        &instruction,
                        &caller,
                        route,
                        session,
                        digest.as_ref().map(|value| value.as_slice()),
                    )
                })
                .await;
                let mut response = CommandResponse::from_core(Some(&[]));
                match outcome {
                    Ok(delegation) => {
                        response.result = Some(CommandResult::DelegationSubmitted { delegation })
                    }
                    Err(error) => response.failure = Some(error.to_string()),
                }
                return to_result(response);
            }
            KernelCommandInput::CancelDelegation { id } => {
                let store = self.delegation_store(&attached.issue_workspace).await?;
                let outcome = delegation::db(store, move |store| store.cancel(&id)).await;
                let mut response = CommandResponse::from_core(Some(&[]));
                match outcome {
                    Ok(Some(delegation)) => {
                        response.result = Some(CommandResult::DelegationCancelled { delegation })
                    }
                    Ok(None) => response.failure = Some("delegation not found".to_owned()),
                    Err(error) => response.failure = Some(error.to_string()),
                }
                return to_result(response);
            }
            KernelCommandInput::CheckpointDelegation { id, token, message } => {
                let store = self.delegation_store(&attached.issue_workspace).await?;
                return delegation_command_result(
                    delegation::db(store, move |store| store.checkpoint(&id, &token, &message))
                        .await,
                    |delegation| CommandResult::DelegationCheckpointed { delegation },
                    "delegation callback rejected",
                );
            }
            KernelCommandInput::ReportDelegationResult { id, token, result } => {
                let store = self.delegation_store(&attached.issue_workspace).await?;
                return delegation_command_result(
                    delegation::db(store, move |store| store.report(&id, &token, &result)).await,
                    |delegation| CommandResult::DelegationResultReported { delegation },
                    "delegation callback rejected",
                );
            }
            KernelCommandInput::RetryDelegation { id } => {
                let store = self.delegation_store(&attached.issue_workspace).await?;
                let key = request_key.expect("validated retry request key");
                return delegation_command_result(
                    delegation::db(store, move |store| store.retry(&id, &key)).await,
                    |delegation| CommandResult::DelegationRetried { delegation },
                    "delegation not found",
                );
            }
        };
        let commands = attached.commands.clone();
        let sent = blocking(move || {
            commands.send_described_command(command, descriptor, KERNEL_CALL_TIMEOUT)
        })
        .await?;
        let response = match sent {
            Ok(KernelCommandOutcome::Handled {
                events,
                operation,
                stored,
            }) => {
                let response = CommandResponse::from_core(Some(&events)).with_operation(operation);
                match stored {
                    Some(StoredCommandEnd::Finished(outcome)) => {
                        response.with_stored(Some(outcome))
                    }
                    Some(StoredCommandEnd::Failed(detail)) => {
                        response.with_failed_stored_command(detail)
                    }
                    None => response,
                }
            }
            Ok(KernelCommandOutcome::Rejected(reason)) => {
                let mut response = CommandResponse::from_core(Some(&[]));
                response.rejection = Some(CommandRejection {
                    reason,
                    message: reason.summary().to_string(),
                    next: reason_next(reason),
                });
                response
            }
            Ok(KernelCommandOutcome::Replayed(receipt)) => CommandResponse::replayed(receipt),
            Ok(KernelCommandOutcome::KeyReused) => {
                return Err(KernelProtocolError::idempotency_key_reused());
            }
            Ok(KernelCommandOutcome::KeyUnavailable) => {
                return Err(KernelProtocolError::idempotency_unavailable());
            }
            Ok(KernelCommandOutcome::ConfirmationRequired) => {
                return Err(KernelProtocolError::confirmation_required(command_name));
            }
            Err(KernelCallError::Stopped) => CommandResponse::from_core(None),
            Err(KernelCallError::TimedOut) => {
                return Err(timed_out("command", KERNEL_CALL_TIMEOUT));
            }
        };
        to_result(response)
    }

    async fn query(&mut self, params: Value) -> Result<Value, KernelProtocolError> {
        let attached = self.attached()?;
        let request: QueryRequest = parse_params(params)?;
        match request.query {
            KernelQuery::FeatureBindings {} | KernelQuery::PrReviewResult { .. } => {
                let number = match request.query {
                    KernelQuery::PrReviewResult { number } => Some(number),
                    _ => None,
                };
                let workspace = attached.issue_workspace.clone();
                let answer = blocking(move || {
                    crate::composition::production::query_pr_review(&workspace, number)
                })
                .await?
                .map_err(|error| unavailable(error.to_string()))?;
                return to_result(answer);
            }
            KernelQuery::DelegationBackends {} => {
                let workspace = attached.issue_workspace.clone();
                let store = self.delegation_store(&workspace).await?;
                let (mut chatgpt_chat, mut codex_thread, circuits) = tokio::join!(
                    self.delegations.status(),
                    self.delegations.codex_status(),
                    self.delegations.circuits(store, workspace),
                );
                if let Ok((chat, codex)) = circuits {
                    chatgpt_chat.circuit = Some(chat);
                    codex_thread.circuit = Some(codex);
                }
                return to_result(KernelQueryAnswer::DelegationBackends {
                    chatgpt_chat,
                    codex_thread,
                });
            }
            KernelQuery::Delegations {}
            | KernelQuery::DelegationMyTurn {}
            | KernelQuery::Delegation { .. }
            | KernelQuery::DelegationResult { .. } => {
                let store = self.delegation_store(&attached.issue_workspace).await?;
                let answer = delegation::db(store, move |store| {
                    Ok(match request.query {
                        KernelQuery::Delegations {} => KernelQueryAnswer::Delegations {
                            delegations: store.list()?,
                        },
                        KernelQuery::Delegation { id } => KernelQueryAnswer::Delegation {
                            delegation: store.detail(&id)?,
                        },
                        KernelQuery::DelegationMyTurn {} => KernelQueryAnswer::DelegationMyTurn {
                            items: store.my_turn()?,
                        },
                        KernelQuery::DelegationResult { id } => {
                            KernelQueryAnswer::DelegationResult {
                                text: store.result(&id)?,
                            }
                        }
                        _ => unreachable!(),
                    })
                })
                .await
                .map_err(|error| unavailable(error.to_string()))?;
                return to_result(answer);
            }
            _ => {}
        }
        if matches!(&request.query, KernelQuery::IssueAutomationPolicy {}) {
            let commands = attached.commands.clone();
            let inspection = read_kernel_log(move || {
                commands
                    .issue_automation_policy(KERNEL_CALL_TIMEOUT)
                    .map(Ok)
            })
            .await?;
            let state = inspection
                .as_ref()
                .map(IssueAutomationPolicyState::from_domain)
                .unwrap_or_else(IssueAutomationPolicyState::unavailable);
            return to_result(KernelQueryAnswer::IssueAutomationPolicy(state));
        }
        if matches!(&request.query, KernelQuery::IssueAutomation {}) {
            let commands = attached.commands.clone();
            let state =
                read_kernel_log(move || commands.issue_automation(KERNEL_CALL_TIMEOUT)).await?;
            return to_result(KernelQueryAnswer::IssueAutomation(
                IssueAutomationState::from_domain(&state.projection)
                    .with_runtime(state.collecting, state.next_poll_after_ms),
            ));
        }
        if let Some(issue) = request.query.issue_reference(&attached.issue_workspace)? {
            let commands = attached.commands.clone();
            let detail =
                read_kernel_log(move || commands.issue(issue, KERNEL_CALL_TIMEOUT)).await?;
            return to_result(KernelQueryAnswer::Issue(IssueState::from_domain(&detail)));
        }
        if let Some(query) = attached.issue_query(&request.query)? {
            let commands = attached.commands.clone();
            let page = read_kernel_log(move || commands.issues(query, KERNEL_CALL_TIMEOUT)).await?;
            return to_result(KernelQueryAnswer::Issues(IssuesState::from_domain(&page)));
        }
        let history = match request.query.history_range() {
            Some((after, limit)) => {
                let commands = attached.commands.clone();
                let read = move || commands.history(after, limit, KERNEL_CALL_TIMEOUT);
                Some(read_kernel_log(read).await?)
            }
            None => None,
        };
        let operation = match request.query.operation_id() {
            Some(op_id) => {
                let commands = attached.commands.clone();
                let read = move || commands.operation(op_id, KERNEL_CALL_TIMEOUT);
                Some(read_kernel_log(read).await?)
            }
            None => None,
        };
        let commands = attached.commands.clone();
        let view = blocking(move || commands.query(KERNEL_CALL_TIMEOUT))
            .await?
            .map_err(|error| match error {
                KernelCallError::Stopped => unavailable(error.to_string()),
                KernelCallError::TimedOut => timed_out("query", KERNEL_CALL_TIMEOUT),
            })?;
        to_result(request.query.answer(
            &view.snapshot,
            &view.thread_activity,
            parallel_source(&view.parallel),
            history,
            operation,
        ))
    }

    /// 커널에서 떨어지고, 이벤트 스레드가 끝날 때까지 기다린다. 그 뒤에는 이 접속에 커널 이벤트가 가지 않는다.
    async fn detach(&mut self) {
        self.confirmation = None;
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
        KernelHostEvent::Parallel(parallel) => Some(KernelEvent::Parallel(
            ParallelState::from_core(parallel_source(&parallel)),
        )),
        KernelHostEvent::Planning(projection) => {
            Some(KernelEvent::Planning(PlanningState::from_core(&projection)))
        }
        KernelHostEvent::Issues(state) => Some(KernelEvent::Issues(
            IssueAutomationState::from_domain(&state.projection)
                .with_runtime(state.collecting, state.next_poll_after_ms),
        )),
        KernelHostEvent::IssueDecision {
            decision,
            revision,
            reason,
        } => Some(KernelEvent::IssueDecision(
            crate::adapter::inbound::kernel_protocol::IssueDecisionChangedState {
                decision: *decision,
                revision,
                reason,
            },
        )),
        KernelHostEvent::Resync => Some(KernelEvent::Resync),
        KernelHostEvent::Recorded(record) => Some(KernelEvent::Recorded(*record)),
    }
}

fn parallel_source(view: &ParallelHostView) -> ParallelSource<'_> {
    ParallelSource {
        enabled: view.enabled,
        supervisor: view.supervisor.as_deref(),
        status: view.status.as_deref(),
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

/// 커널의 사건 기록에서 값을 읽는다 (`history`, `operation`). 커널이 끝났거나, 답이 늦거나, 기록을 읽지 못하면
/// 프로토콜 오류이다.
async fn read_kernel_log<T: Send + 'static>(
    read: impl FnOnce() -> Result<Result<T, String>, KernelCallError> + Send + 'static,
) -> Result<T, KernelProtocolError> {
    blocking(read)
        .await?
        .map_err(|error| match error {
            KernelCallError::Stopped => unavailable(error.to_string()),
            KernelCallError::TimedOut => timed_out("query", KERNEL_CALL_TIMEOUT),
        })?
        .map_err(unavailable)
}

/// 결정 답의 가리기 전 note는 녹화 입력에서 제외한다. 실제 검증은 명령 decoder가 맡는다.
pub(super) fn is_issue_decision_command(method: &str, text: &str) -> bool {
    method == METHOD_COMMAND
        && serde_json::from_str::<Value>(text)
            .ok()
            .is_some_and(|message| is_issue_decision_params(&message["params"]))
}

fn is_issue_decision_params(params: &Value) -> bool {
    params.pointer("/command/type").and_then(Value::as_str) == Some("answer_issue_decision")
}

fn confirmation_command(
    request: &CommandRequest,
    workspace: &str,
) -> Result<(StoredCommand, KernelCommandDescriptor), KernelProtocolError> {
    let descriptor = request
        .descriptor()
        .map_err(|_| invalid_confirmation())?
        .filter(|descriptor| descriptor.idempotency_key.is_some())
        .ok_or_else(invalid_confirmation)?;
    let input = request
        .command
        .clone()
        .into_input(workspace)
        .map_err(|_| invalid_confirmation())?;
    match input {
        KernelCommandInput::Stored(
            command @ (StoredCommand::AnswerIssueDecision { .. }
            | StoredCommand::SetIssueAutomationMode { .. }),
        ) => Ok((command, descriptor)),
        _ => Err(invalid_confirmation()),
    }
}
fn target_matches_command(
    target: &IssueConfirmationTarget,
    command: &StoredCommand,
    workspace: &str,
) -> bool {
    if target.workspace() != workspace || target.validate().is_err() {
        return false;
    }
    match (target, command) {
        (
            IssueConfirmationTarget::Decision {
                decision_id,
                option,
                ..
            },
            StoredCommand::AnswerIssueDecision {
                decision_id: id,
                option: choice,
                ..
            },
        ) => decision_id.as_str() == id && option == choice,
        (
            IssueConfirmationTarget::Mode { to, .. },
            StoredCommand::SetIssueAutomationMode { mode },
        ) => to == mode,
        _ => false,
    }
}
fn parse_confirmation_params<T: DeserializeOwned>(params: Value) -> Result<T, KernelProtocolError> {
    serde_json::from_value(params).map_err(|_| invalid_confirmation())
}
fn invalid_confirmation() -> KernelProtocolError {
    KernelProtocolError {
        code: ERROR_INVALID_PARAMS,
        message: "human confirmation is unavailable or does not match this action".to_owned(),
    }
}

fn parse_command_params(params: Value) -> Result<CommandRequest, KernelProtocolError> {
    let callback = matches!(
        params["command"]["type"].as_str(),
        Some("checkpoint_delegation" | "report_delegation_result")
    );
    let sensitive = is_issue_decision_params(&params);
    parse_params(params).map_err(|error| {
        if callback {
            KernelProtocolError {
                code: ERROR_INVALID_PARAMS,
                message: "invalid delegation callback parameters".to_owned(),
            }
        } else if sensitive {
            KernelProtocolError {
                code: ERROR_INVALID_PARAMS,
                message: "invalid issue decision answer parameters".to_string(),
            }
        } else {
            error
        }
    })
}

fn parse_params<T: DeserializeOwned>(params: Value) -> Result<T, KernelProtocolError> {
    serde_json::from_value(params).map_err(|error| KernelProtocolError {
        code: ERROR_INVALID_PARAMS,
        message: error.to_string(),
    })
}

fn delegation_command_result(
    outcome: anyhow::Result<Option<crate::domain::delegation::Delegation>>,
    success: impl FnOnce(crate::domain::delegation::Delegation) -> CommandResult,
    missing: &str,
) -> Result<Value, KernelProtocolError> {
    let mut response = CommandResponse::from_core(Some(&[]));
    match outcome {
        Ok(Some(delegation)) => response.result = Some(success(delegation)),
        Ok(None) => response.failure = Some(missing.to_owned()),
        Err(error) => response.failure = Some(error.to_string()),
    }
    to_result(response)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::kernel_host::KernelHost;
    use crate::composition::native_client_runtime::tests::recording_runtime_for_test;
    use crate::domain::issue_automation::IssueAutomationBindingId;
    use crate::domain::issue_automation::collection::{IssueCollectionItem, IssueCollectionPage};
    use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};

    fn issue(workspace: &str, id: &str) -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_fixture".to_string(),
            issue_id: id.to_string(),
            project_scope: IssueProjectScope {
                workspace_dir: workspace.to_string(),
            },
        }
    }

    fn page(reference: IssueReference, more: bool) -> IssuesState {
        IssuesState::from_domain(&IssueCollectionPage {
            revision: 3,
            items: vec![IssueCollectionItem {
                open_decisions: Vec::new(),
                issue: reference.clone(),
                binding_id: IssueAutomationBindingId::new("fixture").expect("binding"),
                tracked: None,
                current: None,
                observation_revision: None,
                tracked_revision: None,
                triage: None,
            }],
            next_after: more.then_some(reference),
            report: None,
        })
    }

    #[test]
    fn issue_pagination_from_a_worktree_or_subfolder_uses_the_kernel_workspace() {
        let (runtime, _turns) = recording_runtime_for_test();
        let host = KernelHost::start(runtime).expect("fake kernel host");
        for requested in ["/main/subfolder", "/worktrees/feature"] {
            let client = host.attach().expect("fake client");
            let attached = AttachedKernel {
                workspace: requested.to_string(),
                issue_workspace: "/main".to_string(),
                client_name: "cursor-fixture".to_string(),
                client_id: client.client_id().to_string(),
                attachment_id: "a".repeat(32),
                commands: client.commands(),
                stop_events: Arc::new(AtomicBool::new(false)),
                events: None,
            };
            let first_query = KernelQuery::Issues {
                after: None,
                limit: Some(1),
            };
            let first_request = attached
                .issue_query(&first_query)
                .expect("first page")
                .expect("issue query");
            assert!(first_request.after.is_none());
            let first = issue("/main", "I_first");
            let IssuesState::Ready(first_page) = page(first.clone(), true) else {
                panic!("ready page")
            };
            let second_query = KernelQuery::Issues {
                after: first_page.next_after,
                limit: Some(1),
            };
            let second_request = attached
                .issue_query(&second_query)
                .expect("next page")
                .expect("issue query");
            assert_eq!(second_request.after, Some(first));
            let IssuesState::Ready(second_page) = page(issue("/main", "I_second"), false) else {
                panic!("ready page")
            };
            assert_eq!(second_page.items[0].issue.issue_id, "I_second");
            assert!(second_page.next_after.is_none());

            let IssuesState::Ready(other_page) =
                page(issue("/different-repository", "I_other"), true)
            else {
                panic!("ready page")
            };
            let foreign = KernelQuery::Issues {
                after: other_page.next_after,
                limit: Some(1),
            };
            assert!(
                attached.issue_query(&foreign).is_err(),
                "a different repository scope remains invalid"
            );
            assert_eq!(
                attached.workspace, requested,
                "command and planning keep the requested workspace"
            );
        }
    }
    #[tokio::test]
    async fn policy_query_uses_the_host_workspace_without_starting_collection_or_turns() {
        use crate::composition::native_client_runtime::tests::recording_runtime_with_issue_queries_for_test;

        struct Workspace(std::path::PathBuf);
        impl Drop for Workspace {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-gateway-policy-query-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let _cleanup = Workspace(root.clone());
        let workspace = std::fs::canonicalize(&root).unwrap();
        let workspace = workspace.to_str().unwrap().to_owned();
        let service = crate::composition::issue_collection::build_for_kernel_test(&workspace);
        let (runtime, turns) = recording_runtime_with_issue_queries_for_test(service);
        let host = KernelHost::start(runtime).expect("query-only host");
        let client = host.attach_with_lifetime(true).expect("query-only client");
        let (outbox, _messages) = mpsc::channel(1);
        let mut session = KernelSession {
            kernels: Arc::new(RepositoryKernelRegistry::default()),
            outbox,
            connection_id: "c".repeat(32),
            confirmation: None,
            delegations: Default::default(),
            attached: Some(AttachedKernel {
                workspace: format!("{workspace}/requested-subfolder"),
                issue_workspace: workspace.clone(),
                client_name: "policy-query-fixture".into(),
                client_id: client.client_id().to_string(),
                attachment_id: "a".repeat(32),
                commands: client.commands(),
                stop_events: Arc::new(AtomicBool::new(false)),
                events: None,
            }),
        };
        let answer = session
            .query(json!({
                "query": { "type": "issue_automation_policy" },
            }))
            .await
            .expect("host policy query");
        assert_eq!(answer["type"], "issue_automation_policy");
        assert_eq!(answer["status"], "ready");
        assert_eq!(answer["workspace"], workspace);
        assert_eq!(answer["settings"].as_array().unwrap().len(), 69);
        assert_eq!(answer["files"].as_array().unwrap().len(), 2);
        assert!(
            turns.lock().unwrap().is_empty(),
            "a policy read does not submit a model turn",
        );
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            0,
            "the host query does not initialize the issue or planning ledger",
        );
        session.detach().await;
        drop(client);
        drop(host);
    }
    #[test]
    fn issue_decision_host_events_reach_the_wire_with_committed_metadata_only() {
        use crate::domain::issue_automation::decision::{
            IssueDecisionAgentAnswer, IssueDecisionBasis, IssueDecisionDefinition, IssueDecisionId,
            IssueDecisionKind, IssueDecisionOption, IssueDecisionRequest, IssueDecisionSharedText,
            IssueDecisionSubject, IssueDecisionTimeout,
        };
        use crate::domain::issue_automation::policy::IssueAutomationGate;
        use crate::domain::kernel_event::{IssueDecisionFactRecord, ReasonCode};

        let shared = |text: &str| IssueDecisionSharedText::new_shared(text).unwrap();
        let open = IssueDecisionRequest::new(
            IssueDecisionId::new("wire-decision").unwrap(),
            IssueDecisionDefinition {
                subject: IssueDecisionSubject::Issue(issue("/workspace", "I_decision")),
                gate: IssueAutomationGate::Start,
                kind: IssueDecisionKind::StartApproval,
                question: shared("decision-question-canary"),
                options: vec![IssueDecisionOption {
                    id: "continue".to_owned(),
                    label: shared("계속"),
                    effect: shared("의사를 기록합니다."),
                }],
                recommended: Some("continue".to_owned()),
                basis: IssueDecisionBasis {
                    policy_version: "a".repeat(64),
                    field_hash: None,
                    work_order_id: None,
                    gate_revision: 7,
                },
                on_timeout: IssueDecisionTimeout::Wait,
                due_at: None,
                asked_by: "agent-1".to_owned(),
                evidence: Vec::new(),
            },
        )
        .unwrap();
        let answer = || IssueDecisionAgentAnswer {
            option: "continue".to_owned(),
            note: Some(shared("decision-note-canary")),
            caller: Some("Desktop".to_owned()),
            channel: "kernel".to_owned(),
            at: "2026-11-20T20:00:00Z".to_owned(),
        };
        let answered = open
            .answer_agent(&open.definition().basis, answer())
            .unwrap()
            .request;
        let mut current_basis = open.definition().basis.clone();
        current_basis.gate_revision += 1;
        let superseded = open.answer_agent(&current_basis, answer()).unwrap().request;

        for (request, revision, state, reason) in [
            (open, 41, "open", None),
            (answered, 42, "answered", None),
            (
                superseded,
                43,
                "superseded",
                Some(ReasonCode::DecisionSuperseded),
            ),
        ] {
            let fact = IssueDecisionFactRecord::from(&request.fact());
            let expected = serde_json::to_value(&fact).unwrap();
            let event = protocol_event(KernelHostEvent::IssueDecision {
                decision: Box::new(fact),
                revision,
                reason,
            })
            .expect("a committed decision must be forwarded to attached clients");
            let wire = serde_json::to_value(EventNotification { event }).unwrap();

            assert_eq!(wire["event"]["type"], "issue_decision");
            assert_eq!(wire["event"]["revision"], revision);
            assert_eq!(
                wire["event"]["reason"],
                serde_json::to_value(reason).unwrap()
            );
            assert_eq!(wire["event"]["decision"], expected);
            assert_eq!(wire["event"]["decision"]["state"], state);
            assert_eq!(
                wire["event"]["decision"]["answer_by"],
                if state == "answered" {
                    json!("agent")
                } else {
                    Value::Null
                }
            );
            let text = wire.to_string();
            for forbidden in [
                "decision-question-canary",
                "decision-note-canary",
                "Desktop",
                "\"question\"",
                "\"note\"",
                "\"caller\"",
                "\"confirmation\"",
            ] {
                assert!(!text.contains(forbidden), "{forbidden}");
            }
        }
    }
}

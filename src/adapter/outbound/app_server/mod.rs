/*
 * app_server adapter는 application ports를 `codex app-server` 프로세스에 연결하는 outbound orchestration
 * layer다. domain/application 계층은 "thread 시작", "turn stream 실행", "recent sessions 조회" 같은
 * 의도만 알고, 이 모듈이 JSON-RPC 요청/응답, 프로세스 생명주기, shared runtime 재사용, fallback
 * connection을 조립한다.
 *
 * 큰 흐름:
 * - connection.rs: codex app-server 프로세스를 spawn하고 stdin/stdout line protocol을 관리한다.
 * - protocol.rs: app-server request/response/notification DTO와 변환 함수를 둡니다.
 * - runtime.rs: 여러 짧은 조회 요청이 하나의 app-server connection을 재사용하도록 shared runtime을 관리한다.
 * - 이 mod.rs: port trait 구현체로서 TUI/application service가 호출하는 공개 메서드를 조립한다.
 */
mod approval;
pub(crate) mod connection;
mod execution_policy;
mod planning_worker;
mod planning_worker_skill;
pub(crate) mod protocol;
pub(crate) mod runtime;
mod steering;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, TryLockError};
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use chrono::Utc;

use self::approval::AppServerApprovalBroker;
use self::connection::{
    AppServerApprovalMode, AppServerConnection, AppServerConnectionConfig,
    AppServerTurnInterruptSignal,
};
use self::execution_policy::AppServerExecutionPolicy;
pub use self::planning_worker::AppServerPlanningWorkerAdapter;
pub(crate) use self::planning_worker::PlanningThreadLauncher;
use self::planning_worker_skill::PlanningWorkerSkillAdapter;
use self::protocol::{
    ApprovalPolicyValue, ApprovalsReviewerValue, ReasoningEffortValue, SandboxModeValue,
    ThreadListParams, ThreadResumeParams, ThreadSetNameParams, ThreadStartParams, TurnInputItem,
    TurnStartParams, initialize_detail, runtime_configuration_request, sort_and_dedup_warnings,
    thread_title, to_conversation_snapshot, to_runtime_envelope, to_session_summary,
};
use self::runtime::{
    RequestFailureOutcome, RequestRuntimeMode, SharedAppServerRuntime, SharedRuntimeOutput,
    SharedRuntimeRequestKind, request_failure_outcome,
};
use self::steering::AppServerTurnSteerBroker;
use crate::application::port::outbound::app_server_prompt_log_port::{
    APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS, APP_SERVER_PROMPT_LOG_MAX_ITEMS_PER_DIRECTION,
    APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS, AppServerPromptInputRecord,
    AppServerPromptInteractionRecord, AppServerPromptLogPort, AppServerPromptOutputRecord,
    NoopAppServerPromptLogPort, bounded_prompt_log_string,
};
use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
use crate::application::port::outbound::parallel_agent_worker_port::{
    ParallelAgentWorkerPort, ParallelAgentWorkerStreamRequest,
};
use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
use crate::application::port::outbound::startup_probe_port::{
    AppServerStartupContext, StartupProbePort,
};
use crate::application::service::conversation_runtime_event::{
    ConversationStreamEvent, ConversationStreamSender,
};
use crate::application::service::planning::task_tool::{
    PLANNING_TOOL_PARENT_THREAD_ID_ENV, PLANNING_TOOL_PARENT_TURN_ID_ENV,
};
use crate::diagnostics::event_log;
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalReviewStatus,
    ConversationRuntimeControlTruth, ConversationSnapshot, ConversationTurnOptions,
    ConversationTurnSteerReceipt, ConversationTurnSteerRequest,
};
use crate::domain::conversation_runtime_envelope::{
    ConversationRuntimeEnvelope, ConversationRuntimeEnvelopeObservation,
};
use crate::domain::planning::PostTurnContinuationPermit;
use crate::domain::recent_sessions::{
    RecentSessions, SessionCatalog, SessionCatalogRequest, SessionCatalogTier, SessionRenameRequest,
};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
use crate::domain::turn_terminal::{
    ConversationTurnApplicationDelivery, ConversationTurnError, ConversationTurnErrorInfo,
    ConversationTurnItemsView, ConversationTurnTerminalOutcome, ConversationTurnTerminalReceipt,
    ConversationTurnTerminalUncertainty,
};
use serde_json::{Map, Value, json};

const PLANNING_WORKER_MODEL: &str = "gpt-5.4";
const PLANNING_WORKER_SERVICE_NAME: &str = "akra-planning-worker";
const PROMPT_LOG_CAPTURE_CHANNEL_CAPACITY: usize = APP_SERVER_PROMPT_LOG_MAX_ITEMS_PER_DIRECTION;
const MAX_STREAM_IDENTIFIER_BYTES: usize = 4 * 1024;
const MAX_STREAM_METADATA_BYTES: usize = 64 * 1024;
const MAX_STREAM_COMPLETED_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_STREAM_CHANGED_PATHS: usize = 256;
const MAX_STREAM_PATH_BYTES: usize = 16 * 1024;
const MAX_SNAPSHOT_MESSAGES: usize = 2_048;
const MAX_SNAPSHOT_TOTAL_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_SESSION_CATALOG_ITEMS: usize = 1_000;
const STREAM_TRUNCATION_MARKER: &str = "\n[truncated by Akra at app-server boundary]";
const PLANNING_WORKER_DEVELOPER_INSTRUCTIONS: &str = r#"You are an Akra planning-only sub-session.
Evaluate accepted DB direction authority, accepted DB task authority, and DB queue projection only.
Do not edit planning files, source files, SQL, or JSON authority directly.
Use the attached queue-mutation skill and `akra planning-tool run .` before falling back to final planning_task_commands."#;

#[derive(Debug)]
struct ProtectedThreadWorkspace {
    cwd: String,
    config: BTreeMap<String, Value>,
}

fn protected_thread_workspace(cwd: &str) -> Result<ProtectedThreadWorkspace> {
    let path = Path::new(cwd);
    if !path.is_absolute() {
        return Err(anyhow!(
            "app-server thread workspace must be absolute before project trust can be constrained"
        ));
    }

    let normalized = normalize_absolute_workspace_path(path);
    let normalized_cwd = normalized
        .to_str()
        .ok_or_else(|| anyhow!("app-server thread workspace contains non-UTF-8 path data"))?
        .to_string();
    let mut projects = Map::new();
    // Codex evaluates project config at each directory between its selected
    // project root and cwd, checking a canonical key before the request-path key.
    // Pin both aliases for every ancestor so an existing trusted parent, symlink
    // target, or linked-worktree root cannot enable project MCP servers or hooks.
    for ancestor in normalized.ancestors() {
        insert_untrusted_project_key(&mut projects, codex_raw_trust_key(ancestor)?);
        if let Ok(canonical) = fs::canonicalize(ancestor) {
            let canonical_key = codex_raw_trust_key(&canonical)?;
            insert_untrusted_project_key(&mut projects, canonical_key);
            #[cfg(windows)]
            insert_untrusted_project_key(&mut projects, simplify_windows_verbatim_key(&canonical)?);
        }
    }

    Ok(ProtectedThreadWorkspace {
        cwd: normalized_cwd,
        config: BTreeMap::from([("projects".to_string(), Value::Object(projects))]),
    })
}

fn protected_planning_thread_workspace(
    cwd: &str,
    parent_thread_id: Option<&str>,
    parent_turn_id: Option<&str>,
) -> Result<ProtectedThreadWorkspace> {
    let mut workspace = protected_thread_workspace(cwd)?;
    let mut environment = Map::new();
    for (name, value) in [
        (PLANNING_TOOL_PARENT_THREAD_ID_ENV, parent_thread_id),
        (PLANNING_TOOL_PARENT_TURN_ID_ENV, parent_turn_id),
    ] {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            environment.insert(name.to_string(), Value::String(value.to_string()));
        }
    }
    if !environment.is_empty() {
        workspace.config.insert(
            "shell_environment_policy".to_string(),
            json!({ "set": environment }),
        );
    }
    Ok(workspace)
}

fn exact_applied_workspace_cwd(
    envelope: &ConversationRuntimeEnvelope,
    requested_cwd: &str,
    response_method: &str,
) -> Result<String> {
    let applied_cwd = envelope
        .applied_cwd()
        .ok_or_else(|| anyhow!("{response_method} applied envelope omitted required cwd"))?;
    if applied_cwd != requested_cwd {
        return Err(anyhow!(
            "{response_method} applied cwd did not match the protected requested workspace"
        ));
    }
    Ok(applied_cwd.to_string())
}

fn insert_untrusted_project_key(projects: &mut Map<String, Value>, key: String) {
    projects.insert(key, json!({ "trust_level": "untrusted" }));
}

fn codex_raw_trust_key(path: &Path) -> Result<String> {
    let key = path
        .to_str()
        .ok_or_else(|| anyhow!("app-server thread workspace contains non-UTF-8 path data"))?;
    #[cfg(windows)]
    return Ok(key.to_ascii_lowercase());
    #[cfg(not(windows))]
    return Ok(key.to_string());
}

#[cfg(windows)]
fn simplify_windows_verbatim_key(path: &Path) -> Result<String> {
    let key = path
        .to_str()
        .ok_or_else(|| anyhow!("app-server thread workspace contains non-UTF-8 path data"))?;
    let simplified = if let Some(rest) = key.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = key.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        key.to_string()
    };
    Ok(simplified.to_ascii_lowercase())
}

fn normalize_absolute_workspace_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}
static NEXT_PROMPT_LOG_INTERACTION_ID: AtomicU64 = AtomicU64::new(1);
const PLANNING_WORKER_CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

struct PlanningWorkerContinuationWatcher {
    stop_sender: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl PlanningWorkerContinuationWatcher {
    fn start(
        continuation_permit: Option<PostTurnContinuationPermit>,
        interrupt_signal: AppServerTurnInterruptSignal,
    ) -> Option<Self> {
        let continuation_permit = continuation_permit?;
        let (stop_sender, stop_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            loop {
                if !continuation_permit.is_current() {
                    interrupt_signal.request_stop_all_sessions();
                    break;
                }
                match stop_receiver.recv_timeout(PLANNING_WORKER_CANCELLATION_POLL_INTERVAL) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        });
        Some(Self {
            stop_sender: Some(stop_sender),
            worker: Some(worker),
        })
    }
}

impl Drop for PlanningWorkerContinuationWatcher {
    fn drop(&mut self) {
        self.stop_sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) trait AppServerEventSender {
    fn send(
        &self,
        event: ConversationStreamEvent,
    ) -> std::result::Result<(), AppServerEventTrySendError> {
        self.try_send(event)
    }

    fn try_send(
        &self,
        event: ConversationStreamEvent,
    ) -> std::result::Result<(), AppServerEventTrySendError> {
        self.try_send_prebounded(bounded_app_server_stream_event(event))
    }

    fn try_send_prebounded(
        &self,
        event: ConversationStreamEvent,
    ) -> std::result::Result<(), AppServerEventTrySendError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AppServerEventTrySendError {
    Full,
    Disconnected,
}

impl AppServerEventSender for ConversationStreamSender {
    fn try_send_prebounded(
        &self,
        event: ConversationStreamEvent,
    ) -> std::result::Result<(), AppServerEventTrySendError> {
        match ConversationStreamSender::try_send(self, event) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => Err(AppServerEventTrySendError::Full),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                Err(AppServerEventTrySendError::Disconnected)
            }
        }
    }
}

#[cfg(test)]
impl AppServerEventSender for mpsc::Sender<ConversationStreamEvent> {
    fn try_send_prebounded(
        &self,
        event: ConversationStreamEvent,
    ) -> std::result::Result<(), AppServerEventTrySendError> {
        mpsc::Sender::send(self, event).map_err(|_| AppServerEventTrySendError::Disconnected)
    }
}

#[cfg(test)]
impl AppServerEventSender for mpsc::SyncSender<ConversationStreamEvent> {
    fn try_send_prebounded(
        &self,
        event: ConversationStreamEvent,
    ) -> std::result::Result<(), AppServerEventTrySendError> {
        match mpsc::SyncSender::try_send(self, event) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => Err(AppServerEventTrySendError::Full),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                Err(AppServerEventTrySendError::Disconnected)
            }
        }
    }
}

fn send_required_app_server_event(
    event_sender: &dyn AppServerEventSender,
    event: ConversationStreamEvent,
    event_name: &str,
) -> Result<()> {
    event_sender.send(event).map_err(|failure| {
        anyhow!(
            "required app-server stream event `{event_name}` was not admitted by the application sink ({failure:?})"
        )
    })
}

fn bounded_app_server_stream_event(event: ConversationStreamEvent) -> ConversationStreamEvent {
    match event {
        ConversationStreamEvent::AttachmentObserved { .. }
        | ConversationStreamEvent::ApprovalRequested { .. } => event,
        ConversationStreamEvent::ThreadPrepared {
            thread_id,
            title,
            cwd,
            runtime_envelope,
        } => ConversationStreamEvent::ThreadPrepared {
            thread_id: bounded_stream_text(thread_id, MAX_STREAM_IDENTIFIER_BYTES),
            title: bounded_stream_text(title, MAX_STREAM_METADATA_BYTES),
            cwd: bounded_stream_text(cwd, MAX_STREAM_METADATA_BYTES),
            runtime_envelope,
        },
        ConversationStreamEvent::TurnStarted {
            turn_id,
            runtime_request,
        } => ConversationStreamEvent::TurnStarted {
            turn_id: bounded_stream_text(turn_id, MAX_STREAM_IDENTIFIER_BYTES),
            runtime_request,
        },
        ConversationStreamEvent::RuntimeEnvelopeObserved { observation } => {
            let observation = match *observation {
                ConversationRuntimeEnvelopeObservation::SettingsUpdated {
                    thread_id,
                    settings,
                } => ConversationRuntimeEnvelopeObservation::SettingsUpdated {
                    thread_id: bounded_stream_text(thread_id, MAX_STREAM_IDENTIFIER_BYTES),
                    settings,
                },
                ConversationRuntimeEnvelopeObservation::ModelRerouted {
                    thread_id,
                    turn_id,
                    reroute,
                } => ConversationRuntimeEnvelopeObservation::ModelRerouted {
                    thread_id: bounded_stream_text(thread_id, MAX_STREAM_IDENTIFIER_BYTES),
                    turn_id: bounded_stream_text(turn_id, MAX_STREAM_IDENTIFIER_BYTES),
                    reroute,
                },
                ConversationRuntimeEnvelopeObservation::ThreadStatusChanged {
                    thread_id,
                    status,
                } => ConversationRuntimeEnvelopeObservation::ThreadStatusChanged {
                    thread_id: bounded_stream_text(thread_id, MAX_STREAM_IDENTIFIER_BYTES),
                    status,
                },
                ConversationRuntimeEnvelopeObservation::ProjectionGap { thread_id, gap } => {
                    ConversationRuntimeEnvelopeObservation::ProjectionGap {
                        thread_id: bounded_stream_text(thread_id, MAX_STREAM_IDENTIFIER_BYTES),
                        gap,
                    }
                }
            };
            ConversationStreamEvent::RuntimeEnvelopeObserved {
                observation: Box::new(observation),
            }
        }
        ConversationStreamEvent::ItemLifecycleObserved { observation } => {
            ConversationStreamEvent::ItemLifecycleObserved { observation }
        }
        ConversationStreamEvent::ProgressiveActivityObserved { batch } => {
            ConversationStreamEvent::ProgressiveActivityObserved { batch }
        }
        ConversationStreamEvent::StatusUpdated { text } => ConversationStreamEvent::StatusUpdated {
            text: bounded_stream_text(text, MAX_STREAM_METADATA_BYTES),
        },
        ConversationStreamEvent::AgentMessageCompleted {
            item_id,
            phase,
            text,
        } => ConversationStreamEvent::AgentMessageCompleted {
            item_id: bounded_stream_text(item_id, MAX_STREAM_IDENTIFIER_BYTES),
            phase: phase.map(|phase| bounded_stream_text(phase, MAX_STREAM_IDENTIFIER_BYTES)),
            text: bounded_stream_text(text, MAX_STREAM_COMPLETED_MESSAGE_BYTES),
        },
        ConversationStreamEvent::ToolActivity { mut activity } => {
            activity.text = bounded_stream_text(activity.text, MAX_STREAM_METADATA_BYTES);
            ConversationStreamEvent::ToolActivity { activity }
        }
        ConversationStreamEvent::ApprovalReviewUpdated { mut review } => {
            review.target_item_id =
                bounded_stream_text(review.target_item_id, MAX_STREAM_IDENTIFIER_BYTES);
            review.risk_level = review
                .risk_level
                .map(|risk| bounded_stream_text(risk, MAX_STREAM_IDENTIFIER_BYTES));
            review.rationale = review
                .rationale
                .map(|rationale| bounded_stream_text(rationale, MAX_STREAM_METADATA_BYTES));
            review.status = match review.status {
                ConversationApprovalReviewStatus::Unknown(status) => {
                    ConversationApprovalReviewStatus::Unknown(bounded_stream_text(
                        status,
                        MAX_STREAM_IDENTIFIER_BYTES,
                    ))
                }
                status => status,
            };
            ConversationStreamEvent::ApprovalReviewUpdated { review }
        }
        ConversationStreamEvent::ApprovalResolved {
            approval_id,
            resolution,
        } => ConversationStreamEvent::ApprovalResolved {
            approval_id: bounded_stream_text(approval_id, MAX_STREAM_IDENTIFIER_BYTES),
            resolution,
        },
        ConversationStreamEvent::TurnInterruptRequestFailed { message } => {
            ConversationStreamEvent::TurnInterruptRequestFailed {
                message: bounded_stream_text(message, MAX_STREAM_METADATA_BYTES),
            }
        }
        ConversationStreamEvent::TurnRetrying {
            thread_id,
            turn_id,
            error,
        } => ConversationStreamEvent::TurnRetrying {
            thread_id: bounded_stream_text(thread_id, MAX_STREAM_IDENTIFIER_BYTES),
            turn_id: bounded_stream_text(turn_id, MAX_STREAM_IDENTIFIER_BYTES),
            error: bounded_turn_error(error),
        },
        ConversationStreamEvent::TurnTerminal { receipt } => {
            ConversationStreamEvent::TurnTerminal {
                receipt: bounded_terminal_receipt(receipt),
            }
        }
        ConversationStreamEvent::Failed { message } => ConversationStreamEvent::Failed {
            message: bounded_stream_text(message, MAX_STREAM_METADATA_BYTES),
        },
    }
}

pub(super) fn bounded_terminal_receipt(
    mut receipt: ConversationTurnTerminalReceipt,
) -> ConversationTurnTerminalReceipt {
    receipt.thread_id = bounded_stream_text(receipt.thread_id, MAX_STREAM_IDENTIFIER_BYTES);
    receipt.turn_id = bounded_stream_text(receipt.turn_id, MAX_STREAM_IDENTIFIER_BYTES);
    receipt.observations.changed_planning_file_paths = receipt
        .observations
        .changed_planning_file_paths
        .into_iter()
        .take(MAX_STREAM_CHANGED_PATHS)
        .map(|path| bounded_stream_text(path, MAX_STREAM_PATH_BYTES))
        .collect();
    receipt.items_view = match receipt.items_view {
        ConversationTurnItemsView::Unknown(value) => ConversationTurnItemsView::Unknown(
            bounded_stream_text(value, MAX_STREAM_IDENTIFIER_BYTES),
        ),
        items_view => items_view,
    };
    receipt.outcome = match receipt.outcome {
        ConversationTurnTerminalOutcome::Failed { error } => {
            ConversationTurnTerminalOutcome::Failed {
                error: bounded_turn_error(error),
            }
        }
        ConversationTurnTerminalOutcome::Unknown {
            reason,
            observed_error,
        } => ConversationTurnTerminalOutcome::Unknown {
            reason: bounded_terminal_uncertainty(reason),
            observed_error: observed_error.map(bounded_turn_error),
        },
        outcome => outcome,
    };
    receipt
}

fn bounded_turn_error(mut error: ConversationTurnError) -> ConversationTurnError {
    error.message = bounded_stream_text(error.message, MAX_STREAM_METADATA_BYTES);
    error.additional_details = error
        .additional_details
        .map(|details| bounded_stream_text(details, MAX_STREAM_METADATA_BYTES));
    error.info = error.info.map(|info| match info {
        ConversationTurnErrorInfo::ActiveTurnNotSteerable { turn_kind } => {
            ConversationTurnErrorInfo::ActiveTurnNotSteerable {
                turn_kind: bounded_stream_text(turn_kind, MAX_STREAM_IDENTIFIER_BYTES),
            }
        }
        ConversationTurnErrorInfo::Unknown(value) => ConversationTurnErrorInfo::Unknown(
            bounded_stream_text(value, MAX_STREAM_IDENTIFIER_BYTES),
        ),
        info => info,
    });
    error
}

fn bounded_terminal_uncertainty(
    uncertainty: ConversationTurnTerminalUncertainty,
) -> ConversationTurnTerminalUncertainty {
    match uncertainty {
        ConversationTurnTerminalUncertainty::UnknownStatus(status) => {
            ConversationTurnTerminalUncertainty::UnknownStatus(bounded_stream_text(
                status,
                MAX_STREAM_IDENTIFIER_BYTES,
            ))
        }
        ConversationTurnTerminalUncertainty::StatusErrorContradiction { status } => {
            ConversationTurnTerminalUncertainty::StatusErrorContradiction {
                status: bounded_stream_text(status, MAX_STREAM_IDENTIFIER_BYTES),
            }
        }
        ConversationTurnTerminalUncertainty::MissingRequiredIdentity { field } => {
            ConversationTurnTerminalUncertainty::MissingRequiredIdentity {
                field: bounded_stream_text(field, MAX_STREAM_IDENTIFIER_BYTES),
            }
        }
        ConversationTurnTerminalUncertainty::ProtocolInconsistency(detail) => {
            ConversationTurnTerminalUncertainty::ProtocolInconsistency(bounded_stream_text(
                detail,
                MAX_STREAM_METADATA_BYTES,
            ))
        }
        uncertainty => uncertainty,
    }
}

fn bounded_stream_text(mut value: String, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value;
    }

    let mut boundary = maximum_bytes.min(value.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value.push_str(STREAM_TRUNCATION_MARKER);
    value
}

#[derive(Clone)]
pub struct CodexAppServerAdapter {
    /*
     * client_name/version은 app-server initialize handshake에 쓰이고, execution_policy는 thread/turn 생성 시
     * approval/sandbox 정책으로 전달된다. shared_runtime은 startup/session/snapshot처럼 짧은 요청을 빠르게
     * 처리하기 위한 재사용 connection이며, streaming turn은 같은 mutex를 잡아 notification ordering을 보존한다.
     */
    client_name: String,
    client_version: String,
    connection_config: AppServerConnectionConfig,
    execution_policy: AppServerExecutionPolicy,
    shared_runtime: Arc<Mutex<SharedAppServerRuntime>>,
    turn_interrupt_signal: AppServerTurnInterruptSignal,
    approval_broker: Arc<AppServerApprovalBroker>,
    turn_steer_broker: Arc<AppServerTurnSteerBroker>,
    planning_worker_skill_adapter: PlanningWorkerSkillAdapter,
    prompt_log_port: Arc<dyn AppServerPromptLogPort>,
}

impl CodexAppServerAdapter {
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Self::from_environment(client_name, client_version)
    }

    pub fn from_environment(
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Self {
        Self::from_environment_with_prompt_log(
            client_name,
            client_version,
            Arc::new(NoopAppServerPromptLogPort),
        )
    }

    pub fn from_environment_with_prompt_log(
        client_name: impl Into<String>,
        client_version: impl Into<String>,
        prompt_log_port: Arc<dyn AppServerPromptLogPort>,
    ) -> Self {
        /*
         * Adapter construction snapshots env-driven timeout and execution policy once.
         * That keeps a single TUI process from changing approval/sandbox behavior in
         * the middle of shared runtime reuse or hidden worker launches.
         */
        Self::with_configs_and_prompt_log(
            client_name,
            client_version,
            AppServerConnectionConfig::from_environment(),
            AppServerExecutionPolicy::from_environment(),
            prompt_log_port,
        )
    }

    #[cfg(test)]
    fn with_configs(
        client_name: impl Into<String>,
        client_version: impl Into<String>,
        connection_config: AppServerConnectionConfig,
        execution_policy: AppServerExecutionPolicy,
    ) -> Self {
        Self::with_configs_and_prompt_log(
            client_name,
            client_version,
            connection_config,
            execution_policy,
            Arc::new(NoopAppServerPromptLogPort),
        )
    }

    fn with_configs_and_prompt_log(
        client_name: impl Into<String>,
        client_version: impl Into<String>,
        connection_config: AppServerConnectionConfig,
        execution_policy: AppServerExecutionPolicy,
        prompt_log_port: Arc<dyn AppServerPromptLogPort>,
    ) -> Self {
        Self {
            client_name: client_name.into(),
            client_version: client_version.into(),
            connection_config,
            execution_policy,
            shared_runtime: Arc::new(Mutex::new(SharedAppServerRuntime::default())),
            turn_interrupt_signal: AppServerTurnInterruptSignal::default(),
            approval_broker: Arc::new(AppServerApprovalBroker::default()),
            turn_steer_broker: Arc::new(AppServerTurnSteerBroker::default()),
            planning_worker_skill_adapter: PlanningWorkerSkillAdapter::new(),
            prompt_log_port,
        }
    }

    fn open_connection(&self) -> Result<AppServerConnection> {
        /*
         * open_connection only spawns the child and wires client metadata. Callers
         * still perform initialize so shared, isolated fallback, and isolated stream
         * paths can attach their own lifecycle metadata and warnings.
         */
        AppServerConnection::spawn(
            self.client_name.clone(),
            self.client_version.clone(),
            self.connection_config.clone(),
            self.approval_broker.clone(),
            AppServerApprovalMode::Interactive,
            self.turn_interrupt_signal.clone(),
        )
    }

    fn open_unattended_connection(&self) -> Result<AppServerConnection> {
        AppServerConnection::spawn(
            self.client_name.clone(),
            self.client_version.clone(),
            self.connection_config.clone(),
            self.approval_broker.clone(),
            AppServerApprovalMode::Unattended,
            self.turn_interrupt_signal.clone(),
        )
    }

    #[tracing::instrument(level = "trace", skip(self, cwd, prompt, event_sender))]
    fn run_new_thread_stream_request(
        &self,
        cwd: &str,
        prompt: &str,
        options: ConversationTurnOptions,
        event_sender: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        /*
         * New conversation streaming creates a thread, emits ThreadPrepared for immediate TUI state, then starts a turn.
         * ThreadPrepared arrives before assistant tokens so the UI can display thread id/title/cwd and persist reattach state.
         */
        let result = self.with_streaming_runtime(|connection| {
            let model = options.model.as_deref();
            let effort = options.reasoning_effort.map(ReasoningEffortValue::from);
            let workspace = protected_thread_workspace(cwd)?;
            let requested_cwd = workspace.cwd.clone();
            let thread_request = runtime_configuration_request(
                None,
                None,
                Some(&requested_cwd),
                Some(self.execution_policy.approval_policy),
                self.execution_policy.approvals_reviewer,
                Some(SandboxModeValue::ReadOnly),
            );
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(workspace.cwd),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(SandboxModeValue::ReadOnly),
                config: Some(workspace.config),
                ..ThreadStartParams::default()
            })?;
            let thread_id = thread_response.thread.id.clone();
            if thread_id.is_empty() {
                anyhow::bail!("thread/start response omitted a nonempty thread id");
            }
            let runtime_envelope = to_runtime_envelope(
                thread_request,
                &thread_response.runtime_envelope_fields,
                &thread_response.thread,
                self.connection_config.runtime_launch_environment(),
            )?;
            connection.discard_notifications_before_turn_binding();
            let applied_cwd =
                exact_applied_workspace_cwd(&runtime_envelope, &requested_cwd, "thread/start")?;
            send_required_app_server_event(
                &event_sender,
                ConversationStreamEvent::codex_app_server_launch_attachment(),
                "attachment/launch",
            )?;
            send_required_app_server_event(
                &event_sender,
                ConversationStreamEvent::ThreadPrepared {
                    thread_id: thread_id.clone(),
                    title: thread_title(&thread_response.thread),
                    cwd: applied_cwd,
                    runtime_envelope: Box::new(runtime_envelope),
                },
                "thread/prepared",
            )?;

            self.start_turn_and_wait_for_stream(
                connection,
                vec![TurnInputItem::text(prompt)],
                model,
                effort,
                &event_sender,
                AppServerPromptTraceContext {
                    workspace_dir: cwd.to_string(),
                    session_kind: "main".to_string(),
                    operation: "new_thread_turn".to_string(),
                    service_name: None,
                    developer_instructions: None,
                    thread_id: thread_id.clone(),
                },
            )
        });

        finish_stream_result(result, &event_sender)
    }

    #[tracing::instrument(
        level = "trace",
        skip(
            self,
            workspace_directory,
            prompt,
            parent_thread_id,
            parent_turn_id,
            event_sender,
            continuation_permit
        )
    )]
    fn run_hidden_planning_thread_stream(
        &self,
        workspace_directory: &str,
        prompt: &str,
        parent_thread_id: Option<&str>,
        parent_turn_id: Option<&str>,
        event_sender: ConversationStreamSender,
        continuation_permit: Option<PostTurnContinuationPermit>,
    ) -> Result<ConversationTurnTerminalReceipt> {
        /*
         * Hidden planning workers are app-server threads, but they are isolated from the main user conversation.
         * ephemeral/service_name/developer_instructions identify the sub-session, and planning_worker_turn_input puts
         * the queue-mutation skill before the prompt so the worker returns planning task mutations instead of prose.
         */
        let skill_path = self
            .planning_worker_skill_adapter
            .queue_mutation_skill_path();
        event_log::emit_lazy("hidden_planning_thread_starting", || {
            json!({
                "workspace_directory": workspace_directory,
                "operation": "planning_worker_thread",
                "phase": "starting",
                "decision": "start_hidden_thread",
                "model": PLANNING_WORKER_MODEL,
                "service_name": PLANNING_WORKER_SERVICE_NAME,
                "prompt_chars": prompt.chars().count(),
                "skill_path": skill_path,
            })
        });
        let result = self.with_isolated_streaming_runtime(|connection| {
            if continuation_permit
                .as_ref()
                .is_some_and(|permit| !permit.is_current())
            {
                anyhow::bail!(
                    "post-turn continuation was superseded before hidden planning thread launch"
                );
            }
            let workspace = protected_planning_thread_workspace(
                workspace_directory,
                parent_thread_id,
                parent_turn_id,
            )?;
            let requested_cwd = workspace.cwd.clone();
            let thread_request = runtime_configuration_request(
                Some(PLANNING_WORKER_MODEL),
                None,
                Some(&requested_cwd),
                Some(ApprovalPolicyValue::Never),
                None,
                Some(SandboxModeValue::ReadOnly),
            );
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(workspace.cwd),
                approval_policy: Some(ApprovalPolicyValue::Never),
                approvals_reviewer: None,
                sandbox: Some(SandboxModeValue::ReadOnly),
                config: Some(workspace.config),
                model: Some(PLANNING_WORKER_MODEL.to_string()),
                developer_instructions: Some(PLANNING_WORKER_DEVELOPER_INSTRUCTIONS.to_string()),
                service_name: Some(PLANNING_WORKER_SERVICE_NAME.to_string()),
                ephemeral: Some(true),
            })?;
            let thread_id = thread_response.thread.id.clone();
            if thread_id.is_empty() {
                anyhow::bail!("thread/start response omitted a nonempty thread id");
            }
            let runtime_envelope = to_runtime_envelope(
                thread_request,
                &thread_response.runtime_envelope_fields,
                &thread_response.thread,
                self.connection_config.runtime_launch_environment(),
            )?;
            connection.discard_notifications_before_turn_binding();
            let applied_cwd =
                exact_applied_workspace_cwd(&runtime_envelope, &requested_cwd, "thread/start")?;
            send_required_app_server_event(
                &event_sender,
                ConversationStreamEvent::codex_app_server_launch_attachment(),
                "attachment/launch",
            )?;
            send_required_app_server_event(
                &event_sender,
                ConversationStreamEvent::ThreadPrepared {
                    thread_id: thread_id.clone(),
                    title: thread_title(&thread_response.thread),
                    cwd: applied_cwd,
                    runtime_envelope: Box::new(runtime_envelope),
                },
                "thread/prepared",
            )?;

            if continuation_permit
                .as_ref()
                .is_some_and(|permit| !permit.is_current())
            {
                anyhow::bail!(
                    "post-turn continuation was superseded before hidden planning turn launch"
                );
            }
            let local_interrupt_signal = AppServerTurnInterruptSignal::default();
            let observed_interrupt_generation = local_interrupt_signal.current_generation();
            let _continuation_watcher = PlanningWorkerContinuationWatcher::start(
                continuation_permit.clone(),
                local_interrupt_signal.clone(),
            );
            self.start_turn_and_wait_for_stream_with_policy(
                connection,
                self.planning_worker_turn_input(prompt),
                Some(PLANNING_WORKER_MODEL),
                Some(ReasoningEffortValue::Medium),
                &event_sender,
                AppServerPromptTraceContext {
                    workspace_dir: workspace_directory.to_string(),
                    session_kind: "planning-worker".to_string(),
                    operation: "hidden_planning_thread".to_string(),
                    service_name: Some(PLANNING_WORKER_SERVICE_NAME.to_string()),
                    developer_instructions: Some(
                        PLANNING_WORKER_DEVELOPER_INSTRUCTIONS.to_string(),
                    ),
                    thread_id: thread_id.clone(),
                },
                &local_interrupt_signal,
                observed_interrupt_generation,
                ApprovalPolicyValue::Never,
                None,
                SandboxModeValue::ReadOnly,
            )
        });
        match &result {
            Ok(receipt) if receipt.is_completed_and_confirmed() => {
                event_log::emit_lazy("hidden_planning_thread_completed", || {
                    json!({
                        "workspace_directory": workspace_directory,
                        "operation": "planning_worker_thread",
                        "phase": "completed",
                        "decision": "stream_completed",
                        "service_name": PLANNING_WORKER_SERVICE_NAME,
                    })
                })
            }
            Ok(receipt) => event_log::emit_lazy("hidden_planning_thread_failed", || {
                json!({
                    "workspace_directory": workspace_directory,
                    "operation": "planning_worker_thread",
                    "phase": "terminal_non_success",
                    "decision": "return_terminal_receipt",
                    "service_name": PLANNING_WORKER_SERVICE_NAME,
                    "terminal_status": receipt.outcome.status_label(),
                    "application_delivery": prompt_log_delivery_label(receipt.application_delivery),
                })
            }),
            Err(error) => event_log::emit_lazy("hidden_planning_thread_failed", || {
                json!({
                    "workspace_directory": workspace_directory,
                    "operation": "planning_worker_thread",
                    "phase": "failed",
                    "decision": "return_error",
                    "service_name": PLANNING_WORKER_SERVICE_NAME,
                    "error_summary": persisted_error_summary(error),
                })
            }),
        }

        finish_stream_result(result, &event_sender)
    }

    #[tracing::instrument(level = "trace", skip(self, operation))]
    fn with_shared_runtime<T, F>(
        &self,
        request_kind: SharedRuntimeRequestKind,
        mut operation: F,
    ) -> Result<SharedRuntimeOutput<T>>
    where
        F: FnMut(&mut AppServerConnection, &str) -> Result<T>,
    {
        /*
         * Short requests prefer the shared app-server process. If a turn stream is holding the mutex, they use an
         * isolated fallback connection so startup/session/snapshot UI does not block behind token streaming.
         * First failure is retried according to runtime.rs policy; final failure gets request-kind-specific context.
         */
        for attempt in 0..2 {
            let (mode, result) = match self.shared_runtime.try_lock() {
                Ok(mut runtime) => (
                    RequestRuntimeMode::Shared,
                    self.run_request_on_locked_runtime(&mut runtime, &mut operation),
                ),
                Err(TryLockError::WouldBlock) => (
                    RequestRuntimeMode::IsolatedFallback,
                    self.with_isolated_runtime(
                        Some(request_kind.isolated_fallback_notice()),
                        &mut operation,
                    ),
                ),
                Err(TryLockError::Poisoned(_)) => (
                    RequestRuntimeMode::Shared,
                    Err(anyhow!("shared app-server runtime mutex was poisoned")),
                ),
            };

            match result {
                Ok(output) => return Ok(output),
                Err(error) => match request_failure_outcome(mode, attempt) {
                    RequestFailureOutcome::RetryAfterSharedReset => {
                        self.reset_shared_runtime(Some(request_kind.retry_reset_notice(&error)));
                        continue;
                    }
                    RequestFailureOutcome::RetryWithoutReset => continue,
                    RequestFailureOutcome::ReturnSharedFailure => {
                        self.reset_shared_runtime(None);
                        return Err(error.context(request_kind.shared_retry_failure_context()));
                    }
                    RequestFailureOutcome::ReturnIsolatedFailure => {
                        return Err(error.context(request_kind.isolated_retry_failure_context()));
                    }
                },
            }
        }

        unreachable!("shared runtime retry loop always returns on success or final failure")
    }

    #[tracing::instrument(level = "trace", skip(self, runtime, operation))]
    fn run_request_on_locked_runtime<T, F>(
        &self,
        runtime: &mut SharedAppServerRuntime,
        operation: &mut F,
    ) -> Result<SharedRuntimeOutput<T>>
    where
        F: FnMut(&mut AppServerConnection, &str) -> Result<T>,
    {
        /*
         * The locked runtime is the only path allowed to reuse the shared child. It
         * returns value, attachment profile, retry notices, and connection warnings as
         * one batch so callers never combine a response from one process with
         * diagnostics from another.
         */
        runtime.ensure_connected(self)?;
        let initialize_detail = runtime.initialize_detail()?.to_string();
        let attachment_profile = runtime.attachment_profile()?;
        let (value, connection_warnings) = {
            let connection = runtime.connection_mut()?;
            let value = operation(connection, &initialize_detail)?;
            let warnings = connection.take_warnings();
            (value, warnings)
        };
        let mut warnings = runtime.take_notices();
        warnings.extend(connection_warnings);
        sort_and_dedup_warnings(&mut warnings);
        Ok(SharedRuntimeOutput {
            value,
            warnings,
            attachment_profile,
        })
    }

    fn with_isolated_runtime<T, F>(
        &self,
        notice: Option<String>,
        operation: &mut F,
    ) -> Result<SharedRuntimeOutput<T>>
    where
        F: FnMut(&mut AppServerConnection, &str) -> Result<T>,
    {
        /*
         * Isolated runtime is a pressure-release path for short reads while the shared
         * child is busy streaming. It intentionally does not mutate shared runtime
         * state, because the lock holder may still be reducing the authoritative turn.
         */
        let mut connection = self.open_unattended_connection()?;
        let initialize_response = connection.initialize()?;
        let initialize_detail = bounded_stream_text(
            initialize_detail(&initialize_response),
            MAX_STREAM_METADATA_BYTES,
        );
        let attachment_profile = TerminalBridgeAttachmentProfile::codex_app_server_launch();
        let value = operation(&mut connection, &initialize_detail)?;
        let mut warnings = connection.take_warnings();
        if let Some(notice) = notice {
            warnings.push(notice);
        }
        sort_and_dedup_warnings(&mut warnings);
        Ok(SharedRuntimeOutput {
            value,
            warnings,
            attachment_profile,
        })
    }

    #[tracing::instrument(level = "trace", skip(self, operation))]
    fn with_isolated_streaming_runtime<T, F>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut(&mut AppServerConnection) -> Result<T>,
    {
        /*
         * Worker streams use their own child process so planning/parallel sub-session
         * notifications cannot interleave with the user's active conversation stream.
         * Any warnings are drained locally because hidden workers report meaningful
         * output through the stream events and worker result reduction.
         */
        let mut connection = self.open_unattended_connection()?;
        connection.initialize()?;
        let result = operation(&mut connection);
        let _ = connection.take_warnings();
        result
    }

    #[tracing::instrument(level = "trace", skip(self, operation))]
    fn with_streaming_runtime<T, F>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut(&mut AppServerConnection) -> Result<T>,
    {
        /*
         * User-facing streams deliberately hold the shared runtime mutex until
         * turn/completed. Short requests can still use isolated fallback, but no other
         * shared caller may consume stdout lines while this stream reducer owns
         * notification ordering.
         */
        let mut runtime = self
            .shared_runtime
            .lock()
            .map_err(|_| anyhow!("shared app-server runtime mutex was poisoned"))?;
        runtime.ensure_connected(self)?;

        let (result, warnings) = {
            let connection = runtime.connection_mut()?;
            let result = operation(connection);
            let warnings = connection.take_warnings();
            (result, warnings)
        };
        runtime.push_notices(warnings);

        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                /*
                 * A stream failure may leave the shared child's protocol state
                 * ambiguous: there could be unread notifications, partial stderr, or a
                 * child close in progress. Resetting forces the next short request to
                 * reconnect and keeps the original stream error visible as a notice.
                 */
                runtime.reset();
                runtime.push_notice(format!(
                    "shared runtime reset after turn stream failure; the next request will reconnect ({error})"
                ));
                Err(error)
            }
        }
    }

    #[tracing::instrument(
        level = "trace",
        skip(self, connection, input, event_sender, prompt_trace_context)
    )]
    fn start_turn_and_wait_for_stream(
        &self,
        connection: &mut AppServerConnection,
        input: Vec<TurnInputItem>,
        model: Option<&str>,
        effort: Option<ReasoningEffortValue>,
        event_sender: &ConversationStreamSender,
        prompt_trace_context: AppServerPromptTraceContext,
    ) -> Result<ConversationTurnTerminalReceipt> {
        let observed_interrupt_generation = self.turn_interrupt_signal.current_generation();
        self.start_turn_and_wait_for_stream_with_policy(
            connection,
            input,
            model,
            effort,
            event_sender,
            prompt_trace_context,
            &self.turn_interrupt_signal,
            observed_interrupt_generation,
            self.execution_policy.approval_policy,
            self.execution_policy.approvals_reviewer,
            self.execution_policy.sandbox_mode,
        )
    }

    #[tracing::instrument(
        level = "trace",
        skip(
            self,
            connection,
            input,
            event_sender,
            prompt_trace_context,
            interrupt_signal
        )
    )]
    #[allow(clippy::too_many_arguments)]
    fn start_turn_and_wait_for_stream_with_policy(
        &self,
        connection: &mut AppServerConnection,
        input: Vec<TurnInputItem>,
        model: Option<&str>,
        effort: Option<ReasoningEffortValue>,
        event_sender: &ConversationStreamSender,
        prompt_trace_context: AppServerPromptTraceContext,
        interrupt_signal: &AppServerTurnInterruptSignal,
        observed_interrupt_generation: u64,
        approval_policy: ApprovalPolicyValue,
        approvals_reviewer: Option<ApprovalsReviewerValue>,
        sandbox_mode: SandboxModeValue,
    ) -> Result<ConversationTurnTerminalReceipt> {
        /*
         * The interrupt generation is sampled before turn/start so a stale stop from a
         * previous turn cannot cancel the new one. wait_for_turn_stream compares
         * against this snapshot and translates only later generations into
         * turn/interrupt.
         */
        let prompt_logging_enabled = self.prompt_log_port.is_enabled();
        let started_at = prompt_logging_enabled.then(|| Utc::now().to_rfc3339());
        let mut input_records = prompt_logging_enabled.then(|| prompt_log_input_records(&input));
        let trace_thread_id = prompt_trace_context.thread_id.clone();
        let runtime_request = runtime_configuration_request(
            model,
            effort,
            None,
            Some(approval_policy),
            approvals_reviewer,
            Some(sandbox_mode),
        );
        let turn_response = match connection.start_turn_with_event_sender(
            TurnStartParams {
                thread_id: trace_thread_id.clone(),
                input,
                approval_policy: Some(approval_policy),
                approvals_reviewer,
                sandbox_policy: Some(sandbox_mode.as_turn_sandbox_policy()),
                model: model.map(str::to_string),
                effort,
            },
            event_sender,
            interrupt_signal,
            observed_interrupt_generation,
        ) {
            Ok(response) => response,
            Err(error) => {
                if prompt_logging_enabled {
                    self.record_prompt_interaction(AppServerPromptInteractionRecord {
                        sequence: 0,
                        interaction_id: next_prompt_log_interaction_id(),
                        session_kind: prompt_trace_context.session_kind,
                        operation: prompt_trace_context.operation,
                        status: "failed".to_string(),
                        workspace_dir: prompt_trace_context.workspace_dir,
                        thread_id: Some(trace_thread_id),
                        turn_id: None,
                        service_name: prompt_trace_context.service_name,
                        model: model.map(str::to_string),
                        reasoning_effort: effort.map(reasoning_effort_label).map(str::to_string),
                        developer_instructions: prompt_trace_context.developer_instructions,
                        input_items: input_records.take().unwrap_or_default(),
                        output_items: Vec::new(),
                        error_message: Some(persisted_error_summary(&error)),
                        started_at: started_at.unwrap_or_default(),
                        completed_at: Utc::now().to_rfc3339(),
                    });
                }
                return Err(error);
            }
        };

        if trace_thread_id.is_empty() || turn_response.turn.id.is_empty() {
            anyhow::bail!(
                "turn/start response did not provide nonempty thread and turn identifiers"
            );
        }
        let turn_steer_binding = (prompt_trace_context.session_kind == "main")
            .then(|| {
                self.turn_steer_broker
                    .bind(&trace_thread_id, &turn_response.turn.id)
            })
            .transpose()?;
        if let Err(error) = send_required_app_server_event(
            event_sender,
            ConversationStreamEvent::TurnStarted {
                turn_id: turn_response.turn.id.clone(),
                runtime_request: Box::new(runtime_request),
            },
            "turn/started",
        ) {
            if let Some(binding) = turn_steer_binding.as_ref() {
                self.turn_steer_broker.unbind(
                    binding.binding_id(),
                    "turn start could not be delivered to the application",
                );
            }
            return Err(error);
        }

        if !prompt_logging_enabled {
            return match turn_steer_binding {
                Some(binding) => connection.wait_for_turn_stream_with_steering(
                    &trace_thread_id,
                    &turn_response.turn.id,
                    interrupt_signal,
                    observed_interrupt_generation,
                    event_sender,
                    &self.turn_steer_broker,
                    binding,
                ),
                None => connection.wait_for_turn_stream(
                    &trace_thread_id,
                    &turn_response.turn.id,
                    interrupt_signal,
                    observed_interrupt_generation,
                    event_sender,
                ),
            };
        }

        let (stream_event_sender, capture_worker) =
            prompt_log_stream_forwarder(event_sender.clone());
        let stream_result = match turn_steer_binding {
            Some(binding) => connection.wait_for_turn_stream_with_steering(
                &trace_thread_id,
                &turn_response.turn.id,
                interrupt_signal,
                observed_interrupt_generation,
                &stream_event_sender,
                &self.turn_steer_broker,
                binding,
            ),
            None => connection.wait_for_turn_stream(
                &trace_thread_id,
                &turn_response.turn.id,
                interrupt_signal,
                observed_interrupt_generation,
                &stream_event_sender,
            ),
        };
        drop(stream_event_sender);
        let output_items = match capture_worker.join() {
            Ok(capture) => capture.output_items,
            Err(_) => {
                tracing::warn!("app-server prompt log capture worker panicked");
                Vec::new()
            }
        };
        self.record_prompt_interaction(AppServerPromptInteractionRecord {
            sequence: 0,
            interaction_id: next_prompt_log_interaction_id(),
            session_kind: prompt_trace_context.session_kind,
            operation: prompt_trace_context.operation,
            status: prompt_log_terminal_status(&stream_result).to_string(),
            workspace_dir: prompt_trace_context.workspace_dir,
            thread_id: Some(trace_thread_id),
            turn_id: Some(turn_response.turn.id.clone()),
            service_name: prompt_trace_context.service_name,
            model: model.map(str::to_string),
            reasoning_effort: effort.map(reasoning_effort_label).map(str::to_string),
            developer_instructions: prompt_trace_context.developer_instructions,
            input_items: input_records.unwrap_or_default(),
            output_items,
            error_message: prompt_log_terminal_error(&stream_result),
            started_at: started_at.unwrap_or_default(),
            completed_at: Utc::now().to_rfc3339(),
        });

        stream_result
    }

    fn planning_worker_turn_input(&self, prompt: &str) -> Vec<TurnInputItem> {
        /*
         * Skill first, text second: app-server must load the evaluator contract before
         * interpreting the worker prompt. This is the enforcement point that keeps
         * hidden planning workers on task-command output instead of free-form prose.
         */
        vec![
            self.planning_worker_skill_adapter
                .queue_mutation_skill_input(),
            TurnInputItem::text(prompt),
        ]
    }

    fn reset_shared_runtime(&self, notice: Option<String>) {
        /*
         * Reset can be called from retry paths outside the stream owner. If the mutex
         * is busy, the active stream remains responsible for cleanup; forcing a reset
         * from here would risk dropping the child while stdout is being reduced.
         */
        if let Ok(mut runtime) = self.shared_runtime.lock() {
            runtime.reset();
            if let Some(notice) = notice {
                runtime.push_notice(notice);
            }
        }
    }

    fn request_turn_interrupt_for_all_streams(&self) {
        self.turn_interrupt_signal.request_stop_all_sessions();
    }

    fn record_prompt_interaction(&self, record: AppServerPromptInteractionRecord) {
        let record = record.into_bounded();
        let workspace_dir = record.workspace_dir.clone();
        if let Err(error) = self
            .prompt_log_port
            .append_app_server_prompt_interaction(&workspace_dir, record)
        {
            tracing::warn!(%workspace_dir, %error, "failed to record app-server prompt interaction");
        }
    }

    fn active_policy_summary(&self) -> String {
        format!(
            "app-server policy: {}, {}",
            self.execution_policy.summary(),
            self.connection_config.environment_policy_summary()
        )
    }

    fn elevated_policy_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut elevated_risks = self
            .execution_policy
            .elevated_risk_labels()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        if self.connection_config.uses_full_process_environment() {
            elevated_risks.push("process-env=all inherits parent secrets".to_string());
        }
        if self.connection_config.uses_api_key_auth() {
            elevated_risks.push(
                "api-key-auth forwards OPENAI_API_KEY/CODEX_API_KEY to app-server".to_string(),
            );
        }
        if !elevated_risks.is_empty() {
            warnings.push(format!(
                "elevated-risk app-server policy: {}",
                elevated_risks.join(", ")
            ));
        }
        warnings
    }
}

impl StartupProbePort for CodexAppServerAdapter {
    fn load_startup_context(&self) -> Result<AppServerStartupContext> {
        /*
         * Startup context is the first consumer of the shared runtime batch. It combines
         * initialize detail, account/auth interpretation, transport warnings, and
         * attachment profile so startup UI can show whether the process is usable and
         * what it attached to.
         */
        let output = self.with_shared_runtime(
            SharedRuntimeRequestKind::StartupChecks,
            |connection, initialize_detail| {
                Ok((initialize_detail.to_string(), connection.read_account()?))
            },
        )?;
        let (initialize_detail, account_response) = output.value;
        let initialize_detail = format!("{initialize_detail} / {}", self.active_policy_summary());

        let mut warnings = output.warnings;
        warnings.extend(self.elevated_policy_warnings());
        sort_and_dedup_warnings(&mut warnings);
        Ok(AppServerStartupContext {
            attachment_profile: output.attachment_profile,
            initialize_detail,
            account_detail: bounded_stream_text(
                account_response.to_summary_text(),
                MAX_STREAM_METADATA_BYTES,
            ),
            account_ok: account_response.is_authenticated(),
            warnings,
        })
    }
}

impl SessionCatalogPort for CodexAppServerAdapter {
    fn load_session_catalog(&self, request: SessionCatalogRequest) -> Result<SessionCatalog> {
        /*
         * Recent sessions are provider-backed because app-server owns thread storage,
         * pagination, and source metadata. This adapter only maps wire records to
         * SessionSummary and preserves next_cursor for future catalog expansion.
         */
        let catalog_limit = request.limit.min(MAX_SESSION_CATALOG_ITEMS);
        let output =
            self.with_shared_runtime(SharedRuntimeRequestKind::RecentSessions, |connection, _| {
                connection.list_threads(ThreadListParams {
                    limit: Some(catalog_limit),
                    ..ThreadListParams::default()
                })
            })?;
        let mut warnings = output.warnings;
        if output.value.data.len() > catalog_limit {
            warnings.push(format!(
                "app-server returned {} sessions for a bounded catalog limit of {catalog_limit}; extra records were ignored",
                output.value.data.len()
            ));
        }
        let items = output
            .value
            .data
            .into_iter()
            .take(catalog_limit)
            .map(to_session_summary)
            .collect::<Vec<_>>();

        Ok(SessionCatalog::ready(
            SessionCatalogTier::ProviderBackedCatalog,
            RecentSessions {
                items,
                warnings,
                next_cursor: output
                    .value
                    .next_cursor
                    .map(|cursor| bounded_stream_text(cursor, MAX_STREAM_IDENTIFIER_BYTES)),
            },
        ))
    }

    fn rename_session(&self, request: SessionRenameRequest) -> Result<()> {
        let thread_id = request.thread_id.trim();
        let name = request.name.trim();
        if thread_id.is_empty() {
            anyhow::bail!("session rename requires a thread id");
        }
        if name.is_empty() {
            anyhow::bail!("session rename requires a non-empty name");
        }

        self.with_shared_runtime(SharedRuntimeRequestKind::SessionRename, |connection, _| {
            connection.set_thread_name(ThreadSetNameParams {
                thread_id: thread_id.to_string(),
                name: name.to_string(),
            })
        })?;
        Ok(())
    }
}

impl InteractiveTurnRuntimePort for CodexAppServerAdapter {
    fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
        ConversationRuntimeControlTruth::codex_app_server()
    }

    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        /*
         * Snapshot reads include historical turns because resumed sessions need the
         * same transcript vocabulary as live streams. protocol.rs owns raw item
         * projection so TUI/application layers never inspect app-server JSON directly.
         */
        let output = self.with_shared_runtime(
            SharedRuntimeRequestKind::ConversationSnapshot,
            |connection, _| connection.read_thread(thread_id, true),
        )?;
        Ok(to_conversation_snapshot(
            output.value.thread,
            output.warnings,
        ))
    }

    fn request_stop_all_sessions(&self) -> Result<()> {
        /*
         * Stop is broadcast by generation counter rather than by holding a list of
         * active connections. Each stream loop decides whether it started before the
         * new generation and sends at most one interrupt for its own turn id.
         */
        self.request_turn_interrupt_for_all_streams();
        Ok(())
    }

    fn resolve_approval_request(
        &self,
        approval_id: &str,
        decision: ConversationApprovalDecision,
    ) -> Result<()> {
        self.approval_broker.resolve(approval_id, decision)
    }

    fn steer_turn(
        &self,
        request: ConversationTurnSteerRequest,
    ) -> Result<ConversationTurnSteerReceipt> {
        self.turn_steer_broker.submit(request)
    }

    fn run_new_thread_stream(
        &self,
        cwd: &str,
        prompt: &str,
        options: ConversationTurnOptions,
        event_sender: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        self.run_new_thread_stream_request(cwd, prompt, options, event_sender)
    }

    #[tracing::instrument(level = "trace", skip(self, thread_id, prompt, event_sender))]
    fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        options: ConversationTurnOptions,
        event_sender: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        /*
         * Existing-thread streaming reattaches before turn/start so app-server restores
         * thread context and execution policy on the server side. The reattach
         * attachment event tells the terminal bridge that this stream is bound to an
         * existing app-server session, not a freshly created thread.
         */
        let result = self.with_streaming_runtime(|connection| {
            let model = options.model.as_deref();
            let effort = options.reasoning_effort.map(ReasoningEffortValue::from);
            // Resume does not carry a caller-owned cwd, so read the persisted thread
            // record before applying the project-trust override.
            let thread = connection.read_thread(thread_id, false)?.thread;
            if thread.id != thread_id {
                anyhow::bail!("thread/read response identity did not match requested thread");
            }
            let workspace = protected_thread_workspace(&thread.cwd)?;
            let requested_cwd = workspace.cwd.clone();
            let thread_request = runtime_configuration_request(
                None,
                None,
                Some(&requested_cwd),
                Some(self.execution_policy.approval_policy),
                self.execution_policy.approvals_reviewer,
                Some(SandboxModeValue::ReadOnly),
            );
            let resume_response = connection.resume_thread(ThreadResumeParams {
                thread_id: thread_id.to_string(),
                cwd: Some(workspace.cwd),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(SandboxModeValue::ReadOnly),
                config: Some(workspace.config),
            })?;
            if resume_response.thread.id != thread_id {
                anyhow::bail!("thread/resume response identity did not match requested thread");
            }
            let runtime_envelope = to_runtime_envelope(
                thread_request,
                &resume_response.runtime_envelope_fields,
                &resume_response.thread,
                self.connection_config.runtime_launch_environment(),
            )?;
            connection.discard_notifications_before_turn_binding();
            let applied_cwd =
                exact_applied_workspace_cwd(&runtime_envelope, &requested_cwd, "thread/resume")?;
            send_required_app_server_event(
                &event_sender,
                ConversationStreamEvent::codex_app_server_reattach_attachment(),
                "attachment/reattach",
            )?;
            send_required_app_server_event(
                &event_sender,
                ConversationStreamEvent::ThreadPrepared {
                    thread_id: resume_response.thread.id.clone(),
                    title: thread_title(&resume_response.thread),
                    cwd: applied_cwd.clone(),
                    runtime_envelope: Box::new(runtime_envelope),
                },
                "thread/prepared",
            )?;
            self.start_turn_and_wait_for_stream(
                connection,
                vec![TurnInputItem::text(prompt)],
                model,
                effort,
                &event_sender,
                AppServerPromptTraceContext {
                    workspace_dir: applied_cwd,
                    session_kind: "main".to_string(),
                    operation: "resumed_thread_turn".to_string(),
                    service_name: None,
                    developer_instructions: None,
                    thread_id: thread_id.to_string(),
                },
            )
        });

        finish_stream_result(result, &event_sender)
    }
}

impl PlanningThreadLauncher for CodexAppServerAdapter {
    #[tracing::instrument(
        level = "trace",
        skip(
            self,
            workspace_directory,
            prompt,
            parent_thread_id,
            parent_turn_id,
            event_sender,
            continuation_permit
        )
    )]
    fn run_hidden_planning_thread(
        &self,
        workspace_directory: &str,
        prompt: &str,
        parent_thread_id: Option<&str>,
        parent_turn_id: Option<&str>,
        event_sender: ConversationStreamSender,
        continuation_permit: Option<PostTurnContinuationPermit>,
    ) -> Result<ConversationTurnTerminalReceipt> {
        // PlanningWorkerPort depends on this narrow launcher trait so tests can fake the stream source.
        self.run_hidden_planning_thread_stream(
            workspace_directory,
            prompt,
            parent_thread_id,
            parent_turn_id,
            event_sender,
            continuation_permit,
        )
    }
}

impl ParallelAgentWorkerPort for CodexAppServerAdapter {
    #[tracing::instrument(level = "trace", skip(self, request, event_sender))]
    fn run_isolated_new_thread_stream(
        &self,
        request: ParallelAgentWorkerStreamRequest<'_>,
        event_sender: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        // Parallel worker sessions use isolated processes but persist app-server threads so `:peek` can read them later.
        let result = self.with_isolated_streaming_runtime(|connection| {
            let workspace = protected_thread_workspace(request.cwd)?;
            let requested_cwd = workspace.cwd.clone();
            let thread_request = runtime_configuration_request(
                None,
                None,
                Some(&requested_cwd),
                Some(self.execution_policy.approval_policy),
                self.execution_policy.approvals_reviewer,
                Some(SandboxModeValue::ReadOnly),
            );
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(workspace.cwd),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(SandboxModeValue::ReadOnly),
                config: Some(workspace.config),
                model: None,
                developer_instructions: Some(request.developer_instructions.to_string()),
                service_name: Some(request.service_name.to_string()),
                ephemeral: Some(false),
            })?;
            let thread_id = thread_response.thread.id.clone();
            if thread_id.is_empty() {
                anyhow::bail!("thread/start response omitted a nonempty thread id");
            }
            let runtime_envelope = to_runtime_envelope(
                thread_request,
                &thread_response.runtime_envelope_fields,
                &thread_response.thread,
                self.connection_config.runtime_launch_environment(),
            )?;
            connection.discard_notifications_before_turn_binding();
            let applied_cwd =
                exact_applied_workspace_cwd(&runtime_envelope, &requested_cwd, "thread/start")?;
            send_required_app_server_event(
                &event_sender,
                ConversationStreamEvent::codex_app_server_launch_attachment(),
                "attachment/launch",
            )?;
            send_required_app_server_event(
                &event_sender,
                ConversationStreamEvent::ThreadPrepared {
                    thread_id: thread_id.clone(),
                    title: thread_title(&thread_response.thread),
                    cwd: applied_cwd,
                    runtime_envelope: Box::new(runtime_envelope),
                },
                "thread/prepared",
            )?;

            let stream_result = self.start_turn_and_wait_for_stream(
                connection,
                vec![TurnInputItem::text(request.prompt)],
                None,
                None,
                &event_sender,
                AppServerPromptTraceContext {
                    workspace_dir: request.cwd.to_string(),
                    session_kind: "parallel-worker".to_string(),
                    operation: "isolated_parallel_thread".to_string(),
                    service_name: Some(request.service_name.to_string()),
                    developer_instructions: Some(request.developer_instructions.to_string()),
                    thread_id: thread_id.clone(),
                },
            );
            if stream_result
                .as_ref()
                .is_ok_and(ConversationTurnTerminalReceipt::is_completed_and_confirmed)
                && let Err(error) = connection.archive_thread(&thread_id)
            {
                tracing::warn!(
                    %thread_id,
                    error_chain_depth = error.chain().count(),
                    "failed to archive completed parallel worker thread"
                );
            }
            stream_result
        });

        finish_stream_result(result, &event_sender)
    }
}

#[derive(Debug, Clone)]
struct AppServerPromptTraceContext {
    workspace_dir: String,
    session_kind: String,
    operation: String,
    service_name: Option<String>,
    developer_instructions: Option<String>,
    thread_id: String,
}

#[derive(Debug, Default)]
struct AppServerPromptOutputCapture {
    output_items: Vec<AppServerPromptOutputRecord>,
}

impl AppServerPromptOutputCapture {
    fn record(&mut self, record: AppServerPromptOutputRecord) {
        if self.output_items.len() < APP_SERVER_PROMPT_LOG_MAX_ITEMS_PER_DIRECTION {
            self.output_items.push(record);
        }
    }
}

struct PromptLogStreamSender {
    event_sender: ConversationStreamSender,
    capture_sender: mpsc::SyncSender<AppServerPromptOutputRecord>,
}

impl AppServerEventSender for PromptLogStreamSender {
    fn try_send_prebounded(
        &self,
        event: ConversationStreamEvent,
    ) -> std::result::Result<(), AppServerEventTrySendError> {
        let capture_record = prompt_log_output_record(&event);
        match self.event_sender.try_send(event) {
            Ok(()) => {
                if let Some(record) = capture_record {
                    // Prompt logging is diagnostic-only. A saturated queue drops capture
                    // records instead of delaying the authoritative UI event stream.
                    let _ = self.capture_sender.try_send(record);
                }
                Ok(())
            }
            Err(mpsc::TrySendError::Full(_)) => Err(AppServerEventTrySendError::Full),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                Err(AppServerEventTrySendError::Disconnected)
            }
        }
    }
}

fn prompt_log_stream_forwarder(
    event_sender: ConversationStreamSender,
) -> (
    PromptLogStreamSender,
    thread::JoinHandle<AppServerPromptOutputCapture>,
) {
    let (capture_sender, capture_receiver) =
        mpsc::sync_channel(PROMPT_LOG_CAPTURE_CHANNEL_CAPACITY);
    let handle = thread::spawn(move || {
        let mut capture = AppServerPromptOutputCapture::default();
        for record in capture_receiver {
            capture.record(record);
        }
        capture
    });
    (
        PromptLogStreamSender {
            event_sender,
            capture_sender,
        },
        handle,
    )
}

fn prompt_log_output_record(
    event: &ConversationStreamEvent,
) -> Option<AppServerPromptOutputRecord> {
    let ConversationStreamEvent::AgentMessageCompleted {
        item_id,
        phase,
        text,
    } = event
    else {
        return None;
    };
    Some(AppServerPromptOutputRecord::new(
        bounded_prompt_log_string(item_id, APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS),
        phase.as_deref().map(|phase| {
            bounded_prompt_log_string(phase, APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS)
        }),
        bounded_prompt_log_string(text, APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS),
    ))
}

fn prompt_log_input_records(input: &[TurnInputItem]) -> Vec<AppServerPromptInputRecord> {
    input
        .iter()
        .take(APP_SERVER_PROMPT_LOG_MAX_ITEMS_PER_DIRECTION)
        .map(|item| match item {
            TurnInputItem::Text { text } => AppServerPromptInputRecord::new(
                "text",
                "turn input",
                bounded_prompt_log_string(text, APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS),
            ),
            TurnInputItem::Skill { name, path } => AppServerPromptInputRecord::new(
                "skill",
                bounded_prompt_log_string(name, APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS),
                bounded_prompt_log_string(path, APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS),
            ),
        })
        .collect()
}

fn reasoning_effort_label(effort: ReasoningEffortValue) -> &'static str {
    match effort {
        ReasoningEffortValue::None => "none",
        ReasoningEffortValue::Minimal => "minimal",
        ReasoningEffortValue::Low => "low",
        ReasoningEffortValue::Medium => "medium",
        ReasoningEffortValue::High => "high",
        ReasoningEffortValue::XHigh => "xhigh",
    }
}

fn next_prompt_log_interaction_id() -> String {
    let sequence = NEXT_PROMPT_LOG_INTERACTION_ID.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{}-{sequence}",
        std::process::id(),
        Utc::now().timestamp_millis()
    )
}

fn prompt_log_terminal_status(result: &Result<ConversationTurnTerminalReceipt>) -> &'static str {
    match result {
        Ok(receipt) if receipt.is_completed_and_confirmed() => "completed",
        Ok(receipt) if matches!(receipt.outcome, ConversationTurnTerminalOutcome::Completed) => {
            "recovery_pending"
        }
        Ok(receipt) => receipt.outcome.status_label(),
        Err(_) => "failed",
    }
}

fn prompt_log_terminal_error(result: &Result<ConversationTurnTerminalReceipt>) -> Option<String> {
    match result {
        Ok(receipt) if receipt.is_completed_and_confirmed() => None,
        Ok(receipt) if matches!(receipt.outcome, ConversationTurnTerminalOutcome::Completed) => {
            Some(format!(
                "completed upstream but application delivery was {}",
                prompt_log_delivery_label(receipt.application_delivery)
            ))
        }
        Ok(receipt) => Some(receipt.status_error_summary()),
        Err(error) => Some(persisted_error_summary(error)),
    }
}

fn prompt_log_delivery_label(delivery: ConversationTurnApplicationDelivery) -> &'static str {
    match delivery {
        ConversationTurnApplicationDelivery::Pending => "pending",
        ConversationTurnApplicationDelivery::Confirmed => "confirmed",
        ConversationTurnApplicationDelivery::Unconfirmed(_) => "unconfirmed",
    }
}

fn finish_stream_result(
    result: Result<ConversationTurnTerminalReceipt>,
    event_sender: &ConversationStreamSender,
) -> Result<ConversationTurnTerminalReceipt> {
    /*
     * Stream callers need both an Err return and a Failed event. The Err drives
     * service-level error handling, while the event lets TUI state leave streaming
     * mode even when the caller does not own the render state directly.
     */
    if let Err(error) = &result {
        let _ = AppServerEventSender::send(
            event_sender,
            ConversationStreamEvent::Failed {
                message: error.to_string(),
            },
        );
    }

    result
}

pub(super) fn persisted_error_summary(error: &anyhow::Error) -> String {
    format!(
        "app-server error redacted (message_chars={}, chain_depth={})",
        error.to_string().chars().count(),
        error.chain().count()
    )
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    #[cfg(unix)]
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    use anyhow::Result;
    #[cfg(unix)]
    use serde_json::Value;

    #[cfg(unix)]
    use super::PlanningThreadLauncher;
    use super::connection::{AppServerConnectionConfig, AppServerTurnInterruptSignal};
    use super::execution_policy::AppServerExecutionPolicy;
    use super::protocol::{
        ApprovalPolicyValue, ApprovalsReviewerValue, ReasoningEffortValue, SandboxModeValue,
        ThreadStartParams, TurnInputItem,
    };
    use super::{
        AppServerEventSender, AppServerPromptOutputCapture, CodexAppServerAdapter,
        ConversationTurnApplicationDelivery, ConversationTurnTerminalReceipt,
        MAX_STREAM_CHANGED_PATHS, MAX_STREAM_COMPLETED_MESSAGE_BYTES,
        PLANNING_WORKER_DEVELOPER_INSTRUCTIONS, PLANNING_WORKER_SERVICE_NAME,
        PlanningWorkerContinuationWatcher, STREAM_TRUNCATION_MARKER,
        bounded_app_server_stream_event, codex_raw_trust_key, finish_stream_result,
        persisted_error_summary, prompt_log_input_records, prompt_log_output_record,
        prompt_log_stream_forwarder, prompt_log_terminal_error, prompt_log_terminal_status,
        protected_planning_thread_workspace, protected_thread_workspace, reasoning_effort_label,
        send_required_app_server_event,
    };
    #[cfg(unix)]
    use super::{ConversationTurnTerminalOutcome, PLANNING_WORKER_MODEL};
    #[cfg(unix)]
    use crate::application::port::outbound::app_server_prompt_log_port::{
        AppServerPromptInteractionRecord, AppServerPromptInteractionSnapshot,
        AppServerPromptLogPort,
    };
    use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
    #[cfg(unix)]
    use crate::application::port::outbound::parallel_agent_worker_port::{
        ParallelAgentWorkerPort, ParallelAgentWorkerStreamRequest,
    };
    #[cfg(unix)]
    use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
    #[cfg(unix)]
    use crate::application::port::outbound::startup_probe_port::StartupProbePort;
    use crate::application::service::conversation_runtime_event::{
        ConversationStreamEvent, conversation_stream_channel,
    };
    use crate::application::service::planning::task_tool::{
        PLANNING_TOOL_PARENT_THREAD_ID_ENV, PLANNING_TOOL_PARENT_TURN_ID_ENV,
    };
    use crate::domain::conversation::{
        ConversationApprovalRequest, ConversationApprovalRequestKind,
        ConversationRuntimeControlTruth,
    };
    #[cfg(unix)]
    use crate::domain::conversation::{ConversationReasoningEffort, ConversationTurnOptions};
    use crate::domain::conversation_progressive_activity::{
        ConversationProgressiveActivityBatch, ConversationProgressiveActivityKind,
        ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
    };
    #[cfg(unix)]
    use crate::domain::conversation_runtime_envelope::{
        ConversationRuntimeApprovalPolicy, ConversationRuntimeApprovalsReviewer,
        ConversationRuntimeConfigurationRequest, ConversationRuntimeModelRerouteReason,
        ConversationRuntimeProcessEnvironment, ConversationRuntimeRequestedValue,
        ConversationRuntimeSandboxPolicy, ConversationRuntimeShellEnvironment,
        ConversationRuntimeThreadStatus,
    };
    use crate::domain::conversation_runtime_envelope::{
        ConversationRuntimeEnvelope, ConversationRuntimeEnvelopeObservation,
        ConversationRuntimeObservedValue,
    };
    #[cfg(unix)]
    use crate::domain::recent_sessions::{
        SessionCatalog, SessionCatalogRequest, SessionCatalogTier, SessionRenameRequest,
    };

    #[cfg(unix)]
    #[test]
    fn startup_catalog_and_snapshot_ports_reuse_shared_app_server_runtime() {
        let fake_codex = FakeCodex::install("shared-runtime");
        let adapter = test_adapter_with_fake(&fake_codex);

        let startup = adapter
            .load_startup_context()
            .expect("startup context should come from fake app-server");
        assert_eq!(
            startup.initialize_detail,
            "linux-x64 / unix / codex-app-server/fake / app-server policy: approval=on-request, reviewer=user, sandbox=workspace-write, process-env=scrubbed, api-key-auth=disabled, shell-env=core"
        );
        assert_eq!(
            startup.account_detail,
            "chatgpt / operator@example.com / plus"
        );
        assert!(startup.account_ok);
        assert!(startup.warnings.is_empty());

        let catalog = adapter
            .load_session_catalog(SessionCatalogRequest::for_workspace(5, "/repo"))
            .expect("session catalog should come from fake app-server");
        let SessionCatalog::Ready {
            tier,
            recent_sessions,
        } = catalog
        else {
            panic!("fake app-server should produce a ready provider catalog");
        };
        assert_eq!(tier, SessionCatalogTier::ProviderBackedCatalog);
        assert_eq!(recent_sessions.items[0].id, "listed-thread");
        assert_eq!(recent_sessions.items[0].git_branch.as_deref(), Some("main"));
        assert_eq!(recent_sessions.next_cursor.as_deref(), Some("cursor-next"));

        let snapshot = adapter
            .load_conversation_snapshot("resume-thread")
            .expect("conversation snapshot should come from fake app-server");
        assert_eq!(snapshot.thread_id, "resume-thread");
        assert_eq!(snapshot.title, "Fake resume-thread");
        assert!(snapshot.warnings.is_empty());

        let methods = fake_codex.logged_methods();
        assert_eq!(
            methods,
            [
                "initialize",
                "initialized",
                "account/read",
                "thread/list",
                "thread/read"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn session_catalog_port_renames_the_exact_app_server_thread() {
        let fake_codex = FakeCodex::install("session-rename");
        let adapter = test_adapter_with_fake(&fake_codex);

        adapter
            .rename_session(SessionRenameRequest::new(
                "thread-exact",
                "Release follow-up",
            ))
            .expect("session rename should succeed");

        let request = fake_codex
            .logged_requests()
            .into_iter()
            .find(|request| request["method"] == "thread/name/set")
            .expect("thread/name/set should be sent");
        assert_eq!(request["params"]["threadId"], "thread-exact");
        assert_eq!(request["params"]["name"], "Release follow-up");
    }

    #[cfg(unix)]
    #[test]
    fn shared_runtime_retries_after_first_failure_and_returns_retry_notice() {
        let fake_codex =
            FakeCodex::install_with_scenario("shared-runtime-retry", "fail_account_once");
        let adapter = test_adapter_with_fake(&fake_codex);

        let startup = adapter
            .load_startup_context()
            .expect("startup should retry with a fresh shared runtime");

        assert!(startup.account_ok);
        assert!(startup.warnings.iter().any(|warning| {
            warning.contains("shared runtime reset after startup checks request failure")
                && warning.contains("forced one-time account/read failure")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn shared_runtime_final_failure_keeps_request_kind_context() {
        let fake_codex =
            FakeCodex::install_with_scenario("shared-runtime-final-failure", "fail_account_always");
        let adapter = test_adapter_with_fake(&fake_codex);

        let error = adapter
            .load_startup_context()
            .expect_err("startup should fail after shared retry is exhausted");

        let message = format!("{error:#}");
        assert!(message.contains("startup checks request still failed after resetting"));
        assert!(message.contains("forced account/read failure"));
    }

    #[cfg(unix)]
    #[test]
    fn short_requests_use_isolated_fallback_while_shared_runtime_is_locked() {
        let fake_codex = FakeCodex::install("isolated-fallback-success");
        let adapter = test_adapter_with_fake(&fake_codex);
        let _stream_guard = adapter
            .shared_runtime
            .lock()
            .expect("shared runtime lock should be held by simulated stream");

        let catalog = adapter
            .load_session_catalog(SessionCatalogRequest::for_workspace(3, "/repo"))
            .expect(
                "recent sessions should use isolated fallback while stream owns shared runtime",
            );

        let SessionCatalog::Ready {
            recent_sessions, ..
        } = catalog
        else {
            panic!("fake app-server should produce a ready provider catalog");
        };
        assert_eq!(recent_sessions.items[0].id, "listed-thread");
        assert!(recent_sessions.warnings.iter().any(|warning| {
            warning.contains(
                "recent sessions request used an isolated app-server connection while a turn stream was active",
            )
        }));
    }

    #[cfg(unix)]
    #[test]
    fn isolated_fallback_final_failure_reports_busy_stream_context() {
        let fake_codex = FakeCodex::install_with_scenario(
            "isolated-fallback-final-failure",
            "fail_thread_list_always",
        );
        let adapter = test_adapter_with_fake(&fake_codex);
        let _stream_guard = adapter
            .shared_runtime
            .lock()
            .expect("shared runtime lock should be held by simulated stream");

        let error = adapter
            .load_session_catalog(SessionCatalogRequest::for_workspace(3, "/repo"))
            .expect_err("isolated fallback should fail after retry is exhausted");

        let message = format!("{error:#}");
        assert!(message.contains("recent sessions request still failed on isolated retry"));
        assert!(message.contains("forced thread/list failure"));
    }

    #[cfg(unix)]
    #[test]
    fn user_thread_streams_emit_launch_reattach_and_completion_events() {
        let fake_codex = FakeCodex::install("user-streams");
        let adapter = test_adapter_with_fake(&fake_codex);

        let (new_tx, new_rx) = conversation_stream_channel();
        adapter
            .run_new_thread_stream(
                "/repo",
                "start a new session",
                ConversationTurnOptions::default(),
                new_tx,
            )
            .expect("new thread stream should complete");
        let new_events = new_rx.try_iter().collect::<Vec<_>>();
        assert!(has_launch_attachment(&new_events));
        assert!(has_thread_prepared(&new_events, "started-thread"));
        assert!(has_turn_completed(&new_events));
        let new_envelope = prepared_runtime_envelope(&new_events, "started-thread");
        assert_eq!(
            new_envelope.thread_request.model,
            ConversationRuntimeRequestedValue::Omitted
        );
        assert_eq!(
            new_envelope.applied.model,
            ConversationRuntimeObservedValue::Observed("gpt-applied".to_string())
        );
        assert_eq!(
            new_envelope.applied.reasoning_effort,
            ConversationRuntimeObservedValue::Observed("medium".to_string())
        );
        assert_eq!(
            new_envelope.applied.permission_profile,
            ConversationRuntimeObservedValue::UnavailableOnStableResponse
        );
        assert_eq!(
            new_envelope.launch_environment.process_environment,
            ConversationRuntimeProcessEnvironment::Scrubbed
        );
        assert_eq!(
            new_envelope.launch_environment.shell_environment,
            ConversationRuntimeShellEnvironment::Core
        );
        assert!(!new_envelope.launch_environment.api_key_auth);
        let new_turn_request = started_runtime_request(&new_events);
        assert_eq!(
            new_turn_request.model.as_value().map(String::as_str),
            Some("gpt-5.5")
        );
        assert_eq!(
            new_turn_request
                .reasoning_effort
                .as_value()
                .map(String::as_str),
            Some("high")
        );
        assert!(matches!(
            new_turn_request.sandbox,
            ConversationRuntimeRequestedValue::Value(
                ConversationRuntimeSandboxPolicy::WorkspaceWrite { .. }
            )
        ));

        let (resume_tx, resume_rx) = conversation_stream_channel();
        adapter
            .run_turn_stream(
                "resume-thread",
                "continue session",
                ConversationTurnOptions::default(),
                resume_tx,
            )
            .expect("existing thread stream should complete");
        let resume_events = resume_rx.try_iter().collect::<Vec<_>>();
        assert!(has_reattach_attachment(&resume_events));
        assert!(has_thread_prepared(&resume_events, "resume-thread"));
        assert!(has_turn_completed(&resume_events));
        let resume_envelope = prepared_runtime_envelope(&resume_events, "resume-thread");
        assert_eq!(
            resume_envelope.applied.model,
            ConversationRuntimeObservedValue::Observed("gpt-resumed".to_string())
        );
        assert_eq!(
            resume_envelope
                .thread_request
                .cwd
                .as_value()
                .map(String::as_str),
            Some("/repo")
        );
        assert_eq!(
            started_runtime_request(&resume_events)
                .model
                .as_value()
                .map(String::as_str),
            Some("gpt-5.5")
        );

        let methods = fake_codex.logged_methods();
        assert_eq!(
            methods,
            [
                "initialize",
                "initialized",
                "thread/start",
                "turn/start",
                "thread/read",
                "thread/resume",
                "turn/start"
            ]
        );
        let turn_starts = fake_codex
            .logged_requests()
            .into_iter()
            .filter(|request| request["method"] == "turn/start")
            .collect::<Vec<_>>();
        assert_eq!(turn_starts[0]["params"]["model"], "gpt-5.5");
        assert_eq!(turn_starts[0]["params"]["effort"], "high");
        assert_eq!(turn_starts[1]["params"]["model"], "gpt-5.5");
        assert_eq!(turn_starts[1]["params"]["effort"], "high");
    }

    #[cfg(unix)]
    #[test]
    fn user_thread_streams_pass_turn_option_overrides_to_app_server() {
        let fake_codex = FakeCodex::install("user-turn-options");
        let adapter = test_adapter_with_fake(&fake_codex);
        let options = ConversationTurnOptions {
            model: Some("gpt-5.4".to_string()),
            reasoning_effort: Some(ConversationReasoningEffort::High),
        };

        let (new_tx, new_rx) = conversation_stream_channel();
        adapter
            .run_new_thread_stream("/repo", "start with overrides", options.clone(), new_tx)
            .expect("new thread stream should complete");
        assert!(has_turn_completed(&new_rx.try_iter().collect::<Vec<_>>()));

        let (resume_tx, resume_rx) = conversation_stream_channel();
        adapter
            .run_turn_stream(
                "resume-thread",
                "continue with overrides",
                options,
                resume_tx,
            )
            .expect("existing thread stream should complete");
        assert!(has_turn_completed(
            &resume_rx.try_iter().collect::<Vec<_>>()
        ));

        let requests = fake_codex.logged_requests();
        let thread_starts = requests
            .iter()
            .filter(|request| request["method"] == "thread/start")
            .collect::<Vec<_>>();
        let thread_resumes = requests
            .iter()
            .filter(|request| request["method"] == "thread/resume")
            .collect::<Vec<_>>();
        let turn_starts = requests
            .iter()
            .filter(|request| request["method"] == "turn/start")
            .collect::<Vec<_>>();

        assert!(thread_starts[0]["params"]["model"].is_null());
        assert_eq!(thread_starts[0]["params"]["sandbox"], "read-only");
        assert_eq!(
            thread_starts[0]["params"]["config"]["projects"]["/repo"]["trust_level"],
            "untrusted"
        );
        assert_eq!(thread_resumes[0]["params"]["cwd"], "/repo");
        assert_eq!(thread_resumes[0]["params"]["sandbox"], "read-only");
        assert_eq!(
            thread_resumes[0]["params"]["config"]["projects"]["/repo"]["trust_level"],
            "untrusted"
        );
        assert_eq!(turn_starts[0]["params"]["model"], "gpt-5.4");
        assert_eq!(turn_starts[0]["params"]["effort"], "high");
        assert_eq!(
            turn_starts[0]["params"]["sandboxPolicy"]["type"],
            "workspaceWrite"
        );
        assert_eq!(turn_starts[1]["params"]["model"], "gpt-5.4");
        assert_eq!(turn_starts[1]["params"]["effort"], "high");
        assert_eq!(
            turn_starts[1]["params"]["sandboxPolicy"]["type"],
            "workspaceWrite"
        );
    }

    #[cfg(unix)]
    #[test]
    fn early_settings_reroute_and_status_replay_after_turn_start_in_fifo_order() {
        let fake_codex =
            FakeCodex::install_with_scenario("early-runtime-envelope", "early_runtime_envelope");
        let adapter = test_adapter_with_fake(&fake_codex);
        let (event_sender, event_receiver) = conversation_stream_channel();

        let receipt = adapter
            .run_new_thread_stream(
                "/repo",
                "observe runtime envelope",
                ConversationTurnOptions::default(),
                event_sender,
            )
            .expect("early typed observations should not prevent terminal completion");
        assert!(receipt.is_completed_and_confirmed());
        let events = event_receiver.try_iter().collect::<Vec<_>>();
        let turn_started_index = events
            .iter()
            .position(|event| matches!(event, ConversationStreamEvent::TurnStarted { .. }))
            .expect("turn/start response should emit the accepted request envelope");
        let first_observation_index = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    ConversationStreamEvent::RuntimeEnvelopeObserved { .. }
                )
            })
            .expect("deferred runtime observations should replay");
        assert!(turn_started_index < first_observation_index);
        let observations = events
            .iter()
            .filter_map(|event| match event {
                ConversationStreamEvent::RuntimeEnvelopeObserved { observation } => {
                    Some(observation.as_ref())
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(observations.len(), 3);
        assert!(matches!(
            observations[0],
            ConversationRuntimeEnvelopeObservation::SettingsUpdated { settings, .. }
                if settings.model
                    == ConversationRuntimeObservedValue::Observed("gpt-5.5".to_string())
        ));
        assert!(matches!(
            observations[1],
            ConversationRuntimeEnvelopeObservation::ModelRerouted { reroute, .. }
                if reroute.from_model == "gpt-5.5"
                    && reroute.to_model == "gpt-rerouted"
                    && reroute.reason
                        == ConversationRuntimeModelRerouteReason::HighRiskCyberActivity
        ));
        assert!(matches!(
            observations[2],
            ConversationRuntimeEnvelopeObservation::ThreadStatusChanged {
                status: ConversationRuntimeObservedValue::Observed(
                    ConversationRuntimeThreadStatus::Active {
                        waiting_on_approval: true,
                        ..
                    }
                ),
                ..
            }
        ));
        assert!(matches!(
            events.last(),
            Some(ConversationStreamEvent::TurnTerminal { receipt })
                if receipt.is_completed_and_confirmed()
        ));
        let mut application_projection = crate::application::service::conversation_runtime_event::ConversationRuntimeEnvelopeProjection::default();
        for event in &events {
            application_projection.apply_event(event);
        }
        assert!(application_projection.last_rejection.is_none());
        let envelope = application_projection
            .runtime_envelope
            .expect("application reducer should retain the observed envelope");
        assert_eq!(
            envelope.applied.model,
            ConversationRuntimeObservedValue::Observed("gpt-rerouted".to_string())
        );
        assert!(matches!(
            envelope.thread_status,
            ConversationRuntimeObservedValue::Observed(ConversationRuntimeThreadStatus::Active {
                waiting_on_approval: true,
                ..
            })
        ));
        assert_eq!(envelope.observation_sequence, 3);
    }

    #[cfg(unix)]
    #[test]
    fn pre_thread_response_settings_cannot_override_later_start_or_resume_envelope() {
        let start_fake =
            FakeCodex::install_with_scenario("pre-start-settings", "pre_thread_response_settings");
        let start_adapter = test_adapter_with_fake(&start_fake);
        let (start_sender, start_receiver) = conversation_stream_channel();
        start_adapter
            .run_new_thread_stream(
                "/repo",
                "start after stale settings",
                ConversationTurnOptions::default(),
                start_sender,
            )
            .expect("later thread/start response should remain authoritative");
        let start_events = start_receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(
            prepared_runtime_envelope(&start_events, "started-thread")
                .applied
                .model,
            ConversationRuntimeObservedValue::Observed("gpt-applied".to_string())
        );
        assert!(!start_events.iter().any(|event| matches!(
            event,
            ConversationStreamEvent::RuntimeEnvelopeObserved { observation }
                if matches!(
                    observation.as_ref(),
                    ConversationRuntimeEnvelopeObservation::SettingsUpdated { settings, .. }
                        if settings.model
                            == ConversationRuntimeObservedValue::Observed(
                                "stale-before-response".to_string()
                            )
                )
        )));

        let resume_fake =
            FakeCodex::install_with_scenario("pre-resume-settings", "pre_thread_response_settings");
        let resume_adapter = test_adapter_with_fake(&resume_fake);
        let (resume_sender, resume_receiver) = conversation_stream_channel();
        resume_adapter
            .run_turn_stream(
                "resume-thread",
                "resume after stale settings",
                ConversationTurnOptions::default(),
                resume_sender,
            )
            .expect("later thread/resume response should remain authoritative");
        let resume_events = resume_receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(
            prepared_runtime_envelope(&resume_events, "resume-thread")
                .applied
                .model,
            ConversationRuntimeObservedValue::Observed("gpt-resumed".to_string())
        );
        assert!(!resume_events.iter().any(|event| matches!(
            event,
            ConversationStreamEvent::RuntimeEnvelopeObserved { observation }
                if matches!(
                    observation.as_ref(),
                    ConversationRuntimeEnvelopeObservation::SettingsUpdated { settings, .. }
                        if settings.model
                            == ConversationRuntimeObservedValue::Observed(
                                "stale-before-response".to_string()
                            )
                )
        )));
    }

    #[cfg(unix)]
    #[test]
    fn hidden_planning_stays_ephemeral_while_parallel_threads_are_readable_for_peek() {
        let fake_codex = FakeCodex::install("isolated-workers");
        let adapter = test_adapter_with_fake(&fake_codex);

        let (planning_tx, planning_rx) = conversation_stream_channel();
        adapter
            .run_hidden_planning_thread("/repo", "refresh queue", None, None, planning_tx, None)
            .expect("hidden planning worker stream should complete");
        let planning_events = planning_rx.try_iter().collect::<Vec<_>>();
        assert!(has_thread_prepared(&planning_events, "started-thread"));
        assert!(has_turn_completed(&planning_events));
        let planning_envelope = prepared_runtime_envelope(&planning_events, "started-thread");
        assert_eq!(
            planning_envelope
                .thread_request
                .model
                .as_value()
                .map(String::as_str),
            Some(PLANNING_WORKER_MODEL)
        );
        assert_eq!(
            planning_envelope.applied.model,
            ConversationRuntimeObservedValue::Observed(PLANNING_WORKER_MODEL.to_string())
        );
        let planning_turn_request = started_runtime_request(&planning_events);
        assert_eq!(
            planning_turn_request
                .reasoning_effort
                .as_value()
                .map(String::as_str),
            Some("medium")
        );
        assert_eq!(
            planning_turn_request.approval_policy,
            ConversationRuntimeRequestedValue::Value(ConversationRuntimeApprovalPolicy::Never)
        );

        let (parallel_tx, parallel_rx) = conversation_stream_channel();
        adapter
            .run_isolated_new_thread_stream(
                ParallelAgentWorkerStreamRequest {
                    cwd: "/repo/slot-1",
                    prompt: "implement task",
                    developer_instructions: "You are an isolated worker.",
                    service_name: "akra-parallel-worker",
                },
                parallel_tx,
            )
            .expect("parallel worker stream should complete");
        let parallel_events = parallel_rx.try_iter().collect::<Vec<_>>();
        assert!(has_thread_prepared(&parallel_events, "started-thread"));
        assert!(has_turn_completed(&parallel_events));
        let parallel_envelope = prepared_runtime_envelope(&parallel_events, "started-thread");
        assert_eq!(
            parallel_envelope.thread_request.model,
            ConversationRuntimeRequestedValue::Omitted
        );
        assert_eq!(
            parallel_envelope.applied.model,
            ConversationRuntimeObservedValue::Observed("gpt-applied".to_string())
        );
        let parallel_turn_request = started_runtime_request(&parallel_events);
        assert_eq!(
            parallel_turn_request.model,
            ConversationRuntimeRequestedValue::Omitted
        );
        assert_eq!(
            parallel_turn_request.reasoning_effort,
            ConversationRuntimeRequestedValue::Omitted
        );
        assert_eq!(
            parallel_turn_request.approvals_reviewer,
            ConversationRuntimeRequestedValue::Value(ConversationRuntimeApprovalsReviewer::User)
        );

        let requests = fake_codex.logged_requests();
        let thread_starts = requests
            .iter()
            .filter(|request| request["method"] == "thread/start")
            .collect::<Vec<_>>();
        assert_eq!(thread_starts.len(), 2);

        assert_eq!(
            thread_starts[0]["params"]["serviceName"],
            PLANNING_WORKER_SERVICE_NAME
        );
        assert_eq!(thread_starts[0]["params"]["model"], "gpt-5.4");
        assert_eq!(thread_starts[0]["params"]["ephemeral"], true);
        assert_eq!(thread_starts[0]["params"]["approvalPolicy"], "never");
        assert_eq!(thread_starts[0]["params"]["sandbox"], "read-only");
        assert_eq!(
            thread_starts[0]["params"]["config"]["projects"]["/repo"]["trust_level"],
            "untrusted"
        );
        assert!(
            thread_starts[0]["params"]["developerInstructions"]
                .as_str()
                .is_some_and(|value| value.contains("planning-only sub-session"))
        );

        assert_eq!(
            thread_starts[1]["params"]["serviceName"],
            "akra-parallel-worker"
        );
        assert_eq!(thread_starts[1]["params"]["ephemeral"], false);
        assert_eq!(
            thread_starts[1]["params"]["developerInstructions"],
            "You are an isolated worker."
        );
        assert_eq!(thread_starts[1]["params"]["sandbox"], "read-only");
        assert_eq!(
            thread_starts[1]["params"]["config"]["projects"]["/repo/slot-1"]["trust_level"],
            "untrusted"
        );
        let turn_starts = requests
            .iter()
            .filter(|request| request["method"] == "turn/start")
            .collect::<Vec<_>>();
        assert_eq!(turn_starts[0]["params"]["approvalPolicy"], "never");
        assert_eq!(
            turn_starts[0]["params"]["sandboxPolicy"]["type"],
            "readOnly"
        );
        let thread_archives = requests
            .iter()
            .filter(|request| request["method"] == "thread/archive")
            .collect::<Vec<_>>();
        assert_eq!(thread_archives.len(), 1);
        assert_eq!(thread_archives[0]["params"]["threadId"], "started-thread");
    }

    #[cfg(unix)]
    #[test]
    fn parallel_archive_requires_confirmed_completed_terminal_receipt() {
        for scenario in [
            "terminal_interrupted",
            "terminal_failed",
            "terminal_unknown",
        ] {
            let fake_codex =
                FakeCodex::install_with_scenario(&format!("parallel-archive-{scenario}"), scenario);
            let adapter = test_adapter_with_fake(&fake_codex);
            let (event_sender, _event_receiver) = conversation_stream_channel();

            let receipt = adapter
                .run_isolated_new_thread_stream(
                    ParallelAgentWorkerStreamRequest {
                        cwd: "/repo/slot-1",
                        prompt: "do not archive an uncompleted turn",
                        developer_instructions: "test terminal truth",
                        service_name: "akra-parallel-worker",
                    },
                    event_sender,
                )
                .expect("non-success terminal outcome is still a closed transport receipt");

            assert!(
                !receipt.is_completed_and_confirmed(),
                "scenario: {scenario}"
            );
            assert!(
                fake_codex
                    .logged_requests()
                    .iter()
                    .all(|request| request["method"] != "thread/archive"),
                "scenario: {scenario}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn parallel_archive_failure_preserves_confirmed_completion_receipt() {
        let fake_codex =
            FakeCodex::install_with_scenario("parallel-archive-failure", "fail_thread_archive");
        let adapter = test_adapter_with_fake(&fake_codex);
        let (event_sender, event_receiver) = conversation_stream_channel();

        let receipt = adapter
            .run_isolated_new_thread_stream(
                ParallelAgentWorkerStreamRequest {
                    cwd: "/repo/slot-1",
                    prompt: "complete before archive failure",
                    developer_instructions: "test archive cleanup semantics",
                    service_name: "akra-parallel-worker",
                },
                event_sender,
            )
            .expect("archive cleanup failure should remain nonfatal after confirmed completion");
        let events = event_receiver.try_iter().collect::<Vec<_>>();

        assert!(receipt.is_completed_and_confirmed());
        assert!(events.iter().any(|event| {
            matches!(
                event,
                ConversationStreamEvent::TurnTerminal {
                    receipt: event_receipt,
                } if event_receipt == &receipt
            )
        }));
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, ConversationStreamEvent::Failed { .. }))
        );
        assert!(
            fake_codex
                .logged_requests()
                .iter()
                .any(|request| request["method"] == "thread/archive")
        );
    }

    #[test]
    fn hidden_planning_continuation_watcher_interrupts_only_its_local_signal() {
        let gate = crate::domain::planning::PostTurnContinuationGate::default();
        let permit = gate.capture();
        let local_signal = AppServerTurnInterruptSignal::default();
        let unrelated_signal = AppServerTurnInterruptSignal::default();
        let local_generation = local_signal.current_generation();
        let unrelated_generation = unrelated_signal.current_generation();
        let _watcher = PlanningWorkerContinuationWatcher::start(Some(permit), local_signal.clone())
            .expect("continuation permit should create a watcher");

        gate.advance();

        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while local_signal.current_generation() == local_generation
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(local_signal.current_generation() > local_generation);
        assert_eq!(unrelated_signal.current_generation(), unrelated_generation);
    }

    #[cfg(unix)]
    #[test]
    fn app_server_streams_record_prompt_log_entries() {
        let fake_codex = FakeCodex::install("prompt-log");
        let prompt_log = Arc::new(RecordingPromptLogPort::default());
        let adapter = CodexAppServerAdapter::with_configs_and_prompt_log(
            "test-client",
            "test-version",
            fake_codex.connection_config(),
            AppServerExecutionPolicy::default(),
            prompt_log.clone(),
        );

        let (main_tx, main_rx) = conversation_stream_channel();
        adapter
            .run_new_thread_stream(
                "/repo",
                "start a logged session",
                ConversationTurnOptions::default(),
                main_tx,
            )
            .expect("main stream should complete");
        assert!(has_turn_completed(&main_rx.try_iter().collect::<Vec<_>>()));

        let (worker_tx, worker_rx) = conversation_stream_channel();
        adapter
            .run_isolated_new_thread_stream(
                ParallelAgentWorkerStreamRequest {
                    cwd: "/repo/slot-1",
                    prompt: "implement logged task",
                    developer_instructions: "worker developer instructions",
                    service_name: "akra-parallel-worker",
                },
                worker_tx,
            )
            .expect("parallel stream should complete");
        assert!(has_turn_completed(
            &worker_rx.try_iter().collect::<Vec<_>>()
        ));

        let records = prompt_log.records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].session_kind, "main");
        assert_eq!(records[0].operation, "new_thread_turn");
        assert_eq!(records[0].input_items[0].content, "start a logged session");
        assert_eq!(records[0].output_items[0].text, "fake final response");
        assert_eq!(records[1].session_kind, "parallel-worker");
        assert_eq!(
            records[1].developer_instructions.as_deref(),
            Some("worker developer instructions")
        );
        assert_eq!(
            records[1].service_name.as_deref(),
            Some("akra-parallel-worker")
        );
        assert!(!fake_codex.logged_methods().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn prompt_log_preserves_each_non_success_terminal_status() {
        for (scenario, expected_status) in [
            ("terminal_interrupted", "interrupted"),
            ("terminal_failed", "failed"),
            ("terminal_unknown", "unknown"),
        ] {
            let fake_codex =
                FakeCodex::install_with_scenario(&format!("prompt-log-{scenario}"), scenario);
            let prompt_log = Arc::new(RecordingPromptLogPort::default());
            let adapter = CodexAppServerAdapter::with_configs_and_prompt_log(
                "test-client",
                "test-version",
                fake_codex.connection_config(),
                AppServerExecutionPolicy::default(),
                prompt_log.clone(),
            );
            let (event_sender, _event_receiver) = conversation_stream_channel();

            let receipt = adapter
                .run_new_thread_stream(
                    "/repo",
                    "record terminal status",
                    ConversationTurnOptions::default(),
                    event_sender,
                )
                .expect("terminal status should return a closed receipt");

            assert_eq!(receipt.outcome.status_label(), expected_status);
            assert!(!receipt.is_completed_and_confirmed());
            let records = prompt_log.records();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].status, expected_status);
            assert!(records[0].error_message.is_some());
        }
    }

    #[cfg(unix)]
    #[test]
    fn retry_then_completed_is_the_only_retry_path_logged_as_completed() {
        let fake_codex =
            FakeCodex::install_with_scenario("prompt-log-retry", "terminal_retry_completed");
        let prompt_log = Arc::new(RecordingPromptLogPort::default());
        let adapter = CodexAppServerAdapter::with_configs_and_prompt_log(
            "test-client",
            "test-version",
            fake_codex.connection_config(),
            AppServerExecutionPolicy::default(),
            prompt_log.clone(),
        );
        let (event_sender, event_receiver) = conversation_stream_channel();

        let receipt = adapter
            .run_new_thread_stream(
                "/repo",
                "retry then finish",
                ConversationTurnOptions::default(),
                event_sender,
            )
            .expect("retrying stream should reach its authoritative completion");
        let events = event_receiver.try_iter().collect::<Vec<_>>();

        assert!(receipt.is_completed_and_confirmed());
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ConversationStreamEvent::TurnRetrying { .. }))
        );
        assert!(has_turn_completed(&events));
        assert_eq!(prompt_log.records()[0].status, "completed");
        assert!(matches!(
            receipt.outcome,
            ConversationTurnTerminalOutcome::Completed
        ));
    }

    #[cfg(unix)]
    #[test]
    fn disabled_prompt_log_skips_recording_and_stream_capture_path() {
        let fake_codex = FakeCodex::install("prompt-log-disabled");
        let adapter = CodexAppServerAdapter::with_configs_and_prompt_log(
            "test-client",
            "test-version",
            fake_codex.connection_config(),
            AppServerExecutionPolicy::default(),
            Arc::new(DisabledPromptLogPort),
        );
        let (event_sender, event_receiver) = conversation_stream_channel();

        adapter
            .run_new_thread_stream(
                "/repo",
                "do not retain this prompt",
                ConversationTurnOptions::default(),
                event_sender,
            )
            .expect("disabled prompt logging should use the direct stream path");

        assert!(has_turn_completed(
            &event_receiver.try_iter().collect::<Vec<_>>()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn stream_start_failures_emit_failed_event_and_failed_prompt_log_record() {
        let fake_codex =
            FakeCodex::install_with_scenario("stream-start-failure", "fail_turn_start");
        let prompt_log = Arc::new(RecordingPromptLogPort::default());
        let adapter = CodexAppServerAdapter::with_configs_and_prompt_log(
            "test-client",
            "test-version",
            fake_codex.connection_config(),
            AppServerExecutionPolicy::default(),
            prompt_log.clone(),
        );

        let (tx, rx) = conversation_stream_channel();
        let error = adapter
            .run_new_thread_stream(
                "/repo",
                "start a failing stream",
                ConversationTurnOptions::default(),
                tx,
            )
            .expect_err("turn/start failure should fail the stream");

        assert!(error.to_string().contains("forced turn/start failure"));
        assert!(rx
            .try_iter()
            .any(|event| matches!(event, ConversationStreamEvent::Failed { message } if message.contains("forced turn/start failure"))));

        let records = prompt_log.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, "failed");
        assert_eq!(records[0].thread_id.as_deref(), Some("started-thread"));
        assert!(records[0].turn_id.is_none());
        assert_eq!(records[0].input_items[0].content, "start a failing stream");
        assert!(
            records[0]
                .error_message
                .as_deref()
                .is_some_and(|message| message.starts_with("app-server error redacted"))
        );
        assert!(
            !records[0]
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("forced turn/start failure"))
        );

        let startup = adapter
            .load_startup_context()
            .expect("startup should reconnect after stream failure reset");
        assert!(startup.warnings.iter().any(|warning| {
            warning.contains("shared runtime reset after turn stream failure")
                && warning.contains("forced turn/start failure")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn prompt_log_append_errors_do_not_fail_streams() {
        let fake_codex = FakeCodex::install("prompt-log-append-failure");
        let adapter = CodexAppServerAdapter::with_configs_and_prompt_log(
            "test-client",
            "test-version",
            fake_codex.connection_config(),
            AppServerExecutionPolicy::default(),
            Arc::new(FailingPromptLogPort),
        );

        let (tx, rx) = conversation_stream_channel();
        adapter
            .run_new_thread_stream(
                "/repo",
                "prompt log write fails",
                ConversationTurnOptions::default(),
                tx,
            )
            .expect("prompt log append failure should stay warning-only");

        assert!(has_turn_completed(&rx.try_iter().collect::<Vec<_>>()));
    }

    #[test]
    fn runtime_control_and_stop_requests_are_app_server_truths() {
        let adapter = test_adapter();

        assert_eq!(
            adapter.runtime_control_truth(),
            ConversationRuntimeControlTruth::codex_app_server()
        );
        adapter
            .request_stop_all_sessions()
            .expect("stop request should update interrupt generation without IO");
    }

    #[test]
    fn startup_policy_summary_is_informational_and_only_elevated_risks_warn() {
        let adapter = test_adapter();
        assert_eq!(
            adapter.active_policy_summary(),
            "app-server policy: approval=on-request, reviewer=user, sandbox=workspace-write, process-env=scrubbed, api-key-auth=disabled, shell-env=core"
        );
        assert!(adapter.elevated_policy_warnings().is_empty());

        let elevated = CodexAppServerAdapter::with_configs(
            "test-client",
            "test-version",
            AppServerConnectionConfig::default().with_test_elevated_environment(),
            AppServerExecutionPolicy {
                approval_policy: ApprovalPolicyValue::Never,
                approvals_reviewer: Some(ApprovalsReviewerValue::GuardianSubagent),
                sandbox_mode: SandboxModeValue::DangerFullAccess,
            },
        )
        .elevated_policy_warnings();

        assert_eq!(elevated.len(), 1);
        assert!(elevated[0].starts_with("elevated-risk app-server policy:"));
        for risk in [
            "approval=never",
            "sandbox=danger-full-access",
            "automatic reviewer may auto-approve",
            "process-env=all inherits parent secrets",
        ] {
            assert!(elevated[0].contains(risk));
        }

        let api_key_adapter = CodexAppServerAdapter::with_configs(
            "test-client",
            "test-version",
            AppServerConnectionConfig::default().with_test_api_key_auth(),
            AppServerExecutionPolicy::default(),
        );
        assert!(
            api_key_adapter
                .active_policy_summary()
                .contains("api-key-auth=enabled")
        );
        let api_key_warnings = api_key_adapter.elevated_policy_warnings();
        assert_eq!(api_key_warnings.len(), 1);
        assert!(api_key_warnings[0].contains("OPENAI_API_KEY/CODEX_API_KEY"));
    }

    #[test]
    fn finish_stream_result_reports_failed_event_and_returns_error() {
        let (tx, rx) = conversation_stream_channel();
        let result = finish_stream_result(
            anyhow::Result::<ConversationTurnTerminalReceipt>::Err(anyhow::anyhow!("boom")),
            &tx,
        );

        assert!(result.is_err());
        assert_eq!(
            rx.try_recv().expect("failed event should be sent"),
            ConversationStreamEvent::Failed {
                message: "boom".to_string()
            }
        );
    }

    #[test]
    fn prompt_log_never_labels_unconfirmed_upstream_completion_as_completed() {
        let receipt = ConversationTurnTerminalReceipt::completed(
            "thread-1",
            "turn-1",
            Vec::new(),
        )
        .with_application_delivery(ConversationTurnApplicationDelivery::Unconfirmed(
            crate::domain::turn_terminal::ConversationTurnApplicationDeliveryFailure::DeadlineExceeded,
        ));
        let result = Ok(receipt);

        assert_eq!(prompt_log_terminal_status(&result), "recovery_pending");
        assert!(
            prompt_log_terminal_error(&result)
                .as_deref()
                .is_some_and(|message| message.contains("application delivery was unconfirmed"))
        );
    }

    #[test]
    fn app_server_stream_boundary_bounds_large_text_and_path_payloads() {
        let completed =
            bounded_app_server_stream_event(ConversationStreamEvent::AgentMessageCompleted {
                item_id: "item-1".to_string(),
                phase: Some("final".to_string()),
                text: "a".repeat(MAX_STREAM_COMPLETED_MESSAGE_BYTES + 1),
            });
        let ConversationStreamEvent::AgentMessageCompleted { text, .. } = completed else {
            panic!("completed message should remain the same event kind");
        };
        assert_eq!(
            text.len(),
            MAX_STREAM_COMPLETED_MESSAGE_BYTES + STREAM_TRUNCATION_MARKER.len()
        );
        assert!(text.ends_with(STREAM_TRUNCATION_MARKER));

        let receipt = ConversationTurnTerminalReceipt::completed(
            "thread-1",
            "turn-1",
            (0..MAX_STREAM_CHANGED_PATHS + 1)
                .map(|index| format!("docs/plan/{index}.md"))
                .collect(),
        )
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);
        let completion =
            bounded_app_server_stream_event(ConversationStreamEvent::TurnTerminal { receipt });
        let ConversationStreamEvent::TurnTerminal { receipt } = completion else {
            panic!("turn completion should remain the same event kind");
        };
        assert_eq!(
            receipt.observations.changed_planning_file_paths.len(),
            MAX_STREAM_CHANGED_PATHS
        );

        let runtime_observation = bounded_app_server_stream_event(
            ConversationStreamEvent::RuntimeEnvelopeObserved {
                observation: Box::new(
                    crate::domain::conversation_runtime_envelope::ConversationRuntimeEnvelopeObservation::ModelRerouted {
                        thread_id: "t".repeat(super::MAX_STREAM_IDENTIFIER_BYTES + 1),
                        turn_id: "u".repeat(super::MAX_STREAM_IDENTIFIER_BYTES + 1),
                        reroute: crate::domain::conversation_runtime_envelope::ConversationRuntimeModelReroute {
                            from_model: "gpt-a".to_string(),
                            to_model: "gpt-b".to_string(),
                            reason: crate::domain::conversation_runtime_envelope::ConversationRuntimeModelRerouteReason::HighRiskCyberActivity,
                        },
                    },
                ),
            },
        );
        let ConversationStreamEvent::RuntimeEnvelopeObserved { observation } = runtime_observation
        else {
            panic!("runtime observation should remain the same event kind");
        };
        let crate::domain::conversation_runtime_envelope::ConversationRuntimeEnvelopeObservation::ModelRerouted {
            thread_id,
            turn_id,
            ..
        } = observation.as_ref()
        else {
            panic!("runtime observation should retain reroute semantics");
        };
        assert!(thread_id.ends_with(STREAM_TRUNCATION_MARKER));
        assert!(turn_id.ends_with(STREAM_TRUNCATION_MARKER));
    }

    #[test]
    fn app_server_stream_boundary_preserves_approval_control_event_exactly() {
        let event = ConversationStreamEvent::ApprovalRequested {
            request: ConversationApprovalRequest {
                approval_id: "approval-1".to_string(),
                server_request_id: "request-1".to_string(),
                method: "item/commandExecution/requestApproval".to_string(),
                kind: ConversationApprovalRequestKind::CommandExecution,
                summary: "approve command".to_string(),
                details: vec!["cargo test".to_string()],
            },
        };

        assert_eq!(bounded_app_server_stream_event(event.clone()), event);
    }

    #[test]
    fn app_server_sender_normalizes_payload_before_queue_admission() {
        let (sender, receiver) = conversation_stream_channel();
        AppServerEventSender::send(
            &sender,
            ConversationStreamEvent::StatusUpdated {
                text: "x".repeat(super::MAX_STREAM_METADATA_BYTES + 1),
            },
        )
        .expect("bounded event should be admitted");

        let ConversationStreamEvent::StatusUpdated { text } =
            receiver.recv().expect("normalized event should arrive")
        else {
            panic!("status event should remain the same event kind");
        };
        assert!(text.ends_with(STREAM_TRUNCATION_MARKER));
    }

    #[test]
    fn app_server_progressive_burst_preserves_control_approval_and_terminal_delivery() {
        const PUBLICATION_COUNT: u64 = 10_000;
        let (sender, receiver) = conversation_stream_channel();

        for sequence in 0..PUBLICATION_COUNT {
            send_required_app_server_event(
                &sender,
                progressive_plan_event(sequence),
                "progressive activity",
            )
            .expect("progressive pressure must not consume control FIFO capacity");
        }

        let status = ConversationStreamEvent::StatusUpdated {
            text: "control remains live".to_string(),
        };
        let approval = ConversationStreamEvent::ApprovalRequested {
            request: ConversationApprovalRequest {
                approval_id: "approval-1".to_string(),
                server_request_id: "request-1".to_string(),
                method: "item/commandExecution/requestApproval".to_string(),
                kind: ConversationApprovalRequestKind::CommandExecution,
                summary: "approve command".to_string(),
                details: vec!["cargo test".to_string()],
            },
        };
        let terminal = ConversationStreamEvent::TurnTerminal {
            receipt: ConversationTurnTerminalReceipt::completed(
                "thread-progressive",
                "turn-progressive",
                Vec::new(),
            )
            .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed),
        };
        for (name, event) in [
            ("status", status.clone()),
            ("approval", approval.clone()),
            ("terminal", terminal.clone()),
        ] {
            send_required_app_server_event(&sender, event, name)
                .expect("progressive pressure must not crowd out required events");
        }

        let ConversationStreamEvent::ProgressiveActivityObserved { batch } = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("pending progressive activity should arrive before later controls")
        else {
            panic!("progressive activity was reordered behind a later control event");
        };
        assert_eq!(batch.first_sequence(), Some(0));
        assert_eq!(batch.last_sequence(), Some(PUBLICATION_COUNT - 1));
        assert_eq!(batch.source_observation_count(), PUBLICATION_COUNT);
        assert_eq!(batch.superseded_publication_count(), PUBLICATION_COUNT - 1);
        for expected in [status, approval, terminal] {
            assert_eq!(
                receiver
                    .recv_timeout(Duration::from_secs(1))
                    .expect("required event should remain admitted in FIFO order"),
                expected
            );
        }
    }

    #[test]
    fn progressive_secret_canary_stays_out_of_debug_and_prompt_log_output() {
        let secret = "progressive-secret-canary-2fd9966b";
        let event = progressive_command_output_event(secret);
        let bounded = bounded_app_server_stream_event(event);

        assert!(prompt_log_output_record(&bounded).is_none());
        assert!(!format!("{bounded:?}").contains(secret));

        let (ui_sender, ui_receiver) = conversation_stream_channel();
        let (prompt_sender, capture_worker) = prompt_log_stream_forwarder(ui_sender);
        send_required_app_server_event(&prompt_sender, bounded, "progressive command output")
            .expect("progressive activity should reach the application mailbox");
        drop(prompt_sender);

        let delivered = ui_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("transient progressive activity should remain available to the reducer");
        assert!(!format!("{delivered:?}").contains(secret));
        let ConversationStreamEvent::ProgressiveActivityObserved { batch } = delivered else {
            panic!("progressive activity should retain its event kind");
        };
        let ConversationProgressiveActivityPayload::CommandOutput { tail, .. } =
            &batch.records()[0].observation().payload
        else {
            panic!("progressive activity should retain its typed command payload");
        };
        assert_eq!(
            tail, secret,
            "the canary must exercise raw transient detail"
        );

        let capture = capture_worker
            .join()
            .expect("prompt capture worker should shut down cleanly");
        assert!(capture.output_items.is_empty());
    }

    #[test]
    fn persisted_error_summary_never_contains_the_error_payload() {
        let secret = "private-app-server-error-payload";
        let summary = persisted_error_summary(&anyhow::anyhow!(secret));

        assert!(summary.starts_with("app-server error redacted"));
        assert!(!summary.contains(secret));
    }

    fn progressive_plan_event(sequence: u64) -> ConversationStreamEvent {
        let batch = ConversationProgressiveActivityBatch::single(
            ConversationProgressiveActivityObservation {
                sequence,
                thread_id: "thread-progressive".to_string(),
                turn_id: Some("turn-progressive".to_string()),
                item_id: Some("item-plan".to_string()),
                kind: ConversationProgressiveActivityKind::PlanDelta,
                payload: ConversationProgressiveActivityPayload::PlanDelta {
                    chunk_count: 1,
                    source_bytes: 1,
                },
            },
        )
        .expect("test progressive activity should satisfy domain bounds");
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(batch),
        }
    }

    fn progressive_command_output_event(secret: &str) -> ConversationStreamEvent {
        let batch = ConversationProgressiveActivityBatch::single(
            ConversationProgressiveActivityObservation {
                sequence: 0,
                thread_id: "thread-progressive".to_string(),
                turn_id: Some("turn-progressive".to_string()),
                item_id: Some("item-command".to_string()),
                kind: ConversationProgressiveActivityKind::CommandOutput,
                payload: ConversationProgressiveActivityPayload::CommandOutput {
                    tail: secret.to_string(),
                    chunk_count: 1,
                    source_bytes: secret.len() as u64,
                    newline_count: 0,
                    ends_with_newline: false,
                    truncated_bytes: 0,
                },
            },
        )
        .expect("test progressive activity should satisfy domain bounds");
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(batch),
        }
    }

    #[test]
    fn planning_worker_turn_input_attaches_queue_mutation_skill_before_prompt() {
        // The first input item must be the queue mutation skill; otherwise the hidden worker sees prompt text first.
        let adapter = CodexAppServerAdapter::new("test-client", "test-version");
        let input = adapter.planning_worker_turn_input("refresh queue");
        let serialized = serde_json::to_value(input).expect("turn input should serialize");
        let input_items = serialized
            .as_array()
            .expect("turn input should be an array");

        assert_eq!(input_items[0]["type"], "skill");
        assert_eq!(input_items[0]["name"], "akra-planning-queue-mutation");
        assert_eq!(input_items[1]["type"], "text");
        assert_eq!(input_items[1]["text"], "refresh queue");
    }

    #[test]
    fn thread_start_params_support_sub_session_metadata() {
        // app-server thread/start serialization must preserve metadata used to distinguish hidden worker sessions.
        let params = ThreadStartParams {
            cwd: Some("/repo".to_string()),
            developer_instructions: Some(
                "You are an Akra parallel task sub-session running in a leased worktree."
                    .to_string(),
            ),
            service_name: Some("akra-parallel-worker".to_string()),
            ephemeral: Some(true),
            ..ThreadStartParams::default()
        };

        let serialized = serde_json::to_value(params).expect("params should serialize");

        assert_eq!(serialized["cwd"], "/repo");
        assert_eq!(serialized["serviceName"], "akra-parallel-worker");
        assert_eq!(serialized["ephemeral"], true);
        assert!(
            serialized["developerInstructions"]
                .as_str()
                .is_some_and(|value| value.contains("leased worktree"))
        );
    }

    #[test]
    fn protected_thread_workspace_normalizes_and_marks_the_exact_cwd_untrusted() {
        let base = std::env::current_dir().expect("test cwd should resolve");
        let requested = base.join("nested").join("..").join("workspace");
        let expected = base.join("workspace").to_string_lossy().into_owned();

        let workspace = protected_thread_workspace(&requested.to_string_lossy())
            .expect("absolute workspace should be protected");

        assert_eq!(workspace.cwd, expected);
        assert_eq!(
            workspace.config["projects"][&workspace.cwd]["trust_level"],
            "untrusted"
        );
        assert_eq!(
            workspace.config["projects"]
                [codex_raw_trust_key(&base).expect("base trust key should encode")]["trust_level"],
            "untrusted"
        );
    }

    #[test]
    fn planning_thread_workspace_sets_host_provenance_without_losing_project_protection() {
        let base = std::env::current_dir().expect("test cwd should resolve");
        let requested = base.join("planning-worker-context");

        let workspace = protected_planning_thread_workspace(
            &requested.to_string_lossy(),
            Some(" parent-thread "),
            Some("parent-turn"),
        )
        .expect("planning workspace should be protected");

        assert_eq!(
            workspace.config["projects"][&workspace.cwd]["trust_level"],
            "untrusted"
        );
        assert_eq!(
            workspace.config["shell_environment_policy"]["set"][PLANNING_TOOL_PARENT_THREAD_ID_ENV],
            "parent-thread"
        );
        assert_eq!(
            workspace.config["shell_environment_policy"]["set"][PLANNING_TOOL_PARENT_TURN_ID_ENV],
            "parent-turn"
        );
    }

    #[cfg(unix)]
    #[test]
    fn protected_thread_workspace_overrides_canonical_trust_before_symlink_aliases() {
        use std::os::unix::fs::symlink;

        let fixture = unique_temp_dir("thread-project-trust-alias");
        let canonical_root = fixture.join("canonical-repo");
        let nested = canonical_root.join("nested");
        let alias = fixture.join("repo-alias");
        fs::create_dir_all(&nested).expect("canonical workspace should exist");
        symlink(&canonical_root, &alias).expect("workspace alias should be created");

        let workspace = protected_thread_workspace(&alias.join("nested").to_string_lossy())
            .expect("aliased workspace should be protected");
        let canonical_key = fs::canonicalize(&canonical_root)
            .expect("canonical root should resolve")
            .to_string_lossy()
            .into_owned();

        assert_eq!(
            workspace.config["projects"][canonical_key.as_str()]["trust_level"],
            "untrusted"
        );
        fs::remove_dir_all(fixture).expect("trust alias fixture should clean up");
    }

    #[test]
    fn protected_thread_workspace_rejects_relative_paths() {
        let error = protected_thread_workspace("relative/workspace")
            .expect_err("relative workspaces cannot be trust-pinned");

        assert!(error.to_string().contains("must be absolute"));
    }

    #[test]
    fn applied_workspace_cwd_mismatch_fails_without_reflecting_paths() {
        let requested = "/private/requested-workspace";
        let observed = "/private/provider-override";
        let mut envelope = ConversationRuntimeEnvelope::unobserved();
        envelope.applied.cwd = ConversationRuntimeObservedValue::Observed(observed.to_string());

        let error = super::exact_applied_workspace_cwd(&envelope, requested, "thread/start")
            .expect_err("applied cwd outside the protected workspace must fail closed");

        assert!(error.to_string().contains("did not match"));
        assert!(!error.to_string().contains(requested));
        assert!(!error.to_string().contains(observed));
    }

    #[cfg(windows)]
    #[test]
    fn protected_thread_workspace_preserves_case_while_lowercasing_windows_trust_keys() {
        let workspace = protected_thread_workspace(r"C:\Repo\CaseSensitive\Src")
            .expect("absolute Windows workspace should be protected");

        assert_eq!(workspace.cwd, r"C:\Repo\CaseSensitive\Src");
        assert_eq!(
            workspace.config["projects"][r"c:\repo\casesensitive\src"]["trust_level"],
            "untrusted"
        );
    }

    #[test]
    fn planning_worker_developer_instructions_keep_planning_contract() {
        // Parallel sub-session instructions are assembled in application services; this adapter owns only the planning worker contract.
        assert!(PLANNING_WORKER_DEVELOPER_INSTRUCTIONS.contains("planning-only sub-session"));
        assert!(PLANNING_WORKER_DEVELOPER_INSTRUCTIONS.contains("akra planning-tool run ."));
        assert_eq!(PLANNING_WORKER_SERVICE_NAME, "akra-planning-worker");
    }

    #[test]
    fn prompt_log_helpers_cover_skill_input_effort_labels_and_ignored_events() {
        let input_records = prompt_log_input_records(&[
            TurnInputItem::text("plain prompt"),
            TurnInputItem::skill("queue-skill", "/tmp/SKILL.md"),
        ]);
        assert_eq!(input_records[0].kind, "text");
        assert_eq!(input_records[0].content, "plain prompt");
        assert_eq!(input_records[1].kind, "skill");
        assert_eq!(input_records[1].label, "queue-skill");
        assert_eq!(input_records[1].content, "/tmp/SKILL.md");

        assert_eq!(reasoning_effort_label(ReasoningEffortValue::None), "none");
        assert_eq!(
            reasoning_effort_label(ReasoningEffortValue::Minimal),
            "minimal"
        );
        assert_eq!(reasoning_effort_label(ReasoningEffortValue::Low), "low");
        assert_eq!(
            reasoning_effort_label(ReasoningEffortValue::Medium),
            "medium"
        );
        assert_eq!(reasoning_effort_label(ReasoningEffortValue::High), "high");
        assert_eq!(reasoning_effort_label(ReasoningEffortValue::XHigh), "xhigh");

        let mut capture = AppServerPromptOutputCapture::default();
        assert!(
            prompt_log_output_record(&ConversationStreamEvent::TurnTerminal {
                receipt: crate::application::service::conversation_runtime_event::confirmed_test_terminal_receipt(),
            })
            .is_none()
        );
        assert!(
            prompt_log_output_record(&ConversationStreamEvent::ThreadPrepared {
                thread_id: "thread-secret".to_string(),
                title: "ignored envelope".to_string(),
                cwd: "/tmp/ignored".to_string(),
                runtime_envelope: Box::default(),
            })
            .is_none()
        );
        assert!(
            prompt_log_output_record(&ConversationStreamEvent::TurnStarted {
                turn_id: "turn-secret".to_string(),
                runtime_request: Box::default(),
            })
            .is_none()
        );
        assert!(
            prompt_log_output_record(&ConversationStreamEvent::RuntimeEnvelopeObserved {
                observation: Box::new(
                    ConversationRuntimeEnvelopeObservation::ThreadStatusChanged {
                        thread_id: "thread-secret".to_string(),
                        status: ConversationRuntimeObservedValue::Missing,
                    },
                ),
            })
            .is_none()
        );
        assert!(
            prompt_log_output_record(&ConversationStreamEvent::ItemLifecycleObserved {
                observation: Box::new(
                    crate::domain::conversation_item_lifecycle::ConversationItemLifecycleObservation {
                        thread_id: "thread-secret".to_string(),
                        turn_id: "turn-secret".to_string(),
                        item_id: "item-secret".to_string(),
                        kind: crate::domain::conversation_item_lifecycle::ConversationItemKind::Reasoning,
                        phase: crate::domain::conversation_item_lifecycle::ConversationItemLifecyclePhase::Completed,
                        source: crate::domain::conversation_item_lifecycle::ConversationItemLifecycleSource::Live,
                        observed_at_ms: Some(1),
                        outcome: crate::domain::conversation_item_lifecycle::ConversationItemOutcome::NotReported,
                        summary: "reasoning summary_parts=1; content_parts=1".to_string(),
                    },
                ),
            })
            .is_none()
        );
        assert!(capture.output_items.is_empty());
        capture.record(
            prompt_log_output_record(&ConversationStreamEvent::AgentMessageCompleted {
                item_id: "agent-1".to_string(),
                phase: Some("final".to_string()),
                text: "done".to_string(),
            })
            .expect("completed agent message should be captured"),
        );
        assert_eq!(capture.output_items.len(), 1);
        assert_eq!(capture.output_items[0].text, "done");
    }

    #[test]
    fn prompt_log_capture_bounds_large_utf8_streams_without_blocking_ui_delivery() {
        let oversized = "한".repeat(super::APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS + 4_096);
        let input = (0..(super::APP_SERVER_PROMPT_LOG_MAX_ITEMS_PER_DIRECTION + 8))
            .map(|_| TurnInputItem::text(oversized.clone()))
            .collect::<Vec<_>>();
        let input_records = prompt_log_input_records(&input);
        assert_eq!(
            input_records.len(),
            super::APP_SERVER_PROMPT_LOG_MAX_ITEMS_PER_DIRECTION
        );
        assert!(input_records.iter().all(|record| {
            record.content.chars().count() == super::APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS
        }));

        let (ui_sender, ui_receiver) = conversation_stream_channel();
        let ui_worker = std::thread::spawn(move || ui_receiver.iter().count());
        let (capture_sender, capture_worker) = prompt_log_stream_forwarder(ui_sender);
        let event_count = super::APP_SERVER_PROMPT_LOG_MAX_ITEMS_PER_DIRECTION * 16;
        let mut admitted_count = 0;
        for index in 0..event_count {
            if capture_sender
                .send(ConversationStreamEvent::AgentMessageCompleted {
                    item_id: format!("agent-{index}"),
                    phase: Some("final".to_string()),
                    text: oversized.clone(),
                })
                .is_ok()
            {
                admitted_count += 1;
            }
        }
        drop(capture_sender);

        let capture = capture_worker
            .join()
            .expect("bounded prompt capture worker should complete");
        assert_eq!(
            ui_worker
                .join()
                .expect("UI stream collector should observe channel closure"),
            admitted_count
        );
        assert!(admitted_count > 0);
        assert!(capture.output_items.len() <= super::APP_SERVER_PROMPT_LOG_MAX_ITEMS_PER_DIRECTION);
        assert!(!capture.output_items.is_empty());
        assert!(capture.output_items.iter().all(|record| {
            record.text.chars().count() == super::APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS
        }));
    }

    fn test_adapter() -> CodexAppServerAdapter {
        CodexAppServerAdapter::with_configs(
            "test-client",
            "test-version",
            AppServerConnectionConfig::default(),
            AppServerExecutionPolicy::default(),
        )
    }

    #[cfg(unix)]
    fn test_adapter_with_fake(fake_codex: &FakeCodex) -> CodexAppServerAdapter {
        CodexAppServerAdapter::with_configs(
            "test-client",
            "test-version",
            fake_codex.connection_config(),
            AppServerExecutionPolicy::default(),
        )
    }

    #[cfg(unix)]
    #[derive(Default)]
    struct RecordingPromptLogPort {
        records: Mutex<Vec<AppServerPromptInteractionRecord>>,
    }

    #[cfg(unix)]
    impl RecordingPromptLogPort {
        fn records(&self) -> Vec<AppServerPromptInteractionRecord> {
            self.records
                .lock()
                .expect("prompt log records lock should succeed")
                .clone()
        }
    }

    #[cfg(unix)]
    struct FailingPromptLogPort;
    #[cfg(unix)]
    struct DisabledPromptLogPort;

    #[cfg(unix)]
    impl AppServerPromptLogPort for DisabledPromptLogPort {
        fn is_enabled(&self) -> bool {
            false
        }

        fn append_app_server_prompt_interaction(
            &self,
            _workspace_dir: &str,
            _record: AppServerPromptInteractionRecord,
        ) -> Result<()> {
            panic!("disabled prompt log must not receive records")
        }

        fn load_recent_app_server_prompt_interactions(
            &self,
            _workspace_dir: &str,
            _limit: usize,
        ) -> Result<AppServerPromptInteractionSnapshot> {
            Ok(AppServerPromptInteractionSnapshot::empty())
        }
    }

    #[cfg(unix)]
    impl AppServerPromptLogPort for FailingPromptLogPort {
        fn is_enabled(&self) -> bool {
            true
        }

        fn append_app_server_prompt_interaction(
            &self,
            _workspace_dir: &str,
            _record: AppServerPromptInteractionRecord,
        ) -> Result<()> {
            anyhow::bail!("prompt log append failed")
        }

        fn load_recent_app_server_prompt_interactions(
            &self,
            _workspace_dir: &str,
            _limit: usize,
        ) -> Result<AppServerPromptInteractionSnapshot> {
            Ok(AppServerPromptInteractionSnapshot {
                records: Vec::new(),
            })
        }
    }

    #[cfg(unix)]
    impl AppServerPromptLogPort for RecordingPromptLogPort {
        fn is_enabled(&self) -> bool {
            true
        }

        fn append_app_server_prompt_interaction(
            &self,
            _workspace_dir: &str,
            record: AppServerPromptInteractionRecord,
        ) -> Result<()> {
            self.records
                .lock()
                .expect("prompt log records lock should succeed")
                .push(record);
            Ok(())
        }

        fn load_recent_app_server_prompt_interactions(
            &self,
            _workspace_dir: &str,
            _limit: usize,
        ) -> Result<AppServerPromptInteractionSnapshot> {
            Ok(AppServerPromptInteractionSnapshot {
                records: self.records(),
            })
        }
    }

    #[cfg(unix)]
    fn has_launch_attachment(events: &[ConversationStreamEvent]) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                ConversationStreamEvent::AttachmentObserved { profile }
                    if *profile == crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile::codex_app_server_launch()
            )
        })
    }

    #[cfg(unix)]
    fn has_reattach_attachment(events: &[ConversationStreamEvent]) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                ConversationStreamEvent::AttachmentObserved { profile }
                    if *profile == crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile::codex_app_server_reattach()
            )
        })
    }

    #[cfg(unix)]
    fn has_thread_prepared(events: &[ConversationStreamEvent], thread_id: &str) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                ConversationStreamEvent::ThreadPrepared { thread_id: observed, .. }
                    if observed == thread_id
            )
        })
    }

    #[cfg(unix)]
    fn prepared_runtime_envelope<'a>(
        events: &'a [ConversationStreamEvent],
        thread_id: &str,
    ) -> &'a ConversationRuntimeEnvelope {
        events
            .iter()
            .find_map(|event| match event {
                ConversationStreamEvent::ThreadPrepared {
                    thread_id: observed,
                    runtime_envelope,
                    ..
                } if observed == thread_id => Some(runtime_envelope.as_ref()),
                _ => None,
            })
            .expect("stream should contain the requested thread runtime envelope")
    }

    #[cfg(unix)]
    fn started_runtime_request(
        events: &[ConversationStreamEvent],
    ) -> &ConversationRuntimeConfigurationRequest {
        events
            .iter()
            .find_map(|event| match event {
                ConversationStreamEvent::TurnStarted {
                    runtime_request, ..
                } => Some(runtime_request.as_ref()),
                _ => None,
            })
            .expect("stream should contain a turn runtime request")
    }

    #[cfg(unix)]
    fn has_turn_completed(events: &[ConversationStreamEvent]) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                ConversationStreamEvent::TurnTerminal { receipt }
                    if receipt.is_completed_and_confirmed()
            )
        })
    }

    #[cfg(unix)]
    struct FakeCodex {
        temp_dir: PathBuf,
        codex_path: PathBuf,
        log_path: PathBuf,
        scenario: String,
        marker_path: PathBuf,
        process_path: Option<std::ffi::OsString>,
    }

    #[cfg(unix)]
    impl FakeCodex {
        fn install(name: &str) -> Self {
            Self::install_with_scenario(name, "")
        }

        fn install_with_scenario(name: &str, scenario: &str) -> Self {
            let temp_dir = unique_temp_dir(name);
            let codex_path = temp_dir.join("codex");
            let log_path = temp_dir.join("requests.jsonl");
            let marker_path = temp_dir.join("scenario-marker");
            fs::write(&codex_path, fake_codex_script()).expect("fake codex script should write");
            make_executable(&codex_path);
            // Some overlay filesystems briefly report ETXTBSY when a freshly
            // written executable is spawned from another test thread.
            std::thread::sleep(std::time::Duration::from_millis(2));

            Self {
                temp_dir,
                codex_path,
                log_path,
                scenario: scenario.to_string(),
                marker_path,
                process_path: std::env::var_os("PATH"),
            }
        }

        fn connection_config(&self) -> AppServerConnectionConfig {
            let mut environment = vec![
                (
                    "AKRA_FAKE_APP_SERVER_LOG".into(),
                    self.log_path.as_os_str().to_owned(),
                ),
                (
                    "AKRA_FAKE_APP_SERVER_SCENARIO".into(),
                    self.scenario.as_str().into(),
                ),
                (
                    "AKRA_FAKE_APP_SERVER_MARKER".into(),
                    self.marker_path.as_os_str().to_owned(),
                ),
            ];
            if let Some(path) = &self.process_path {
                environment.push(("PATH".into(), path.clone()));
            }
            AppServerConnectionConfig::default()
                .with_test_process(self.codex_path.clone(), environment)
        }

        fn logged_requests(&self) -> Vec<Value> {
            fs::read_to_string(&self.log_path)
                .unwrap_or_default()
                .lines()
                .map(|line| serde_json::from_str(line).expect("logged request should be JSON"))
                .collect()
        }

        fn logged_methods(&self) -> Vec<String> {
            self.logged_requests()
                .into_iter()
                .map(|request| {
                    request["method"]
                        .as_str()
                        .expect("logged request should include method")
                        .to_string()
                })
                .collect()
        }
    }

    #[cfg(unix)]
    impl Drop for FakeCodex {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.temp_dir);
        }
    }

    #[cfg(unix)]
    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "app-server-mod-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temp dir should be created");
        path
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)
            .expect("fake codex metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("fake codex should be executable");
    }

    #[cfg(unix)]
    fn fake_codex_script() -> &'static str {
        r#"#!/usr/bin/env python3
import json
import os
import sys

log_path = os.environ.get("AKRA_FAKE_APP_SERVER_LOG")
scenario = os.environ.get("AKRA_FAKE_APP_SERVER_SCENARIO", "")
marker_path = os.environ.get("AKRA_FAKE_APP_SERVER_MARKER")

def log_request(request):
    if not log_path:
        return
    with open(log_path, "a", encoding="utf-8") as handle:
        handle.write(json.dumps(request, sort_keys=True) + "\n")

def send(value):
    sys.stdout.write(json.dumps(value) + "\n")
    sys.stdout.flush()

def send_error(request_id, message):
    send({
        "id": request_id,
        "error": {
            "message": message,
        },
    })

def should_fail_once(expected):
    if scenario != expected:
        return False
    if not marker_path:
        return True
    if os.path.exists(marker_path):
        return False
    with open(marker_path, "w", encoding="utf-8") as handle:
        handle.write(expected)
    return True

def thread_record(thread_id, params=None):
    params = params or {}
    cwd = params.get("cwd") or "/repo"
    name = "Fake " + thread_id
    return {
        "id": thread_id,
        "name": name,
        "preview": "Preview for " + thread_id,
        "cwd": cwd,
        "source": "vscode",
        "modelProvider": "openai",
        "updatedAt": 1770000000,
        "path": "/tmp/" + thread_id + ".jsonl",
        "status": {"type": "idle"},
        "gitInfo": {"branch": "main"},
        "turns": [],
    }

def runtime_envelope(params, default_model):
    sandbox_type = {
        "read-only": "readOnly",
        "workspace-write": "workspaceWrite",
        "danger-full-access": "dangerFullAccess",
    }.get(params.get("sandbox"), "readOnly")
    return {
        "approvalPolicy": params.get("approvalPolicy") or "on-request",
        "approvalsReviewer": params.get("approvalsReviewer") or "user",
        "cwd": params.get("cwd") or "/repo",
        "model": params.get("model") or default_model,
        "modelProvider": "openai",
        "reasoningEffort": "medium",
        "sandbox": {"type": sandbox_type},
        "serviceTier": None,
    }

def send_settings(thread_id, model, cwd):
    send({
        "method": "thread/settings/updated",
        "params": {
            "threadId": thread_id,
            "threadSettings": {
                "model": model,
                "modelProvider": "openai",
                "effort": "medium",
                "serviceTier": None,
                "cwd": cwd,
                "approvalPolicy": "on-request",
                "approvalsReviewer": "user",
                "sandboxPolicy": {"type": "readOnly"},
                "activePermissionProfile": None,
                "collaborationMode": {},
            },
        },
    })

for line in sys.stdin:
    request = json.loads(line)
    log_request(request)
    method = request.get("method")
    if "id" not in request:
        continue

    request_id = request["id"]
    params = request.get("params") or {}

    if method == "initialize":
        send({
            "id": request_id,
            "result": {
                "userAgent": "codex-app-server/fake",
                "platformFamily": "unix",
                "platformOs": "linux-x64",
            },
        })
    elif method == "account/read":
        if scenario == "fail_account_always":
            send_error(request_id, "forced account/read failure")
            continue
        if should_fail_once("fail_account_once"):
            send_error(request_id, "forced one-time account/read failure")
            continue
        send({
            "id": request_id,
            "result": {
                "account": {
                    "type": "chatgpt",
                    "email": "operator@example.com",
                    "planType": "plus",
                },
                "requiresOpenAIAuth": False,
            },
        })
    elif method == "thread/list":
        if scenario == "fail_thread_list_always":
            send_error(request_id, "forced thread/list failure")
            continue
        send({
            "id": request_id,
            "result": {
                "data": [thread_record("listed-thread")],
                "nextCursor": "cursor-next",
            },
        })
    elif method == "thread/name/set":
        send({"id": request_id, "result": {}})
    elif method == "thread/read":
        send({
            "id": request_id,
            "result": {
                "thread": thread_record(params.get("threadId", "read-thread")),
            },
        })
    elif method == "thread/start":
        if scenario == "pre_thread_response_settings":
            send_settings("started-thread", "stale-before-response", params.get("cwd") or "/repo")
        send({
            "id": request_id,
            "result": {
                "thread": thread_record("started-thread", params),
                **runtime_envelope(params, "gpt-applied"),
            },
        })
    elif method == "thread/resume":
        if scenario == "pre_thread_response_settings":
            send_settings(params.get("threadId", "resumed-thread"), "stale-before-response", params.get("cwd") or "/repo")
        send({
            "id": request_id,
            "result": {
                "thread": thread_record(params.get("threadId", "resumed-thread")),
                **runtime_envelope(params, "gpt-resumed"),
            },
        })
    elif method == "turn/start":
        if scenario == "fail_turn_start":
            send_error(request_id, "forced turn/start failure")
            continue
        thread_id = params.get("threadId", "started-thread")
        turn_id = "turn-" + str(request_id)
        if scenario == "early_runtime_envelope":
            requested_model = params.get("model") or "gpt-applied"
            send({
                "method": "thread/settings/updated",
                "params": {
                    "threadId": thread_id,
                    "threadSettings": {
                        "model": requested_model,
                        "modelProvider": "openai",
                        "effort": params.get("effort"),
                        "serviceTier": None,
                        "cwd": "/repo",
                        "approvalPolicy": params.get("approvalPolicy") or "on-request",
                        "approvalsReviewer": params.get("approvalsReviewer") or "user",
                        "sandboxPolicy": params.get("sandboxPolicy") or {"type": "readOnly"},
                        "activePermissionProfile": None,
                        "collaborationMode": {},
                    },
                },
            })
            send({
                "method": "model/rerouted",
                "params": {
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "fromModel": requested_model,
                    "toModel": "gpt-rerouted",
                    "reason": "highRiskCyberActivity",
                },
            })
            send({
                "method": "thread/status/changed",
                "params": {
                    "threadId": thread_id,
                    "status": {
                        "type": "active",
                        "activeFlags": ["waitingOnApproval"],
                    },
                },
            })
        send({
            "id": request_id,
            "result": {
                "turn": {
                    "id": turn_id,
                },
            },
        })
        if scenario != "early_runtime_envelope":
            send({
                "method": "item/started",
                "params": {
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "startedAtMs": 0,
                    "item": {
                        "type": "agentMessage",
                        "id": "agent-1",
                        "phase": "commentary",
                        "text": "",
                    },
                },
            })
            send({
                "method": "item/agentMessage/delta",
                "params": {
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "itemId": "agent-1",
                    "delta": "fake delta",
                },
            })
            send({
                "method": "item/completed",
                "params": {
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "completedAtMs": 1,
                    "item": {
                        "type": "agentMessage",
                        "id": "agent-1",
                        "phase": "final",
                        "text": "fake final response",
                    },
                },
            })
        if scenario == "terminal_retry_completed":
            send({
                "method": "error",
                "params": {
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "willRetry": True,
                    "error": {
                        "message": "temporary fake retry",
                        "codexErrorInfo": "serverOverloaded",
                    },
                },
            })
        terminal_status = {
            "terminal_interrupted": "interrupted",
            "terminal_failed": "failed",
            "terminal_unknown": "inProgress",
        }.get(scenario, "completed")
        terminal_turn = {
            "id": turn_id,
            "items": [],
            "status": terminal_status,
        }
        if terminal_status == "failed":
            terminal_turn["error"] = {
                "message": "forced terminal failure",
                "codexErrorInfo": "serverOverloaded",
            }
        send({
            "method": "turn/completed",
            "params": {
                "threadId": thread_id,
                "turn": terminal_turn,
            },
        })
    elif method == "thread/archive":
        if scenario == "fail_thread_archive":
            send_error(request_id, "forced thread/archive failure")
            continue
        send({
            "id": request_id,
            "result": {},
        })
    elif method == "turn/interrupt":
        send({
            "id": request_id,
            "result": {},
        })
    else:
        send({
            "id": request_id,
            "error": {
                "message": "unexpected method " + str(method),
            },
        })
"#
    }
}

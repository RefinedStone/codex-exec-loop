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
pub(crate) mod connection;
mod execution_policy;
mod planning_worker;
mod planning_worker_skill;
pub(crate) mod protocol;
pub(crate) mod runtime;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, TryLockError};
use std::thread;

use anyhow::{Result, anyhow};
use chrono::Utc;

use self::connection::{
    AppServerConnection, AppServerConnectionConfig, AppServerTurnInterruptSignal,
};
use self::execution_policy::AppServerExecutionPolicy;
pub use self::planning_worker::AppServerPlanningWorkerAdapter;
pub(crate) use self::planning_worker::PlanningThreadLauncher;
use self::planning_worker_skill::PlanningWorkerSkillAdapter;
use self::protocol::{
    ReasoningEffortValue, ThreadListParams, ThreadResumeParams, ThreadStartParams, TurnInputItem,
    TurnStartParams, initialize_detail, sort_and_dedup_warnings, thread_title,
    to_conversation_snapshot, to_session_summary,
};
use self::runtime::{
    RequestFailureOutcome, RequestRuntimeMode, SharedAppServerRuntime, SharedRuntimeOutput,
    SharedRuntimeRequestKind, request_failure_outcome,
};
use crate::application::port::outbound::app_server_prompt_log_port::{
    AppServerPromptInputRecord, AppServerPromptInteractionRecord, AppServerPromptLogPort,
    AppServerPromptOutputRecord, NoopAppServerPromptLogPort,
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
    ConversationStreamEvent, emit_codex_app_server_launch_attachment,
    emit_codex_app_server_reattach_attachment,
};
use crate::diagnostics::event_log;
use crate::domain::conversation::{
    ConversationRuntimeControlTruth, ConversationSnapshot, ConversationTurnOptions,
};
use crate::domain::recent_sessions::{
    RecentSessions, SessionCatalog, SessionCatalogRequest, SessionCatalogTier,
};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
use serde_json::json;

const PLANNING_WORKER_MODEL: &str = "gpt-5.4";
const PLANNING_WORKER_SERVICE_NAME: &str = "akra-planning-worker";
const PLANNING_WORKER_DEVELOPER_INSTRUCTIONS: &str = r#"You are an Akra planning-only sub-session.
Evaluate accepted DB direction authority, accepted DB task authority, and DB queue projection only.
Do not edit planning files, source files, SQL, or JSON authority directly.
Use the attached queue-mutation skill and `akra planning-tool run .` before falling back to final planning_task_commands."#;
static NEXT_PROMPT_LOG_INTERACTION_ID: AtomicU64 = AtomicU64::new(1);

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
        )
    }

    #[tracing::instrument(level = "trace", skip(self, event_sender))]
    fn run_new_thread_stream_request(
        &self,
        cwd: &str,
        prompt: &str,
        options: ConversationTurnOptions,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        /*
         * New conversation streaming creates a thread, emits ThreadPrepared for immediate TUI state, then starts a turn.
         * ThreadPrepared arrives before assistant tokens so the UI can display thread id/title/cwd and persist reattach state.
         */
        let result = self.with_streaming_runtime(|connection| {
            let model = options.model.as_deref();
            let effort = options.reasoning_effort.map(ReasoningEffortValue::from);
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(cwd.to_string()),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(self.execution_policy.sandbox_mode),
                ..ThreadStartParams::default()
            })?;
            let thread_id = thread_response.thread.id.clone();
            emit_codex_app_server_launch_attachment(&event_sender);
            let _ = event_sender.send(ConversationStreamEvent::ThreadPrepared {
                thread_id: thread_id.clone(),
                title: thread_title(&thread_response.thread),
                cwd: thread_response.thread.cwd.clone(),
            });

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

    #[tracing::instrument(level = "trace", skip(self, event_sender))]
    fn run_hidden_planning_thread_stream(
        &self,
        workspace_directory: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
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
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(workspace_directory.to_string()),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(self.execution_policy.sandbox_mode),
                model: Some(PLANNING_WORKER_MODEL.to_string()),
                developer_instructions: Some(PLANNING_WORKER_DEVELOPER_INSTRUCTIONS.to_string()),
                service_name: Some(PLANNING_WORKER_SERVICE_NAME.to_string()),
                ephemeral: Some(true),
            })?;
            let thread_id = thread_response.thread.id.clone();
            emit_codex_app_server_launch_attachment(&event_sender);
            let _ = event_sender.send(ConversationStreamEvent::ThreadPrepared {
                thread_id: thread_id.clone(),
                title: thread_title(&thread_response.thread),
                cwd: thread_response.thread.cwd.clone(),
            });

            self.start_turn_and_wait_for_stream(
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
            )
        });
        match &result {
            Ok(()) => event_log::emit_lazy("hidden_planning_thread_completed", || {
                json!({
                    "workspace_directory": workspace_directory,
                    "operation": "planning_worker_thread",
                    "phase": "completed",
                    "decision": "stream_completed",
                    "service_name": PLANNING_WORKER_SERVICE_NAME,
                })
            }),
            Err(error) => event_log::emit_lazy("hidden_planning_thread_failed", || {
                json!({
                    "workspace_directory": workspace_directory,
                    "operation": "planning_worker_thread",
                    "phase": "failed",
                    "decision": "return_error",
                    "service_name": PLANNING_WORKER_SERVICE_NAME,
                    "error": error.to_string(),
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
        let mut connection = self.open_connection()?;
        let initialize_response = connection.initialize()?;
        let initialize_detail = initialize_detail(&initialize_response);
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
    fn with_isolated_streaming_runtime<F>(&self, mut operation: F) -> Result<()>
    where
        F: FnMut(&mut AppServerConnection) -> Result<()>,
    {
        /*
         * Worker streams use their own child process so planning/parallel sub-session
         * notifications cannot interleave with the user's active conversation stream.
         * Any warnings are drained locally because hidden workers report meaningful
         * output through the stream events and worker result reduction.
         */
        let mut connection = self.open_connection()?;
        connection.initialize()?;
        let result = operation(&mut connection);
        let _ = connection.take_warnings();
        result
    }

    #[tracing::instrument(level = "trace", skip(self, operation))]
    fn with_streaming_runtime<F>(&self, mut operation: F) -> Result<()>
    where
        F: FnMut(&mut AppServerConnection) -> Result<()>,
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
            Ok(()) => Ok(()),
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

    #[tracing::instrument(level = "trace", skip(self, connection, event_sender))]
    fn start_turn_and_wait_for_stream(
        &self,
        connection: &mut AppServerConnection,
        input: Vec<TurnInputItem>,
        model: Option<&str>,
        effort: Option<ReasoningEffortValue>,
        event_sender: &Sender<ConversationStreamEvent>,
        prompt_trace_context: AppServerPromptTraceContext,
    ) -> Result<()> {
        /*
         * The interrupt generation is sampled before turn/start so a stale stop from a
         * previous turn cannot cancel the new one. wait_for_turn_stream compares
         * against this snapshot and translates only later generations into
         * turn/interrupt.
         */
        let observed_interrupt_generation = self.turn_interrupt_signal.current_generation();
        let started_at = Utc::now().to_rfc3339();
        let input_records = prompt_log_input_records(&input);
        let trace_thread_id = prompt_trace_context.thread_id.clone();
        let turn_response = match connection.start_turn(TurnStartParams {
            thread_id: trace_thread_id.clone(),
            input,
            approval_policy: Some(self.execution_policy.approval_policy),
            approvals_reviewer: self.execution_policy.approvals_reviewer,
            sandbox_policy: Some(self.execution_policy.sandbox_mode.as_turn_sandbox_policy()),
            model: model.map(str::to_string),
            effort,
        }) {
            Ok(response) => response,
            Err(error) => {
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
                    input_items: input_records,
                    output_items: Vec::new(),
                    error_message: Some(error.to_string()),
                    started_at,
                    completed_at: Utc::now().to_rfc3339(),
                });
                return Err(error);
            }
        };

        let _ = event_sender.send(ConversationStreamEvent::TurnStarted {
            turn_id: turn_response.turn.id.clone(),
        });

        let output_capture = Arc::new(Mutex::new(AppServerPromptOutputCapture::default()));
        let (stream_event_sender, forwarder) =
            prompt_log_stream_forwarder(event_sender.clone(), output_capture.clone());
        let stream_result = connection.wait_for_turn_stream(
            &trace_thread_id,
            &turn_response.turn.id,
            &self.turn_interrupt_signal,
            observed_interrupt_generation,
            &stream_event_sender,
        );
        drop(stream_event_sender);
        if forwarder.join().is_err() {
            tracing::warn!("app-server prompt log stream forwarder panicked");
        }
        let output_items = output_capture
            .lock()
            .map(|capture| capture.output_items.clone())
            .unwrap_or_default();
        self.record_prompt_interaction(AppServerPromptInteractionRecord {
            sequence: 0,
            interaction_id: next_prompt_log_interaction_id(),
            session_kind: prompt_trace_context.session_kind,
            operation: prompt_trace_context.operation,
            status: if stream_result.is_ok() {
                "completed".to_string()
            } else {
                "failed".to_string()
            },
            workspace_dir: prompt_trace_context.workspace_dir,
            thread_id: Some(trace_thread_id),
            turn_id: Some(turn_response.turn.id.clone()),
            service_name: prompt_trace_context.service_name,
            model: model.map(str::to_string),
            reasoning_effort: effort.map(reasoning_effort_label).map(str::to_string),
            developer_instructions: prompt_trace_context.developer_instructions,
            input_items: input_records,
            output_items,
            error_message: stream_result.as_ref().err().map(ToString::to_string),
            started_at,
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
        let workspace_dir = record.workspace_dir.clone();
        if let Err(error) = self
            .prompt_log_port
            .append_app_server_prompt_interaction(&workspace_dir, record)
        {
            tracing::warn!(%workspace_dir, %error, "failed to record app-server prompt interaction");
        }
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

        Ok(AppServerStartupContext {
            attachment_profile: output.attachment_profile,
            initialize_detail,
            account_detail: account_response.to_summary_text(),
            account_ok: account_response.is_authenticated(),
            warnings: output.warnings,
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
        let output =
            self.with_shared_runtime(SharedRuntimeRequestKind::RecentSessions, |connection, _| {
                connection.list_threads(ThreadListParams {
                    limit: Some(request.limit),
                    ..ThreadListParams::default()
                })
            })?;
        let items = output
            .value
            .data
            .into_iter()
            .map(to_session_summary)
            .collect::<Vec<_>>();

        Ok(SessionCatalog::ready(
            SessionCatalogTier::ProviderBackedCatalog,
            RecentSessions {
                items,
                warnings: output.warnings,
                next_cursor: output.value.next_cursor,
            },
        ))
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

    fn run_new_thread_stream(
        &self,
        cwd: &str,
        prompt: &str,
        options: ConversationTurnOptions,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.run_new_thread_stream_request(cwd, prompt, options, event_sender)
    }

    #[tracing::instrument(level = "trace", skip(self, event_sender))]
    fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        options: ConversationTurnOptions,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        /*
         * Existing-thread streaming reattaches before turn/start so app-server restores
         * thread context and execution policy on the server side. The reattach
         * attachment event tells the terminal bridge that this stream is bound to an
         * existing app-server session, not a freshly created thread.
         */
        let result = self.with_streaming_runtime(|connection| {
            let model = options.model.as_deref();
            let effort = options.reasoning_effort.map(ReasoningEffortValue::from);
            let resume_response = connection.resume_thread(ThreadResumeParams {
                thread_id: thread_id.to_string(),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(self.execution_policy.sandbox_mode),
            })?;
            emit_codex_app_server_reattach_attachment(&event_sender);
            self.start_turn_and_wait_for_stream(
                connection,
                vec![TurnInputItem::text(prompt)],
                model,
                effort,
                &event_sender,
                AppServerPromptTraceContext {
                    workspace_dir: resume_response.thread.cwd,
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
    #[tracing::instrument(level = "trace", skip(self, event_sender))]
    fn run_hidden_planning_thread(
        &self,
        workspace_directory: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        // PlanningWorkerPort depends on this narrow launcher trait so tests can fake the stream source.
        self.run_hidden_planning_thread_stream(workspace_directory, prompt, event_sender)
    }
}

impl ParallelAgentWorkerPort for CodexAppServerAdapter {
    #[tracing::instrument(level = "trace", skip(self, event_sender))]
    fn run_isolated_new_thread_stream(
        &self,
        request: ParallelAgentWorkerStreamRequest<'_>,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        // Parallel worker sessions use isolated processes but persist app-server threads so `:peek` can read them later.
        let result = self.with_isolated_streaming_runtime(|connection| {
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(request.cwd.to_string()),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(self.execution_policy.sandbox_mode),
                model: None,
                developer_instructions: Some(request.developer_instructions.to_string()),
                service_name: Some(request.service_name.to_string()),
                ephemeral: Some(false),
            })?;
            let thread_id = thread_response.thread.id.clone();
            emit_codex_app_server_launch_attachment(&event_sender);
            let _ = event_sender.send(ConversationStreamEvent::ThreadPrepared {
                thread_id: thread_id.clone(),
                title: thread_title(&thread_response.thread),
                cwd: thread_response.thread.cwd.clone(),
            });

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
            if stream_result.is_ok()
                && let Err(error) = connection.archive_thread(&thread_id)
            {
                tracing::warn!(
                    %thread_id,
                    %error,
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
    fn record_event(&mut self, event: &ConversationStreamEvent) {
        if let ConversationStreamEvent::AgentMessageCompleted {
            item_id,
            phase,
            text,
        } = event
        {
            self.output_items.push(AppServerPromptOutputRecord::new(
                item_id.clone(),
                phase.clone(),
                text.clone(),
            ));
        }
    }
}

fn prompt_log_stream_forwarder(
    event_sender: Sender<ConversationStreamEvent>,
    output_capture: Arc<Mutex<AppServerPromptOutputCapture>>,
) -> (Sender<ConversationStreamEvent>, thread::JoinHandle<()>) {
    let (forward_tx, forward_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        for event in forward_rx {
            if let Ok(mut capture) = output_capture.lock() {
                capture.record_event(&event);
            }
            let _ = event_sender.send(event);
        }
    });
    (forward_tx, handle)
}

fn prompt_log_input_records(input: &[TurnInputItem]) -> Vec<AppServerPromptInputRecord> {
    input
        .iter()
        .map(|item| match item {
            TurnInputItem::Text { text } => {
                AppServerPromptInputRecord::new("text", "turn input", text.clone())
            }
            TurnInputItem::Skill { name, path } => {
                AppServerPromptInputRecord::new("skill", name.clone(), path.clone())
            }
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

fn finish_stream_result(
    result: Result<()>,
    event_sender: &Sender<ConversationStreamEvent>,
) -> Result<()> {
    /*
     * Stream callers need both an Err return and a Failed event. The Err drives
     * service-level error handling, while the event lets TUI state leave streaming
     * mode even when the caller does not own the render state directly.
     */
    if let Err(error) = &result {
        let _ = event_sender.send(ConversationStreamEvent::Failed {
            message: error.to_string(),
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::sync::{Mutex, MutexGuard};
    use std::time::{SystemTime, UNIX_EPOCH};

    use anyhow::Result;
    use serde_json::Value;

    use super::connection::AppServerConnectionConfig;
    use super::execution_policy::AppServerExecutionPolicy;
    use super::protocol::{ReasoningEffortValue, ThreadStartParams, TurnInputItem};
    use super::{
        AppServerPromptOutputCapture, CodexAppServerAdapter,
        PLANNING_WORKER_DEVELOPER_INSTRUCTIONS, PLANNING_WORKER_SERVICE_NAME,
        PlanningThreadLauncher, finish_stream_result, prompt_log_input_records,
        reasoning_effort_label,
    };
    use crate::application::port::outbound::app_server_prompt_log_port::{
        AppServerPromptInteractionRecord, AppServerPromptInteractionSnapshot,
        AppServerPromptLogPort,
    };
    use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
    use crate::application::port::outbound::parallel_agent_worker_port::{
        ParallelAgentWorkerPort, ParallelAgentWorkerStreamRequest,
    };
    use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
    use crate::application::port::outbound::startup_probe_port::StartupProbePort;
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::domain::conversation::{
        ConversationReasoningEffort, ConversationRuntimeControlTruth, ConversationTurnOptions,
    };
    use crate::domain::recent_sessions::{
        SessionCatalog, SessionCatalogRequest, SessionCatalogTier,
    };

    static FAKE_CODEX_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn startup_catalog_and_snapshot_ports_reuse_shared_app_server_runtime() {
        let fake_codex = FakeCodex::install("shared-runtime");
        let adapter = test_adapter();

        let startup = adapter
            .load_startup_context()
            .expect("startup context should come from fake app-server");
        assert_eq!(
            startup.initialize_detail,
            "linux-x64 / unix / codex-app-server/fake"
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

    #[test]
    fn shared_runtime_retries_after_first_failure_and_returns_retry_notice() {
        let _fake_codex =
            FakeCodex::install_with_scenario("shared-runtime-retry", "fail_account_once");
        let adapter = test_adapter();

        let startup = adapter
            .load_startup_context()
            .expect("startup should retry with a fresh shared runtime");

        assert!(startup.account_ok);
        assert!(startup.warnings.iter().any(|warning| {
            warning.contains("shared runtime reset after startup checks request failure")
                && warning.contains("forced one-time account/read failure")
        }));
    }

    #[test]
    fn shared_runtime_final_failure_keeps_request_kind_context() {
        let _fake_codex =
            FakeCodex::install_with_scenario("shared-runtime-final-failure", "fail_account_always");
        let adapter = test_adapter();

        let error = adapter
            .load_startup_context()
            .expect_err("startup should fail after shared retry is exhausted");

        let message = format!("{error:#}");
        assert!(message.contains("startup checks request still failed after resetting"));
        assert!(message.contains("forced account/read failure"));
    }

    #[test]
    fn short_requests_use_isolated_fallback_while_shared_runtime_is_locked() {
        let _fake_codex = FakeCodex::install("isolated-fallback-success");
        let adapter = test_adapter();
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

    #[test]
    fn isolated_fallback_final_failure_reports_busy_stream_context() {
        let _fake_codex = FakeCodex::install_with_scenario(
            "isolated-fallback-final-failure",
            "fail_thread_list_always",
        );
        let adapter = test_adapter();
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

    #[test]
    fn user_thread_streams_emit_launch_reattach_and_completion_events() {
        let fake_codex = FakeCodex::install("user-streams");
        let adapter = test_adapter();

        let (new_tx, new_rx) = mpsc::channel();
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

        let (resume_tx, resume_rx) = mpsc::channel();
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
        assert!(has_turn_completed(&resume_events));

        let methods = fake_codex.logged_methods();
        assert_eq!(
            methods,
            [
                "initialize",
                "initialized",
                "thread/start",
                "turn/start",
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

    #[test]
    fn user_thread_streams_pass_turn_option_overrides_to_app_server() {
        let fake_codex = FakeCodex::install("user-turn-options");
        let adapter = test_adapter();
        let options = ConversationTurnOptions {
            model: Some("gpt-5.4".to_string()),
            reasoning_effort: Some(ConversationReasoningEffort::High),
        };

        let (new_tx, new_rx) = mpsc::channel();
        adapter
            .run_new_thread_stream("/repo", "start with overrides", options.clone(), new_tx)
            .expect("new thread stream should complete");
        assert!(has_turn_completed(&new_rx.try_iter().collect::<Vec<_>>()));

        let (resume_tx, resume_rx) = mpsc::channel();
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
        let turn_starts = requests
            .iter()
            .filter(|request| request["method"] == "turn/start")
            .collect::<Vec<_>>();

        assert!(thread_starts[0]["params"]["model"].is_null());
        assert_eq!(turn_starts[0]["params"]["model"], "gpt-5.4");
        assert_eq!(turn_starts[0]["params"]["effort"], "high");
        assert_eq!(turn_starts[1]["params"]["model"], "gpt-5.4");
        assert_eq!(turn_starts[1]["params"]["effort"], "high");
    }

    #[test]
    fn hidden_planning_stays_ephemeral_while_parallel_threads_are_readable_for_peek() {
        let fake_codex = FakeCodex::install("isolated-workers");
        let adapter = test_adapter();

        let (planning_tx, planning_rx) = mpsc::channel();
        adapter
            .run_hidden_planning_thread("/repo", "refresh queue", planning_tx)
            .expect("hidden planning worker stream should complete");
        let planning_events = planning_rx.try_iter().collect::<Vec<_>>();
        assert!(has_thread_prepared(&planning_events, "started-thread"));
        assert!(has_turn_completed(&planning_events));

        let (parallel_tx, parallel_rx) = mpsc::channel();
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
        let thread_archives = requests
            .iter()
            .filter(|request| request["method"] == "thread/archive")
            .collect::<Vec<_>>();
        assert_eq!(thread_archives.len(), 1);
        assert_eq!(thread_archives[0]["params"]["threadId"], "started-thread");
    }

    #[test]
    fn app_server_streams_record_prompt_log_entries() {
        let fake_codex = FakeCodex::install("prompt-log");
        let prompt_log = Arc::new(RecordingPromptLogPort::default());
        let adapter = CodexAppServerAdapter::with_configs_and_prompt_log(
            "test-client",
            "test-version",
            AppServerConnectionConfig::default(),
            AppServerExecutionPolicy::default(),
            prompt_log.clone(),
        );

        let (main_tx, main_rx) = mpsc::channel();
        adapter
            .run_new_thread_stream(
                "/repo",
                "start a logged session",
                ConversationTurnOptions::default(),
                main_tx,
            )
            .expect("main stream should complete");
        assert!(has_turn_completed(&main_rx.try_iter().collect::<Vec<_>>()));

        let (worker_tx, worker_rx) = mpsc::channel();
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

    #[test]
    fn stream_start_failures_emit_failed_event_and_failed_prompt_log_record() {
        let _fake_codex =
            FakeCodex::install_with_scenario("stream-start-failure", "fail_turn_start");
        let prompt_log = Arc::new(RecordingPromptLogPort::default());
        let adapter = CodexAppServerAdapter::with_configs_and_prompt_log(
            "test-client",
            "test-version",
            AppServerConnectionConfig::default(),
            AppServerExecutionPolicy::default(),
            prompt_log.clone(),
        );

        let (tx, rx) = mpsc::channel();
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

    #[test]
    fn prompt_log_append_errors_do_not_fail_streams() {
        let _fake_codex = FakeCodex::install("prompt-log-append-failure");
        let adapter = CodexAppServerAdapter::with_configs_and_prompt_log(
            "test-client",
            "test-version",
            AppServerConnectionConfig::default(),
            AppServerExecutionPolicy::default(),
            Arc::new(FailingPromptLogPort),
        );

        let (tx, rx) = mpsc::channel();
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
    fn finish_stream_result_reports_failed_event_and_returns_error() {
        let (tx, rx) = mpsc::channel();
        let result = finish_stream_result(anyhow::Result::<()>::Err(anyhow::anyhow!("boom")), &tx);

        assert!(result.is_err());
        assert_eq!(
            rx.try_recv().expect("failed event should be sent"),
            ConversationStreamEvent::Failed {
                message: "boom".to_string()
            }
        );
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
        capture.record_event(&ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
        });
        assert!(capture.output_items.is_empty());
        capture.record_event(&ConversationStreamEvent::AgentMessageCompleted {
            item_id: "agent-1".to_string(),
            phase: Some("final".to_string()),
            text: "done".to_string(),
        });
        assert_eq!(capture.output_items.len(), 1);
        assert_eq!(capture.output_items[0].text, "done");
    }

    fn test_adapter() -> CodexAppServerAdapter {
        CodexAppServerAdapter::with_configs(
            "test-client",
            "test-version",
            AppServerConnectionConfig::default(),
            AppServerExecutionPolicy::default(),
        )
    }

    #[derive(Default)]
    struct RecordingPromptLogPort {
        records: Mutex<Vec<AppServerPromptInteractionRecord>>,
    }

    impl RecordingPromptLogPort {
        fn records(&self) -> Vec<AppServerPromptInteractionRecord> {
            self.records
                .lock()
                .expect("prompt log records lock should succeed")
                .clone()
        }
    }

    struct FailingPromptLogPort;

    impl AppServerPromptLogPort for FailingPromptLogPort {
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

    impl AppServerPromptLogPort for RecordingPromptLogPort {
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

    fn has_launch_attachment(events: &[ConversationStreamEvent]) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                ConversationStreamEvent::AttachmentObserved { profile }
                    if *profile == crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile::codex_app_server_launch()
            )
        })
    }

    fn has_reattach_attachment(events: &[ConversationStreamEvent]) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                ConversationStreamEvent::AttachmentObserved { profile }
                    if *profile == crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile::codex_app_server_reattach()
            )
        })
    }

    fn has_thread_prepared(events: &[ConversationStreamEvent], thread_id: &str) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                ConversationStreamEvent::ThreadPrepared { thread_id: observed, .. }
                    if observed == thread_id
            )
        })
    }

    fn has_turn_completed(events: &[ConversationStreamEvent]) -> bool {
        events
            .iter()
            .any(|event| matches!(event, ConversationStreamEvent::TurnCompleted { .. }))
    }

    struct FakeCodex {
        _guard: MutexGuard<'static, ()>,
        temp_dir: PathBuf,
        log_path: PathBuf,
        previous_path: Option<OsString>,
        previous_log: Option<OsString>,
        previous_scenario: Option<OsString>,
        previous_marker: Option<OsString>,
    }

    impl FakeCodex {
        fn install(name: &str) -> Self {
            Self::install_with_scenario(name, "")
        }

        fn install_with_scenario(name: &str, scenario: &str) -> Self {
            let guard = FAKE_CODEX_ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let temp_dir = unique_temp_dir(name);
            let codex_path = temp_dir.join("codex");
            let log_path = temp_dir.join("requests.jsonl");
            let marker_path = temp_dir.join("scenario-marker");
            fs::write(&codex_path, fake_codex_script()).expect("fake codex script should write");
            make_executable(&codex_path);

            let previous_path = std::env::var_os("PATH");
            let previous_log = std::env::var_os("AKRA_FAKE_APP_SERVER_LOG");
            let previous_scenario = std::env::var_os("AKRA_FAKE_APP_SERVER_SCENARIO");
            let previous_marker = std::env::var_os("AKRA_FAKE_APP_SERVER_MARKER");
            let mut paths = vec![temp_dir.clone()];
            if let Some(path) = &previous_path {
                paths.extend(std::env::split_paths(path));
            }
            let joined_path = std::env::join_paths(paths).expect("PATH should join");
            unsafe {
                std::env::set_var("PATH", joined_path);
                std::env::set_var("AKRA_FAKE_APP_SERVER_LOG", &log_path);
                std::env::set_var("AKRA_FAKE_APP_SERVER_SCENARIO", scenario);
                std::env::set_var("AKRA_FAKE_APP_SERVER_MARKER", &marker_path);
            }

            Self {
                _guard: guard,
                temp_dir,
                log_path,
                previous_path,
                previous_log,
                previous_scenario,
                previous_marker,
            }
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

    impl Drop for FakeCodex {
        fn drop(&mut self) {
            unsafe {
                if let Some(path) = &self.previous_path {
                    std::env::set_var("PATH", path);
                } else {
                    std::env::remove_var("PATH");
                }

                if let Some(log) = &self.previous_log {
                    std::env::set_var("AKRA_FAKE_APP_SERVER_LOG", log);
                } else {
                    std::env::remove_var("AKRA_FAKE_APP_SERVER_LOG");
                }

                if let Some(scenario) = &self.previous_scenario {
                    std::env::set_var("AKRA_FAKE_APP_SERVER_SCENARIO", scenario);
                } else {
                    std::env::remove_var("AKRA_FAKE_APP_SERVER_SCENARIO");
                }

                if let Some(marker) = &self.previous_marker {
                    std::env::set_var("AKRA_FAKE_APP_SERVER_MARKER", marker);
                } else {
                    std::env::remove_var("AKRA_FAKE_APP_SERVER_MARKER");
                }
            }
            let _ = fs::remove_dir_all(&self.temp_dir);
        }
    }

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
    elif method == "thread/read":
        send({
            "id": request_id,
            "result": {
                "thread": thread_record(params.get("threadId", "read-thread")),
            },
        })
    elif method == "thread/start":
        send({
            "id": request_id,
            "result": {
                "thread": thread_record("started-thread", params),
            },
        })
    elif method == "thread/resume":
        send({
            "id": request_id,
            "result": {
                "thread": thread_record(params.get("threadId", "resumed-thread")),
            },
        })
    elif method == "turn/start":
        if scenario == "fail_turn_start":
            send_error(request_id, "forced turn/start failure")
            continue
        thread_id = params.get("threadId", "started-thread")
        turn_id = "turn-" + str(request_id)
        send({
            "id": request_id,
            "result": {
                "turn": {
                    "id": turn_id,
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
                "item": {
                    "type": "agentMessage",
                    "id": "agent-1",
                    "phase": "final",
                    "text": "fake final response",
                },
            },
        })
        send({
            "method": "turn/completed",
            "params": {
                "threadId": thread_id,
                "turn": {
                    "id": turn_id,
                },
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

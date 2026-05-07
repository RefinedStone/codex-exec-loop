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

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, TryLockError};

use anyhow::{Result, anyhow};

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
use crate::domain::conversation::{ConversationRuntimeControlTruth, ConversationSnapshot};
use crate::domain::recent_sessions::{
    RecentSessions, SessionCatalog, SessionCatalogRequest, SessionCatalogTier,
};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
use serde_json::json;

const PLANNING_WORKER_MODEL: &str = "gpt-5.4";
const PLANNING_WORKER_SERVICE_NAME: &str = "akra-planning-worker";
const PLANNING_WORKER_DEVELOPER_INSTRUCTIONS: &str = r#"You are an Akra planning-only sub session.
Evaluate accepted DB direction authority, accepted DB task authority, and DB queue projection only.
Do not edit planning files, source files, SQL, or JSON authority directly.
Use the attached queue-mutation skill and `akra planning-tool run .` before falling back to final planning_task_commands."#;

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
}

impl CodexAppServerAdapter {
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Self::from_environment(client_name, client_version)
    }

    pub fn from_environment(
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Self {
        /*
         * Adapter construction snapshots env-driven timeout and execution policy once.
         * That keeps a single TUI process from changing approval/sandbox behavior in
         * the middle of shared runtime reuse or hidden worker launches.
         */
        Self::with_configs(
            client_name,
            client_version,
            AppServerConnectionConfig::from_environment(),
            AppServerExecutionPolicy::from_environment(),
        )
    }

    fn with_configs(
        client_name: impl Into<String>,
        client_version: impl Into<String>,
        connection_config: AppServerConnectionConfig,
        execution_policy: AppServerExecutionPolicy,
    ) -> Self {
        Self {
            client_name: client_name.into(),
            client_version: client_version.into(),
            connection_config,
            execution_policy,
            shared_runtime: Arc::new(Mutex::new(SharedAppServerRuntime::default())),
            turn_interrupt_signal: AppServerTurnInterruptSignal::default(),
            planning_worker_skill_adapter: PlanningWorkerSkillAdapter::new(),
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
        model: Option<&str>,
        effort: Option<ReasoningEffortValue>,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        /*
         * New conversation streaming creates a thread, emits ThreadPrepared for immediate TUI state, then starts a turn.
         * ThreadPrepared arrives before assistant tokens so the UI can display thread id/title/cwd and persist reattach state.
         */
        let result = self.with_streaming_runtime(|connection| {
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(cwd.to_string()),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(self.execution_policy.sandbox_mode),
                model: model.map(str::to_string),
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
                &thread_id,
                vec![TurnInputItem::text(prompt)],
                model,
                effort,
                &event_sender,
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
                &thread_id,
                self.planning_worker_turn_input(prompt),
                Some(PLANNING_WORKER_MODEL),
                Some(ReasoningEffortValue::Medium),
                &event_sender,
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
        thread_id: &str,
        input: Vec<TurnInputItem>,
        model: Option<&str>,
        effort: Option<ReasoningEffortValue>,
        event_sender: &Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        /*
         * The interrupt generation is sampled before turn/start so a stale stop from a
         * previous turn cannot cancel the new one. wait_for_turn_stream compares
         * against this snapshot and translates only later generations into
         * turn/interrupt.
         */
        let observed_interrupt_generation = self.turn_interrupt_signal.current_generation();
        let turn_response = connection.start_turn(TurnStartParams {
            thread_id: thread_id.to_string(),
            input,
            approval_policy: Some(self.execution_policy.approval_policy),
            approvals_reviewer: self.execution_policy.approvals_reviewer,
            sandbox_policy: Some(self.execution_policy.sandbox_mode.as_turn_sandbox_policy()),
            model: model.map(str::to_string),
            effort,
        })?;

        let _ = event_sender.send(ConversationStreamEvent::TurnStarted {
            turn_id: turn_response.turn.id.clone(),
        });

        connection.wait_for_turn_stream(
            thread_id,
            &turn_response.turn.id,
            &self.turn_interrupt_signal,
            observed_interrupt_generation,
            event_sender,
        )
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
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.run_new_thread_stream_request(cwd, prompt, None, None, event_sender)
    }

    #[tracing::instrument(level = "trace", skip(self, event_sender))]
    fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        /*
         * Existing-thread streaming reattaches before turn/start so app-server restores
         * thread context and execution policy on the server side. The reattach
         * attachment event tells the terminal bridge that this stream is bound to an
         * existing app-server session, not a freshly created thread.
         */
        let result = self.with_streaming_runtime(|connection| {
            connection.resume_thread(ThreadResumeParams {
                thread_id: thread_id.to_string(),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(self.execution_policy.sandbox_mode),
            })?;
            emit_codex_app_server_reattach_attachment(&event_sender);
            self.start_turn_and_wait_for_stream(
                connection,
                thread_id,
                vec![TurnInputItem::text(prompt)],
                None,
                None,
                &event_sender,
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
        // Parallel worker sessions are isolated and ephemeral; distributor code handles commits/PRs after the worker exits.
        let result = self.with_isolated_streaming_runtime(|connection| {
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(request.cwd.to_string()),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(self.execution_policy.sandbox_mode),
                model: None,
                developer_instructions: Some(request.developer_instructions.to_string()),
                service_name: Some(request.service_name.to_string()),
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
                &thread_id,
                vec![TurnInputItem::text(request.prompt)],
                None,
                None,
                &event_sender,
            )
        });

        finish_stream_result(result, &event_sender)
    }
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
    use super::protocol::ThreadStartParams;
    use super::{
        CodexAppServerAdapter, PLANNING_WORKER_DEVELOPER_INSTRUCTIONS, PLANNING_WORKER_SERVICE_NAME,
    };

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
                "You are an Akra parallel task sub session running in a leased worktree."
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
        assert!(PLANNING_WORKER_DEVELOPER_INSTRUCTIONS.contains("planning-only sub session"));
        assert!(PLANNING_WORKER_DEVELOPER_INSTRUCTIONS.contains("akra planning-tool run ."));
        assert_eq!(PLANNING_WORKER_SERVICE_NAME, "akra-planning-worker");
    }
}

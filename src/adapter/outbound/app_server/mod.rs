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
use crate::application::port::outbound::codex_app_server_port::{
    AppServerStartupContext, CodexAppServerPort,
};
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::service::conversation_runtime_event::{
    ConversationStreamEvent, emit_codex_app_server_launch_attachment,
    emit_codex_app_server_reattach_attachment,
};
use crate::domain::conversation::{ConversationRuntimeControlTruth, ConversationSnapshot};
use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogTier};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

const PLANNING_WORKER_MODEL: &str = "gpt-5.4";

#[derive(Clone)]
pub struct CodexAppServerAdapter {
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
        AppServerConnection::spawn(
            self.client_name.clone(),
            self.client_version.clone(),
            self.connection_config.clone(),
        )
    }

    fn run_new_thread_stream_request(
        &self,
        cwd: &str,
        prompt: &str,
        model: Option<&str>,
        effort: Option<ReasoningEffortValue>,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        let result = self.with_streaming_runtime(|connection| {
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(cwd.to_string()),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(self.execution_policy.sandbox_mode),
                model: model.map(str::to_string),
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

    fn run_hidden_planning_thread_stream(
        &self,
        workspace_directory: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        let result = self.with_isolated_streaming_runtime(|connection| {
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(workspace_directory.to_string()),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(self.execution_policy.sandbox_mode),
                model: Some(PLANNING_WORKER_MODEL.to_string()),
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

        finish_stream_result(result, &event_sender)
    }

    fn with_shared_runtime<T, F>(
        &self,
        request_kind: SharedRuntimeRequestKind,
        mut operation: F,
    ) -> Result<SharedRuntimeOutput<T>>
    where
        F: FnMut(&mut AppServerConnection, &str) -> Result<T>,
    {
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

    fn run_request_on_locked_runtime<T, F>(
        &self,
        runtime: &mut SharedAppServerRuntime,
        operation: &mut F,
    ) -> Result<SharedRuntimeOutput<T>>
    where
        F: FnMut(&mut AppServerConnection, &str) -> Result<T>,
    {
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

    fn with_isolated_streaming_runtime<F>(&self, mut operation: F) -> Result<()>
    where
        F: FnMut(&mut AppServerConnection) -> Result<()>,
    {
        let mut connection = self.open_connection()?;
        connection.initialize()?;
        let result = operation(&mut connection);
        let _ = connection.take_warnings();
        result
    }

    fn with_streaming_runtime<F>(&self, mut operation: F) -> Result<()>
    where
        F: FnMut(&mut AppServerConnection) -> Result<()>,
    {
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
                runtime.reset();
                runtime.push_notice(format!(
                    "shared runtime reset after turn stream failure; the next request will reconnect ({error})"
                ));
                Err(error)
            }
        }
    }

    fn start_turn_and_wait_for_stream(
        &self,
        connection: &mut AppServerConnection,
        thread_id: &str,
        input: Vec<TurnInputItem>,
        model: Option<&str>,
        effort: Option<ReasoningEffortValue>,
        event_sender: &Sender<ConversationStreamEvent>,
    ) -> Result<()> {
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
        vec![
            self.planning_worker_skill_adapter
                .queue_mutation_skill_input(),
            TurnInputItem::text(prompt),
        ]
    }

    fn reset_shared_runtime(&self, notice: Option<String>) {
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

impl CodexAppServerPort for CodexAppServerAdapter {
    fn load_startup_context(&self) -> Result<AppServerStartupContext> {
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

    fn load_recent_sessions(&self, limit: usize) -> Result<SessionCatalog> {
        let output =
            self.with_shared_runtime(SharedRuntimeRequestKind::RecentSessions, |connection, _| {
                connection.list_threads(ThreadListParams {
                    limit: Some(limit),
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

    fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
        ConversationRuntimeControlTruth::codex_app_server()
    }

    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
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

    fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
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
    fn run_hidden_planning_thread(
        &self,
        workspace_directory: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.run_hidden_planning_thread_stream(workspace_directory, prompt, event_sender)
    }
}

impl ParallelAgentWorkerPort for CodexAppServerAdapter {
    fn run_isolated_new_thread_stream(
        &self,
        cwd: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        let result = self.with_isolated_streaming_runtime(|connection| {
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(cwd.to_string()),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(self.execution_policy.sandbox_mode),
                model: None,
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
    if let Err(error) = &result {
        let _ = event_sender.send(ConversationStreamEvent::Failed {
            message: error.to_string(),
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::CodexAppServerAdapter;

    #[test]
    fn planning_worker_turn_input_attaches_queue_mutation_skill_before_prompt() {
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
}

mod connection;
mod protocol;
mod runtime;

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, TryLockError};

use anyhow::{Result, anyhow};

use self::connection::{AppServerConnection, AppServerConnectionConfig};
use self::protocol::{
    ThreadListParams, ThreadStartParams, initialize_detail, sort_and_dedup_warnings, thread_title,
    to_conversation_snapshot, to_session_summary,
};
use self::runtime::{
    RequestFailureOutcome, RequestRuntimeMode, SharedAppServerRuntime, SharedRuntimeOutput,
    SharedRuntimeRequestKind, request_failure_outcome,
};
use crate::application::port::outbound::codex_app_server_port::{
    AppServerStartupContext, CodexAppServerPort,
};
use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};
use crate::domain::recent_sessions::RecentSessions;

#[derive(Clone)]
pub struct CodexAppServerAdapter {
    client_name: String,
    client_version: String,
    connection_config: AppServerConnectionConfig,
    shared_runtime: Arc<Mutex<SharedAppServerRuntime>>,
}

impl CodexAppServerAdapter {
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Self::from_environment(client_name, client_version)
    }

    pub fn from_environment(
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Self {
        Self::with_connection_config(
            client_name,
            client_version,
            AppServerConnectionConfig::from_environment(),
        )
    }

    fn with_connection_config(
        client_name: impl Into<String>,
        client_version: impl Into<String>,
        connection_config: AppServerConnectionConfig,
    ) -> Self {
        Self {
            client_name: client_name.into(),
            client_version: client_version.into(),
            connection_config,
            shared_runtime: Arc::new(Mutex::new(SharedAppServerRuntime::default())),
        }
    }

    fn open_connection(&self) -> Result<AppServerConnection> {
        AppServerConnection::spawn(
            self.client_name.clone(),
            self.client_version.clone(),
            self.connection_config.clone(),
        )
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
        let (value, connection_warnings) = {
            let connection = runtime.connection_mut()?;
            let value = operation(connection, &initialize_detail)?;
            let warnings = connection.take_warnings();
            (value, warnings)
        };
        let mut warnings = runtime.take_notices();
        warnings.extend(connection_warnings);
        sort_and_dedup_warnings(&mut warnings);
        Ok(SharedRuntimeOutput { value, warnings })
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
        let value = operation(&mut connection, &initialize_detail)?;
        let mut warnings = connection.take_warnings();
        if let Some(notice) = notice {
            warnings.push(notice);
        }
        sort_and_dedup_warnings(&mut warnings);
        Ok(SharedRuntimeOutput { value, warnings })
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

    fn reset_shared_runtime(&self, notice: Option<String>) {
        if let Ok(mut runtime) = self.shared_runtime.lock() {
            runtime.reset();
            if let Some(notice) = notice {
                runtime.push_notice(notice);
            }
        }
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
            initialize_detail,
            account_detail: account_response.to_summary_text(),
            account_ok: account_response.is_authenticated(),
            warnings: output.warnings,
        })
    }

    fn load_recent_sessions(&self, limit: usize) -> Result<RecentSessions> {
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

        Ok(RecentSessions {
            items,
            warnings: output.warnings,
            next_cursor: output.value.next_cursor,
        })
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

    fn run_new_thread_stream(
        &self,
        cwd: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        let result = self.with_streaming_runtime(|connection| {
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(cwd.to_string()),
            })?;
            let thread_id = thread_response.thread.id.clone();
            let _ = event_sender.send(ConversationStreamEvent::ThreadPrepared {
                thread_id: thread_id.clone(),
                title: thread_title(&thread_response.thread),
                cwd: thread_response.thread.cwd.clone(),
            });

            let turn_response = connection.start_turn(&thread_id, prompt)?;

            let _ = event_sender.send(ConversationStreamEvent::TurnStarted {
                turn_id: turn_response.turn.id.clone(),
            });

            connection.wait_for_turn_stream(&thread_id, &turn_response.turn.id, &event_sender)
        });

        if let Err(error) = &result {
            let _ = event_sender.send(ConversationStreamEvent::Failed {
                message: error.to_string(),
            });
        }

        result
    }

    fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        let result = self.with_streaming_runtime(|connection| {
            connection.resume_thread(thread_id)?;
            let turn_response = connection.start_turn(thread_id, prompt)?;

            let _ = event_sender.send(ConversationStreamEvent::TurnStarted {
                turn_id: turn_response.turn.id.clone(),
            });

            connection.wait_for_turn_stream(thread_id, &turn_response.turn.id, &event_sender)
        });

        if let Err(error) = &result {
            let _ = event_sender.send(ConversationStreamEvent::Failed {
                message: error.to_string(),
            });
        }

        result
    }
}

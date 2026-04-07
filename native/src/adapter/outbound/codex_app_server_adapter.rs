use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, TryLockError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::application::port::outbound::codex_app_server_port::{
    AppServerStartupContext, CodexAppServerPort,
};
use crate::domain::conversation::{
    ConversationMessage, ConversationMessageKind, ConversationSnapshot, ConversationStreamEvent,
    ConversationToolActivity, ConversationToolActivityKind,
};
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;

const APP_SERVER_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct CodexAppServerAdapter {
    client_name: String,
    client_version: String,
    shared_runtime: Arc<Mutex<SharedAppServerRuntime>>,
}

impl CodexAppServerAdapter {
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Self {
            client_name: client_name.into(),
            client_version: client_version.into(),
            shared_runtime: Arc::new(Mutex::new(SharedAppServerRuntime::default())),
        }
    }

    fn open_connection(&self) -> Result<AppServerConnection> {
        let mut child = Command::new("codex")
            .arg("app-server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn `codex app-server`")?;

        let stdin = child
            .stdin
            .take()
            .context("failed to take app-server stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("failed to take app-server stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("failed to take app-server stderr")?;

        let (tx, rx) = mpsc::channel();
        spawn_pipe_reader(stdout, tx.clone(), false);
        spawn_pipe_reader(stderr, tx, true);

        Ok(AppServerConnection {
            child,
            stdin,
            rx,
            warnings: Vec::new(),
            next_request_id: 1,
            client_name: self.client_name.clone(),
            client_version: self.client_version.clone(),
            initialized: false,
        })
    }

    fn to_session_summary(thread_record: ThreadRecord) -> SessionSummary {
        SessionSummary {
            id: thread_record.id,
            name: thread_record.name,
            preview: thread_record.preview,
            cwd: thread_record.cwd,
            source: thread_record.source,
            model_provider: thread_record.model_provider,
            updated_at_epoch: thread_record.updated_at,
            status_type: thread_record.status.status_type,
            path: thread_record.path,
            git_branch: thread_record.git_info.and_then(|git_info| git_info.branch),
        }
    }

    fn to_conversation_snapshot(
        thread_record: ThreadRecord,
        warnings: Vec<String>,
    ) -> ConversationSnapshot {
        let title = Self::thread_title(&thread_record);

        let messages = thread_record
            .turns
            .into_iter()
            .flat_map(|turn| turn.items.into_iter())
            .filter_map(Self::to_conversation_message)
            .collect::<Vec<_>>();

        ConversationSnapshot {
            thread_id: thread_record.id,
            title,
            cwd: thread_record.cwd,
            messages,
            warnings,
        }
    }

    fn thread_title(thread_record: &ThreadRecord) -> String {
        thread_record
            .name
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                thread_record
                    .preview
                    .lines()
                    .next()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Untitled thread")
                    .to_string()
            })
    }

    fn to_conversation_message(item: Value) -> Option<ConversationMessage> {
        let item_type = item.get("type")?.as_str()?;
        match item_type {
            "userMessage" => {
                let text = item
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|content| Self::extract_user_input_text(content.as_slice()))
                    .filter(|value| !value.trim().is_empty())?;

                Some(ConversationMessage::new(
                    ConversationMessageKind::User,
                    text,
                    None,
                    item.get("id").and_then(Value::as_str).map(str::to_string),
                ))
            }
            "agentMessage" => Some(ConversationMessage::new(
                ConversationMessageKind::Agent,
                item.get("text").and_then(Value::as_str).unwrap_or_default(),
                item.get("phase")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                item.get("id").and_then(Value::as_str).map(str::to_string),
            )),
            "fileChange" => Some(ConversationMessage::new(
                ConversationMessageKind::Tool,
                Self::format_file_change_summary(&item),
                None,
                item.get("id").and_then(Value::as_str).map(str::to_string),
            )),
            "commandExecution" => Some(ConversationMessage::new(
                ConversationMessageKind::Tool,
                Self::format_command_execution_summary(&item),
                None,
                item.get("id").and_then(Value::as_str).map(str::to_string),
            )),
            _ => None,
        }
    }

    fn extract_user_input_text(items: &[Value]) -> String {
        items
            .iter()
            .filter_map(|content| {
                if content.get("type").and_then(Value::as_str) == Some("text") {
                    content.get("text").and_then(Value::as_str)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn count_file_changes(item: &Value) -> usize {
        item.get("changes")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default()
    }

    fn format_file_change_summary(item: &Value) -> String {
        let changes = item
            .get("changes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if changes.is_empty() {
            return "file change completed".to_string();
        }

        let entries = changes
            .iter()
            .map(|change| {
                let path = change
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown-path");
                let kind = change
                    .get("kind")
                    .and_then(|value| value.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("update");
                format!("{kind} {path}")
            })
            .collect::<Vec<_>>();

        format!("file change: {}", entries.join(", "))
    }

    fn format_command_execution_summary(item: &Value) -> String {
        let command = item
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("command");
        let status = item
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("completed");
        format!("command: {command} [{status}]")
    }

    fn initialize_detail(initialize_response: &InitializeResponse) -> String {
        format!(
            "{} / {} / {}",
            initialize_response.platform_os,
            initialize_response.platform_family,
            initialize_response.user_agent,
        )
    }

    fn with_shared_runtime<T, F>(&self, mut operation: F) -> Result<SharedRuntimeOutput<T>>
    where
        F: FnMut(&mut AppServerConnection, &str) -> Result<T>,
    {
        for attempt in 0..2 {
            let result = match self.shared_runtime.try_lock() {
                Ok(mut runtime) => self.run_request_on_locked_runtime(&mut runtime, &mut operation),
                Err(TryLockError::WouldBlock) => {
                    return self.with_isolated_runtime(
                        Some(
                            "shared runtime busy with an active turn stream; request used an isolated app-server connection"
                                .to_string(),
                        ),
                        &mut operation,
                    );
                }
                Err(TryLockError::Poisoned(_)) => {
                    Err(anyhow!("shared app-server runtime mutex was poisoned"))
                }
            };

            match result {
                Ok(output) => return Ok(output),
                Err(error) => {
                    if attempt == 0 {
                        self.reset_shared_runtime(Some(format!(
                            "shared runtime reset after request failure; retrying with a fresh app-server connection ({error})"
                        )));
                        continue;
                    }

                    self.reset_shared_runtime(None);
                    return Err(error.context(
                        "shared runtime retry also failed after reset; open diagnostics and rerun the request",
                    ));
                }
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
            let connection = runtime
                .connection
                .as_mut()
                .context("shared app-server runtime was not connected")?;
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
        let initialize_detail = Self::initialize_detail(&initialize_response);
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
            let connection = runtime
                .connection
                .as_mut()
                .context("shared app-server runtime was not connected")?;
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
        let output = self.with_shared_runtime(|connection, initialize_detail| {
            Ok((initialize_detail.to_string(), connection.read_account()?))
        })?;
        let (initialize_detail, account_response) = output.value;

        Ok(AppServerStartupContext {
            initialize_detail,
            account_detail: account_response.to_summary_text(),
            account_ok: account_response.is_authenticated(),
            warnings: output.warnings,
        })
    }

    fn load_recent_sessions(&self, limit: usize) -> Result<RecentSessions> {
        let output = self.with_shared_runtime(|connection, _| {
            connection.list_threads(ThreadListParams {
                limit: Some(limit),
                ..ThreadListParams::default()
            })
        })?;
        let ThreadListResponse { data, next_cursor } = output.value;

        let items = data
            .into_iter()
            .map(Self::to_session_summary)
            .collect::<Vec<_>>();

        Ok(RecentSessions {
            items,
            warnings: output.warnings,
            next_cursor,
        })
    }

    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        let output =
            self.with_shared_runtime(|connection, _| connection.read_thread(thread_id, true))?;
        Ok(Self::to_conversation_snapshot(
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
                title: Self::thread_title(&thread_response.thread),
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

#[derive(Default)]
struct SharedAppServerRuntime {
    connection: Option<AppServerConnection>,
    initialize_detail: Option<String>,
    pending_notices: Vec<String>,
}

impl SharedAppServerRuntime {
    fn ensure_connected(&mut self, adapter: &CodexAppServerAdapter) -> Result<()> {
        let reconnect_notice = match self.connection.as_mut() {
            Some(connection) => {
                if connection.is_alive()? {
                    return Ok(());
                }

                Some(
                    "shared runtime reconnected after the previous app-server process exited"
                        .to_string(),
                )
            }
            None => None,
        };

        self.reset();

        let mut connection = adapter.open_connection()?;
        let initialize_response = connection.initialize()?;
        self.initialize_detail = Some(CodexAppServerAdapter::initialize_detail(
            &initialize_response,
        ));
        self.connection = Some(connection);
        if let Some(notice) = reconnect_notice {
            self.push_notice(notice);
        }
        Ok(())
    }

    fn initialize_detail(&self) -> Result<&str> {
        self.initialize_detail
            .as_deref()
            .context("shared runtime initialize detail was not available")
    }

    fn reset(&mut self) {
        self.initialize_detail = None;
        self.connection.take();
    }

    fn push_notice(&mut self, notice: String) {
        self.pending_notices.push(notice);
    }

    fn push_notices(&mut self, notices: Vec<String>) {
        self.pending_notices.extend(notices);
    }

    fn take_notices(&mut self) -> Vec<String> {
        let mut notices = std::mem::take(&mut self.pending_notices);
        sort_and_dedup_warnings(&mut notices);
        notices
    }
}

struct SharedRuntimeOutput<T> {
    value: T,
    warnings: Vec<String>,
}

struct AppServerConnection {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<AppServerLine>,
    warnings: Vec<String>,
    next_request_id: i64,
    client_name: String,
    client_version: String,
    initialized: bool,
}

impl AppServerConnection {
    fn is_alive(&mut self) -> Result<bool> {
        Ok(self.child.try_wait()?.is_none())
    }

    fn initialize(&mut self) -> Result<InitializeResponse> {
        if self.initialized {
            bail!("initialize was already called");
        }

        let response = self.send_request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": self.client_name,
                    "version": self.client_version,
                },
                "capabilities": {
                    "experimentalApi": false,
                }
            }),
        )?;

        self.send_notification("initialized", json!({}))?;
        self.initialized = true;
        Ok(response)
    }

    fn read_account(&mut self) -> Result<AccountReadResponse> {
        self.ensure_initialized()?;
        self.send_request("account/read", json!({}))
    }

    fn list_threads(&mut self, params: ThreadListParams) -> Result<ThreadListResponse> {
        self.ensure_initialized()?;
        self.send_request("thread/list", serde_json::to_value(params)?)
    }

    fn read_thread(&mut self, thread_id: &str, include_turns: bool) -> Result<ThreadReadResponse> {
        self.ensure_initialized()?;
        self.send_request(
            "thread/read",
            json!({
                "threadId": thread_id,
                "includeTurns": include_turns,
            }),
        )
    }

    fn start_thread(&mut self, params: ThreadStartParams) -> Result<ThreadStartResponse> {
        self.ensure_initialized()?;
        self.send_request("thread/start", serde_json::to_value(params)?)
    }

    fn resume_thread(&mut self, thread_id: &str) -> Result<ThreadResumeResponse> {
        self.ensure_initialized()?;
        self.send_request(
            "thread/resume",
            json!({
                "threadId": thread_id,
            }),
        )
    }

    fn start_turn(&mut self, thread_id: &str, prompt: &str) -> Result<TurnStartResponse> {
        self.ensure_initialized()?;
        self.send_request(
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [
                    {
                        "type": "text",
                        "text": prompt,
                    }
                ],
            }),
        )
    }

    fn wait_for_turn_stream(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        event_sender: &Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        loop {
            if let Some(status) = self.child.try_wait()? {
                bail!("app-server exited before the turn completed: {status}");
            }

            match self.rx.recv_timeout(Duration::from_millis(200)) {
                Ok(AppServerLine::Stderr(line)) => self.warnings.push(line),
                Ok(AppServerLine::Stdout(line)) => {
                    let value: Value = serde_json::from_str(&line)
                        .with_context(|| format!("invalid JSON from app-server: {line}"))?;

                    if self.capture_notification_warning(&value) {
                        continue;
                    }

                    if let Some(method) = value.get("method").and_then(Value::as_str) {
                        if self.handle_turn_notification(
                            method,
                            value.get("params"),
                            thread_id,
                            turn_id,
                            event_sender,
                        )? {
                            return Ok(());
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("app-server pipe closed while waiting for turn events");
                }
            }
        }
    }

    fn handle_turn_notification(
        &mut self,
        method: &str,
        params: Option<&Value>,
        thread_id: &str,
        turn_id: &str,
        event_sender: &Sender<ConversationStreamEvent>,
    ) -> Result<bool> {
        let Some(params) = params else {
            return Ok(false);
        };

        match method {
            "thread/status/changed" => {
                if params.get("threadId").and_then(Value::as_str) == Some(thread_id) {
                    let status = params
                        .get("status")
                        .and_then(|value| value.get("type"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let _ = event_sender.send(ConversationStreamEvent::StatusUpdated {
                        text: format!("thread status: {status}"),
                    });
                }
            }
            "turn/started" => {
                if params.get("threadId").and_then(Value::as_str) == Some(thread_id) {
                    let started_turn_id = params
                        .get("turn")
                        .and_then(|value| value.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or(turn_id);
                    let _ = event_sender.send(ConversationStreamEvent::TurnStarted {
                        turn_id: started_turn_id.to_string(),
                    });
                }
            }
            "item/agentMessage/delta" => {
                if params.get("turnId").and_then(Value::as_str) == Some(turn_id) {
                    let item_id = params
                        .get("itemId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let delta = params
                        .get("delta")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let _ = event_sender.send(ConversationStreamEvent::AgentMessageDelta {
                        item_id,
                        phase: params
                            .get("phase")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        delta,
                    });
                }
            }
            "item/completed" => {
                if params.get("turnId").and_then(Value::as_str) == Some(turn_id) {
                    self.handle_completed_item(params.get("item"), event_sender);
                }
            }
            "error" => {
                let message = params
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("app-server reported an error")
                    .to_string();
                bail!(message);
            }
            "turn/completed" => {
                let completed_thread_id = params.get("threadId").and_then(Value::as_str);
                let completed_turn_id = params
                    .get("turn")
                    .and_then(|value| value.get("id"))
                    .and_then(Value::as_str);

                if completed_thread_id == Some(thread_id) && completed_turn_id == Some(turn_id) {
                    let _ = event_sender.send(ConversationStreamEvent::TurnCompleted {
                        turn_id: turn_id.to_string(),
                    });
                    return Ok(true);
                }
            }
            _ => {}
        }

        Ok(false)
    }

    fn handle_completed_item(
        &self,
        item: Option<&Value>,
        event_sender: &Sender<ConversationStreamEvent>,
    ) {
        let Some(item) = item else {
            return;
        };

        let item_type = item.get("type").and_then(Value::as_str);
        match item_type {
            Some("agentMessage") => {
                let item_id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let text = item
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let phase = item
                    .get("phase")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let _ = event_sender.send(ConversationStreamEvent::AgentMessageCompleted {
                    item_id,
                    phase,
                    text,
                });
            }
            Some("fileChange") => {
                let _ = event_sender.send(ConversationStreamEvent::ToolActivity {
                    activity: ConversationToolActivity {
                        kind: ConversationToolActivityKind::FileChange,
                        text: CodexAppServerAdapter::format_file_change_summary(item),
                        file_change_count: CodexAppServerAdapter::count_file_changes(item),
                    },
                });
            }
            Some("commandExecution") => {
                let _ = event_sender.send(ConversationStreamEvent::ToolActivity {
                    activity: ConversationToolActivity {
                        kind: ConversationToolActivityKind::CommandExecution,
                        text: CodexAppServerAdapter::format_command_execution_summary(item),
                        file_change_count: 0,
                    },
                });
            }
            _ => {}
        }
    }

    fn take_warnings(&mut self) -> Vec<String> {
        self.collect_remaining_warnings();
        sort_and_dedup_warnings(&mut self.warnings);
        std::mem::take(&mut self.warnings)
    }

    fn ensure_initialized(&self) -> Result<()> {
        if self.initialized {
            Ok(())
        } else {
            bail!("app-server connection is not initialized")
        }
    }

    fn send_request<T>(&mut self, method: &str, params: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let request_id = self.next_request_id;
        self.next_request_id += 1;

        self.send_json_line(json!({
            "id": request_id,
            "method": method,
            "params": params,
        }))?;

        let response_value = self.wait_for_response(request_id)?;
        serde_json::from_value(response_value)
            .with_context(|| format!("failed to deserialize app-server response for `{method}`"))
    }

    fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        self.send_json_line(json!({
            "method": method,
            "params": params,
        }))
    }

    fn send_json_line(&mut self, value: Value) -> Result<()> {
        writeln!(self.stdin, "{}", serde_json::to_string(&value)?)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn wait_for_response(&mut self, request_id: i64) -> Result<Value> {
        let deadline = Instant::now() + APP_SERVER_TIMEOUT;

        loop {
            if Instant::now() > deadline {
                bail!("timed out waiting for app-server response id={request_id}");
            }

            if let Some(status) = self.child.try_wait()? {
                bail!("app-server exited early with status {status}");
            }

            match self.rx.recv_timeout(Duration::from_millis(200)) {
                Ok(AppServerLine::Stderr(line)) => self.warnings.push(line),
                Ok(AppServerLine::Stdout(line)) => {
                    let value: Value = serde_json::from_str(&line)
                        .with_context(|| format!("invalid JSON from app-server: {line}"))?;

                    if self.capture_notification_warning(&value) {
                        continue;
                    }

                    if value.get("id").and_then(Value::as_i64) == Some(request_id) {
                        if let Some(error) = value.get("error") {
                            return Err(anyhow!(
                                "app-server returned error for id {request_id}: {error}"
                            ));
                        }

                        if let Some(result) = value.get("result") {
                            return Ok(result.clone());
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("app-server pipe closed while waiting for response id={request_id}");
                }
            }
        }
    }

    fn capture_notification_warning(&mut self, value: &Value) -> bool {
        if value.get("method").and_then(Value::as_str) != Some("configWarning") {
            return false;
        }

        if let Some(summary) = value
            .get("params")
            .and_then(|params| params.get("summary"))
            .and_then(Value::as_str)
        {
            self.warnings.push(summary.to_string());
        }
        true
    }

    fn collect_remaining_warnings(&mut self) {
        let drain_deadline = Instant::now() + Duration::from_millis(300);
        while Instant::now() < drain_deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }

            match self.rx.recv_timeout(Duration::from_millis(50)) {
                Ok(AppServerLine::Stderr(line)) => self.warnings.push(line),
                Ok(AppServerLine::Stdout(line)) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&line) {
                        self.capture_notification_warning(&value);
                    }
                }
                Err(_) => break,
            }
        }
    }

    fn terminate_child(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for AppServerConnection {
    fn drop(&mut self) {
        self.terminate_child();
    }
}

enum AppServerLine {
    Stdout(String),
    Stderr(String),
}

fn sort_and_dedup_warnings(warnings: &mut Vec<String>) {
    warnings.sort();
    warnings.dedup();
}

fn spawn_pipe_reader<T: std::io::Read + Send + 'static>(
    pipe: T,
    tx: mpsc::Sender<AppServerLine>,
    is_stderr: bool,
) {
    thread::spawn(move || {
        let reader = BufReader::new(pipe);
        for line in reader.lines().map_while(|value| value.ok()) {
            let payload = if is_stderr {
                AppServerLine::Stderr(line)
            } else {
                AppServerLine::Stdout(line)
            };
            let _ = tx.send(payload);
        }
    });
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializeResponse {
    user_agent: String,
    platform_family: String,
    platform_os: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountReadResponse {
    account: Option<AccountRecord>,
    requires_openai_auth: Option<bool>,
}

impl AccountReadResponse {
    fn is_authenticated(&self) -> bool {
        self.account.is_some() || !self.requires_openai_auth.unwrap_or(false)
    }

    fn to_summary_text(&self) -> String {
        match &self.account {
            Some(account) if account.account_type == "chatgpt" => format!(
                "chatgpt / {} / {}",
                account.email.as_deref().unwrap_or("unknown-email"),
                account.plan_type.as_deref().unwrap_or("unknown-plan"),
            ),
            Some(account) if account.account_type == "apiKey" => "api key account".to_string(),
            Some(account) => format!("account type: {}", account.account_type),
            None if self.requires_openai_auth.unwrap_or(false) => {
                "not logged in (OpenAI auth required)".to_string()
            }
            None => "no account configured".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountRecord {
    #[serde(rename = "type")]
    account_type: String,
    email: Option<String>,
    plan_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    search_term: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_kinds: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadStartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadListResponse {
    data: Vec<ThreadRecord>,
    next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ThreadReadResponse {
    thread: ThreadRecord,
}

#[derive(Debug, Clone, Deserialize)]
struct ThreadStartResponse {
    thread: ThreadRecord,
}

#[derive(Debug, Clone, Deserialize)]
struct ThreadResumeResponse {
    #[serde(rename = "thread")]
    _thread: ThreadRecord,
}

#[derive(Debug, Clone, Deserialize)]
struct TurnStartResponse {
    turn: TurnRecord,
}

#[derive(Debug, Clone, Deserialize)]
struct TurnRecord {
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadRecord {
    id: String,
    name: Option<String>,
    preview: String,
    cwd: String,
    source: String,
    model_provider: String,
    updated_at: i64,
    path: String,
    status: ThreadStatus,
    git_info: Option<ThreadGitInfo>,
    #[serde(default)]
    turns: Vec<ThreadTurnRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct ThreadTurnRecord {
    #[serde(default)]
    items: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct ThreadStatus {
    #[serde(rename = "type")]
    status_type: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadGitInfo {
    branch: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{SharedAppServerRuntime, sort_and_dedup_warnings};

    #[test]
    fn reset_preserves_pending_runtime_notices() {
        let mut runtime = SharedAppServerRuntime::default();
        runtime.push_notice("runtime retried".to_string());

        runtime.reset();

        assert_eq!(runtime.take_notices(), vec!["runtime retried".to_string()]);
        assert!(runtime.take_notices().is_empty());
    }

    #[test]
    fn warning_lists_are_sorted_and_deduplicated() {
        let mut warnings = vec!["zeta".to_string(), "alpha".to_string(), "alpha".to_string()];

        sort_and_dedup_warnings(&mut warnings);

        assert_eq!(warnings, vec!["alpha".to_string(), "zeta".to_string()]);
    }

    #[test]
    fn take_notices_normalizes_stream_and_retry_messages() {
        let mut runtime = SharedAppServerRuntime::default();
        runtime.push_notice("shared runtime reset".to_string());
        runtime.push_notices(vec![
            "stream warning".to_string(),
            "shared runtime reset".to_string(),
        ]);

        assert_eq!(
            runtime.take_notices(),
            vec![
                "shared runtime reset".to_string(),
                "stream warning".to_string(),
            ]
        );
    }
}

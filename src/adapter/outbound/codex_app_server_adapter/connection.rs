use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::domain::conversation::ConversationStreamEvent;

use super::protocol::{
    AccountReadResponse, AppServerNotification, InitializeResponse, ThreadListParams,
    ThreadListResponse, ThreadReadResponse, ThreadResumeParams, ThreadResumeResponse,
    ThreadStartParams, ThreadStartResponse, TurnNotificationHandling, TurnStartParams,
    TurnStartResponse, handle_turn_notification, sort_and_dedup_warnings,
};

const RESPONSE_TIMEOUT_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_RESPONSE_TIMEOUT_SECS";
const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(200);
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_millis(300);
const DEFAULT_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_FATAL_STDERR_LINES: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AppServerConnectionConfig {
    response_timeout: Duration,
    poll_interval: Duration,
    drain_timeout: Duration,
    drain_poll_interval: Duration,
}

impl Default for AppServerConnectionConfig {
    fn default() -> Self {
        Self {
            response_timeout: DEFAULT_RESPONSE_TIMEOUT,
            poll_interval: DEFAULT_POLL_INTERVAL,
            drain_timeout: DEFAULT_DRAIN_TIMEOUT,
            drain_poll_interval: DEFAULT_DRAIN_POLL_INTERVAL,
        }
    }
}

impl AppServerConnectionConfig {
    pub(super) fn from_environment() -> Self {
        Self::from_response_timeout_secs_value(
            std::env::var(RESPONSE_TIMEOUT_ENV_VAR).ok().as_deref(),
        )
    }

    fn from_response_timeout_secs_value(value: Option<&str>) -> Self {
        let mut config = Self::default();

        let Some(raw_value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            return config;
        };
        let Ok(seconds) = raw_value.parse::<u64>() else {
            return config;
        };
        if seconds == 0 {
            return config;
        }

        config.response_timeout = Duration::from_secs(seconds);
        config
    }
}

pub(super) struct AppServerConnection {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<AppServerLine>,
    diagnostics: ConnectionDiagnostics,
    pending_notifications: PendingNotifications,
    next_request_id: i64,
    client_name: String,
    client_version: String,
    initialized: bool,
    config: AppServerConnectionConfig,
}

impl AppServerConnection {
    pub(super) fn spawn(
        client_name: String,
        client_version: String,
        config: AppServerConnectionConfig,
    ) -> Result<Self> {
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

        Ok(Self {
            child,
            stdin,
            rx,
            diagnostics: ConnectionDiagnostics::default(),
            pending_notifications: PendingNotifications::default(),
            next_request_id: 1,
            client_name,
            client_version,
            initialized: false,
            config,
        })
    }

    pub(super) fn is_alive(&mut self) -> Result<bool> {
        Ok(self.child.try_wait()?.is_none())
    }

    pub(super) fn initialize(&mut self) -> Result<InitializeResponse> {
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

    pub(super) fn read_account(&mut self) -> Result<AccountReadResponse> {
        self.ensure_initialized()?;
        self.send_request("account/read", json!({}))
    }

    pub(super) fn list_threads(&mut self, params: ThreadListParams) -> Result<ThreadListResponse> {
        self.ensure_initialized()?;
        self.send_request("thread/list", serde_json::to_value(params)?)
    }

    pub(super) fn read_thread(
        &mut self,
        thread_id: &str,
        include_turns: bool,
    ) -> Result<ThreadReadResponse> {
        self.ensure_initialized()?;
        self.send_request(
            "thread/read",
            json!({
                "threadId": thread_id,
                "includeTurns": include_turns,
            }),
        )
    }

    pub(super) fn start_thread(
        &mut self,
        params: ThreadStartParams,
    ) -> Result<ThreadStartResponse> {
        self.ensure_initialized()?;
        self.send_request("thread/start", serde_json::to_value(params)?)
    }

    pub(super) fn resume_thread(
        &mut self,
        params: ThreadResumeParams,
    ) -> Result<ThreadResumeResponse> {
        self.ensure_initialized()?;
        self.send_request("thread/resume", serde_json::to_value(params)?)
    }

    pub(super) fn start_turn(&mut self, params: TurnStartParams) -> Result<TurnStartResponse> {
        self.ensure_initialized()?;
        self.send_request("turn/start", serde_json::to_value(params)?)
    }

    pub(super) fn wait_for_turn_stream(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        event_sender: &Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        let mut changed_planning_file_paths = Vec::new();

        loop {
            if let Some(status) = self.child.try_wait()? {
                return Err(self.error_with_diagnostics(format!(
                    "app-server exited before the turn completed: {status}"
                )));
            }

            if self.process_pending_turn_notification(
                thread_id,
                turn_id,
                &mut changed_planning_file_paths,
                event_sender,
            )? {
                return Ok(());
            }

            match self.rx.recv_timeout(self.config.poll_interval) {
                Ok(AppServerLine::Stderr(line)) => self.diagnostics.record_stderr(line),
                Ok(AppServerLine::Stdout(line)) => {
                    let value = self.parse_json_line(&line)?;

                    if let Some(notification) = AppServerNotification::from_value(value) {
                        if self.handle_turn_stream_notification(
                            notification,
                            thread_id,
                            turn_id,
                            &mut changed_planning_file_paths,
                            event_sender,
                        )? {
                            return Ok(());
                        }
                    } else {
                        self.diagnostics.record_warning(
                            "app-server sent a non-notification JSON message while streaming the active turn"
                                .to_string(),
                        );
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(self.error_with_diagnostics(
                        "app-server pipe closed while waiting for turn events",
                    ));
                }
            }
        }
    }

    pub(super) fn take_warnings(&mut self) -> Vec<String> {
        self.collect_remaining_warnings();
        self.diagnostics
            .record_warnings(self.pending_notifications.drain_warning_texts());
        self.diagnostics.take_warnings()
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
        let deadline = Instant::now() + self.config.response_timeout;

        loop {
            if Instant::now() > deadline {
                return Err(self.error_with_diagnostics(format!(
                    "timed out waiting for app-server response id={request_id} after {}s",
                    self.config.response_timeout.as_secs()
                )));
            }

            if let Some(status) = self.child.try_wait()? {
                return Err(self.error_with_diagnostics(format!(
                    "app-server exited early with status {status}"
                )));
            }

            match self.rx.recv_timeout(self.config.poll_interval) {
                Ok(AppServerLine::Stderr(line)) => self.diagnostics.record_stderr(line),
                Ok(AppServerLine::Stdout(line)) => {
                    let value = self.parse_json_line(&line)?;

                    if let Some(response_id) = value.get("id").and_then(Value::as_i64) {
                        if response_id != request_id {
                            self.diagnostics.record_warning(format!(
                                "app-server returned response id={response_id} while waiting for id={request_id}"
                            ));
                            continue;
                        }

                        if let Some(error) = value.get("error") {
                            return Err(self.error_with_diagnostics(format!(
                                "app-server returned error for id {request_id}: {error}"
                            )));
                        }

                        if let Some(result) = value.get("result") {
                            return Ok(result.clone());
                        }

                        return Err(self.error_with_diagnostics(format!(
                            "app-server returned response id {request_id} without a result payload"
                        )));
                    }

                    if let Some(notification) = AppServerNotification::from_value(value) {
                        self.handle_response_wait_notification(request_id, notification);
                        continue;
                    }

                    self.diagnostics.record_warning(format!(
                        "app-server sent an unexpected JSON message while waiting for response id={request_id}"
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(self.error_with_diagnostics(format!(
                        "app-server pipe closed while waiting for response id={request_id}"
                    )));
                }
            }
        }
    }

    fn handle_response_wait_notification(
        &mut self,
        request_id: i64,
        notification: AppServerNotification,
    ) {
        if notification.should_defer_to_turn_stream() {
            self.pending_notifications.push(notification);
            return;
        }

        self.diagnostics.record_warning(
            notification.warning_text(&format!("while waiting for response id={request_id}")),
        );
    }

    fn process_pending_turn_notification(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        changed_planning_file_paths: &mut Vec<String>,
        event_sender: &Sender<ConversationStreamEvent>,
    ) -> Result<bool> {
        let Some(notification) = self.pending_notifications.pop_front() else {
            return Ok(false);
        };

        self.handle_turn_stream_notification(
            notification,
            thread_id,
            turn_id,
            changed_planning_file_paths,
            event_sender,
        )
    }

    fn handle_turn_stream_notification(
        &mut self,
        notification: AppServerNotification,
        thread_id: &str,
        turn_id: &str,
        changed_planning_file_paths: &mut Vec<String>,
        event_sender: &Sender<ConversationStreamEvent>,
    ) -> Result<bool> {
        if !notification.should_defer_to_turn_stream() {
            self.diagnostics
                .record_warning(notification.warning_text("while streaming the active turn"));
            return Ok(false);
        }

        match handle_turn_notification(
            &notification,
            thread_id,
            turn_id,
            changed_planning_file_paths,
            event_sender,
        )? {
            TurnNotificationHandling::Consumed => Ok(false),
            TurnNotificationHandling::Completed => Ok(true),
            TurnNotificationHandling::Dropped(warning) => {
                self.diagnostics.record_warning(warning);
                Ok(false)
            }
        }
    }

    fn parse_json_line(&self, line: &str) -> Result<Value> {
        serde_json::from_str(line).map_err(|error| {
            self.error_with_diagnostics(format!("invalid JSON from app-server: {line} ({error})"))
        })
    }

    fn collect_remaining_warnings(&mut self) {
        let drain_deadline = Instant::now() + self.config.drain_timeout;
        while Instant::now() < drain_deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }

            match self.rx.recv_timeout(self.config.drain_poll_interval) {
                Ok(AppServerLine::Stderr(line)) => self.diagnostics.record_stderr(line),
                Ok(AppServerLine::Stdout(line)) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&line)
                        && let Some(notification) = AppServerNotification::from_value(value)
                    {
                        if notification.should_defer_to_turn_stream() {
                            self.pending_notifications.push(notification);
                        } else {
                            self.diagnostics.record_warning(
                                notification.warning_text("while draining app-server notices"),
                            );
                        }
                    }
                }
                Err(_) => break,
            }
        }
    }

    fn error_with_diagnostics(&self, message: impl Into<String>) -> anyhow::Error {
        self.diagnostics.error(message)
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

#[derive(Default)]
struct PendingNotifications {
    entries: VecDeque<AppServerNotification>,
}

impl PendingNotifications {
    fn push(&mut self, notification: AppServerNotification) {
        self.entries.push_back(notification);
    }

    fn pop_front(&mut self) -> Option<AppServerNotification> {
        self.entries.pop_front()
    }

    fn drain_warning_texts(&mut self) -> Vec<String> {
        self.entries
            .drain(..)
            .map(|notification| {
                notification
                    .warning_text("after the response completed without a turn stream consumer")
            })
            .collect()
    }
}

#[derive(Default)]
struct ConnectionDiagnostics {
    warnings: Vec<String>,
    fatal_stderr: Vec<String>,
}

impl ConnectionDiagnostics {
    fn record_warning(&mut self, warning: String) {
        if !warning.trim().is_empty() {
            self.warnings.push(warning);
        }
    }

    fn record_warnings<I>(&mut self, warnings: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.warnings.extend(
            warnings
                .into_iter()
                .filter(|warning| !warning.trim().is_empty()),
        );
    }

    fn record_stderr(&mut self, line: String) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }

        if is_fatal_stderr_line(trimmed) {
            self.fatal_stderr.push(trimmed.to_string());
            if self.fatal_stderr.len() > MAX_FATAL_STDERR_LINES {
                self.fatal_stderr.remove(0);
            }
        } else {
            self.warnings.push(trimmed.to_string());
        }
    }

    fn take_warnings(&mut self) -> Vec<String> {
        sort_and_dedup_warnings(&mut self.warnings);
        std::mem::take(&mut self.warnings)
    }

    fn error(&self, message: impl Into<String>) -> anyhow::Error {
        let mut message = message.into();
        if !self.fatal_stderr.is_empty() {
            message.push_str(" / recent stderr: ");
            message.push_str(&self.fatal_stderr.join(" | "));
        }
        anyhow!(message)
    }
}

fn is_fatal_stderr_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();

    lower.starts_with("fatal")
        || lower.starts_with("panic")
        || lower.starts_with("error")
        || lower.contains(" fatal ")
        || lower.contains(" panic")
        || lower.contains(" error")
        || lower.contains("failed")
        || lower.contains("backtrace")
}

enum AppServerLine {
    Stdout(String),
    Stderr(String),
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::{
        AppServerConnectionConfig, ConnectionDiagnostics, PendingNotifications,
        RESPONSE_TIMEOUT_ENV_VAR,
    };
    use crate::adapter::outbound::codex_app_server_adapter::protocol::AppServerNotification;

    #[test]
    fn response_timeout_defaults_to_fifteen_seconds() {
        assert_eq!(
            AppServerConnectionConfig::default().response_timeout,
            Duration::from_secs(15)
        );
    }

    #[test]
    fn response_timeout_uses_positive_environment_override() {
        assert_eq!(
            AppServerConnectionConfig::from_response_timeout_secs_value(Some("12"))
                .response_timeout,
            Duration::from_secs(12)
        );
    }

    #[test]
    fn response_timeout_ignores_invalid_environment_values() {
        for value in [Some("0"), Some("bogus"), Some("-1"), Some("  ")] {
            assert_eq!(
                AppServerConnectionConfig::from_response_timeout_secs_value(value).response_timeout,
                Duration::from_secs(15)
            );
        }
    }

    #[test]
    fn pending_notifications_become_warnings_if_no_turn_stream_consumes_them() {
        let mut pending = PendingNotifications::default();
        pending.push(
            AppServerNotification::from_value(json!({
                "method": "item/agentMessage/delta",
                "params": {
                    "turnId": "turn-1"
                }
            }))
            .expect("notification should parse"),
        );

        assert_eq!(
            pending.drain_warning_texts(),
            vec![
                "app-server sent notification `item/agentMessage/delta` after the response completed without a turn stream consumer"
                    .to_string()
            ]
        );
    }

    #[test]
    fn fatal_stderr_is_attached_to_errors_instead_of_warning_bucket() {
        let mut diagnostics = ConnectionDiagnostics::default();
        diagnostics.record_stderr("fatal: transport closed".to_string());
        diagnostics.record_stderr("workspace template missing".to_string());

        assert_eq!(
            diagnostics.take_warnings(),
            vec!["workspace template missing".to_string()]
        );
        assert!(
            diagnostics
                .error("turn failed")
                .to_string()
                .contains("fatal: transport closed")
        );
    }

    #[test]
    fn timeout_env_var_name_is_stable() {
        assert_eq!(
            RESPONSE_TIMEOUT_ENV_VAR,
            "CODEX_EXEC_LOOP_APP_SERVER_RESPONSE_TIMEOUT_SECS"
        );
    }
}

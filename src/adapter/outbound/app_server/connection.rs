/*
 * connection.rs는 `codex app-server` child process와 직접 대화하는 lowest-level outbound boundary다.
 * 위 계층은 typed method(start_thread, start_turn 등)를 호출하지만, 이 파일은 stdin에 JSON line을 쓰고
 * stdout/stderr reader thread에서 notification/response line을 받아 request id와 매칭한다.
 */
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::service::conversation_runtime_event::ConversationStreamEvent;

use super::protocol::{
    AccountReadResponse, AppServerNotification, InitializeResponse, ThreadListParams,
    ThreadListResponse, ThreadReadResponse, ThreadResumeParams, ThreadResumeResponse,
    ThreadStartParams, ThreadStartResponse, TurnInterruptParams, TurnInterruptResponse,
    TurnNotificationHandling, TurnStartParams, TurnStartResponse, handle_turn_notification,
};

const RESPONSE_TIMEOUT_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_RESPONSE_TIMEOUT_SECS";
const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(200);
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_millis(300);
const DEFAULT_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(50);

mod diagnostics;

use self::diagnostics::{ConnectionDiagnostics, PendingNotifications};

#[derive(Clone, Default)]
pub(super) struct AppServerTurnInterruptSignal {
    /*
     * Ctrl-C 같은 stop 요청은 특정 connection instance가 아니라 모든 active session에 적용된다.
     * generation counter를 쓰면 각 stream loop가 시작 시점에 본 값과 현재 값을 비교해, 자신이 시작한 뒤
     * stop 요청이 들어왔는지 lock 없이 판단할 수 있다.
     */
    generation: Arc<AtomicU64>,
}

impl AppServerTurnInterruptSignal {
    pub(super) fn request_stop_all_sessions(&self) {
        // SeqCst를 사용해 UI thread의 stop 요청과 stream loop의 관찰 순서를 가장 보수적으로 맞춘다.
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    pub(super) fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn requested_after(&self, observed_generation: u64) -> bool {
        self.current_generation() > observed_generation
    }
}

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
        // 운영 override는 response timeout만 열어두고, poll/drain 간격은 stream responsiveness 기준으로 고정한다.
        Self::from_response_timeout_secs_value(
            std::env::var(RESPONSE_TIMEOUT_ENV_VAR).ok().as_deref(),
        )
    }

    fn from_response_timeout_secs_value(value: Option<&str>) -> Self {
        // 잘못된 env 값은 startup failure가 아니라 default fallback이다. app-server diagnostics가 더 중요하다.
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
    /*
     * AppServerConnection owns the child handle and stdin writer, while stdout/stderr are moved into reader
     * threads. The mpsc receiver is therefore the single place where response lines, stream notifications, and
     * stderr diagnostics are serialized back into request/stream control flow.
     */
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<AppServerLine>,
    diagnostics: ConnectionDiagnostics,
    // Notifications observed while waiting for a normal response are buffered until the active turn stream can consume them.
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
        /*
         * stdin/stdout/stderr를 모두 piped로 열어야 app-server와 line protocol을 주고받을 수 있다.
         * stdout/stderr는 blocking read가 필요하므로 reader thread가 mpsc sender로 AppServerLine을 넘기고,
         * 이 connection object는 request id matching과 stream notification reduction만 수행한다.
         */
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
        // JSON-RPC initialize must be exactly once per child process before any typed app-server methods are called.
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

    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) fn list_threads(&mut self, params: ThreadListParams) -> Result<ThreadListResponse> {
        self.ensure_initialized()?;
        self.send_request("thread/list", serde_json::to_value(params)?)
    }

    #[tracing::instrument(level = "trace", skip(self))]
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

    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) fn start_thread(
        &mut self,
        params: ThreadStartParams,
    ) -> Result<ThreadStartResponse> {
        self.ensure_initialized()?;
        self.send_request("thread/start", serde_json::to_value(params)?)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) fn resume_thread(
        &mut self,
        params: ThreadResumeParams,
    ) -> Result<ThreadResumeResponse> {
        self.ensure_initialized()?;
        self.send_request("thread/resume", serde_json::to_value(params)?)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) fn start_turn(&mut self, params: TurnStartParams) -> Result<TurnStartResponse> {
        self.ensure_initialized()?;
        self.send_request("turn/start", serde_json::to_value(params)?)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) fn interrupt_turn(
        &mut self,
        params: TurnInterruptParams,
    ) -> Result<TurnInterruptResponse> {
        self.ensure_initialized()?;
        self.send_request("turn/interrupt", serde_json::to_value(params)?)
    }

    pub(super) fn wait_for_turn_stream(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        interrupt_signal: &AppServerTurnInterruptSignal,
        observed_interrupt_generation: u64,
        event_sender: &Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        /*
         * Turn streaming interleaves three input sources: child process exit, global interrupt generation, and
         * stdout/stderr lines. Pending notifications are drained before blocking on rx so notifications that arrived
         * during `turn/start` response waiting are not delayed until a new line appears.
         */
        let mut changed_planning_file_paths = Vec::new();
        let mut interrupt_sent = false;

        loop {
            if let Some(status) = self.child.try_wait()? {
                return Err(self.error_with_diagnostics(format!(
                    "app-server exited before the turn completed: {status}"
                )));
            }

            if !interrupt_sent && interrupt_signal.requested_after(observed_interrupt_generation) {
                /*
                 * The interrupt signal is process-wide, while this loop owns exactly one
                 * active turn. After translating the first newer generation into
                 * `turn/interrupt`, later increments are left as UI intent instead of
                 * repeatedly sending the same app-server method for this turn id.
                 */
                self.interrupt_active_turn(thread_id, turn_id, event_sender);
                interrupt_sent = true;
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

    fn interrupt_active_turn(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        event_sender: &Sender<ConversationStreamEvent>,
    ) {
        /*
         * Interrupt failure is warning-level because the stream is still authoritative.
         * app-server may complete, fail, or close the turn naturally after the UI asks
         * for stop, so this method reports status without taking over stream outcome.
         */
        match self.interrupt_turn(TurnInterruptParams {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
        }) {
            Ok(_) => {
                let _ = event_sender.send(ConversationStreamEvent::StatusUpdated {
                    text: "stop requested / app-server interrupt sent".to_string(),
                });
            }
            Err(error) => self.diagnostics.record_warning(format!(
                "stop requested but app-server interrupt failed for turn `{turn_id}`: {error}"
            )),
        }
    }

    pub(super) fn take_warnings(&mut self) -> Vec<String> {
        /*
         * A successful response can be followed immediately by stderr or loose
         * notifications from the child. The short drain window gives callers the
         * operator-visible context without turning normal request completion into a
         * blocking "wait until silence" operation.
         */
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
        /*
         * Request ids are connection-local because one child process owns one JSON-RPC
         * sequence. Keeping them monotonic lets response wait detect stale or
         * out-of-order replies instead of accidentally deserializing the wrong method's
         * result into the typed response expected by the caller.
         */
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
        /*
         * app-server speaks newline-delimited JSON over stdio, not a framed socket.
         * The explicit flush is part of the transport contract: without it, a request
         * can sit in the parent process buffer while the caller waits for a response
         * that the child has not been allowed to observe.
         */
        writeln!(self.stdin, "{}", serde_json::to_string(&value)?)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn wait_for_response(&mut self, request_id: i64) -> Result<Value> {
        /*
         * Normal requests wait for exactly one matching response id. Notifications that arrive during this wait are
         * either downgraded to diagnostics or buffered for the turn stream if they are stream-owned methods.
         */
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
        /*
         * Stream-owned notifications can legitimately race ahead of the `turn/start`
         * response. Buffering them preserves app-server arrival order across the
         * response-to-stream handoff instead of converting early deltas into warnings.
         */
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
        /*
         * Pending notifications are consumed before blocking on the reader channel so
         * deltas observed during response waiting are reduced before later stdout
         * lines. This keeps the stream reducer's changed-file and completion state in
         * protocol order.
         */
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
        /*
         * protocol::handle_turn_notification owns payload translation into domain
         * stream events. This connection layer keeps the transport responsibilities:
         * reject non-stream notifications, retain diagnostics, and decide when the
         * outer wait loop can stop.
         */
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
        /*
         * The drain is deliberately short and warning-only. It catches stderr/config
         * notices emitted just after the response line, while avoiding a second stream
         * consumer that could steal turn notifications from wait_for_turn_stream.
         */
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
        // Drop is best-effort cleanup; errors are ignored because callers already have request/stream diagnostics.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for AppServerConnection {
    fn drop(&mut self) {
        // Shared runtimes may hold app-server children for a long time; dropping the connection must not leave a child behind.
        self.terminate_child();
    }
}

enum AppServerLine {
    // Stdout carries JSON-RPC responses and notifications.
    Stdout(String),
    // Stderr is diagnostics-only and never parsed as protocol.
    Stderr(String),
}

fn spawn_pipe_reader<T: std::io::Read + Send + 'static>(
    pipe: T,
    tx: mpsc::Sender<AppServerLine>,
    is_stderr: bool,
) {
    // One reader thread per pipe converts blocking line reads into nonblocking channel messages for the connection loop.
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

    use super::{AppServerConnectionConfig, RESPONSE_TIMEOUT_ENV_VAR};

    #[test]
    fn response_timeout_defaults_to_fifteen_seconds() {
        // Default timeout bounds short app-server calls without making startup checks too eager to fail.
        assert_eq!(
            AppServerConnectionConfig::default().response_timeout,
            Duration::from_secs(15)
        );
    }

    #[test]
    fn response_timeout_uses_positive_environment_override() {
        // Positive override lets slow machines or instrumented app-server builds extend request wait time.
        assert_eq!(
            AppServerConnectionConfig::from_response_timeout_secs_value(Some("12"))
                .response_timeout,
            Duration::from_secs(12)
        );
    }

    #[test]
    fn response_timeout_ignores_invalid_environment_values() {
        // Invalid env values must not break TUI startup; they simply fall back to the compiled default.
        for value in [Some("0"), Some("bogus"), Some("-1"), Some("  ")] {
            assert_eq!(
                AppServerConnectionConfig::from_response_timeout_secs_value(value).response_timeout,
                Duration::from_secs(15)
            );
        }
    }

    #[test]
    fn timeout_env_var_name_is_stable() {
        assert_eq!(
            RESPONSE_TIMEOUT_ENV_VAR,
            "CODEX_EXEC_LOOP_APP_SERVER_RESPONSE_TIMEOUT_SECS"
        );
    }
}

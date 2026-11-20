//! hub 전용 `codex app-server` 하나와의 JSON-RPC 세션이다.
//!
//! `akra mcp` 명령 하나가 세션 하나를 열고, 끝나면 child 를 정리한다. 줄 단위 JSON 을 주고받고,
//! app-server 가 먼저 보내는 요청(elicitation 등)은 거절해서 호출이 멈추지 않게 한다.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::subprocess::{self, ManagedChild};
use crate::trusted_executable::TrustedCommand;

const STDERR_TAIL_LINES: usize = 20;

const MAX_STDOUT_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_STDERR_LINE_BYTES: usize = 16 * 1024;
const MAX_PENDING_STDOUT_LINES: usize = 8;

type HubOutputEvent = std::result::Result<String, String>;

// Bound each line before allocating, including lines with no newline delimiter.
fn read_bounded_line(reader: &mut impl BufRead, limit: usize) -> io::Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            break;
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let length = newline.map_or(buffer.len(), |position| position + 1);
        if length > limit.saturating_sub(line.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("app-server output line exceeded {limit} bytes"),
            ));
        }
        line.extend_from_slice(&buffer[..length]);
        reader.consume(length);
        if newline.is_some() {
            break;
        }
    }
    if line.is_empty() {
        return Ok(None);
    }
    if line.last() == Some(&b'\n') {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 app-server output"))
}

fn report_transport_error(
    sender: &SyncSender<HubOutputEvent>,
    failure: &Arc<Mutex<Option<String>>>,
    message: String,
) {
    if let Ok(mut first_failure) = failure.lock()
        && first_failure.is_none()
    {
        *first_failure = Some(message.clone());
    }
    // An error event wakes a blocked receiver. A full queue leaves the shared failure
    // readable before any previously queued success can be returned.
    let _ = sender.try_send(Err(message));
}

fn pump_stdout(
    stdout: impl Read,
    sender: SyncSender<HubOutputEvent>,
    failure: Arc<Mutex<Option<String>>>,
    line_limit: usize,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        match read_bounded_line(&mut reader, line_limit) {
            Ok(Some(line)) => {
                // A bounded queue applies pipe backpressure during idle notification bursts.
                // Dropping the session receiver releases a producer blocked in send.
                if sender.send(Ok(line)).is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                report_transport_error(&sender, &failure, format!("stdout: {error}"));
                break;
            }
        }
    }
}

// Diagnostics may be arbitrarily long. Keep a bounded prefix and drain to the
// delimiter so stderr never blocks the child or becomes an RPC transport error.
fn read_truncated_line(reader: &mut impl BufRead, limit: usize) -> io::Result<Option<String>> {
    let mut prefix = Vec::new();
    let mut consumed = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            break;
        }
        consumed = true;
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let length = newline.map_or(buffer.len(), |position| position + 1);
        let retained = length.min(limit.saturating_sub(prefix.len()));
        prefix.extend_from_slice(&buffer[..retained]);
        reader.consume(length);
        if newline.is_some() {
            break;
        }
    }
    if !consumed {
        return Ok(None);
    }
    while prefix
        .last()
        .is_some_and(|byte| matches!(byte, b'\r' | b'\n'))
    {
        prefix.pop();
    }
    let mut line = String::from_utf8_lossy(&prefix).into_owned();
    let mut end = line.len().min(limit);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line.truncate(end);
    Ok(Some(line))
}

fn pump_stderr(stderr: impl Read, tail: Arc<Mutex<Vec<String>>>, line_limit: usize) {
    let mut reader = BufReader::new(stderr);
    while let Ok(Some(line)) = read_truncated_line(&mut reader, line_limit) {
        let Ok(mut tail) = tail.lock() else { break };
        tail.push(line);
        if tail.len() > STDERR_TAIL_LINES {
            tail.remove(0);
        }
    }
}

// A stalled child stdin must not block the calling RPC thread past its deadline.
struct WriteRequest {
    line: Vec<u8>,
    response: SyncSender<io::Result<()>>,
}

fn spawn_stdin_writer(writer: impl Write + Send + 'static) -> SyncSender<WriteRequest> {
    let (sender, receiver) = mpsc::sync_channel::<WriteRequest>(1);
    std::thread::spawn(move || {
        let mut writer = writer;
        for request in receiver {
            let result = writer
                .write_all(&request.line)
                .and_then(|()| writer.flush());
            let failed = result.is_err();
            let _ = request.response.send(result);
            if failed {
                break;
            }
        }
    });
    sender
}

const WRITE_PROGRESS_INTERVAL: Duration = Duration::from_millis(10);
// One initial RPC plus a bounded batch of control replies; the writer channel stays one slot.
const MAX_PENDING_WRITES: usize = MAX_PENDING_STDOUT_LINES + 1;

struct PendingWrite {
    queued: Option<WriteRequest>,
    acknowledgement: Receiver<io::Result<()>>,
}

#[derive(Default)]
struct PendingWrites {
    writes: std::collections::VecDeque<PendingWrite>,
}

impl PendingWrites {
    fn enqueue(
        &mut self,
        writer: &SyncSender<WriteRequest>,
        message: &Value,
        deadline: Instant,
    ) -> Result<()> {
        self.progress(writer, deadline)?;
        if self.writes.len() >= MAX_PENDING_WRITES {
            bail!("MCP hub app-server write queue is full");
        }
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        if Instant::now() >= deadline {
            bail!("MCP hub timed out writing app-server request");
        }
        let (response, acknowledgement) = mpsc::sync_channel(1);
        self.writes.push_back(PendingWrite {
            queued: Some(WriteRequest { line, response }),
            acknowledgement,
        });
        self.progress(writer, deadline)
    }

    fn progress(&mut self, writer: &SyncSender<WriteRequest>, deadline: Instant) -> Result<()> {
        if Instant::now() >= deadline {
            bail!("MCP hub timed out writing app-server request");
        }
        let mut index = 0;
        while index < self.writes.len() {
            let pending = &self.writes[index];
            if pending.queued.is_some() {
                index += 1;
                continue;
            }
            match pending.acknowledgement.try_recv() {
                Ok(Ok(())) => {
                    self.writes.remove(index);
                }
                Ok(Err(error)) => bail!("MCP hub app-server write failed: {error}"),
                Err(mpsc::TryRecvError::Empty) => index += 1,
                Err(mpsc::TryRecvError::Disconnected) => {
                    bail!("MCP hub app-server writer stopped");
                }
            }
        }
        for pending in &mut self.writes {
            if let Some(request) = pending.queued.take() {
                match writer.try_send(request) {
                    Ok(()) => {}
                    Err(TrySendError::Full(request)) => {
                        // Keep the bounded control replies even if the writer has not
                        // dequeued the initial request yet. Continue draining stdout.
                        pending.queued = Some(request);
                        break;
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        bail!("MCP hub app-server writer stopped");
                    }
                }
            }
        }
        Ok(())
    }

    fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }

    fn finish(&mut self, writer: &SyncSender<WriteRequest>, deadline: Instant) -> Result<()> {
        loop {
            self.progress(writer, deadline)?;
            if Instant::now() >= deadline {
                bail!("MCP hub timed out writing app-server request");
            }
            if self.is_empty() {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let wait = remaining.min(WRITE_PROGRESS_INTERVAL);
            if let Some(index) = self
                .writes
                .iter()
                .position(|pending| pending.queued.is_none())
            {
                match self.writes[index].acknowledgement.recv_timeout(wait) {
                    Ok(Ok(())) => {
                        self.writes.remove(index);
                    }
                    Ok(Err(error)) => bail!("MCP hub app-server write failed: {error}"),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        bail!("MCP hub app-server writer stopped");
                    }
                }
            } else {
                std::thread::sleep(wait);
            }
        }
    }
}

fn write_before_deadline(
    writer: &SyncSender<WriteRequest>,
    message: &Value,
    deadline: Instant,
) -> Result<()> {
    let mut pending = PendingWrites::default();
    pending.enqueue(writer, message, deadline)?;
    pending.finish(writer, deadline)
}

#[derive(Debug)]
enum HubMessage {
    Json(Value),
    Idle,
    Closed,
}

fn recv_json_message_until(
    lines: &Receiver<HubOutputEvent>,
    deadline: Instant,
    waiting_for: &str,
    poll_writes: bool,
    check_transport: impl Fn() -> Result<()>,
    stderr_context: impl Fn() -> String,
) -> Result<HubMessage> {
    check_transport()?;
    if Instant::now() >= deadline {
        bail!("MCP hub timed out waiting for {waiting_for}");
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    let wait = if poll_writes {
        remaining.min(WRITE_PROGRESS_INTERVAL)
    } else {
        remaining
    };
    match lines.recv_timeout(wait) {
        Ok(Ok(line)) => {
            check_transport()?;
            let message = serde_json::from_str::<Value>(&line);
            if Instant::now() >= deadline {
                bail!("MCP hub timed out waiting for {waiting_for}");
            }
            Ok(message.map_or(HubMessage::Idle, HubMessage::Json))
        }
        Ok(Err(error)) => {
            bail!(
                "MCP hub app-server transport error: {error}{}",
                stderr_context()
            );
        }
        Err(RecvTimeoutError::Timeout) => {
            check_transport()?;
            if poll_writes && Instant::now() < deadline {
                Ok(HubMessage::Idle)
            } else {
                bail!("MCP hub timed out waiting for {waiting_for}");
            }
        }
        Err(RecvTimeoutError::Disconnected) => {
            check_transport()?;
            Ok(HubMessage::Closed)
        }
    }
}

pub(super) struct HubAppServerSession {
    // child is last so session teardown kills an unresponsive writer after dropping sender.
    writer: SyncSender<WriteRequest>,
    lines: Receiver<HubOutputEvent>,
    transport_failure: Arc<Mutex<Option<String>>>,
    stderr_tail: Arc<Mutex<Vec<String>>>,
    next_id: u64,
    _child: ManagedChild,
}

impl HubAppServerSession {
    pub(super) fn open(codex: &TrustedCommand, codex_home: &Path) -> Result<Self> {
        let config_path = codex_home.join(super::CONFIG_FILE);
        let document = match std::fs::read_to_string(&config_path) {
            Ok(text) => text.parse::<toml_edit::DocumentMut>().with_context(|| {
                format!("failed to parse MCP hub config `{}`", config_path.display())
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                toml_edit::DocumentMut::new()
            }
            Err(error) => return Err(error).context("failed to read MCP hub configuration"),
        };
        let selected_environment = super::catalog::referenced_environment_variables(&document);
        let mut command = Command::new(&codex.program);
        // The hub must not inherit arbitrary parent credentials while regular app-server
        // processes use the scrubbed policy. Explicit AKRA_APP_SERVER_PROCESS_ENVIRONMENT=all
        // retains the same opt-in semantics as ordinary app-server sessions.
        crate::adapter::outbound::app_server::connection::configure_hub_app_server_process_environment(
            &mut command,
            selected_environment,
        );
        command
            .args(&codex.prefix_args)
            .arg("app-server")
            .env("CODEX_HOME", codex_home)
            .current_dir(codex_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = subprocess::spawn(&mut command).with_context(|| {
            format!(
                "failed to start `{} app-server` for the MCP hub",
                codex.program.display()
            )
        })?;
        let stdin = child.take_stdin().context("MCP hub app-server stdin")?;
        let writer = spawn_stdin_writer(stdin);
        let stdout = child.take_stdout().context("MCP hub app-server stdout")?;
        let stderr = child.take_stderr().context("MCP hub app-server stderr")?;

        let (sender, lines) = mpsc::sync_channel(MAX_PENDING_STDOUT_LINES);
        let transport_failure = Arc::new(Mutex::new(None));
        let stdout_failure = transport_failure.clone();
        std::thread::spawn(move || {
            // stdout owns the sole sender: EOF disconnects after queued complete lines,
            // independently of a still-open stderr pipe.
            pump_stdout(stdout, sender, stdout_failure, MAX_STDOUT_LINE_BYTES);
        });
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        let tail = stderr_tail.clone();
        std::thread::spawn(move || {
            pump_stderr(stderr, tail, MAX_STDERR_LINE_BYTES);
        });

        let mut session = Self {
            writer,
            lines,
            transport_failure,
            stderr_tail,
            next_id: 1,
            _child: child,
        };
        session.request(
            "initialize",
            json!({
                "clientInfo": { "name": "akra-mcp-hub", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": false }
            }),
            Duration::from_secs(30),
        )?;
        session.send_until(
            &json!({ "method": "initialized" }),
            Instant::now() + Duration::from_secs(30),
        )?;
        Ok(session)
    }

    pub(super) fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        self.request_observing_notifications(method, params, timeout, |_| {})
    }

    /// Capture the first expected completion while RPC writes are still draining.
    /// The caller can consume it after handling the response, or continue waiting.
    pub(super) fn request_with_notification(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        notification_method: &str,
        accept: impl Fn(&Value) -> bool,
    ) -> Result<(Value, Option<Value>)> {
        let mut completed = None;
        let response =
            self.request_observing_notifications(method, params, timeout, |message| {
                let params = message.get("params").unwrap_or(&Value::Null);
                if completed.is_none()
                    && message.get("method").and_then(Value::as_str) == Some(notification_method)
                    && message.get("id").is_none()
                    && accept(params)
                {
                    completed = Some(params.clone());
                }
            })?;
        Ok((response, completed))
    }

    fn request_observing_notifications(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        observe_notification: impl FnMut(&Value),
    ) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let deadline = Instant::now() + timeout;
        let mut pending = PendingWrites::default();
        self.queue_message(
            &mut pending,
            &json!({ "id": id, "method": method, "params": params }),
            deadline,
        )?;
        let message = self.receive_matching(
            &mut pending,
            deadline,
            method,
            |message| {
                message.get("id").and_then(Value::as_u64) == Some(id)
                    && message.get("method").is_none()
            },
            observe_notification,
        )?;
        if let Some(error) = message.get("error") {
            let detail = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            bail!("`{method}` failed: {detail}");
        }
        Ok(message.get("result").cloned().unwrap_or(Value::Null))
    }

    /// `method` 알림 중 `accept` 가 받아들이는 첫 알림의 params 를 돌려준다.
    pub(super) fn wait_for_notification(
        &mut self,
        method: &str,
        timeout: Duration,
        accept: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        let deadline = Instant::now() + timeout;
        let message = self.receive_matching(
            &mut PendingWrites::default(),
            deadline,
            method,
            |message| {
                message.get("method").and_then(Value::as_str) == Some(method)
                    && message.get("id").is_none()
                    && accept(message.get("params").unwrap_or(&Value::Null))
            },
            |_| {},
        )?;
        Ok(message.get("params").cloned().unwrap_or(Value::Null))
    }

    fn receive_matching(
        &mut self,
        pending: &mut PendingWrites,
        deadline: Instant,
        waiting_for: &str,
        accept: impl Fn(&Value) -> bool,
        mut observe_notification: impl FnMut(&Value),
    ) -> Result<Value> {
        // Preserve at most one accepted response while reverse replies finish.
        // Waiting for their ACK must not stop stdout drain and recreate pipe deadlock.
        let mut accepted = None;
        loop {
            self.check_transport_failure()?;
            pending.progress(&self.writer, deadline)?;
            if Instant::now() >= deadline {
                bail!("MCP hub timed out waiting for {waiting_for}");
            }
            if pending.is_empty()
                && let Some(message) = accepted.take()
            {
                return Ok(message);
            }
            match self.next_message(deadline, waiting_for, !pending.is_empty())? {
                HubMessage::Json(message) => {
                    if message.get("id").is_none() {
                        observe_notification(&message);
                    }
                    if accepted.is_none() && accept(&message) {
                        accepted = Some(message);
                    } else {
                        self.queue_server_response(pending, &message, deadline)?;
                    }
                }
                HubMessage::Idle => {}
                HubMessage::Closed => {
                    if let Some(message) = accepted {
                        // All complete stdout messages precede EOF. There is no
                        // remaining output to drain while finishing these receipts.
                        pending.finish(&self.writer, deadline)?;
                        return Ok(message);
                    }
                    bail!(
                        "MCP hub app-server exited while waiting for {waiting_for}{}",
                        self.stderr_context()
                    );
                }
            }
        }
    }

    fn next_message(
        &mut self,
        deadline: Instant,
        waiting_for: &str,
        poll_writes: bool,
    ) -> Result<HubMessage> {
        recv_json_message_until(
            &self.lines,
            deadline,
            waiting_for,
            poll_writes,
            || self.check_transport_failure(),
            || self.stderr_context(),
        )
    }

    fn check_transport_failure(&self) -> Result<()> {
        let failure = self
            .transport_failure
            .lock()
            .ok()
            .and_then(|failure| failure.clone());
        if let Some(failure) = failure {
            bail!(
                "MCP hub app-server transport error: {failure}{}",
                self.stderr_context()
            );
        }
        Ok(())
    }

    /// app-server 가 클라이언트에게 묻는 요청이다. hub 에는 답할 화면이 없으므로 거절한다.
    fn queue_server_response(
        &mut self,
        pending: &mut PendingWrites,
        message: &Value,
        deadline: Instant,
    ) -> Result<()> {
        let (Some(id), Some(method)) = (
            message.get("id").cloned(),
            message.get("method").and_then(Value::as_str),
        ) else {
            return Ok(());
        };
        let reply = if method == "mcpServer/elicitation/request" {
            json!({ "id": id, "result": { "action": "decline" } })
        } else {
            json!({ "id": id, "error": { "code": -32601, "message": "the Akra MCP hub cannot answer this request" } })
        };
        self.queue_message(pending, &reply, deadline)
    }

    fn queue_message(
        &mut self,
        pending: &mut PendingWrites,
        message: &Value,
        deadline: Instant,
    ) -> Result<()> {
        self.check_transport_failure()?;
        pending
            .enqueue(&self.writer, message, deadline)
            .with_context(|| format!("MCP hub app-server transport{}", self.stderr_context()))
    }

    fn send_until(&mut self, message: &Value, deadline: Instant) -> Result<()> {
        self.check_transport_failure()?;
        write_before_deadline(&self.writer, message, deadline)
            .with_context(|| format!("MCP hub app-server transport{}", self.stderr_context()))
    }

    fn stderr_context(&self) -> String {
        let tail = self
            .stderr_tail
            .lock()
            .map(|tail| tail.join("\n"))
            .unwrap_or_default();
        if tail.trim().is_empty() {
            String::new()
        } else {
            format!("\n{tail}")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Instant;

    use serde_json::json;

    use super::{recv_json_message_until, write_before_deadline};

    #[test]
    fn expired_deadline_wins_over_queued_unrelated_notifications() {
        let (sender, lines) = mpsc::channel();
        for _ in 0..100 {
            sender
                .send(Ok(r#"{"method":"notification","params":{}}"#.to_owned()))
                .unwrap();
        }
        let error = recv_json_message_until(
            &lines,
            Instant::now(),
            "target-notification",
            false,
            || Ok(()),
            String::new,
        )
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        // The unread queue proves an already-expired deadline is not extended by messages.
        assert!(lines.try_recv().is_ok());
    }

    #[test]
    fn expired_write_deadline_does_not_enqueue_work() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let error =
            write_before_deadline(&sender, &json!({"payload":"test"}), Instant::now()).unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(receiver.try_recv().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn child_pipe_backpressure_cannot_extend_the_request_deadline() {
        use std::process::{Command, Stdio};
        use std::time::Duration;

        use super::spawn_stdin_writer;
        let mut child = Command::new("sleep")
            .arg("5")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let writer = spawn_stdin_writer(child.stdin.take().unwrap());
        let deadline = Instant::now() + Duration::from_millis(150);
        let result = write_before_deadline(
            &writer,
            &json!({"payload":"x".repeat(2 * 1024 * 1024)}),
            deadline,
        );
        let _ = child.kill();
        let _ = child.wait();
        let error = result.unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error:?}");
    }
}

#[cfg(test)]
#[path = "session_environment_tests.rs"]
mod environment_tests;

#[cfg(test)]
#[path = "session_deadline_tests.rs"]
mod deadline_tests;

#[cfg(test)]
mod output_tests {
    use std::io::{BufReader, Cursor};
    use std::sync::{Arc, Mutex, mpsc};

    use super::{pump_stderr, pump_stdout, read_bounded_line};

    #[test]
    fn bounded_line_reader_rejects_oversized_unterminated_output() {
        let mut reader = BufReader::new(Cursor::new(vec![b'x'; 1024]));
        let error = read_bounded_line(&mut reader, 16).unwrap_err();
        assert!(error.to_string().contains("16 bytes"));
    }

    #[test]
    fn oversized_stdout_is_reported_as_a_transport_failure() {
        let (sender, receiver) = mpsc::sync_channel(2);
        let failure = Arc::new(Mutex::new(None));
        pump_stdout(Cursor::new(vec![b'x'; 1024]), sender, failure.clone(), 16);
        assert!(receiver.try_recv().unwrap().unwrap_err().contains("stdout"));
        assert!(
            failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("16 bytes")
        );
    }

    #[test]
    fn notification_bursts_apply_backpressure_without_failing_transport() {
        let (sender, receiver) = mpsc::sync_channel(2);
        let failure = Arc::new(Mutex::new(None));
        let producer_failure = failure.clone();
        let producer = std::thread::spawn(move || {
            pump_stdout(
                Cursor::new(b"one\ntwo\nthree\nfour\n".to_vec()),
                sender,
                producer_failure,
                16,
            );
        });
        for expected in ["one", "two", "three", "four"] {
            assert_eq!(
                receiver
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap()
                    .unwrap(),
                expected
            );
        }
        producer.join().unwrap();
        assert!(failure.lock().unwrap().is_none());
        assert!(receiver.recv().is_err());
    }

    #[test]
    fn dropping_receiver_releases_backpressured_stdout_producer() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (finished, completion) = mpsc::sync_channel(1);
        let producer = std::thread::spawn(move || {
            pump_stdout(
                Cursor::new(b"one\ntwo\nthree\n".to_vec()),
                sender,
                Arc::new(Mutex::new(None)),
                16,
            );
            finished.send(()).unwrap();
        });
        assert_eq!(receiver.recv().unwrap().unwrap(), "one");
        drop(receiver);
        completion
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        producer.join().unwrap();
    }

    #[test]
    fn oversized_stderr_keeps_a_bounded_prefix_and_drains_subsequent_lines() {
        let tail = Arc::new(Mutex::new(Vec::new()));
        let mut bytes = vec![b'e'; 1024 * 1024];
        bytes.extend_from_slice(b"\nnext diagnostic\n");
        pump_stderr(Cursor::new(bytes), tail.clone(), 16);
        let tail = tail.lock().unwrap();
        assert_eq!(*tail, vec!["e".repeat(16), "next diagnostic".to_owned()]);
    }

    #[test]
    fn stdout_eof_delivers_final_line_without_waiting_for_stderr() {
        struct BlockedStderr(mpsc::Receiver<()>);
        impl std::io::Read for BlockedStderr {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                let _ = self.0.recv();
                Ok(0)
            }
        }
        let (release, blocked) = mpsc::channel();
        let (finished, stderr_done) = mpsc::channel();
        let stderr = std::thread::spawn(move || {
            pump_stderr(BlockedStderr(blocked), Arc::new(Mutex::new(Vec::new())), 16);
            let _ = finished.send(());
        });
        let (sender, receiver) = mpsc::sync_channel(2);
        let stdout = std::thread::spawn(move || {
            pump_stdout(
                Cursor::new(b"last response\n".to_vec()),
                sender,
                Arc::new(Mutex::new(None)),
                32,
            );
        });
        assert_eq!(receiver.recv().unwrap().unwrap(), "last response");
        assert!(matches!(
            receiver.recv_timeout(std::time::Duration::from_secs(1)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
        assert!(stderr_done.try_recv().is_err(), "stderr must still be open");
        drop(release);
        stdout.join().unwrap();
        stderr.join().unwrap();
    }

    #[test]
    fn bounded_line_reader_accepts_newline_and_final_line() {
        let mut reader = BufReader::new(Cursor::new(b"first\r\nlast".to_vec()));
        assert_eq!(
            read_bounded_line(&mut reader, 16).unwrap().unwrap(),
            "first"
        );
        assert_eq!(read_bounded_line(&mut reader, 16).unwrap().unwrap(), "last");
        assert!(read_bounded_line(&mut reader, 16).unwrap().is_none());
    }
}

#[cfg(test)]
#[path = "session_transport_tests.rs"]
mod transport_tests;

#[cfg(test)]
#[path = "session_duplex_tests.rs"]
mod duplex_tests;

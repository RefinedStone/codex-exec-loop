//! hub 전용 `codex app-server` 하나와의 JSON-RPC 세션이다.
//!
//! `akra mcp` 명령 하나가 세션 하나를 열고, 끝나면 child 를 정리한다. 줄 단위 JSON 을 주고받고,
//! app-server 가 먼저 보내는 요청(elicitation 등)은 거절해서 호출이 멈추지 않게 한다.

use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::subprocess::{self, ManagedChild};
use crate::trusted_executable::TrustedCommand;

const STDERR_TAIL_LINES: usize = 20;

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

fn write_before_deadline(
    writer: &SyncSender<WriteRequest>,
    message: &Value,
    deadline: Instant,
) -> Result<()> {
    if Instant::now() >= deadline {
        bail!("MCP hub timed out writing app-server request");
    }
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    let (response_sender, response_receiver) = mpsc::sync_channel(1);
    if Instant::now() >= deadline {
        bail!("MCP hub timed out writing app-server request");
    }
    match writer.try_send(WriteRequest {
        line,
        response: response_sender,
    }) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => bail!("MCP hub app-server write queue is full"),
        Err(TrySendError::Disconnected(_)) => bail!("MCP hub app-server writer stopped"),
    }
    match response_receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Ok(())) if Instant::now() < deadline => Ok(()),
        Ok(Ok(())) | Err(RecvTimeoutError::Timeout) => {
            bail!("MCP hub timed out writing app-server request")
        }
        Ok(Err(error)) => Err(anyhow!("MCP hub app-server write failed: {error}")),
        Err(RecvTimeoutError::Disconnected) => {
            bail!("MCP hub app-server writer stopped")
        }
    }
}

fn recv_json_message_until(
    lines: &Receiver<String>,
    deadline: Instant,
    waiting_for: &str,
    stderr_context: impl Fn() -> String,
) -> Result<Value> {
    loop {
        // Checking before recv is essential: a continuously non-empty queue can otherwise
        // make recv_timeout(0) succeed forever and starve an expired deadline.
        if Instant::now() >= deadline {
            bail!("MCP hub timed out waiting for {waiting_for}");
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        match lines.recv_timeout(remaining) {
            Ok(line) => {
                if Instant::now() >= deadline {
                    bail!("MCP hub timed out waiting for {waiting_for}");
                }
                if let Ok(message) = serde_json::from_str::<Value>(&line) {
                    return Ok(message);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                bail!("MCP hub timed out waiting for {waiting_for}")
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(anyhow!(
                    "MCP hub app-server exited while waiting for {waiting_for}{}",
                    stderr_context()
                ));
            }
        }
    }
}

pub(super) struct HubAppServerSession {
    // child is last so session teardown kills an unresponsive writer after dropping sender.
    writer: SyncSender<WriteRequest>,
    lines: Receiver<String>,
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

        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        let tail = stderr_tail.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let Ok(mut tail) = tail.lock() else { break };
                tail.push(line);
                if tail.len() > STDERR_TAIL_LINES {
                    tail.remove(0);
                }
            }
        });

        let mut session = Self {
            writer,
            lines,
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
        let id = self.next_id;
        self.next_id += 1;
        let deadline = Instant::now() + timeout;
        self.send_until(
            &json!({ "id": id, "method": method, "params": params }),
            deadline,
        )?;
        loop {
            let message = self.next_message(deadline, method)?;
            if message.get("id").and_then(Value::as_u64) == Some(id)
                && message.get("method").is_none()
            {
                if let Some(error) = message.get("error") {
                    let detail = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error");
                    bail!("`{method}` failed: {detail}")
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
            self.answer_server_request(&message, deadline)?;
        }
    }

    /// `method` 알림 중 `accept` 가 받아들이는 첫 알림의 params 를 돌려준다.
    pub(super) fn wait_for_notification(
        &mut self,
        method: &str,
        timeout: Duration,
        accept: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        let deadline = Instant::now() + timeout;
        loop {
            let message = self.next_message(deadline, method)?;
            if message.get("method").and_then(Value::as_str) == Some(method)
                && message.get("id").is_none()
            {
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                if accept(&params) {
                    return Ok(params);
                }
                continue;
            }
            self.answer_server_request(&message, deadline)?;
        }
    }

    fn next_message(&mut self, deadline: Instant, waiting_for: &str) -> Result<Value> {
        recv_json_message_until(&self.lines, deadline, waiting_for, || self.stderr_context())
    }

    /// app-server 가 클라이언트에게 묻는 요청이다. hub 에는 답할 화면이 없으므로 거절한다.
    fn answer_server_request(&mut self, message: &Value, deadline: Instant) -> Result<()> {
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
        self.send_until(&reply, deadline)
    }

    fn send_until(&mut self, message: &Value, deadline: Instant) -> Result<()> {
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
                .send(r#"{"method":"notification","params":{}}"#.to_owned())
                .unwrap();
        }
        let error =
            recv_json_message_until(&lines, Instant::now(), "target-notification", String::new)
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

//! hub 전용 `codex app-server` 하나와의 JSON-RPC 세션이다.
//!
//! `akra mcp` 명령 하나가 세션 하나를 열고, 끝나면 child 를 정리한다. 줄 단위 JSON 을 주고받고,
//! app-server 가 먼저 보내는 요청(elicitation 등)은 거절해서 호출이 멈추지 않게 한다.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::subprocess::{self, ManagedChild};
use crate::trusted_executable::TrustedCommand;

const STDERR_TAIL_LINES: usize = 20;

pub(super) struct HubAppServerSession {
    // child 는 drop 될 때 process tree 를 정리한다. stdin 보다 늦게 drop 되도록 마지막 필드로 둔다.
    stdin: ChildStdin,
    lines: Receiver<String>,
    stderr_tail: Arc<Mutex<Vec<String>>>,
    next_id: u64,
    _child: ManagedChild,
}

impl HubAppServerSession {
    pub(super) fn open(codex: &TrustedCommand, codex_home: &Path) -> Result<Self> {
        let mut command = Command::new(&codex.program);
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
            stdin,
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
        session.send(&json!({ "method": "initialized" }))?;
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
        self.send(&json!({ "id": id, "method": method, "params": params }))?;
        let deadline = Instant::now() + timeout;
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
            self.answer_server_request(&message)?;
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
            self.answer_server_request(&message)?;
        }
    }

    fn next_message(&mut self, deadline: Instant, waiting_for: &str) -> Result<Value> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(remaining) {
                Ok(line) => {
                    if let Ok(message) = serde_json::from_str::<Value>(&line) {
                        return Ok(message);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    bail!("MCP hub timed out waiting for `{waiting_for}`")
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(anyhow!(
                        "MCP hub app-server exited while waiting for `{waiting_for}`{}",
                        self.stderr_context()
                    ));
                }
            }
        }
    }

    /// app-server 가 클라이언트에게 묻는 요청이다. hub 에는 답할 화면이 없으므로 거절한다.
    fn answer_server_request(&mut self, message: &Value) -> Result<()> {
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
        self.send(&reply)
    }

    fn send(&mut self, message: &Value) -> Result<()> {
        let mut line = serde_json::to_string(message)?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|()| self.stdin.flush())
            .with_context(|| format!("MCP hub app-server stopped{}", self.stderr_context()))
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

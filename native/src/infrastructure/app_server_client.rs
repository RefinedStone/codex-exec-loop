use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const APP_SERVER_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct AppServerClient {
    client_name: String,
    client_version: String,
}

impl AppServerClient {
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Self {
            client_name: client_name.into(),
            client_version: client_version.into(),
        }
    }

    pub fn open_connection(&self) -> Result<AppServerConnection> {
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
}

pub struct AppServerConnection {
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
    pub fn initialize(&mut self) -> Result<InitializeResponse> {
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

    pub fn read_account(&mut self) -> Result<AccountReadResponse> {
        self.ensure_initialized()?;
        self.send_request("account/read", json!({}))
    }

    pub fn list_threads(&mut self, params: ThreadListParams) -> Result<ThreadListResponse> {
        self.ensure_initialized()?;
        self.send_request("thread/list", serde_json::to_value(params)?)
    }

    pub fn finish(mut self) -> Vec<String> {
        self.collect_remaining_warnings();
        self.warnings.sort();
        self.warnings.dedup();
        self.warnings.clone()
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
pub struct InitializeResponse {
    pub user_agent: String,
    pub codex_home: String,
    pub platform_family: String,
    pub platform_os: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountReadResponse {
    pub account: Option<AccountRecord>,
    pub requires_openai_auth: Option<bool>,
}

impl AccountReadResponse {
    pub fn is_authenticated(&self) -> bool {
        self.account.is_some() || !self.requires_openai_auth.unwrap_or(false)
    }

    pub fn to_summary_text(&self) -> String {
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
pub struct AccountRecord {
    #[serde(rename = "type")]
    pub account_type: String,
    pub email: Option<String>,
    pub plan_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_term: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_kinds: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListResponse {
    pub data: Vec<ThreadRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRecord {
    pub id: String,
    pub name: Option<String>,
    pub preview: String,
    pub cwd: String,
    pub source: String,
    pub model_provider: String,
    pub updated_at: i64,
    pub path: String,
    pub status: ThreadStatus,
    pub git_info: Option<ThreadGitInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThreadStatus {
    #[serde(rename = "type")]
    pub status_type: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGitInfo {
    pub branch: Option<String>,
}

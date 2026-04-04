use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

const APP_SERVER_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct StartupDiagnostics {
    pub cwd: String,
    pub codex_binary_ok: bool,
    pub codex_binary_detail: String,
    pub workspace_ok: bool,
    pub workspace_detail: String,
    pub initialize_ok: bool,
    pub initialize_detail: String,
    pub account_ok: bool,
    pub account_detail: String,
    pub warnings: Vec<String>,
    pub schema_snapshot: String,
}

impl StartupDiagnostics {
    pub fn can_continue(&self) -> bool {
        self.codex_binary_ok && self.workspace_ok && self.initialize_ok && self.account_ok
    }
}

enum AppServerLine {
    Stdout(String),
    Stderr(String),
}

pub fn run_startup_probe() -> Result<StartupDiagnostics> {
    let cwd = std::env::current_dir()
        .context("failed to resolve current directory")?
        .display()
        .to_string();

    let codex_path = which::which("codex").context("`codex` was not found on PATH")?;
    let workspace_detail = detect_workspace_status();
    let workspace_ok =
        workspace_detail.starts_with("git repo") || workspace_detail.starts_with("directory");

    let (initialize_detail, account_detail, warnings) = probe_app_server()?;

    Ok(StartupDiagnostics {
        cwd,
        codex_binary_ok: true,
        codex_binary_detail: codex_path.display().to_string(),
        workspace_ok,
        workspace_detail,
        initialize_ok: true,
        initialize_detail,
        account_ok: !account_detail.starts_with("not logged in"),
        account_detail,
        warnings,
        schema_snapshot: "native/schema/codex_app_server_protocol.v2.schemas.json".to_string(),
    })
}

fn detect_workspace_status() -> String {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    match output {
        Ok(result) if result.status.success() => {
            let root = String::from_utf8_lossy(&result.stdout).trim().to_string();
            format!("git repo: {root}")
        }
        _ => "directory only (not inside a git repo)".to_string(),
    }
}

fn probe_app_server() -> Result<(String, String, Vec<String>)> {
    let mut child = Command::new("codex")
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn `codex app-server`")?;

    let mut stdin = child
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

    send_json_line(
        &mut stdin,
        json!({
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "codex-exec-loop-native",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": false
                }
            }
        }),
    )?;

    let init_result = wait_for_response(&rx, &mut child, 1)?;
    send_json_line(&mut stdin, json!({ "method": "initialized", "params": {} }))?;
    send_json_line(
        &mut stdin,
        json!({
            "id": 2,
            "method": "account/read",
            "params": {}
        }),
    )?;
    let account_result = wait_for_response(&rx, &mut child, 2)?;

    let mut warnings = collect_remaining_warnings(&rx, &mut child);
    terminate_child(&mut child);

    let initialize_detail = format!(
        "{} / {} / {}",
        init_result
            .get("platformOs")
            .and_then(Value::as_str)
            .unwrap_or("unknown-os"),
        init_result
            .get("platformFamily")
            .and_then(Value::as_str)
            .unwrap_or("unknown-family"),
        init_result
            .get("userAgent")
            .and_then(Value::as_str)
            .unwrap_or("unknown-user-agent"),
    );

    let account_detail = match account_result.get("account") {
        Some(Value::Object(account)) => match account.get("type").and_then(Value::as_str) {
            Some("chatgpt") => format!(
                "chatgpt / {} / {}",
                account
                    .get("email")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown-email"),
                account
                    .get("planType")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown-plan")
            ),
            Some("apiKey") => "api key account".to_string(),
            Some(other) => format!("account type: {other}"),
            None => "account present".to_string(),
        },
        _ => {
            let requires_auth = account_result
                .get("requiresOpenaiAuth")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if requires_auth {
                "not logged in (OpenAI auth required)".to_string()
            } else {
                "no account configured".to_string()
            }
        }
    };

    warnings.sort();
    warnings.dedup();
    Ok((initialize_detail, account_detail, warnings))
}

fn spawn_pipe_reader<T: std::io::Read + Send + 'static>(
    pipe: T,
    tx: mpsc::Sender<AppServerLine>,
    is_stderr: bool,
) {
    thread::spawn(move || {
        let reader = BufReader::new(pipe);
        for line in reader.lines().map_while(Result::ok) {
            let payload = if is_stderr {
                AppServerLine::Stderr(line)
            } else {
                AppServerLine::Stdout(line)
            };
            let _ = tx.send(payload);
        }
    });
}

fn send_json_line(stdin: &mut ChildStdin, value: Value) -> Result<()> {
    writeln!(stdin, "{}", serde_json::to_string(&value)?)?;
    stdin.flush()?;
    Ok(())
}

fn wait_for_response(rx: &Receiver<AppServerLine>, child: &mut Child, id: i64) -> Result<Value> {
    let deadline = Instant::now() + APP_SERVER_TIMEOUT;
    let mut warnings = Vec::new();
    loop {
        if Instant::now() > deadline {
            bail!("timed out waiting for app-server response id={id}");
        }
        if let Some(status) = child.try_wait()? {
            bail!("app-server exited early with status {status}");
        }

        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(AppServerLine::Stderr(line)) => warnings.push(line),
            Ok(AppServerLine::Stdout(line)) => {
                let value: Value = serde_json::from_str(&line)
                    .with_context(|| format!("invalid JSON from app-server: {line}"))?;

                if let Some(method) = value.get("method").and_then(Value::as_str)
                    && method == "configWarning"
                {
                    if let Some(summary) = value
                        .get("params")
                        .and_then(|params| params.get("summary"))
                        .and_then(Value::as_str)
                    {
                        warnings.push(summary.to_string());
                    }
                    continue;
                }

                if value.get("id").and_then(Value::as_i64) == Some(id) {
                    if let Some(error) = value.get("error") {
                        return Err(anyhow!("app-server returned error for id {id}: {error}"));
                    }
                    if let Some(result) = value.get("result") {
                        if !warnings.is_empty() {
                            return Ok(json!({
                                "__warnings": warnings,
                                "__result": result
                            })["__result"]
                                .clone());
                        }
                        return Ok(result.clone());
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("app-server pipe closed while waiting for response id={id}");
            }
        }
    }
}

fn collect_remaining_warnings(rx: &Receiver<AppServerLine>, child: &mut Child) -> Vec<String> {
    let mut warnings = Vec::new();
    let drain_deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < drain_deadline {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(AppServerLine::Stderr(line)) => warnings.push(line),
            Ok(AppServerLine::Stdout(line)) => {
                if let Ok(value) = serde_json::from_str::<Value>(&line)
                    && value.get("method").and_then(Value::as_str) == Some("configWarning")
                    && let Some(summary) = value
                        .get("params")
                        .and_then(|params| params.get("summary"))
                        .and_then(Value::as_str)
                {
                    warnings.push(summary.to_string());
                }
            }
            Err(_) => break,
        }
    }
    warnings
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

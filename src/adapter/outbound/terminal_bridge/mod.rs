use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};

use crate::adapter::outbound::app_server::CodexAppServerAdapter;
use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
use crate::application::port::outbound::startup_probe_port::{
    StartupProbeContext, StartupProbePort,
};
use crate::application::service::conversation_runtime_event::{
    emit_attachment_observed, ConversationStreamEvent,
};
use crate::domain::conversation::{
    ConversationControlSupport, ConversationMessage, ConversationMessageKind,
    ConversationRuntimeControlTruth, ConversationSnapshot,
};
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogTier};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

const TERMINAL_BRIDGE_MODE_ENV_VAR: &str = "CODEX_EXEC_LOOP_TERMINAL_BRIDGE_MODE";
const TMUX_TARGET_ENV_VAR: &str = "CODEX_EXEC_LOOP_TMUX_TARGET";
const TMUX_SOCKET_ENV_VAR: &str = "CODEX_EXEC_LOOP_TMUX_SOCKET";
const TMUX_POLL_INTERVAL_MS_ENV_VAR: &str = "CODEX_EXEC_LOOP_TMUX_POLL_INTERVAL_MS";
const TMUX_IDLE_TIMEOUT_MS_ENV_VAR: &str = "CODEX_EXEC_LOOP_TMUX_IDLE_TIMEOUT_MS";
const TMUX_NO_OUTPUT_TIMEOUT_SECS_ENV_VAR: &str = "CODEX_EXEC_LOOP_TMUX_NO_OUTPUT_TIMEOUT_SECS";
const TMUX_DEFAULT_POLL_INTERVAL_MS: u64 = 250;
const TMUX_DEFAULT_IDLE_TIMEOUT_MS: u64 = 3000;
const TMUX_DEFAULT_NO_OUTPUT_TIMEOUT_SECS: u64 = 30;
const TMUX_SCHEMA_SNAPSHOT_LABEL: &str = "not applicable for tmux local attach";
const TMUX_STREAMING_STATUS_PREFIX: &str = "streaming through tmux pane";
const TMUX_MANUAL_HANDOFF_STATUS_PREFIX: &str = "manual operator handoff required in tmux pane";

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TerminalConversationPorts {
    pub startup_probe_port: Arc<dyn StartupProbePort>,
    pub session_catalog_port: Arc<dyn SessionCatalogPort>,
    pub interactive_turn_runtime_port: Arc<dyn InteractiveTurnRuntimePort>,
}

pub fn build_terminal_conversation_ports(
    app_server_adapter: Arc<CodexAppServerAdapter>,
) -> TerminalConversationPorts {
    match TerminalBridgeMode::from_environment() {
        TerminalBridgeMode::CodexAppServer => TerminalConversationPorts {
            startup_probe_port: app_server_adapter.clone(),
            session_catalog_port: app_server_adapter.clone(),
            interactive_turn_runtime_port: app_server_adapter,
        },
        TerminalBridgeMode::TmuxLocalAttach => {
            let adapter = Arc::new(TmuxLocalAttachAdapter::from_environment());
            TerminalConversationPorts {
                startup_probe_port: adapter.clone(),
                session_catalog_port: adapter.clone(),
                interactive_turn_runtime_port: adapter,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalBridgeMode {
    CodexAppServer,
    TmuxLocalAttach,
}

impl TerminalBridgeMode {
    fn from_environment() -> Self {
        Self::from_env_value(std::env::var(TERMINAL_BRIDGE_MODE_ENV_VAR).ok().as_deref())
    }

    fn from_env_value(value: Option<&str>) -> Self {
        let normalized = value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase().replace(['_', ' '], "-"));
        match normalized.as_deref() {
            Some("tmux") | Some("tmux-local-attach") | Some("local-attach") => {
                Self::TmuxLocalAttach
            }
            _ => Self::CodexAppServer,
        }
    }
}

#[derive(Debug, Clone)]
struct TmuxLocalAttachConfig {
    socket: Option<String>,
    target: Option<String>,
    poll_interval: Duration,
    idle_timeout: Duration,
    no_output_timeout: Duration,
}

impl TmuxLocalAttachConfig {
    fn from_environment() -> Self {
        Self {
            socket: normalized_env(TMUX_SOCKET_ENV_VAR),
            target: normalized_env(TMUX_TARGET_ENV_VAR),
            poll_interval: Duration::from_millis(env_u64(
                TMUX_POLL_INTERVAL_MS_ENV_VAR,
                TMUX_DEFAULT_POLL_INTERVAL_MS,
            )),
            idle_timeout: Duration::from_millis(env_u64(
                TMUX_IDLE_TIMEOUT_MS_ENV_VAR,
                TMUX_DEFAULT_IDLE_TIMEOUT_MS,
            )),
            no_output_timeout: Duration::from_secs(env_u64(
                TMUX_NO_OUTPUT_TIMEOUT_SECS_ENV_VAR,
                TMUX_DEFAULT_NO_OUTPUT_TIMEOUT_SECS,
            )),
        }
    }
}

#[derive(Clone)]
pub struct TmuxLocalAttachAdapter {
    config: TmuxLocalAttachConfig,
}

impl TmuxLocalAttachAdapter {
    pub fn from_environment() -> Self {
        Self {
            config: TmuxLocalAttachConfig::from_environment(),
        }
    }

    fn startup_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if which::which("codex").is_err() {
            warnings.push(
                "planning worker still uses codex app-server; planning operations may fail in tmux mode without codex on PATH"
                    .to_string(),
            );
        }
        warnings
    }

    fn resolve_startup_target(&self) -> Result<TmuxPaneTarget> {
        match self.config.target.as_deref() {
            Some(target) => self.resolve_target(target),
            None => self.discover_single_target(),
        }
    }

    fn resolve_turn_target(&self, thread_id: Option<&str>) -> Result<TmuxPaneTarget> {
        match thread_id {
            Some(thread_id) if !thread_id.trim().is_empty() => self.resolve_target(thread_id),
            _ => self.resolve_startup_target(),
        }
    }

    fn discover_single_target(&self) -> Result<TmuxPaneTarget> {
        let panes = self.list_panes(None)?;
        match panes.as_slice() {
            [] => Err(anyhow!(
                "no tmux panes are available; start a tmux server or set {TMUX_TARGET_ENV_VAR} to an explicit pane handle"
            )),
            [target] => Ok(target.clone()),
            _ => Err(anyhow!(
                "multiple tmux panes are available; set {TMUX_TARGET_ENV_VAR} to an explicit pane handle"
            )),
        }
    }

    fn resolve_target(&self, target: &str) -> Result<TmuxPaneTarget> {
        let mut panes = self.list_panes(Some(target))?;
        match panes.len() {
            0 => Err(anyhow!(
                "tmux target `{target}` did not resolve to a live pane"
            )),
            1 => Ok(panes.remove(0)),
            _ => Err(anyhow!(
                "tmux target `{target}` resolved to multiple panes; use a pane id such as `%1`"
            )),
        }
    }

    fn list_panes(&self, target: Option<&str>) -> Result<Vec<TmuxPaneTarget>> {
        let format = "#{pane_id}\t#{pane_current_path}\t#{session_name}:#{window_index}.#{pane_index}\t#{pane_pipe}\t#{pane_current_command}";
        let mut args = vec![
            "list-panes".to_string(),
            "-F".to_string(),
            format.to_string(),
        ];
        if target.is_none() {
            args.push("-a".to_string());
        }
        if let Some(target) = target {
            args.push("-t".to_string());
            args.push(target.to_string());
        }
        let stdout = self.run_tmux(&args)?;
        parse_list_panes_output(&stdout)
    }

    fn capture_transcript(&self, target: &str) -> Result<String> {
        self.run_tmux(&[
            "capture-pane".to_string(),
            "-p".to_string(),
            "-t".to_string(),
            target.to_string(),
        ])
    }

    fn inject_prompt(&self, target: &str, prompt: &str) -> Result<()> {
        let buffer_name = unique_label("codex-exec-loop-tmux-buffer");
        let prompt_path = temp_file_path("codex-exec-loop-tmux-prompt", "txt");
        fs::write(&prompt_path, prompt).with_context(|| {
            format!(
                "failed to write tmux prompt buffer at {}",
                prompt_path.display()
            )
        })?;

        let load_result = self.run_tmux(&[
            "load-buffer".to_string(),
            "-b".to_string(),
            buffer_name.clone(),
            prompt_path.display().to_string(),
        ]);
        let _ = fs::remove_file(&prompt_path);
        load_result?;

        self.run_tmux(&[
            "paste-buffer".to_string(),
            "-d".to_string(),
            "-r".to_string(),
            "-b".to_string(),
            buffer_name,
            "-t".to_string(),
            target.to_string(),
        ])?;
        self.run_tmux(&[
            "send-keys".to_string(),
            "-t".to_string(),
            target.to_string(),
            "C-m".to_string(),
        ])?;
        Ok(())
    }

    fn send_interrupt(&self, target: &str) -> Result<()> {
        self.run_tmux(&[
            "send-keys".to_string(),
            "-t".to_string(),
            target.to_string(),
            "C-c".to_string(),
        ])?;
        Ok(())
    }

    fn enable_pipe_capture(&self, target: &str) -> Result<TmuxPipeCapture> {
        let log_path = temp_file_path("codex-exec-loop-tmux-stream", "log");
        File::create(&log_path)
            .with_context(|| format!("failed to create tmux capture log {}", log_path.display()))?;
        self.run_tmux(&[
            "pipe-pane".to_string(),
            "-t".to_string(),
            target.to_string(),
            format!("cat >> {}", log_path.display()),
        ])?;
        let file = OpenOptions::new()
            .read(true)
            .open(&log_path)
            .with_context(|| format!("failed to open tmux capture log {}", log_path.display()))?;
        Ok(TmuxPipeCapture {
            adapter: self.clone(),
            target: target.to_string(),
            log_path,
            file,
            cursor: 0,
        })
    }

    fn run_tmux<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new("tmux");
        if let Some(socket) = self.config.socket.as_deref() {
            if socket.starts_with('/') {
                command.args(["-S", socket]);
            } else {
                command.args(["-L", socket]);
            }
        }
        command.args(args);
        let output = command
            .output()
            .context("failed to spawn tmux command for local attach")?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).to_string());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(anyhow!(if stderr.is_empty() {
            "tmux command failed without stderr output".to_string()
        } else {
            stderr
        }))
    }

    fn run_stream_on_target(
        &self,
        target: &TmuxPaneTarget,
        prompt: &str,
        event_sender: &std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        if target.pipe_active {
            return Err(anyhow!(
                "tmux pane {} already has an active pipe; disable it before attaching",
                target.pane_id
            ));
        }

        let turn_id = unique_label("tmux-turn");
        let item_id = unique_label("tmux-item");
        let mut capture = self.enable_pipe_capture(&target.pane_id)?;
        let mut expected_echo = expected_prompt_echo(prompt);
        let mut aggregated_output = String::new();
        let mut manual_handoff_active = false;
        let mut saw_output = false;
        let start = Instant::now();
        let mut last_output_change = start;

        let _ = event_sender.send(ConversationStreamEvent::TurnStarted {
            turn_id: turn_id.clone(),
        });
        let _ = event_sender.send(ConversationStreamEvent::StatusUpdated {
            text: streaming_status_text(target),
        });

        self.inject_prompt(&target.pane_id, prompt)?;

        loop {
            let chunk = sanitize_terminal_transcript(&capture.read_new_text()?);
            if !chunk.is_empty() {
                let chunk = strip_prompt_echo(chunk, &mut expected_echo);
                if !chunk.is_empty() {
                    saw_output = true;
                    last_output_change = Instant::now();
                    aggregated_output.push_str(&chunk);
                    let _ = event_sender.send(ConversationStreamEvent::AgentMessageDelta {
                        item_id: item_id.clone(),
                        phase: None,
                        delta: chunk,
                    });
                    sync_manual_handoff_status(
                        &aggregated_output,
                        &mut manual_handoff_active,
                        target,
                        event_sender,
                    );
                }
            }

            if saw_output
                && !manual_handoff_active
                && last_output_change.elapsed() >= self.config.idle_timeout
            {
                break;
            }
            if !saw_output && start.elapsed() >= self.config.no_output_timeout {
                return Err(anyhow!(
                    "tmux local attach observed no transcript output within {}s for pane {}",
                    self.config.no_output_timeout.as_secs(),
                    target.pane_id
                ));
            }

            thread::sleep(self.config.poll_interval);
        }

        let final_chunk = sanitize_terminal_transcript(&capture.read_new_text()?);
        let final_chunk = strip_prompt_echo(final_chunk, &mut expected_echo);
        if !final_chunk.is_empty() {
            aggregated_output.push_str(&final_chunk);
            let _ = event_sender.send(ConversationStreamEvent::AgentMessageDelta {
                item_id: item_id.clone(),
                phase: None,
                delta: final_chunk,
            });
        }

        let final_text = aggregated_output.trim_end().to_string();
        if !final_text.is_empty() {
            let _ = event_sender.send(ConversationStreamEvent::AgentMessageCompleted {
                item_id,
                phase: None,
                text: final_text,
            });
        }
        let _ = event_sender.send(ConversationStreamEvent::TurnCompleted {
            turn_id,
            changed_planning_file_paths: Vec::new(),
        });
        Ok(())
    }
}

impl StartupProbePort for TmuxLocalAttachAdapter {
    fn load_startup_context(&self) -> Result<StartupProbeContext> {
        let warnings = self.startup_warnings();
        let tmux_path = match which::which("tmux") {
            Ok(path) => path,
            Err(error) => {
                return Ok(StartupProbeContext {
                    launch_target_ok: false,
                    launch_target_detail: error.to_string(),
                    readiness_ok: false,
                    attachment_profile: TerminalBridgeAttachmentProfile::tmux_local_attach(),
                    readiness_detail: format!(
                        "install tmux and provide a live pane through {TMUX_TARGET_ENV_VAR}"
                    ),
                    access_detail: "manual operator handoff".to_string(),
                    access_ok: true,
                    schema_snapshot: TMUX_SCHEMA_SNAPSHOT_LABEL.to_string(),
                    warnings,
                });
            }
        };

        let startup_target = self.resolve_startup_target();
        let (readiness_ok, readiness_detail, warnings) = match startup_target {
            Ok(target) if target.pipe_active => (
                false,
                format!(
                    "pane {} ({}) already has an active tmux pipe; disable it before attaching",
                    target.pane_id, target.display_name
                ),
                warnings,
            ),
            Ok(target) => (
                true,
                format!(
                    "pane {} ({}) / cwd {} / command {}",
                    target.pane_id, target.display_name, target.cwd, target.current_command
                ),
                warnings,
            ),
            Err(error) => (false, error.to_string(), warnings),
        };

        Ok(StartupProbeContext {
            launch_target_ok: true,
            launch_target_detail: tmux_path.display().to_string(),
            readiness_ok,
            attachment_profile: TerminalBridgeAttachmentProfile::tmux_local_attach(),
            readiness_detail,
            access_detail: "manual operator handoff".to_string(),
            access_ok: true,
            schema_snapshot: TMUX_SCHEMA_SNAPSHOT_LABEL.to_string(),
            warnings,
        })
    }
}

impl SessionCatalogPort for TmuxLocalAttachAdapter {
    fn load_recent_sessions(&self, _limit: usize) -> Result<SessionCatalog> {
        let detail = match self.resolve_startup_target() {
            Ok(target) => format!(
                "tmux local attach keeps the pane handle {} ({}), but does not expose a queryable session catalog",
                target.pane_id, target.display_name
            ),
            Err(_) => format!(
                "tmux local attach requires an explicit pane handle through {TMUX_TARGET_ENV_VAR} or a single discoverable tmux pane"
            ),
        };
        Ok(SessionCatalog::unsupported(
            SessionCatalogTier::HandleBasedReattach,
            detail,
            self.startup_warnings(),
        ))
    }
}

impl InteractiveTurnRuntimePort for TmuxLocalAttachAdapter {
    fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
        ConversationRuntimeControlTruth::new(
            ConversationControlSupport::ManualHandoff,
            ConversationControlSupport::RuntimeNative,
        )
    }

    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        let target = self.resolve_turn_target(Some(thread_id))?;
        let transcript = sanitize_terminal_transcript(&self.capture_transcript(&target.pane_id)?);
        let messages = if transcript.trim().is_empty() {
            Vec::new()
        } else {
            vec![ConversationMessage::new(
                ConversationMessageKind::Agent,
                transcript.trim_end().to_string(),
                None,
                Some(format!("tmux-snapshot-{}", target.pane_id)),
            )]
        };
        Ok(ConversationSnapshot {
            thread_id: target.pane_id.clone(),
            title: format!("tmux {}", target.display_name),
            cwd: target.cwd,
            messages,
            warnings: Vec::new(),
            runtime_notices: vec![format!(
                "loaded transcript from tmux pane {}",
                target.pane_id
            )],
        })
    }

    fn run_new_thread_stream(
        &self,
        _cwd: &str,
        prompt: &str,
        event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        let target = self.resolve_startup_target()?;
        emit_attachment_observed(
            &event_sender,
            TerminalBridgeAttachmentProfile::tmux_local_attach(),
        );
        let _ = event_sender.send(ConversationStreamEvent::ThreadPrepared {
            thread_id: target.pane_id.clone(),
            title: format!("tmux {}", target.display_name),
            cwd: target.cwd.clone(),
        });
        self.run_stream_on_target(&target, prompt, &event_sender)
    }

    fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        let target = self.resolve_turn_target(Some(thread_id))?;
        emit_attachment_observed(
            &event_sender,
            TerminalBridgeAttachmentProfile::tmux_local_attach(),
        );
        self.run_stream_on_target(&target, prompt, &event_sender)
    }

    fn request_interrupt(&self, thread_id: Option<&str>) -> Result<()> {
        let target = self.resolve_turn_target(thread_id)?;
        self.send_interrupt(&target.pane_id).with_context(|| {
            format!(
                "failed to send interrupt to tmux pane {} ({})",
                target.pane_id, target.display_name
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TmuxPaneTarget {
    pane_id: String,
    cwd: String,
    display_name: String,
    pipe_active: bool,
    current_command: String,
}

struct TmuxPipeCapture {
    adapter: TmuxLocalAttachAdapter,
    target: String,
    log_path: PathBuf,
    file: File,
    cursor: u64,
}

impl TmuxPipeCapture {
    fn read_new_text(&mut self) -> Result<String> {
        self.file
            .seek(SeekFrom::Start(self.cursor))
            .context("failed to seek tmux pipe capture log")?;
        let mut buffer = Vec::new();
        self.file
            .read_to_end(&mut buffer)
            .context("failed to read tmux pipe capture log")?;
        self.cursor += buffer.len() as u64;
        Ok(String::from_utf8_lossy(&buffer).to_string())
    }
}

impl Drop for TmuxPipeCapture {
    fn drop(&mut self) {
        let _ = self.adapter.run_tmux(&[
            "pipe-pane".to_string(),
            "-t".to_string(),
            self.target.clone(),
        ]);
        let _ = fs::remove_file(&self.log_path);
    }
}

fn parse_list_panes_output(stdout: &str) -> Result<Vec<TmuxPaneTarget>> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut fields = line.splitn(5, '\t');
            let pane_id = fields
                .next()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow!("tmux pane discovery output was missing pane id"))?;
            let cwd = fields
                .next()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow!("tmux pane discovery output was missing pane cwd"))?;
            let display_name = fields
                .next()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow!("tmux pane discovery output was missing pane label"))?;
            let pipe_active = match fields.next() {
                Some("1") => true,
                Some("0") => false,
                Some(value) => {
                    return Err(anyhow!(
                        "tmux pane discovery output returned an unknown pipe state `{value}`"
                    ));
                }
                None => return Err(anyhow!("tmux pane discovery output was missing pipe state")),
            };
            let current_command = fields
                .next()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("unknown");
            Ok(TmuxPaneTarget {
                pane_id: pane_id.to_string(),
                cwd: cwd.to_string(),
                display_name: display_name.to_string(),
                pipe_active,
                current_command: current_command.to_string(),
            })
        })
        .collect()
}

fn sync_manual_handoff_status(
    transcript: &str,
    manual_handoff_active: &mut bool,
    target: &TmuxPaneTarget,
    event_sender: &std::sync::mpsc::Sender<ConversationStreamEvent>,
) {
    let handoff_detected = detect_manual_handoff_prompt(transcript);
    if handoff_detected == *manual_handoff_active {
        return;
    }

    *manual_handoff_active = handoff_detected;
    let text = if handoff_detected {
        manual_handoff_status_text(target)
    } else {
        streaming_status_text(target)
    };
    let _ = event_sender.send(ConversationStreamEvent::StatusUpdated { text });
}

fn streaming_status_text(target: &TmuxPaneTarget) -> String {
    format!("{TMUX_STREAMING_STATUS_PREFIX} {}", target.display_name)
}

fn manual_handoff_status_text(target: &TmuxPaneTarget) -> String {
    format!(
        "{TMUX_MANUAL_HANDOFF_STATUS_PREFIX} {} ({}); respond in the attached terminal to continue",
        target.display_name, target.pane_id
    )
}

fn detect_manual_handoff_prompt(transcript: &str) -> bool {
    let Some(line) = transcript
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
    else {
        return false;
    };

    let normalized = line
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let confirmation_suffixes = ["[y/n]", "(y/n)", "[yes/no]", "(yes/no)"];

    confirmation_suffixes
        .iter()
        .any(|suffix| normalized.ends_with(suffix))
}

fn sanitize_terminal_transcript(text: &str) -> String {
    let mut sanitized = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for candidate in chars.by_ref() {
                        if ('@'..='~').contains(&candidate) {
                            break;
                        }
                    }
                    continue;
                }
                Some(']') => {
                    chars.next();
                    let mut previous = None;
                    for candidate in chars.by_ref() {
                        if candidate == '\u{7}' || (previous == Some('\u{1b}') && candidate == '\\')
                        {
                            break;
                        }
                        previous = Some(candidate);
                    }
                    continue;
                }
                _ => continue,
            }
        }

        match ch {
            '\r' | '\0' => {}
            value if value.is_control() && value != '\n' && value != '\t' => {}
            value => sanitized.push(value),
        }
    }

    sanitized
}

fn expected_prompt_echo(prompt: &str) -> String {
    let mut echo = sanitize_terminal_transcript(prompt);
    if !echo.ends_with('\n') {
        echo.push('\n');
    }
    echo
}

fn strip_prompt_echo(text: String, remaining_echo: &mut String) -> String {
    if remaining_echo.is_empty() || text.is_empty() {
        return text;
    }

    let matched_bytes = common_prefix_byte_len(&text, remaining_echo);
    if matched_bytes == 0 {
        return text;
    }

    remaining_echo.drain(..matched_bytes);
    let mut stripped = text[matched_bytes..].to_string();
    if remaining_echo.is_empty() && stripped.starts_with('\n') {
        stripped.remove(0);
    }
    stripped
}

fn common_prefix_byte_len(left: &str, right: &str) -> usize {
    let mut matched = 0;
    for ((left_index, left_char), (right_index, right_char)) in
        left.char_indices().zip(right.char_indices())
    {
        if left_char != right_char {
            break;
        }
        matched = left_index + left_char.len_utf8();
        if right_index + right_char.len_utf8() != matched {
            matched = right_index + right_char.len_utf8();
        }
    }
    matched
}

fn normalized_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_u64(name: &str, default: u64) -> u64 {
    normalized_env(name)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn unique_label(prefix: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{millis}-{counter}")
}

fn temp_file_path(prefix: &str, extension: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{}-{}.{}",
        prefix,
        unique_label("capture"),
        extension
    ))
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    use super::{
        common_prefix_byte_len, detect_manual_handoff_prompt, parse_list_panes_output,
        sanitize_terminal_transcript, strip_prompt_echo, InteractiveTurnRuntimePort,
        TerminalBridgeMode, TmuxLocalAttachAdapter, TmuxLocalAttachConfig,
        TMUX_MANUAL_HANDOFF_STATUS_PREFIX,
    };
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;

    #[test]
    fn tmux_bridge_mode_accepts_tmux_aliases() {
        assert_eq!(
            TerminalBridgeMode::from_env_value(Some("tmux-local-attach")),
            TerminalBridgeMode::TmuxLocalAttach
        );
        assert_eq!(
            TerminalBridgeMode::from_env_value(Some("tmux")),
            TerminalBridgeMode::TmuxLocalAttach
        );
        assert_eq!(
            TerminalBridgeMode::from_env_value(Some("local_attach")),
            TerminalBridgeMode::TmuxLocalAttach
        );
        assert_eq!(
            TerminalBridgeMode::from_env_value(Some("codex")),
            TerminalBridgeMode::CodexAppServer
        );
    }

    #[test]
    fn tmux_pane_listing_parser_tracks_pipe_state_and_command() {
        let panes = parse_list_panes_output(
            "%2\t/tmp/workspace\twork:1.0\t0\tclaude\n%3\t/tmp/other\tother:0.0\t1\tbash\n",
        )
        .expect("pane listing should parse");

        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].pane_id, "%2");
        assert_eq!(panes[0].cwd, "/tmp/workspace");
        assert_eq!(panes[0].display_name, "work:1.0");
        assert!(!panes[0].pipe_active);
        assert_eq!(panes[0].current_command, "claude");
        assert!(panes[1].pipe_active);
    }

    #[test]
    fn transcript_sanitizer_strips_escape_sequences_and_control_bytes() {
        let sanitized = sanitize_terminal_transcript(
            "hello\u{1b}[200~ world\r\n\u{1b}]0;title\u{7}next\u{0}\n",
        );

        assert_eq!(sanitized, "hello world\nnext\n");
    }

    #[test]
    fn prompt_echo_is_stripped_across_multiple_chunks() {
        let mut remaining_echo = "ship it\nwith context\n".to_string();
        let first = strip_prompt_echo("ship it\nwith ".to_string(), &mut remaining_echo);
        let second = strip_prompt_echo(
            "context\nassistant reply\n".to_string(),
            &mut remaining_echo,
        );

        assert_eq!(first, "");
        assert_eq!(second, "assistant reply\n");
        assert!(remaining_echo.is_empty());
    }

    #[test]
    fn common_prefix_length_tracks_utf8_boundaries() {
        assert_eq!(common_prefix_byte_len("alpha", "alphabet"), 5);
        assert_eq!(common_prefix_byte_len("한글", "한"), "한".len());
        assert_eq!(common_prefix_byte_len("left", "right"), 0);
    }

    #[test]
    fn manual_handoff_prompt_detection_only_matches_pending_tail_prompts() {
        assert!(detect_manual_handoff_prompt(
            "PROMPT> printf \"approve? [y/N] \"; read answer\napprove? [y/N]"
        ));
        assert!(!detect_manual_handoff_prompt(
            "approve? [y/N] y\nanswer=y\nPROMPT>"
        ));
        assert!(!detect_manual_handoff_prompt("assistant reply\nall done"));
    }

    #[test]
    fn tmux_local_attach_streams_a_shell_prompt_through_a_real_pane() {
        if which::which("tmux").is_err() {
            return;
        }

        let socket = format!(
            "codex-exec-loop-test-{}",
            super::unique_label("tmux-local-attach")
        );
        let session_name = "codex-exec-loop-smoke";
        let create_status = Command::new("tmux")
            .args([
                "-L",
                socket.as_str(),
                "new-session",
                "-d",
                "-s",
                session_name,
                "bash --noprofile --norc",
            ])
            .status()
            .expect("tmux test session should start");
        assert!(create_status.success());

        let pane_id = Command::new("tmux")
            .args([
                "-L",
                socket.as_str(),
                "display-message",
                "-p",
                "-t",
                &format!("{session_name}:0.0"),
                "#{pane_id}",
            ])
            .output()
            .expect("pane id should resolve");
        let pane_id = String::from_utf8_lossy(&pane_id.stdout).trim().to_string();
        assert!(!pane_id.is_empty());

        let adapter = TmuxLocalAttachAdapter {
            config: TmuxLocalAttachConfig {
                socket: Some(socket.clone()),
                target: Some(pane_id.clone()),
                poll_interval: Duration::from_millis(50),
                idle_timeout: Duration::from_millis(350),
                no_output_timeout: Duration::from_secs(5),
            },
        };
        let (tx, rx) = channel();

        let result = adapter.run_new_thread_stream("/tmp", "printf 'smoke-bridge\\n'", tx);

        let _ = Command::new("tmux")
            .args(["-L", socket.as_str(), "kill-server"])
            .status();
        result.expect("tmux local attach stream should succeed");

        let events = rx.iter().collect::<Vec<_>>();
        assert!(events
            .iter()
            .any(|event| matches!(event, ConversationStreamEvent::AttachmentObserved { .. })));
        assert!(events.iter().any(|event| matches!(
            event,
            ConversationStreamEvent::ThreadPrepared { thread_id, .. } if thread_id == &pane_id
        )));
        assert!(events
            .iter()
            .any(|event| matches!(event, ConversationStreamEvent::TurnCompleted { .. })));
        assert!(events.iter().any(|event| matches!(
            event,
            ConversationStreamEvent::AgentMessageCompleted { text, .. } if text.contains("smoke-bridge")
        )));
    }

    #[test]
    fn tmux_local_attach_interrupts_a_long_running_turn() {
        if which::which("tmux").is_err() {
            return;
        }

        let socket = format!(
            "codex-exec-loop-test-{}",
            super::unique_label("tmux-local-attach-interrupt")
        );
        let session_name = "codex-exec-loop-interrupt";
        let create_status = Command::new("tmux")
            .args([
                "-L",
                socket.as_str(),
                "new-session",
                "-d",
                "-s",
                session_name,
                "bash --noprofile --norc",
            ])
            .status()
            .expect("tmux test session should start");
        assert!(create_status.success());

        let pane_id = Command::new("tmux")
            .args([
                "-L",
                socket.as_str(),
                "display-message",
                "-p",
                "-t",
                &format!("{session_name}:0.0"),
                "#{pane_id}",
            ])
            .output()
            .expect("pane id should resolve");
        let pane_id = String::from_utf8_lossy(&pane_id.stdout).trim().to_string();
        assert!(!pane_id.is_empty());

        let adapter = TmuxLocalAttachAdapter {
            config: TmuxLocalAttachConfig {
                socket: Some(socket.clone()),
                target: Some(pane_id.clone()),
                poll_interval: Duration::from_millis(50),
                idle_timeout: Duration::from_millis(250),
                no_output_timeout: Duration::from_secs(10),
            },
        };
        let (tx, rx) = channel();
        let interrupt_adapter = adapter.clone();
        let stream_started_at = Instant::now();
        let stream_handle =
            std::thread::spawn(move || adapter.run_new_thread_stream("/tmp", "sleep 30", tx));
        let mut events = Vec::new();

        loop {
            let event = rx
                .recv_timeout(Duration::from_secs(2))
                .expect("turn should emit startup events before interrupt");
            let saw_turn_started = matches!(&event, ConversationStreamEvent::TurnStarted { .. });
            events.push(event);
            if saw_turn_started {
                break;
            }
        }

        interrupt_adapter
            .request_interrupt(Some(&pane_id))
            .expect("interrupt request should succeed");

        loop {
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(event) => {
                    let should_stop = matches!(
                        &event,
                        ConversationStreamEvent::TurnCompleted { .. }
                            | ConversationStreamEvent::Failed { .. }
                    );
                    events.push(event);
                    if should_stop {
                        break;
                    }
                }
                Err(error) => panic!("timed out waiting for interrupted turn to settle: {error}"),
            }
        }

        let _ = Command::new("tmux")
            .args(["-L", socket.as_str(), "kill-server"])
            .status();
        let stream_result = stream_handle
            .join()
            .expect("stream worker thread should not panic");
        stream_result.expect("interrupted stream should still settle cleanly");
        assert!(stream_started_at.elapsed() < Duration::from_secs(5));
        assert!(events.iter().any(|event| matches!(
            event,
            ConversationStreamEvent::ThreadPrepared { thread_id, .. } if thread_id == &pane_id
        )));
        assert!(events
            .iter()
            .any(|event| matches!(event, ConversationStreamEvent::TurnCompleted { .. })));
    }

    #[test]
    fn tmux_local_attach_keeps_manual_handoff_turn_open_until_input_arrives() {
        if which::which("tmux").is_err() {
            return;
        }

        let socket = format!(
            "codex-exec-loop-test-{}",
            super::unique_label("tmux-local-attach-handoff")
        );
        let session_name = "codex-exec-loop-handoff";
        let create_status = Command::new("tmux")
            .args([
                "-L",
                socket.as_str(),
                "new-session",
                "-d",
                "-s",
                session_name,
                "bash --noprofile --norc",
            ])
            .status()
            .expect("tmux test session should start");
        assert!(create_status.success());

        let pane_id = Command::new("tmux")
            .args([
                "-L",
                socket.as_str(),
                "display-message",
                "-p",
                "-t",
                &format!("{session_name}:0.0"),
                "#{pane_id}",
            ])
            .output()
            .expect("pane id should resolve");
        let pane_id = String::from_utf8_lossy(&pane_id.stdout).trim().to_string();
        assert!(!pane_id.is_empty());

        let adapter = TmuxLocalAttachAdapter {
            config: TmuxLocalAttachConfig {
                socket: Some(socket.clone()),
                target: Some(pane_id.clone()),
                poll_interval: Duration::from_millis(50),
                idle_timeout: Duration::from_millis(250),
                no_output_timeout: Duration::from_secs(10),
            },
        };
        let (tx, rx) = channel();
        let stream_handle = std::thread::spawn(move || {
            adapter.run_new_thread_stream(
                "/tmp",
                r#"printf "approve? [y/N] "; read answer; printf "answer=%s\n" "$answer""#,
                tx,
            )
        });
        let mut events = Vec::new();
        let mut saw_manual_handoff = false;

        while !saw_manual_handoff {
            let event = rx
                .recv_timeout(Duration::from_secs(2))
                .expect("handoff flow should emit prompt and status updates");
            saw_manual_handoff = matches!(
                &event,
                ConversationStreamEvent::StatusUpdated { text }
                    if text.contains(TMUX_MANUAL_HANDOFF_STATUS_PREFIX)
            );
            events.push(event);
        }

        match rx.recv_timeout(Duration::from_millis(700)) {
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Ok(ConversationStreamEvent::TurnCompleted { .. }) => {
                panic!("manual handoff turn should not complete before operator input")
            }
            Ok(event) => events.push(event),
            Err(error) => panic!("manual handoff wait failed unexpectedly: {error}"),
        }

        let answer_status = Command::new("tmux")
            .args([
                "-L",
                socket.as_str(),
                "send-keys",
                "-t",
                pane_id.as_str(),
                "y",
                "C-m",
            ])
            .status()
            .expect("operator input should be relayed into pane");
        assert!(answer_status.success());

        loop {
            let event = rx
                .recv_timeout(Duration::from_secs(2))
                .expect("manual handoff turn should settle after operator input");
            let should_stop = matches!(&event, ConversationStreamEvent::TurnCompleted { .. });
            events.push(event);
            if should_stop {
                break;
            }
        }

        let _ = Command::new("tmux")
            .args(["-L", socket.as_str(), "kill-server"])
            .status();
        let stream_result = stream_handle
            .join()
            .expect("stream worker thread should not panic");
        stream_result.expect("manual handoff stream should settle cleanly");

        assert!(events.iter().any(|event| matches!(
            event,
            ConversationStreamEvent::StatusUpdated { text }
                if text.contains(TMUX_MANUAL_HANDOFF_STATUS_PREFIX)
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ConversationStreamEvent::AgentMessageCompleted { text, .. } if text.contains("answer=y")
        )));
        assert!(events
            .iter()
            .any(|event| matches!(event, ConversationStreamEvent::TurnCompleted { .. })));
    }
}

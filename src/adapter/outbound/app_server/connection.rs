/*
 * connection.rs는 `codex app-server` child process와 직접 대화하는 lowest-level outbound boundary다.
 * 위 계층은 typed method(start_thread, start_turn 등)를 호출하지만, 이 파일은 stdin에 JSON line을 쓰고
 * stdout/stderr reader thread에서 notification/response line을 받아 request id와 매칭한다.
 */
#[cfg(test)]
use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::port::conversation_stream::ConversationStreamEvent;
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalRequest, ConversationApprovalRequestIdentity,
    ConversationApprovalResolution, ConversationTurnSteerReceipt,
};
use crate::domain::conversation_runtime_envelope::{
    ConversationRuntimeLaunchEnvironment, ConversationRuntimeProcessEnvironment,
    ConversationRuntimeShellEnvironment,
};
use crate::domain::turn_terminal::{
    ConversationTurnApplicationDelivery, ConversationTurnApplicationDeliveryFailure,
    ConversationTurnError, ConversationTurnItemsView, ConversationTurnObservations,
    ConversationTurnTerminalOutcome, ConversationTurnTerminalReceipt,
    ConversationTurnTerminalUncertainty,
};
use crate::subprocess::{self, ManagedChild};

use super::approval::{
    AppServerApprovalBroker, AppServerApprovalSpec, EXPLICITLY_DECLINED_APPROVAL_METHODS,
    INTERACTIVE_APPROVAL_METHODS, UNINSPECTABLE_APPROVAL_METHODS, parse_interactive_approval,
};
use super::protocol::{
    AccountReadResponse, ActiveTurnNotificationState, AppServerNotification, InitializeResponse,
    ThreadListParams, ThreadListResponse, ThreadReadResponse, ThreadResumeParams,
    ThreadResumeResponse, ThreadSetNameParams, ThreadSetNameResponse, ThreadStartParams,
    ThreadStartResponse, TurnInterruptParams, TurnInterruptResponse, TurnNotificationHandling,
    TurnStartParams, TurnStartResponse, TurnSteerParams, TurnSteerResponse,
    handle_turn_notification,
};
use super::steering::{AppServerTurnSteerBinding, AppServerTurnSteerBroker};
use super::{AppServerEventSender, AppServerEventTrySendError, bounded_terminal_receipt};

const RESPONSE_TIMEOUT_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_RESPONSE_TIMEOUT_SECS";
const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_TIMEOUT_SECS: u64 = 300;
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(200);
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_millis(300);
const DEFAULT_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(50);
const DEFAULT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
// A stop request is a safety boundary, not a normal control-plane request. Keep
// the complete interrupt exchange short even when the general JSON-RPC response
// timeout is raised for a slow app-server installation.
const DEFAULT_INTERRUPT_TOTAL_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_INTERRUPT_RETRY_BACKOFF: Duration = Duration::from_millis(100);
const DEFAULT_INTERRUPT_RETRY_LIMIT: usize = 3;
const DEFAULT_TERMINAL_GRACE_TIMEOUT: Duration = Duration::from_secs(1);
const DEFAULT_TERMINAL_DELIVERY_TIMEOUT: Duration = Duration::from_millis(250);
const DEFAULT_TERMINAL_DELIVERY_RETRY_INTERVAL: Duration = Duration::from_millis(5);
const MAX_PENDING_TURN_NOTIFICATIONS_PER_POLL: usize = 32;
const MAX_PENDING_TURN_NOTIFICATION_DRAIN_TIME: Duration = Duration::from_millis(2);
const MAX_GRACE_RECOVERY_LINES: usize = 16;
const MAX_GRACE_RECOVERY_TIME: Duration = Duration::from_millis(5);
const MAX_TRANSPORT_CLOSING_RECOVERY_TIME: Duration = Duration::from_millis(100);
const MAX_TRANSPORT_CLOSING_RECOVERY_BYTES: usize = MAX_PENDING_NOTIFICATION_BYTES;
// Keep at most one fully materialized raw line between the pipe readers and the
// JSON consumer. A larger count-only queue would multiply the 128 MiB history
// allowance before parsed notification byte accounting can take effect.
const APP_SERVER_LINE_CHANNEL_CAPACITY: usize = 1;
const APP_SERVER_WRITE_CHANNEL_CAPACITY: usize = 1;
const MAX_APP_SERVER_WRITE_ACK_TIMEOUT: Duration = Duration::from_secs(3);
const APP_SERVER_WRITER_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(100);
const MAX_APP_SERVER_WRITE_FRAME_BYTES: usize = 16 * 1024 * 1024;
// `thread/read` and `thread/resume` return the complete thread as one JSON-RPC
// line. Keep a hard transport bound, but leave enough room for long-lived
// conversations and their tool output instead of treating a normal history as
// a poisoned connection.
const MAX_STDOUT_LINE_BYTES: usize = 128 * 1024 * 1024;
const MAX_STDERR_LINE_BYTES: usize = 64 * 1024;
const SHELL_ENVIRONMENT_INHERIT_ENV_VAR: &str = "AKRA_APP_SERVER_SHELL_ENVIRONMENT_INHERIT";
const PROCESS_ENVIRONMENT_ENV_VAR: &str = "AKRA_APP_SERVER_PROCESS_ENVIRONMENT";
const API_KEY_AUTH_ENV_VAR: &str = "AKRA_APP_SERVER_API_KEY_AUTH";
const SHELL_ENVIRONMENT_SECRET_EXCLUDES_OVERRIDE: &str =
    "shell_environment_policy.ignore_default_excludes=false";
const DISABLE_LOGIN_SHELL_OVERRIDE: &str = "allow_login_shell=false";
const APP_SERVER_API_KEY_ENV_VARS: &[&str] = &["OPENAI_API_KEY", "CODEX_API_KEY"];
const APP_SERVER_PROCESS_ENVIRONMENT_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "TMP",
    "TEMP",
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    "FORCE_COLOR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
    "CODEX_HOME",
    "OPENAI_BASE_URL",
    "OPENAI_ORGANIZATION",
    "OPENAI_ORG_ID",
    "OPENAI_PROJECT",
    "OPENAI_PROJECT_ID",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "SYSTEMROOT",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "WSLENV",
    "WSL_DISTRO_NAME",
    "WSL_INTEROP",
];

#[cfg(test)]
pub(crate) const CANCELLED_SERVER_REQUEST_METHODS: &[&str] = &[
    "item/tool/requestUserInput",
    "mcpServer/elicitation/request",
];
#[cfg(test)]
pub(crate) const METHOD_SPECIFIC_UNSUPPORTED_SERVER_REQUEST_METHODS: &[&str] = &[
    "item/tool/call",
    "account/chatgptAuthTokens/refresh",
    "attestation/generate",
];
#[cfg(test)]
pub(crate) const UNINSPECTABLE_SERVER_REQUEST_METHODS: &[&str] = UNINSPECTABLE_APPROVAL_METHODS;
#[cfg(test)]
pub(crate) const LOCALLY_SUPPORTED_SERVER_REQUEST_METHODS: &[&str] = &["currentTime/read"];

mod diagnostics;

use self::diagnostics::{
    ConnectionDiagnostics, MAX_PENDING_NOTIFICATION_BYTES, MAX_PENDING_NOTIFICATIONS,
    PendingNotifications,
};

#[cfg(test)]
thread_local! {
    static AFTER_APPROVAL_DECISION_RECEIVED_HOOK: RefCell<Option<Box<dyn FnOnce()>>> =
        RefCell::new(None);
}

#[cfg(test)]
fn install_after_approval_decision_received_hook(hook: impl FnOnce() + 'static) {
    AFTER_APPROVAL_DECISION_RECEIVED_HOOK.with(|slot| {
        let previous = slot.borrow_mut().replace(Box::new(hook));
        assert!(
            previous.is_none(),
            "approval decision test hook already installed"
        );
    });
}

#[cfg(test)]
fn run_after_approval_decision_received_hook() {
    AFTER_APPROVAL_DECISION_RECEIVED_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_after_approval_decision_received_hook() {}

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
    executable: PathBuf,
    executable_prefix_args: Vec<OsString>,
    executable_resolution_error: Option<String>,
    command_environment: Vec<(OsString, OsString)>,
    shell_environment_inherit: ShellEnvironmentInherit,
    process_environment_policy: ProcessEnvironmentPolicy,
    api_key_auth: bool,
    response_timeout: Duration,
    poll_interval: Duration,
    drain_timeout: Duration,
    drain_poll_interval: Duration,
    approval_timeout: Duration,
    interrupt_total_timeout: Duration,
    interrupt_retry_backoff: Duration,
    interrupt_retry_limit: usize,
    terminal_grace_timeout: Duration,
    terminal_delivery_timeout: Duration,
    terminal_delivery_retry_interval: Duration,
}

impl Default for AppServerConnectionConfig {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("codex"),
            executable_prefix_args: Vec::new(),
            executable_resolution_error: None,
            command_environment: Vec::new(),
            shell_environment_inherit: ShellEnvironmentInherit::Core,
            process_environment_policy: ProcessEnvironmentPolicy::Scrubbed,
            api_key_auth: false,
            response_timeout: DEFAULT_RESPONSE_TIMEOUT,
            poll_interval: DEFAULT_POLL_INTERVAL,
            drain_timeout: DEFAULT_DRAIN_TIMEOUT,
            drain_poll_interval: DEFAULT_DRAIN_POLL_INTERVAL,
            approval_timeout: DEFAULT_APPROVAL_TIMEOUT,
            interrupt_total_timeout: DEFAULT_INTERRUPT_TOTAL_TIMEOUT,
            interrupt_retry_backoff: DEFAULT_INTERRUPT_RETRY_BACKOFF,
            interrupt_retry_limit: DEFAULT_INTERRUPT_RETRY_LIMIT,
            terminal_grace_timeout: DEFAULT_TERMINAL_GRACE_TIMEOUT,
            terminal_delivery_timeout: DEFAULT_TERMINAL_DELIVERY_TIMEOUT,
            terminal_delivery_retry_interval: DEFAULT_TERMINAL_DELIVERY_RETRY_INTERVAL,
        }
    }
}

impl AppServerConnectionConfig {
    pub(super) const fn runtime_launch_environment(&self) -> ConversationRuntimeLaunchEnvironment {
        let process_environment = match self.process_environment_policy {
            ProcessEnvironmentPolicy::Scrubbed => ConversationRuntimeProcessEnvironment::Scrubbed,
            ProcessEnvironmentPolicy::All => ConversationRuntimeProcessEnvironment::InheritedAll,
        };
        let shell_environment = match self.shell_environment_inherit {
            ShellEnvironmentInherit::None => ConversationRuntimeShellEnvironment::None,
            ShellEnvironmentInherit::Core => ConversationRuntimeShellEnvironment::Core,
            ShellEnvironmentInherit::All => ConversationRuntimeShellEnvironment::All,
        };
        ConversationRuntimeLaunchEnvironment {
            process_environment,
            shell_environment,
            api_key_auth: self.api_key_auth,
        }
    }

    pub(super) fn from_environment() -> Self {
        // 운영 override는 response timeout만 열어두고, poll/drain 간격은 stream responsiveness 기준으로 고정한다.
        let configured_timeout = crate::configuration::current_process_config()
            .map(|config| config.config.app_server.response_timeout_secs.to_string());
        let environment_timeout = std::env::var(RESPONSE_TIMEOUT_ENV_VAR).ok();
        let mut config = Self::from_response_timeout_secs_value(
            configured_timeout
                .as_deref()
                .or(environment_timeout.as_deref()),
        );
        config.shell_environment_inherit = configured_shell_environment_inherit();
        config.process_environment_policy = configured_process_environment_policy();
        config.api_key_auth = configured_api_key_auth();
        match crate::trusted_executable::pinned_codex_command() {
            Ok(command) => {
                config.executable = command.program;
                config.executable_prefix_args = command.prefix_args;
            }
            Err(error) => {
                config.executable = unresolved_codex_executable_path();
                config.executable_resolution_error = Some(format!("{error:#}"));
            }
        }
        config
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

        config.response_timeout = Duration::from_secs(seconds.min(MAX_RESPONSE_TIMEOUT_SECS));
        config
    }

    #[cfg(test)]
    pub(super) fn with_test_process(
        mut self,
        executable: impl Into<PathBuf>,
        command_environment: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Self {
        self.executable = executable.into();
        self.executable_prefix_args.clear();
        self.executable_resolution_error = None;
        self.command_environment = command_environment.into_iter().collect();
        self
    }

    #[cfg(test)]
    pub(super) fn with_test_elevated_environment(mut self) -> Self {
        self.shell_environment_inherit = ShellEnvironmentInherit::All;
        self.process_environment_policy = ProcessEnvironmentPolicy::All;
        self
    }

    #[cfg(test)]
    pub(super) fn with_test_api_key_auth(mut self) -> Self {
        self.api_key_auth = true;
        self
    }

    pub(super) fn environment_policy_summary(&self) -> String {
        format!(
            "process-env={}, api-key-auth={}, shell-env={}",
            self.process_environment_policy.label(),
            if self.api_key_auth {
                "enabled"
            } else {
                "disabled"
            },
            self.shell_environment_inherit.label()
        )
    }

    pub(super) fn uses_full_process_environment(&self) -> bool {
        self.process_environment_policy == ProcessEnvironmentPolicy::All
    }

    pub(super) fn uses_api_key_auth(&self) -> bool {
        self.api_key_auth
    }

    fn ensure_executable_is_pinned(&self) -> Result<()> {
        if let Some(error) = &self.executable_resolution_error {
            bail!("Codex executable could not be pinned safely at startup: {error}")
        }
        if !self.executable.is_absolute() && !cfg!(test) {
            bail!("Codex executable pin is not an absolute path")
        }
        Ok(())
    }
}

#[cfg(unix)]
fn unresolved_codex_executable_path() -> PathBuf {
    PathBuf::from("/__akra_unresolved_codex_executable__")
}

#[cfg(windows)]
fn unresolved_codex_executable_path() -> PathBuf {
    PathBuf::from(r"C:\__akra_unresolved_codex_executable__.exe")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ShellEnvironmentInherit {
    None,
    #[default]
    Core,
    All,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ProcessEnvironmentPolicy {
    #[default]
    Scrubbed,
    All,
}

impl ProcessEnvironmentPolicy {
    fn label(self) -> &'static str {
        match self {
            Self::Scrubbed => "scrubbed",
            Self::All => "all",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ProcessEnvironmentResolution {
    policy: ProcessEnvironmentPolicy,
    warning: Option<String>,
}

fn resolve_process_environment(value: Option<&str>) -> ProcessEnvironmentResolution {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("scrubbed") => ProcessEnvironmentResolution {
            policy: ProcessEnvironmentPolicy::Scrubbed,
            warning: None,
        },
        Some("all") => ProcessEnvironmentResolution {
            policy: ProcessEnvironmentPolicy::All,
            warning: Some(format!(
                "{PROCESS_ENVIRONMENT_ENV_VAR}=all explicitly exposes the complete parent environment, including credentials, to the app-server process"
            )),
        },
        Some(_) => ProcessEnvironmentResolution {
            policy: ProcessEnvironmentPolicy::Scrubbed,
            warning: Some(format!(
                "invalid {PROCESS_ENVIRONMENT_ENV_VAR}; expected scrubbed or all; using scrubbed"
            )),
        },
    }
}

fn configured_process_environment_policy() -> ProcessEnvironmentPolicy {
    let resolution =
        resolve_process_environment(std::env::var(PROCESS_ENVIRONMENT_ENV_VAR).ok().as_deref());
    if let Some(warning) = resolution.warning {
        eprintln!("warning: {warning}");
        tracing::warn!(warning = %warning, "invalid app-server process environment policy");
    }
    resolution.policy
}

#[derive(Debug, PartialEq, Eq)]
struct ApiKeyAuthResolution {
    enabled: bool,
    warning: Option<String>,
}

fn resolve_api_key_auth(value: Option<&OsStr>) -> ApiKeyAuthResolution {
    match value {
        None => ApiKeyAuthResolution {
            enabled: false,
            warning: None,
        },
        Some(value) if value == "1" => ApiKeyAuthResolution {
            enabled: true,
            warning: None,
        },
        Some(_) => ApiKeyAuthResolution {
            enabled: false,
            warning: Some(format!(
                "invalid {API_KEY_AUTH_ENV_VAR}; expected exact value 1; API-key forwarding remains disabled"
            )),
        },
    }
}

fn configured_api_key_auth() -> bool {
    let value = std::env::var_os(API_KEY_AUTH_ENV_VAR);
    let resolution = resolve_api_key_auth(value.as_deref());
    if let Some(warning) = resolution.warning {
        eprintln!("warning: {warning}");
        tracing::warn!(warning = %warning, "invalid app-server API-key auth policy");
    }
    resolution.enabled
}

fn app_server_process_environment_variable_allowed(key: &OsString) -> bool {
    let Some(key) = key.to_str() else {
        return false;
    };
    app_server_process_environment_key_allowed(key, cfg!(windows))
}

fn app_server_api_key_environment_variable_allowed(key: &OsString) -> bool {
    let Some(key) = key.to_str() else {
        return false;
    };
    if cfg!(windows) {
        APP_SERVER_API_KEY_ENV_VARS
            .iter()
            .any(|allowed| key.eq_ignore_ascii_case(allowed))
    } else {
        APP_SERVER_API_KEY_ENV_VARS.contains(&key)
    }
}

fn app_server_process_environment_key_allowed(key: &str, ascii_case_insensitive: bool) -> bool {
    if ascii_case_insensitive {
        key.eq_ignore_ascii_case("LANG")
            || key
                .get(..3)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("LC_"))
            || APP_SERVER_PROCESS_ENVIRONMENT_ALLOWLIST
                .iter()
                .any(|allowed| key.eq_ignore_ascii_case(allowed))
    } else {
        key == "LANG"
            || key.starts_with("LC_")
            || APP_SERVER_PROCESS_ENVIRONMENT_ALLOWLIST.contains(&key)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct FilteredProcessEnvironment {
    variables: Vec<(OsString, OsString)>,
    dropped_credential_variables: Vec<&'static str>,
}

fn filtered_app_server_process_environment(
    environment: impl IntoIterator<Item = (OsString, OsString)>,
    api_key_auth: bool,
) -> FilteredProcessEnvironment {
    let mut variables = Vec::new();
    let mut dropped_credential_variables = Vec::new();
    for (key, value) in environment {
        if !(app_server_process_environment_variable_allowed(&key)
            || api_key_auth && app_server_api_key_environment_variable_allowed(&key))
        {
            continue;
        }
        if let Some(proxy_variable) = canonical_proxy_environment_key(&key)
            && proxy_environment_value_is_unsafe(&value)
        {
            if !dropped_credential_variables.contains(&proxy_variable) {
                dropped_credential_variables.push(proxy_variable);
            }
            continue;
        }
        if key
            .to_str()
            .is_some_and(|key| key.eq_ignore_ascii_case("OPENAI_BASE_URL"))
            && openai_base_url_is_unsafe(&value)
        {
            if !dropped_credential_variables.contains(&"OPENAI_BASE_URL") {
                dropped_credential_variables.push("OPENAI_BASE_URL");
            }
            continue;
        }
        variables.push((key, value));
    }
    FilteredProcessEnvironment {
        variables,
        dropped_credential_variables,
    }
}

fn canonical_proxy_environment_key(key: &OsString) -> Option<&'static str> {
    let key = key.to_str()?;
    ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"]
        .into_iter()
        .find(|candidate| key.eq_ignore_ascii_case(candidate))
}

fn proxy_environment_value_is_unsafe(value: &OsString) -> bool {
    value
        .to_str()
        .is_none_or(|value| proxy_url_has_userinfo(value).unwrap_or(true))
}

fn openai_base_url_is_unsafe(value: &OsString) -> bool {
    value.to_str().is_none_or(|value| {
        if value.is_empty()
            || value.trim() != value
            || value.contains(['\\', '#'])
            || value.bytes().any(|byte| byte.is_ascii_whitespace())
        {
            return true;
        }
        let Ok(uri) = value.parse::<http::Uri>() else {
            return true;
        };
        if uri.query().is_some() {
            return true;
        }
        let Some(scheme) = uri.scheme_str() else {
            return true;
        };
        let Some(authority) = uri.authority() else {
            return true;
        };
        if authority.as_str().contains('@') {
            return true;
        }
        if scheme.eq_ignore_ascii_case("https") {
            return false;
        }
        !scheme.eq_ignore_ascii_case("http")
            || !openai_base_url_http_host_is_loopback(authority.host())
    })
}

fn openai_base_url_http_host_is_loopback(host: &str) -> bool {
    let ip_host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    host.eq_ignore_ascii_case("localhost")
        || ip_host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn proxy_url_has_userinfo(value: &str) -> std::result::Result<bool, ()> {
    if value.is_empty()
        || value.trim() != value
        || value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte == b'\\')
    {
        return Err(());
    }

    let authority_and_suffix = if let Some((scheme, remainder)) = value.split_once("://") {
        let mut scheme_bytes = scheme.bytes();
        if !scheme_bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic())
            || !scheme_bytes
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        {
            return Err(());
        }
        remainder
    } else if let Some(remainder) = value.strip_prefix("//") {
        remainder
    } else {
        value
    };
    let authority = authority_and_suffix
        .split(['/', '?', '#'])
        .next()
        .ok_or(())?;
    if authority.is_empty() {
        return Err(());
    }

    let bytes = authority.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'@' => return Ok(true),
            // Percent-encoded `@` is not RFC userinfo without a raw delimiter, but
            // rejecting it avoids disagreement with permissive downstream parsers.
            b'%' => {
                let encoded = bytes.get(index + 1..index + 3).ok_or(())?;
                if !encoded.iter().all(u8::is_ascii_hexdigit) {
                    return Err(());
                }
                let decoded = u8::from_str_radix(std::str::from_utf8(encoded).map_err(|_| ())?, 16)
                    .map_err(|_| ())?;
                if decoded == b'@' {
                    return Ok(true);
                }
                index += 2;
            }
            _ => {}
        }
        index += 1;
    }

    if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, suffix) = bracketed.split_once(']').ok_or(())?;
        if host.is_empty()
            || host.contains(['[', ']'])
            || (!suffix.is_empty()
                && (!suffix.starts_with(':')
                    || suffix[1..].is_empty()
                    || !suffix[1..].bytes().all(|byte| byte.is_ascii_digit())))
        {
            return Err(());
        }
    } else {
        if authority.contains(['[', ']']) {
            return Err(());
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        if host.is_empty()
            || host.contains(':')
            || port.is_some_and(|port| {
                port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err(());
        }
    }
    Ok(false)
}

impl ShellEnvironmentInherit {
    fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Core => "core",
            Self::All => "all",
        }
    }

    fn codex_override(self) -> String {
        format!("shell_environment_policy.inherit=\"{}\"", self.label())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ShellEnvironmentInheritResolution {
    inherit: ShellEnvironmentInherit,
    warning: Option<String>,
}

fn resolve_shell_environment_inherit(value: Option<&str>) -> ShellEnvironmentInheritResolution {
    let normalized = value.map(str::trim).map(str::to_ascii_lowercase);
    let inherit = match normalized.as_deref() {
        None => ShellEnvironmentInherit::Core,
        Some("none") => ShellEnvironmentInherit::None,
        Some("core") => ShellEnvironmentInherit::Core,
        Some("all") => ShellEnvironmentInherit::All,
        Some(_) => {
            return ShellEnvironmentInheritResolution {
                inherit: ShellEnvironmentInherit::Core,
                warning: Some(format!(
                    "invalid {SHELL_ENVIRONMENT_INHERIT_ENV_VAR}; expected none, core, or all; using core"
                )),
            };
        }
    };

    ShellEnvironmentInheritResolution {
        inherit,
        warning: None,
    }
}

fn configured_shell_environment_inherit() -> ShellEnvironmentInherit {
    let resolution = resolve_shell_environment_inherit(
        std::env::var(SHELL_ENVIRONMENT_INHERIT_ENV_VAR)
            .ok()
            .as_deref(),
    );
    if let Some(warning) = resolution.warning {
        // This runs before the TUI is fully attached, so stderr keeps configuration errors visible.
        eprintln!("warning: {warning}");
        tracing::warn!(warning = %warning, "invalid app-server shell environment policy");
    }
    resolution.inherit
}

fn app_server_command(config: &AppServerConnectionConfig) -> Command {
    app_server_command_with_environment(
        config,
        config.shell_environment_inherit,
        config.process_environment_policy,
        std::env::vars_os(),
    )
}

fn app_server_command_with_environment(
    config: &AppServerConnectionConfig,
    inherit: ShellEnvironmentInherit,
    process_environment_policy: ProcessEnvironmentPolicy,
    process_environment: impl IntoIterator<Item = (OsString, OsString)>,
) -> Command {
    /*
     * The app-server process uses the existing Codex login stored under HOME/CODEX_HOME by
     * default. An exact API-key-auth opt-in copies only the two supported auth keys into this
     * child process. Pin the upstream shell policy to the platform's core variables and keep
     * Codex's default KEY/SECRET/TOKEN exclusions active so those credentials are not projected
     * into generated tools. Login shells stay disabled because profile scripts can reintroduce
     * variables after that policy has built the initial tool environment.
     */
    let mut command = Command::new(&config.executable);
    command.args(&config.executable_prefix_args);
    if process_environment_policy == ProcessEnvironmentPolicy::Scrubbed {
        let filtered =
            filtered_app_server_process_environment(process_environment, config.api_key_auth);
        for environment_variable in filtered.dropped_credential_variables {
            let warning = format!(
                "scrubbed app-server process environment dropped unsafe or credential-bearing {environment_variable}; URL details were redacted"
            );
            eprintln!("warning: {warning}");
            tracing::warn!(
                environment_variable,
                "unsafe app-server URL environment was dropped"
            );
        }
        command.env_clear().envs(filtered.variables);
    }
    command
        .arg("app-server")
        .arg("-c")
        .arg(inherit.codex_override())
        .arg("-c")
        .arg(SHELL_ENVIRONMENT_SECRET_EXCLUDES_OVERRIDE)
        .arg("-c")
        .arg(DISABLE_LOGIN_SHELL_OVERRIDE)
        .envs(config.command_environment.iter().cloned());
    command
}

fn approval_denied_notice(method: &str, reason: &str) -> String {
    format!("app-server approval request `{method}` declined ({reason})")
}

fn declined_approval_result(method: &str) -> Value {
    if method == "item/permissions/requestApproval" {
        json!({ "permissions": {}, "scope": "turn" })
    } else {
        json!({ "decision": "decline" })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AppServerApprovalMode {
    Interactive,
    Unattended,
}

#[derive(Clone, Copy)]
struct ApprovalInterruptContext<'a> {
    signal: &'a AppServerTurnInterruptSignal,
    observed_generation: u64,
}

#[derive(Clone, Copy)]
struct RequestInterruptContext<'a> {
    signal: &'a AppServerTurnInterruptSignal,
    observed_generation: u64,
    method: &'a str,
}

#[derive(Clone, Copy)]
struct BoundApprovalContext<'a> {
    thread_id: &'a str,
    turn_id: &'a str,
    interrupt: ApprovalInterruptContext<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseWaitFailureKind {
    // A matching JSON-RPC error proves that the peer consumed this exact request,
    // so a bounded retry cannot be confused with a late response from the prior id.
    ExplicitRemoteError,
    // A matching response id proves request ownership even when its result
    // violates the method schema. The caller fails, but the stream stays usable.
    CorrelatedResponseInvalid,
    // The active turn reached an authoritative terminal notification before a
    // turn/steer acknowledgement. The caller cannot claim success, but the
    // stream must drain the already-buffered terminal instead of failing.
    TurnTerminalDeferred,
    // Timeout, transport, framing, and local serialization failures leave request
    // consumption ambiguous. The connection must not be reused for cancellation.
    CorrelationUnknown,
}

#[derive(Debug)]
struct ResponseWaitFailure {
    kind: ResponseWaitFailureKind,
    error: anyhow::Error,
}

impl ResponseWaitFailure {
    fn correlation_unknown(error: impl Into<anyhow::Error>) -> Self {
        Self {
            kind: ResponseWaitFailureKind::CorrelationUnknown,
            error: error.into(),
        }
    }

    fn explicit_remote_error(error: impl Into<anyhow::Error>) -> Self {
        Self {
            kind: ResponseWaitFailureKind::ExplicitRemoteError,
            error: error.into(),
        }
    }

    fn correlated_response_invalid(error: impl Into<anyhow::Error>) -> Self {
        Self {
            kind: ResponseWaitFailureKind::CorrelatedResponseInvalid,
            error: error.into(),
        }
    }

    fn turn_terminal_deferred(error: impl Into<anyhow::Error>) -> Self {
        Self {
            kind: ResponseWaitFailureKind::TurnTerminalDeferred,
            error: error.into(),
        }
    }

    fn into_error(self) -> anyhow::Error {
        self.error
    }
}

#[derive(Default)]
struct TransportFailure {
    message: Mutex<Option<String>>,
}

struct AppServerWriteRequest {
    frame: Vec<u8>,
    acknowledgement: SyncSender<std::result::Result<(), String>>,
}

struct AppServerStdinWriter {
    sender: Option<SyncSender<AppServerWriteRequest>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl AppServerStdinWriter {
    fn spawn(mut stdin: ChildStdin, transport_failure: Arc<TransportFailure>) -> Self {
        let (sender, receiver) =
            mpsc::sync_channel::<AppServerWriteRequest>(APP_SERVER_WRITE_CHANNEL_CAPACITY);
        let worker = thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                let result = stdin
                    .write_all(&request.frame)
                    .and_then(|()| stdin.flush())
                    .map_err(|error| {
                        format!(
                            "failed to write a bounded JSON-RPC frame to app-server stdin: {error}"
                        )
                    });
                if let Err(message) = &result {
                    transport_failure.record(message.clone());
                }
                let failed = result.is_err();
                let _ = request.acknowledgement.send(result);
                if failed {
                    return;
                }
            }
        });
        Self {
            sender: Some(sender),
            worker: Some(worker),
        }
    }

    fn try_send(
        &self,
        request: AppServerWriteRequest,
    ) -> std::result::Result<(), TrySendError<AppServerWriteRequest>> {
        let Some(sender) = self.sender.as_ref() else {
            return Err(TrySendError::Disconnected(request));
        };
        sender.try_send(request)
    }

    fn shutdown(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + APP_SERVER_WRITER_SHUTDOWN_TIMEOUT;
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                tracing::warn!(
                    timeout_ms = APP_SERVER_WRITER_SHUTDOWN_TIMEOUT.as_millis(),
                    "detaching an app-server stdin writer that did not exit after process termination"
                );
            }
        }
    }
}

impl TransportFailure {
    fn record(&self, message: impl Into<String>) {
        let mut stored = self
            .message
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if stored.is_none() {
            *stored = Some(message.into());
        }
    }

    fn current(&self) -> Option<String> {
        self.message
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

enum TurnStreamNotificationProgress {
    Continue,
    NonRetryErrorCandidate(ConversationTurnError),
    Terminal(ConversationTurnTerminalReceipt),
}

#[derive(Clone, Copy)]
enum BufferedTurnRecoveryMode {
    GraceExpired { deadline: Instant },
    TransportClosing,
}

impl BufferedTurnRecoveryMode {
    fn grace_expired(interrupt_completion_deadline: Option<Instant>) -> Self {
        Self::GraceExpired {
            deadline: bounded_grace_recovery_deadline(interrupt_completion_deadline),
        }
    }

    fn write_deadline(self) -> Option<Instant> {
        match self {
            Self::GraceExpired { deadline } => Some(deadline),
            Self::TransportClosing => None,
        }
    }

    fn is_transport_closing(self) -> bool {
        matches!(self, Self::TransportClosing)
    }
}

fn terminal_receipt_after_grace_deadline(
    thread_id: &str,
    turn_id: &str,
    error: ConversationTurnError,
    changed_planning_file_paths: Vec<String>,
) -> ConversationTurnTerminalReceipt {
    ConversationTurnTerminalReceipt::new(
        thread_id,
        turn_id,
        ConversationTurnTerminalOutcome::Unknown {
            reason: ConversationTurnTerminalUncertainty::NonRetryErrorGraceExpired,
            observed_error: Some(error),
        },
    )
    .with_turn_metadata(ConversationTurnItemsView::NotLoaded, None, None, None)
    .with_observations(ConversationTurnObservations::new(
        changed_planning_file_paths,
    ))
}

fn bounded_grace_recovery_deadline(interrupt_completion_deadline: Option<Instant>) -> Instant {
    let recovery_deadline = Instant::now() + MAX_GRACE_RECOVERY_TIME;
    interrupt_completion_deadline.map_or(recovery_deadline, |interrupt_deadline| {
        recovery_deadline.min(interrupt_deadline)
    })
}

fn bounded_transport_closing_deadline(interrupt_completion_deadline: Option<Instant>) -> Instant {
    let recovery_deadline = Instant::now() + MAX_TRANSPORT_CLOSING_RECOVERY_TIME;
    interrupt_completion_deadline.map_or(recovery_deadline, |interrupt_deadline| {
        recovery_deadline.min(interrupt_deadline)
    })
}

pub(super) struct AppServerConnection {
    /*
     * AppServerConnection owns the child handle and stdin writer, while stdout/stderr are moved into reader
     * threads. The mpsc receiver is therefore the single place where response lines, stream notifications, and
     * stderr diagnostics are serialized back into request/stream control flow.
     */
    child: ManagedChild,
    stdin_writer: AppServerStdinWriter,
    rx: Receiver<AppServerLine>,
    transport_failure: Arc<TransportFailure>,
    diagnostics: ConnectionDiagnostics,
    // Notifications observed while waiting for a normal response are buffered until the active turn stream can consume them.
    pending_notifications: PendingNotifications,
    next_request_id: i64,
    client_name: String,
    client_version: String,
    initialized: bool,
    config: AppServerConnectionConfig,
    terminal_recovery_write_deadline: Option<Instant>,
    approval_broker: Arc<AppServerApprovalBroker>,
    approval_mode: AppServerApprovalMode,
    interrupt_signal: AppServerTurnInterruptSignal,
}

impl AppServerConnection {
    pub(super) fn spawn(
        client_name: String,
        client_version: String,
        config: AppServerConnectionConfig,
        approval_broker: Arc<AppServerApprovalBroker>,
        approval_mode: AppServerApprovalMode,
        interrupt_signal: AppServerTurnInterruptSignal,
    ) -> Result<Self> {
        /*
         * stdin/stdout/stderr를 모두 piped로 열어야 app-server와 line protocol을 주고받을 수 있다.
         * stdout/stderr는 blocking read가 필요하므로 reader thread가 mpsc sender로 AppServerLine을 넘기고,
         * 이 connection object는 request id matching과 stream notification reduction만 수행한다.
         */
        config.ensure_executable_is_pinned()?;
        let mut command = app_server_command(&config);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = subprocess::spawn(&mut command).with_context(|| {
            format!(
                "failed to spawn `{} app-server`",
                config.executable.display()
            )
        })?;

        let stdin = child
            .take_stdin()
            .context("failed to take app-server stdin")?;
        let stdout = child
            .take_stdout()
            .context("failed to take app-server stdout")?;
        let stderr = child
            .take_stderr()
            .context("failed to take app-server stderr")?;

        let transport_failure = Arc::new(TransportFailure::default());
        let stdin_writer = AppServerStdinWriter::spawn(stdin, transport_failure.clone());
        let (tx, rx) = mpsc::sync_channel(APP_SERVER_LINE_CHANNEL_CAPACITY);
        spawn_pipe_reader(
            stdout,
            tx.clone(),
            false,
            MAX_STDOUT_LINE_BYTES,
            transport_failure.clone(),
        );
        spawn_pipe_reader(
            stderr,
            tx,
            true,
            MAX_STDERR_LINE_BYTES,
            transport_failure.clone(),
        );

        Ok(Self {
            child,
            stdin_writer,
            rx,
            transport_failure,
            diagnostics: ConnectionDiagnostics::default(),
            pending_notifications: PendingNotifications::default(),
            next_request_id: 1,
            client_name,
            client_version,
            initialized: false,
            config,
            terminal_recovery_write_deadline: None,
            approval_broker,
            approval_mode,
            interrupt_signal,
        })
    }

    pub(super) fn is_alive(&mut self) -> Result<bool> {
        self.ensure_transport_healthy()?;
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

    #[tracing::instrument(
        level = "trace",
        skip(self, params),
        fields(
            limit = ?params.limit,
            archived = ?params.archived,
            has_cwd = params.cwd.is_some(),
            has_search_term = params.search_term.is_some(),
            source_kind_count = params.source_kinds.as_ref().map_or(0, Vec::len),
        )
    )]
    pub(super) fn list_threads(&mut self, params: ThreadListParams) -> Result<ThreadListResponse> {
        self.ensure_initialized()?;
        self.send_request("thread/list", serde_json::to_value(params)?)
    }

    pub(super) fn set_thread_name(&mut self, params: ThreadSetNameParams) -> Result<()> {
        self.ensure_initialized()?;
        let _: ThreadSetNameResponse =
            self.send_request("thread/name/set", serde_json::to_value(params)?)?;
        Ok(())
    }

    #[tracing::instrument(
        level = "trace",
        skip(self, thread_id),
        fields(include_turns = include_turns)
    )]
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

    #[tracing::instrument(level = "trace", skip(self, params))]
    pub(super) fn start_thread(
        &mut self,
        params: ThreadStartParams,
    ) -> Result<ThreadStartResponse> {
        self.ensure_initialized()?;
        self.send_request("thread/start", serde_json::to_value(params)?)
    }

    #[tracing::instrument(
        level = "trace",
        skip(self, params),
        fields(
            has_approval_policy = params.approval_policy.is_some(),
            has_reviewer = params.approvals_reviewer.is_some(),
            has_sandbox = params.sandbox.is_some(),
        )
    )]
    pub(super) fn resume_thread(
        &mut self,
        params: ThreadResumeParams,
    ) -> Result<ThreadResumeResponse> {
        self.ensure_initialized()?;
        self.send_request("thread/resume", serde_json::to_value(params)?)
    }

    #[tracing::instrument(level = "trace", skip(self, thread_id))]
    pub(super) fn archive_thread(&mut self, thread_id: &str) -> Result<()> {
        self.ensure_initialized()?;
        let _: Value = self.send_request(
            "thread/archive",
            json!({
                "threadId": thread_id,
            }),
        )?;
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self, params))]
    #[cfg(test)]
    pub(super) fn start_turn(&mut self, params: TurnStartParams) -> Result<TurnStartResponse> {
        self.ensure_initialized()?;
        self.send_request("turn/start", serde_json::to_value(params)?)
    }

    #[tracing::instrument(level = "trace", skip(self, params, event_sender, interrupt_signal))]
    pub(super) fn start_turn_with_event_sender(
        &mut self,
        params: TurnStartParams,
        event_sender: &dyn AppServerEventSender,
        interrupt_signal: &AppServerTurnInterruptSignal,
        observed_interrupt_generation: u64,
    ) -> Result<TurnStartResponse> {
        self.ensure_initialized()?;
        self.send_request_with_event_sender_and_interrupt(
            "turn/start",
            serde_json::to_value(params)?,
            event_sender,
            ApprovalInterruptContext {
                signal: interrupt_signal,
                observed_generation: observed_interrupt_generation,
            },
        )
    }

    #[tracing::instrument(level = "trace", skip(self, params))]
    #[cfg(test)]
    pub(super) fn interrupt_turn(
        &mut self,
        params: TurnInterruptParams,
    ) -> Result<TurnInterruptResponse> {
        self.ensure_initialized()?;
        self.send_request("turn/interrupt", serde_json::to_value(params)?)
    }

    fn interrupt_turn_with_event_sender_classified(
        &mut self,
        params: TurnInterruptParams,
        event_sender: &dyn AppServerEventSender,
        approval_context: BoundApprovalContext<'_>,
        response_timeout: Duration,
    ) -> std::result::Result<TurnInterruptResponse, ResponseWaitFailure> {
        self.ensure_initialized()
            .map_err(ResponseWaitFailure::correlation_unknown)?;
        self.send_request_with_approval_context_classified(
            "turn/interrupt",
            serde_json::to_value(params).map_err(ResponseWaitFailure::correlation_unknown)?,
            Some(event_sender),
            Some(approval_context),
            None,
            response_timeout,
        )
    }

    fn steer_turn_with_event_sender_classified(
        &mut self,
        params: TurnSteerParams,
        event_sender: &dyn AppServerEventSender,
        approval_context: BoundApprovalContext<'_>,
        interrupt_context: ApprovalInterruptContext<'_>,
    ) -> std::result::Result<TurnSteerResponse, ResponseWaitFailure> {
        self.ensure_initialized()
            .map_err(ResponseWaitFailure::correlation_unknown)?;
        let response_timeout = self.config.response_timeout;
        let response_value: Value = self.send_request_with_approval_context_classified(
            "turn/steer",
            serde_json::to_value(params).map_err(ResponseWaitFailure::correlation_unknown)?,
            Some(event_sender),
            Some(approval_context),
            Some(RequestInterruptContext {
                signal: interrupt_context.signal,
                observed_generation: interrupt_context.observed_generation,
                method: "turn/steer",
            }),
            response_timeout,
        )?;
        serde_json::from_value(response_value)
            .context("failed to deserialize app-server response for turn/steer")
            .map_err(ResponseWaitFailure::correlated_response_invalid)
    }

    pub(super) fn wait_for_turn_stream(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        interrupt_signal: &AppServerTurnInterruptSignal,
        observed_interrupt_generation: u64,
        event_sender: &dyn AppServerEventSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        self.wait_for_turn_stream_inner(
            thread_id,
            turn_id,
            interrupt_signal,
            observed_interrupt_generation,
            event_sender,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn wait_for_turn_stream_with_steering(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        interrupt_signal: &AppServerTurnInterruptSignal,
        observed_interrupt_generation: u64,
        event_sender: &dyn AppServerEventSender,
        broker: &AppServerTurnSteerBroker,
        binding: AppServerTurnSteerBinding,
    ) -> Result<ConversationTurnTerminalReceipt> {
        let binding_id = binding.binding_id();
        let result = self.wait_for_turn_stream_inner(
            thread_id,
            turn_id,
            interrupt_signal,
            observed_interrupt_generation,
            event_sender,
            Some((broker, &binding)),
        );
        broker.unbind(
            binding_id,
            if result.is_ok() {
                "active turn completed before steering was applied"
            } else {
                "active turn connection ended before steering was applied"
            },
        );
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn wait_for_turn_stream_inner(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        interrupt_signal: &AppServerTurnInterruptSignal,
        observed_interrupt_generation: u64,
        event_sender: &dyn AppServerEventSender,
        mut turn_steering: Option<(&AppServerTurnSteerBroker, &AppServerTurnSteerBinding)>,
    ) -> Result<ConversationTurnTerminalReceipt> {
        /*
         * Turn streaming interleaves three input sources: child process exit, global interrupt generation, and
         * stdout/stderr lines. Pending notifications are drained before blocking on rx so notifications that arrived
         * during `turn/start` response waiting are not delayed until a new line appears.
         */
        let mut notification_state = ActiveTurnNotificationState::new();
        let mut non_retry_error_candidate = None;
        let mut interrupt_sent = false;
        let mut interrupt_completion_deadline = None;
        let mut last_interrupt_generation_attempted = observed_interrupt_generation;

        loop {
            if let Some(receipt) = self.drain_pending_turn_notifications(
                thread_id,
                turn_id,
                &mut notification_state,
                event_sender,
                &mut non_retry_error_candidate,
            )? {
                return Ok(self.deliver_terminal_receipt(
                    receipt,
                    thread_id,
                    &mut notification_state,
                    event_sender,
                ));
            }
            if !self.pending_notifications.is_empty() {
                self.advance_turn_interrupt_state(
                    thread_id,
                    turn_id,
                    event_sender,
                    interrupt_signal,
                    observed_interrupt_generation,
                    &mut interrupt_sent,
                    &mut interrupt_completion_deadline,
                    &mut last_interrupt_generation_attempted,
                )?;
                continue;
            }

            if self.transport_failure.current().is_some() {
                if let Some(receipt) = self.drain_transport_closing_turn_lines(
                    thread_id,
                    turn_id,
                    &mut notification_state,
                    event_sender,
                    &mut non_retry_error_candidate,
                    interrupt_completion_deadline,
                )? {
                    return Ok(self.deliver_terminal_receipt(
                        receipt,
                        thread_id,
                        &mut notification_state,
                        event_sender,
                    ));
                }
                self.ensure_transport_healthy()?;
            }
            if let Some(status) = self.child.try_wait()? {
                if let Some(receipt) = self.drain_transport_closing_turn_lines(
                    thread_id,
                    turn_id,
                    &mut notification_state,
                    event_sender,
                    &mut non_retry_error_candidate,
                    interrupt_completion_deadline,
                )? {
                    return Ok(self.deliver_terminal_receipt(
                        receipt,
                        thread_id,
                        &mut notification_state,
                        event_sender,
                    ));
                }
                return Err(self.error_with_diagnostics(format!(
                    "app-server exited before the turn completed: {status}"
                )));
            }

            self.advance_turn_interrupt_state(
                thread_id,
                turn_id,
                event_sender,
                interrupt_signal,
                observed_interrupt_generation,
                &mut interrupt_sent,
                &mut interrupt_completion_deadline,
                &mut last_interrupt_generation_attempted,
            )?;
            if !self.pending_notifications.is_empty() {
                continue;
            }

            if let Some((broker, binding)) = turn_steering {
                if interrupt_sent || non_retry_error_candidate.is_some() {
                    broker.unbind(
                        binding.binding_id(),
                        "active turn is no longer accepting steering",
                    );
                    turn_steering = None;
                } else {
                    self.process_pending_turn_steering(
                        thread_id,
                        turn_id,
                        event_sender,
                        interrupt_signal,
                        observed_interrupt_generation,
                        broker,
                        binding,
                    )?;
                }
            }

            let grace_expired = non_retry_error_candidate
                .as_ref()
                .is_some_and(|(_, deadline)| Instant::now() >= *deadline);
            if grace_expired {
                if let Some(receipt) = self.scan_grace_expired_turn_lines(
                    thread_id,
                    turn_id,
                    &mut notification_state,
                    event_sender,
                    &mut non_retry_error_candidate,
                    interrupt_completion_deadline,
                )? {
                    return Ok(self.deliver_terminal_receipt(
                        receipt,
                        thread_id,
                        &mut notification_state,
                        event_sender,
                    ));
                }
                // The recovery scan and its no-UI response share the interrupt
                // deadline. Re-check it before synthesizing Unknown so an explicit
                // stop cannot be converted into a grace-expired receipt.
                self.advance_turn_interrupt_state(
                    thread_id,
                    turn_id,
                    event_sender,
                    interrupt_signal,
                    observed_interrupt_generation,
                    &mut interrupt_sent,
                    &mut interrupt_completion_deadline,
                    &mut last_interrupt_generation_attempted,
                )?;
                if !self.pending_notifications.is_empty() {
                    continue;
                }
                let error = non_retry_error_candidate
                    .as_ref()
                    .expect("expired grace candidate remains present after buffered line drain")
                    .0
                    .clone();
                let receipt = terminal_receipt_after_grace_deadline(
                    thread_id,
                    turn_id,
                    error,
                    notification_state.changed_planning_file_paths().to_vec(),
                );
                return Ok(self.deliver_terminal_receipt(
                    receipt,
                    thread_id,
                    &mut notification_state,
                    event_sender,
                ));
            }

            let now = Instant::now();
            let mut receive_timeout = self.config.poll_interval;
            if let Some(deadline) = interrupt_completion_deadline {
                receive_timeout = receive_timeout.min(deadline.saturating_duration_since(now));
            }
            if let Some((_, deadline)) = non_retry_error_candidate.as_ref() {
                receive_timeout = receive_timeout.min(deadline.saturating_duration_since(now));
            }
            let received = self.rx.recv_timeout(receive_timeout);
            match received {
                Ok(line) => {
                    let progress = self.process_turn_stream_line(
                        line,
                        thread_id,
                        turn_id,
                        &mut notification_state,
                        event_sender,
                        interrupt_signal,
                        observed_interrupt_generation,
                    )?;
                    if let Some(receipt) = self
                        .apply_turn_notification_progress(progress, &mut non_retry_error_candidate)
                    {
                        return Ok(self.deliver_terminal_receipt(
                            receipt,
                            thread_id,
                            &mut notification_state,
                            event_sender,
                        ));
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => self.ensure_transport_healthy()?,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.ensure_transport_healthy()?;
                    return Err(self.error_with_diagnostics(
                        "app-server pipe closed while waiting for turn events",
                    ));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn process_pending_turn_steering(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        event_sender: &dyn AppServerEventSender,
        interrupt_signal: &AppServerTurnInterruptSignal,
        observed_interrupt_generation: u64,
        broker: &AppServerTurnSteerBroker,
        binding: &AppServerTurnSteerBinding,
    ) -> Result<()> {
        let Some(command) = binding.try_receive()? else {
            return Ok(());
        };
        let binding_id = command.binding_id();
        let request_id = command.request_id();
        if binding_id != binding.binding_id()
            || command.request.thread_id != thread_id
            || command.request.expected_turn_id != turn_id
        {
            self.complete_turn_steer_caller(
                broker,
                binding_id,
                request_id,
                Err("turn steering command no longer matched the active turn".to_string()),
            );
            return Ok(());
        }

        let interrupt_context = ApprovalInterruptContext {
            signal: interrupt_signal,
            observed_generation: observed_interrupt_generation,
        };
        let response = self.steer_turn_with_event_sender_classified(
            TurnSteerParams {
                thread_id: command.request.thread_id,
                input: vec![super::protocol::TurnInputItem::text(command.request.prompt)],
                expected_turn_id: command.request.expected_turn_id,
            },
            event_sender,
            BoundApprovalContext {
                thread_id,
                turn_id,
                interrupt: interrupt_context,
            },
            interrupt_context,
        );

        match response {
            Ok(response) if response.turn_id == turn_id => {
                self.complete_turn_steer_caller(
                    broker,
                    binding_id,
                    request_id,
                    Ok(ConversationTurnSteerReceipt {
                        turn_id: response.turn_id,
                    }),
                );
                Ok(())
            }
            Ok(_) => {
                self.diagnostics.record_warning(
                    "app-server turn/steer response did not match the active turn".to_string(),
                );
                self.complete_turn_steer_caller(
                    broker,
                    binding_id,
                    request_id,
                    Err("turn/steer response turnId did not match the active turn".to_string()),
                );
                Ok(())
            }
            Err(failure) => {
                let failure_kind = failure.kind;
                let error = failure.into_error();
                self.complete_turn_steer_caller(
                    broker,
                    binding_id,
                    request_id,
                    Err(error.to_string()),
                );
                match failure_kind {
                    ResponseWaitFailureKind::ExplicitRemoteError => {
                        self.diagnostics.record_warning(
                            "app-server rejected turn/steer; the active stream remains connected"
                                .to_string(),
                        );
                        Ok(())
                    }
                    ResponseWaitFailureKind::CorrelatedResponseInvalid => {
                        self.diagnostics.record_warning(
                            "app-server returned an invalid turn/steer response; the active stream remains connected"
                                .to_string(),
                        );
                        Ok(())
                    }
                    ResponseWaitFailureKind::TurnTerminalDeferred => Ok(()),
                    ResponseWaitFailureKind::CorrelationUnknown => Err(error),
                }
            }
        }
    }

    fn complete_turn_steer_caller(
        &mut self,
        broker: &AppServerTurnSteerBroker,
        binding_id: u64,
        request_id: u64,
        result: std::result::Result<ConversationTurnSteerReceipt, String>,
    ) {
        if broker.complete(binding_id, request_id, result).is_err() {
            self.diagnostics.record_warning(
                "turn steering result arrived after its caller stopped waiting".to_string(),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn advance_turn_interrupt_state(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        event_sender: &dyn AppServerEventSender,
        interrupt_signal: &AppServerTurnInterruptSignal,
        observed_interrupt_generation: u64,
        interrupt_sent: &mut bool,
        interrupt_completion_deadline: &mut Option<Instant>,
        last_interrupt_generation_attempted: &mut u64,
    ) -> Result<()> {
        let interrupt_generation = interrupt_signal.current_generation();
        if !*interrupt_sent
            && interrupt_generation > observed_interrupt_generation
            && interrupt_generation > *last_interrupt_generation_attempted
        {
            /*
             * The interrupt signal is process-wide, while this loop owns exactly one
             * active turn. After translating the first newer generation into
             * `turn/interrupt`, later increments are left as UI intent instead of
             * repeatedly sending the same app-server method for this turn id.
             */
            *last_interrupt_generation_attempted = interrupt_generation;
            *interrupt_completion_deadline = Some(self.interrupt_active_turn(
                thread_id,
                turn_id,
                event_sender,
                interrupt_signal,
                observed_interrupt_generation,
            )?);
            *interrupt_sent = true;
        }

        if interrupt_completion_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            let message = "app-server acknowledged the stop request but did not terminate the active turn within the bounded interrupt deadline; terminating the active app-server process tree";
            return Err(self.fail_closed_interrupt(event_sender, message));
        }

        Ok(())
    }

    fn interrupt_active_turn(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        event_sender: &dyn AppServerEventSender,
        interrupt_signal: &AppServerTurnInterruptSignal,
        observed_interrupt_generation: u64,
    ) -> Result<Instant> {
        /*
         * An explicit stop is authoritative. A matching remote error may be retried
         * inside one short total deadline, but timeout/framing/transport failures make
         * JSON-RPC correlation ambiguous and therefore close the whole connection.
         */
        let retry_limit = self.config.interrupt_retry_limit.max(1);
        let interrupt_deadline = Instant::now() + self.config.interrupt_total_timeout;
        for attempt in 1..=retry_limit {
            let now = Instant::now();
            if now >= interrupt_deadline {
                break;
            }
            let remaining_timeout = interrupt_deadline.saturating_duration_since(now);
            match self.interrupt_turn_with_event_sender_classified(
                TurnInterruptParams {
                    thread_id: thread_id.to_string(),
                    turn_id: turn_id.to_string(),
                },
                event_sender,
                BoundApprovalContext {
                    thread_id,
                    turn_id,
                    interrupt: ApprovalInterruptContext {
                        signal: interrupt_signal,
                        observed_generation: observed_interrupt_generation,
                    },
                },
                remaining_timeout,
            ) {
                Ok(_) => {
                    let _ = event_sender.send(ConversationStreamEvent::StatusUpdated {
                        text: "stop requested / app-server interrupt sent".to_string(),
                    });
                    return Ok(interrupt_deadline);
                }
                Err(failure) => {
                    self.diagnostics.record_warning(format!(
                        "app-server interrupt attempt {attempt}/{retry_limit} failed for active turn (error_chars={}, chain_depth={})",
                        failure.error.to_string().chars().count(),
                        failure.error.chain().count()
                    ));
                    if failure.kind == ResponseWaitFailureKind::CorrelationUnknown {
                        break;
                    }
                    if attempt < retry_limit && Instant::now() < interrupt_deadline {
                        let multiplier = u32::try_from(attempt).unwrap_or(u32::MAX);
                        let retry_delay = self
                            .config
                            .interrupt_retry_backoff
                            .saturating_mul(multiplier)
                            .min(interrupt_deadline.saturating_duration_since(Instant::now()));
                        thread::sleep(retry_delay);
                    }
                }
            }
        }
        let message = "stop request failed within the bounded app-server interrupt deadline; terminating the active app-server process tree";
        Err(self.fail_closed_interrupt(event_sender, message))
    }

    fn fail_closed_interrupt(
        &mut self,
        event_sender: &dyn AppServerEventSender,
        message: &str,
    ) -> anyhow::Error {
        let _ = event_sender.send(ConversationStreamEvent::TurnInterruptRequestFailed {
            message: message.to_string(),
        });
        self.fail_transport(message)
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

    pub(super) fn discard_notifications_before_turn_binding(&mut self) {
        let warnings = self.pending_notifications.drain_warning_texts_with_context(
            "before active turn binding; the later thread response remains authoritative",
        );
        self.diagnostics.record_warnings(warnings);
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
        self.send_request_with_event_sender(method, params, None)
    }

    fn send_request_with_event_sender<T>(
        &mut self,
        method: &str,
        params: Value,
        event_sender: Option<&dyn AppServerEventSender>,
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.send_request_with_approval_context(method, params, event_sender, None)
    }

    fn send_request_with_event_sender_and_interrupt<T>(
        &mut self,
        method: &str,
        params: Value,
        event_sender: &dyn AppServerEventSender,
        interrupt_context: ApprovalInterruptContext<'_>,
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let response_timeout = self.config.response_timeout;
        self.send_request_with_approval_context_classified(
            method,
            params,
            Some(event_sender),
            None,
            Some(RequestInterruptContext {
                signal: interrupt_context.signal,
                observed_generation: interrupt_context.observed_generation,
                method,
            }),
            response_timeout,
        )
        .map_err(ResponseWaitFailure::into_error)
    }

    fn send_request_with_approval_context<T>(
        &mut self,
        method: &str,
        params: Value,
        event_sender: Option<&dyn AppServerEventSender>,
        approval_context: Option<BoundApprovalContext<'_>>,
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let response_timeout = self.config.response_timeout;
        let interrupt_signal = self.interrupt_signal.clone();
        let observed_generation = interrupt_signal.current_generation();
        self.send_request_with_approval_context_classified(
            method,
            params,
            event_sender,
            approval_context,
            Some(RequestInterruptContext {
                signal: &interrupt_signal,
                observed_generation,
                method,
            }),
            response_timeout,
        )
        .map_err(ResponseWaitFailure::into_error)
    }

    fn send_request_with_approval_context_classified<T>(
        &mut self,
        method: &str,
        params: Value,
        event_sender: Option<&dyn AppServerEventSender>,
        approval_context: Option<BoundApprovalContext<'_>>,
        response_interrupt_context: Option<RequestInterruptContext<'_>>,
        response_timeout: Duration,
    ) -> std::result::Result<T, ResponseWaitFailure>
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

        self.send_json_line_with_timeout(
            json!({
                "id": request_id,
                "method": method,
                "params": params,
            }),
            response_timeout,
            response_interrupt_context,
            event_sender,
        )
        .map_err(ResponseWaitFailure::correlation_unknown)?;

        let response_value = self.wait_for_response_with_event_sender_classified(
            request_id,
            event_sender,
            approval_context,
            response_interrupt_context,
            response_timeout,
        )?;
        serde_json::from_value(response_value)
            .with_context(|| format!("failed to deserialize app-server response for `{method}`"))
            .map_err(ResponseWaitFailure::correlation_unknown)
    }

    fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        self.send_json_line(json!({
            "method": method,
            "params": params,
        }))
    }

    fn send_json_line(&mut self, value: Value) -> Result<()> {
        if let Some(deadline) = self.terminal_recovery_write_deadline {
            return self.send_json_line_with_terminal_recovery_deadline(value, deadline);
        }
        self.send_json_line_with_timeout(value, self.config.response_timeout, None, None)
    }

    fn send_json_line_with_terminal_recovery_deadline(
        &mut self,
        value: Value,
        deadline: Instant,
    ) -> Result<()> {
        /*
         * Terminal recovery must not inherit the normal three-second write cap.
         * It shares one absolute deadline with the bounded line scan and only
         * taints the transport on failure. The typed terminal/unknown receipt can
         * then leave this stack before the next request performs process cleanup.
         */
        if let Some(message) = self.transport_failure.current() {
            return Err(self.error_with_diagnostics(format!(
                "app-server transport failed during terminal recovery: {message}"
            )));
        }
        let mut frame = serde_json::to_vec(&value)?;
        frame.push(b'\n');
        if frame.len() > MAX_APP_SERVER_WRITE_FRAME_BYTES {
            return Err(self.taint_terminal_recovery_transport(format!(
                "app-server terminal-recovery JSON-RPC frame exceeded the {MAX_APP_SERVER_WRITE_FRAME_BYTES}-byte limit"
            )));
        }
        let (acknowledgement, receiver) = mpsc::sync_channel(1);
        match self.stdin_writer.try_send(AppServerWriteRequest {
            frame,
            acknowledgement,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(self.taint_terminal_recovery_transport(
                    "app-server stdin writer queue was occupied during terminal recovery",
                ));
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err(self.taint_terminal_recovery_transport(
                    "app-server stdin writer disconnected during terminal recovery",
                ));
            }
        }

        match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(self.taint_terminal_recovery_transport(message)),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(self.taint_terminal_recovery_transport(
                "app-server did not acknowledge a terminal-recovery JSON-RPC frame before its bounded deadline",
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(
                self.taint_terminal_recovery_transport(
                    "app-server stdin writer closed during terminal recovery without acknowledging the frame",
                ),
            ),
        }
    }

    fn send_json_line_with_timeout(
        &mut self,
        value: Value,
        timeout: Duration,
        interrupt_context: Option<RequestInterruptContext<'_>>,
        event_sender: Option<&dyn AppServerEventSender>,
    ) -> Result<()> {
        /*
         * app-server speaks newline-delimited JSON over stdio, not a framed socket.
         * A dedicated writer owns ChildStdin so a peer that stops reading cannot pin
         * the control thread inside a blocking OS write. Every frame has one bounded
         * acknowledgement; timeout, stop, queue saturation, or writer failure poisons
         * the connection and terminates the child before returning.
         */
        self.ensure_transport_healthy()?;
        let mut frame = serde_json::to_vec(&value)?;
        frame.push(b'\n');
        if frame.len() > MAX_APP_SERVER_WRITE_FRAME_BYTES {
            return Err(self.fail_transport(format!(
                "app-server JSON-RPC write frame exceeded the {MAX_APP_SERVER_WRITE_FRAME_BYTES}-byte limit"
            )));
        }
        let (acknowledgement, receiver) = mpsc::sync_channel(1);
        match self.stdin_writer.try_send(AppServerWriteRequest {
            frame,
            acknowledgement,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(self.fail_transport(
                    "app-server stdin writer queue remained occupied by an unacknowledged frame",
                ));
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err(self.fail_transport("app-server stdin writer disconnected"));
            }
        }

        let timeout = timeout.min(MAX_APP_SERVER_WRITE_ACK_TIMEOUT);
        let started_at = Instant::now();
        loop {
            self.ensure_transport_healthy()?;
            if interrupt_context
                .is_some_and(|context| context.signal.requested_after(context.observed_generation))
            {
                let method = interrupt_context
                    .map(|context| context.method)
                    .unwrap_or("JSON-RPC request");
                let message = format!(
                    "stop requested while app-server was accepting `{method}`; terminating the active app-server process tree"
                );
                if let Some(event_sender) = event_sender {
                    let _ =
                        event_sender.send(ConversationStreamEvent::TurnInterruptRequestFailed {
                            message: message.clone(),
                        });
                }
                return Err(self.fail_transport(message));
            }
            let elapsed = started_at.elapsed();
            if elapsed >= timeout {
                return Err(self.fail_transport(format!(
                    "timed out writing an app-server JSON-RPC frame after {}s",
                    timeout.as_secs_f64()
                )));
            }
            let wait = self
                .config
                .poll_interval
                .min(timeout.saturating_sub(elapsed));
            match receiver.recv_timeout(wait) {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(message)) => return Err(self.fail_transport(message)),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(self.fail_transport(
                        "app-server stdin writer closed without acknowledging the frame",
                    ));
                }
            }
        }
    }

    #[cfg(test)]
    fn wait_for_response(&mut self, request_id: i64) -> Result<Value> {
        self.wait_for_response_with_event_sender(request_id, None, None)
    }

    #[cfg(test)]
    fn wait_for_response_with_event_sender(
        &mut self,
        request_id: i64,
        event_sender: Option<&dyn AppServerEventSender>,
        approval_context: Option<BoundApprovalContext<'_>>,
    ) -> Result<Value> {
        let response_timeout = self.config.response_timeout;
        self.wait_for_response_with_event_sender_classified(
            request_id,
            event_sender,
            approval_context,
            None,
            response_timeout,
        )
        .map_err(ResponseWaitFailure::into_error)
    }

    fn wait_for_response_with_event_sender_classified(
        &mut self,
        request_id: i64,
        event_sender: Option<&dyn AppServerEventSender>,
        approval_context: Option<BoundApprovalContext<'_>>,
        response_interrupt_context: Option<RequestInterruptContext<'_>>,
        response_timeout: Duration,
    ) -> std::result::Result<Value, ResponseWaitFailure> {
        /*
         * Normal requests wait for exactly one matching response id. Notifications that arrive during this wait are
         * either downgraded to diagnostics or buffered for the turn stream if they are stream-owned methods.
         */
        let mut deadline = Instant::now() + response_timeout;

        loop {
            self.ensure_transport_healthy()
                .map_err(ResponseWaitFailure::correlation_unknown)?;
            if response_interrupt_context
                .is_some_and(|context| context.signal.requested_after(context.observed_generation))
            {
                let method = response_interrupt_context
                    .map(|context| context.method)
                    .unwrap_or("JSON-RPC request");
                let message = format!(
                    "stop requested before app-server acknowledged `{method}`; terminating the active app-server process tree"
                );
                if let Some(event_sender) = event_sender {
                    let _ =
                        event_sender.send(ConversationStreamEvent::TurnInterruptRequestFailed {
                            message: message.clone(),
                        });
                }
                return Err(ResponseWaitFailure::correlation_unknown(
                    self.fail_transport(message),
                ));
            }
            if Instant::now() > deadline {
                return Err(ResponseWaitFailure::correlation_unknown(
                    self.error_with_diagnostics(format!(
                        "timed out waiting for app-server response id={request_id} after {}s",
                        response_timeout.as_secs_f64()
                    )),
                ));
            }

            if let Some(status) = self
                .child
                .try_wait()
                .map_err(ResponseWaitFailure::correlation_unknown)?
            {
                return Err(ResponseWaitFailure::correlation_unknown(
                    self.error_with_diagnostics(format!(
                        "app-server exited early with status {status}"
                    )),
                ));
            }

            let received = self.rx.recv_timeout(
                self.config
                    .poll_interval
                    .min(deadline.saturating_duration_since(Instant::now())),
            );
            self.ensure_transport_healthy()
                .map_err(ResponseWaitFailure::correlation_unknown)?;
            match received {
                Ok(AppServerLine::Stderr(line)) => self.diagnostics.record_stderr(line),
                Ok(AppServerLine::ReaderFinished {
                    source: AppServerReaderSource::Stderr,
                    termination,
                }) => self.diagnostics.record_warning(format!(
                    "app-server stderr reader {} while waiting for a response",
                    termination.description()
                )),
                Ok(AppServerLine::ReaderFinished {
                    source: AppServerReaderSource::Stdout,
                    termination,
                }) => {
                    return Err(ResponseWaitFailure::correlation_unknown(
                        self.fail_transport(format!(
                            "app-server stdout reader {} while waiting for response id={request_id}",
                            termination.description()
                        )),
                    ));
                }
                Ok(AppServerLine::Stdout(line)) => {
                    let value = self
                        .parse_json_line(&line)
                        .map_err(ResponseWaitFailure::correlation_unknown)?;

                    let interactive_approval_started_at = value
                        .get("method")
                        .and_then(Value::as_str)
                        .filter(|method| INTERACTIVE_APPROVAL_METHODS.contains(method))
                        .map(|_| Instant::now());
                    if self
                        .handle_server_request(&value, event_sender, approval_context)
                        .map_err(ResponseWaitFailure::correlation_unknown)?
                    {
                        if let Some(started_at) = interactive_approval_started_at {
                            deadline += started_at.elapsed();
                        }
                        // Operator review has its own bounded deadline. Other server
                        // requests remain inside the enclosing response budget.
                        continue;
                    }

                    if let Some(response_id) = value.get("id").and_then(Value::as_i64) {
                        if response_id != request_id {
                            self.diagnostics.record_warning(format!(
                                "app-server returned response id={response_id} while waiting for id={request_id}"
                            ));
                            continue;
                        }

                        if let Some(error) = value.get("error") {
                            return Err(ResponseWaitFailure::explicit_remote_error(
                                self.error_with_diagnostics(format!(
                                    "app-server returned error for id {request_id}: {error}"
                                )),
                            ));
                        }

                        if let Some(result) = value.get("result") {
                            return Ok(result.clone());
                        }

                        return Err(ResponseWaitFailure::correlated_response_invalid(
                            self.error_with_diagnostics(format!(
                                "app-server returned response id {request_id} without a result payload"
                            )),
                        ));
                    }

                    if let Some(notification) = AppServerNotification::from_value(value) {
                        let terminal_preempts_steer_response = response_interrupt_context
                            .is_some_and(|context| context.method == "turn/steer")
                            && approval_context.is_some_and(|context| {
                                Self::notification_matches_bound_terminal(&notification, context)
                            });
                        self.handle_response_wait_notification(request_id, notification)
                            .map_err(ResponseWaitFailure::correlation_unknown)?;
                        if terminal_preempts_steer_response {
                            return Err(ResponseWaitFailure::turn_terminal_deferred(
                                anyhow::anyhow!(
                                    "active turn completed before app-server confirmed turn/steer; draft kept"
                                ),
                            ));
                        }
                        continue;
                    }

                    self.diagnostics.record_warning(format!(
                        "app-server sent an unexpected JSON message while waiting for response id={request_id}"
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(ResponseWaitFailure::correlation_unknown(
                        self.error_with_diagnostics(format!(
                            "app-server pipe closed while waiting for response id={request_id}"
                        )),
                    ));
                }
            }
        }
    }

    fn notification_matches_bound_terminal(
        notification: &AppServerNotification,
        context: BoundApprovalContext<'_>,
    ) -> bool {
        if notification.method() != "turn/completed" {
            return false;
        }
        let params = notification.params();
        let observed_thread_id = params.get("threadId").and_then(Value::as_str);
        let observed_turn_id = params
            .get("turn")
            .and_then(Value::as_object)
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str);
        observed_thread_id.is_none_or(|thread_id| thread_id == context.thread_id)
            && observed_turn_id.is_none_or(|turn_id| turn_id == context.turn_id)
    }

    fn handle_response_wait_notification(
        &mut self,
        request_id: i64,
        notification: AppServerNotification,
    ) -> Result<()> {
        /*
         * Stream-owned notifications can legitimately race ahead of the `turn/start`
         * response. Buffering them preserves app-server arrival order across the
         * response-to-stream handoff instead of converting early deltas into warnings.
         */
        if notification.should_defer_to_turn_stream() {
            return self.defer_turn_notification(notification);
        }

        self.diagnostics.record_warning(
            notification.warning_text(&format!("while waiting for response id={request_id}")),
        );
        Ok(())
    }

    fn defer_turn_notification(&mut self, notification: AppServerNotification) -> Result<()> {
        if self.pending_notifications.try_push(notification) {
            return Ok(());
        }

        Err(self.fail_transport(format!(
            "app-server exceeded the bounded pending turn-notification queue before the active stream could consume it (entries={MAX_PENDING_NOTIFICATIONS}, bytes={MAX_PENDING_NOTIFICATION_BYTES})"
        )))
    }

    fn handle_server_request(
        &mut self,
        value: &Value,
        event_sender: Option<&dyn AppServerEventSender>,
        approval_context: Option<BoundApprovalContext<'_>>,
    ) -> Result<bool> {
        /*
         * app-server is a bidirectional JSON-RPC peer. A message with both `id` and
         * `method` is a request from the server, not a response to one of Akra's ids.
         * Stable approval methods are routed through a one-shot broker. All other
         * server requests remain explicit decline/error paths so a new upstream
         * method can never inherit approval behavior accidentally.
         */
        let Some(request_id) = value.get("id").filter(|id| !id.is_null()) else {
            return Ok(false);
        };
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            return Ok(false);
        };

        match method {
            method if INTERACTIVE_APPROVAL_METHODS.contains(&method) => {
                self.handle_interactive_approval_request(
                    request_id,
                    method,
                    value.get("params"),
                    event_sender,
                    approval_context,
                )?;
            }
            method if UNINSPECTABLE_APPROVAL_METHODS.contains(&method) => {
                self.send_rejected_server_request(
                    json!({
                        "id": request_id,
                        "result": { "decision": "decline" },
                    }),
                    approval_denied_notice(
                        method,
                        "requested file changes and grant scope cannot be reviewed completely",
                    ),
                    event_sender,
                )?;
            }
            method if EXPLICITLY_DECLINED_APPROVAL_METHODS.contains(&method) => {
                self.send_rejected_server_request(
                    json!({
                    "id": request_id,
                    "result": { "decision": "denied" },
                    }),
                    approval_denied_notice(method, "legacy approval protocol"),
                    event_sender,
                )?;
            }
            "item/tool/requestUserInput" => {
                self.send_rejected_server_request(
                    json!({ "id": request_id, "result": { "answers": {} } }),
                    "app-server user-input request cancelled because Akra has no typed input form"
                        .to_string(),
                    event_sender,
                )?;
            }
            "mcpServer/elicitation/request" => {
                self.send_rejected_server_request(
                    json!({ "id": request_id, "result": { "action": "decline" } }),
                    "app-server MCP elicitation declined because Akra has no typed elicitation form"
                        .to_string(),
                    event_sender,
                )?;
            }
            "currentTime/read" => {
                let valid_thread_id = value
                    .get("params")
                    .and_then(Value::as_object)
                    .and_then(|params| params.get("threadId"))
                    .and_then(Value::as_str)
                    .is_some_and(|thread_id| {
                        !thread_id.is_empty() && thread_id.chars().count() <= 256
                    });
                if valid_thread_id {
                    let current_time_at = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |duration| {
                            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
                        });
                    self.send_json_line(json!({
                        "id": request_id,
                        "result": { "currentTimeAt": current_time_at },
                    }))?;
                } else {
                    self.send_rejected_server_request(
                        json!({
                            "id": request_id,
                            "error": {
                                "code": -32602,
                                "message": "currentTime/read requires a bounded threadId",
                            }
                        }),
                        "app-server current-time request rejected because its params were invalid"
                            .to_string(),
                        event_sender,
                    )?;
                }
            }
            "item/tool/call" => {
                self.send_method_specific_unsupported(
                    request_id,
                    method,
                    "dynamic tool calls are not implemented by this client",
                    event_sender,
                )?;
            }
            "account/chatgptAuthTokens/refresh" => {
                self.send_method_specific_unsupported(
                    request_id,
                    method,
                    "ChatGPT auth-token refresh is owned by the Codex runtime",
                    event_sender,
                )?;
            }
            "attestation/generate" => {
                self.send_method_specific_unsupported(
                    request_id,
                    method,
                    "client attestation generation is not configured",
                    event_sender,
                )?;
            }
            _ => {
                self.send_rejected_server_request(
                    json!({
                    "id": request_id,
                    "error": {
                        "code": -32601,
                        "message": "server request is not supported by the Akra app-server client",
                    },
                    }),
                    format!(
                        "app-server server request `{method}` rejected because Akra does not support it"
                    ),
                    event_sender,
                )?;
            }
        }
        Ok(true)
    }

    fn send_method_specific_unsupported(
        &mut self,
        request_id: &Value,
        method: &str,
        message: &str,
        event_sender: Option<&dyn AppServerEventSender>,
    ) -> Result<()> {
        self.send_rejected_server_request(
            json!({
                "id": request_id,
                "error": { "code": -32601, "message": message },
            }),
            format!("app-server server request `{method}` rejected: {message}"),
            event_sender,
        )
    }

    fn handle_interactive_approval_request(
        &mut self,
        request_id: &Value,
        method: &str,
        params: Option<&Value>,
        event_sender: Option<&dyn AppServerEventSender>,
        approval_context: Option<BoundApprovalContext<'_>>,
    ) -> Result<()> {
        // The operator's review budget starts at protocol receipt, not after a
        // potentially contended UI delivery.
        let deadline = Instant::now() + self.config.approval_timeout;
        let spec = match parse_interactive_approval(method, request_id, params) {
            Ok(spec) => spec,
            Err(error) => {
                let notice = approval_denied_notice(method, "invalid or unsupported request shape");
                self.diagnostics.record_warning(format!(
                    "{notice}; validation error chars={}",
                    error.to_string().chars().count()
                ));
                self.send_json_line(json!({
                    "id": request_id,
                    "result": declined_approval_result(method),
                }))?;
                if let Some(event_sender) = event_sender {
                    let _ = event_sender
                        .try_send(ConversationStreamEvent::StatusUpdated { text: notice });
                }
                return Ok(());
            }
        };

        if self.approval_mode != AppServerApprovalMode::Interactive {
            return self.send_unattended_approval_decline(request_id, spec);
        }
        let Some(approval_context) = approval_context else {
            return self.send_unbound_approval_decline(
                request_id,
                spec,
                "no active turn binding",
                event_sender,
            );
        };
        if spec.thread_id != approval_context.thread_id || spec.turn_id != approval_context.turn_id
        {
            return self.send_unbound_approval_decline(
                request_id,
                spec,
                "request did not match the active thread and turn",
                event_sender,
            );
        }
        let Some(event_sender) = event_sender else {
            return self.send_unbound_approval_decline(
                request_id,
                spec,
                "interactive UI was unavailable",
                None,
            );
        };
        let interrupt_context = approval_context.interrupt;
        if interrupt_context
            .signal
            .requested_after(interrupt_context.observed_generation)
        {
            return self.send_pre_ui_approval_decline(
                request_id,
                spec,
                "the active turn was already interrupted",
            );
        }
        if Instant::now() >= deadline {
            return self.send_pre_ui_approval_decline(
                request_id,
                spec,
                "the operator review deadline elapsed before UI delivery",
            );
        }

        let (approval_id, decision_receiver) = self.approval_broker.register()?;
        let request = ConversationApprovalRequest {
            approval_id: approval_id.clone(),
            server_request_id: spec.server_request_id.clone(),
            method: spec.method.clone(),
            kind: spec.kind,
            summary: spec.summary.clone(),
            details: spec.details.clone(),
        };
        let request_identity = request.identity();
        if interrupt_context
            .signal
            .requested_after(interrupt_context.observed_generation)
            || Instant::now() >= deadline
        {
            self.approval_broker.cancel(&approval_id);
            return self.send_pre_ui_approval_decline(
                request_id,
                spec,
                "the approval became stale before UI delivery",
            );
        }
        match event_sender.try_send(ConversationStreamEvent::ApprovalRequested { request }) {
            Ok(()) => {}
            Err(AppServerEventTrySendError::Full) => {
                self.approval_broker.cancel(&approval_id);
                return self.send_pre_ui_approval_decline(
                    request_id,
                    spec,
                    "the interactive UI event queue was full",
                );
            }
            Err(AppServerEventTrySendError::Disconnected) => {
                self.approval_broker.cancel(&approval_id);
                return self.send_pre_ui_approval_decline(
                    request_id,
                    spec,
                    "the interactive UI event queue was disconnected",
                );
            }
        }

        let (result, resolution) = loop {
            if let Err(error) = self.ensure_transport_healthy() {
                self.approval_broker.cancel(&approval_id);
                let _ = event_sender.try_send(ConversationStreamEvent::ApprovalResolved {
                    request_identity,
                    resolution: ConversationApprovalResolution::Disconnected,
                });
                return Err(error);
            }
            if interrupt_context
                .signal
                .requested_after(interrupt_context.observed_generation)
            {
                self.approval_broker.cancel(&approval_id);
                break (
                    spec.declined_result.clone(),
                    ConversationApprovalResolution::Interrupted,
                );
            }
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    self.approval_broker.cancel(&approval_id);
                    let _ = event_sender.try_send(ConversationStreamEvent::ApprovalResolved {
                        request_identity,
                        resolution: ConversationApprovalResolution::Disconnected,
                    });
                    return Ok(());
                }
                Ok(None) => {}
                Err(error) => {
                    self.approval_broker.cancel(&approval_id);
                    let _ = event_sender.try_send(ConversationStreamEvent::ApprovalResolved {
                        request_identity,
                        resolution: ConversationApprovalResolution::Disconnected,
                    });
                    return Err(error.into());
                }
            }
            let now = Instant::now();
            if now >= deadline {
                self.approval_broker.cancel(&approval_id);
                break (
                    spec.declined_result.clone(),
                    ConversationApprovalResolution::TimedOut,
                );
            }
            let wait = self
                .config
                .poll_interval
                .min(deadline.saturating_duration_since(now));
            match decision_receiver.recv_timeout(wait) {
                Ok(ConversationApprovalDecision::Accept) => {
                    run_after_approval_decision_received_hook();
                    if interrupt_context
                        .signal
                        .requested_after(interrupt_context.observed_generation)
                    {
                        self.approval_broker.cancel(&approval_id);
                        break (
                            spec.declined_result.clone(),
                            ConversationApprovalResolution::Interrupted,
                        );
                    }
                    if Instant::now() >= deadline {
                        self.approval_broker.cancel(&approval_id);
                        break (
                            spec.declined_result.clone(),
                            ConversationApprovalResolution::TimedOut,
                        );
                    }
                    break (
                        spec.accepted_result.clone(),
                        ConversationApprovalResolution::Accepted,
                    );
                }
                Ok(ConversationApprovalDecision::Decline) => {
                    break (
                        spec.declined_result.clone(),
                        ConversationApprovalResolution::Declined,
                    );
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.approval_broker.cancel(&approval_id);
                    break (
                        spec.declined_result.clone(),
                        ConversationApprovalResolution::Disconnected,
                    );
                }
            }
        };
        self.send_approval_response(
            request_id,
            &request_identity,
            result,
            resolution,
            event_sender,
        )
    }

    fn send_pre_ui_approval_decline(
        &mut self,
        request_id: &Value,
        spec: AppServerApprovalSpec,
        reason: &str,
    ) -> Result<()> {
        self.send_json_line(json!({
            "id": request_id,
            "result": spec.declined_result,
        }))?;
        let notice = approval_denied_notice(&spec.method, reason);
        self.diagnostics.record_warning(notice.clone());
        tracing::warn!(
            server_request_method = spec.method,
            approval_item_id_chars = spec.item_id.chars().count(),
            notice = %notice,
            "app-server approval request declined before UI delivery"
        );
        Ok(())
    }

    fn send_unattended_approval_decline(
        &mut self,
        request_id: &Value,
        spec: AppServerApprovalSpec,
    ) -> Result<()> {
        self.send_json_line(json!({
            "id": request_id,
            "result": spec.declined_result,
        }))?;
        let notice = approval_denied_notice(&spec.method, "unattended runtime");
        self.diagnostics.record_warning(notice.clone());
        tracing::warn!(
            server_request_method = spec.method,
            notice = %notice,
            "app-server approval request declined"
        );
        Ok(())
    }

    fn send_unbound_approval_decline(
        &mut self,
        request_id: &Value,
        spec: AppServerApprovalSpec,
        reason: &str,
        event_sender: Option<&dyn AppServerEventSender>,
    ) -> Result<()> {
        self.send_json_line(json!({
            "id": request_id,
            "result": spec.declined_result,
        }))?;
        let notice = approval_denied_notice(&spec.method, reason);
        self.diagnostics.record_warning(notice.clone());
        tracing::warn!(
            server_request_method = spec.method,
            approval_item_id_chars = spec.item_id.chars().count(),
            notice = %notice,
            "app-server approval request failed active-turn binding"
        );
        if let Some(event_sender) = event_sender {
            let _ = event_sender.try_send(ConversationStreamEvent::StatusUpdated { text: notice });
        }
        Ok(())
    }

    fn send_approval_response(
        &mut self,
        request_id: &Value,
        request_identity: &ConversationApprovalRequestIdentity,
        result: Value,
        resolution: ConversationApprovalResolution,
        event_sender: &dyn AppServerEventSender,
    ) -> Result<()> {
        self.send_json_line(json!({ "id": request_id, "result": result }))?;
        let _ = event_sender.try_send(ConversationStreamEvent::ApprovalResolved {
            request_identity: request_identity.clone(),
            resolution,
        });
        Ok(())
    }

    fn send_rejected_server_request(
        &mut self,
        response: Value,
        notice: String,
        event_sender: Option<&dyn AppServerEventSender>,
    ) -> Result<()> {
        self.send_json_line(response)?;
        self.diagnostics.record_warning(notice.clone());
        tracing::warn!(notice = %notice, "app-server server request rejected");
        if let Some(event_sender) = event_sender {
            let _ = event_sender.try_send(ConversationStreamEvent::StatusUpdated { text: notice });
        }
        Ok(())
    }

    fn drain_pending_turn_notifications(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        notification_state: &mut ActiveTurnNotificationState,
        event_sender: &dyn AppServerEventSender,
        non_retry_error_candidate: &mut Option<(ConversationTurnError, Instant)>,
    ) -> Result<Option<ConversationTurnTerminalReceipt>> {
        /*
         * Pending notifications were already received while turn/start was waiting for
         * its response. Reduce one bounded batch in FIFO order, then return control to
         * the loop for interrupt deadline checks. Grace and transport classification
         * remain deferred while entries are left, so a queued authoritative terminal
         * still wins in a later batch.
         */
        let drain_deadline = Instant::now() + MAX_PENDING_TURN_NOTIFICATION_DRAIN_TIME;
        for _ in 0..MAX_PENDING_TURN_NOTIFICATIONS_PER_POLL {
            let Some(notification) = self.pending_notifications.pop_front() else {
                return Ok(None);
            };
            let progress = self.handle_turn_stream_notification(
                notification,
                thread_id,
                turn_id,
                notification_state,
                event_sender,
            )?;
            if let Some(receipt) =
                self.apply_turn_notification_progress(progress, non_retry_error_candidate)
            {
                return Ok(Some(receipt));
            }
            if Instant::now() >= drain_deadline {
                break;
            }
        }

        Ok(None)
    }

    fn drain_transport_closing_turn_lines(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        notification_state: &mut ActiveTurnNotificationState,
        event_sender: &dyn AppServerEventSender,
        non_retry_error_candidate: &mut Option<(ConversationTurnError, Instant)>,
        interrupt_completion_deadline: Option<Instant>,
    ) -> Result<Option<ConversationTurnTerminalReceipt>> {
        let recovery_deadline = bounded_transport_closing_deadline(interrupt_completion_deadline);
        let mut accumulated_bytes = 0usize;
        let mut terminal_receipt = None;

        loop {
            let remaining = recovery_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(terminal_receipt);
            }
            let line = match self.rx.recv_timeout(remaining) {
                Ok(AppServerLine::ReaderFinished {
                    source: AppServerReaderSource::Stdout,
                    ..
                }) => return Ok(terminal_receipt),
                Ok(AppServerLine::ReaderFinished {
                    source: AppServerReaderSource::Stderr,
                    ..
                }) => continue,
                Ok(line) => line,
                Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                    return Ok(terminal_receipt);
                }
            };
            let encoded_bytes = line.encoded_size_bytes();
            if encoded_bytes
                > MAX_TRANSPORT_CLOSING_RECOVERY_BYTES.saturating_sub(accumulated_bytes)
            {
                self.diagnostics.record_warning(format!(
                    "app-server transport-closing recovery exceeded its {MAX_TRANSPORT_CLOSING_RECOVERY_BYTES}-byte bound"
                ));
                return Ok(terminal_receipt);
            }
            accumulated_bytes += encoded_bytes;
            if terminal_receipt.is_some() {
                continue;
            }

            let progress = self.process_recovery_turn_line(
                line,
                thread_id,
                turn_id,
                notification_state,
                event_sender,
                BufferedTurnRecoveryMode::TransportClosing,
            )?;
            if let Some(receipt) =
                self.apply_turn_notification_progress(progress, non_retry_error_candidate)
            {
                terminal_receipt = Some(receipt);
            }
        }
    }

    fn scan_grace_expired_turn_lines(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        notification_state: &mut ActiveTurnNotificationState,
        event_sender: &dyn AppServerEventSender,
        non_retry_error_candidate: &mut Option<(ConversationTurnError, Instant)>,
        interrupt_completion_deadline: Option<Instant>,
    ) -> Result<Option<ConversationTurnTerminalReceipt>> {
        let recovery_mode = BufferedTurnRecoveryMode::grace_expired(interrupt_completion_deadline);
        let recovery_deadline = recovery_mode
            .write_deadline()
            .expect("grace recovery always carries a write deadline");
        let mut drained_lines = 0;
        while drained_lines < MAX_GRACE_RECOVERY_LINES {
            let remaining = recovery_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let line = match self.rx.recv_timeout(remaining) {
                Ok(line) => line,
                Err(mpsc::RecvTimeoutError::Timeout) => return Ok(None),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(self.fail_transport(
                        "app-server line channel disconnected during grace-expired terminal recovery",
                    ));
                }
            };
            if let AppServerLine::ReaderFinished {
                source: AppServerReaderSource::Stdout,
                termination,
            } = &line
            {
                return Err(self.fail_transport(format!(
                    "app-server stdout reader {} during grace-expired terminal recovery",
                    termination.description()
                )));
            }
            if matches!(
                &line,
                AppServerLine::ReaderFinished {
                    source: AppServerReaderSource::Stderr,
                    ..
                }
            ) {
                continue;
            }
            drained_lines += 1;
            let progress = self.process_recovery_turn_line(
                line,
                thread_id,
                turn_id,
                notification_state,
                event_sender,
                recovery_mode,
            )?;
            if let Some(receipt) =
                self.apply_turn_notification_progress(progress, non_retry_error_candidate)
            {
                return Ok(Some(receipt));
            }
        }

        Ok(None)
    }

    fn process_recovery_turn_line(
        &mut self,
        line: AppServerLine,
        thread_id: &str,
        turn_id: &str,
        notification_state: &mut ActiveTurnNotificationState,
        event_sender: &dyn AppServerEventSender,
        recovery_mode: BufferedTurnRecoveryMode,
    ) -> Result<TurnStreamNotificationProgress> {
        let line = match line {
            AppServerLine::Stderr(line) => {
                self.diagnostics.record_stderr(line);
                return Ok(TurnStreamNotificationProgress::Continue);
            }
            AppServerLine::Stdout(line) => line,
            AppServerLine::ReaderFinished { .. } => {
                return Ok(TurnStreamNotificationProgress::Continue);
            }
        };
        let value = self.parse_json_line(&line)?;

        let is_server_request = value.get("id").is_some_and(|id| !id.is_null())
            && value.get("method").and_then(Value::as_str).is_some();
        if is_server_request && recovery_mode.is_transport_closing() {
            self.diagnostics.record_warning(
                "app-server server request skipped during terminal recovery after transport closure"
                    .to_string(),
            );
            return Ok(TurnStreamNotificationProgress::Continue);
        }
        if is_server_request {
            // A live grace-expiry scan uses the no-UI decline path, but every write
            // shares the scan's absolute deadline instead of the normal response
            // timeout. A failed write taints the connection without hiding a typed
            // terminal or grace-expired receipt already available to this stack.
            let previous_deadline = self.terminal_recovery_write_deadline.replace(
                recovery_mode
                    .write_deadline()
                    .expect("live grace recovery always carries a write deadline"),
            );
            let response = self.handle_server_request(&value, None, None);
            self.terminal_recovery_write_deadline = previous_deadline;
            if response.is_err() {
                self.diagnostics.record_warning(
                    "app-server server request response failed during terminal recovery; continuing the bounded terminal scan"
                        .to_string(),
                );
            }
            return Ok(TurnStreamNotificationProgress::Continue);
        }
        let Some(notification) = AppServerNotification::from_value(value) else {
            self.diagnostics.record_warning(
                "app-server sent a non-notification JSON message during bounded terminal recovery"
                    .to_string(),
            );
            return Ok(TurnStreamNotificationProgress::Continue);
        };

        self.handle_turn_stream_notification(
            notification,
            thread_id,
            turn_id,
            notification_state,
            event_sender,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn process_turn_stream_line(
        &mut self,
        line: AppServerLine,
        thread_id: &str,
        turn_id: &str,
        notification_state: &mut ActiveTurnNotificationState,
        event_sender: &dyn AppServerEventSender,
        interrupt_signal: &AppServerTurnInterruptSignal,
        observed_interrupt_generation: u64,
    ) -> Result<TurnStreamNotificationProgress> {
        let line = match line {
            AppServerLine::Stderr(line) => {
                self.diagnostics.record_stderr(line);
                return Ok(TurnStreamNotificationProgress::Continue);
            }
            AppServerLine::ReaderFinished {
                source: AppServerReaderSource::Stderr,
                termination,
            } => {
                self.diagnostics.record_warning(format!(
                    "app-server stderr reader {} while streaming the active turn",
                    termination.description()
                ));
                return Ok(TurnStreamNotificationProgress::Continue);
            }
            AppServerLine::ReaderFinished {
                source: AppServerReaderSource::Stdout,
                termination,
            } => {
                return Err(self.fail_transport(format!(
                    "app-server stdout reader {} before the active turn produced a terminal notification",
                    termination.description()
                )));
            }
            AppServerLine::Stdout(line) => line,
        };
        let value = self.parse_json_line(&line)?;

        if self.handle_server_request(
            &value,
            Some(event_sender),
            Some(BoundApprovalContext {
                thread_id,
                turn_id,
                interrupt: ApprovalInterruptContext {
                    signal: interrupt_signal,
                    observed_generation: observed_interrupt_generation,
                },
            }),
        )? {
            return Ok(TurnStreamNotificationProgress::Continue);
        }

        let Some(notification) = AppServerNotification::from_value(value) else {
            self.diagnostics.record_warning(
                "app-server sent a non-notification JSON message while streaming the active turn"
                    .to_string(),
            );
            return Ok(TurnStreamNotificationProgress::Continue);
        };

        self.handle_turn_stream_notification(
            notification,
            thread_id,
            turn_id,
            notification_state,
            event_sender,
        )
    }

    fn handle_turn_stream_notification(
        &mut self,
        notification: AppServerNotification,
        thread_id: &str,
        turn_id: &str,
        notification_state: &mut ActiveTurnNotificationState,
        event_sender: &dyn AppServerEventSender,
    ) -> Result<TurnStreamNotificationProgress> {
        /*
         * protocol::handle_turn_notification owns payload translation into domain
         * stream events. This connection layer keeps the transport responsibilities:
         * reject non-stream notifications, retain diagnostics, and decide when the
         * outer wait loop can stop.
         */
        self.try_flush_runtime_envelope_gap(thread_id, notification_state, event_sender);

        if !notification.should_defer_to_turn_stream() {
            self.diagnostics
                .record_warning(notification.warning_text("while streaming the active turn"));
            return Ok(TurnStreamNotificationProgress::Continue);
        }

        match handle_turn_notification(
            &notification,
            thread_id,
            turn_id,
            notification_state,
            event_sender,
        )? {
            TurnNotificationHandling::Consumed | TurnNotificationHandling::RetryObserved { .. } => {
                Ok(TurnStreamNotificationProgress::Continue)
            }
            TurnNotificationHandling::NonRetryErrorCandidate { error } => Ok(
                TurnStreamNotificationProgress::NonRetryErrorCandidate(error),
            ),
            TurnNotificationHandling::Terminal { receipt } => {
                Ok(TurnStreamNotificationProgress::Terminal(receipt))
            }
            TurnNotificationHandling::DuplicateTerminal { receipt } => {
                self.diagnostics.record_warning(
                    "app-server repeated a terminal notification; preserving the first terminal receipt"
                        .to_string(),
                );
                Ok(TurnStreamNotificationProgress::Terminal(receipt))
            }
            TurnNotificationHandling::Dropped(warning) => {
                self.diagnostics.record_warning(warning);
                Ok(TurnStreamNotificationProgress::Continue)
            }
        }
    }

    fn try_flush_runtime_envelope_gap(
        &mut self,
        thread_id: &str,
        notification_state: &mut ActiveTurnNotificationState,
        event_sender: &dyn AppServerEventSender,
    ) {
        let Some(observation) = notification_state.runtime_envelope_gap_observation(thread_id)
        else {
            return;
        };
        if event_sender
            .try_send(ConversationStreamEvent::RuntimeEnvelopeObserved {
                observation: Box::new(observation),
            })
            .is_ok()
        {
            notification_state.clear_runtime_envelope_gap();
        }
    }

    fn apply_turn_notification_progress(
        &self,
        progress: TurnStreamNotificationProgress,
        non_retry_error_candidate: &mut Option<(ConversationTurnError, Instant)>,
    ) -> Option<ConversationTurnTerminalReceipt> {
        match progress {
            TurnStreamNotificationProgress::Continue => None,
            TurnStreamNotificationProgress::NonRetryErrorCandidate(error) => {
                if non_retry_error_candidate.is_none() {
                    *non_retry_error_candidate =
                        Some((error, Instant::now() + self.config.terminal_grace_timeout));
                }
                None
            }
            TurnStreamNotificationProgress::Terminal(receipt) => Some(receipt),
        }
    }

    fn deliver_terminal_receipt(
        &mut self,
        receipt: ConversationTurnTerminalReceipt,
        thread_id: &str,
        notification_state: &mut ActiveTurnNotificationState,
        event_sender: &dyn AppServerEventSender,
    ) -> ConversationTurnTerminalReceipt {
        let receipt = bounded_terminal_receipt(receipt);
        let confirmed_receipt = receipt
            .clone()
            .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);
        let delivery_deadline = Instant::now() + self.config.terminal_delivery_timeout;

        loop {
            if let Some(observation) =
                notification_state.runtime_envelope_gap_observation(thread_id)
            {
                match event_sender.try_send_prebounded(
                    ConversationStreamEvent::RuntimeEnvelopeObserved {
                        observation: Box::new(observation),
                    },
                ) {
                    Ok(()) => {
                        notification_state.clear_runtime_envelope_gap();
                        continue;
                    }
                    Err(AppServerEventTrySendError::Disconnected) => {
                        self.diagnostics.record_warning(
                            "runtime-envelope gap marker application sink disconnected before terminal acknowledgement"
                                .to_string(),
                        );
                        return receipt.with_application_delivery(
                            ConversationTurnApplicationDelivery::Unconfirmed(
                                ConversationTurnApplicationDeliveryFailure::Disconnected,
                            ),
                        );
                    }
                    Err(AppServerEventTrySendError::Full)
                        if self.config.terminal_delivery_timeout.is_zero() =>
                    {
                        self.diagnostics.record_warning(
                            "runtime-envelope gap marker application sink was full before terminal acknowledgement"
                                .to_string(),
                        );
                        return receipt.with_application_delivery(
                            ConversationTurnApplicationDelivery::Unconfirmed(
                                ConversationTurnApplicationDeliveryFailure::Full,
                            ),
                        );
                    }
                    Err(AppServerEventTrySendError::Full) => {
                        let now = Instant::now();
                        if now >= delivery_deadline {
                            self.diagnostics.record_warning(
                                "runtime-envelope gap marker application sink remained full through the terminal delivery deadline"
                                    .to_string(),
                            );
                            return receipt.with_application_delivery(
                                ConversationTurnApplicationDelivery::Unconfirmed(
                                    ConversationTurnApplicationDeliveryFailure::DeadlineExceeded,
                                ),
                            );
                        }
                        thread::sleep(
                            self.config
                                .terminal_delivery_retry_interval
                                .min(delivery_deadline.saturating_duration_since(now)),
                        );
                        continue;
                    }
                }
            }
            match event_sender.try_send_prebounded(ConversationStreamEvent::TurnTerminal {
                receipt: confirmed_receipt.clone(),
            }) {
                Ok(()) => return confirmed_receipt,
                Err(AppServerEventTrySendError::Disconnected) => {
                    self.diagnostics.record_warning(
                        "terminal receipt application sink disconnected before acknowledgement"
                            .to_string(),
                    );
                    return receipt.with_application_delivery(
                        ConversationTurnApplicationDelivery::Unconfirmed(
                            ConversationTurnApplicationDeliveryFailure::Disconnected,
                        ),
                    );
                }
                Err(AppServerEventTrySendError::Full)
                    if self.config.terminal_delivery_timeout.is_zero() =>
                {
                    self.diagnostics
                        .record_warning("terminal receipt application sink was full".to_string());
                    return receipt.with_application_delivery(
                        ConversationTurnApplicationDelivery::Unconfirmed(
                            ConversationTurnApplicationDeliveryFailure::Full,
                        ),
                    );
                }
                Err(AppServerEventTrySendError::Full) => {
                    let now = Instant::now();
                    if now >= delivery_deadline {
                        self.diagnostics.record_warning(
                            "terminal receipt application sink remained full through its bounded delivery deadline"
                                .to_string(),
                        );
                        return receipt.with_application_delivery(
                            ConversationTurnApplicationDelivery::Unconfirmed(
                                ConversationTurnApplicationDeliveryFailure::DeadlineExceeded,
                            ),
                        );
                    }
                    thread::sleep(
                        self.config
                            .terminal_delivery_retry_interval
                            .min(delivery_deadline.saturating_duration_since(now)),
                    );
                }
            }
        }
    }

    fn parse_json_line(&self, line: &str) -> Result<Value> {
        serde_json::from_str(line).map_err(|error| {
            self.error_with_diagnostics(format!(
                "invalid JSON from app-server (bytes={}, chars={}, category={:?}, line={}, column={})",
                line.len(),
                line.chars().count(),
                error.classify(),
                error.line(),
                error.column(),
            ))
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
            if let Some(message) = self.transport_failure.current() {
                self.diagnostics.record_warning(format!(
                    "app-server connection terminated after bounded transport failure: {message}"
                ));
                self.terminate_child();
                break;
            }
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }

            let received = self.rx.recv_timeout(self.config.drain_poll_interval);
            if let Some(message) = self.transport_failure.current() {
                self.diagnostics.record_warning(format!(
                    "app-server connection terminated after bounded transport failure: {message}"
                ));
                self.terminate_child();
                break;
            }
            match received {
                Ok(AppServerLine::Stderr(line)) => self.diagnostics.record_stderr(line),
                Ok(AppServerLine::Stdout(line)) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&line) {
                        match self.handle_server_request(&value, None, None) {
                            Ok(true) => {}
                            Err(error) => self.diagnostics.record_warning(format!(
                                "failed to reject app-server server request while draining notices: {error}"
                            )),
                            Ok(false) => {
                                if let Some(notification) =
                                    AppServerNotification::from_value(value)
                                {
                                    if notification.should_defer_to_turn_stream() {
                                        if let Err(error) =
                                            self.defer_turn_notification(notification)
                                        {
                                            self.diagnostics.record_warning(error.to_string());
                                            break;
                                        }
                                    } else {
                                        self.diagnostics.record_warning(notification.warning_text(
                                            "while draining app-server notices",
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(AppServerLine::ReaderFinished {
                    source,
                    termination,
                }) => {
                    self.diagnostics.record_warning(format!(
                        "app-server {} reader {} while draining notices",
                        source.description(),
                        termination.description()
                    ));
                    if source == AppServerReaderSource::Stdout {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }

    fn error_with_diagnostics(&self, message: impl Into<String>) -> anyhow::Error {
        self.diagnostics.error(message)
    }

    fn ensure_transport_healthy(&mut self) -> Result<()> {
        let Some(message) = self.transport_failure.current() else {
            return Ok(());
        };
        self.terminate_child();
        Err(self.error_with_diagnostics(format!("app-server transport failed: {message}")))
    }

    fn fail_transport(&mut self, message: impl Into<String>) -> anyhow::Error {
        self.transport_failure.record(message);
        let message = self
            .transport_failure
            .current()
            .unwrap_or_else(|| "unknown bounded transport failure".to_string());
        self.terminate_child();
        self.error_with_diagnostics(format!("app-server transport failed: {message}"))
    }

    fn taint_terminal_recovery_transport(&self, message: impl Into<String>) -> anyhow::Error {
        self.transport_failure.record(message);
        let message = self
            .transport_failure
            .current()
            .unwrap_or_else(|| "unknown terminal-recovery transport failure".to_string());
        self.error_with_diagnostics(format!(
            "app-server transport tainted during terminal recovery: {message}"
        ))
    }

    fn terminate_child(&mut self) {
        if let Err(error) = self.child.terminate_and_wait() {
            tracing::warn!(error = %error, "app-server process containment cleanup failed");
        }
        // Killing the child closes the stdin read end, which releases a writer blocked
        // in WriteFile/write before we join it and discard the queue sender.
        self.stdin_writer.shutdown();
    }
}

impl Drop for AppServerConnection {
    fn drop(&mut self) {
        // Shared runtimes may hold app-server children for a long time; dropping the connection must not leave a child behind.
        self.terminate_child();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppServerReaderSource {
    Stdout,
    Stderr,
}

impl AppServerReaderSource {
    fn description(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppServerReaderTermination {
    EndOfFile,
    Failed,
}

impl AppServerReaderTermination {
    fn description(self) -> &'static str {
        match self {
            Self::EndOfFile => "reached EOF",
            Self::Failed => "failed",
        }
    }
}

enum AppServerLine {
    // Stdout carries JSON-RPC responses and notifications.
    Stdout(String),
    // Stderr is diagnostics-only and never parsed as protocol.
    Stderr(String),
    // ReaderFinished is ordered after every line produced by that pipe reader.
    ReaderFinished {
        source: AppServerReaderSource,
        termination: AppServerReaderTermination,
    },
}

impl AppServerLine {
    fn encoded_size_bytes(&self) -> usize {
        match self {
            Self::Stdout(line) | Self::Stderr(line) => line.len(),
            Self::ReaderFinished { .. } => 0,
        }
    }
}

enum BoundedLineRead {
    Line(String),
    EndOfFile,
    LimitExceeded,
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    maximum_bytes: usize,
) -> io::Result<BoundedLineRead> {
    let mut line = Vec::with_capacity(8 * 1024);
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(BoundedLineRead::EndOfFile);
            }
            return String::from_utf8(line)
                .map(BoundedLineRead::Line)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let payload_bytes = newline.unwrap_or(available.len());
        if line.len().saturating_add(payload_bytes) > maximum_bytes {
            return Ok(BoundedLineRead::LimitExceeded);
        }
        line.extend_from_slice(&available[..payload_bytes]);
        let consumed = newline.map_or(payload_bytes, |index| index + 1);
        reader.consume(consumed);

        if newline.is_some() {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return String::from_utf8(line)
                .map(BoundedLineRead::Line)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
        }
    }
}

fn spawn_pipe_reader<T: std::io::Read + Send + 'static>(
    pipe: T,
    tx: mpsc::SyncSender<AppServerLine>,
    is_stderr: bool,
    maximum_bytes: usize,
    transport_failure: Arc<TransportFailure>,
) {
    // One reader thread per pipe converts bounded blocking reads into a bounded connection backlog.
    // A blocking send is intentional backpressure: a short, valid notification burst must not be
    // reclassified as a transport failure merely because the consumer lost one scheduler timeslice.
    thread::spawn(move || {
        let source = if is_stderr {
            AppServerReaderSource::Stderr
        } else {
            AppServerReaderSource::Stdout
        };
        let mut reader = BufReader::new(pipe);
        loop {
            let line = match read_bounded_line(&mut reader, maximum_bytes) {
                Ok(BoundedLineRead::Line(line)) => line,
                Ok(BoundedLineRead::EndOfFile) => {
                    let _ = tx.send(AppServerLine::ReaderFinished {
                        source,
                        termination: AppServerReaderTermination::EndOfFile,
                    });
                    return;
                }
                Ok(BoundedLineRead::LimitExceeded) => {
                    transport_failure.record(format!(
                        "app-server {} line exceeded the {maximum_bytes}-byte limit",
                        source.description()
                    ));
                    let _ = tx.send(AppServerLine::ReaderFinished {
                        source,
                        termination: AppServerReaderTermination::Failed,
                    });
                    return;
                }
                Err(error) => {
                    transport_failure.record(format!(
                        "failed to read app-server {} safely: {error}",
                        source.description()
                    ));
                    let _ = tx.send(AppServerLine::ReaderFinished {
                        source,
                        termination: AppServerReaderTermination::Failed,
                    });
                    return;
                }
            };
            let payload = if is_stderr {
                AppServerLine::Stderr(line)
            } else {
                AppServerLine::Stdout(line)
            };
            if tx.send(payload).is_err() {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fmt::Debug;
    use std::fs;
    use std::io::{BufReader, Cursor};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::mpsc::{self, SyncSender};
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use anyhow::Result;
    use serde_json::{Value, json};

    #[cfg(unix)]
    use super::AppServerWriteRequest;
    #[cfg(windows)]
    use super::app_server_process_environment_variable_allowed;
    use super::diagnostics::{ConnectionDiagnostics, PendingNotifications};
    use super::{
        API_KEY_AUTH_ENV_VAR, APP_SERVER_LINE_CHANNEL_CAPACITY, APP_SERVER_WRITE_CHANNEL_CAPACITY,
        AppServerApprovalMode, AppServerConnection, AppServerConnectionConfig,
        AppServerEventSender, AppServerEventTrySendError, AppServerLine, AppServerReaderSource,
        AppServerReaderTermination, AppServerStdinWriter, AppServerTurnInterruptSignal,
        ApprovalInterruptContext, BoundApprovalContext, BoundedLineRead,
        CANCELLED_SERVER_REQUEST_METHODS, DISABLE_LOGIN_SHELL_OVERRIDE,
        LOCALLY_SUPPORTED_SERVER_REQUEST_METHODS, MAX_PENDING_NOTIFICATIONS,
        MAX_RESPONSE_TIMEOUT_SECS, MAX_STDERR_LINE_BYTES, MAX_STDOUT_LINE_BYTES,
        METHOD_SPECIFIC_UNSUPPORTED_SERVER_REQUEST_METHODS, PROCESS_ENVIRONMENT_ENV_VAR,
        ProcessEnvironmentPolicy, RESPONSE_TIMEOUT_ENV_VAR, SHELL_ENVIRONMENT_INHERIT_ENV_VAR,
        SHELL_ENVIRONMENT_SECRET_EXCLUDES_OVERRIDE, ShellEnvironmentInherit, TransportFailure,
        UNINSPECTABLE_SERVER_REQUEST_METHODS, app_server_api_key_environment_variable_allowed,
        app_server_command_with_environment, app_server_process_environment_key_allowed,
        canonical_proxy_environment_key, filtered_app_server_process_environment,
        install_after_approval_decision_received_hook, openai_base_url_is_unsafe,
        proxy_url_has_userinfo, read_bounded_line, resolve_api_key_auth,
        resolve_process_environment, resolve_shell_environment_inherit, spawn_pipe_reader,
    };
    use crate::adapter::outbound::app_server::approval::{
        AppServerApprovalBroker, EXPLICITLY_DECLINED_APPROVAL_METHODS, INTERACTIVE_APPROVAL_METHODS,
    };
    use crate::adapter::outbound::app_server::protocol::{
        AppServerNotification, ReasoningEffortValue, ThreadListParams, ThreadResumeParams,
        ThreadSetNameParams, ThreadStartParams, TurnInputItem, TurnInterruptParams,
        TurnStartParams,
    };
    use crate::adapter::outbound::app_server::steering::AppServerTurnSteerBroker;
    use crate::application::port::conversation_stream::{
        CONVERSATION_STREAM_CHANNEL_CAPACITY, ConversationStreamEvent, conversation_stream_channel,
    };
    use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
    use crate::domain::conversation::{
        ConversationApprovalDecision, ConversationApprovalResolution, ConversationTurnSteerReceipt,
        ConversationTurnSteerRequest,
    };
    use crate::domain::conversation_runtime_envelope::{
        ConversationRuntimeEnvelopeObservation, ConversationRuntimeObservationGap,
        ConversationRuntimeObservedValue, ConversationRuntimeThreadStatus,
    };
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDelivery, ConversationTurnApplicationDeliveryFailure,
        ConversationTurnItemsView, ConversationTurnTerminalOutcome,
        ConversationTurnTerminalReceipt, ConversationTurnTerminalUncertainty,
    };
    use crate::subprocess;

    fn completed_turn_notification(thread_id: &str, turn_id: &str) -> Value {
        json!({
            "method": "turn/completed",
            "params": {
                "threadId": thread_id,
                "turn": {
                    "id": turn_id,
                    "items": [],
                    "status": "completed"
                }
            }
        })
    }

    struct RecoveryHandoffEventSender {
        release_terminal: Mutex<Option<mpsc::Sender<()>>>,
        statuses_before_release: Mutex<usize>,
        events: Mutex<Vec<ConversationStreamEvent>>,
    }

    impl RecoveryHandoffEventSender {
        fn new(release_terminal: mpsc::Sender<()>) -> Self {
            Self::after_statuses(1, release_terminal)
        }

        fn after_statuses(
            statuses_before_release: usize,
            release_terminal: mpsc::Sender<()>,
        ) -> Self {
            Self {
                release_terminal: Mutex::new(Some(release_terminal)),
                statuses_before_release: Mutex::new(statuses_before_release),
                events: Mutex::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<ConversationStreamEvent> {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    impl AppServerEventSender for RecoveryHandoffEventSender {
        fn try_send_prebounded(
            &self,
            event: ConversationStreamEvent,
        ) -> std::result::Result<(), AppServerEventTrySendError> {
            if matches!(
                event,
                ConversationStreamEvent::StatusUpdated { .. }
                    | ConversationStreamEvent::RuntimeEnvelopeObserved { .. }
            ) {
                let should_release = {
                    let mut remaining = self
                        .statuses_before_release
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    *remaining = remaining.saturating_sub(1);
                    *remaining == 0
                };
                if should_release
                    && let Some(release_terminal) = self
                        .release_terminal
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                {
                    let _ = release_terminal.send(());
                }
            }
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event);
            Ok(())
        }
    }

    struct RuntimeGapPressureEventSender {
        rejected_initial_observation: Mutex<bool>,
        events: Mutex<Vec<ConversationStreamEvent>>,
    }

    impl RuntimeGapPressureEventSender {
        fn new() -> Self {
            Self {
                rejected_initial_observation: Mutex::new(false),
                events: Mutex::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<ConversationStreamEvent> {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    impl AppServerEventSender for RuntimeGapPressureEventSender {
        fn try_send_prebounded(
            &self,
            event: ConversationStreamEvent,
        ) -> std::result::Result<(), AppServerEventTrySendError> {
            let reject_initial = matches!(
                &event,
                ConversationStreamEvent::RuntimeEnvelopeObserved { observation }
                    if !matches!(
                        observation.as_ref(),
                        ConversationRuntimeEnvelopeObservation::ProjectionGap { .. }
                    )
            );
            if reject_initial {
                let mut rejected = self
                    .rejected_initial_observation
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !*rejected {
                    *rejected = true;
                    return Err(AppServerEventTrySendError::Full);
                }
            }
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event);
            Ok(())
        }
    }

    fn confirmed_completed_receipt(
        thread_id: &str,
        turn_id: &str,
        changed_planning_file_paths: Vec<String>,
    ) -> ConversationTurnTerminalReceipt {
        ConversationTurnTerminalReceipt::completed(thread_id, turn_id, changed_planning_file_paths)
            .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed)
    }

    fn bound_approval_context<'a>(
        signal: &'a AppServerTurnInterruptSignal,
        thread_id: &'a str,
        turn_id: &'a str,
    ) -> BoundApprovalContext<'a> {
        BoundApprovalContext {
            thread_id,
            turn_id,
            interrupt: ApprovalInterruptContext {
                signal,
                observed_generation: signal.current_generation(),
            },
        }
    }

    #[test]
    fn server_request_method_classification_is_complete_and_disjoint() {
        let groups = [
            INTERACTIVE_APPROVAL_METHODS,
            UNINSPECTABLE_SERVER_REQUEST_METHODS,
            EXPLICITLY_DECLINED_APPROVAL_METHODS,
            CANCELLED_SERVER_REQUEST_METHODS,
            METHOD_SPECIFIC_UNSUPPORTED_SERVER_REQUEST_METHODS,
            LOCALLY_SUPPORTED_SERVER_REQUEST_METHODS,
        ];
        let classified = groups
            .into_iter()
            .flatten()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let schema: Value = serde_json::from_str(include_str!(
            "../../../../schema/codex_app_server_protocol.server_request.schema.json"
        ))
        .expect("pinned ServerRequest schema should parse");
        let schema_methods = schema
            .get("oneOf")
            .and_then(Value::as_array)
            .expect("ServerRequest schema should expose oneOf")
            .iter()
            .map(|request| {
                request
                    .pointer("/properties/method/enum/0")
                    .and_then(Value::as_str)
                    .expect("ServerRequest variant should expose one method")
            })
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(
            classified.len(),
            groups.iter().map(|group| group.len()).sum::<usize>()
        );
        assert_eq!(
            classified, schema_methods,
            "every pinned ServerRequest method must have one explicit client behavior"
        );
    }

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
        assert_eq!(
            AppServerConnectionConfig::from_response_timeout_secs_value(Some(
                "18446744073709551615"
            ))
            .response_timeout,
            Duration::from_secs(MAX_RESPONSE_TIMEOUT_SECS)
        );
    }

    #[cfg(unix)]
    #[test]
    fn app_server_command_keeps_the_canonical_codex_pin_after_path_changes() {
        use std::os::unix::fs::PermissionsExt;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let install = PathBuf::from(std::env::var_os("HOME").expect("HOME should exist"))
            .join(".cache")
            .join(format!("akra-codex-pin-{}-{now}", std::process::id()));
        fs::create_dir_all(&install).expect("safe install directory should be created");
        let mut directory_permissions = fs::metadata(&install)
            .expect("install metadata should exist")
            .permissions();
        directory_permissions.set_mode(0o755);
        fs::set_permissions(&install, directory_permissions)
            .expect("install directory should be private from other writers");
        let codex = install.join("codex");
        fs::write(&codex, "#!/bin/sh\nexit 0\n").expect("codex fixture should write");
        let mut permissions = fs::metadata(&codex)
            .expect("codex fixture metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&codex, permissions).expect("codex fixture should be executable");
        let cwd = std::env::current_dir().expect("test cwd should exist");
        let pinned =
            crate::trusted_executable::resolve_from_path("codex", install.as_os_str(), &cwd)
                .expect("safe user-local Codex should resolve");
        let config = AppServerConnectionConfig::default()
            .with_test_process(pinned.clone(), std::iter::empty::<(OsString, OsString)>());

        let command = super::app_server_command(&config);
        assert_eq!(command.get_program(), pinned.as_os_str());
        assert!(Path::new(command.get_program()).is_absolute());
        let _ = fs::remove_dir_all(install);
    }

    #[test]
    fn unsafe_codex_resolution_is_reported_before_spawn() {
        let config = AppServerConnectionConfig {
            executable: super::unresolved_codex_executable_path(),
            executable_resolution_error: Some(
                "PATH selected a repository-controlled codex".to_string(),
            ),
            ..AppServerConnectionConfig::default()
        };
        let error = config
            .ensure_executable_is_pinned()
            .expect_err("unsafe Codex resolution must fail before process creation");
        assert!(error.to_string().contains("could not be pinned safely"));
        assert!(error.to_string().contains("repository-controlled"));
    }

    #[test]
    fn app_server_command_restricts_model_shell_environment_by_default() {
        let command = app_server_command_with_environment(
            &AppServerConnectionConfig::default(),
            ShellEnvironmentInherit::Core,
            ProcessEnvironmentPolicy::Scrubbed,
            [],
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(command.get_program(), "codex");
        assert_eq!(
            args,
            [
                "app-server",
                "-c",
                "shell_environment_policy.inherit=\"core\"",
                "-c",
                SHELL_ENVIRONMENT_SECRET_EXCLUDES_OVERRIDE,
                "-c",
                DISABLE_LOGIN_SHELL_OVERRIDE,
            ]
        );
    }

    #[test]
    fn shell_environment_inherit_accepts_only_supported_values() {
        for (value, expected) in [
            ("none", ShellEnvironmentInherit::None),
            (" CORE ", ShellEnvironmentInherit::Core),
            ("ALL", ShellEnvironmentInherit::All),
        ] {
            assert_eq!(
                resolve_shell_environment_inherit(Some(value)),
                super::ShellEnvironmentInheritResolution {
                    inherit: expected,
                    warning: None,
                }
            );
        }
    }

    #[test]
    fn shell_environment_inherit_defaults_and_invalid_values_fail_closed_to_core() {
        assert_eq!(
            resolve_shell_environment_inherit(None),
            super::ShellEnvironmentInheritResolution {
                inherit: ShellEnvironmentInherit::Core,
                warning: None,
            }
        );

        for value in ["", "everything", "inherit-all"] {
            let resolution = resolve_shell_environment_inherit(Some(value));
            assert_eq!(resolution.inherit, ShellEnvironmentInherit::Core);
            assert!(
                resolution
                    .warning
                    .as_deref()
                    .is_some_and(|warning| warning.contains("expected none, core, or all"))
            );
        }

        let raw_value = "private-shell-policy-value";
        let warning = resolve_shell_environment_inherit(Some(raw_value))
            .warning
            .expect("invalid shell policy should warn");
        assert!(!warning.contains(raw_value));
    }

    #[test]
    fn shell_environment_inherit_override_preserves_default_secret_exclusions() {
        for inherit in [
            ShellEnvironmentInherit::None,
            ShellEnvironmentInherit::Core,
            ShellEnvironmentInherit::All,
        ] {
            let command = app_server_command_with_environment(
                &AppServerConnectionConfig::default(),
                inherit,
                ProcessEnvironmentPolicy::Scrubbed,
                [],
            );
            let args = command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>();

            assert!(args.contains(&inherit.codex_override()));
            assert!(args.contains(&SHELL_ENVIRONMENT_SECRET_EXCLUDES_OVERRIDE.to_string()));
        }
    }

    #[test]
    fn app_server_process_environment_is_scrubbed_unless_all_is_explicit() {
        assert_eq!(
            resolve_process_environment(None).policy,
            ProcessEnvironmentPolicy::Scrubbed
        );
        assert_eq!(
            resolve_process_environment(Some("scrubbed")).policy,
            ProcessEnvironmentPolicy::Scrubbed
        );
        let all = resolve_process_environment(Some(" ALL "));
        assert_eq!(all.policy, ProcessEnvironmentPolicy::All);
        assert!(
            all.warning
                .as_deref()
                .is_some_and(|warning| warning.contains("complete parent environment"))
        );

        for value in ["", "inherit", "true"] {
            let resolution = resolve_process_environment(Some(value));
            assert_eq!(resolution.policy, ProcessEnvironmentPolicy::Scrubbed);
            assert!(
                resolution
                    .warning
                    .as_deref()
                    .is_some_and(|warning| warning.contains("expected scrubbed or all"))
            );
        }

        let raw_value = "private-process-policy-value";
        let warning = resolve_process_environment(Some(raw_value))
            .warning
            .expect("invalid process policy should warn");
        assert!(!warning.contains(raw_value));
    }

    #[test]
    fn app_server_api_key_auth_requires_exact_explicit_opt_in() {
        assert_eq!(
            resolve_api_key_auth(None),
            super::ApiKeyAuthResolution {
                enabled: false,
                warning: None,
            }
        );
        assert_eq!(
            resolve_api_key_auth(Some(std::ffi::OsStr::new("1"))),
            super::ApiKeyAuthResolution {
                enabled: true,
                warning: None,
            }
        );

        for value in ["", " 1", "1 ", "true", "01"] {
            let resolution = resolve_api_key_auth(Some(std::ffi::OsStr::new(value)));
            assert!(!resolution.enabled);
            let warning = resolution
                .warning
                .expect("invalid API-key auth value should warn");
            assert!(warning.contains("expected exact value 1"));
        }
        let private_value = "private-api-key-auth-value";
        let warning = resolve_api_key_auth(Some(std::ffi::OsStr::new(private_value)))
            .warning
            .expect("invalid API-key auth value should warn");
        assert!(!warning.contains(private_value));
    }

    #[test]
    fn scrubbed_app_server_process_environment_excludes_unlisted_secrets() {
        let config = AppServerConnectionConfig::default().with_test_process(
            "codex-fixture",
            [("AKRA_FAKE_CHILD_ONLY".into(), "fixture".into())],
        );
        let command = app_server_command_with_environment(
            &config,
            ShellEnvironmentInherit::Core,
            ProcessEnvironmentPolicy::Scrubbed,
            [
                ("PATH".into(), "/usr/bin".into()),
                ("HOME".into(), "/home/operator".into()),
                ("LC_ALL".into(), "C.UTF-8".into()),
                ("OPENAI_API_KEY".into(), "required-auth".into()),
                ("CODEX_API_KEY".into(), "alternate-auth".into()),
                ("HTTPS_PROXY".into(), "http://corporate-proxy".into()),
                ("SSH_AUTH_SOCK".into(), "/tmp/private-agent.sock".into()),
                ("AWS_SECRET_ACCESS_KEY".into(), "aws-secret".into()),
                ("DATABASE_URL".into(), "postgres://secret".into()),
                ("GH_TOKEN".into(), "github-secret".into()),
                ("ARBITRARY_TOKEN".into(), "other-secret".into()),
            ],
        );
        let environment = command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|value| {
                    (
                        key.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(command.get_program(), "codex-fixture");
        assert_eq!(
            environment.get("PATH").map(String::as_str),
            Some("/usr/bin")
        );
        assert_eq!(
            environment.get("HTTPS_PROXY").map(String::as_str),
            Some("http://corporate-proxy")
        );
        assert_eq!(
            environment.get("AKRA_FAKE_CHILD_ONLY").map(String::as_str),
            Some("fixture")
        );
        for secret in [
            "SSH_AUTH_SOCK",
            "AWS_SECRET_ACCESS_KEY",
            "DATABASE_URL",
            "GH_TOKEN",
            "ARBITRARY_TOKEN",
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
        ] {
            assert!(!environment.contains_key(secret));
        }
    }

    #[test]
    fn explicit_api_key_auth_forwards_only_supported_keys_to_app_server() {
        let config = AppServerConnectionConfig::default().with_test_api_key_auth();
        let command = app_server_command_with_environment(
            &config,
            ShellEnvironmentInherit::Core,
            ProcessEnvironmentPolicy::Scrubbed,
            [
                ("PATH".into(), "/usr/bin".into()),
                ("OPENAI_API_KEY".into(), "openai-auth-value".into()),
                ("CODEX_API_KEY".into(), "codex-auth-value".into()),
                (API_KEY_AUTH_ENV_VAR.into(), "1".into()),
                ("GH_TOKEN".into(), "github-secret".into()),
                ("ARBITRARY_KEY".into(), "other-secret".into()),
            ],
        );
        let environment = command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|value| {
                    (
                        key.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            environment.get("OPENAI_API_KEY").map(String::as_str),
            Some("openai-auth-value")
        );
        assert_eq!(
            environment.get("CODEX_API_KEY").map(String::as_str),
            Some("codex-auth-value")
        );
        assert!(!environment.contains_key("GH_TOKEN"));
        assert!(!environment.contains_key("ARBITRARY_KEY"));
        assert!(!environment.contains_key(API_KEY_AUTH_ENV_VAR));
        assert!(args.contains(&SHELL_ENVIRONMENT_SECRET_EXCLUDES_OVERRIDE.to_string()));
        assert!(args.contains(&DISABLE_LOGIN_SHELL_OVERRIDE.to_string()));
    }

    #[test]
    fn scrubbed_process_environment_drops_credentialed_and_ambiguous_proxy_urls() {
        let secret = "proxy-password-must-not-appear";
        let filtered = filtered_app_server_process_environment(
            [
                (
                    "HTTPS_PROXY".into(),
                    format!("http://operator:{secret}@proxy.example:8443").into(),
                ),
                (
                    "http_proxy".into(),
                    "http://user:p%40ss@[2001:db8::1]:8080".into(),
                ),
                (
                    "ALL_PROXY".into(),
                    "socks5://user%40name@proxy.example:1080".into(),
                ),
                ("all_proxy".into(), "http://[2001:db8::1".into()),
                (
                    "HTTP_PROXY".into(),
                    "http://user%40proxy.example:8080".into(),
                ),
                ("NO_PROXY".into(), "localhost,user@example.test".into()),
                ("PATH".into(), "/usr/bin".into()),
            ],
            false,
        );

        assert_eq!(
            filtered.dropped_credential_variables,
            ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY"]
        );
        let environment = filtered
            .variables
            .into_iter()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            environment.get("PATH").map(String::as_str),
            Some("/usr/bin")
        );
        assert_eq!(
            environment.get("NO_PROXY").map(String::as_str),
            Some("localhost,user@example.test")
        );
        assert!(!format!("{environment:?}").contains(secret));
    }

    #[test]
    fn proxy_userinfo_parser_handles_ipv6_percent_encoding_and_scheme_authority() {
        for safe in [
            "http://proxy.example:8080",
            "https://[2001:db8::1]:8443",
            "socks5h://[fe80::1%25eth0]:1080",
            "proxy.example:3128",
        ] {
            assert_eq!(proxy_url_has_userinfo(safe), Ok(false), "safe: {safe}");
        }
        for credentialed in [
            "http://user:password@proxy.example:8080",
            "http://user:p%40ss@[2001:db8::1]:8080",
            "socks5://user%40name@proxy.example:1080",
            "http://user%40proxy.example:8080",
            "user:password@proxy.example:8080",
        ] {
            assert_eq!(
                proxy_url_has_userinfo(credentialed),
                Ok(true),
                "credentialed: {credentialed}"
            );
        }
        for ambiguous in [
            " http://proxy.example:8080",
            "http://[2001:db8::1",
            "http://2001:db8::1",
            "http://proxy.example:not-a-port",
            "1http://proxy.example",
        ] {
            assert!(
                proxy_url_has_userinfo(ambiguous).is_err(),
                "ambiguous: {ambiguous}"
            );
        }
    }

    #[test]
    fn scrubbed_process_environment_rejects_credentialed_or_ambiguous_openai_base_urls() {
        for unsafe_url in [
            "https://user:secret@api.example.test/v1",
            "https://api.example.test/v1?token=secret",
            "https://api.example.test/v1#secret",
            "https://@api.example.test/v1",
            "https:\\api.example.test\\v1",
            "http://api.example.test/v1",
            "http://10.0.0.1/v1",
            "http://localhost.example.test/v1",
            "http://2130706433/v1",
            "http://[::ffff:127.0.0.1]/v1",
            "api.example.test/v1",
        ] {
            assert!(openai_base_url_is_unsafe(&OsString::from(unsafe_url)));
            let filtered = filtered_app_server_process_environment(
                [
                    ("OPENAI_BASE_URL".into(), unsafe_url.into()),
                    ("PATH".into(), "/usr/bin".into()),
                ],
                false,
            );
            assert_eq!(filtered.dropped_credential_variables, ["OPENAI_BASE_URL"]);
            assert!(
                filtered
                    .variables
                    .iter()
                    .all(|(key, _)| key != "OPENAI_BASE_URL")
            );
        }

        for safe_url in [
            "https://api.openai.com/v1",
            "http://127.0.0.1:11434/v1",
            "http://127.9.8.7/v1",
            "http://[::1]:11434/v1",
            "http://LOCALHOST:11434/v1",
        ] {
            assert!(!openai_base_url_is_unsafe(&OsString::from(safe_url)));
        }

        let filtered = filtered_app_server_process_environment(
            [
                (
                    "OPENAI_BASE_URL".into(),
                    "http://api.example.test/v1".into(),
                ),
                ("OPENAI_API_KEY".into(), "explicit-api-key".into()),
            ],
            true,
        );
        assert_eq!(filtered.dropped_credential_variables, ["OPENAI_BASE_URL"]);
        assert!(
            filtered
                .variables
                .iter()
                .any(|(key, value)| key == "OPENAI_API_KEY" && value == "explicit-api-key")
        );
        assert!(
            filtered
                .variables
                .iter()
                .all(|(key, _)| key != "OPENAI_BASE_URL")
        );
    }

    #[test]
    fn proxy_variable_names_are_classified_without_case_leaks() {
        for (raw, expected) in [
            ("http_proxy", "HTTP_PROXY"),
            ("HtTpS_pRoXy", "HTTPS_PROXY"),
            ("all_proxy", "ALL_PROXY"),
        ] {
            assert_eq!(
                canonical_proxy_environment_key(&std::ffi::OsString::from(raw)),
                Some(expected)
            );
        }
        assert_eq!(
            canonical_proxy_environment_key(&std::ffi::OsString::from("NO_PROXY")),
            None
        );
    }

    #[test]
    fn windows_process_environment_allowlist_is_ascii_case_insensitive() {
        for key in ["Path", "uSeRpRoFiLe", "lC_aLl", "https_PrOxY"] {
            assert!(app_server_process_environment_key_allowed(key, true));
        }
        assert!(!app_server_process_environment_key_allowed(
            "OpenAI_Api_Key",
            true
        ));
        assert!(!app_server_process_environment_key_allowed("Path", false));
        assert_eq!(
            app_server_api_key_environment_variable_allowed(&OsString::from("OpenAI_Api_Key")),
            cfg!(windows)
        );
    }

    #[cfg(windows)]
    #[test]
    fn actual_windows_environment_filter_accepts_mixed_case_allowlisted_keys() {
        assert!(app_server_process_environment_variable_allowed(
            &OsString::from("Path")
        ));
        assert!(app_server_process_environment_variable_allowed(
            &OsString::from("userProfile")
        ));
    }

    #[test]
    fn timeout_env_var_name_is_stable() {
        assert_eq!(
            RESPONSE_TIMEOUT_ENV_VAR,
            "CODEX_EXEC_LOOP_APP_SERVER_RESPONSE_TIMEOUT_SECS"
        );
        assert_eq!(
            SHELL_ENVIRONMENT_INHERIT_ENV_VAR,
            "AKRA_APP_SERVER_SHELL_ENVIRONMENT_INHERIT"
        );
        assert_eq!(
            PROCESS_ENVIRONMENT_ENV_VAR,
            "AKRA_APP_SERVER_PROCESS_ENVIRONMENT"
        );
        assert_eq!(API_KEY_AUTH_ENV_VAR, "AKRA_APP_SERVER_API_KEY_AUTH");
    }

    #[test]
    fn interrupt_signal_tracks_newer_stop_generations() {
        /*
         * The stream loop compares generation snapshots, so an old stop request must
         * not cancel a future turn while a newer request must be observable without a lock.
         */
        let signal = AppServerTurnInterruptSignal::default();

        assert_eq!(signal.current_generation(), 0);
        assert!(!signal.requested_after(0));

        signal.request_stop_all_sessions();

        assert_eq!(signal.current_generation(), 1);
        assert!(signal.requested_after(0));
        assert!(!signal.requested_after(1));
    }

    #[test]
    fn initialize_sends_handshake_notification_and_rejects_second_call() {
        let mut harness = TestConnection::new(false);
        harness.send_stdout(json!({
            "id": 1,
            "result": {
                "userAgent": "codex-app-server/test",
                "platformFamily": "unix",
                "platformOs": "linux-x64"
            }
        }));

        let response = harness
            .connection
            .initialize()
            .expect("initialize response should deserialize");

        assert_eq!(response.user_agent, "codex-app-server/test");
        assert_eq!(response.platform_family, "unix");
        assert_eq!(response.platform_os, "linux-x64");
        assert!(harness.connection.initialized);

        let logged = harness.logged_json_lines(2);
        assert_eq!(logged[0]["id"], 1);
        assert_eq!(logged[0]["method"], "initialize");
        assert_eq!(logged[0]["params"]["clientInfo"]["name"], "test-client");
        assert_eq!(logged[0]["params"]["clientInfo"]["version"], "test-version");
        assert_eq!(
            logged[0]["params"]["capabilities"]["experimentalApi"],
            false
        );
        assert_eq!(logged[1]["method"], "initialized");
        assert!(logged[1].get("id").is_none());

        let error = harness
            .connection
            .initialize()
            .expect_err("initialize must be exactly once per app-server child");

        assert!(error.to_string().contains("already called"));
    }

    #[test]
    fn typed_requests_reject_uninitialized_connections_before_writing() {
        let mut harness = TestConnection::new(false);

        assert_not_initialized(harness.connection.read_account());
        assert_not_initialized(harness.connection.list_threads(ThreadListParams::default()));
        assert_not_initialized(harness.connection.set_thread_name(ThreadSetNameParams {
            thread_id: "thread-1".to_string(),
            name: "renamed".to_string(),
        }));
        assert_not_initialized(harness.connection.read_thread("thread-1", true));
        assert_not_initialized(
            harness
                .connection
                .start_thread(ThreadStartParams::default()),
        );
        assert_not_initialized(harness.connection.resume_thread(ThreadResumeParams {
            thread_id: "thread-1".to_string(),
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox: None,
            config: None,
        }));
        assert_not_initialized(harness.connection.archive_thread("thread-1"));
        assert_not_initialized(harness.connection.start_turn(TurnStartParams {
            thread_id: "thread-1".to_string(),
            input: vec![TurnInputItem::text("prompt")],
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            model: None,
            effort: None,
        }));
        assert_not_initialized(harness.connection.interrupt_turn(TurnInterruptParams {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
        }));

        assert!(harness.logged_json_lines(0).is_empty());
    }

    #[test]
    fn thread_name_set_sends_exact_thread_and_name() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(json!({ "id": 1, "result": {} }));

        harness
            .connection
            .set_thread_name(ThreadSetNameParams {
                thread_id: "thread-exact".to_string(),
                name: "Release follow-up".to_string(),
            })
            .expect("thread/name/set should succeed");

        let logged = harness.logged_json_lines(1);
        assert_eq!(logged[0]["method"], "thread/name/set");
        assert_eq!(logged[0]["params"]["threadId"], "thread-exact");
        assert_eq!(logged[0]["params"]["name"], "Release follow-up");
    }

    #[test]
    fn typed_requests_serialize_method_specific_payloads_after_initialize() {
        /*
         * The typed helpers are the only place where higher-level app intent becomes
         * app-server method names. This keeps method spelling and request field shape
         * covered without launching a real app-server process.
         */
        let mut harness = TestConnection::new(true);
        harness.send_stdout(json!({
            "id": 1,
            "result": {
                "account": null,
                "requiresOpenAIAuth": false
            }
        }));
        harness.send_stdout(json!({
            "id": 2,
            "result": {
                "data": [thread_record_json("thread-listed")],
                "nextCursor": "cursor-next"
            }
        }));
        harness.send_stdout(json!({
            "id": 3,
            "result": {
                "thread": thread_record_json("thread-read")
            }
        }));
        harness.send_stdout(json!({
            "id": 4,
            "result": {
                "thread": thread_record_json("thread-started")
            }
        }));
        harness.send_stdout(json!({
            "id": 5,
            "result": {
                "thread": thread_record_json("thread-resumed")
            }
        }));
        harness.send_stdout(json!({
            "id": 6,
            "result": {}
        }));
        harness.send_stdout(json!({
            "id": 7,
            "result": {
                "turn": {
                    "id": "turn-started"
                }
            }
        }));
        harness.send_stdout(json!({
            "id": 8,
            "result": {}
        }));

        let account = harness
            .connection
            .read_account()
            .expect("account/read should deserialize");
        let threads = harness
            .connection
            .list_threads(ThreadListParams {
                archived: Some(false),
                cwd: Some("/repo".to_string()),
                limit: Some(25),
                search_term: Some("planning".to_string()),
                source_kinds: Some(vec!["vscode".to_string()]),
            })
            .expect("thread/list should deserialize");
        let read_thread = harness
            .connection
            .read_thread("thread-read", true)
            .expect("thread/read should deserialize");
        let started_thread = harness
            .connection
            .start_thread(ThreadStartParams {
                cwd: Some("/repo".to_string()),
                model: Some("gpt-test".to_string()),
                developer_instructions: Some("stay focused".to_string()),
                service_name: Some("akra-test-worker".to_string()),
                ephemeral: Some(true),
                ..ThreadStartParams::default()
            })
            .expect("thread/start should deserialize");
        let resumed_thread = harness
            .connection
            .resume_thread(ThreadResumeParams {
                thread_id: "thread-resumed".to_string(),
                cwd: None,
                approval_policy: None,
                approvals_reviewer: None,
                sandbox: None,
                config: None,
            })
            .expect("thread/resume should deserialize");
        harness
            .connection
            .archive_thread("thread-resumed")
            .expect("thread/archive should deserialize");
        let started_turn = harness
            .connection
            .start_turn(TurnStartParams {
                thread_id: "thread-started".to_string(),
                input: vec![
                    TurnInputItem::skill("akra-test-skill", "/tmp/SKILL.md"),
                    TurnInputItem::text("prompt"),
                ],
                approval_policy: None,
                approvals_reviewer: None,
                sandbox_policy: None,
                model: Some("gpt-test".to_string()),
                effort: Some(ReasoningEffortValue::Medium),
            })
            .expect("turn/start should deserialize");
        harness
            .connection
            .interrupt_turn(TurnInterruptParams {
                thread_id: "thread-started".to_string(),
                turn_id: "turn-started".to_string(),
            })
            .expect("turn/interrupt should deserialize");

        assert!(account.is_authenticated());
        assert_eq!(threads.data[0].id, "thread-listed");
        assert_eq!(threads.next_cursor.as_deref(), Some("cursor-next"));
        assert_eq!(read_thread.thread.id, "thread-read");
        assert_eq!(started_thread.thread.id, "thread-started");
        assert_eq!(resumed_thread.thread.id, "thread-resumed");
        assert_eq!(started_turn.turn.id, "turn-started");

        let logged = harness.logged_json_lines(8);
        assert_eq!(logged[0]["method"], "account/read");
        assert_eq!(logged[1]["method"], "thread/list");
        assert_eq!(logged[1]["params"]["limit"], 25);
        assert_eq!(logged[1]["params"]["searchTerm"], "planning");
        assert_eq!(logged[2]["method"], "thread/read");
        assert_eq!(logged[2]["params"]["threadId"], "thread-read");
        assert_eq!(logged[2]["params"]["includeTurns"], true);
        assert_eq!(logged[3]["method"], "thread/start");
        assert_eq!(logged[3]["params"]["model"], "gpt-test");
        assert_eq!(logged[3]["params"]["developerInstructions"], "stay focused");
        assert_eq!(logged[3]["params"]["serviceName"], "akra-test-worker");
        assert_eq!(logged[3]["params"]["ephemeral"], true);
        assert_eq!(logged[4]["method"], "thread/resume");
        assert_eq!(logged[4]["params"]["threadId"], "thread-resumed");
        assert_eq!(logged[5]["method"], "thread/archive");
        assert_eq!(logged[5]["params"]["threadId"], "thread-resumed");
        assert_eq!(logged[6]["method"], "turn/start");
        assert_eq!(logged[6]["params"]["input"][0]["type"], "skill");
        assert_eq!(logged[6]["params"]["input"][1]["type"], "text");
        assert_eq!(logged[6]["params"]["effort"], "medium");
        assert_eq!(logged[7]["method"], "turn/interrupt");
        assert_eq!(logged[7]["params"]["turnId"], "turn-started");
    }

    #[test]
    fn send_request_matches_response_and_preserves_transport_warnings() {
        let mut harness = TestConnection::new(true);
        let config_secret = "AKRA_TEST_SECRET_CANARY_CONFIG_WARNING";
        harness.send_stderr("workspace prompt missing");
        harness.send_stdout(json!({
            "id": 99,
            "result": {
                "ignored": true
            }
        }));
        harness.send_stdout(json!({
            "method": "configWarning",
            "params": {
                "summary": config_secret
            }
        }));
        harness.send_stdout(json!({
            "method": "item/agentMessage/delta",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "agent-1",
                "delta": "early"
            }
        }));
        harness.send_stdout(json!({
            "id": 1,
            "result": {
                "ok": true
            }
        }));

        let response: Value = harness
            .connection
            .send_request("unit/test", json!({ "value": 1 }))
            .expect("matching response id should complete the request");

        assert_eq!(response, json!({ "ok": true }));
        assert_eq!(harness.connection.next_request_id, 2);

        let logged = harness.logged_json_lines(1);
        assert_eq!(logged[0]["id"], 1);
        assert_eq!(logged[0]["method"], "unit/test");
        assert_eq!(logged[0]["params"]["value"], 1);

        let warnings = harness.connection.take_warnings();
        assert_contains_warning(&warnings, "workspace prompt missing");
        assert_contains_warning(&warnings, "response id=99 while waiting for id=1");
        assert_contains_warning(&warnings, "configuration warning");
        assert!(
            warnings
                .iter()
                .all(|warning| !warning.contains(config_secret))
        );
        assert_contains_warning(
            &warnings,
            "after the response completed without a turn stream consumer",
        );
    }

    #[test]
    fn turn_start_response_wait_declines_interleaved_approval_before_turn_binding() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(json!({
            "id": "approval-1",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test --lib",
                "cwd": "/workspace",
                "reason": "run focused tests",
                "availableDecisions": ["accept", "decline", "acceptForSession"]
            }
        }));
        harness.send_stdout(json!({
            "id": 1,
            "result": { "ok": true }
        }));
        let (event_sender, event_receiver) = mpsc::channel();
        let response: Value = harness
            .connection
            .send_request_with_event_sender(
                "turn/start",
                json!({ "threadId": "thread-1" }),
                Some(&event_sender),
            )
            .expect("declining an unbound approval should not block the matching response");

        assert_eq!(response, json!({ "ok": true }));
        let logged = harness.logged_json_lines(2);
        assert_eq!(logged[0]["method"], "turn/start");
        assert_eq!(logged[1]["id"], "approval-1");
        assert_eq!(logged[1]["result"]["decision"], "decline");
        assert!(matches!(
            event_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("operator warning should be emitted"),
            ConversationStreamEvent::StatusUpdated { text }
                if text.contains("no active turn binding")
        ));
        assert!(event_receiver.try_recv().is_err());
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn active_turn_exact_approval_is_visible_and_can_be_accepted_once() {
        let mut harness = TestConnection::new(true);
        let request = json!({
            "id": "approval-exact",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-active",
                "turnId": "turn-active",
                "itemId": "command-active",
                "startedAtMs": 1,
                "command": "cargo test --lib",
                "availableDecisions": ["accept", "decline"]
            }
        });
        let signal = AppServerTurnInterruptSignal::default();
        let (event_sender, event_receiver) = mpsc::channel();
        let approval_broker = harness.connection.approval_broker.clone();
        let resolver = thread::spawn(move || {
            let event = event_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("exact approval request should reach the UI channel");
            let ConversationStreamEvent::ApprovalRequested { request } = event else {
                panic!("expected exact approval request, got {event:?}");
            };
            let request_identity = request.identity();
            for expected in [
                "Thread: thread-active",
                "Turn: turn-active",
                "Item: command-active",
            ] {
                assert!(request.details.iter().any(|detail| detail == expected));
            }
            approval_broker
                .resolve(&request.approval_id, ConversationApprovalDecision::Accept)
                .expect("exact approval should resolve once");
            assert!(
                approval_broker
                    .resolve(&request.approval_id, ConversationApprovalDecision::Accept)
                    .is_err()
            );
            let resolved = event_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("approval resolution should reach the UI channel");
            (request_identity, resolved)
        });

        assert!(
            harness
                .connection
                .handle_server_request(
                    &request,
                    Some(&event_sender),
                    Some(bound_approval_context(
                        &signal,
                        "thread-active",
                        "turn-active",
                    )),
                )
                .expect("exact active approval should be handled")
        );

        let logged = harness.logged_json_lines(1);
        assert_eq!(logged[0]["result"]["decision"], "accept");
        let (request_identity, resolved) = resolver.join().expect("resolver thread should finish");
        assert!(matches!(
            resolved,
            ConversationStreamEvent::ApprovalResolved {
                request_identity: resolved_identity,
                resolution: ConversationApprovalResolution::Accepted,
            } if resolved_identity == request_identity
        ));
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn stale_other_thread_and_old_turn_approvals_are_declined_before_broker_registration() {
        let mut harness = TestConnection::new(true);
        let requests = [
            json!({
                "id": "approval-other-thread",
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "thread-other",
                    "turnId": "turn-active",
                    "itemId": "command-other",
                    "startedAtMs": 1,
                    "command": "cargo test",
                    "availableDecisions": ["accept", "decline"]
                }
            }),
            json!({
                "id": "approval-old-turn",
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "thread-active",
                    "turnId": "turn-old",
                    "itemId": "command-old",
                    "startedAtMs": 1,
                    "command": "cargo test",
                    "availableDecisions": ["accept", "decline"]
                }
            }),
        ];
        let signal = AppServerTurnInterruptSignal::default();
        let (event_sender, event_receiver) = mpsc::channel();

        for request in &requests {
            assert!(
                harness
                    .connection
                    .handle_server_request(
                        request,
                        Some(&event_sender),
                        Some(bound_approval_context(
                            &signal,
                            "thread-active",
                            "turn-active",
                        )),
                    )
                    .expect("stale approval should be declined")
            );
        }

        let logged = harness.logged_json_lines(requests.len());
        assert!(
            logged
                .iter()
                .all(|response| response["result"]["decision"] == "decline")
        );
        let events = event_receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(events.len(), requests.len());
        assert!(events.iter().all(|event| matches!(
            event,
            ConversationStreamEvent::StatusUpdated { text }
                if text.contains("did not match the active thread and turn")
        )));
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn file_change_approval_is_declined_without_reaching_the_ui() {
        let mut harness = TestConnection::new(true);
        let request = json!({
            "id": "file-decline",
            "method": "item/fileChange/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "file-1",
                "startedAtMs": 1,
                "reason": "update generated output",
                "grantRoot": "/workspace"
            }
        });
        let (event_sender, event_receiver) = mpsc::channel();
        let signal = AppServerTurnInterruptSignal::default();

        assert!(
            harness
                .connection
                .handle_server_request(
                    &request,
                    Some(&event_sender),
                    Some(bound_approval_context(&signal, "thread-1", "turn-1")),
                )
                .expect("file approval should be handled")
        );

        let logged = harness.logged_json_lines(1);
        assert_eq!(logged[0]["result"], json!({ "decision": "decline" }));
        assert!(matches!(
            event_receiver.try_recv(),
            Ok(ConversationStreamEvent::StatusUpdated { text })
                if text.contains("cannot be reviewed completely")
        ));
        assert!(event_receiver.try_recv().is_err());
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn disconnected_approval_ui_declines_without_leaking_a_broker_entry() {
        let mut harness = TestConnection::new(true);
        let request = json!({
            "id": "approval-disconnected",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        });
        let (event_sender, event_receiver) = conversation_stream_channel();
        drop(event_receiver);
        let signal = AppServerTurnInterruptSignal::default();

        assert!(
            harness
                .connection
                .handle_server_request(
                    &request,
                    Some(&event_sender),
                    Some(bound_approval_context(&signal, "thread-1", "turn-1")),
                )
                .expect("disconnected UI should receive a fail-closed response")
        );

        let logged = harness.logged_json_lines(1);
        assert_eq!(logged[0]["result"], json!({ "decision": "decline" }));
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
        assert!(
            harness
                .connection
                .approval_broker
                .resolve("approval-1", ConversationApprovalDecision::Accept)
                .is_err(),
            "a disconnected delivery must remove the exact broker entry once"
        );
    }

    #[test]
    fn full_live_approval_ui_queue_declines_without_waiting_or_leaking() {
        let mut harness = TestConnection::new(true);
        let request = json!({
            "id": "approval-full",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        });
        let (event_sender, event_receiver) = conversation_stream_channel();
        for sequence in 0..CONVERSATION_STREAM_CHANNEL_CAPACITY {
            event_sender
                .try_send(ConversationStreamEvent::StatusUpdated {
                    text: format!("queued-{sequence}"),
                })
                .expect("fixture should fill the live bounded queue exactly");
        }
        let signal = AppServerTurnInterruptSignal::default();
        let started = Instant::now();

        assert!(
            harness
                .connection
                .handle_server_request(
                    &request,
                    Some(&event_sender),
                    Some(bound_approval_context(&signal, "thread-1", "turn-1")),
                )
                .expect("a full UI queue should fail closed")
        );

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "approval delivery must not wait for capacity in a live bounded queue"
        );
        let logged = harness.logged_json_lines(1);
        assert_eq!(logged[0]["result"], json!({ "decision": "decline" }));
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
        assert!(
            harness
                .connection
                .approval_broker
                .resolve("approval-1", ConversationApprovalDecision::Accept)
                .is_err()
        );
        let queued = event_receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(queued.len(), CONVERSATION_STREAM_CHANNEL_CAPACITY);
        assert!(queued.iter().all(|event| matches!(
            event,
            ConversationStreamEvent::StatusUpdated { text } if text.starts_with("queued-")
        )));
    }

    #[test]
    fn expired_approval_deadline_declines_before_ui_registration() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.approval_timeout = Duration::ZERO;
        let request = json!({
            "id": "approval-expired",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        });
        let signal = AppServerTurnInterruptSignal::default();
        let (event_sender, event_receiver) = mpsc::channel();

        assert!(
            harness
                .connection
                .handle_server_request(
                    &request,
                    Some(&event_sender),
                    Some(bound_approval_context(&signal, "thread-1", "turn-1")),
                )
                .expect("an already expired approval should fail closed")
        );

        assert_eq!(
            harness.logged_json_lines(1)[0]["result"],
            json!({ "decision": "decline" })
        );
        assert!(event_receiver.try_recv().is_err());
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn newer_turn_interrupt_declines_before_approval_reaches_the_ui() {
        let mut harness = TestConnection::new(true);
        let request = json!({
            "id": "approval-interrupted",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        });
        let signal = AppServerTurnInterruptSignal::default();
        let observed_generation = signal.current_generation();
        signal.request_stop_all_sessions();
        let (event_sender, event_receiver) = mpsc::channel();

        assert!(
            harness
                .connection
                .handle_server_request(
                    &request,
                    Some(&event_sender),
                    Some(BoundApprovalContext {
                        thread_id: "thread-1",
                        turn_id: "turn-1",
                        interrupt: ApprovalInterruptContext {
                            signal: &signal,
                            observed_generation,
                        },
                    }),
                )
                .expect("newer interrupt should fail-close the approval")
        );

        let logged = harness.logged_json_lines(1);
        assert_eq!(logged[0]["result"], json!({ "decision": "decline" }));
        assert!(event_receiver.try_recv().is_err());
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn interrupt_after_ui_delivery_declines_and_cleans_the_exact_broker_entry() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.approval_timeout = Duration::from_secs(1);
        let request = json!({
            "id": "approval-interrupted-after-delivery",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        });
        let signal = AppServerTurnInterruptSignal::default();
        let observed_generation = signal.current_generation();
        let signal_for_operator = signal.clone();
        let (event_sender, event_receiver) = mpsc::channel();
        let operator = thread::spawn(move || {
            let requested = event_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("approval should reach the UI before the interrupt");
            let ConversationStreamEvent::ApprovalRequested { request } = requested else {
                panic!("expected approval request, got {requested:?}");
            };
            signal_for_operator.request_stop_all_sessions();
            let resolved = event_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("interrupted approval should emit a resolution");
            (request.approval_id, resolved)
        });

        assert!(
            harness
                .connection
                .handle_server_request(
                    &request,
                    Some(&event_sender),
                    Some(BoundApprovalContext {
                        thread_id: "thread-1",
                        turn_id: "turn-1",
                        interrupt: ApprovalInterruptContext {
                            signal: &signal,
                            observed_generation,
                        },
                    }),
                )
                .expect("an interrupt after delivery should fail-close the approval")
        );

        let (approval_id, resolved) = operator.join().expect("operator thread should finish");
        assert!(matches!(
            resolved,
            ConversationStreamEvent::ApprovalResolved {
                resolution: ConversationApprovalResolution::Interrupted,
                ..
            }
        ));
        assert_eq!(
            harness.logged_json_lines(1)[0]["result"],
            json!({ "decision": "decline" })
        );
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
        assert!(
            harness
                .connection
                .approval_broker
                .resolve(&approval_id, ConversationApprovalDecision::Accept)
                .is_err(),
            "the interrupted broker entry must be removed exactly once"
        );
    }

    #[test]
    fn interrupt_racing_with_received_accept_is_rechecked_before_response() {
        let mut harness = TestConnection::new(true);
        let request = json!({
            "id": "approval-accept-interrupt-race",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        });
        let signal = AppServerTurnInterruptSignal::default();
        let observed_generation = signal.current_generation();
        let signal_during_accept = signal.clone();
        install_after_approval_decision_received_hook(move || {
            signal_during_accept.request_stop_all_sessions();
        });
        let (event_sender, event_receiver) = mpsc::channel();
        let approval_broker = harness.connection.approval_broker.clone();
        let operator = thread::spawn(move || {
            let requested = event_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("approval should reach the UI before the decision race");
            let ConversationStreamEvent::ApprovalRequested { request } = requested else {
                panic!("expected approval request, got {requested:?}");
            };
            approval_broker
                .resolve(&request.approval_id, ConversationApprovalDecision::Accept)
                .expect("operator accept should reach the connection");
            event_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("raced approval should emit a resolution")
        });

        assert!(
            harness
                .connection
                .handle_server_request(
                    &request,
                    Some(&event_sender),
                    Some(BoundApprovalContext {
                        thread_id: "thread-1",
                        turn_id: "turn-1",
                        interrupt: ApprovalInterruptContext {
                            signal: &signal,
                            observed_generation,
                        },
                    }),
                )
                .expect("an interrupt observed after accept receipt should fail closed")
        );

        assert_eq!(
            harness.logged_json_lines(1)[0]["result"],
            json!({ "decision": "decline" })
        );
        assert!(matches!(
            operator.join().expect("operator thread should finish"),
            ConversationStreamEvent::ApprovalResolved {
                resolution: ConversationApprovalResolution::Interrupted,
                ..
            }
        ));
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn deadline_expiring_after_accept_receipt_is_rechecked_before_response() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.approval_timeout = Duration::from_millis(200);
        let request = json!({
            "id": "approval-accept-deadline-race",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        });
        install_after_approval_decision_received_hook(|| {
            thread::sleep(Duration::from_millis(500));
        });
        let signal = AppServerTurnInterruptSignal::default();
        let (event_sender, event_receiver) = mpsc::channel();
        let approval_broker = harness.connection.approval_broker.clone();
        let operator = thread::spawn(move || {
            let requested = event_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("approval should reach the UI before its deadline");
            let ConversationStreamEvent::ApprovalRequested { request } = requested else {
                panic!("expected approval request, got {requested:?}");
            };
            approval_broker
                .resolve(&request.approval_id, ConversationApprovalDecision::Accept)
                .expect("operator accept should reach the connection before the deadline");
            event_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("expired approval should emit a resolution")
        });

        assert!(
            harness
                .connection
                .handle_server_request(
                    &request,
                    Some(&event_sender),
                    Some(bound_approval_context(&signal, "thread-1", "turn-1")),
                )
                .expect("a deadline elapsed after accept receipt should fail closed")
        );

        assert_eq!(
            harness.logged_json_lines(1)[0]["result"],
            json!({ "decision": "decline" })
        );
        assert!(matches!(
            operator.join().expect("operator thread should finish"),
            ConversationStreamEvent::ApprovalResolved {
                resolution: ConversationApprovalResolution::TimedOut,
                ..
            }
        ));
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn server_request_handler_uses_codex_0_144_schema_shaped_fail_closed_responses() {
        let mut harness = TestConnection::new(true);
        let (event_sender, event_receiver) = mpsc::channel();
        let requests = [
            json!({
                "id": "command",
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "itemId": "command-1",
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "startedAtMs": 1,
                    "command": "cat config",
                    "availableDecisions": ["accept", "decline"],
                    "additionalPermissions": {
                        "fileSystem": { "read": ["relative/path"] }
                    }
                }
            }),
            json!({
                "id": "file",
                "method": "item/fileChange/requestApproval",
                "params": {}
            }),
            json!({
                "id": "legacy-command",
                "method": "execCommandApproval",
                "params": {}
            }),
            json!({
                "id": "legacy-file",
                "method": "applyPatchApproval",
                "params": {}
            }),
            json!({
                "id": "permissions",
                "method": "item/permissions/requestApproval",
                "params": {}
            }),
            json!({
                "id": "input",
                "method": "item/tool/requestUserInput",
                "params": {}
            }),
            json!({
                "id": "mcp",
                "method": "mcpServer/elicitation/request",
                "params": {}
            }),
            json!({
                "id": "tool",
                "method": "item/tool/call",
                "params": {}
            }),
            json!({
                "id": "auth",
                "method": "account/chatgptAuthTokens/refresh",
                "params": {}
            }),
            json!({
                "id": "attestation",
                "method": "attestation/generate",
                "params": {}
            }),
            json!({
                "id": "time",
                "method": "currentTime/read",
                "params": { "threadId": "thread-1" }
            }),
            json!({
                "id": "unknown",
                "method": "future/unknown",
                "params": {}
            }),
        ];

        for request in &requests {
            assert!(
                harness
                    .connection
                    .handle_server_request(request, Some(&event_sender), None)
                    .expect("server request should receive a fail-closed response")
            );
        }

        let logged = harness.logged_json_lines(requests.len());
        assert_eq!(logged[0]["result"]["decision"], "decline");
        assert_eq!(logged[1]["result"]["decision"], "decline");
        assert_eq!(logged[2]["result"]["decision"], "denied");
        assert_eq!(logged[3]["result"]["decision"], "denied");
        assert_eq!(logged[4]["result"]["permissions"], json!({}));
        assert_eq!(logged[4]["result"]["scope"], "turn");
        assert_eq!(logged[5]["result"]["answers"], json!({}));
        assert_eq!(logged[6]["result"]["action"], "decline");
        assert_eq!(logged[7]["error"]["code"], -32601);
        assert_eq!(logged[8]["error"]["code"], -32601);
        assert_eq!(logged[9]["error"]["code"], -32601);
        assert!(logged[10]["result"]["currentTimeAt"].as_i64().is_some());
        assert_eq!(logged[11]["error"]["code"], -32601);
        assert!(logged.iter().all(|response| {
            !matches!(
                response.pointer("/result/decision").and_then(Value::as_str),
                Some("accept")
                    | Some("acceptForSession")
                    | Some("approved")
                    | Some("approved_for_session")
            )
        }));
        assert_eq!(event_receiver.try_iter().count(), requests.len() - 1);
    }

    #[test]
    fn oversized_approval_detail_is_declined_without_reaching_the_ui() {
        let mut harness = TestConnection::new(true);
        let (event_sender, event_receiver) = mpsc::channel();
        let signal = AppServerTurnInterruptSignal::default();
        let request = json!({
            "id": "oversized-command",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "itemId": "command-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "command": format!("echo safe {}", "x".repeat(480)),
                "availableDecisions": ["accept", "decline"]
            }
        });

        assert!(
            harness
                .connection
                .handle_server_request(
                    &request,
                    Some(&event_sender),
                    Some(bound_approval_context(&signal, "thread-1", "turn-1")),
                )
                .expect("oversized approval should receive a fail-closed response")
        );

        let logged = harness.logged_json_lines(1);
        assert_eq!(logged[0]["result"]["decision"], "decline");
        let events = event_receiver.try_iter().collect::<Vec<_>>();
        assert!(
            events.iter().all(|event| {
                !matches!(event, ConversationStreamEvent::ApprovalRequested { .. })
            })
        );
        assert!(events.iter().any(|event| {
            matches!(event, ConversationStreamEvent::StatusUpdated { text } if text.contains("declined"))
        }));
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn blank_or_incomplete_command_approvals_are_declined_before_ui_delivery() {
        let mut harness = TestConnection::new(true);
        let (event_sender, event_receiver) = mpsc::channel();
        let signal = AppServerTurnInterruptSignal::default();
        let invalid_fields = [
            json!({ "command": null }),
            json!({ "command": "" }),
            json!({ "command": " \t\n" }),
            json!({ "command": "cargo test", "commandActions": null }),
            json!({
                "command": "cargo test",
                "commandActions": [{ "type": "unknown" }]
            }),
        ];

        for (index, invalid) in invalid_fields.iter().cloned().enumerate() {
            let mut params = json!({
                "itemId": format!("command-{index}"),
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "availableDecisions": ["accept", "decline"]
            });
            params
                .as_object_mut()
                .expect("approval params should be an object")
                .extend(
                    invalid
                        .as_object()
                        .expect("invalid fixture should be an object")
                        .clone(),
                );
            let request = json!({
                "id": format!("invalid-command-{index}"),
                "method": "item/commandExecution/requestApproval",
                "params": params
            });

            assert!(
                harness
                    .connection
                    .handle_server_request(
                        &request,
                        Some(&event_sender),
                        Some(bound_approval_context(&signal, "thread-1", "turn-1")),
                    )
                    .expect("invalid command approval should fail closed")
            );
        }

        let logged = harness.logged_json_lines(invalid_fields.len());
        assert!(
            logged
                .iter()
                .all(|response| response["result"] == json!({ "decision": "decline" }))
        );
        let events = event_receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(events.len(), invalid_fields.len());
        assert!(events.iter().all(|event| matches!(
            event,
            ConversationStreamEvent::StatusUpdated { text } if text.contains("declined")
        )));
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn wait_for_response_reports_protocol_errors_with_diagnostics() {
        let mut harness = TestConnection::new(true);
        harness.send_stderr("fatal: child transport crashed");
        harness.send_stdout(json!({
            "id": 7,
            "error": {
                "message": "boom"
            }
        }));

        let error = harness
            .connection
            .wait_for_response(7)
            .expect_err("JSON-RPC error payload should fail the request");

        assert!(error.to_string().contains("returned error for id 7"));
        assert!(error.to_string().contains("fatal: child transport crashed"));
    }

    #[test]
    fn wait_for_response_reports_missing_result_invalid_json_timeout_and_closed_pipe() {
        let mut missing_result = TestConnection::new(true);
        missing_result.send_stdout(json!({ "id": 3 }));
        let error = missing_result
            .connection
            .wait_for_response(3)
            .expect_err("response without result should be rejected");
        assert!(error.to_string().contains("without a result payload"));

        let mut invalid_json = TestConnection::new(true);
        let malformed_secret = "private-prompt-secret";
        invalid_json
            .tx
            .send(AppServerLine::Stdout(format!(
                "{{\"prompt\":\"{malformed_secret}\""
            )))
            .expect("test channel should accept stdout line");
        let error = invalid_json
            .connection
            .wait_for_response(1)
            .expect_err("invalid JSON line should fail the request");
        assert!(error.to_string().contains("invalid JSON from app-server"));
        assert!(error.to_string().contains("category=Eof"));
        assert!(!error.to_string().contains(malformed_secret));

        let mut timeout = TestConnection::new(true);
        let error = timeout
            .connection
            .wait_for_response(1)
            .expect_err("silent app-server should time out");
        assert!(error.to_string().contains("timed out waiting"));

        let closed_pipe = TestConnection::new(true);
        let mut connection = closed_pipe.connection;
        drop(closed_pipe.tx);
        let error = connection
            .wait_for_response(1)
            .expect_err("closed reader channel should be reported");
        assert!(error.to_string().contains("pipe closed"));
    }

    #[test]
    fn same_connection_turn_steering_writes_exact_request_and_keeps_stream_open() {
        let mut harness = TestConnection::new(true);
        let broker = Arc::new(AppServerTurnSteerBroker::default());
        let binding = broker.bind("thread-1", "turn-1").expect("bind active turn");
        let caller_broker = broker.clone();
        let caller = thread::spawn(move || {
            caller_broker.submit(ConversationTurnSteerRequest {
                thread_id: "thread-1".to_string(),
                expected_turn_id: "turn-1".to_string(),
                prompt: "correct course".to_string(),
            })
        });
        while broker.pending_count() == 0 {
            thread::yield_now();
        }
        harness.send_stdout(json!({
            "id": 1,
            "result": { "turnId": "turn-1" }
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, _event_receiver) = mpsc::channel();

        let terminal = harness
            .connection
            .wait_for_turn_stream_with_steering(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
                &broker,
                binding,
            )
            .expect("steering success must not end the active stream");

        assert!(terminal.is_completed_and_confirmed());
        assert_eq!(
            caller
                .join()
                .expect("steering caller should finish")
                .expect("matching response should reach caller"),
            ConversationTurnSteerReceipt {
                turn_id: "turn-1".to_string()
            }
        );
        assert_eq!(
            harness.logged_json_lines(1),
            vec![json!({
                "id": 1,
                "method": "turn/steer",
                "params": {
                    "threadId": "thread-1",
                    "input": [{ "type": "text", "text": "correct course" }],
                    "expectedTurnId": "turn-1"
                }
            })]
        );
    }

    #[test]
    fn approval_review_time_does_not_consume_turn_steer_response_deadline() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.response_timeout = Duration::from_millis(10);
        harness.connection.config.approval_timeout = Duration::from_secs(1);
        let broker = Arc::new(AppServerTurnSteerBroker::default());
        let binding = broker.bind("thread-1", "turn-1").expect("bind active turn");
        let caller_broker = broker.clone();
        let caller = thread::spawn(move || {
            caller_broker.submit(ConversationTurnSteerRequest {
                thread_id: "thread-1".to_string(),
                expected_turn_id: "turn-1".to_string(),
                prompt: "steer after approval".to_string(),
            })
        });
        while broker.pending_count() == 0 {
            thread::yield_now();
        }
        harness.send_stdout(json!({
            "id": "approval-before-steer",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        }));
        harness.send_stdout(json!({
            "id": 1,
            "result": { "turnId": "turn-1" }
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, event_receiver) = mpsc::channel();
        let approval_broker = harness.connection.approval_broker.clone();
        let resolver = thread::spawn(move || {
            let event = event_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("approval should reach the operator");
            let ConversationStreamEvent::ApprovalRequested { request } = event else {
                panic!("expected approval request, got {event:?}");
            };
            thread::sleep(Duration::from_millis(40));
            approval_broker
                .resolve(&request.approval_id, ConversationApprovalDecision::Accept)
                .expect("approval should resolve");
            event_receiver
        });

        let terminal = harness
            .connection
            .wait_for_turn_stream_with_steering(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
                &broker,
                binding,
            )
            .expect("operator review must not expire the steer response budget");

        let _event_receiver = resolver.join().expect("approval resolver should finish");
        assert!(terminal.is_completed_and_confirmed());
        assert_eq!(
            caller
                .join()
                .expect("steering caller should finish")
                .expect("steering should succeed after approval"),
            ConversationTurnSteerReceipt {
                turn_id: "turn-1".to_string()
            }
        );
        let logged = harness.logged_json_lines(2);
        assert_eq!(logged[0]["method"], "turn/steer");
        assert_eq!(logged[1]["id"], "approval-before-steer");
        assert_eq!(logged[1]["result"]["decision"], "accept");
    }

    #[test]
    fn terminal_before_steer_response_preserves_authoritative_terminal_and_fails_caller() {
        let mut harness = TestConnection::new(true);
        let broker = Arc::new(AppServerTurnSteerBroker::default());
        let binding = broker.bind("thread-1", "turn-1").expect("bind active turn");
        let caller_broker = broker.clone();
        let caller = thread::spawn(move || {
            caller_broker.submit(ConversationTurnSteerRequest {
                thread_id: "thread-1".to_string(),
                expected_turn_id: "turn-1".to_string(),
                prompt: "late correction".to_string(),
            })
        });
        while broker.pending_count() == 0 {
            thread::yield_now();
        }
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, _event_receiver) = mpsc::channel();

        let terminal = harness
            .connection
            .wait_for_turn_stream_with_steering(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
                &broker,
                binding,
            )
            .expect("terminal notification must win over the missing steer response");
        let caller_error = caller
            .join()
            .expect("steering caller should finish")
            .expect_err("unacknowledged steering must keep the caller draft");

        assert!(terminal.is_completed_and_confirmed());
        assert!(
            caller_error
                .to_string()
                .contains("before app-server confirmed")
        );
        assert_eq!(harness.logged_json_lines(1)[0]["method"], "turn/steer");
    }

    #[test]
    fn turn_steering_remote_error_fails_only_caller_and_preserves_terminal_stream() {
        let mut harness = TestConnection::new(true);
        let broker = Arc::new(AppServerTurnSteerBroker::default());
        let binding = broker.bind("thread-1", "turn-1").expect("bind active turn");
        let caller_broker = broker.clone();
        let caller = thread::spawn(move || {
            caller_broker.submit(ConversationTurnSteerRequest {
                thread_id: "thread-1".to_string(),
                expected_turn_id: "turn-1".to_string(),
                prompt: "late correction".to_string(),
            })
        });
        while broker.pending_count() == 0 {
            thread::yield_now();
        }
        harness.send_stdout(json!({
            "id": 1,
            "error": { "code": -32602, "message": "active turn is not steerable" }
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, _event_receiver) = mpsc::channel();

        let terminal = harness
            .connection
            .wait_for_turn_stream_with_steering(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
                &broker,
                binding,
            )
            .expect("explicit steering rejection must leave the stream healthy");
        let caller_error = caller
            .join()
            .expect("steering caller should finish")
            .expect_err("remote error must fail the steering caller");

        assert!(terminal.is_completed_and_confirmed());
        assert!(caller_error.to_string().contains("not steerable"));
        assert_eq!(harness.logged_json_lines(1)[0]["method"], "turn/steer");
        assert!(
            harness
                .connection
                .take_warnings()
                .iter()
                .any(|warning| warning.contains("stream remains connected"))
        );
    }

    #[test]
    fn invalid_turn_steering_responses_fail_caller_without_ending_stream() {
        for (response, expected_error) in [
            (
                json!({ "id": 1, "result": { "turnId": "turn-other" } }),
                "did not match",
            ),
            (json!({ "id": 1, "result": {} }), "failed to deserialize"),
            (json!({ "id": 1 }), "without a result payload"),
        ] {
            let mut harness = TestConnection::new(true);
            let broker = Arc::new(AppServerTurnSteerBroker::default());
            let binding = broker.bind("thread-1", "turn-1").expect("bind active turn");
            let caller_broker = broker.clone();
            let caller = thread::spawn(move || {
                caller_broker.submit(ConversationTurnSteerRequest {
                    thread_id: "thread-1".to_string(),
                    expected_turn_id: "turn-1".to_string(),
                    prompt: "keep this draft".to_string(),
                })
            });
            while broker.pending_count() == 0 {
                thread::yield_now();
            }
            harness.send_stdout(response);
            harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
            let (event_sender, _event_receiver) = mpsc::channel();

            let terminal = harness
                .connection
                .wait_for_turn_stream_with_steering(
                    "thread-1",
                    "turn-1",
                    &AppServerTurnInterruptSignal::default(),
                    0,
                    &event_sender,
                    &broker,
                    binding,
                )
                .expect("an invalid correlated response must leave the stream usable");
            let caller_error = caller
                .join()
                .expect("steering caller should finish")
                .expect_err("an invalid response must fail the steering caller");

            assert!(terminal.is_completed_and_confirmed());
            assert!(caller_error.to_string().contains(expected_error));
            assert_eq!(
                harness.logged_json_lines(1)[0]["params"]["input"],
                json!([{ "type": "text", "text": "keep this draft" }])
            );
        }
    }

    #[test]
    fn terminal_before_steering_dispatch_rejects_pending_caller_without_writing() {
        let mut harness = TestConnection::new(true);
        let broker = Arc::new(AppServerTurnSteerBroker::default());
        let binding = broker.bind("thread-1", "turn-1").expect("bind active turn");
        let caller_broker = broker.clone();
        let caller = thread::spawn(move || {
            caller_broker.submit(ConversationTurnSteerRequest {
                thread_id: "thread-1".to_string(),
                expected_turn_id: "turn-1".to_string(),
                prompt: "too late".to_string(),
            })
        });
        while broker.pending_count() == 0 {
            thread::yield_now();
        }
        assert!(
            harness
                .connection
                .pending_notifications
                .try_push(notification(completed_turn_notification(
                    "thread-1", "turn-1"
                )))
        );
        let (event_sender, _event_receiver) = mpsc::channel();

        let terminal = harness
            .connection
            .wait_for_turn_stream_with_steering(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
                &broker,
                binding,
            )
            .expect("queued terminal should remain authoritative");
        let caller_error = caller
            .join()
            .expect("steering caller should finish")
            .expect_err("terminal unbind must reject pending steering");

        assert!(terminal.is_completed_and_confirmed());
        assert!(caller_error.to_string().contains("completed"));
        assert!(harness.logged_json_lines(0).is_empty());
    }

    #[test]
    fn turn_steering_transport_error_unbinds_pending_caller_without_writing() {
        let mut harness = TestConnection::new(true);
        let broker = Arc::new(AppServerTurnSteerBroker::default());
        let binding = broker.bind("thread-1", "turn-1").expect("bind active turn");
        let caller_broker = broker.clone();
        let caller = thread::spawn(move || {
            caller_broker.submit(ConversationTurnSteerRequest {
                thread_id: "thread-1".to_string(),
                expected_turn_id: "turn-1".to_string(),
                prompt: "preserve after disconnect".to_string(),
            })
        });
        while broker.pending_count() == 0 {
            thread::yield_now();
        }
        harness
            .connection
            .transport_failure
            .record("synthetic steering transport failure");
        let (event_sender, _event_receiver) = mpsc::channel();

        assert!(
            harness
                .connection
                .wait_for_turn_stream_with_steering(
                    "thread-1",
                    "turn-1",
                    &AppServerTurnInterruptSignal::default(),
                    0,
                    &event_sender,
                    &broker,
                    binding,
                )
                .is_err()
        );
        let caller_error = caller
            .join()
            .expect("steering caller should finish")
            .expect_err("transport failure must reject pending steering");

        assert!(caller_error.to_string().contains("connection ended"));
        assert!(harness.logged_json_lines(0).is_empty());
    }

    #[test]
    fn turn_stream_reduces_stdout_notifications_and_records_loose_messages() {
        let mut harness = TestConnection::new(true);
        let config_secret = "AKRA_TEST_SECRET_CANARY_STREAM_CONFIG_WARNING";
        harness.send_stdout(json!({
            "id": 55,
            "result": {
                "not": "a notification"
            }
        }));
        harness.send_stderr("stream side warning");
        harness.send_stdout(json!({
            "method": "thread/status/changed",
            "params": {
                "threadId": "thread-1",
                "status": {
                    "type": "running"
                }
            }
        }));
        harness.send_stdout(json!({
            "method": "configWarning",
            "params": {
                "summary": config_secret
            }
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, event_receiver) = mpsc::channel();

        harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("turn/completed should finish the stream");

        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![
                ConversationStreamEvent::RuntimeEnvelopeObserved {
                    observation: Box::new(
                        ConversationRuntimeEnvelopeObservation::ThreadStatusChanged {
                            thread_id: "thread-1".to_string(),
                            status: ConversationRuntimeObservedValue::Observed(
                                ConversationRuntimeThreadStatus::Unknown("running".to_string()),
                            ),
                        },
                    ),
                },
                ConversationStreamEvent::TurnTerminal {
                    receipt: confirmed_completed_receipt("thread-1", "turn-1", Vec::new()),
                },
            ]
        );

        let warnings = harness.connection.take_warnings();
        assert_contains_warning(&warnings, "non-notification JSON message");
        assert_contains_warning(&warnings, "stream side warning");
        assert_contains_warning(&warnings, "configuration warning");
        assert!(
            warnings
                .iter()
                .all(|warning| !warning.contains(config_secret))
        );
    }

    #[test]
    fn retry_error_keeps_stream_open_until_confirmed_completion() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(json!({
            "method": "error",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "willRetry": true,
                "error": {
                    "message": "temporary upstream overload",
                    "codexErrorInfo": "serverOverloaded"
                }
            }
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, event_receiver) = mpsc::channel();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("retry should continue to the authoritative completion");

        assert!(receipt.is_completed_and_confirmed());
        let events = event_receiver.try_iter().collect::<Vec<_>>();
        assert!(matches!(
            events.as_slice(),
            [
                ConversationStreamEvent::TurnRetrying { error, .. },
                ConversationStreamEvent::TurnTerminal { receipt }
            ] if error.message == "temporary upstream overload"
                && receipt.is_completed_and_confirmed()
        ));
    }

    #[test]
    fn non_retry_error_without_final_turn_expires_to_confirmed_unknown() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.terminal_grace_timeout = Duration::from_millis(2);
        harness.send_stdout(json!({
            "method": "error",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "willRetry": false,
                "error": {
                    "message": "non-retry stream failure",
                    "additionalDetails": "final turn omitted"
                }
            }
        }));
        let (event_sender, event_receiver) = mpsc::channel();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("live transport should close the grace deadline with an unknown receipt");

        assert!(matches!(
            receipt.outcome,
            ConversationTurnTerminalOutcome::Unknown {
                reason: ConversationTurnTerminalUncertainty::NonRetryErrorGraceExpired,
                observed_error: Some(_),
            }
        ));
        assert_eq!(receipt.items_view, ConversationTurnItemsView::NotLoaded);
        assert_eq!(
            receipt.application_delivery,
            ConversationTurnApplicationDelivery::Confirmed
        );
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![ConversationStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
            }]
        );
    }

    #[test]
    fn grace_recovery_declines_buffered_approval_without_waiting_for_operator_timeout() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.terminal_grace_timeout = Duration::ZERO;
        harness.connection.config.approval_timeout = Duration::from_secs(5);
        assert!(
            harness
                .connection
                .pending_notifications
                .try_push(notification(json!({
                    "method": "error",
                    "params": {
                        "threadId": "thread-1",
                        "turnId": "turn-1",
                        "willRetry": false,
                        "error": { "message": "grace candidate" }
                    }
                })))
        );
        harness.send_stdout(json!({
            "id": "approval-during-grace-recovery",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        }));
        let (event_sender, event_receiver) = mpsc::channel();
        let started_at = Instant::now();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("grace recovery should return a typed unknown receipt");

        assert!(started_at.elapsed() < Duration::from_millis(500));
        assert!(matches!(
            receipt.outcome,
            ConversationTurnTerminalOutcome::Unknown {
                reason: ConversationTurnTerminalUncertainty::NonRetryErrorGraceExpired,
                ..
            }
        ));
        assert_eq!(receipt.items_view, ConversationTurnItemsView::NotLoaded);
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![ConversationStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
            }]
        );
        assert_eq!(
            harness.logged_json_lines(1)[0]["result"],
            json!({ "decision": "decline" })
        );
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn grace_recovery_write_deadline_taints_transport_without_hiding_unknown_receipt() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.response_timeout = Duration::from_secs(30);
        harness.connection.config.approval_timeout = Duration::from_secs(30);
        harness.connection.config.terminal_grace_timeout = Duration::ZERO;
        assert!(
            harness
                .connection
                .pending_notifications
                .try_push(notification(json!({
                    "method": "error",
                    "params": {
                        "threadId": "thread-1",
                        "turnId": "turn-1",
                        "willRetry": false,
                        "error": { "message": "grace candidate before blocked decline" }
                    }
                })))
        );
        harness.send_stdout(json!({
            "id": "blocked-grace-recovery-approval",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        }));
        let (blocked_writer_sender, blocked_writer_receiver) =
            mpsc::sync_channel(APP_SERVER_WRITE_CHANNEL_CAPACITY);
        let original_writer = std::mem::replace(
            &mut harness.connection.stdin_writer,
            AppServerStdinWriter {
                sender: Some(blocked_writer_sender),
                worker: None,
            },
        );
        let (event_sender, event_receiver) = mpsc::channel();
        let started_at = Instant::now();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("a recovery write timeout must not hide the typed unknown receipt");
        let blocked_writer =
            std::mem::replace(&mut harness.connection.stdin_writer, original_writer);
        drop(blocked_writer);
        drop(blocked_writer_receiver);

        assert!(started_at.elapsed() < Duration::from_millis(100));
        assert!(matches!(
            receipt.outcome,
            ConversationTurnTerminalOutcome::Unknown {
                reason: ConversationTurnTerminalUncertainty::NonRetryErrorGraceExpired,
                ..
            }
        ));
        assert_eq!(receipt.items_view, ConversationTurnItemsView::NotLoaded);
        assert!(
            harness
                .connection
                .transport_failure
                .current()
                .is_some_and(|message| message.contains("terminal-recovery JSON-RPC frame"))
        );
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![ConversationStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
            }]
        );
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn pending_nonterminal_burst_yields_to_interrupt_safety_deadline() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.interrupt_total_timeout = Duration::ZERO;
        for index in 0..super::MAX_PENDING_TURN_NOTIFICATIONS_PER_POLL * 2 {
            assert!(
                harness
                    .connection
                    .pending_notifications
                    .try_push(notification(json!({
                        "method": "thread/status/changed",
                        "params": {
                            "threadId": "thread-1",
                            "status": { "type": format!("queued-{index}") }
                        }
                    })))
            );
        }
        let signal = AppServerTurnInterruptSignal::default();
        let observed_generation = signal.current_generation();
        signal.request_stop_all_sessions();
        let (event_sender, _event_receiver) = mpsc::channel();
        let started_at = Instant::now();

        let error = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &signal,
                observed_generation,
                &event_sender,
            )
            .expect_err("interrupt safety deadline must preempt a pending nonterminal burst");

        assert!(started_at.elapsed() < Duration::from_millis(500));
        assert!(
            error
                .to_string()
                .contains("stop request failed within the bounded app-server interrupt deadline")
        );
        assert!(!harness.connection.pending_notifications.is_empty());
    }

    #[test]
    fn expired_interrupt_deadline_beats_simultaneous_grace_unknown() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.terminal_grace_timeout = Duration::ZERO;
        harness.connection.config.interrupt_total_timeout = Duration::from_millis(2);
        assert!(
            harness
                .connection
                .pending_notifications
                .try_push(notification(json!({
                    "method": "error",
                    "params": {
                        "threadId": "thread-1",
                        "turnId": "turn-1",
                        "willRetry": false,
                        "error": { "message": "grace candidate racing with stop" }
                    }
                })))
        );
        harness.send_stdout(json!({ "id": 1, "result": {} }));
        harness.send_stdout(json!({
            "id": "approval-during-interrupt-deadline",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        }));
        let signal = AppServerTurnInterruptSignal::default();
        let observed_generation = signal.current_generation();
        signal.request_stop_all_sessions();
        let (event_sender, event_receiver) = mpsc::channel();

        harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &signal,
                observed_generation,
                &event_sender,
            )
            .expect_err("an expired interrupt deadline must beat grace-expired Unknown");

        let events = event_receiver.try_iter().collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            ConversationStreamEvent::TurnInterruptRequestFailed { .. }
        )));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ConversationStreamEvent::TurnTerminal { .. }))
        );
    }

    #[test]
    fn authoritative_failed_turn_replaces_non_retry_error_candidate() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(json!({
            "method": "error",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "willRetry": false,
                "error": { "message": "candidate failure" }
            }
        }));
        harness.send_stdout(json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": {
                    "id": "turn-1",
                    "items": [],
                    "status": "failed",
                    "error": {
                        "message": "authoritative failure",
                        "codexErrorInfo": "serverOverloaded"
                    }
                }
            }
        }));
        let (event_sender, _event_receiver) = mpsc::channel();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("authoritative failed turn is a transport-success receipt");

        assert!(matches!(
            receipt.outcome,
            ConversationTurnTerminalOutcome::Failed { ref error }
                if error.message == "authoritative failure"
        ));
        assert!(!receipt.is_completed_and_confirmed());
    }

    #[test]
    fn malformed_terminal_identity_returns_confirmed_unknown_receipt() {
        for (params, missing_field) in [
            (
                json!({
                    "turn": { "id": "turn-1", "items": [], "status": "completed" }
                }),
                "threadId",
            ),
            (
                json!({
                    "threadId": null,
                    "turn": { "id": "turn-1", "items": [], "status": "completed" }
                }),
                "threadId",
            ),
            (
                json!({
                    "threadId": "",
                    "turn": { "id": "turn-1", "items": [], "status": "completed" }
                }),
                "threadId",
            ),
            (
                json!({
                    "threadId": 7,
                    "turn": { "id": "turn-1", "items": [], "status": "completed" }
                }),
                "threadId",
            ),
            (
                json!({
                    "threadId": "thread-1",
                    "turn": { "items": [], "status": "completed" }
                }),
                "turn.id",
            ),
            (
                json!({
                    "threadId": "thread-1",
                    "turn": { "id": null, "items": [], "status": "completed" }
                }),
                "turn.id",
            ),
            (
                json!({
                    "threadId": "thread-1",
                    "turn": { "id": "", "items": [], "status": "completed" }
                }),
                "turn.id",
            ),
            (
                json!({
                    "threadId": "thread-1",
                    "turn": { "id": 7, "items": [], "status": "completed" }
                }),
                "turn.id",
            ),
        ] {
            let mut harness = TestConnection::new(true);
            harness.send_stdout(json!({
                "method": "turn/completed",
                "params": params,
            }));
            let (event_sender, event_receiver) = mpsc::channel();

            let receipt = harness
                .connection
                .wait_for_turn_stream(
                    "thread-1",
                    "turn-1",
                    &AppServerTurnInterruptSignal::default(),
                    0,
                    &event_sender,
                )
                .expect("malformed terminal identity should remain a typed terminal fact");

            assert!(matches!(
                receipt.outcome,
                ConversationTurnTerminalOutcome::Unknown {
                    reason: ConversationTurnTerminalUncertainty::MissingRequiredIdentity {
                        ref field
                    },
                    ..
                } if field == missing_field
            ));
            assert_eq!(
                receipt.application_delivery,
                ConversationTurnApplicationDelivery::Confirmed
            );
            assert!(harness.connection.transport_failure.current().is_none());
            assert_eq!(
                event_receiver.try_iter().collect::<Vec<_>>(),
                vec![ConversationStreamEvent::TurnTerminal {
                    receipt: receipt.clone(),
                }]
            );
            assert!(
                harness
                    .connection
                    .child
                    .try_wait()
                    .expect("healthy child status should be observable")
                    .is_none()
            );
        }
    }

    #[test]
    fn late_malformed_terminal_closes_the_next_turn_as_recovery_pending() {
        let mut harness = TestConnection::new(true);
        assert!(
            harness
                .connection
                .pending_notifications
                .try_push(notification(json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": "thread-1",
                        "turn": { "items": [], "status": "completed" }
                    }
                })))
        );
        let (event_sender, event_receiver) = mpsc::channel();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-next",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("a late terminal without identity should remain unknown");

        assert_eq!(receipt.thread_id, "thread-1");
        assert_eq!(receipt.turn_id, "turn-next");
        assert!(matches!(
            receipt.outcome,
            ConversationTurnTerminalOutcome::Unknown {
                reason: ConversationTurnTerminalUncertainty::MissingRequiredIdentity {
                    ref field
                },
                ..
            } if field == "turn.id"
        ));
        assert!(!receipt.is_completed_and_confirmed());
        assert!(harness.connection.transport_failure.current().is_none());
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![ConversationStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
            }]
        );
    }

    #[test]
    fn queued_terminal_receipt_wins_over_exited_child() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        harness.send_stdout_eof();
        harness
            .connection
            .child
            .terminate_and_wait()
            .expect("fixture child should exit after the terminal line was queued");
        let (event_sender, event_receiver) = mpsc::channel();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("a queued terminal receipt must win over later child exit observation");

        assert!(receipt.is_completed_and_confirmed());
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![ConversationStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
            }]
        );
    }

    #[test]
    fn exited_child_recovery_skips_buffered_approval_and_preserves_terminal() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.approval_timeout = Duration::from_secs(5);
        harness.send_stdout(json!({
            "id": "approval-before-exited-terminal",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        harness.send_stdout_eof();
        harness
            .connection
            .child
            .terminate_and_wait()
            .expect("fixture child should exit after the terminal line was queued");
        let (event_sender, event_receiver) = mpsc::channel();
        let started_at = Instant::now();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("closed-transport recovery should still find the queued terminal");

        assert!(started_at.elapsed() < Duration::from_millis(500));
        assert!(receipt.is_completed_and_confirmed());
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![ConversationStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
            }]
        );
        assert!(harness.logged_json_lines(0).is_empty());
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn exited_child_recovery_observes_capacity_one_reader_handoff() {
        let mut harness = TestConnection::new(true);
        let (line_sender, line_receiver) = mpsc::sync_channel(APP_SERVER_LINE_CHANNEL_CAPACITY);
        harness.connection.rx = line_receiver;
        line_sender
            .send(AppServerLine::Stdout(
                json!({
                    "method": "thread/status/changed",
                    "params": {
                        "threadId": "thread-1",
                        "status": { "type": "busy" }
                    }
                })
                .to_string(),
            ))
            .expect("the production-capacity channel should accept the first line");
        let (release_terminal, await_status_delivery) = mpsc::channel();
        let event_sender = RecoveryHandoffEventSender::new(release_terminal);
        let terminal_sender = thread::spawn(move || {
            await_status_delivery
                .recv()
                .expect("status reduction should release the terminal sender");
            line_sender
                .send(AppServerLine::Stdout(
                    completed_turn_notification("thread-1", "turn-1").to_string(),
                ))
                .expect("reader handoff should retain the terminal line");
            line_sender
                .send(AppServerLine::ReaderFinished {
                    source: AppServerReaderSource::Stdout,
                    termination: AppServerReaderTermination::EndOfFile,
                })
                .expect("reader handoff should finish with stdout EOF");
        });
        harness
            .connection
            .child
            .terminate_and_wait()
            .expect("fixture child should exit before terminal recovery");

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("bounded recovery should observe the capacity-one reader handoff");
        terminal_sender
            .join()
            .expect("terminal sender should finish after the bounded handoff");

        assert!(receipt.is_completed_and_confirmed());
        assert_eq!(
            event_sender.events(),
            vec![
                ConversationStreamEvent::RuntimeEnvelopeObserved {
                    observation: Box::new(
                        ConversationRuntimeEnvelopeObservation::ThreadStatusChanged {
                            thread_id: "thread-1".to_string(),
                            status: ConversationRuntimeObservedValue::Observed(
                                ConversationRuntimeThreadStatus::Unknown("busy".to_string()),
                            ),
                        },
                    ),
                },
                ConversationStreamEvent::TurnTerminal {
                    receipt: receipt.clone(),
                },
            ]
        );
    }

    #[test]
    fn exited_child_recovery_waits_for_initial_capacity_one_terminal_handoff() {
        let mut harness = TestConnection::new(true);
        let (line_sender, line_receiver) = mpsc::sync_channel(APP_SERVER_LINE_CHANNEL_CAPACITY);
        harness.connection.rx = line_receiver;
        harness
            .connection
            .child
            .terminate_and_wait()
            .expect("fixture child should exit before the reader handoff");
        let handoff = Arc::new(Barrier::new(2));
        let sender_handoff = handoff.clone();
        let terminal_sender = thread::spawn(move || {
            sender_handoff.wait();
            line_sender
                .send(AppServerLine::Stdout(
                    completed_turn_notification("thread-1", "turn-1").to_string(),
                ))
                .expect("the terminal-only reader handoff should remain connected");
            line_sender
                .send(AppServerLine::ReaderFinished {
                    source: AppServerReaderSource::Stdout,
                    termination: AppServerReaderTermination::EndOfFile,
                })
                .expect("terminal-only handoff should finish with stdout EOF");
        });
        let (event_sender, event_receiver) = mpsc::channel();
        handoff.wait();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("initial Empty must allow one bounded terminal-only reader handoff");
        terminal_sender
            .join()
            .expect("terminal-only sender should finish inside the recovery window");

        assert!(receipt.is_completed_and_confirmed());
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![ConversationStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
            }]
        );
    }

    #[test]
    fn transport_closing_drain_preserves_terminal_after_large_capacity_one_burst() {
        const STATUS_COUNT: usize = 40;

        let mut harness = TestConnection::new(true);
        let (line_sender, line_receiver) = mpsc::sync_channel(APP_SERVER_LINE_CHANNEL_CAPACITY);
        harness.connection.rx = line_receiver;
        harness
            .connection
            .child
            .terminate_and_wait()
            .expect("fixture child should exit before transport-closing recovery");
        let (release_terminal, await_statuses) = mpsc::channel();
        let event_sender =
            RecoveryHandoffEventSender::after_statuses(STATUS_COUNT, release_terminal);
        let reader = thread::spawn(move || {
            for index in 0..STATUS_COUNT {
                line_sender
                    .send(AppServerLine::Stdout(
                        json!({
                            "method": "thread/status/changed",
                            "params": {
                                "threadId": "thread-1",
                                "status": { "type": format!("closing-{index}") }
                            }
                        })
                        .to_string(),
                    ))
                    .expect("capacity-one reader should backpressure without dropping status");
            }
            await_statuses
                .recv()
                .expect("all status reductions should release the delayed terminal");
            line_sender
                .send(AppServerLine::Stdout(
                    completed_turn_notification("thread-1", "turn-1").to_string(),
                ))
                .expect("terminal should follow the delayed nonterminal burst");
            line_sender
                .send(AppServerLine::ReaderFinished {
                    source: AppServerReaderSource::Stdout,
                    termination: AppServerReaderTermination::EndOfFile,
                })
                .expect("reader should publish EOF after the terminal");
        });

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("EOF-aware recovery should preserve a terminal beyond 32 lines");
        reader.join().expect("fixture reader should reach EOF");

        assert!(receipt.is_completed_and_confirmed());
        let events = event_sender.events();
        assert_eq!(events.len(), STATUS_COUNT + 1);
        assert_eq!(
            events.last(),
            Some(&ConversationStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
            })
        );
    }

    #[test]
    fn transport_closing_stdout_eof_without_terminal_returns_child_error() {
        let mut harness = TestConnection::new(true);
        let (line_sender, line_receiver) = mpsc::sync_channel(APP_SERVER_LINE_CHANNEL_CAPACITY);
        harness.connection.rx = line_receiver;
        harness
            .connection
            .child
            .terminate_and_wait()
            .expect("fixture child should exit before EOF recovery");
        line_sender
            .send(AppServerLine::ReaderFinished {
                source: AppServerReaderSource::Stdout,
                termination: AppServerReaderTermination::EndOfFile,
            })
            .expect("fixture should publish stdout EOF");
        let (event_sender, event_receiver) = mpsc::channel();
        let started_at = Instant::now();

        let error = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect_err("EOF without a terminal must remain a child-exit error");

        assert!(started_at.elapsed() < Duration::from_millis(50));
        assert!(
            error
                .to_string()
                .contains("exited before the turn completed")
        );
        assert!(event_receiver.try_iter().next().is_none());
    }

    #[test]
    fn exited_child_recovery_beats_expired_grace_without_writing_approval() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.terminal_grace_timeout = Duration::ZERO;
        assert!(
            harness
                .connection
                .pending_notifications
                .try_push(notification(json!({
                    "method": "error",
                    "params": {
                        "threadId": "thread-1",
                        "turnId": "turn-1",
                        "willRetry": false,
                        "error": { "message": "expired candidate before child exit" }
                    }
                })))
        );
        let approval = json!({
            "id": "approval-after-expired-grace",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        });
        let (line_sender, line_receiver) = mpsc::sync_channel(APP_SERVER_LINE_CHANNEL_CAPACITY);
        harness.connection.rx = line_receiver;
        harness
            .connection
            .child
            .terminate_and_wait()
            .expect("fixture child should exit after terminal lines were queued");
        let reader = thread::spawn(move || {
            for line in [approval, completed_turn_notification("thread-1", "turn-1")] {
                line_sender
                    .send(AppServerLine::Stdout(line.to_string()))
                    .expect("transport-closing fixture line should be consumed");
            }
            line_sender
                .send(AppServerLine::ReaderFinished {
                    source: AppServerReaderSource::Stdout,
                    termination: AppServerReaderTermination::EndOfFile,
                })
                .expect("transport-closing fixture should publish stdout EOF");
        });
        let (event_sender, event_receiver) = mpsc::channel();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("transport-closing recovery should win over expired grace synthesis");
        reader
            .join()
            .expect("transport-closing fixture reader should finish");

        assert!(receipt.is_completed_and_confirmed());
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![ConversationStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
            }]
        );
        assert!(harness.logged_json_lines(0).is_empty());
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn pending_authoritative_terminal_wins_over_expired_non_retry_grace() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.terminal_grace_timeout = Duration::ZERO;
        assert!(
            harness
                .connection
                .pending_notifications
                .try_push(notification(json!({
                    "method": "error",
                    "params": {
                        "threadId": "thread-1",
                        "turnId": "turn-1",
                        "willRetry": false,
                        "error": { "message": "candidate before queued terminal" }
                    }
                })))
        );
        assert!(
            harness
                .connection
                .pending_notifications
                .try_push(notification(completed_turn_notification(
                    "thread-1", "turn-1"
                )))
        );
        let (event_sender, event_receiver) = mpsc::channel();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("an authoritative pending terminal must win over grace expiry");

        assert!(receipt.is_completed_and_confirmed());
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![ConversationStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
            }]
        );
    }

    #[test]
    fn terminal_sink_full_without_retry_preserves_unconfirmed_upstream_completion() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.terminal_delivery_timeout = Duration::ZERO;
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, _event_receiver) = mpsc::sync_channel(0);

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("full application sink must not erase upstream terminal truth");

        assert!(matches!(
            receipt.outcome,
            ConversationTurnTerminalOutcome::Completed
        ));
        assert_eq!(
            receipt.application_delivery,
            ConversationTurnApplicationDelivery::Unconfirmed(
                ConversationTurnApplicationDeliveryFailure::Full,
            )
        );
        assert!(!receipt.is_completed_and_confirmed());
    }

    #[test]
    fn terminal_sink_full_through_deadline_is_unconfirmed() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.terminal_delivery_timeout = Duration::from_millis(2);
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, _event_receiver) = mpsc::sync_channel(0);

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("bounded delivery timeout should return a typed receipt");

        assert_eq!(
            receipt.application_delivery,
            ConversationTurnApplicationDelivery::Unconfirmed(
                ConversationTurnApplicationDeliveryFailure::DeadlineExceeded,
            )
        );
    }

    #[test]
    fn nonterminal_events_cannot_block_terminal_delivery_deadline() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.terminal_delivery_timeout = Duration::from_millis(2);
        for status in ["first", "second"] {
            harness.send_stdout(json!({
                "method": "thread/status/changed",
                "params": {
                    "threadId": "thread-1",
                    "status": { "type": status }
                }
            }));
        }
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, event_receiver) = mpsc::sync_channel(1);

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("ordinary backpressure must not prevent a typed terminal result");

        assert_eq!(
            receipt.application_delivery,
            ConversationTurnApplicationDelivery::Unconfirmed(
                ConversationTurnApplicationDeliveryFailure::DeadlineExceeded,
            )
        );
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![ConversationStreamEvent::RuntimeEnvelopeObserved {
                observation: Box::new(
                    ConversationRuntimeEnvelopeObservation::ThreadStatusChanged {
                        thread_id: "thread-1".to_string(),
                        status: ConversationRuntimeObservedValue::Observed(
                            ConversationRuntimeThreadStatus::Unknown("first".to_string()),
                        ),
                    },
                ),
            }]
        );
    }

    #[test]
    fn rejected_runtime_observation_delivers_gap_before_confirmed_terminal() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(json!({
            "method": "thread/settings/updated",
            "params": {
                "threadId": "thread-1",
                "threadSettings": {
                    "model": "gpt-settings",
                    "modelProvider": "openai",
                    "effort": "medium",
                    "serviceTier": null,
                    "cwd": "/repo",
                    "approvalPolicy": "on-request",
                    "approvalsReviewer": "user",
                    "sandboxPolicy": { "type": "readOnly" },
                    "activePermissionProfile": null,
                    "collaborationMode": {}
                }
            }
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let event_sender = RuntimeGapPressureEventSender::new();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("gap and terminal should retain typed delivery");

        assert!(receipt.is_completed_and_confirmed());
        let events = event_sender.events();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ConversationStreamEvent::RuntimeEnvelopeObserved { observation }
                if matches!(
                    observation.as_ref(),
                    ConversationRuntimeEnvelopeObservation::ProjectionGap { gap, .. }
                        if *gap == ConversationRuntimeObservationGap::settings()
                )
        ));
        assert!(matches!(
            &events[1],
            ConversationStreamEvent::TurnTerminal { receipt }
                if receipt.is_completed_and_confirmed()
        ));
    }

    #[test]
    fn rejected_retry_fact_cannot_be_promoted_by_later_completion() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(json!({
            "method": "error",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "willRetry": true,
                "error": { "message": "retry before completion" }
            }
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, _event_receiver) = mpsc::sync_channel(1);
        event_sender
            .try_send(ConversationStreamEvent::StatusUpdated {
                text: "occupy application sink".to_string(),
            })
            .expect("fixture should fill the bounded sink");

        let error = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect_err("a lost retry fact must fail closed before later completion");

        assert!(error.to_string().contains("turn/retrying"));
    }

    #[test]
    fn terminal_event_and_return_share_the_same_bounded_receipt() {
        let oversized_thread_id = format!("thread-{}", "t".repeat(8 * 1024));
        let oversized_turn_id = format!("turn-{}", "u".repeat(8 * 1024));
        let mut harness = TestConnection::new(true);
        harness.send_stdout(completed_turn_notification(
            &oversized_thread_id,
            &oversized_turn_id,
        ));
        let (event_sender, event_receiver) = mpsc::channel();

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                &oversized_thread_id,
                &oversized_turn_id,
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("oversized identifiers should be bounded at the adapter boundary");
        let ConversationStreamEvent::TurnTerminal {
            receipt: projected_receipt,
        } = event_receiver
            .recv()
            .expect("terminal event should be delivered")
        else {
            panic!("expected terminal receipt event");
        };

        assert_eq!(projected_receipt, receipt);
        assert!(receipt.is_completed_and_confirmed());
        assert!(receipt.thread_id.len() < oversized_thread_id.len());
        assert!(receipt.turn_id.len() < oversized_turn_id.len());
    }

    #[test]
    fn terminal_delivery_bounds_all_receipt_fields_before_projection() {
        let oversized_thread_id = format!("thread-{}", "t".repeat(8 * 1024));
        let oversized_turn_id = format!("turn-{}", "u".repeat(8 * 1024));
        let oversized_path = format!(".codex-exec-loop/planning/{}.md", "p".repeat(32 * 1024));
        let oversized_items_view = "future-view-".repeat(8 * 1024);
        let upstream_receipt = ConversationTurnTerminalReceipt::completed(
            oversized_thread_id.clone(),
            oversized_turn_id.clone(),
            vec![oversized_path.clone()],
        )
        .with_turn_metadata(
            ConversationTurnItemsView::Unknown(oversized_items_view.clone()),
            Some(10),
            Some(20),
            Some(10_000),
        );
        let mut harness = TestConnection::new(true);
        let (event_sender, event_receiver) = mpsc::channel();
        let mut notification_state = super::ActiveTurnNotificationState::new();

        let receipt = harness.connection.deliver_terminal_receipt(
            upstream_receipt,
            &oversized_thread_id,
            &mut notification_state,
            &event_sender,
        );
        let ConversationStreamEvent::TurnTerminal {
            receipt: projected_receipt,
        } = event_receiver
            .recv()
            .expect("terminal event should be delivered")
        else {
            panic!("expected terminal receipt event");
        };

        assert_eq!(projected_receipt, receipt);
        assert!(receipt.is_completed_and_confirmed());
        assert!(receipt.thread_id.len() < oversized_thread_id.len());
        assert!(receipt.turn_id.len() < oversized_turn_id.len());
        assert!(receipt.observations.changed_planning_file_paths[0].len() < oversized_path.len());
        let ConversationTurnItemsView::Unknown(items_view) = &receipt.items_view else {
            panic!("unknown items view should be retained");
        };
        assert!(items_view.len() < oversized_items_view.len());
    }

    #[test]
    fn disconnected_terminal_sink_is_unconfirmed() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, event_receiver) = mpsc::sync_channel(1);
        drop(event_receiver);

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("disconnected application sink must retain upstream terminal truth");

        assert_eq!(
            receipt.application_delivery,
            ConversationTurnApplicationDelivery::Unconfirmed(
                ConversationTurnApplicationDeliveryFailure::Disconnected,
            )
        );
    }

    #[test]
    fn temporarily_full_terminal_sink_can_recover_before_deadline() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.terminal_delivery_timeout = Duration::from_millis(20);
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, event_receiver) = mpsc::sync_channel(1);
        event_sender
            .try_send(ConversationStreamEvent::StatusUpdated {
                text: "occupy terminal sink".to_string(),
            })
            .expect("fixture should fill the bounded sink");
        let drain_worker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(2));
            let first = event_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("preloaded event should drain");
            let terminal = event_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("terminal event should be retried after drain");
            (first, terminal)
        });

        let receipt = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("temporary backpressure should recover before the deadline");
        let (_, terminal) = drain_worker.join().expect("drain worker should finish");

        assert!(receipt.is_completed_and_confirmed());
        assert_eq!(
            terminal,
            ConversationStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
            }
        );
    }

    #[test]
    fn turn_stream_declines_file_change_without_prompt_and_continues() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(json!({
            "id": "approval-stream",
            "method": "item/fileChange/requestApproval",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "file-change-1",
                "startedAtMs": 1
            }
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, event_receiver) = mpsc::channel();

        harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("file-change decline should not block turn completion");

        let logged = harness.logged_json_lines(1);
        assert_eq!(logged[0]["id"], "approval-stream");
        assert_eq!(logged[0]["result"]["decision"], "decline");
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![
                ConversationStreamEvent::StatusUpdated {
                    text: "app-server approval request `item/fileChange/requestApproval` declined (requested file changes and grant scope cannot be reviewed completely)"
                        .to_string(),
                },
                ConversationStreamEvent::TurnTerminal {
                    receipt: confirmed_completed_receipt("thread-1", "turn-1", Vec::new()),
                },
            ]
        );
        assert_eq!(harness.connection.approval_broker.pending_count(), 0);
    }

    #[test]
    fn unattended_stream_declines_approval_without_emitting_a_prompt() {
        let mut harness = TestConnection::new(true);
        harness.connection.approval_mode = AppServerApprovalMode::Unattended;
        harness.send_stdout(json!({
            "id": "hidden-approval",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-hidden",
                "turnId": "turn-hidden",
                "itemId": "command-hidden",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        }));
        harness.send_stdout(completed_turn_notification("thread-hidden", "turn-hidden"));
        let (event_sender, event_receiver) = mpsc::channel();

        harness
            .connection
            .wait_for_turn_stream(
                "thread-hidden",
                "turn-hidden",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("unattended approval decline should not block completion");

        let logged = harness.logged_json_lines(1);
        assert_eq!(logged[0]["result"]["decision"], "decline");
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![ConversationStreamEvent::TurnTerminal {
                receipt: confirmed_completed_receipt("thread-hidden", "turn-hidden", Vec::new(),),
            }]
        );
    }

    #[test]
    fn turn_stream_consumes_deferred_notifications_before_blocking_for_more_lines() {
        let mut harness = TestConnection::new(true);
        assert!(
            harness
                .connection
                .pending_notifications
                .try_push(notification(json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "completedAtMs": 1,
                    "item": {
                        "id": "file-change-1",
                        "type": "fileChange",
                        "status": "completed",
                        "changes": [
                            {
                                "path": ".codex-exec-loop/planning/result-output.md",
                                "diff": "",
                                "kind": {
                                    "type": "update"
                                }
                            },
                            {
                                "path": "src/main.rs",
                                "diff": "",
                                "kind": {
                                    "type": "update"
                                }
                            }
                        ]
                    }
                }
                })))
        );
        assert!(
            harness
                .connection
                .pending_notifications
                .try_push(notification(completed_turn_notification(
                    "thread-1", "turn-1"
                )))
        );
        let (event_sender, event_receiver) = mpsc::channel();

        harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("pending turn/completed should finish the stream");

        let events = event_receiver.try_iter().collect::<Vec<_>>();
        assert!(matches!(
            events.as_slice(),
            [
                ConversationStreamEvent::ItemLifecycleObserved { .. },
                ConversationStreamEvent::ToolActivity { .. },
                ConversationStreamEvent::TurnTerminal { .. }
            ]
        ));
        assert_eq!(
            events.last(),
            Some(&ConversationStreamEvent::TurnTerminal {
                receipt: confirmed_completed_receipt(
                    "thread-1",
                    "turn-1",
                    vec![RESULT_OUTPUT_FILE_PATH.to_string()],
                ),
            })
        );
    }

    #[test]
    fn malformed_active_item_completion_cannot_be_followed_by_confirmed_terminal_delivery() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "completedAtMs": 1,
                "item": {
                    "id": "agent-final",
                    "type": "agentMessage",
                    "text": "authoritative final answer"
                }
            }
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, event_receiver) = mpsc::channel();

        let error = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect_err("malformed final item must fail before terminal confirmation");

        assert!(
            error
                .to_string()
                .contains("missing or invalid lifecycle correlation identity")
        );
        assert!(event_receiver.try_iter().all(|event| !matches!(
            event,
            ConversationStreamEvent::AgentMessageCompleted { .. }
                | ConversationStreamEvent::TurnTerminal { .. }
        )));
    }

    #[test]
    fn timestamp_regressed_first_completion_delivers_agent_text_before_terminal_confirmation() {
        let mut harness = TestConnection::new(true);
        let item = json!({
            "id": "agent-clock-regression",
            "type": "agentMessage",
            "text": "authoritative final answer"
        });
        harness.send_stdout(json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 10,
                "item": item.clone()
            }
        }));
        harness.send_stdout(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 9,
                "item": item
            }
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, event_receiver) = mpsc::channel();

        harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect("clock regression must not discard a first completion payload");

        let events = event_receiver.try_iter().collect::<Vec<_>>();
        assert!(matches!(
            events.as_slice(),
            [
                ConversationStreamEvent::ItemLifecycleObserved { .. },
                ConversationStreamEvent::ItemLifecycleObserved { .. },
                ConversationStreamEvent::AgentMessageCompleted { text, .. },
                ConversationStreamEvent::TurnTerminal { .. }
            ] if text == "authoritative final answer"
        ));
    }

    #[test]
    fn item_kind_drift_cannot_discard_agent_text_then_confirm_terminal_delivery() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1,
                "item": {
                    "id": "kind-drift",
                    "type": "contextCompaction"
                }
            }
        }));
        harness.send_stdout(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 2,
                "item": {
                    "id": "kind-drift",
                    "type": "agentMessage",
                    "text": "must not be silently lost"
                }
            }
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, event_receiver) = mpsc::channel();

        let error = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &AppServerTurnInterruptSignal::default(),
                0,
                &event_sender,
            )
            .expect_err("kind drift must fail before terminal confirmation");

        assert!(error.to_string().contains("changed kind"));
        let events = event_receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    ConversationStreamEvent::ItemLifecycleObserved { .. }
                ))
                .count(),
            2
        );
        assert!(events.iter().all(|event| !matches!(
            event,
            ConversationStreamEvent::AgentMessageCompleted { .. }
                | ConversationStreamEvent::TurnTerminal { .. }
        )));
    }

    #[test]
    fn turn_stream_translates_new_interrupt_generation_once() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(json!({
            "id": 1,
            "result": {}
        }));
        harness.send_stdout(completed_turn_notification("thread-1", "turn-1"));
        let (event_sender, event_receiver) = mpsc::channel();
        let signal = AppServerTurnInterruptSignal::default();
        let observed_generation = signal.current_generation();
        signal.request_stop_all_sessions();

        harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &signal,
                observed_generation,
                &event_sender,
            )
            .expect("stream should continue after successful interrupt request");

        let logged = harness.logged_json_lines(1);
        assert_eq!(logged[0]["method"], "turn/interrupt");
        assert_eq!(logged[0]["params"]["threadId"], "thread-1");
        assert_eq!(logged[0]["params"]["turnId"], "turn-1");

        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![
                ConversationStreamEvent::StatusUpdated {
                    text: "stop requested / app-server interrupt sent".to_string(),
                },
                ConversationStreamEvent::TurnTerminal {
                    receipt: confirmed_completed_receipt("thread-1", "turn-1", Vec::new()),
                },
            ]
        );

        harness.send_stdout(json!({
            "id": 2,
            "result": { "connection": "reused" }
        }));
        assert_eq!(
            harness
                .connection
                .send_request::<Value>("test/after-interrupt", json!({}))
                .expect("a normally terminated interrupt must preserve the connection"),
            json!({ "connection": "reused" })
        );
    }

    #[test]
    fn acknowledged_interrupt_without_terminal_event_fails_closed() {
        let mut harness = TestConnection::new(true);
        harness.send_stdout(json!({
            "id": 1,
            "result": {}
        }));
        let (event_sender, event_receiver) = mpsc::channel();
        let signal = AppServerTurnInterruptSignal::default();
        let observed_generation = signal.current_generation();
        signal.request_stop_all_sessions();

        let error = harness
            .connection
            .wait_for_turn_stream(
                "thread-ack-only",
                "turn-ack-only",
                &signal,
                observed_generation,
                &event_sender,
            )
            .expect_err("an interrupt acknowledgement cannot replace a terminal turn event");
        assert!(error.to_string().contains("acknowledged the stop request"));
        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![
                ConversationStreamEvent::StatusUpdated {
                    text: "stop requested / app-server interrupt sent".to_string(),
                },
                ConversationStreamEvent::TurnInterruptRequestFailed {
                    message: "app-server acknowledged the stop request but did not terminate the active turn within the bounded interrupt deadline; terminating the active app-server process tree"
                        .to_string(),
                },
            ]
        );
        let follow_up_error = harness
            .connection
            .send_request::<Value>("test/after-ack-timeout", json!({}))
            .expect_err("an ack-only connection must remain poisoned");
        assert!(follow_up_error.to_string().contains("transport failed"));
    }

    #[test]
    fn turn_stream_closes_connection_after_correlated_interrupt_retries_fail() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.interrupt_total_timeout = Duration::from_secs(1);
        for request_id in 1..=3 {
            harness.send_stdout(json!({
                "id": request_id,
                "error": {
                    "message": "interrupt rejected"
                }
            }));
        }
        let (event_sender, event_receiver) = mpsc::channel();
        let signal = AppServerTurnInterruptSignal::default();
        let observed_generation = signal.current_generation();
        signal.request_stop_all_sessions();

        let error = harness
            .connection
            .wait_for_turn_stream(
                "thread-1",
                "turn-1",
                &signal,
                observed_generation,
                &event_sender,
            )
            .expect_err("exhausted interrupt retries must close the active connection");
        assert!(
            error
                .to_string()
                .contains("terminating the active app-server")
        );

        assert_eq!(
            event_receiver.try_iter().collect::<Vec<_>>(),
            vec![ConversationStreamEvent::TurnInterruptRequestFailed {
                message: "stop request failed within the bounded app-server interrupt deadline; terminating the active app-server process tree"
                    .to_string(),
            }]
        );

        assert_eq!(harness.connection.next_request_id, 4);

        let warnings = harness.connection.take_warnings();
        assert_contains_warning(&warnings, "interrupt attempt 1/3 failed");
        assert_contains_warning(&warnings, "interrupt attempt 3/3 failed");

        let follow_up_error = harness
            .connection
            .send_request::<Value>("account/read", json!({}))
            .expect_err("a fail-closed connection must reject every follow-up request");
        assert!(follow_up_error.to_string().contains("transport failed"));

        let mut replacement = TestConnection::new(true);
        replacement.send_stdout(json!({
            "id": 1,
            "result": { "recovered": true }
        }));
        assert_eq!(
            replacement
                .connection
                .send_request::<Value>("test/recovery", json!({}))
                .expect("a new connection should recover independently"),
            json!({ "recovered": true })
        );
    }

    #[test]
    fn production_interrupt_deadline_is_short_and_independent_from_response_timeout() {
        let config = AppServerConnectionConfig::default();
        assert_eq!(config.interrupt_total_timeout, Duration::from_secs(3));
        assert!(config.interrupt_total_timeout < config.response_timeout);
        assert_eq!(config.interrupt_retry_limit, 3);
    }

    #[test]
    fn stop_during_silent_turn_start_terminates_transport_within_deadline() {
        let mut harness = TestConnection::new(true);
        harness.connection.config.response_timeout = Duration::from_secs(30);
        let signal = AppServerTurnInterruptSignal::default();
        let observed_generation = signal.current_generation();
        let stop_signal = signal.clone();
        let stop_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            stop_signal.request_stop_all_sessions();
        });
        let (event_sender, event_receiver) = mpsc::channel();
        let started_at = Instant::now();

        let error = harness
            .connection
            .start_turn_with_event_sender(
                TurnStartParams {
                    thread_id: "thread-1".to_string(),
                    input: vec![TurnInputItem::text("prompt")],
                    approval_policy: None,
                    approvals_reviewer: None,
                    sandbox_policy: None,
                    model: None,
                    effort: None,
                },
                &event_sender,
                &signal,
                observed_generation,
            )
            .expect_err("stop must abort a turn/start peer that never responds");
        stop_thread.join().expect("stop thread should finish");

        assert!(
            error.to_string().contains("before app-server acknowledged"),
            "unexpected turn/start stop error: {error:#}"
        );
        assert!(
            started_at.elapsed() < Duration::from_secs(3),
            "turn/start stop must not inherit the normal response timeout"
        );
        assert!(matches!(
            event_receiver.try_recv(),
            Ok(ConversationStreamEvent::TurnInterruptRequestFailed { message })
                if message.contains("before app-server acknowledged")
        ));
        assert!(
            harness
                .connection
                .child
                .try_wait()
                .expect("terminated child status should be observable")
                .is_some()
        );
    }

    #[test]
    fn shared_stop_interrupts_unattended_pre_stream_response_waits() {
        for method in ["initialize", "thread/start", "thread/resume"] {
            let signal = AppServerTurnInterruptSignal::default();
            let mut harness = TestConnection::new_with_interrupt_signal(
                true,
                AppServerApprovalMode::Unattended,
                signal.clone(),
            );
            harness.connection.config.response_timeout = Duration::from_secs(30);
            let stop_signal = signal.clone();
            let stop_thread = thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                stop_signal.request_stop_all_sessions();
            });
            let started_at = Instant::now();

            let error = harness
                .connection
                .send_request::<Value>(method, json!({}))
                .expect_err("shared stop must abort every silent pre-stream request");
            stop_thread.join().expect("stop thread should finish");

            assert!(
                error.to_string().contains(method),
                "pre-stream stop error should identify {method}: {error:#}"
            );
            assert!(
                started_at.elapsed() < Duration::from_secs(3),
                "unattended {method} must observe the shared stop generation"
            );
            assert!(
                harness
                    .connection
                    .child
                    .try_wait()
                    .expect("terminated hidden-worker child should be observable")
                    .is_some()
            );
        }
    }

    #[test]
    fn stale_shared_stop_does_not_cancel_a_new_request() {
        let signal = AppServerTurnInterruptSignal::default();
        signal.request_stop_all_sessions();
        let mut harness = TestConnection::new_with_interrupt_signal(
            true,
            AppServerApprovalMode::Unattended,
            signal,
        );
        harness.send_stdout(json!({
            "id": 1,
            "result": { "ok": true }
        }));

        assert_eq!(
            harness
                .connection
                .send_request::<Value>("thread/read", json!({}))
                .expect("a stop predating the request must be treated as stale"),
            json!({ "ok": true })
        );
    }

    #[cfg(unix)]
    #[test]
    fn full_app_server_stdin_is_bounded_by_write_ack_timeout() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child =
            subprocess::spawn(&mut command).expect("non-reading app-server child should spawn");
        let stdin = child
            .take_stdin()
            .expect("non-reading app-server stdin should be piped");
        let (_tx, rx) = mpsc::sync_channel(1);
        let transport_failure = Arc::new(TransportFailure::default());
        let mut config = test_config();
        config.response_timeout = Duration::from_millis(50);
        let mut connection = AppServerConnection {
            child,
            stdin_writer: AppServerStdinWriter::spawn(stdin, transport_failure.clone()),
            rx,
            transport_failure,
            diagnostics: ConnectionDiagnostics::default(),
            pending_notifications: PendingNotifications::default(),
            next_request_id: 1,
            client_name: "test-client".to_string(),
            client_version: "test-version".to_string(),
            initialized: true,
            config,
            terminal_recovery_write_deadline: None,
            approval_broker: Arc::new(AppServerApprovalBroker::default()),
            approval_mode: AppServerApprovalMode::Interactive,
            interrupt_signal: AppServerTurnInterruptSignal::default(),
        };
        let started_at = Instant::now();

        let error = connection
            .send_json_line(json!({ "payload": "x".repeat(1024 * 1024) }))
            .expect_err("a peer that does not read stdin must time out");

        assert!(
            error.to_string().contains("timed out writing"),
            "unexpected blocked-write error: {error:#}"
        );
        assert!(
            started_at.elapsed() < Duration::from_secs(1),
            "blocked stdin must not pin the control thread"
        );
        assert!(
            connection
                .child
                .try_wait()
                .expect("terminated child status should be observable")
                .is_some()
        );
    }

    #[cfg(unix)]
    #[test]
    fn writer_shutdown_detaches_when_a_live_reader_keeps_write_blocked() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child =
            subprocess::spawn(&mut command).expect("non-reading app-server child should spawn");
        let stdin = child
            .take_stdin()
            .expect("non-reading app-server stdin should be piped");
        let transport_failure = Arc::new(TransportFailure::default());
        let mut writer = AppServerStdinWriter::spawn(stdin, transport_failure);
        let (acknowledgement, acknowledgement_receiver) = mpsc::sync_channel(1);
        writer
            .try_send(AppServerWriteRequest {
                frame: vec![b'x'; 4 * 1024 * 1024],
                acknowledgement,
            })
            .expect("writer should accept one bounded request");
        thread::sleep(Duration::from_millis(20));
        assert!(
            writer
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished()),
            "non-reading peer should keep the writer blocked"
        );
        let started_at = Instant::now();

        writer.shutdown();

        assert!(
            started_at.elapsed() < Duration::from_millis(500),
            "writer shutdown must not join an indefinitely blocked write"
        );
        assert!(acknowledgement_receiver.try_recv().is_err());
        child
            .terminate_and_wait()
            .expect("non-reading fixture should terminate");
    }

    #[test]
    fn is_alive_reflects_child_process_exit() {
        let mut harness = TestConnection::new(true);

        assert!(
            harness
                .connection
                .is_alive()
                .expect("live fake child should be observable")
        );

        harness
            .connection
            .child
            .terminate_and_wait()
            .expect("test app-server containment cleanup should succeed");

        assert!(
            !harness
                .connection
                .is_alive()
                .expect("exited fake child should be observable")
        );
    }

    #[cfg(unix)]
    #[test]
    fn dropping_connection_terminates_long_lived_app_server_descendants() {
        let pid_path = unique_log_path().with_extension("descendant-pid");
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "sleep 30 & descendant=$!; printf '%s' \"$descendant\" > \"$1\"; while IFS= read -r _; do :; done",
                "fake-app-server-with-descendant",
            ])
            .arg(&pid_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child =
            subprocess::spawn(&mut command).expect("fake app-server process tree should spawn");
        let stdin = child
            .take_stdin()
            .expect("fake app-server stdin should be piped");
        let (tx, rx) = mpsc::sync_channel(1);
        let transport_failure = Arc::new(TransportFailure::default());
        let connection = AppServerConnection {
            child,
            stdin_writer: AppServerStdinWriter::spawn(stdin, transport_failure.clone()),
            rx,
            transport_failure,
            diagnostics: ConnectionDiagnostics::default(),
            pending_notifications: PendingNotifications::default(),
            next_request_id: 1,
            client_name: "test-client".to_string(),
            client_version: "test-version".to_string(),
            initialized: true,
            config: test_config(),
            terminal_recovery_write_deadline: None,
            approval_broker: Arc::new(AppServerApprovalBroker::default()),
            approval_mode: AppServerApprovalMode::Interactive,
            interrupt_signal: AppServerTurnInterruptSignal::default(),
        };
        let descendant_pid = wait_for_published_pid(&pid_path, "app-server descendant");

        drop(connection);
        drop(tx);
        let gone_deadline = Instant::now() + Duration::from_secs(2);
        while crate::process_liveness::process_is_alive(descendant_pid)
            .expect("descendant liveness should be inspectable")
            && Instant::now() < gone_deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::remove_file(&pid_path);
        assert!(
            !crate::process_liveness::process_is_alive(descendant_pid)
                .expect("descendant liveness should remain inspectable"),
            "dropping app-server connection must terminate descendant {descendant_pid}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn silent_interrupt_fail_close_terminates_descendants_inside_test_deadline() {
        let pid_path = unique_log_path().with_extension("silent-interrupt-descendant-pid");
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "sleep 30 & descendant=$!; printf '%s' \"$descendant\" > \"$1\"; while IFS= read -r _; do :; done",
                "silent-fake-app-server-with-descendant",
            ])
            .arg(&pid_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child =
            subprocess::spawn(&mut command).expect("silent fake app-server tree should spawn");
        let stdin = child
            .take_stdin()
            .expect("silent fake app-server stdin should be piped");
        let (tx, rx) = mpsc::sync_channel(1);
        let transport_failure = Arc::new(TransportFailure::default());
        let mut config = test_config();
        config.interrupt_total_timeout = Duration::from_millis(50);
        let mut connection = AppServerConnection {
            child,
            stdin_writer: AppServerStdinWriter::spawn(stdin, transport_failure.clone()),
            rx,
            transport_failure,
            diagnostics: ConnectionDiagnostics::default(),
            pending_notifications: PendingNotifications::default(),
            next_request_id: 1,
            client_name: "test-client".to_string(),
            client_version: "test-version".to_string(),
            initialized: true,
            config,
            terminal_recovery_write_deadline: None,
            approval_broker: Arc::new(AppServerApprovalBroker::default()),
            approval_mode: AppServerApprovalMode::Interactive,
            interrupt_signal: AppServerTurnInterruptSignal::default(),
        };
        let descendant_pid = wait_for_published_pid(&pid_path, "silent app-server descendant");
        let signal = AppServerTurnInterruptSignal::default();
        let observed_generation = signal.current_generation();
        signal.request_stop_all_sessions();
        let (event_sender, _event_receiver) = mpsc::channel();
        let started_at = Instant::now();

        let error = connection
            .wait_for_turn_stream(
                "thread-silent",
                "turn-silent",
                &signal,
                observed_generation,
                &event_sender,
            )
            .expect_err("a silent interrupt peer must fail closed");
        assert!(
            error
                .to_string()
                .contains("terminating the active app-server")
        );
        assert!(
            started_at.elapsed() < Duration::from_secs(1),
            "test interrupt deadline must bound process-tree termination"
        );

        let gone_deadline = Instant::now() + Duration::from_secs(2);
        while crate::process_liveness::process_is_alive(descendant_pid)
            .expect("silent descendant liveness should be inspectable")
            && Instant::now() < gone_deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::remove_file(&pid_path);
        assert!(
            !crate::process_liveness::process_is_alive(descendant_pid)
                .expect("silent descendant liveness should remain inspectable"),
            "failed interrupt must terminate descendant {descendant_pid}"
        );
        let follow_up_error = connection
            .send_request::<Value>("test/after-stop", json!({}))
            .expect_err("terminated connection must remain poisoned");
        assert!(follow_up_error.to_string().contains("transport failed"));
        connection.terminate_child();
        connection.terminate_child();
        drop(tx);
    }

    #[test]
    fn pipe_reader_classifies_stdout_and_stderr_lines() {
        let (tx, rx) = mpsc::sync_channel(APP_SERVER_LINE_CHANNEL_CAPACITY);
        let transport_failure = Arc::new(TransportFailure::default());

        spawn_pipe_reader(
            Cursor::new(b"out-one\nout-two\n".to_vec()),
            tx.clone(),
            false,
            MAX_STDOUT_LINE_BYTES,
            transport_failure.clone(),
        );
        spawn_pipe_reader(
            Cursor::new(b"err-one\n".to_vec()),
            tx,
            true,
            MAX_STDERR_LINE_BYTES,
            transport_failure.clone(),
        );

        let mut stdout_lines = Vec::new();
        let mut stderr_lines = Vec::new();
        let mut finished_readers = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        while stdout_lines.len() < 2 || stderr_lines.is_empty() || finished_readers.len() < 2 {
            assert!(
                Instant::now() < deadline,
                "pipe reader did not send all expected lines"
            );
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(AppServerLine::Stdout(line)) => stdout_lines.push(line),
                Ok(AppServerLine::Stderr(line)) => stderr_lines.push(line),
                Ok(AppServerLine::ReaderFinished {
                    source,
                    termination,
                }) => finished_readers.push((source, termination)),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        stdout_lines.sort();
        assert_eq!(stdout_lines, vec!["out-one", "out-two"]);
        assert_eq!(stderr_lines, vec!["err-one"]);
        finished_readers.sort_by_key(|(source, _)| match source {
            AppServerReaderSource::Stdout => 0,
            AppServerReaderSource::Stderr => 1,
        });
        assert_eq!(
            finished_readers,
            vec![
                (
                    AppServerReaderSource::Stdout,
                    AppServerReaderTermination::EndOfFile,
                ),
                (
                    AppServerReaderSource::Stderr,
                    AppServerReaderTermination::EndOfFile,
                ),
            ]
        );
        assert!(transport_failure.current().is_none());
    }

    #[test]
    fn bounded_line_reader_accepts_the_limit_and_rejects_limit_plus_one() {
        const TEST_LINE_LIMIT: usize = 8 * 1024;
        let mut exact = vec![b'a'; TEST_LINE_LIMIT];
        exact.push(b'\n');
        let mut exact_reader = BufReader::new(Cursor::new(exact));
        let BoundedLineRead::Line(line) = read_bounded_line(&mut exact_reader, TEST_LINE_LIMIT)
            .expect("the exact line limit should be readable")
        else {
            panic!("the exact line limit should produce one line");
        };
        assert_eq!(line.len(), TEST_LINE_LIMIT);

        let mut oversized_reader = BufReader::new(Cursor::new(vec![b'b'; TEST_LINE_LIMIT + 1]));
        assert!(matches!(
            read_bounded_line(&mut oversized_reader, TEST_LINE_LIMIT)
                .expect("line overflow is a protocol state, not an I/O error"),
            BoundedLineRead::LimitExceeded
        ));
    }

    #[test]
    fn bounded_line_reader_rejects_non_utf8_protocol_bytes() {
        let mut reader = BufReader::new(Cursor::new(vec![0xff, b'\n']));
        let error = match read_bounded_line(&mut reader, 16) {
            Err(error) => error,
            Ok(_) => panic!("app-server protocol lines must be valid UTF-8"),
        };

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn stdout_limit_supports_complete_long_lived_thread_responses() {
        let payload = vec![b'a'; 2 * 1024 * 1024 + 1];
        let mut reader = BufReader::new(Cursor::new(payload));
        let BoundedLineRead::Line(line) = read_bounded_line(&mut reader, MAX_STDOUT_LINE_BYTES)
            .expect("a multi-megabyte thread response should remain readable")
        else {
            panic!("a normal long thread response must not poison the transport");
        };
        assert_eq!(line.len(), 2 * 1024 * 1024 + 1);
    }

    #[test]
    fn pipe_reader_applies_backpressure_without_dropping_a_valid_burst() {
        let (tx, rx) = mpsc::sync_channel(1);
        let transport_failure = Arc::new(TransportFailure::default());
        spawn_pipe_reader(
            Cursor::new(b"first\nsecond\nthird\n".to_vec()),
            tx,
            false,
            MAX_STDOUT_LINE_BYTES,
            transport_failure.clone(),
        );

        for expected in ["first", "second", "third"] {
            assert!(matches!(
                rx.recv_timeout(Duration::from_secs(1)),
                Ok(AppServerLine::Stdout(line)) if line == expected
            ));
        }
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(1)),
            Ok(AppServerLine::ReaderFinished {
                source: AppServerReaderSource::Stdout,
                termination: AppServerReaderTermination::EndOfFile,
            })
        ));
        assert!(transport_failure.current().is_none());
    }

    #[test]
    fn oversized_pipe_line_poisoning_terminates_the_connection() {
        const TEST_LINE_LIMIT: usize = 8 * 1024;
        let (tx, rx) = mpsc::sync_channel(1);
        let transport_failure = Arc::new(TransportFailure::default());
        spawn_pipe_reader(
            Cursor::new(vec![b'x'; TEST_LINE_LIMIT + 1]),
            tx,
            false,
            TEST_LINE_LIMIT,
            transport_failure.clone(),
        );
        let failure = wait_for_transport_failure(&transport_failure);
        assert!(failure.contains("line exceeded"));
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(1)),
            Ok(AppServerLine::ReaderFinished {
                source: AppServerReaderSource::Stdout,
                termination: AppServerReaderTermination::Failed,
            })
        ));

        let mut harness = TestConnection::new(true);
        harness.connection.transport_failure.record(failure);
        let error = harness
            .connection
            .wait_for_response(1)
            .expect_err("a poisoned reader must fail the connection");
        assert!(error.to_string().contains("app-server transport failed"));
        assert!(
            error
                .to_string()
                .contains(&format!("{TEST_LINE_LIMIT}-byte limit"))
        );
        assert!(
            harness
                .connection
                .child
                .try_wait()
                .expect("terminated child status should be readable")
                .is_some()
        );
    }

    #[test]
    fn pending_notification_overflow_fails_closed_and_terminates_the_connection() {
        let mut harness = TestConnection::new(true);
        let error = (0..=MAX_PENDING_NOTIFICATIONS)
            .find_map(|index| {
                harness
                    .connection
                    .defer_turn_notification(notification(json!({
                        "method": "item/agentMessage/delta",
                        "params": { "turnId": "turn-1", "delta": index.to_string() }
                    })))
                    .err()
            })
            .expect("the bounded pending queue should reject overflow");

        assert!(
            error
                .to_string()
                .contains("pending turn-notification queue")
        );
        assert!(
            harness
                .connection
                .child
                .try_wait()
                .expect("terminated child status should be readable")
                .is_some()
        );
    }

    fn wait_for_transport_failure(transport_failure: &TransportFailure) -> String {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(failure) = transport_failure.current() {
                return failure;
            }
            assert!(
                Instant::now() < deadline,
                "pipe reader did not report its bounded transport failure"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn assert_not_initialized<T>(result: Result<T>)
    where
        T: Debug,
    {
        let error = result.expect_err("typed request should require initialize first");
        assert!(
            error
                .to_string()
                .contains("app-server connection is not initialized")
        );
    }

    fn assert_contains_warning(warnings: &[String], expected_fragment: &str) {
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains(expected_fragment)),
            "expected warning containing `{expected_fragment}`, got {warnings:?}"
        );
    }

    fn notification(value: Value) -> AppServerNotification {
        AppServerNotification::from_value(value).expect("test value should be a notification")
    }

    fn thread_record_json(id: &str) -> Value {
        json!({
            "id": id,
            "name": "Thread title",
            "preview": "Thread preview",
            "cwd": "/repo",
            "source": "vscode",
            "modelProvider": "openai",
            "updatedAt": 1,
            "path": null,
            "status": {
                "type": "idle"
            },
            "gitInfo": null,
            "turns": []
        })
    }

    struct TestConnection {
        connection: AppServerConnection,
        tx: SyncSender<AppServerLine>,
        log_path: PathBuf,
    }

    impl TestConnection {
        fn new(initialized: bool) -> Self {
            Self::new_with_interrupt_signal(
                initialized,
                AppServerApprovalMode::Interactive,
                AppServerTurnInterruptSignal::default(),
            )
        }

        fn new_with_interrupt_signal(
            initialized: bool,
            approval_mode: AppServerApprovalMode,
            interrupt_signal: AppServerTurnInterruptSignal,
        ) -> Self {
            let log_path = unique_log_path();
            let mut command = Command::new("sh");
            command
                .arg("-c")
                .arg("while IFS= read -r line; do printf '%s\\n' \"$line\" >> \"$1\"; done")
                .arg("fake-app-server-stdin-log")
                .arg(&log_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let mut child =
                subprocess::spawn(&mut command).expect("fake app-server child should spawn");
            let stdin = child
                .take_stdin()
                .expect("fake app-server stdin should be piped");
            // Unit tests enqueue scripted peer messages before entering the consumer
            // loop. Production pipe readers use the one-line backpressure channel.
            let (tx, rx) = mpsc::sync_channel(128);
            let transport_failure = Arc::new(TransportFailure::default());
            let stdin_writer = AppServerStdinWriter::spawn(stdin, transport_failure.clone());

            Self {
                connection: AppServerConnection {
                    child,
                    stdin_writer,
                    rx,
                    transport_failure,
                    diagnostics: ConnectionDiagnostics::default(),
                    pending_notifications: PendingNotifications::default(),
                    next_request_id: 1,
                    client_name: "test-client".to_string(),
                    client_version: "test-version".to_string(),
                    initialized,
                    config: test_config(),
                    terminal_recovery_write_deadline: None,
                    approval_broker: Arc::new(AppServerApprovalBroker::default()),
                    approval_mode,
                    interrupt_signal,
                },
                tx,
                log_path,
            }
        }

        fn send_stdout(&self, value: Value) {
            self.tx
                .send(AppServerLine::Stdout(value.to_string()))
                .expect("test channel should accept stdout JSON");
        }

        fn send_stderr(&self, line: &str) {
            self.tx
                .send(AppServerLine::Stderr(line.to_string()))
                .expect("test channel should accept stderr line");
        }

        fn send_stdout_eof(&self) {
            self.tx
                .send(AppServerLine::ReaderFinished {
                    source: AppServerReaderSource::Stdout,
                    termination: AppServerReaderTermination::EndOfFile,
                })
                .expect("test channel should accept the stdout EOF marker");
        }

        fn logged_json_lines(&self, expected_count: usize) -> Vec<Value> {
            logged_json_lines(&self.log_path, expected_count)
        }
    }

    fn test_config() -> AppServerConnectionConfig {
        AppServerConnectionConfig {
            executable: PathBuf::from("codex"),
            executable_prefix_args: Vec::new(),
            executable_resolution_error: None,
            command_environment: Vec::new(),
            shell_environment_inherit: ShellEnvironmentInherit::Core,
            process_environment_policy: ProcessEnvironmentPolicy::Scrubbed,
            api_key_auth: false,
            response_timeout: Duration::from_millis(10),
            poll_interval: Duration::from_millis(1),
            drain_timeout: Duration::from_millis(2),
            drain_poll_interval: Duration::from_millis(1),
            approval_timeout: Duration::from_millis(10),
            interrupt_total_timeout: Duration::from_millis(25),
            interrupt_retry_backoff: Duration::from_millis(1),
            interrupt_retry_limit: 3,
            terminal_grace_timeout: Duration::from_millis(20),
            terminal_delivery_timeout: Duration::from_millis(20),
            terminal_delivery_retry_interval: Duration::from_millis(1),
        }
    }

    fn unique_log_path() -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "codex-exec-loop-app-server-connection-{}-{now}.jsonl",
            std::process::id()
        ))
    }

    #[cfg(unix)]
    fn wait_for_published_pid(path: &Path, label: &str) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(body) = fs::read_to_string(path)
                && let Ok(pid) = body.trim().parse::<u32>()
                && pid > 0
            {
                return pid;
            }
            assert!(
                Instant::now() < deadline,
                "{label} PID should be published as a non-zero number at {}",
                path.display()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn logged_json_lines(path: &Path, expected_count: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let body = fs::read_to_string(path).unwrap_or_default();
            let lines = body
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    serde_json::from_str::<Value>(line)
                        .unwrap_or_else(|error| panic!("logged request should be JSON: {error}"))
                })
                .collect::<Vec<_>>();

            if lines.len() >= expected_count {
                return lines;
            }
            assert!(
                Instant::now() < deadline,
                "expected at least {expected_count} logged JSON lines in {}, got {}",
                path.display(),
                lines.len()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}

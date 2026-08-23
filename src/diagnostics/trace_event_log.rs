use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{Map, Value};
use tracing::{Event, Level, Subscriber, field};
use tracing_appender::non_blocking::{ErrorCounter, NonBlocking, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::{FmtSpan, Writer};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, FormattedFields};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

#[cfg(windows)]
use crate::private_fs::{
    WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT, WINDOWS_FILE_FLAG_BACKUP_SEMANTICS,
    WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
    WINDOWS_GENERIC_WRITE, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC, set_windows_private_acl,
    validate_windows_path_identity, validate_windows_private_owner_and_acl,
};

pub(crate) const AKRA_EVENT_TARGET: &str = "codex_exec_loop_native::diagnostics::akra_event";
const TRACE_MAX_FILES_ENV_VAR: &str = "AKRA_TRACE_MAX_FILES";
const DEFAULT_TRACE_MAX_FILES: usize = 7;
const MAX_TRACE_MAX_FILES: usize = 365;
const TRACE_MAX_FILE_BYTES_ENV_VAR: &str = "AKRA_TRACE_MAX_FILE_BYTES";
const TRACE_MAX_TOTAL_BYTES_ENV_VAR: &str = "AKRA_TRACE_MAX_TOTAL_BYTES";
const DEFAULT_TRACE_MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_TRACE_MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MIN_TRACE_MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_TRACE_MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_TRACE_MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ROLLING_TRACE_SEQUENCE: u32 = 9_999;
static TRACE_DROPPED_LINES: OnceLock<ErrorCounter> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
struct TraceConfig {
    filter: String,
    destination: TraceDestination,
    span_mode: TraceSpanMode,
    tokio_console: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TraceDestination {
    DailyRolling {
        directory: PathBuf,
        file_name: String,
    },
    ExactFile(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TraceSpanMode {
    None,
    Close,
    Full,
}

impl TraceSpanMode {
    fn fmt_span(self) -> FmtSpan {
        match self {
            Self::None => FmtSpan::NONE,
            Self::Close => FmtSpan::CLOSE,
            Self::Full => FmtSpan::FULL,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TraceSettings {
    filter: String,
    span_mode: TraceSpanMode,
}

#[derive(Debug, Default)]
struct TraceEnvironment {
    akra_trace: Option<String>,
    rust_log: Option<String>,
    span_mode: Option<String>,
    trace_file: Option<PathBuf>,
    tokio_console: Option<String>,
    default_log_directory: Option<PathBuf>,
}

impl TraceEnvironment {
    fn from_process() -> Self {
        let configuration = crate::configuration::current_process_config();
        Self {
            akra_trace: configuration
                .map(|config| config.config.diagnostics.trace.clone())
                .or_else(|| std::env::var("AKRA_TRACE").ok()),
            rust_log: std::env::var("RUST_LOG").ok(),
            span_mode: configuration
                .map(|config| config.config.diagnostics.spans.clone())
                .or_else(|| std::env::var("AKRA_TRACE_SPANS").ok()),
            trace_file: std::env::var_os("AKRA_TRACE_FILE").map(PathBuf::from),
            tokio_console: configuration
                .map(|config| config.config.diagnostics.tokio_console.to_string())
                .or_else(|| std::env::var("AKRA_TOKIO_CONSOLE").ok()),
            default_log_directory: default_trace_log_directory(),
        }
    }
}

pub(super) fn init_from_env() -> Option<WorkerGuard> {
    let config = trace_config_from_env()?;
    match build_trace_guard(config) {
        Ok(guard) => Some(guard),
        Err(error) => {
            eprintln!("akra trace initialization failed: {error}");
            None
        }
    }
}

pub(super) fn akra_event_enabled() -> bool {
    tracing::enabled!(target: AKRA_EVENT_TARGET, Level::DEBUG)
}

pub(super) fn emit_akra_event(event: &str, detail: &Value) {
    let detail_json = sanitize_trace_value(None, detail).to_string();
    tracing::debug!(
        target: AKRA_EVENT_TARGET,
        pid = std::process::id(),
        event = event,
        detail = detail_json.as_str(),
        "akra_event"
    );
}

fn trace_config_from_env() -> Option<TraceConfig> {
    trace_config_from_environment(&TraceEnvironment::from_process())
}

fn trace_config_from_environment(environment: &TraceEnvironment) -> Option<TraceConfig> {
    let mut settings = trace_settings_from_environment(environment)?;
    if let Some(span_mode) = environment
        .span_mode
        .as_deref()
        .and_then(trace_span_mode_from_value)
    {
        settings.span_mode = span_mode;
    }
    Some(TraceConfig {
        filter: settings.filter,
        span_mode: settings.span_mode,
        destination: trace_destination_from_environment(environment)?,
        tokio_console: environment
            .tokio_console
            .as_deref()
            .is_some_and(tokio_console_value_is_enabled),
    })
}

fn build_trace_guard(config: TraceConfig) -> Result<WorkerGuard, String> {
    let (writer, guard) = non_blocking_writer(config.destination)?;
    let _ = TRACE_DROPPED_LINES.set(writer.error_counter());
    let env_filter = env_filter_from_filter_value(&config.filter);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_span_events(config.span_mode.fmt_span())
        .event_format(AkraJsonFormat)
        .with_writer(writer);

    install_tracing_subscriber(env_filter, fmt_layer, config.tokio_console)?;

    Ok(guard)
}

pub(super) fn dropped_lines() -> usize {
    TRACE_DROPPED_LINES
        .get()
        .map(ErrorCounter::dropped_lines)
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy)]
struct AkraJsonFormat;

impl<S, N> FormatEvent<S, N> for AkraJsonFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        let mut visitor = AkraJsonVisitor::default();
        visitor.insert_value(
            "timestamp",
            Value::String(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)),
        );
        visitor.insert_value("level", Value::String(meta.level().to_string()));
        event.record(&mut visitor);
        visitor.insert_value("target", Value::String(meta.target().to_string()));
        if let Some(span) = ctx.parent_span() {
            visitor.insert_value("span", span_to_json::<S, N>(&span));
        }
        if let Some(scope) = ctx.event_scope() {
            let spans = scope
                .from_root()
                .map(|span| span_to_json::<S, N>(&span))
                .collect::<Vec<_>>();
            if !spans.is_empty() {
                visitor.insert_value("spans", Value::Array(spans));
            }
        }

        let line = Value::Object(visitor.into_map()).to_string();
        writer.write_str(&line)?;
        writer.write_char('\n')
    }
}

#[derive(Debug, Default)]
struct AkraJsonVisitor {
    values: Map<String, Value>,
}

impl AkraJsonVisitor {
    fn insert_value(&mut self, field_name: &str, value: Value) {
        let field_name = field_name.strip_prefix("r#").unwrap_or(field_name);
        if field_name == "detail" && merge_detail_field(&mut self.values, &value) {
            return;
        }
        self.values.insert(
            field_name.to_string(),
            sanitize_trace_value(Some(field_name), &value),
        );
    }

    fn into_map(self) -> Map<String, Value> {
        self.values
    }
}

fn sanitize_trace_value(field_name: Option<&str>, value: &Value) -> Value {
    if let Some(field_name) = field_name {
        let explicitly_sensitive = trace_field_contains_sensitive_body(field_name, value);
        let reviewable = match value {
            Value::Null | Value::Bool(_) | Value::Number(_) => true,
            Value::String(value) => trace_string_field_is_safe(field_name, value),
            Value::Array(_) | Value::Object(_) => trace_container_field_is_safe(field_name),
        };
        if explicitly_sensitive || !reviewable {
            return redacted_trace_value(value);
        }
    }

    match value {
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| sanitize_trace_value(None, value))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), sanitize_trace_value(Some(key), value)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn redacted_trace_value(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::String(value) => serde_json::json!({
            "redacted": true,
            "chars": value.chars().count(),
        }),
        Value::Array(values) => serde_json::json!({
            "redacted": true,
            "items": values.len(),
        }),
        Value::Object(values) => serde_json::json!({
            "redacted": true,
            "fields": values.len(),
        }),
        Value::Bool(_) | Value::Number(_) => serde_json::json!({
            "redacted": true,
        }),
    }
}

fn trace_string_field_is_safe(field_name: &str, value: &str) -> bool {
    let normalized = field_name.trim().to_ascii_lowercase();
    let field_is_allowed = matches!(
        normalized.as_str(),
        "decision"
            | "environment_variable"
            | "event"
            | "level"
            | "mode"
            | "model"
            | "name"
            | "operation"
            | "origin"
            | "phase"
            | "reasoning_effort"
            | "server_request_method"
            | "service_name"
            | "session_kind"
            | "source"
            | "state"
            | "status"
            | "target"
            | "timestamp"
    ) || normalized.ends_with("_id")
        || normalized.ends_with("_kind")
        || normalized.ends_with("_mode")
        || normalized.ends_with("_phase")
        || normalized.ends_with("_state")
        || normalized.ends_with("_status");
    if !field_is_allowed {
        return false;
    }
    if normalized == "timestamp" {
        return value.len() <= 64 && chrono::DateTime::parse_from_rfc3339(value).is_ok();
    }
    if normalized == "level" {
        return matches!(value, "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR");
    }
    value.len() <= 256
        && value.is_ascii()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'+' | b'-')
        })
}

fn trace_container_field_is_safe(field_name: &str) -> bool {
    matches!(
        field_name.trim().to_ascii_lowercase().as_str(),
        "span" | "spans"
    )
}

fn trace_field_contains_sensitive_body(field_name: &str, value: &Value) -> bool {
    let normalized = field_name.trim().to_ascii_lowercase();
    if normalized.ends_with("_chars")
        || normalized.ends_with("_count")
        || normalized.ends_with("_present")
    {
        return !matches!(value, Value::Null | Value::Bool(_) | Value::Number(_));
    }
    let compact = normalized
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>();
    if [
        "apikey",
        "authorization",
        "authtoken",
        "authheader",
        "bearer",
        "cookie",
        "credential",
        "password",
        "passwd",
        "privatekey",
        "secret",
        "sessionkey",
        "accesskey",
        "signingkey",
        "token",
    ]
    .iter()
    .any(|marker| compact.contains(marker))
    {
        return true;
    }
    matches!(
        normalized.as_str(),
        "content"
            | "detail"
            | "developer_instructions"
            | "direction_title"
            | "error"
            | "error_message"
            | "failure_summary"
            | "latest_main_reply"
            | "message"
            | "notice"
            | "pause_reason"
            | "prompt"
            | "raw_prompt"
            | "reason"
            | "repair_failure_summary"
            | "reply"
            | "response"
            | "summary"
            | "task_title"
            | "text"
            | "title"
            | "worker_summary"
            | "warning"
    ) || [
        "_content",
        "_detail",
        "_error",
        "_message",
        "_notice",
        "_prompt",
        "_rationale",
        "_reason",
        "_reply",
        "_response",
        "_summary",
        "_text",
        "_title",
    ]
    .iter()
    .any(|suffix| normalized.ends_with(suffix))
}

impl field::Visit for AkraJsonVisitor {
    fn record_f64(&mut self, field: &field::Field, value: f64) {
        self.insert_value(field.name(), Value::from(value));
    }

    fn record_i64(&mut self, field: &field::Field, value: i64) {
        self.insert_value(field.name(), Value::from(value));
    }

    fn record_u64(&mut self, field: &field::Field, value: u64) {
        self.insert_value(field.name(), Value::from(value));
    }

    fn record_bool(&mut self, field: &field::Field, value: bool) {
        self.insert_value(field.name(), Value::from(value));
    }

    fn record_str(&mut self, field: &field::Field, value: &str) {
        self.insert_value(field.name(), Value::from(value));
    }

    fn record_bytes(&mut self, field: &field::Field, value: &[u8]) {
        self.insert_value(field.name(), Value::from(value));
    }

    fn record_debug(&mut self, field: &field::Field, value: &dyn fmt::Debug) {
        self.insert_value(field.name(), Value::String(format!("{value:?}")));
    }
}

fn merge_detail_field(values: &mut Map<String, Value>, value: &Value) -> bool {
    let Value::String(detail) = value else {
        return false;
    };
    let Ok(detail) = serde_json::from_str::<Value>(detail) else {
        return false;
    };
    let Value::Object(detail) = sanitize_trace_value(None, &detail) else {
        return false;
    };
    values.extend(detail);
    true
}

fn span_to_json<S, N>(span: &tracing_subscriber::registry::SpanRef<'_, S>) -> Value
where
    S: for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    let fields = span
        .extensions()
        .get::<FormattedFields<N>>()
        .and_then(|fields| serde_json::from_str::<Value>(fields).ok())
        .and_then(|fields| match fields {
            Value::Object(fields) => Some(fields),
            _ => None,
        })
        .unwrap_or_default();
    let mut fields = match sanitize_trace_value(None, &Value::Object(fields)) {
        Value::Object(fields) => fields,
        _ => Map::new(),
    };
    fields.insert(
        "name".to_string(),
        Value::String(span.metadata().name().to_string()),
    );
    Value::Object(fields)
}

fn non_blocking_writer(
    destination: TraceDestination,
) -> Result<(NonBlocking, WorkerGuard), String> {
    match destination {
        TraceDestination::DailyRolling {
            directory,
            file_name,
        } => {
            let appender =
                PrivateRollingFileAppender::new(directory, file_name, trace_retained_file_count())?;
            Ok(non_blocking_with_thread_name(appender, "akra-trace-log"))
        }
        TraceDestination::ExactFile(path) => {
            let (path, parent, parent_identity) = prepare_exact_trace_file_path(&path)?;
            parent_identity.validate(&parent)?;
            let file_name = path
                .file_name()
                .ok_or_else(|| "exact trace log path omitted its file name".to_string())?;
            let file = open_private_trace_file_in_directory(&parent_identity, file_name, &path)?;
            parent_identity.validate(&parent)?;
            let appender = BoundedExactTraceWriter::new(
                file,
                trace_max_file_bytes().min(trace_max_total_bytes()),
            )
            .map_err(|error| {
                format!(
                    "failed to inspect exact trace log file `{}`: {error}",
                    path.display()
                )
            })?;
            Ok(non_blocking_with_thread_name(appender, "akra-trace-log"))
        }
    }
}

#[cfg(not(unix))]
fn open_private_trace_file(path: &Path) -> Result<std::fs::File, String> {
    let mut create_options = std::fs::OpenOptions::new();
    create_options.append(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        create_options
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        create_options
            .read(true)
            .access_mode(
                WINDOWS_GENERIC_READ
                    | WINDOWS_GENERIC_WRITE
                    | WINDOWS_READ_CONTROL
                    | WINDOWS_WRITE_DAC,
            )
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = match create_options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            validate_existing_trace_path(path)?;
            let mut existing_options = std::fs::OpenOptions::new();
            existing_options.append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;

                existing_options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;

                existing_options
                    .read(true)
                    .access_mode(
                        WINDOWS_GENERIC_READ
                            | WINDOWS_GENERIC_WRITE
                            | WINDOWS_READ_CONTROL
                            | WINDOWS_WRITE_DAC,
                    )
                    .share_mode(WINDOWS_FILE_SHARE_ALL)
                    .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT);
            }
            existing_options.open(path).map_err(|error| {
                format!(
                    "failed to open existing trace log file `{}`: {error}",
                    path.display()
                )
            })?
        }
        Err(error) => {
            return Err(format!(
                "failed to create trace log file `{}`: {error}",
                path.display()
            ));
        }
    };
    validate_opened_trace_file(&file, path)?;
    #[cfg(windows)]
    {
        validate_windows_path_identity(path, &file, false).map_err(|error| error.to_string())?;
        set_windows_private_acl(&file, false).map_err(|error| error.to_string())?;
        validate_windows_private_owner_and_acl(path, &file).map_err(|error| error.to_string())?;
        validate_windows_path_identity(path, &file, false).map_err(|error| error.to_string())?;
    }
    set_private_open_file_permissions(&file, path)?;
    Ok(file)
}

fn open_private_trace_file_in_directory(
    directory_identity: &TraceDirectoryIdentity,
    file_name: &std::ffi::OsStr,
    display_path: &Path,
) -> Result<std::fs::File, String> {
    #[cfg(unix)]
    {
        open_private_trace_file_at(&directory_identity.directory, file_name, display_path)
    }
    #[cfg(not(unix))]
    {
        let _ = (directory_identity, file_name);
        open_private_trace_file(display_path)
    }
}

#[cfg(unix)]
fn open_private_trace_file_at(
    directory: &std::fs::File,
    file_name: &std::ffi::OsStr,
    display_path: &Path,
) -> Result<std::fs::File, String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let file_name = CString::new(file_name.as_bytes()).map_err(|_| {
        format!(
            "trace log file name contains a NUL byte `{}`",
            display_path.display()
        )
    })?;
    let create_flags = libc::O_WRONLY
        | libc::O_APPEND
        | libc::O_CREAT
        | libc::O_EXCL
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW;
    // SAFETY: directory is a live directory descriptor, file_name is NUL-terminated,
    // and openat does not retain the pointer after returning.
    let mut descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            file_name.as_ptr(),
            create_flags,
            0o600,
        )
    };
    if descriptor < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EEXIST) {
            return Err(format!(
                "failed to create trace log file `{}`: {error}",
                display_path.display()
            ));
        }
        let existing_flags = libc::O_WRONLY | libc::O_APPEND | libc::O_CLOEXEC | libc::O_NOFOLLOW;
        // SAFETY: same descriptor and CString guarantees as the create attempt.
        descriptor =
            unsafe { libc::openat(directory.as_raw_fd(), file_name.as_ptr(), existing_flags) };
        if descriptor < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ELOOP) {
                return Err(format!(
                    "refusing symbolic-link trace log file `{}`",
                    display_path.display()
                ));
            }
            return Err(format!(
                "failed to open existing trace log file `{}`: {error}",
                display_path.display()
            ));
        }
    }

    // SAFETY: descriptor was returned by openat and ownership is transferred exactly once.
    let file = unsafe { std::fs::File::from_raw_fd(descriptor) };
    validate_opened_trace_file_at(directory, &file_name, &file, display_path)?;
    set_private_open_file_permissions(&file, display_path)?;
    Ok(file)
}

#[cfg(unix)]
fn validate_opened_trace_file_at(
    directory: &std::fs::File,
    file_name: &std::ffi::CStr,
    file: &std::fs::File,
    display_path: &Path,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;

    let opened_metadata = file.metadata().map_err(|error| {
        format!(
            "failed to inspect opened trace log file `{}`: {error}",
            display_path.display()
        )
    })?;
    if !opened_metadata.is_file() || opened_metadata.nlink() != 1 {
        return Err(format!(
            "refusing non-regular or multiply-linked trace log file `{}`",
            display_path.display()
        ));
    }
    // SAFETY: status points to valid writable storage, and fstatat only borrows
    // the directory descriptor and NUL-terminated file name for this call.
    let mut status = unsafe { std::mem::zeroed::<libc::stat>() };
    let result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            file_name.as_ptr(),
            &mut status,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        return Err(format!(
            "failed to validate anchored trace log path `{}`: {}",
            display_path.display(),
            io::Error::last_os_error()
        ));
    }
    if opened_metadata.dev() != status.st_dev as u64
        || opened_metadata.ino() != status.st_ino as u64
    {
        return Err(format!(
            "trace log path identity changed while opening `{}`",
            display_path.display()
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_existing_trace_path(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect existing trace log file `{}`: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing symbolic-link trace log file `{}`",
            path.display()
        ));
    }
    if !metadata.is_file() {
        return Err(format!(
            "refusing non-regular trace log file `{}`",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_opened_trace_file(file: &std::fs::File, path: &Path) -> Result<(), String> {
    let opened_metadata = file.metadata().map_err(|error| {
        format!(
            "failed to inspect opened trace log file `{}`: {error}",
            path.display()
        )
    })?;
    if !opened_metadata.is_file() {
        return Err(format!(
            "refusing opened non-regular trace log file `{}`",
            path.display()
        ));
    }
    let path_metadata = std::fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to re-inspect trace log file `{}`: {error}",
            path.display()
        )
    })?;
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
        return Err(format!(
            "trace log path changed to a symbolic link or non-regular file `{}`",
            path.display()
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if opened_metadata.dev() != path_metadata.dev()
            || opened_metadata.ino() != path_metadata.ino()
        {
            return Err(format!(
                "trace log path identity changed while opening `{}`",
                path.display()
            ));
        }
        if opened_metadata.nlink() != 1 {
            return Err(format!(
                "refusing multiply-linked trace log file `{}`",
                path.display()
            ));
        }
    }
    Ok(())
}

fn set_private_open_file_permissions(file: &std::fs::File, path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                format!(
                    "failed to secure opened trace log file `{}`: {error}",
                    path.display()
                )
            })?;
    }
    #[cfg(not(unix))]
    let _ = (file, path);
    Ok(())
}

#[derive(Debug)]
struct PrivateRollingFileAppender {
    file: std::fs::File,
    directory: PathBuf,
    directory_identity: TraceDirectoryIdentity,
    file_name_prefix: String,
    current_day: chrono::NaiveDate,
    current_sequence: u32,
    current_file_bytes: u64,
    max_log_files: usize,
    max_file_bytes: u64,
    max_total_bytes: u64,
}

impl PrivateRollingFileAppender {
    fn new(
        directory: PathBuf,
        file_name_prefix: String,
        max_log_files: usize,
    ) -> Result<Self, String> {
        Self::new_with_limits(
            directory,
            file_name_prefix,
            max_log_files,
            trace_max_file_bytes(),
            trace_max_total_bytes(),
        )
    }

    fn new_with_limits(
        directory: PathBuf,
        file_name_prefix: String,
        max_log_files: usize,
        max_file_bytes: u64,
        max_total_bytes: u64,
    ) -> Result<Self, String> {
        validate_trace_file_name_prefix(&file_name_prefix)?;
        let max_total_bytes = max_total_bytes.max(1);
        let max_file_bytes = max_file_bytes.max(1).min(max_total_bytes);
        let (directory, directory_identity) = prepare_private_trace_directory(&directory)?;
        let current_day = chrono::Utc::now().date_naive();
        let (current_sequence, current_file_bytes) = next_rolling_trace_sequence(
            &directory,
            &file_name_prefix,
            current_day,
            max_file_bytes,
        )?;
        let file = open_private_rolling_trace_file(
            &directory,
            &directory_identity,
            &file_name_prefix,
            current_day,
            current_sequence,
        )?;
        let mut appender = Self {
            file,
            directory,
            directory_identity,
            file_name_prefix,
            current_day,
            current_sequence,
            current_file_bytes,
            max_log_files: max_log_files.max(1),
            max_file_bytes,
            max_total_bytes,
        };
        appender.prune_old_files()?;
        Ok(appender)
    }

    fn roll_to(&mut self, day: chrono::NaiveDate, sequence: u32) -> Result<(), String> {
        self.directory_identity.validate(&self.directory)?;
        let mut sequence = sequence;
        let (file, current_file_bytes) = loop {
            let file = open_private_rolling_trace_file(
                &self.directory,
                &self.directory_identity,
                &self.file_name_prefix,
                day,
                sequence,
            )?;
            let current_file_bytes = file
                .metadata()
                .map_err(|error| {
                    format!(
                        "failed to inspect opened rolling trace file for {day}.{sequence:04}: {error}"
                    )
                })?
                .len();
            if current_file_bytes < self.max_file_bytes {
                break (file, current_file_bytes);
            }
            sequence = next_rolling_trace_sequence_number(sequence)?;
        };
        self.file = file;
        self.current_day = day;
        self.current_sequence = sequence;
        self.current_file_bytes = current_file_bytes;
        self.prune_old_files()
    }

    fn prune_old_files(&mut self) -> Result<(), String> {
        self.directory_identity.validate(&self.directory)?;
        let mut matching_paths = Vec::new();
        for entry in std::fs::read_dir(&self.directory).map_err(|error| {
            format!(
                "failed to inspect rolling trace directory `{}`: {error}",
                self.directory.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to inspect rolling trace entry in `{}`: {error}",
                    self.directory.display()
                )
            })?;
            if entry
                .file_type()
                .map_err(|error| {
                    format!(
                        "failed to inspect rolling trace entry type `{}`: {error}",
                        entry.path().display()
                    )
                })?
                .is_file()
                && let Some((day, sequence)) = rolling_trace_file_identity(
                    entry.file_name().to_string_lossy().as_ref(),
                    &self.file_name_prefix,
                )
            {
                let bytes = entry
                    .metadata()
                    .map_err(|error| {
                        format!(
                            "failed to inspect rolling trace entry metadata `{}`: {error}",
                            entry.path().display()
                        )
                    })?
                    .len();
                matching_paths.push((day, sequence, bytes, entry.path()));
            }
        }
        let mut expired_candidates = matching_paths
            .into_iter()
            .filter(|(day, sequence, _, _)| {
                (*day, *sequence) != (self.current_day, self.current_sequence)
            })
            .collect::<Vec<_>>();
        expired_candidates.sort();
        // The open current-day file is never a retention candidate, even when
        // future-dated files sort after it. Reserve one slot for that live file.
        let retained_non_current = self.max_log_files.saturating_sub(1);
        let remove_count = expired_candidates
            .len()
            .saturating_sub(retained_non_current);
        let retained_byte_budget = self.max_total_bytes.saturating_sub(self.max_file_bytes);
        let mut retained_bytes = expired_candidates
            .iter()
            .map(|(_, _, bytes, _)| *bytes)
            .sum::<u64>();
        for (index, (_, _, bytes, path)) in expired_candidates.into_iter().enumerate() {
            if index >= remove_count
                && bytes <= self.max_file_bytes
                && retained_bytes <= retained_byte_budget
            {
                continue;
            }
            std::fs::remove_file(&path).map_err(|error| {
                format!(
                    "failed to remove expired trace log file `{}`: {error}",
                    path.display()
                )
            })?;
            retained_bytes = retained_bytes.saturating_sub(bytes);
        }
        self.directory_identity.validate(&self.directory)?;
        Ok(())
    }
}

#[derive(Debug)]
struct BoundedExactTraceWriter {
    file: std::fs::File,
    current_bytes: u64,
    max_bytes: u64,
}

impl BoundedExactTraceWriter {
    fn new(file: std::fs::File, max_bytes: u64) -> io::Result<Self> {
        let current_bytes = file.metadata()?.len();
        let max_bytes = max_bytes.max(1);
        if current_bytes > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "existing exact trace log exceeds the configured byte limit",
            ));
        }
        Ok(Self {
            file,
            current_bytes,
            max_bytes,
        })
    }
}

impl Write for BoundedExactTraceWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let buffer_bytes = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if self.current_bytes.saturating_add(buffer_bytes) > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "exact trace log reached the configured byte limit",
            ));
        }
        let written = self.file.write(buffer)?;
        self.current_bytes = self
            .current_bytes
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
fn rolling_trace_file_day(file_name: &str, prefix: &str) -> Option<chrono::NaiveDate> {
    rolling_trace_file_identity(file_name, prefix).map(|(day, _)| day)
}

fn rolling_trace_file_identity(file_name: &str, prefix: &str) -> Option<(chrono::NaiveDate, u32)> {
    let suffix = file_name.strip_prefix(prefix)?.strip_prefix('.')?;
    let (day, sequence) = match suffix.split_once('.') {
        Some((day, sequence)) => {
            if sequence.len() != 4 || !sequence.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            (day, sequence.parse::<u32>().ok()?)
        }
        None => (suffix, 0),
    };
    let bytes = day.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return None;
    }
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .ok()
        .map(|day| (day, sequence))
}

#[derive(Debug)]
struct TraceDirectoryIdentity {
    #[cfg(any(unix, windows))]
    directory: std::fs::File,
    #[cfg(windows)]
    require_current_owner: bool,
    #[cfg(unix)]
    require_current_owner: bool,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl TraceDirectoryIdentity {
    fn from_opened(
        directory: &std::fs::File,
        metadata: &std::fs::Metadata,
        require_current_owner: bool,
    ) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            let _ = directory;
            Ok(Self {
                directory: directory.try_clone().map_err(|error| {
                    format!("failed to retain Unix trace directory handle: {error}")
                })?,
                require_current_owner,
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        }
        #[cfg(windows)]
        {
            let _ = metadata;
            Ok(Self {
                directory: directory.try_clone().map_err(|error| {
                    format!("failed to retain private Windows trace directory handle: {error}")
                })?,
                require_current_owner,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (directory, metadata, require_current_owner);
            Ok(Self {})
        }
    }

    fn validate(&self, path: &Path) -> Result<(), String> {
        let metadata = std::fs::symlink_metadata(path).map_err(|error| {
            format!(
                "failed to validate private trace directory `{}`: {error}",
                path.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "private trace directory became a symbolic link or non-directory `{}`",
                path.display()
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            if metadata.dev() != self.device || metadata.ino() != self.inode {
                return Err(format!(
                    "private trace directory identity changed `{}`",
                    path.display()
                ));
            }
            if self.require_current_owner && metadata.uid() != effective_user_id() {
                return Err(format!(
                    "private trace directory is not owned by the current user `{}`",
                    path.display()
                ));
            }
        }
        #[cfg(windows)]
        {
            validate_windows_path_identity(path, &self.directory, true)
                .map_err(|error| error.to_string())?;
            if self.require_current_owner {
                validate_windows_private_owner_and_acl(path, &self.directory)
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }
}

fn prepare_private_trace_directory(
    path: &Path,
) -> Result<(PathBuf, TraceDirectoryIdentity), String> {
    prepare_trace_directory(path, true)
}

fn prepare_exact_trace_file_path(
    path: &Path,
) -> Result<(PathBuf, PathBuf, TraceDirectoryIdentity), String> {
    let file_name = path.file_name().ok_or_else(|| {
        format!(
            "exact trace log path must end in a file name `{}`",
            path.display()
        )
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let (parent, identity) = prepare_trace_directory(parent, false)?;
    Ok((parent.join(file_name), parent, identity))
}

fn prepare_trace_directory(
    path: &Path,
    private: bool,
) -> Result<(PathBuf, TraceDirectoryIdentity), String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("failed to resolve current directory: {error}"))?
            .join(path)
    };
    create_and_validate_private_directory_components(&absolute)?;
    #[cfg(windows)]
    let directory = {
        use std::os::windows::fs::OpenOptionsExt;

        let mut options = std::fs::OpenOptions::new();
        options
            .read(true)
            .access_mode(
                WINDOWS_GENERIC_READ
                    | WINDOWS_READ_CONTROL
                    | if private { WINDOWS_WRITE_DAC } else { 0 },
            )
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(
                WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS,
            );
        options.open(&absolute)
    }
    .map_err(|error| {
        format!(
            "failed to open private trace directory `{}`: {error}",
            absolute.display()
        )
    })?;
    #[cfg(not(windows))]
    let directory = std::fs::File::open(&absolute).map_err(|error| {
        format!(
            "failed to open private trace directory `{}`: {error}",
            absolute.display()
        )
    })?;
    let opened_metadata = directory.metadata().map_err(|error| {
        format!(
            "failed to inspect opened private trace directory `{}`: {error}",
            absolute.display()
        )
    })?;
    validate_trace_directory_component_ownership(&absolute, &opened_metadata)?;
    #[cfg(windows)]
    {
        validate_windows_path_identity(&absolute, &directory, true)
            .map_err(|error| error.to_string())?;
        if private {
            set_windows_private_acl(&directory, true).map_err(|error| error.to_string())?;
        }
    }
    let identity = TraceDirectoryIdentity::from_opened(&directory, &opened_metadata, private)?;
    identity.validate(&absolute)?;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;

        directory
            .set_permissions(std::fs::Permissions::from_mode(0o700))
            .map_err(|error| {
                format!(
                    "failed to secure private trace directory `{}`: {error}",
                    absolute.display()
                )
            })?;
    }
    identity.validate(&absolute)?;
    Ok((absolute, identity))
}

fn create_and_validate_private_directory_components(path: &Path) -> Result<(), String> {
    use std::path::Component;

    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!(
                    "private trace directory cannot contain parent traversal `{}`",
                    path.display()
                ));
            }
            Component::Normal(component) => {
                current.push(component);
                let metadata = match std::fs::symlink_metadata(&current) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        create_private_directory_component(&current)?;
                        std::fs::symlink_metadata(&current).map_err(|error| {
                            format!(
                                "failed to inspect created trace directory `{}`: {error}",
                                current.display()
                            )
                        })?
                    }
                    Err(error) => {
                        return Err(format!(
                            "failed to inspect trace directory component `{}`: {error}",
                            current.display()
                        ));
                    }
                };
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;

                    if metadata.file_attributes() & WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                        return Err(format!(
                            "refusing Windows reparse-point trace component `{}`",
                            current.display()
                        ));
                    }
                }
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "refusing symbolic-link or non-directory trace component `{}`",
                        current.display()
                    ));
                }
                validate_trace_directory_component_ownership(&current, &metadata)?;
            }
        }
    }
    Ok(())
}

fn create_private_directory_component(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(not(unix))]
    let builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        builder.mode(0o700);
    }
    builder.create(path).map_err(|error| {
        format!(
            "failed to create private trace directory `{}`: {error}",
            path.display()
        )
    })?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        let directory = std::fs::OpenOptions::new()
            .read(true)
            .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL | WINDOWS_WRITE_DAC)
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .map_err(|error| {
                format!(
                    "failed to securely open created Windows trace directory `{}`: {error}",
                    path.display()
                )
            })?;
        validate_windows_path_identity(path, &directory, true)
            .map_err(|error| error.to_string())?;
        set_windows_private_acl(&directory, true).map_err(|error| error.to_string())?;
        validate_windows_private_owner_and_acl(path, &directory)
            .map_err(|error| error.to_string())?;
        validate_windows_path_identity(path, &directory, true)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn validate_trace_directory_component_ownership(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let mode = u64::from(metadata.permissions().mode());
        let owner_is_trusted = metadata.uid() == effective_user_id() || metadata.uid() == 0;
        let protected_from_replacement = mode & 0o022 == 0 || mode & u64::from(libc::S_ISVTX) != 0;
        if !owner_is_trusted || !protected_from_replacement {
            return Err(format!(
                "trace directory component is writable by another user `{}`",
                path.display()
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = (path, metadata);
    Ok(())
}

#[cfg(unix)]
fn effective_user_id() -> u32 {
    // SAFETY: geteuid has no preconditions and does not dereference pointers.
    unsafe { libc::geteuid() }
}

fn validate_trace_file_name_prefix(prefix: &str) -> Result<(), String> {
    if prefix.is_empty()
        || !prefix.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(
            "trace file prefix must contain only ASCII letters, digits, dot, underscore, or dash"
                .to_string(),
        );
    }
    Ok(())
}

fn open_private_rolling_trace_file(
    directory: &Path,
    directory_identity: &TraceDirectoryIdentity,
    prefix: &str,
    day: chrono::NaiveDate,
    sequence: u32,
) -> Result<std::fs::File, String> {
    if sequence > MAX_ROLLING_TRACE_SEQUENCE {
        return Err("rolling trace sequence exhausted".to_string());
    }
    directory_identity.validate(directory)?;
    let file_name = rolling_trace_file_name(prefix, day, sequence);
    let path = directory.join(&file_name);
    let file = open_private_trace_file_in_directory(
        directory_identity,
        std::ffi::OsStr::new(&file_name),
        &path,
    )?;
    directory_identity.validate(directory)?;
    Ok(file)
}

fn rolling_trace_file_name(prefix: &str, day: chrono::NaiveDate, sequence: u32) -> String {
    if sequence == 0 {
        format!("{prefix}.{day}")
    } else {
        format!("{prefix}.{day}.{sequence:04}")
    }
}

fn next_rolling_trace_sequence(
    directory: &Path,
    prefix: &str,
    day: chrono::NaiveDate,
    max_file_bytes: u64,
) -> Result<(u32, u64), String> {
    let mut latest = None;
    for entry in std::fs::read_dir(directory).map_err(|error| {
        format!(
            "failed to inspect rolling trace directory `{}`: {error}",
            directory.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to inspect rolling trace entry in `{}`: {error}",
                directory.display()
            )
        })?;
        let Some((entry_day, sequence)) =
            rolling_trace_file_identity(entry.file_name().to_string_lossy().as_ref(), prefix)
        else {
            continue;
        };
        let is_file = entry
            .file_type()
            .map_err(|error| {
                format!(
                    "failed to inspect rolling trace entry type `{}`: {error}",
                    entry.path().display()
                )
            })?
            .is_file();
        if entry_day != day || !is_file {
            continue;
        }
        let bytes = entry
            .metadata()
            .map_err(|error| {
                format!(
                    "failed to inspect rolling trace entry metadata `{}`: {error}",
                    entry.path().display()
                )
            })?
            .len();
        if latest.is_none_or(|(latest_sequence, _)| sequence > latest_sequence) {
            latest = Some((sequence, bytes));
        }
    }
    match latest {
        Some((sequence, bytes)) if bytes >= max_file_bytes => {
            Ok((next_rolling_trace_sequence_number(sequence)?, 0))
        }
        Some(latest) => Ok(latest),
        None => Ok((0, 0)),
    }
}

fn next_rolling_trace_sequence_number(sequence: u32) -> Result<u32, String> {
    let sequence = sequence
        .checked_add(1)
        .ok_or_else(|| "rolling trace sequence exhausted".to_string())?;
    if sequence > MAX_ROLLING_TRACE_SEQUENCE {
        return Err("rolling trace sequence exhausted".to_string());
    }
    Ok(sequence)
}

fn trace_retained_file_count() -> usize {
    if let Some(config) = crate::configuration::current_process_config() {
        return config.config.diagnostics.max_files as usize;
    }
    trace_retained_file_count_from_value(std::env::var(TRACE_MAX_FILES_ENV_VAR).ok().as_deref())
}

fn trace_retained_file_count_from_value(value: Option<&str>) -> usize {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|count| (1..=MAX_TRACE_MAX_FILES).contains(count))
        .unwrap_or(DEFAULT_TRACE_MAX_FILES)
}

fn trace_max_file_bytes() -> u64 {
    if let Some(config) = crate::configuration::current_process_config() {
        return config.config.diagnostics.max_file_bytes;
    }
    trace_max_file_bytes_from_value(std::env::var(TRACE_MAX_FILE_BYTES_ENV_VAR).ok().as_deref())
}

fn trace_max_file_bytes_from_value(value: Option<&str>) -> u64 {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|bytes| (MIN_TRACE_MAX_FILE_BYTES..=MAX_TRACE_MAX_FILE_BYTES).contains(bytes))
        .unwrap_or(DEFAULT_TRACE_MAX_FILE_BYTES)
}

fn trace_max_total_bytes() -> u64 {
    if let Some(config) = crate::configuration::current_process_config() {
        return config.config.diagnostics.max_total_bytes;
    }
    trace_max_total_bytes_from_value(std::env::var(TRACE_MAX_TOTAL_BYTES_ENV_VAR).ok().as_deref())
}

fn trace_max_total_bytes_from_value(value: Option<&str>) -> u64 {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|bytes| (MIN_TRACE_MAX_FILE_BYTES..=MAX_TRACE_MAX_TOTAL_BYTES).contains(bytes))
        .unwrap_or(DEFAULT_TRACE_MAX_TOTAL_BYTES)
}

impl Write for PrivateRollingFileAppender {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let current_day = chrono::Utc::now().date_naive();
        if current_day != self.current_day {
            self.roll_to(current_day, 0).map_err(io::Error::other)?;
        }
        let buffer_bytes = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if buffer_bytes > self.max_file_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "one trace event exceeded the configured per-file byte limit",
            ));
        }
        if self.current_file_bytes > 0
            && self.current_file_bytes.saturating_add(buffer_bytes) > self.max_file_bytes
        {
            let next_sequence = next_rolling_trace_sequence_number(self.current_sequence)
                .map_err(io::Error::other)?;
            self.roll_to(current_day, next_sequence)
                .map_err(io::Error::other)?;
        }
        let written = self.file.write(buffer)?;
        self.current_file_bytes = self
            .current_file_bytes
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn non_blocking_with_thread_name<T: std::io::Write + Send + 'static>(
    writer: T,
    thread_name: &str,
) -> (NonBlocking, WorkerGuard) {
    NonBlockingBuilder::default()
        .thread_name(thread_name)
        .finish(writer)
}

#[cfg(feature = "tokio-console")]
fn install_tracing_subscriber<L>(
    env_filter: EnvFilter,
    fmt_layer: L,
    tokio_console: bool,
) -> Result<(), String>
where
    L: tracing_subscriber::Layer<
            tracing_subscriber::layer::Layered<EnvFilter, tracing_subscriber::Registry>,
        > + Send
        + Sync
        + 'static,
{
    if tokio_console && tokio_console_runtime_hint_enabled() {
        return tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .with(
                console_subscriber::ConsoleLayer::builder()
                    .with_default_env()
                    .spawn(),
            )
            .try_init()
            .map_err(|error| format!("failed to install tracing subscriber: {error}"));
    }
    if tokio_console {
        eprintln!(
            "akra tokio-console requested but RUSTFLAGS does not include `tokio_unstable`; tracing file logging remains enabled"
        );
    }
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .try_init()
        .map_err(|error| format!("failed to install tracing subscriber: {error}"))
}

#[cfg(not(feature = "tokio-console"))]
fn install_tracing_subscriber<L>(
    env_filter: EnvFilter,
    fmt_layer: L,
    tokio_console: bool,
) -> Result<(), String>
where
    L: tracing_subscriber::Layer<
            tracing_subscriber::layer::Layered<EnvFilter, tracing_subscriber::Registry>,
        > + Send
        + Sync
        + 'static,
{
    if tokio_console {
        eprintln!(
            "akra tokio-console requested but this binary was built without the `tokio-console` feature; tracing file logging remains enabled"
        );
    }
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .try_init()
        .map_err(|error| format!("failed to install tracing subscriber: {error}"))
}

fn tokio_console_value_is_enabled(value: &str) -> bool {
    trace_value_is_enabled_bool(&value.trim().to_ascii_lowercase())
}

#[cfg(feature = "tokio-console")]
fn tokio_console_runtime_hint_enabled() -> bool {
    std::env::var("RUSTFLAGS")
        .ok()
        .is_some_and(|value| value.contains("tokio_unstable"))
}

fn trace_settings_from_environment(environment: &TraceEnvironment) -> Option<TraceSettings> {
    match environment.akra_trace.as_deref() {
        Some(value) => apply_rust_log_override_from_value(
            trace_settings_from_value(value)?,
            environment.rust_log.as_deref(),
        ),
        None => None,
    }
}

fn trace_settings_from_value(value: &str) -> Option<TraceSettings> {
    let trimmed = value.trim();
    if trace_value_is_disabled(trimmed) {
        return None;
    }

    let normalized = trimmed.to_ascii_lowercase();
    if trace_value_is_enabled_bool(&normalized) {
        return Some(concise_trace_settings());
    }
    match normalized.as_str() {
        "planning" => Some(planning_trace_settings()),
        "full" => Some(full_trace_settings()),
        _ => EnvFilter::try_new(trimmed).ok().map(|_| TraceSettings {
            filter: trimmed.to_string(),
            span_mode: TraceSpanMode::None,
        }),
    }
}

fn trace_value_is_disabled(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    value.is_empty() || matches!(normalized.as_str(), "0" | "false" | "no" | "off")
}

fn trace_value_is_enabled_bool(value: &str) -> bool {
    matches!(value, "1" | "true" | "yes" | "on")
}

fn apply_rust_log_override_from_value(
    mut settings: TraceSettings,
    rust_log: Option<&str>,
) -> Option<TraceSettings> {
    if let Some(rust_log) = rust_log {
        settings.filter = trace_filter_from_rust_log_value(rust_log)?;
    }
    Some(settings)
}

fn trace_filter_from_rust_log_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trace_value_is_disabled(trimmed) {
        None
    } else if EnvFilter::try_new(trimmed).is_ok() {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn concise_trace_settings() -> TraceSettings {
    TraceSettings {
        filter: concise_trace_filter(),
        span_mode: TraceSpanMode::None,
    }
}

fn planning_trace_settings() -> TraceSettings {
    TraceSettings {
        filter: planning_trace_filter(),
        span_mode: TraceSpanMode::Close,
    }
}

fn full_trace_settings() -> TraceSettings {
    TraceSettings {
        filter: full_trace_filter(),
        span_mode: TraceSpanMode::Full,
    }
}

fn concise_trace_filter() -> String {
    format!("{AKRA_EVENT_TARGET}=debug,codex_exec_loop_native=debug,warn")
}

fn planning_trace_filter() -> String {
    format!(
        "{AKRA_EVENT_TARGET}=debug,\
         codex_exec_loop_native::application::service::planning=trace,\
         codex_exec_loop_native::adapter::inbound::tui::app::turn_submission_runtime::post_turn_execution=trace,\
         codex_exec_loop_native::adapter::outbound::app_server::planning_worker=trace,\
         codex_exec_loop_native::adapter::outbound::app_server=debug,\
         codex_exec_loop_native=info,warn"
    )
}

fn full_trace_filter() -> String {
    "trace".to_string()
}

fn env_filter_from_filter_value(filter: &str) -> EnvFilter {
    EnvFilter::new(valid_filter_or_fallback(filter))
}

fn valid_filter_or_fallback(filter: &str) -> String {
    EnvFilter::try_new(filter)
        .map(|_| filter.to_string())
        .unwrap_or_else(|_| concise_trace_filter())
}

fn trace_span_mode_from_value(value: &str) -> Option<TraceSpanMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" | "0" | "off" => Some(TraceSpanMode::None),
        "close" => Some(TraceSpanMode::Close),
        "full" => Some(TraceSpanMode::Full),
        _ => None,
    }
}

fn trace_destination_from_environment(environment: &TraceEnvironment) -> Option<TraceDestination> {
    if let Some(path) = &environment.trace_file {
        if path.as_os_str().is_empty() {
            return None;
        }
        return Some(TraceDestination::ExactFile(path.clone()));
    }
    let directory = environment.default_log_directory.clone()?;
    Some(TraceDestination::DailyRolling {
        directory,
        file_name: "akra-trace.jsonl".to_string(),
    })
}

fn default_trace_log_directory() -> Option<PathBuf> {
    Some(
        std::env::current_dir()
            .ok()?
            .join(".codex-exec-loop")
            .join("runtime")
            .join("log"),
    )
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use serde_json::{Map, json};
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    #[cfg(windows)]
    use super::open_private_trace_file;
    #[cfg(windows)]
    use super::prepare_private_trace_directory;
    use super::{
        AKRA_EVENT_TARGET, AkraJsonFormat, BoundedExactTraceWriter, PrivateRollingFileAppender,
        TraceConfig, TraceDestination, TraceEnvironment, TraceSpanMode, concise_trace_filter,
        default_trace_log_directory, full_trace_filter, merge_detail_field, non_blocking_writer,
        planning_trace_filter, rolling_trace_file_day, sanitize_trace_value,
        tokio_console_value_is_enabled, trace_config_from_environment,
        trace_destination_from_environment, trace_filter_from_rust_log_value,
        trace_max_file_bytes_from_value, trace_max_total_bytes_from_value,
        trace_retained_file_count_from_value, trace_settings_from_environment,
        trace_settings_from_value, trace_span_mode_from_value, valid_filter_or_fallback,
    };

    #[test]
    fn structured_trace_sanitizer_redacts_body_fields_but_preserves_counts() {
        let detail = json!({
            "task_id": "task-1",
            "task_title": "private customer incident",
            "title": "private thread prompt preview",
            "prompt_chars": 42,
            "nested": {
                "error": "database password was rejected",
                "notice_count": 1,
                "authorization_header": "Bearer private-token",
                "x-api-key": "private-api-key",
                "auth.header": "Bearer private-auth-token",
                "token_count": 7,
                "credential_present": "private-metric-bypass",
                "message_count": ["private-count-bypass"],
            },
            "opaque_label": "private unknown string",
            "opaque_payload": { "value": "private unknown object" },
            "origin": "https://operator:secret@example.test/path?token=secret",
            "source": "/home/operator/private.rs",
            "model": "gpt-5.4\nprivate",
            "thread_id": "x".repeat(257),
            "status": "ready",
            "message": "dynamic provider body",
        });

        let sanitized = sanitize_trace_value(None, &detail);
        assert_eq!(sanitized["task_id"], json!("task-1"));
        assert_eq!(sanitized["task_title"]["redacted"], json!(true));
        assert_eq!(sanitized["task_title"]["chars"], json!(25));
        assert_eq!(sanitized["title"]["redacted"], json!(true));
        assert_eq!(sanitized["title"]["chars"], json!(29));
        assert_eq!(sanitized["prompt_chars"], json!(42));
        assert_eq!(sanitized["nested"]["redacted"], json!(true));
        assert_eq!(sanitized["nested"]["fields"], json!(8));
        assert_eq!(sanitized["opaque_label"]["redacted"], json!(true));
        assert_eq!(sanitized["opaque_payload"]["redacted"], json!(true));
        assert_eq!(sanitized["opaque_payload"]["fields"], json!(1));
        assert_eq!(sanitized["origin"]["redacted"], json!(true));
        assert_eq!(sanitized["source"]["redacted"], json!(true));
        assert_eq!(sanitized["model"]["redacted"], json!(true));
        assert_eq!(sanitized["thread_id"]["redacted"], json!(true));
        assert_eq!(sanitized["status"], json!("ready"));
        assert_eq!(sanitized["message"]["redacted"], json!(true));
        assert!(!sanitized.to_string().contains("customer incident"));
        assert!(!sanitized.to_string().contains("thread prompt preview"));
        assert!(!sanitized.to_string().contains("password"));
        assert!(!sanitized.to_string().contains("private-token"));
        assert!(!sanitized.to_string().contains("private-api-key"));
        assert!(!sanitized.to_string().contains("private-auth-token"));
        assert!(!sanitized.to_string().contains("private-metric-bypass"));
        assert!(!sanitized.to_string().contains("private-count-bypass"));
        assert!(!sanitized.to_string().contains("provider body"));
        assert!(!sanitized.to_string().contains("unknown string"));
        assert!(!sanitized.to_string().contains("unknown object"));
        assert!(!sanitized.to_string().contains("operator:secret"));
        assert!(!sanitized.to_string().contains("/home/operator"));
    }

    #[test]
    fn trace_preset_values_map_to_expected_filters_and_spans() {
        let concise = trace_settings_from_value("1").expect("concise preset should be enabled");
        assert_eq!(concise.filter, concise_trace_filter());
        assert_eq!(concise.span_mode, TraceSpanMode::None);

        let planning =
            trace_settings_from_value("planning").expect("planning preset should be enabled");
        assert_eq!(planning.filter, planning_trace_filter());
        assert_eq!(planning.span_mode, TraceSpanMode::Close);

        let full = trace_settings_from_value("full").expect("full preset should be enabled");
        assert_eq!(full.filter, full_trace_filter());
        assert_eq!(full.span_mode, TraceSpanMode::Full);
    }

    #[test]
    fn trace_boolean_semantics_do_not_promote_enabled_values_to_full_trace() {
        for value in ["1", "true", "yes", "on"] {
            let settings = trace_settings_from_value(value).expect("value should enable tracing");
            assert_eq!(settings.filter, concise_trace_filter());
            assert_eq!(settings.span_mode, TraceSpanMode::None);
        }
        for value in ["", "0", "false", "no", "off"] {
            assert_eq!(trace_settings_from_value(value), None);
        }
        assert_eq!(trace_settings_from_value("not a valid filter["), None);
    }

    #[test]
    fn trace_file_policy_is_off_without_valid_explicit_opt_in() {
        assert_eq!(
            with_trace_values(&[], trace_settings_from_environment),
            None
        );
        assert_eq!(
            with_trace_values(
                &[("AKRA_TRACE", Some("1")), ("RUST_LOG", Some("[invalid"))],
                trace_settings_from_environment,
            ),
            None
        );
        assert_eq!(
            with_trace_values(
                &[("AKRA_TRACE", None), ("RUST_LOG", Some("[invalid"))],
                trace_settings_from_environment,
            ),
            None
        );
    }

    #[test]
    fn trace_custom_filter_falls_back_to_spanless_mode() {
        let settings = trace_settings_from_value(
            "codex_exec_loop_native::adapter::outbound::app_server=debug",
        )
        .expect("custom filter should be enabled");

        assert_eq!(
            settings.filter,
            "codex_exec_loop_native::adapter::outbound::app_server=debug"
        );
        assert_eq!(settings.span_mode, TraceSpanMode::None);
    }

    #[test]
    fn trace_span_override_parser_accepts_only_documented_values() {
        assert_eq!(
            trace_span_mode_from_value("none"),
            Some(TraceSpanMode::None)
        );
        assert_eq!(trace_span_mode_from_value("0"), Some(TraceSpanMode::None));
        assert_eq!(trace_span_mode_from_value("off"), Some(TraceSpanMode::None));
        assert_eq!(
            trace_span_mode_from_value("close"),
            Some(TraceSpanMode::Close)
        );
        assert_eq!(
            trace_span_mode_from_value("full"),
            Some(TraceSpanMode::Full)
        );
        assert_eq!(trace_span_mode_from_value("verbose"), None);
    }

    #[test]
    fn fmt_span_mapping_matches_trace_span_mode_contract() {
        assert_eq!(
            TraceSpanMode::None.fmt_span(),
            tracing_subscriber::fmt::format::FmtSpan::NONE
        );
        assert_eq!(
            TraceSpanMode::Close.fmt_span(),
            tracing_subscriber::fmt::format::FmtSpan::CLOSE
        );
        assert_eq!(
            TraceSpanMode::Full.fmt_span(),
            tracing_subscriber::fmt::format::FmtSpan::FULL
        );
    }

    #[test]
    fn concise_filter_includes_stable_akra_event_target() {
        assert!(concise_trace_filter().contains(AKRA_EVENT_TARGET));
    }

    #[test]
    fn invalid_custom_filter_falls_back_to_concise_filter() {
        assert_eq!(
            valid_filter_or_fallback("not a valid filter["),
            concise_trace_filter()
        );
    }

    #[test]
    fn rust_log_filter_uses_env_filter_semantics_without_enabling_disabled_values() {
        assert_eq!(
            trace_filter_from_rust_log_value("codex_exec_loop_native=trace"),
            Some("codex_exec_loop_native=trace".to_string())
        );
        assert_eq!(
            trace_filter_from_rust_log_value("  codex_exec_loop_native=debug  "),
            Some("codex_exec_loop_native=debug".to_string())
        );
        assert_eq!(trace_filter_from_rust_log_value("off"), None);
        assert_eq!(trace_filter_from_rust_log_value(""), None);
        assert_eq!(trace_filter_from_rust_log_value("[invalid"), None);
    }

    #[test]
    fn tokio_console_setting_uses_documented_boolean_values() {
        for value in ["1", "true", "yes", "on", " ON "] {
            assert!(tokio_console_value_is_enabled(value));
        }
        for value in ["", "0", "false", "no", "off", "full"] {
            assert!(!tokio_console_value_is_enabled(value));
        }
    }

    #[test]
    fn akra_detail_json_is_flattened_into_event_fields() {
        let mut fields = Map::new();

        assert!(merge_detail_field(
            &mut fields,
            &json!(r#"{"operation":"submit","prompt_len":5,"task_title":"private task"}"#)
        ));

        assert_eq!(fields["operation"], "submit");
        assert_eq!(fields["prompt_len"], 5);
        assert!(!fields.contains_key("prompt"));
        assert!(!fields.contains_key("detail"));
        assert_eq!(fields["task_title"]["redacted"], true);
        assert_eq!(fields["task_title"]["chars"], 12);
    }

    #[test]
    fn akra_json_visitor_flattens_detail_without_leaving_detail_field() {
        let mut visitor = super::AkraJsonVisitor::default();

        visitor.insert_value("event", json!("user_prompt_submit_inspected"));
        visitor.insert_value(
            "detail",
            json!(
                r#"{"origin":"Manual","transcript_text_len":10,"prompt_len":12,"parallel_mode_enabled":true}"#
            ),
        );
        let fields = visitor.into_map();

        assert_eq!(fields["event"], "user_prompt_submit_inspected");
        assert_eq!(fields["origin"], "Manual");
        assert_eq!(fields["transcript_text_len"], 10);
        assert_eq!(fields["prompt_len"], 12);
        assert_eq!(fields["parallel_mode_enabled"], true);
        assert!(!fields.contains_key("transcript_text"));
        assert!(!fields.contains_key("prompt"));
        assert!(!fields.contains_key("detail"));
    }

    #[test]
    fn akra_json_formatter_emits_flattened_event_and_span_context() {
        let capture = CaptureWriter::default();
        let fmt_layer = tracing_subscriber::fmt::layer()
            .json()
            .with_span_events(TraceSpanMode::Full.fmt_span())
            .event_format(AkraJsonFormat)
            .with_writer(capture.clone());
        let subscriber = tracing_subscriber::registry()
            .with(EnvFilter::new("trace"))
            .with(fmt_layer);

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(
                "turn_stream",
                thread_id = "thread-1",
                turn = 7_u64,
                task_title = "private span task",
                x_api_key = "private span credential"
            );
            let _entered = span.enter();
            tracing::debug!(
                target: AKRA_EVENT_TARGET,
                event = "turn_stream_reduced",
                detail = r#"{"phase":"delta","token_count":3}"#,
                ok = true,
                latency_ms = 12_i64,
                ratio = 1.5_f64,
                "private dynamic event message"
            );
        });

        let lines = capture.lines();
        let event = lines
            .iter()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).expect("trace line should be JSON")
            })
            .find(|value| {
                value.get("event").and_then(serde_json::Value::as_str)
                    == Some("turn_stream_reduced")
            })
            .expect("captured trace should include the emitted akra event");

        assert_eq!(event["target"], AKRA_EVENT_TARGET);
        assert_eq!(event["phase"], "delta");
        assert_eq!(event["token_count"], 3);
        assert_eq!(event["ok"], true);
        assert_eq!(event["latency_ms"], 12);
        assert_eq!(event["ratio"], 1.5);
        assert_eq!(event["message"]["redacted"], true);
        assert!(
            !event
                .as_object()
                .expect("event should be an object")
                .contains_key("detail")
        );
        assert_eq!(event["span"]["name"], "turn_stream");
        assert_eq!(event["span"]["task_title"]["redacted"], true);
        assert_eq!(event["span"]["x_api_key"]["redacted"], true);
        assert!(!event.to_string().contains("private span task"));
        assert!(!event.to_string().contains("private span credential"));
        assert!(!event.to_string().contains("private dynamic event message"));
        assert_eq!(event["spans"][0]["name"], "turn_stream");
    }

    #[test]
    fn trace_destination_can_represent_rolling_and_exact_file_modes() {
        assert_eq!(
            TraceDestination::DailyRolling {
                directory: "/tmp/log".into(),
                file_name: "akra-trace.jsonl".to_string(),
            },
            TraceDestination::DailyRolling {
                directory: "/tmp/log".into(),
                file_name: "akra-trace.jsonl".to_string(),
            }
        );
        assert_eq!(
            TraceDestination::ExactFile("/tmp/akra-trace.jsonl".into()),
            TraceDestination::ExactFile("/tmp/akra-trace.jsonl".into())
        );
    }

    #[test]
    fn trace_config_from_env_applies_file_destination_span_override_and_tokio_flag() {
        let trace_file = unique_temp_dir("trace-config").join("akra-trace.jsonl");

        let config = with_trace_values(
            &[
                ("AKRA_TRACE", Some("planning")),
                ("RUST_LOG", None),
                ("AKRA_TRACE_SPANS", Some("full")),
                ("AKRA_TRACE_FILE", Some(path_str(&trace_file))),
                ("AKRA_TOKIO_CONSOLE", Some("yes")),
            ],
            trace_config_from_environment,
        )
        .expect("AKRA_TRACE planning should enable trace config");

        assert_eq!(
            config,
            TraceConfig {
                filter: planning_trace_filter(),
                destination: TraceDestination::ExactFile(trace_file),
                span_mode: TraceSpanMode::Full,
                tokio_console: true,
            }
        );
    }

    #[test]
    fn rust_log_cannot_enable_trace_files_without_akra_trace_opt_in() {
        let settings = with_trace_values(
            &[
                ("AKRA_TRACE", None),
                ("RUST_LOG", Some("codex_exec_loop_native=trace")),
                ("AKRA_TRACE_SPANS", None),
                ("AKRA_TRACE_FILE", None),
                ("AKRA_TOKIO_CONSOLE", None),
            ],
            trace_settings_from_environment,
        );

        assert_eq!(settings, None);
    }

    #[test]
    fn trace_settings_from_env_lets_rust_log_override_akra_trace_filter_only() {
        let settings = with_trace_values(
            &[
                ("AKRA_TRACE", Some("full")),
                ("RUST_LOG", Some("codex_exec_loop_native::adapter=debug")),
                ("AKRA_TRACE_SPANS", None),
                ("AKRA_TRACE_FILE", None),
                ("AKRA_TOKIO_CONSOLE", None),
            ],
            trace_settings_from_environment,
        )
        .expect("AKRA_TRACE full should remain enabled");

        assert_eq!(settings.filter, "codex_exec_loop_native::adapter=debug");
        assert_eq!(settings.span_mode, TraceSpanMode::Full);
    }

    #[test]
    fn trace_config_from_env_respects_disabled_trace_and_empty_file_path() {
        assert_eq!(
            with_trace_values(
                &[
                    ("AKRA_TRACE", Some("off")),
                    ("RUST_LOG", None),
                    ("AKRA_TRACE_SPANS", None),
                    ("AKRA_TRACE_FILE", Some("/tmp/ignored.jsonl")),
                    ("AKRA_TOKIO_CONSOLE", None),
                ],
                trace_config_from_environment,
            ),
            None
        );

        assert_eq!(
            with_trace_values(
                &[
                    ("AKRA_TRACE", Some("1")),
                    ("RUST_LOG", None),
                    ("AKRA_TRACE_SPANS", None),
                    ("AKRA_TRACE_FILE", Some("")),
                    ("AKRA_TOKIO_CONSOLE", None),
                ],
                trace_config_from_environment,
            ),
            None
        );
    }

    #[test]
    fn env_backed_span_and_tokio_helpers_parse_documented_values() {
        assert_eq!(
            with_trace_values(&[("AKRA_TRACE_SPANS", Some("close"))], |environment| {
                environment
                    .span_mode
                    .as_deref()
                    .and_then(trace_span_mode_from_value)
            }),
            Some(TraceSpanMode::Close)
        );
        assert_eq!(
            with_trace_values(&[("AKRA_TRACE_SPANS", Some("verbose"))], |environment| {
                environment
                    .span_mode
                    .as_deref()
                    .and_then(trace_span_mode_from_value)
            }),
            None
        );
        assert!(with_trace_values(
            &[("AKRA_TOKIO_CONSOLE", Some("on"))],
            |environment| environment
                .tokio_console
                .as_deref()
                .is_some_and(tokio_console_value_is_enabled),
        ));
        assert!(!with_trace_values(
            &[("AKRA_TOKIO_CONSOLE", Some("full"))],
            |environment| environment
                .tokio_console
                .as_deref()
                .is_some_and(tokio_console_value_is_enabled),
        ));
    }

    #[test]
    fn trace_destination_prefers_exact_file_and_falls_back_to_default_daily_directory() {
        let exact = unique_temp_dir("trace-destination").join("exact.jsonl");
        assert_eq!(
            with_trace_values(
                &[("AKRA_TRACE_FILE", Some(path_str(&exact)))],
                trace_destination_from_environment,
            ),
            Some(TraceDestination::ExactFile(exact))
        );

        let daily = with_trace_values(
            &[("AKRA_TRACE_FILE", None)],
            trace_destination_from_environment,
        )
        .expect("default trace destination should be available under current dir");
        assert_eq!(
            daily,
            TraceDestination::DailyRolling {
                directory: default_trace_log_directory()
                    .expect("default trace log directory should resolve"),
                file_name: "akra-trace.jsonl".to_string(),
            }
        );
    }

    #[test]
    fn non_blocking_writer_creates_destinations_and_reports_directory_failures() {
        let root = unique_temp_dir("trace-writer");
        let exact_file = root.join("nested").join("trace.jsonl");
        let (mut writer, guard) =
            non_blocking_writer(TraceDestination::ExactFile(exact_file.clone()))
                .expect("exact file writer should create parent directories");
        writer
            .write_all(b"{\"event\":\"exact\"}\n")
            .expect("non-blocking writer should accept bytes");
        drop(writer);
        drop(guard);
        wait_for_file_contains(&exact_file, "exact");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                std::fs::metadata(&exact_file)
                    .expect("exact trace metadata should load")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        let daily_dir = root.join("daily");
        let (writer, guard) = non_blocking_writer(TraceDestination::DailyRolling {
            directory: daily_dir.clone(),
            file_name: "akra-trace.jsonl".to_string(),
        })
        .expect("daily rolling writer should create its directory");
        drop(writer);
        drop(guard);
        assert!(daily_dir.is_dir());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                std::fs::metadata(&daily_dir)
                    .expect("daily trace directory metadata should load")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            for entry in
                std::fs::read_dir(&daily_dir).expect("daily trace directory should be readable")
            {
                let entry = entry.expect("daily trace entry should load");
                if entry.file_type().expect("entry type should load").is_file() {
                    assert_eq!(
                        entry
                            .metadata()
                            .expect("daily trace metadata should load")
                            .permissions()
                            .mode()
                            & 0o777,
                        0o600
                    );
                }
            }
        }

        let parent_file = root.join("not-a-directory");
        std::fs::write(&parent_file, "file").expect("fixture file should be written");
        let error =
            non_blocking_writer(TraceDestination::ExactFile(parent_file.join("trace.jsonl")))
                .expect_err("file parent should fail directory creation");
        assert!(error.contains("refusing symbolic-link or non-directory trace component"));
    }

    #[cfg(unix)]
    #[test]
    fn exact_trace_file_rejects_symbolic_and_hard_links_without_touching_targets() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = unique_temp_dir("trace-link-safety");
        let target = root.join("target.txt");
        let symlink_path = root.join("trace-symlink.jsonl");
        let hard_link_path = root.join("trace-hard-link.jsonl");
        std::fs::write(&target, "do-not-modify\n").expect("target fixture should write");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644))
            .expect("target permissions should set");
        symlink(&target, &symlink_path).expect("trace symlink fixture should create");

        let symlink_error = non_blocking_writer(TraceDestination::ExactFile(symlink_path.clone()))
            .expect_err("exact trace path must reject symbolic links");
        assert!(symlink_error.contains("trace log file"));
        assert_eq!(
            std::fs::read_to_string(&target).expect("target content should remain readable"),
            "do-not-modify\n"
        );
        assert_eq!(
            std::fs::metadata(&target)
                .expect("target metadata should load")
                .permissions()
                .mode()
                & 0o777,
            0o644
        );

        std::fs::hard_link(&target, &hard_link_path)
            .expect("trace hard-link fixture should create");
        let hard_link_error =
            non_blocking_writer(TraceDestination::ExactFile(hard_link_path.clone()))
                .expect_err("exact trace path must reject multiply-linked files");
        assert!(hard_link_error.contains("multiply-linked"));
        assert_eq!(
            std::fs::metadata(&target)
                .expect("hard-link target metadata should load")
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
    }

    #[cfg(unix)]
    #[test]
    fn exact_trace_file_rejects_owned_world_writable_nonsticky_parent() {
        use std::os::unix::fs::PermissionsExt;

        let root = unique_temp_dir("trace-world-writable-parent");
        std::fs::create_dir_all(&root).expect("world-writable parent should create");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777))
            .expect("world-writable parent permissions should set");

        let error = non_blocking_writer(TraceDestination::ExactFile(root.join("trace.jsonl")))
            .expect_err("world-writable nonsticky parent must be rejected");

        assert!(error.contains("writable by another user"));
        assert!(!root.join("trace.jsonl").exists());
    }

    #[cfg(windows)]
    #[test]
    fn exact_trace_file_rejects_windows_hardlinks_before_touching_the_target() {
        let root = unique_temp_dir("trace-hardlink-windows");
        let target = root.join("target.txt");
        let hard_link_path = root.join("trace-hard-link.jsonl");
        std::fs::write(&target, "do-not-modify\n").expect("target fixture should write");
        std::fs::hard_link(&target, &hard_link_path)
            .expect("trace hard-link fixture should create");

        let error = open_private_trace_file(&hard_link_path)
            .expect_err("Windows trace path must reject multiply-linked files");

        assert!(error.contains("single-link"));
        assert_eq!(
            std::fs::read_to_string(&target).expect("target content should remain readable"),
            "do-not-modify\n"
        );
    }

    #[cfg(windows)]
    #[test]
    fn exact_trace_file_rejects_windows_junction_parents_without_touching_targets() {
        let root = unique_temp_dir("trace-junction-windows");
        let redirected_parent = root.join("redirected-parent");
        let linked_parent = root.join("linked-parent");
        std::fs::create_dir(&redirected_parent).expect("redirected parent should create");
        let victim = redirected_parent.join("victim.log");
        std::fs::write(&victim, "do-not-append\n").expect("victim fixture should write");
        let output = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                linked_parent.to_string_lossy().as_ref(),
                redirected_parent.to_string_lossy().as_ref(),
            ])
            .output()
            .expect("junction command should run");
        assert!(output.status.success(), "junction should create");

        let error = non_blocking_writer(TraceDestination::ExactFile(
            linked_parent.join("victim.log"),
        ))
        .expect_err("Windows trace must reject a junction parent");

        assert!(error.contains("reparse-point"));
        assert_eq!(
            std::fs::read_to_string(&victim).expect("victim content should remain readable"),
            "do-not-append\n"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_trace_file_and_rolling_directory_receive_private_owner_acl() {
        let root = unique_temp_dir("trace-private-acl-windows");
        let exact_file = root.join("exact.jsonl");
        let file = open_private_trace_file(&exact_file).expect("private trace file should open");
        crate::private_fs::validate_windows_private_owner_and_acl(&exact_file, &file)
            .expect("trace file ACL should be current-user-only");

        let rolling_dir = root.join("rolling");
        let (_path, identity) = prepare_private_trace_directory(&rolling_dir)
            .expect("private rolling directory should open");
        identity
            .validate(&rolling_dir)
            .expect("rolling directory ACL should be current-user-only");
    }

    #[cfg(unix)]
    #[test]
    fn exact_trace_file_rejects_symlinked_parent_without_chmoding_real_parents() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = unique_temp_dir("exact-trace-parent-safety");
        let redirected_parent = root.join("redirected-parent");
        let linked_parent = root.join("linked-parent");
        std::fs::create_dir(&redirected_parent).expect("redirected parent should create");
        let victim = redirected_parent.join("victim.log");
        std::fs::write(&victim, "do-not-append\n").expect("victim fixture should write");
        std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o644))
            .expect("victim permissions should set");
        symlink(&redirected_parent, &linked_parent).expect("parent symlink should create");

        let error = match non_blocking_writer(TraceDestination::ExactFile(
            linked_parent.join("victim.log"),
        )) {
            Ok(_) => panic!("exact trace must reject a symlinked parent"),
            Err(error) => error,
        };
        assert!(error.contains("symbolic-link or non-directory trace component"));
        assert_eq!(
            std::fs::read_to_string(&victim).expect("victim content should remain readable"),
            "do-not-append\n"
        );
        assert_eq!(
            std::fs::metadata(&victim)
                .expect("victim metadata should load")
                .permissions()
                .mode()
                & 0o777,
            0o644
        );

        let existing_parent = root.join("operator-parent");
        std::fs::create_dir(&existing_parent).expect("operator parent should create");
        std::fs::set_permissions(&existing_parent, std::fs::Permissions::from_mode(0o755))
            .expect("operator parent permissions should set");
        let (writer, guard) = non_blocking_writer(TraceDestination::ExactFile(
            existing_parent.join("trace.jsonl"),
        ))
        .expect("regular operator parent should remain supported");
        drop(writer);
        drop(guard);
        assert_eq!(
            std::fs::metadata(&existing_parent)
                .expect("operator parent metadata should load")
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }

    #[cfg(unix)]
    #[test]
    fn rolling_trace_rejects_symlinked_directory_and_daily_file() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = unique_temp_dir("rolling-trace-link-safety");
        let redirected_directory = root.join("redirect-target");
        let linked_directory = root.join("linked-log-dir");
        std::fs::create_dir(&redirected_directory)
            .expect("redirect target directory should create");
        symlink(&redirected_directory, &linked_directory)
            .expect("rolling directory symlink should create");
        let directory_error =
            PrivateRollingFileAppender::new(linked_directory, "akra-trace.jsonl".to_string(), 7)
                .expect_err("rolling trace must reject a symlinked directory");
        assert!(directory_error.contains("symbolic-link or non-directory trace component"));
        assert!(
            std::fs::read_dir(&redirected_directory)
                .expect("redirect target should remain readable")
                .next()
                .is_none()
        );

        let safe_directory = root.join("safe-log-dir");
        std::fs::create_dir(&safe_directory).expect("safe rolling directory should create");
        let target = root.join("daily-target.txt");
        std::fs::write(&target, "do-not-append\n").expect("daily target should write");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644))
            .expect("daily target permissions should set");
        let daily_path = safe_directory.join(format!(
            "akra-trace.jsonl.{}",
            chrono::Utc::now().date_naive()
        ));
        symlink(&target, &daily_path).expect("daily trace symlink should create");

        let daily_error =
            PrivateRollingFileAppender::new(safe_directory, "akra-trace.jsonl".to_string(), 7)
                .expect_err("rolling trace must reject a symlinked daily file");
        assert!(daily_error.contains("refusing symbolic-link"));
        assert_eq!(
            std::fs::read_to_string(&target).expect("daily target should remain readable"),
            "do-not-append\n"
        );
        assert_eq!(
            std::fs::metadata(&target)
                .expect("daily target metadata should load")
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
    }

    #[test]
    fn trace_retention_uses_bounded_positive_file_counts() {
        assert_eq!(trace_retained_file_count_from_value(None), 7);
        assert_eq!(trace_retained_file_count_from_value(Some(" 14 ")), 14);
        for value in [Some(""), Some("0"), Some("bogus"), Some("366")] {
            assert_eq!(trace_retained_file_count_from_value(value), 7);
        }
    }

    #[test]
    fn trace_byte_limits_reject_unbounded_or_invalid_values() {
        assert_eq!(trace_max_file_bytes_from_value(None), 16 * 1024 * 1024);
        assert_eq!(trace_max_file_bytes_from_value(Some(" 131072 ")), 131_072);
        assert_eq!(trace_max_total_bytes_from_value(None), 64 * 1024 * 1024);
        assert_eq!(trace_max_total_bytes_from_value(Some(" 262144 ")), 262_144);
        for value in [Some(""), Some("0"), Some("1024"), Some("bogus")] {
            assert_eq!(trace_max_file_bytes_from_value(value), 16 * 1024 * 1024);
            assert_eq!(trace_max_total_bytes_from_value(value), 64 * 1024 * 1024);
        }
    }

    #[test]
    fn rolling_trace_rotates_and_prunes_to_total_byte_budget() {
        let directory = unique_temp_dir("trace-byte-budget");
        let mut appender = PrivateRollingFileAppender::new_with_limits(
            directory.clone(),
            "akra-trace.jsonl".to_string(),
            10,
            12,
            30,
        )
        .expect("bounded rolling appender should create");

        for line in [b"1234567890\n", b"abcdefghij\n", b"ABCDEFGHIJ\n"] {
            appender
                .write_all(line)
                .expect("bounded event should rotate successfully");
        }
        appender.flush().expect("trace appender should flush");
        drop(appender);

        let files = std::fs::read_dir(&directory)
            .expect("trace directory should load")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .collect::<Vec<_>>();
        let sizes = files
            .iter()
            .map(|entry| entry.metadata().expect("trace metadata should load").len())
            .collect::<Vec<_>>();
        assert_eq!(files.len(), 2);
        assert!(sizes.iter().all(|size| *size <= 12));
        assert!(sizes.iter().sum::<u64>() <= 30);
    }

    #[test]
    fn rolling_trace_day_transition_does_not_reopen_a_full_segment() {
        let directory = unique_temp_dir("trace-day-transition");
        let mut appender = PrivateRollingFileAppender::new_with_limits(
            directory.clone(),
            "akra-trace.jsonl".to_string(),
            4,
            12,
            48,
        )
        .expect("bounded rolling appender should create");
        appender
            .write_all(b"123456789012")
            .expect("first segment should reach its exact limit");
        appender.current_day = appender.current_day - chrono::Days::new(1);

        appender
            .write_all(b"x")
            .expect("day transition should select a new segment");
        appender.flush().expect("trace appender should flush");
        assert_eq!(appender.current_sequence, 1);
        drop(appender);

        let sizes = std::fs::read_dir(&directory)
            .expect("trace directory should load")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.metadata().expect("trace metadata should load").len())
            .collect::<Vec<_>>();
        assert_eq!(sizes.len(), 2);
        assert!(sizes.iter().all(|size| *size <= 12));
    }

    #[test]
    fn exact_trace_writer_stops_at_the_configured_byte_limit() {
        let directory = unique_temp_dir("trace-exact-byte-limit");
        std::fs::create_dir_all(&directory).expect("exact trace directory should create");
        let path = directory.join("trace.jsonl");
        let mut file = std::fs::File::create(&path).expect("exact trace file should create");
        file.write_all(b"1234")
            .expect("exact trace fixture should write");
        let mut writer =
            BoundedExactTraceWriter::new(file, 6).expect("bounded exact writer should create");

        writer
            .write_all(b"56")
            .expect("write through the exact limit should succeed");
        let error = writer
            .write_all(b"7")
            .expect_err("write beyond the exact limit must fail closed");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        drop(writer);
        assert_eq!(
            std::fs::read(&path).expect("exact trace file should remain readable"),
            b"123456"
        );

        let oversized =
            std::fs::File::open(&path).expect("oversized exact trace should remain inspectable");
        assert!(
            BoundedExactTraceWriter::new(oversized, 5).is_err(),
            "an existing exact file above the limit must fail initialization"
        );
    }

    #[test]
    fn rolling_trace_retention_prunes_old_matching_files() {
        let directory = unique_temp_dir("trace-retention");
        std::fs::create_dir_all(&directory).expect("retention directory should create");
        for day in 1..=5 {
            std::fs::write(
                directory.join(format!("akra-trace.jsonl.2026-01-{day:02}")),
                "old trace\n",
            )
            .expect("old trace fixture should write");
        }
        let prefixed_backup = directory.join("akra-trace.jsonl.000-important-backup");
        let invalid_date = directory.join("akra-trace.jsonl.2026-99-99");
        std::fs::write(&prefixed_backup, "must survive retention\n")
            .expect("prefixed backup fixture should write");
        std::fs::write(&invalid_date, "must survive retention\n")
            .expect("invalid date fixture should write");

        let appender =
            PrivateRollingFileAppender::new(directory.clone(), "akra-trace.jsonl".to_string(), 3)
                .expect("rolling appender should enforce retention");
        drop(appender);

        let retained = std::fs::read_dir(&directory)
            .expect("retention directory should load")
            .filter_map(Result::ok)
            .filter(|entry| {
                rolling_trace_file_day(
                    entry.file_name().to_string_lossy().as_ref(),
                    "akra-trace.jsonl",
                )
                .is_some()
            })
            .count();
        assert_eq!(retained, 3);
        assert_eq!(
            std::fs::read_to_string(prefixed_backup).expect("prefixed backup must not be pruned"),
            "must survive retention\n"
        );
        assert_eq!(
            std::fs::read_to_string(invalid_date).expect("invalid date file must not be pruned"),
            "must survive retention\n"
        );
    }

    #[test]
    fn rolling_trace_retention_never_removes_the_open_current_day_file() {
        let directory = unique_temp_dir("trace-retention-current");
        let mut appender =
            PrivateRollingFileAppender::new(directory.clone(), "akra-trace.jsonl".to_string(), 2)
                .expect("rolling appender should create");
        let current_path = directory.join(format!(
            "akra-trace.jsonl.{}",
            appender.current_day.format("%Y-%m-%d")
        ));
        for offset in 1..=3 {
            let future_day = appender.current_day + chrono::Days::new(offset);
            std::fs::write(
                directory.join(format!(
                    "akra-trace.jsonl.{}",
                    future_day.format("%Y-%m-%d")
                )),
                "future trace\n",
            )
            .expect("future trace fixture should write");
        }

        appender
            .prune_old_files()
            .expect("retention should complete");
        std::io::Write::write_all(&mut appender, b"current trace remains writable\n")
            .expect("current trace should remain open and writable");
        drop(appender);

        assert!(current_path.is_file());
        assert!(
            std::fs::read_to_string(current_path)
                .expect("current trace should remain readable")
                .contains("current trace remains writable")
        );
    }

    #[derive(Clone, Default)]
    struct CaptureWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl CaptureWriter {
        fn lines(&self) -> Vec<String> {
            let bytes = self
                .bytes
                .lock()
                .expect("capture lock should not be poisoned");
            String::from_utf8(bytes.clone())
                .expect("captured trace should be UTF-8")
                .lines()
                .map(str::to_string)
                .collect()
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureSink;

        fn make_writer(&'a self) -> Self::Writer {
            CaptureSink {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }

    struct CaptureSink {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for CaptureSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes
                .lock()
                .expect("capture lock should not be poisoned")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn with_trace_values<T>(
        updates: &[(&str, Option<&str>)],
        body: impl FnOnce(&TraceEnvironment) -> T,
    ) -> T {
        let mut environment = TraceEnvironment {
            default_log_directory: default_trace_log_directory(),
            ..TraceEnvironment::default()
        };
        for (key, value) in updates {
            match *key {
                "AKRA_TRACE" => environment.akra_trace = value.map(ToString::to_string),
                "RUST_LOG" => environment.rust_log = value.map(ToString::to_string),
                "AKRA_TRACE_SPANS" => environment.span_mode = value.map(ToString::to_string),
                "AKRA_TRACE_FILE" => environment.trace_file = value.map(PathBuf::from),
                "AKRA_TOKIO_CONSOLE" => {
                    environment.tokio_console = value.map(ToString::to_string);
                }
                other => panic!("unsupported trace test environment key: {other}"),
            }
        }
        body(&environment)
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        /*
         * macOS temp_dir는 물리 경로의 symlink다. trace 디렉터리 검증은 symlink
         * 구성요소를 거부하므로 fixture 루트도 물리 경로로 정규화한다.
         */
        let physical_temp = std::fs::canonicalize(std::env::temp_dir())
            .unwrap_or_else(|_| std::env::temp_dir());
        let path = physical_temp.join(format!(
            "codex-exec-loop-{label}-{}-{now}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("temporary directory should be created");
        path
    }

    fn wait_for_file_contains(path: &Path, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if std::fs::read_to_string(path).is_ok_and(|body| body.contains(needle)) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "expected {} to contain `{needle}`",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn path_str(path: &Path) -> &str {
        path.to_str().expect("test path should be valid UTF-8")
    }
}

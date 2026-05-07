use std::fmt;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::{Map, Value};
use tracing::{Event, Level, Subscriber, field};
use tracing_appender::non_blocking::{ErrorCounter, NonBlocking, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::{FmtSpan, Writer};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, FormattedFields};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

pub(crate) const AKRA_EVENT_TARGET: &str = "codex_exec_loop_native::diagnostics::akra_event";
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
    let detail_json = detail.to_string();
    tracing::debug!(
        target: AKRA_EVENT_TARGET,
        pid = std::process::id(),
        event = event,
        detail = detail_json.as_str(),
        "akra_event"
    );
}

fn trace_config_from_env() -> Option<TraceConfig> {
    let mut settings = trace_settings_from_env()?;
    if let Some(span_mode) = trace_span_mode_from_env() {
        settings.span_mode = span_mode;
    }
    Some(TraceConfig {
        filter: settings.filter,
        span_mode: settings.span_mode,
        destination: trace_destination()?,
        tokio_console: tokio_console_requested(),
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
        self.values.insert(field_name.to_string(), value);
    }

    fn into_map(self) -> Map<String, Value> {
        self.values
    }
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
    let Ok(Value::Object(detail)) = serde_json::from_str::<Value>(detail) else {
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
    let mut fields = span
        .extensions()
        .get::<FormattedFields<N>>()
        .and_then(|fields| serde_json::from_str::<Value>(fields).ok())
        .and_then(|fields| match fields {
            Value::Object(fields) => Some(fields),
            _ => None,
        })
        .unwrap_or_default();
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
            std::fs::create_dir_all(&directory).map_err(|error| {
                format!(
                    "failed to create trace log directory `{}`: {error}",
                    directory.display()
                )
            })?;
            let appender = tracing_appender::rolling::daily(directory, file_name);
            Ok(non_blocking_with_thread_name(appender, "akra-trace-log"))
        }
        TraceDestination::ExactFile(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "failed to create trace log directory `{}`: {error}",
                        parent.display()
                    )
                })?;
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|error| {
                    format!(
                        "failed to open trace log file `{}`: {error}",
                        path.display()
                    )
                })?;
            Ok(non_blocking_with_thread_name(file, "akra-trace-log"))
        }
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

fn tokio_console_requested() -> bool {
    std::env::var("AKRA_TOKIO_CONSOLE")
        .ok()
        .is_some_and(|value| tokio_console_value_is_enabled(&value))
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

fn trace_settings_from_env() -> Option<TraceSettings> {
    match std::env::var("AKRA_TRACE") {
        Ok(value) => apply_rust_log_override(trace_settings_from_value(&value)?),
        Err(_) => {
            if let Some(settings) = trace_settings_from_rust_log() {
                return Some(settings);
            }
            default_debug_trace_settings().and_then(apply_rust_log_override)
        }
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
        _ => Some(TraceSettings {
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

fn trace_settings_from_rust_log() -> Option<TraceSettings> {
    let filter = std::env::var("RUST_LOG").ok()?;
    trace_filter_from_rust_log_value(&filter).map(|filter| TraceSettings {
        filter,
        span_mode: TraceSpanMode::None,
    })
}

fn apply_rust_log_override(mut settings: TraceSettings) -> Option<TraceSettings> {
    if let Some(filter) = std::env::var("RUST_LOG")
        .ok()
        .and_then(|filter| trace_filter_from_rust_log_value(&filter))
    {
        settings.filter = filter;
    }
    Some(settings)
}

fn trace_filter_from_rust_log_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trace_value_is_disabled(trimmed) {
        None
    } else {
        Some(trimmed.to_string())
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

#[cfg(debug_assertions)]
fn default_debug_trace_settings() -> Option<TraceSettings> {
    if !super::executable::debug_executable_allows_default_diagnostics(
        std::env::current_exe().ok().as_deref(),
    ) {
        return None;
    }
    Some(concise_trace_settings())
}

#[cfg(not(debug_assertions))]
fn default_debug_trace_settings() -> Option<TraceSettings> {
    None
}

fn trace_span_mode_from_env() -> Option<TraceSpanMode> {
    std::env::var("AKRA_TRACE_SPANS")
        .ok()
        .and_then(|value| trace_span_mode_from_value(&value))
}

fn trace_span_mode_from_value(value: &str) -> Option<TraceSpanMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" | "0" | "off" => Some(TraceSpanMode::None),
        "close" => Some(TraceSpanMode::Close),
        "full" => Some(TraceSpanMode::Full),
        _ => None,
    }
}

fn trace_destination() -> Option<TraceDestination> {
    if let Some(path) = std::env::var_os("AKRA_TRACE_FILE") {
        let path = PathBuf::from(path);
        if path.as_os_str().is_empty() {
            return None;
        }
        return Some(TraceDestination::ExactFile(path));
    }
    let directory = default_trace_log_directory()?;
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
    use serde_json::{Map, json};

    use super::{
        AKRA_EVENT_TARGET, TraceDestination, TraceSpanMode, concise_trace_filter,
        full_trace_filter, merge_detail_field, planning_trace_filter,
        tokio_console_value_is_enabled, trace_filter_from_rust_log_value,
        trace_settings_from_value, trace_span_mode_from_value, valid_filter_or_fallback,
    };

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
        assert_eq!(trace_filter_from_rust_log_value("off"), None);
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
            &json!(r#"{"prompt":"hello","prompt_len":5}"#)
        ));

        assert_eq!(fields["prompt"], "hello");
        assert_eq!(fields["prompt_len"], 5);
        assert!(!fields.contains_key("detail"));
    }

    #[test]
    fn akra_json_visitor_flattens_detail_without_leaving_detail_field() {
        let mut visitor = super::AkraJsonVisitor::default();

        visitor.insert_value("event", json!("user_prompt_submit_inspected"));
        visitor.insert_value(
            "detail",
            json!(
                r#"{"origin":"Manual","transcript_text":"typed text","prompt":"final prompt","prompt_len":12}"#
            ),
        );
        let fields = visitor.into_map();

        assert_eq!(fields["event"], "user_prompt_submit_inspected");
        assert_eq!(fields["origin"], "Manual");
        assert_eq!(fields["transcript_text"], "typed text");
        assert_eq!(fields["prompt"], "final prompt");
        assert_eq!(fields["prompt_len"], 12);
        assert!(!fields.contains_key("detail"));
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
}

use std::path::PathBuf;

use serde_json::Value;
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

pub(crate) const AKRA_EVENT_TARGET: &str = "codex_exec_loop_native::diagnostics::akra_event";

#[derive(Debug, Clone, PartialEq, Eq)]
struct TraceConfig {
    filter: String,
    destination: TraceDestination,
    span_mode: TraceSpanMode,
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
    tracing::debug!(
        target: AKRA_EVENT_TARGET,
        pid = std::process::id(),
        event = event,
        detail = %detail,
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
    })
}

fn build_trace_guard(config: TraceConfig) -> Result<WorkerGuard, String> {
    let (writer, guard) = non_blocking_writer(config.destination)?;
    let env_filter = env_filter_from_filter_value(&config.filter);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_span_list(true)
        .with_span_events(config.span_mode.fmt_span())
        .with_writer(writer);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .try_init()
        .map_err(|error| format!("failed to install tracing subscriber: {error}"))?;

    Ok(guard)
}

fn non_blocking_writer(
    destination: TraceDestination,
) -> Result<(tracing_appender::non_blocking::NonBlocking, WorkerGuard), String> {
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
            Ok(tracing_appender::non_blocking(appender))
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
            Ok(tracing_appender::non_blocking(file))
        }
    }
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
    use super::{
        AKRA_EVENT_TARGET, TraceDestination, TraceSpanMode, concise_trace_filter,
        full_trace_filter, planning_trace_filter, trace_filter_from_rust_log_value,
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

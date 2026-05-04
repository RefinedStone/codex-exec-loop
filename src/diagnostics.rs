/*
 * diagnostics는 UI copy나 domain state가 아닌 개발/운영 관측용 side channel이다.
 * TUI stdout/stderr를 건드리면 terminal protocol과 app-server stream을 오염시킬 수 있으므로,
 * raw event log는 release에서는 명시적인 env var가 있을 때만, debug Akra binary 실행에서는
 * workspace-local runtime 파일에 JSON Lines로 append한다.
 */
#[cfg(debug_assertions)]
fn debug_executable_allows_default_diagnostics(executable_path: Option<&std::path::Path>) -> bool {
    let Some(executable_path) = executable_path else {
        return false;
    };
    if executable_is_target_debug_deps_harness(executable_path) {
        return false;
    }

    let Some(file_name) = executable_path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
    else {
        return false;
    };
    let binary_name = file_name.strip_suffix(".exe").unwrap_or(file_name);
    matches!(
        binary_name,
        "codex-exec-loop-native" | "akra" | "akra-admin" | "akra-telegram"
    )
}

#[cfg(debug_assertions)]
fn executable_is_target_debug_deps_harness(executable_path: &std::path::Path) -> bool {
    let components = executable_path
        .components()
        .map(|component| component.as_os_str())
        .collect::<Vec<_>>();
    components.windows(3).any(|window| {
        window[0] == std::ffi::OsStr::new("target")
            && window[1] == std::ffi::OsStr::new("debug")
            && window[2] == std::ffi::OsStr::new("deps")
    })
}

pub mod raw_event_log {
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    use serde_json::{Value, json};

    static RAW_LOG_FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();

    pub fn is_enabled() -> bool {
        RAW_LOG_FILE.get_or_init(open_raw_log_file).is_some()
    }

    pub fn emit_lazy<F>(event: &str, detail: F)
    where
        F: FnOnce() -> Value,
    {
        if !is_enabled() {
            return;
        }
        emit(event, detail());
    }

    /*
     * AKRA_RAW_LOG가 설정된 프로세스나 debug Akra binary에서만 한 줄 JSON event를 쓴다.
     * 실패해도 product flow를 방해하지 않는 best-effort 경로이며, prompt/body 원문은 caller가 명시적으로 넣지 않는 한 기록하지 않는다.
     */
    pub fn emit(event: &str, detail: Value) {
        let Some(file) = RAW_LOG_FILE.get_or_init(open_raw_log_file).as_ref() else {
            return;
        };
        let entry = json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "pid": std::process::id(),
            "event": event,
            "detail": detail,
        });
        if let Ok(mut file) = file.lock() {
            let _ = writeln!(file, "{entry}");
        }
    }

    fn open_raw_log_file() -> Option<Mutex<File>> {
        let path = raw_log_path()?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(Mutex::new)
    }

    fn raw_log_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("AKRA_RAW_LOG") {
            let path = PathBuf::from(path);
            if path.as_os_str().is_empty() {
                return None;
            }
            return Some(path);
        }
        default_debug_raw_log_path()
    }

    #[cfg(debug_assertions)]
    fn default_debug_raw_log_path() -> Option<PathBuf> {
        if !super::debug_executable_allows_default_diagnostics(
            std::env::current_exe().ok().as_deref(),
        ) {
            return None;
        }
        Some(
            std::env::current_dir()
                .ok()?
                .join(".codex-exec-loop")
                .join("runtime")
                .join("akra-raw.jsonl"),
        )
    }

    #[cfg(not(debug_assertions))]
    fn default_debug_raw_log_path() -> Option<PathBuf> {
        None
    }

    #[cfg(test)]
    mod tests {
        use std::path::Path;

        use super::super::debug_executable_allows_default_diagnostics;

        #[test]
        fn debug_default_raw_log_is_disabled_for_test_harness_binaries() {
            assert!(!debug_executable_allows_default_diagnostics(Some(
                Path::new("/repo/target/debug/deps/integration_test-abc123",)
            )));
        }

        #[test]
        fn debug_default_raw_log_is_enabled_for_cargo_run_binaries() {
            assert!(debug_executable_allows_default_diagnostics(Some(
                Path::new("/repo/target/debug/codex-exec-loop-native",)
            )));
            assert!(debug_executable_allows_default_diagnostics(Some(
                Path::new("/repo/target/debug/akra",)
            )));
        }
    }
}

pub mod trace_event_log {
    use std::fs::OpenOptions;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use tracing_appender::non_blocking::WorkerGuard;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::format::FmtSpan;
    use tracing_subscriber::prelude::*;

    static TRACE_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

    struct TraceConfig {
        filter: String,
        path: PathBuf,
    }

    pub fn init_from_env() {
        if TRACE_GUARD.get().is_some() {
            return;
        }
        let Some(config) = trace_config_from_env() else {
            return;
        };
        match build_trace_guard(config) {
            Ok(guard) => {
                let _ = TRACE_GUARD.set(guard);
            }
            Err(error) => {
                eprintln!("akra trace initialization failed: {error}");
            }
        }
    }

    fn trace_config_from_env() -> Option<TraceConfig> {
        Some(TraceConfig {
            filter: trace_filter_from_env()?,
            path: trace_log_path()?,
        })
    }

    fn build_trace_guard(config: TraceConfig) -> Result<WorkerGuard, String> {
        if let Some(parent) = config.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create trace log directory `{}`: {error}",
                    parent.display()
                )
            })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&config.path)
            .map_err(|error| {
                format!(
                    "failed to open trace log file `{}`: {error}",
                    config.path.display()
                )
            })?;
        let (writer, guard) = tracing_appender::non_blocking(file);
        let env_filter = EnvFilter::try_new(config.filter)
            .unwrap_or_else(|_| EnvFilter::new(default_trace_filter()));
        let fmt_layer = tracing_subscriber::fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .with_span_events(FmtSpan::FULL)
            .with_writer(writer);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .try_init()
            .map_err(|error| format!("failed to install tracing subscriber: {error}"))?;

        Ok(guard)
    }

    fn trace_filter_from_env() -> Option<String> {
        match std::env::var("AKRA_TRACE") {
            Ok(value) => trace_filter_from_value(&value),
            Err(_) => default_debug_trace_filter(),
        }
    }

    fn trace_filter_from_value(value: &str) -> Option<String> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("0") {
            return None;
        }
        if matches!(
            trimmed.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ) {
            return Some(default_trace_filter());
        }
        Some(trimmed.to_string())
    }

    fn default_trace_filter() -> String {
        "trace".to_string()
    }

    #[cfg(debug_assertions)]
    fn default_debug_trace_filter() -> Option<String> {
        if !super::debug_executable_allows_default_diagnostics(
            std::env::current_exe().ok().as_deref(),
        ) {
            return None;
        }
        Some(default_trace_filter())
    }

    #[cfg(not(debug_assertions))]
    fn default_debug_trace_filter() -> Option<String> {
        None
    }

    fn trace_log_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("AKRA_TRACE_FILE") {
            let path = PathBuf::from(path);
            if path.as_os_str().is_empty() {
                return None;
            }
            return Some(path);
        }
        Some(
            std::env::current_dir()
                .ok()?
                .join(".codex-exec-loop")
                .join("runtime")
                .join("akra-trace.jsonl"),
        )
    }

    #[cfg(test)]
    mod tests {
        use std::path::Path;

        use super::super::debug_executable_allows_default_diagnostics;
        use super::trace_filter_from_value;

        #[test]
        fn trace_filter_value_accepts_on_off_and_directives() {
            assert_eq!(trace_filter_from_value("1"), Some("trace".to_string()));
            assert_eq!(trace_filter_from_value("0"), None);
            assert_eq!(
                trace_filter_from_value("codex_exec_loop_native::adapter=debug"),
                Some("codex_exec_loop_native::adapter=debug".to_string())
            );
        }

        #[test]
        fn debug_default_trace_is_disabled_for_test_harness_binaries() {
            assert!(!debug_executable_allows_default_diagnostics(Some(
                Path::new("/repo/target/debug/deps/integration_test-abc123",)
            )));
            assert!(!debug_executable_allows_default_diagnostics(Some(
                Path::new("/repo/target/debug/deps/akra-abc123",)
            )));
        }

        #[test]
        fn debug_default_trace_is_enabled_for_cargo_run_binaries() {
            assert!(debug_executable_allows_default_diagnostics(Some(
                Path::new("/repo/target/debug/codex-exec-loop-native",)
            )));
            assert!(debug_executable_allows_default_diagnostics(Some(
                Path::new("/repo/target/debug/akra",)
            )));
            assert!(debug_executable_allows_default_diagnostics(Some(
                Path::new("/repo/target/debug/akra-admin",)
            )));
            assert!(debug_executable_allows_default_diagnostics(Some(
                Path::new("/home/user/deps/repo/target/debug/akra",)
            )));
        }
    }
}

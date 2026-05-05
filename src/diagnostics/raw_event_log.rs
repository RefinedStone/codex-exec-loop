use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};

use super::trace_event_log;

static RAW_LOG_FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();

pub fn is_enabled() -> bool {
    RAW_LOG_FILE.get_or_init(open_raw_log_file).is_some()
}

pub fn emit_lazy<F>(event: &str, detail: F)
where
    F: FnOnce() -> Value,
{
    if !is_enabled() && !trace_event_log::akra_event_enabled() {
        return;
    }
    emit(event, detail());
}

/*
 * AKRA_RAW_LOG가 설정된 프로세스나 debug Akra binary에서만 한 줄 JSON event를 쓴다.
 * 실패해도 product flow를 방해하지 않는 best-effort 경로이며, prompt/body 원문은 caller가 명시적으로 넣지 않는 한 기록하지 않는다.
 */
pub fn emit(event: &str, detail: Value) {
    emit_raw_event(event, &detail);
    trace_event_log::emit_akra_event(event, &detail);
}

fn emit_raw_event(event: &str, detail: &Value) {
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
    if !super::executable::debug_executable_allows_default_diagnostics(
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

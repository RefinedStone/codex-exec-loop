/*
 * diagnostics는 UI copy나 domain state가 아닌 개발/운영 관측용 side channel이다.
 * TUI stdout/stderr를 건드리면 terminal protocol과 app-server stream을 오염시킬 수 있으므로,
 * raw event log는 명시적인 env var가 있을 때만 파일에 JSON Lines로 append한다.
 */
pub mod raw_event_log {
    use std::fs::{File, OpenOptions};
    use std::io::Write;
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
     * AKRA_RAW_LOG가 설정된 프로세스에서만 한 줄 JSON event를 쓴다.
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
        let path = std::env::var_os("AKRA_RAW_LOG")?;
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(Mutex::new)
    }
}

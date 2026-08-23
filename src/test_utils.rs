use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;

pub(crate) fn platform_safe_temp_dir() -> PathBuf {
    let temp_dir = std::env::temp_dir();
    #[cfg(target_os = "macos")]
    {
        // macOS exposes /var as a symlink to /private/var; NOFOLLOW fixtures need
        // the physical path. Other platforms must retain their native spelling:
        // Windows canonicalization adds a \\?\ prefix that Git for Windows rejects.
        std::fs::canonicalize(&temp_dir).unwrap_or(temp_dir)
    }
    #[cfg(not(target_os = "macos"))]
    temp_dir
}

pub(crate) fn process_environment_mutex() -> &'static Mutex<()> {
    static PROCESS_ENVIRONMENT_MUTEX: Mutex<()> = Mutex::new(());
    &PROCESS_ENVIRONMENT_MUTEX
}

pub(crate) fn json_payload_contains(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(value) => value.contains(needle),
        Value::Array(values) => values
            .iter()
            .any(|value| json_payload_contains(value, needle)),
        Value::Object(values) => values
            .values()
            .any(|value| json_payload_contains(value, needle)),
        _ => false,
    }
}

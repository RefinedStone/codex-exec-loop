use serde_json::Value;

use super::trace_event_log;

pub fn emit_lazy<F>(event: &str, detail: F)
where
    F: FnOnce() -> Value,
{
    if !trace_event_log::akra_event_enabled() {
        return;
    }
    trace_event_log::emit_akra_event(event, &detail());
}

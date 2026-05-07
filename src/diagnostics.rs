/*
 * diagnostics는 UI copy나 domain state가 아닌 개발/운영 관측용 side channel이다.
 * TUI stdout/stderr를 건드리면 terminal protocol과 app-server stream을 오염시킬 수 있으므로,
 * diagnostics는 tracing JSONL 파일이나 선택적 tokio-console layer로만 나간다.
 */
mod executable;

use std::fmt;

pub mod event_log;
pub mod trace_event_log;

pub struct DiagnosticsGuards {
    _trace_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

impl Drop for DiagnosticsGuards {
    fn drop(&mut self) {
        let dropped = dropped_log_lines();
        if dropped.total() == 0 {
            return;
        }
        tracing::warn!(
            trace_dropped_lines = dropped.trace,
            total_dropped_lines = dropped.total(),
            "akra_diagnostics_dropped_log_lines"
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DroppedLogLines {
    pub trace: usize,
}

impl DroppedLogLines {
    pub fn total(self) -> usize {
        self.trace
    }
}

pub fn init_from_env() -> DiagnosticsGuards {
    let trace_guard = trace_event_log::init_from_env();
    DiagnosticsGuards {
        _trace_guard: trace_guard,
    }
}

pub fn dropped_log_lines() -> DroppedLogLines {
    DroppedLogLines {
        trace: trace_event_log::dropped_lines(),
    }
}

pub struct LazyPayload<F>(pub F);

impl<F, T> fmt::Display for LazyPayload<F>
where
    F: Fn() -> T,
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", (self.0)())
    }
}

#[macro_export]
macro_rules! akra_event {
    ($level:expr, $message:literal $(, $key:ident = $value:expr)* $(,)?) => {{
        if tracing::enabled!(
            target: "codex_exec_loop_native::diagnostics::akra_event",
            $level
        ) {
            tracing::event!(
                target: "codex_exec_loop_native::diagnostics::akra_event",
                $level,
                pid = std::process::id(),
                event = $message,
                detail = tracing::field::display($crate::diagnostics::LazyPayload(|| {
                    serde_json::json!({
                        $( stringify!($key): &$value, )*
                    })
                })),
                "akra_event"
            );
        }
    }};
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{DroppedLogLines, LazyPayload};

    #[test]
    fn dropped_log_lines_total_saturates_instead_of_wrapping() {
        let snapshot = DroppedLogLines { trace: usize::MAX };

        assert_eq!(snapshot.total(), usize::MAX);
    }

    #[test]
    fn akra_event_macro_does_not_evaluate_payload_when_level_is_disabled() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);

        crate::akra_event!(
            tracing::Level::DEBUG,
            "disabled_diagnostic_payload",
            payload = {
                CALLS.fetch_add(1, Ordering::SeqCst);
                "expensive"
            },
        );

        assert_eq!(CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn lazy_payload_evaluates_only_when_formatted() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        let payload = LazyPayload(|| {
            CALLS.fetch_add(1, Ordering::SeqCst);
            "formatted"
        });

        assert_eq!(CALLS.load(Ordering::SeqCst), 0);
        assert_eq!(payload.to_string(), "formatted");
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }
}

/*
 * diagnostics는 UI copy나 domain state가 아닌 개발/운영 관측용 side channel이다.
 * TUI stdout/stderr를 건드리면 terminal protocol과 app-server stream을 오염시킬 수 있으므로,
 * raw event log는 release에서는 명시적인 env var가 있을 때만, debug Akra binary 실행에서는
 * workspace-local runtime 파일에 JSON Lines로 append한다.
 */
mod executable;

pub mod raw_event_log;
pub mod trace_event_log;

pub struct DiagnosticsGuards {
    _trace_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
    _raw_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

impl Drop for DiagnosticsGuards {
    fn drop(&mut self) {
        let dropped = dropped_log_lines();
        if dropped.total() == 0 {
            return;
        }
        tracing::warn!(
            trace_dropped_lines = dropped.trace,
            raw_dropped_lines = dropped.raw,
            total_dropped_lines = dropped.total(),
            "akra_diagnostics_dropped_log_lines"
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DroppedLogLines {
    pub trace: usize,
    pub raw: usize,
}

impl DroppedLogLines {
    pub fn total(self) -> usize {
        self.trace.saturating_add(self.raw)
    }
}

pub fn init_from_env() -> DiagnosticsGuards {
    let trace_guard = trace_event_log::init_from_env();
    let raw_guard = raw_event_log::init_from_env();
    DiagnosticsGuards {
        _trace_guard: trace_guard,
        _raw_guard: raw_guard,
    }
}

pub fn dropped_log_lines() -> DroppedLogLines {
    DroppedLogLines {
        trace: trace_event_log::dropped_lines(),
        raw: raw_event_log::dropped_lines(),
    }
}

#[macro_export]
macro_rules! akra_event {
    ($level:expr, $message:literal $(, $key:ident = $value:expr)* $(,)?) => {{
        if tracing::enabled!($level) {
            tracing::event!(
                $level,
                $( $key = tracing::field::debug(&$value), )*
                $message
            );
        }
    }};
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::DroppedLogLines;

    #[test]
    fn dropped_log_lines_total_saturates_instead_of_wrapping() {
        let snapshot = DroppedLogLines {
            trace: usize::MAX,
            raw: 1,
        };

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
}

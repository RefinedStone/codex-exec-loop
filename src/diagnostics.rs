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

pub fn init_from_env() -> DiagnosticsGuards {
    let trace_guard = trace_event_log::init_from_env();
    let raw_guard = raw_event_log::init_from_env();
    DiagnosticsGuards {
        _trace_guard: trace_guard,
        _raw_guard: raw_guard,
    }
}

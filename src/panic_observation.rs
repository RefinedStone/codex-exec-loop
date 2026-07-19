use std::any::Any;
use std::cell::Cell;
use std::io::Write;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

pub(crate) const REDACTED_WORKER_PANIC_MESSAGE: &str = "akra worker panicked (payload redacted)";

thread_local! {
    static REDACT_WORKER_PANIC: Cell<bool> = const { Cell::new(false) };
}

static INSTALL_REDACTING_HOOK: Once = Once::new();

pub(crate) fn catch_redacted_worker_unwind<F, T>(
    operation: F,
) -> Result<T, Box<dyn Any + Send + 'static>>
where
    F: FnOnce() -> T,
{
    install_redacting_hook();
    let _guard = RedactedWorkerPanicGuard::enter();
    panic::catch_unwind(AssertUnwindSafe(operation))
}

fn install_redacting_hook() {
    INSTALL_REDACTING_HOOK.call_once(|| {
        let delegated_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            let redact_payload = REDACT_WORKER_PANIC.try_with(Cell::get).unwrap_or(false);
            if redact_payload {
                let _ = writeln!(std::io::stderr().lock(), "{REDACTED_WORKER_PANIC_MESSAGE}");
            } else {
                delegated_hook(panic_info);
            }
        }));
    });
}

struct RedactedWorkerPanicGuard {
    previous: bool,
}

impl RedactedWorkerPanicGuard {
    fn enter() -> Self {
        let previous = REDACT_WORKER_PANIC.with(|redact| redact.replace(true));
        Self { previous }
    }
}

impl Drop for RedactedWorkerPanicGuard {
    fn drop(&mut self) {
        let _ = REDACT_WORKER_PANIC.try_with(|redact| redact.set(self.previous));
    }
}

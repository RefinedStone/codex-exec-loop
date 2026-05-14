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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    use super::emit_lazy;
    use crate::diagnostics::trace_event_log::AKRA_EVENT_TARGET;

    #[test]
    fn emit_lazy_does_not_evaluate_detail_when_akra_event_target_is_disabled() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);

        emit_lazy("disabled_event", || {
            CALLS.fetch_add(1, Ordering::SeqCst);
            json!({ "expensive": true })
        });

        assert_eq!(CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn emit_lazy_evaluates_detail_once_when_akra_event_target_is_enabled() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        let capture = CaptureWriter::default();
        let subscriber = tracing_subscriber::registry()
            .with(EnvFilter::new(format!("{AKRA_EVENT_TARGET}=debug")))
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(capture.clone()),
            );

        tracing::subscriber::with_default(subscriber, || {
            emit_lazy("enabled_event", || {
                CALLS.fetch_add(1, Ordering::SeqCst);
                json!({ "answer": 42 })
            });
        });

        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
        let joined = capture.lines().join("\n");
        assert!(joined.contains("enabled_event"));
        assert!(joined.contains(r#"\"answer\":42"#));
    }

    #[derive(Clone, Default)]
    struct CaptureWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl CaptureWriter {
        fn lines(&self) -> Vec<String> {
            let bytes = self
                .bytes
                .lock()
                .expect("capture lock should not be poisoned");
            String::from_utf8(bytes.clone())
                .expect("captured diagnostics should be UTF-8")
                .lines()
                .map(str::to_string)
                .collect()
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureSink;

        fn make_writer(&'a self) -> Self::Writer {
            CaptureSink {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }

    struct CaptureSink {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl std::io::Write for CaptureSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes
                .lock()
                .expect("capture lock should not be poisoned")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}

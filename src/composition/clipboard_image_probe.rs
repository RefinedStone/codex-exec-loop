use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::application::port::outbound::clipboard_image_probe_port::ClipboardImageProbePort;
use crate::composition::core_effect_worker::spawn_worker_with_panic_fallback;
use crate::domain::clipboard_image::ClipboardImageProbeOutcome;

/*
 * ClipboardImageProbeTrigger is the composition-owned worker seam between the
 * TUI and the outbound clipboard probe port. Composition owns the worker and
 * the in-flight guard so duplicate Ctrl+V presses never stack osascript /
 * PowerShell workers, while the inbound adapter only handles an opaque trigger
 * plus a completion callback.
 */
pub(crate) trait ClipboardImageProbeTrigger: Send + Sync {
    fn request_probe(
        &self,
        on_complete: Box<dyn FnOnce(ClipboardImageProbeOutcome) + Send>,
    ) -> bool;
}

pub(crate) struct CompositionClipboardImageProbeTrigger {
    port: Arc<dyn ClipboardImageProbePort>,
    in_flight: Arc<AtomicBool>,
}

impl CompositionClipboardImageProbeTrigger {
    pub(crate) fn new_shared(
        port: Arc<dyn ClipboardImageProbePort>,
    ) -> Arc<dyn ClipboardImageProbeTrigger> {
        Arc::new(Self {
            port,
            in_flight: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl ClipboardImageProbeTrigger for CompositionClipboardImageProbeTrigger {
    fn request_probe(
        &self,
        on_complete: Box<dyn FnOnce(ClipboardImageProbeOutcome) + Send>,
    ) -> bool {
        /*
         * Swap guards concurrent probes; a stale `true` can only come from a
         * still-running worker whose completion callback has not landed yet.
         * Panic recovery releases the guard so one poisoned probe cannot wedge
         * future attachments.
         */
        if self.in_flight.swap(true, Ordering::SeqCst) {
            return false;
        }
        let port = Arc::clone(&self.port);
        let completed_flag = Arc::clone(&self.in_flight);
        let panicked_flag = Arc::clone(&self.in_flight);
        spawn_worker_with_panic_fallback(
            move || {
                let outcome = port.probe_clipboard_image();
                on_complete(outcome);
                completed_flag.store(false, Ordering::SeqCst);
            },
            move || panicked_flag.store(false, Ordering::SeqCst),
        );
        true
    }
}

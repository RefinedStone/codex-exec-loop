//! Bounded 24-hour result retention. Cleanup always uses the original job identity.
use super::*;
use std::time::Duration;
pub(super) const RESULT_RETENTION_MS: u64 = 24 * 60 * 60 * 1000;
impl ExtensionService {
    pub(super) fn spawn_retention_worker(self: &Arc<Self>) -> Result<()> {
        let weak = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("akra-extension-retention".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(60));
                    let Some(host) = weak.upgrade() else {
                        break;
                    };
                    if host.lock().stopped {
                        break;
                    }
                    if let Err(error) = host.reap_expired() {
                        tracing::warn!(error=%error,"extension result cleanup failed");
                    }
                }
            })?;
        Ok(())
    }
    pub(super) fn reap_expired(&self) -> Result<()> {
        let now = now_ms();
        let context = self.context(
            "retention".into(),
            (self.scope == ExtensionScope::Repository).then(|| self.host_id.clone()),
        );
        let expired = self
            .lock()
            .durable
            .jobs
            .values()
            .filter(|job| {
                !job.state.active()
                    && job.cleanup != ExtensionCleanupState::Complete
                    && job.result_expires_at_ms <= now
            })
            .cloned()
            .collect::<Vec<_>>();
        for job in expired {
            if self.registry.get(&job.extension).is_ok() {
                self.cleanup_job(&context, &job)?;
            }
        }
        let mut state = self.lock();
        let mut next = state.durable.clone();
        next.receipts.retain(|_, receipt| {
            receipt.recorded_at_ms.saturating_add(RESULT_RETENTION_MS) > now
                || next
                    .jobs
                    .get(&receipt.job_id)
                    .is_some_and(|job| job.state.active())
        });
        if next.receipts.len() != state.durable.receipts.len() {
            self.persist(&mut state, next)?;
        }
        Ok(())
    }
}

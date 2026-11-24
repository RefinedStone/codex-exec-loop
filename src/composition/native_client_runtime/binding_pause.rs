//! Durable request acceptance and completion have separate lifetimes. The kernel never waits
//! for an active provider call here: only bounded local recovery and outbox pages are polled.
use super::*;
use crate::application::service::issue_binding_admission::IssueBindingAdmission;
use crate::application::service::issue_binding_pause::IssueBindingPauseService;

const OUTBOX_BATCH: usize = 64;

impl NativeClientRuntime {
    /// The repository owner recovered this exact admission before constructing either graph.
    pub(crate) fn attach_issue_binding_pause(
        &mut self,
        service: Arc<IssueBindingPauseService>,
        admission: Arc<IssueBindingAdmission>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.binding_pause.is_none() && self.binding_admission.is_none(),
            "binding pause is already attached"
        );
        let waker = self.host_waker();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || waker.wake());
        admission.set_waker(wake.clone());
        service.set_waker(wake);
        self.binding_pause = Some(service);
        self.binding_admission = Some(admission);
        self.drain_binding_pause_outbox(Instant::now());
        Ok(())
    }

    pub(super) fn poll_binding_pause(&mut self, now: Instant) {
        if let Some(service) = &self.binding_pause
            && service
                .next_deadline()
                .is_some_and(|deadline| deadline <= now)
        {
            match service.poll() {
                Ok(true) => {
                    self.published_issue_change = true;
                    if let Some(observer) = &mut self.issue_observer {
                        observer.request_sync(now);
                    }
                }
                Ok(false) => {}
                Err(_) => tracing::warn!("binding pause recovery remains pending"),
            }
            // A poll may commit its command and then fail while verifying the receipt.
            // Drain the durable outbox even when poll cannot report that completion.
            self.drain_binding_pause_outbox(now);
        }
        if self
            .binding_pause_outbox_deadline
            .is_some_and(|deadline| deadline <= now)
        {
            self.drain_binding_pause_outbox(now);
        }
    }

    pub(super) fn drain_binding_pause_outbox(&mut self, now: Instant) {
        let (Some(service), Some((log, workspace))) =
            (self.binding_pause.clone(), self.kernel_event_log.clone())
        else {
            return;
        };
        // All recorder enqueue and this append run on the same kernel thread. A completed
        // async batch is required before reserving a later sequence for the direct append.
        if self
            .kernel_events
            .as_ref()
            .is_some_and(|recorder| !recorder.is_caught_up())
        {
            self.binding_pause_outbox_deadline = now.checked_add(Duration::from_secs(1));
            return;
        }
        let result = (|| -> anyhow::Result<bool> {
            let pending = service.pending_events(OUTBOX_BATCH)?;
            let full = pending.len() >= OUTBOX_BATCH;
            for event in pending.into_iter().take(OUTBOX_BATCH) {
                let record_event = KernelEvent::issue_binding_pause(&event);
                if !log.has_issue_binding_pause_event(&workspace, &record_event)? {
                    self.last_kernel_sequence = self
                        .last_kernel_sequence
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("kernel event sequence exhausted"))?;
                    let record = RecordedKernelEvent {
                        sequence: self.last_kernel_sequence,
                        recorded_at: chrono::Utc::now()
                            .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
                        event: record_event,
                    };
                    log.append_kernel_events(&workspace, std::slice::from_ref(&record))?;
                    self.recent_kernel_records.push_back(record.clone());
                    self.published_kernel_records.push(record);
                    self.published_issue_change = true;
                    while self.recent_kernel_records.len() > RECENT_KERNEL_RECORDS {
                        self.recent_kernel_records.pop_front();
                    }
                }
                // ACK only the exact immutable receipt after its append is known durable.
                service.acknowledge_event(&event)?;
            }
            Ok(full)
        })();
        self.binding_pause_outbox_deadline = match result {
            Ok(true) => Some(now),
            Ok(false) => None,
            Err(_) => {
                tracing::warn!("binding pause completion remains in the durable outbox");
                now.checked_add(Duration::from_secs(1))
            }
        };
    }
}

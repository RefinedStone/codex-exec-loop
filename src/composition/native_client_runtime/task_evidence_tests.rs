use super::*;
use anyhow::Result;
use std::sync::{Mutex, mpsc};

struct SlowLog {
    records: Mutex<Vec<RecordedKernelEvent>>,
    release: Mutex<Option<mpsc::Receiver<()>>>,
    started: mpsc::SyncSender<()>,
}

impl KernelEventLogPort for SlowLog {
    fn append_kernel_events(&self, _: &str, records: &[RecordedKernelEvent]) -> Result<()> {
        if let Some(release) = self.release.lock().unwrap().take() {
            self.started.send(())?;
            release.recv_timeout(Duration::from_secs(5))?;
        }
        self.records.lock().unwrap().extend_from_slice(records);
        Ok(())
    }

    fn load_kernel_events(&self, _: &str, kinds: &[&str]) -> Result<Vec<KernelEvent>> {
        Ok(self
            .records
            .lock()
            .unwrap()
            .iter()
            .filter(|record| kinds.contains(&record.event.kind()))
            .map(|record| record.event.clone())
            .collect())
    }
}

#[test]
fn slow_recorder_cannot_publish_a_higher_evidence_sequence_before_older_kernel_records() {
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::channel();
    let log = Arc::new(SlowLog {
        records: Mutex::new(Vec::new()),
        release: Mutex::new(Some(release_rx)),
        started: started_tx,
    });
    let (mut runtime, _) = tests::recording_runtime_for_test();
    runtime.attach_kernel_event_log(log.clone(), "/workspace");
    let _ = runtime.dispatch_client_event(NativeClientEvent::core(CoreInput::Command(
        AppCommand::SetAutoFollowMaxTurns { value: 2 },
    )));
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!runtime.kernel_events.as_ref().unwrap().is_caught_up());
    let (service, _, _) = crate::composition::task_evidence_outbox::tests::fixture(
        crate::domain::kernel_event::sample_task_evidence_events().to_vec(),
    );
    let began = Instant::now();
    runtime.attach_task_evidence(service);
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "outbox deferral must not block on the writer"
    );
    assert!(
        log.records.lock().unwrap().is_empty(),
        "higher fact sequences must not pass the queued record"
    );
    let retry = runtime
        .task_evidence_outbox_deadline
        .expect("pending writer schedules a bounded retry");
    release_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !runtime.kernel_events.as_ref().unwrap().is_caught_up() {
        assert!(
            Instant::now() < deadline,
            "writer did not finish its accepted batch"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    runtime.run_host_cycle(retry, Duration::ZERO, 32);
    let records = log.records.lock().unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].event.kind(), "auto_follow_configured");
    assert_eq!(records[1].event.kind(), "criterion_evidence_recorded");
    assert_eq!(records[2].event.kind(), "criterion_waived");
    assert_eq!(
        records
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

use super::*;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
use crate::application::port::outbound::task_evidence_repository_port::*;
use crate::domain::kernel_event::{sample_task_evidence_events, sample_task_verification_event};
use crate::domain::planning::evidence::*;
use anyhow::Result;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

pub(crate) struct Outbox {
    events: Mutex<Vec<TaskEvidenceEvent>>,
    reads: AtomicUsize,
    acknowledgements: AtomicUsize,
    fail_ack_once: AtomicBool,
}

impl crate::application::port::outbound::task_verification_repository_port::TaskVerificationRepositoryPort for Outbox {}

impl TaskEvidenceRepositoryPort for Outbox {
    fn load_current_snapshot(
        &self,
        _: &str,
        _: &TaskEvidenceSnapshotQuery,
    ) -> Result<TaskEvidenceStoredSnapshot> {
        unreachable!()
    }
    fn load_attempt(&self, _: &str, _: &str) -> Result<Option<StoredTaskEvidenceAttempt>> {
        unreachable!()
    }
    fn prepare_attempt(
        &self,
        _: &str,
        _: &TaskEvidencePrepareCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceAttempt>> {
        unreachable!()
    }
    fn record_evidence(
        &self,
        _: &str,
        _: &TaskEvidenceRecordCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceReceipt>> {
        unreachable!()
    }
    fn waive_criterion(
        &self,
        _: &str,
        _: &TaskCriterionWaiverCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskCriterionWaiverReceipt>> {
        unreachable!()
    }
    fn pending_events(&self, _: &str, limit: usize) -> Result<Vec<TaskEvidenceEvent>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .events
            .lock()
            .unwrap()
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }
    fn acknowledge_event(&self, _: &str, id: &str) -> Result<()> {
        self.acknowledgements.fetch_add(1, Ordering::SeqCst);
        if self.fail_ack_once.swap(false, Ordering::SeqCst) {
            anyhow::bail!("injected ACK failure");
        }
        self.events
            .lock()
            .unwrap()
            .retain(|event| event.event_id() != id);
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct Log {
    records: Mutex<Vec<RecordedKernelEvent>>,
    fail_append_once: AtomicBool,
}

impl KernelEventLogPort for Log {
    fn append_kernel_events(&self, _: &str, records: &[RecordedKernelEvent]) -> Result<()> {
        if self.fail_append_once.swap(false, Ordering::SeqCst) {
            anyhow::bail!("injected append failure");
        }
        let mut stored = self.records.lock().unwrap();
        for record in records {
            anyhow::ensure!(
                !stored.iter().any(|prior| prior.sequence == record.sequence),
                "sequence collision"
            );
            stored.push(record.clone());
        }
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
    fn last_kernel_event_sequence(&self, _: &str) -> Result<i64> {
        Ok(self
            .records
            .lock()
            .unwrap()
            .iter()
            .map(|record| record.sequence)
            .max()
            .unwrap_or(0))
    }
}

pub(crate) fn fixture(
    events: Vec<TaskEvidenceEvent>,
) -> (Arc<TaskEvidenceService>, Arc<Outbox>, Arc<Log>) {
    let outbox = Arc::new(Outbox {
        events: Mutex::new(events),
        reads: AtomicUsize::new(0),
        acknowledgements: AtomicUsize::new(0),
        fail_ack_once: AtomicBool::new(false),
    });
    let service = Arc::new(TaskEvidenceService::new(
        "/workspace".into(),
        outbox.clone(),
        crate::composition::task_evidence::artifacts(),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
        Arc::new(NoopPlanningAuthorityPort::default()),
    ));
    (service, outbox, Arc::new(Log::default()))
}

#[test]
fn append_failure_preserves_pending_events_and_ack_loss_does_not_publish_twice() {
    let (service, outbox, log) = fixture(sample_task_evidence_events().to_vec());
    log.fail_append_once.store(true, Ordering::SeqCst);
    let mut sequence = 8;
    let failed = drain(&service, log.as_ref(), "/workspace", &mut sequence);
    assert!(failed.retry);
    assert!(failed.records.is_empty());
    assert_eq!(outbox.acknowledgements.load(Ordering::SeqCst), 0);
    assert_eq!(outbox.events.lock().unwrap().len(), 2);
    outbox.fail_ack_once.store(true, Ordering::SeqCst);
    let ack_lost = drain(&service, log.as_ref(), "/workspace", &mut sequence);
    assert!(ack_lost.retry);
    assert_eq!(ack_lost.records.len(), 1);
    let recovered = drain(&service, log.as_ref(), "/workspace", &mut sequence);
    assert!(!recovered.retry);
    assert_eq!(
        recovered.records.len(),
        1,
        "the first committed event is not published twice"
    );
    assert!(outbox.events.lock().unwrap().is_empty());
    let stored = log.records.lock().unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].sequence, 10);
    assert_eq!(stored[1].sequence, 11);
}

#[test]
fn a_reused_event_id_with_changed_payload_never_acknowledges_the_outbox() {
    let events = sample_task_evidence_events();
    let (service, outbox, log) = fixture(vec![events[0].clone()]);
    let mut changed = KernelEvent::task_evidence(&events[0]);
    if let KernelEvent::CriterionEvidenceRecorded {
        evidence_revision, ..
    } = &mut changed
    {
        *evidence_revision += 1;
    }
    log.records.lock().unwrap().push(RecordedKernelEvent {
        sequence: 1,
        recorded_at: "2026-11-23T00:00:00Z".into(),
        event: changed,
    });
    let mut sequence = 1;
    let result = drain(&service, log.as_ref(), "/workspace", &mut sequence);
    assert!(result.retry);
    assert!(result.records.is_empty());
    assert_eq!(outbox.acknowledgements.load(Ordering::SeqCst), 0);
    assert_eq!(sequence, 1);
}

#[test]
fn host_verification_ack_loss_retries_without_republishing_or_reusing_issue_identity() {
    let event = sample_task_verification_event();
    let (service, outbox, log) = fixture(vec![event.clone()]);
    outbox.fail_ack_once.store(true, Ordering::SeqCst);
    let mut sequence = 8;
    let first = drain(&service, log.as_ref(), "/workspace", &mut sequence);
    assert!(first.retry);
    assert_eq!(first.records.len(), 1);
    assert_eq!(first.records[0].event.kind(), "task_verification_finished");
    assert_eq!(
        first.records[0].event.task_evidence_event_id(),
        Some(event.event_id())
    );
    assert!(first.records[0].event.issue_work_event_id().is_none());
    assert_eq!(outbox.events.lock().unwrap().len(), 1);

    let recovered = drain(&service, log.as_ref(), "/workspace", &mut sequence);
    assert!(!recovered.retry);
    assert!(recovered.records.is_empty());
    assert!(outbox.events.lock().unwrap().is_empty());
    assert_eq!(log.records.lock().unwrap().len(), 1);
    assert_eq!(sequence, 9);
    assert_eq!(outbox.acknowledgements.load(Ordering::SeqCst), 2);
}

#[test]
fn host_verification_changed_execution_fact_cannot_ack_an_existing_event_id() {
    let event = sample_task_verification_event();
    let (service, outbox, log) = fixture(vec![event.clone()]);
    let mut changed = KernelEvent::task_evidence(&event);
    let KernelEvent::TaskVerificationFinished { verification, .. } = &mut changed else {
        panic!("expected a host execution fact");
    };
    let crate::domain::planning::verification::TaskVerificationResult::Executed { observation } =
        &mut verification.result
    else {
        panic!("expected an executed fact");
    };
    observation.outcome =
        crate::domain::planning::verification::TaskVerificationExecutionOutcome::Exited {
            exit_code: 1,
        };
    log.records.lock().unwrap().push(RecordedKernelEvent {
        sequence: 1,
        recorded_at: "2026-11-23T00:00:00Z".into(),
        event: changed,
    });
    let mut sequence = 1;
    let result = drain(&service, log.as_ref(), "/workspace", &mut sequence);
    assert!(result.retry);
    assert!(result.records.is_empty());
    assert_eq!(outbox.acknowledgements.load(Ordering::SeqCst), 0);
    assert_eq!(outbox.events.lock().unwrap().len(), 1);
    assert_eq!(sequence, 1);
}

#[test]
fn live_outbox_retries_at_one_second_then_stops_reading_when_acknowledged() {
    let (service, outbox, log) = fixture(sample_task_evidence_events().to_vec());
    log.fail_append_once.store(true, Ordering::SeqCst);
    let (mut runtime, turns) =
        crate::composition::native_client_runtime::tests::recording_runtime_for_test();
    runtime.attach_kernel_event_log(log.clone(), "/workspace");
    let before = Instant::now();
    runtime.attach_task_evidence(service);
    let after = Instant::now();
    let retry = runtime
        .next_deadline(before)
        .expect("append failure schedules a retry");
    assert!(retry >= before + Duration::from_secs(1));
    assert!(retry <= after + Duration::from_secs(1));
    assert!(runtime.has_remaining_work());
    let reads = outbox.reads.load(Ordering::SeqCst);
    runtime.run_host_cycle(retry - Duration::from_millis(1), Duration::ZERO, 32);
    assert_eq!(outbox.reads.load(Ordering::SeqCst), reads);
    runtime.run_host_cycle(retry, Duration::ZERO, 32);
    assert!(outbox.events.lock().unwrap().is_empty());
    assert_eq!(
        runtime
            .take_published_kernel_records()
            .iter()
            .filter(|record| record.event.task_evidence_event_id().is_some())
            .count(),
        2
    );
    assert_eq!(runtime.next_deadline(retry), None);
    assert!(!runtime.has_remaining_work());
    let reads = outbox.reads.load(Ordering::SeqCst);
    runtime.run_host_cycle(retry + Duration::from_secs(100), Duration::ZERO, 32);
    assert_eq!(
        outbox.reads.load(Ordering::SeqCst),
        reads,
        "successful drain must not create periodic database polling"
    );
    assert!(turns.lock().unwrap().is_empty());
}

#[test]
fn live_boot_recovery_continues_full_pages_with_bounded_work_per_cycle() {
    let source = sample_task_evidence_events()[0].clone();
    let events = (0..TASK_EVIDENCE_OUTBOX_BATCH + 1)
        .map(|index| {
            let mut event = source.clone();
            if let TaskEvidenceEvent::Recorded { receipt } = &mut event {
                receipt.event_id = format!("criterion-evidence:{index}");
                receipt.evidence_revision = index as u64 + 1;
            }
            event
        })
        .collect();
    let (service, outbox, log) = fixture(events);
    let (mut runtime, _) =
        crate::composition::native_client_runtime::tests::recording_runtime_for_test();
    runtime.attach_task_evidence(service);
    assert_eq!(
        outbox.reads.load(Ordering::SeqCst),
        0,
        "no recorder means no ACK"
    );
    runtime.attach_kernel_event_log(log.clone(), "/workspace");
    assert_eq!(outbox.events.lock().unwrap().len(), 1);
    let due = runtime
        .next_deadline(Instant::now())
        .expect("full batch continues next cycle");
    runtime.run_host_cycle(due, Duration::ZERO, 32);
    assert!(outbox.events.lock().unwrap().is_empty());
    assert_eq!(
        log.records.lock().unwrap().len(),
        TASK_EVIDENCE_OUTBOX_BATCH + 1
    );
    assert_eq!(runtime.next_deadline(due), None);
}

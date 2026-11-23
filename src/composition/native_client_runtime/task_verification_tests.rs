use super::*;
use std::sync::atomic::Ordering;

#[test]
fn verification_finish_and_outbox_retries_keep_the_kernel_alive_without_rerunning() {
    let fixture = crate::application::service::task_verification::tests::Fixture::new();
    let log = Arc::new(crate::composition::task_evidence_outbox::tests::Log::default());
    let (mut runtime, _) = tests::recording_runtime_for_test();
    runtime.attach_kernel_event_log(log.clone(), "/repo");
    runtime.attach_task_evidence(fixture.evidence.clone());
    runtime
        .attach_task_verification(fixture.service.clone())
        .unwrap();
    assert!(!runtime.has_remaining_work());
    let claim = fixture.start();
    assert!(runtime.has_remaining_work());
    let original = fixture.deliver_executed(&claim);
    fixture
        .repository
        .fail_finish_once
        .store(true, Ordering::SeqCst);
    fixture
        .repository
        .fail_ack_once
        .store(true, Ordering::SeqCst);

    let first = fixture.service.next_deadline().unwrap();
    runtime.run_host_cycle(first, Duration::ZERO, 32);
    assert!(fixture.service.has_pending());
    assert!(runtime.has_remaining_work());
    assert!(fixture.repository.outbox.lock().unwrap().is_empty());
    assert!(
        log.load_kernel_events("/repo", &["task_verification_finished"])
            .unwrap()
            .is_empty()
    );

    let retry = fixture.service.next_deadline().unwrap();
    runtime.run_host_cycle(retry, Duration::ZERO, 32);
    assert!(!fixture.service.has_pending());
    assert!(runtime.has_remaining_work());
    assert_eq!(fixture.repository.outbox.lock().unwrap().len(), 1);
    assert_eq!(
        log.load_kernel_events("/repo", &["task_verification_finished"])
            .unwrap()
            .len(),
        1
    );
    let ack_retry = runtime.task_evidence_outbox_deadline.unwrap();
    runtime.run_host_cycle(ack_retry, Duration::ZERO, 32);
    assert!(fixture.repository.outbox.lock().unwrap().is_empty());
    assert!(!runtime.has_remaining_work());
    assert_eq!(
        log.load_kernel_events("/repo", &["task_verification_finished"])
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fixture.repository.acknowledgements.load(Ordering::SeqCst),
        2
    );
    assert_eq!(fixture.spawn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.runner.prepares.load(Ordering::SeqCst), 0);
    let attempts = fixture.repository.finish_calls.lock().unwrap();
    assert_eq!(attempts.len(), 2);
    for attempt in attempts.iter() {
        assert_eq!(attempt.execution, original.execution);
        assert_eq!(attempt.result, original.result);
        assert_eq!(attempt.artifact, original.artifact);
        assert_eq!(attempt.at, original.at);
    }
}

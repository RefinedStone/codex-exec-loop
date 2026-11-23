use super::*;
use std::sync::atomic::Ordering;

use crate::application::port::inbound::stored_command_port::StoredCommandContext;
use crate::application::port::inbound::stored_command_port::task_evidence::RequestTaskVerificationRequest;
use crate::domain::planning::evidence::TaskEvidenceError;

#[test]
fn verification_finish_and_outbox_retries_keep_the_kernel_alive_without_rerunning() {
    let fixture = crate::application::service::task_verification::tests::Fixture::new();
    let log = Arc::new(crate::composition::task_evidence_outbox::tests::Log::default());
    let (mut runtime, _) = tests::recording_runtime_for_test();
    runtime.attach_kernel_event_log(log.clone(), "/repo");
    runtime.attach_task_evidence(fixture.evidence.clone());
    let handle = TaskVerificationHandle::default();
    runtime
        .attach_task_verification(fixture.service.clone(), &handle)
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

#[test]
fn native_attachment_binds_the_existing_handle_once_and_refuses_owner_replacement() {
    let fixture = crate::application::service::task_verification::tests::Fixture::new();
    let other = crate::application::service::task_verification::tests::Fixture::new();
    let (mut runtime, _) = tests::recording_runtime_for_test();
    let handle = TaskVerificationHandle::default();
    let earlier_clone = handle.clone();
    runtime
        .attach_task_verification(fixture.service.clone(), &handle)
        .unwrap();
    assert_eq!(fixture.repository.recoveries.load(Ordering::SeqCst), 1);

    assert!(
        runtime
            .attach_task_verification(fixture.service.clone(), &handle)
            .is_err()
    );
    assert!(
        runtime
            .attach_task_verification(other.service.clone(), &TaskVerificationHandle::default())
            .is_err()
    );
    assert_eq!(fixture.repository.recoveries.load(Ordering::SeqCst), 1);
    assert_eq!(other.repository.recoveries.load(Ordering::SeqCst), 0);
    assert!(Arc::ptr_eq(
        runtime.task_verification.as_ref().unwrap(),
        &fixture.service,
    ));

    // A clone made before Native attachment must now identify the same owner.
    // Initialize the other service so a failed bind cannot be attributed to it
    // lacking a live owner.
    other
        .service
        .initialize_live_owner("2026-11-23T12:00:00Z")
        .unwrap();
    assert!(earlier_clone.attach(&other.service).is_err());
    earlier_clone.attach(&fixture.service).unwrap();
    fixture.start();
    assert_eq!(fixture.spawn_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_owner_initialization_does_not_bind_the_shared_handle() {
    let closed = crate::application::service::task_verification::tests::Fixture::new();
    closed.service.shutdown();
    let fixture = crate::application::service::task_verification::tests::Fixture::new();
    let (mut runtime, _) = tests::recording_runtime_for_test();
    let handle = TaskVerificationHandle::default();

    let error = runtime
        .attach_task_verification(closed.service.clone(), &handle)
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::Unavailable),
    );
    assert_eq!(closed.repository.recoveries.load(Ordering::SeqCst), 0);
    assert!(runtime.task_verification.is_none());

    runtime
        .attach_task_verification(fixture.service.clone(), &handle)
        .unwrap();
    assert_eq!(fixture.repository.recoveries.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.spawn_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn native_drop_revokes_a_retained_service_without_database_writes_or_git_preparation() {
    let fixture = crate::application::service::task_verification::tests::Fixture::new();
    let (mut runtime, _) = tests::recording_runtime_for_test();
    runtime
        .attach_task_verification(fixture.service.clone(), &TaskVerificationHandle::default())
        .unwrap();
    let claim = fixture.start();
    let (work, interrupted) = fixture.take_job();
    let retained_service = fixture.service.clone();
    let request = RequestTaskVerificationRequest {
        request_id: "after-native-drop".into(),
        task_id: claim.stored.attempt.scope.task_id.clone(),
        criterion_id: claim.stored.attempt.criterion_id.clone(),
        scope_id: claim.stored.attempt.scope.scope_id.clone(),
        expected_acceptance_revision: claim.stored.attempt.scope.acceptance_revision,
    };
    let context = StoredCommandContext {
        human_confirmation: None,
        caller: None,
        channel: "kernel".into(),
        at: claim.stored.attempt.requested_at.clone(),
    };
    assert_eq!(fixture.repository.recoveries.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.repository.claims.load(Ordering::SeqCst), 1);

    drop(runtime);

    // The worker's service remains alive, but Native ownership does not.
    assert_eq!(fixture.repository.recoveries.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.repository.claims.load(Ordering::SeqCst), 1);
    assert!(fixture.repository.finish_calls.lock().unwrap().is_empty());
    assert!(fixture.repository.outbox.lock().unwrap().is_empty());
    assert_eq!(
        fixture.repository.acknowledgements.load(Ordering::SeqCst),
        0,
    );
    let error = retained_service
        .request_verification(request, &context)
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::Unavailable),
    );
    assert!(retained_service.initialize_live_owner(&context.at).is_err());
    assert_eq!(fixture.repository.recoveries.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.repository.claims.load(Ordering::SeqCst), 1);

    drop(interrupted);
    work();
    assert_eq!(fixture.runner.prepares.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.repository.reads.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.spawn_calls.load(Ordering::SeqCst), 1);
    assert!(fixture.repository.finish_calls.lock().unwrap().is_empty());
    assert!(fixture.repository.outbox.lock().unwrap().is_empty());
}

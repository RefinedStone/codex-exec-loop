use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::service::parallel_mode::tests::automatic_verification::{
    Fixture, REAL_GIT_WAIT,
};
use std::sync::atomic::Ordering;
use std::time::Instant;

fn context_and_request(
    fixture: &Fixture,
) -> (PostTurnEvaluationContext, PostTurnEvaluationRequest) {
    let projection = fixture
        .planning
        .runtime
        .load_runtime_projection_or_invalid(&fixture.workspace());
    let mut context = test_context(projection);
    context.planning_workspace_directory = fixture.workspace();
    context.parallel_mode_enabled = true;
    let mut request = test_request(context.clone());
    request.workspace_directory = fixture.captured.worktree_path.clone();
    request.completed_turn_id = "automatic-post-turn".into();
    attach_expected_lease(&mut request, fixture.captured.clone());
    (context, request)
}

#[test]
fn automatic_verification_post_turn_commits_and_records_before_capture_and_refresh() {
    let fixture = Fixture::new("auto-post-turn-durable-order");
    let before = fixture.head();
    let (context, request) = context_and_request(&fixture);
    let mut executor = PostTurnEvaluationExecutor::new(
        fixture.planning.clone(),
        fixture.turn_service(),
        PlanningWorkerPanelState::default(),
    );
    let completion = std::thread::spawn(move || {
        let capture = executor.begin_official_completion_if_needed(&context, &request);
        if let OfficialCompletionCapture::Captured(report) = &capture {
            executor.run_official_completion_refresh(
                &context,
                &request,
                &context.planning_workspace_directory,
                &context.current_runtime_projection,
                report,
            );
        }
        capture
    });
    let job = fixture.take_job();
    let committed = fixture.head();
    assert_ne!(committed, before);
    assert_eq!(fixture.next_refresh_order(), 1);
    job();
    assert!(fixture.receipt().is_none());
    assert_eq!(fixture.next_refresh_order(), 1);
    assert_eq!(fixture.worker.calls.load(Ordering::SeqCst), 0);
    assert!(fixture.host.poll(Instant::now()));
    assert!(matches!(
        fixture.join(completion),
        OfficialCompletionCapture::Captured(_)
    ));
    assert_eq!(fixture.next_refresh_order(), 2);
    assert_eq!(fixture.worker.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture
            .receipt()
            .unwrap()
            .manifest
            .unwrap()
            .source
            .head_commit_sha,
        committed
    );
}

#[test]
fn automatic_verification_post_turn_wait_does_not_hold_continuation_commit_lock() {
    let fixture = Fixture::new("auto-post-turn-continuation");
    let (context, request) = context_and_request(&fixture);
    let continuation = request.continuation_permit.clone();
    let executor = PostTurnEvaluationExecutor::new(
        fixture.planning.clone(),
        fixture.turn_service(),
        PlanningWorkerPanelState::default(),
    );
    let completion = std::thread::spawn(move || executor.run(&context, &request));
    let job = fixture.take_job();
    let before =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&fixture.workspace()).unwrap();
    let before_file = fixture.session_detail_bytes();
    let (sender, receiver) = std::sync::mpsc::channel();
    let cancellation = std::thread::spawn(move || {
        sender.send(continuation.invalidate_if_current()).unwrap();
    });
    assert!(
        receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
        "Pause must not wait for the verifier or for durable finish"
    );
    cancellation.join().unwrap();
    let outcome = fixture.join(completion);
    assert_eq!(
        outcome.evaluation.action,
        PostTurnContinuationAction::SkipAutoFollow {
            reason: PostTurnAutoFollowSkipReason::PostTurnContinuationPaused,
        }
    );
    assert_eq!(fixture.next_refresh_order(), 1);
    assert_eq!(fixture.worker.calls.load(Ordering::SeqCst), 0);
    let after =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&fixture.workspace()).unwrap();
    assert_eq!(after.session_details, before.session_details);
    assert_eq!(after.runtime_events, before.runtime_events);
    assert_eq!(fixture.session_detail_bytes(), before_file);
    job();
    fixture.host.poll(Instant::now() + Duration::from_secs(2));
    assert_eq!(fixture.runner.runs.load(Ordering::SeqCst), 0);
}

#[test]
fn automatic_verification_post_turn_cancellation_after_success_creates_no_capture() {
    use crate::application::service::parallel_mode::ParallelModeAutomationGuard;
    use crate::application::service::parallel_mode::set_before_automatic_completion_commit_hook;

    for cancel_epoch in [true, false] {
        let fixture = Fixture::new("auto-post-turn-after-success");
        let guard = ParallelModeAutomationGuard::default();
        guard.activate(fixture.workspace(), 11);
        let (mut context, mut request) = context_and_request(&fixture);
        context.parallel_automation_epoch_id = Some(11);
        request.context = context.clone();
        let continuation = request.continuation_permit.clone();
        let mut executor = PostTurnEvaluationExecutor::new(
            fixture.planning.clone(),
            fixture.turn_service().with_automation_guard(guard.clone()),
            PlanningWorkerPanelState::default(),
        );
        let (ready, at_boundary) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let completion = std::thread::spawn(move || {
            set_before_automatic_completion_commit_hook(move || {
                ready.send(()).unwrap();
                released.recv_timeout(REAL_GIT_WAIT).unwrap();
            });
            let capture = executor.begin_official_completion_if_needed(&context, &request);
            if let OfficialCompletionCapture::Captured(report) = &capture {
                executor.run_official_completion_refresh(
                    &context,
                    &request,
                    &context.planning_workspace_directory,
                    &context.current_runtime_projection,
                    report,
                );
            }
            capture
        });
        fixture.take_job()();
        assert!(fixture.host.poll(Instant::now()));
        at_boundary.recv_timeout(REAL_GIT_WAIT).unwrap();
        assert!(fixture.receipt().unwrap().passed());
        let before = fixture
            .authority
            .load_runtime_projections(&fixture.workspace())
            .unwrap();
        let before_file = fixture.session_detail_bytes();
        if cancel_epoch {
            guard.cancel(&fixture.workspace());
        } else {
            assert!(continuation.invalidate_if_current());
        }
        release.send(()).unwrap();
        assert!(matches!(
            fixture.join(completion),
            OfficialCompletionCapture::Cancelled
        ));
        assert_eq!(fixture.next_refresh_order(), 1);
        assert_eq!(fixture.worker.calls.load(Ordering::SeqCst), 0);
        let after = fixture
            .authority
            .load_runtime_projections(&fixture.workspace())
            .unwrap();
        assert_eq!(after.session_details, before.session_details);
        assert_eq!(after.runtime_events, before.runtime_events);
        assert_eq!(fixture.session_detail_bytes(), before_file);
    }
}

#[test]
fn automatic_verification_third_failed_fact_preserves_waiting_worker_without_refresh() {
    let fixture = Fixture::new("auto-post-turn-third-failure");
    fixture.runner.exit_code.store(2, Ordering::SeqCst);
    for attempt in 1..=2 {
        fixture.record_failed_automatic_attempt(attempt);
    }
    fs::write(
        std::path::Path::new(&fixture.lease.worktree_path).join("feature.txt"),
        "changed attempt 3\n",
    )
    .unwrap();
    let (context, request) = context_and_request(&fixture);
    let mut executor = PostTurnEvaluationExecutor::new(
        fixture.planning.clone(),
        fixture.turn_service(),
        PlanningWorkerPanelState::default(),
    );
    let completion = std::thread::spawn(move || {
        executor.begin_official_completion_if_needed(&context, &request)
    });
    fixture.take_job()();
    assert!(fixture.host.poll(Instant::now() + Duration::from_secs(2)));
    assert!(matches!(
        fixture.join(completion),
        OfficialCompletionCapture::AwaitingUser { .. }
    ));
    let task = fixture
        .authority
        .load_task_authority_snapshot(&fixture.workspace())
        .unwrap()
        .unwrap()
        .task_authority
        .tasks
        .remove(0);
    assert_eq!(task.status, TaskStatus::AwaitingUser);
    assert_eq!(fixture.runner.runs.load(Ordering::SeqCst), 3);
    assert_eq!(fixture.worker.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.next_refresh_order(), 1);
    assert!(std::path::Path::new(&fixture.lease.worktree_path).is_dir());
    assert!(!fixture.receipt().unwrap().passed());
    fixture.assert_waiting_session();
}

#[test]
fn automatic_verification_post_turn_cancellation_after_third_failure_preserves_durable_question() {
    use crate::application::service::parallel_mode::ParallelModeAutomationGuard;
    use crate::application::service::parallel_mode::set_before_automatic_completion_commit_hook;

    for cancel_epoch in [true, false] {
        let fixture = Fixture::new("auto-post-turn-third-failure-cancel");
        for attempt in 1..=2 {
            fixture.record_failed_automatic_attempt(attempt);
        }
        fs::write(
            std::path::Path::new(&fixture.lease.worktree_path).join("feature.txt"),
            "changed attempt 3\n",
        )
        .unwrap();
        let guard = ParallelModeAutomationGuard::default();
        guard.activate(fixture.workspace(), 11);
        let (mut context, mut request) = context_and_request(&fixture);
        context.parallel_automation_epoch_id = Some(11);
        request.context = context.clone();
        let continuation = request.continuation_permit.clone();
        let executor = PostTurnEvaluationExecutor::new(
            fixture.planning.clone(),
            fixture.turn_service().with_automation_guard(guard.clone()),
            PlanningWorkerPanelState::default(),
        );
        let (ready, at_boundary) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let completion = std::thread::spawn(move || {
            set_before_automatic_completion_commit_hook(move || {
                ready.send(()).unwrap();
                released.recv_timeout(REAL_GIT_WAIT).unwrap();
            });
            executor.run(&context, &request)
        });
        fixture.take_job()();
        assert!(fixture.host.poll(Instant::now()));
        at_boundary.recv_timeout(REAL_GIT_WAIT).unwrap();

        // Caller-side preservation has not run. Its cancellation must not erase
        // the already committed task and session needed to display the question.
        fixture.assert_waiting_session();
        let durable = fixture.receipt().unwrap();
        assert!(!durable.passed());
        let before = fixture
            .authority
            .load_runtime_projections(&fixture.workspace())
            .unwrap();
        let before_file = fixture.session_detail_bytes();
        if cancel_epoch {
            guard.cancel(&fixture.workspace());
        } else {
            assert!(continuation.invalidate_if_current());
        }
        release.send(()).unwrap();
        let outcome = fixture.join(completion);
        assert!(matches!(
            outcome.evaluation.action,
            PostTurnContinuationAction::SkipAutoFollow { .. }
        ));
        fixture.assert_waiting_session();
        assert_eq!(fixture.receipt().unwrap(), durable);
        assert_eq!(fixture.next_refresh_order(), 1);
        assert_eq!(fixture.worker.calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.runner.runs.load(Ordering::SeqCst), 3);
        let after = fixture
            .authority
            .load_runtime_projections(&fixture.workspace())
            .unwrap();
        assert_eq!(after.session_details, before.session_details);
        assert_eq!(after.runtime_events, before.runtime_events);
        assert_eq!(fixture.session_detail_bytes(), before_file);
    }
}

use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::service::parallel_mode::tests::automatic_verification::{
    Fixture, REAL_GIT_WAIT, TASK,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

fn request(fixture: &Fixture) -> ParallelDispatchWorkerRequest {
    let mut request = worker_request_with_secret_bodies();
    request.planning_workspace_directory = fixture.workspace();
    request.worktree_directory = fixture.captured.worktree_path.clone();
    request.expected_lease = fixture.captured.clone();
    request.automation_epoch_id = 11;
    request.handoff_task.task_id = TASK.into();
    request.handoff_task.task_title = fixture.captured.task_title.clone();
    request
}

#[test]
fn automatic_verification_dispatch_commits_then_waits_for_durable_receipt_before_refresh() {
    // A failed check is still a recorded fact. It must not be conflated with Done
    // or with delivery permission, which the E3 boundary evaluates separately.
    for exit_code in [0, 2] {
        let fixture = Fixture::new("auto-dispatch-durable-order");
        fixture.runner.exit_code.store(exit_code, Ordering::SeqCst);
        let before = fixture.head();
        let request = request(&fixture);
        let turn_service = fixture.turn_service();
        let planning = fixture.planning.clone();
        let completion = std::thread::spawn(move || {
            run_parallel_dispatch_official_completion(
                &request,
                &turn_service,
                &planning,
                &ParallelDispatchTurnCompleted::from_receipt(completed_receipt(
                    "auto-dispatch-turn",
                    Vec::new(),
                ))
                .unwrap(),
                Some("finished the isolated work"),
            )
        });
        let job = fixture.take_job();
        let committed = fixture.head();
        assert_ne!(
            committed, before,
            "the host commits before claiming verification"
        );
        assert_eq!(fixture.next_refresh_order(), 1);
        assert_eq!(fixture.worker.calls.load(Ordering::SeqCst), 0);
        job();
        assert_eq!(fixture.runner.runs.load(Ordering::SeqCst), 1);
        assert!(
            fixture.receipt().is_none(),
            "a runner response is not a durable receipt"
        );
        assert!(!completion.is_finished());
        assert_eq!(fixture.next_refresh_order(), 1);

        fixture
            .repository
            .fail_next_finish
            .store(true, Ordering::SeqCst);
        assert!(!fixture.host.poll(Instant::now()));
        assert!(fixture.receipt().is_none());
        assert_eq!(fixture.next_refresh_order(), 1);
        assert_eq!(fixture.worker.calls.load(Ordering::SeqCst), 0);
        assert!(fixture.host.poll(Instant::now() + Duration::from_secs(2)));
        let outcome = fixture.join(completion);
        assert!(
            !outcome.official_completion_refresh_succeeded,
            "the fake planner deliberately stops"
        );
        assert_eq!(
            fixture.worker.calls.load(Ordering::SeqCst),
            1,
            "{:?}",
            outcome.notices
        );
        assert_eq!(fixture.next_refresh_order(), 2);
        let receipt = fixture.receipt().unwrap();
        assert_eq!(receipt.passed(), exit_code == 0);
        assert_eq!(receipt.manifest.unwrap().source.head_commit_sha, committed);
        assert_eq!(fixture.runner.runs.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn automatic_verification_dispatch_epoch_closure_stops_before_run_and_refresh() {
    let fixture = Fixture::new("auto-dispatch-cancel");
    let guard = ParallelModeAutomationGuard::default();
    guard.activate(fixture.workspace(), 11);
    let turn_service = fixture.turn_service().with_automation_guard(guard.clone());
    let request = request(&fixture);
    let planning = fixture.planning.clone();
    let completion = std::thread::spawn(move || {
        run_parallel_dispatch_official_completion(
            &request,
            &turn_service,
            &planning,
            &ParallelDispatchTurnCompleted::from_receipt(completed_receipt(
                "auto-dispatch-cancel-turn",
                Vec::new(),
            ))
            .unwrap(),
            Some("completed before the operator paused"),
        )
    });
    let job = fixture.take_job();
    let before =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&fixture.workspace()).unwrap();
    let before_file = fixture.session_detail_bytes();
    guard.cancel(&fixture.workspace());
    let outcome = fixture.join(completion);
    assert!(!outcome.official_completion_refresh_succeeded);
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
    assert!(!fixture.receipt().unwrap().passed());
}

#[test]
fn automatic_verification_dispatch_epoch_closure_after_success_creates_no_completion_records() {
    use crate::application::service::parallel_mode::set_before_automatic_completion_commit_hook;

    let fixture = Fixture::new("auto-dispatch-after-success");
    let guard = ParallelModeAutomationGuard::default();
    guard.activate(fixture.workspace(), 11);
    let turn_service = fixture.turn_service().with_automation_guard(guard.clone());
    let request = request(&fixture);
    let planning = fixture.planning.clone();
    let (ready, at_boundary) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let completion = std::thread::spawn(move || {
        set_before_automatic_completion_commit_hook(move || {
            ready.send(()).unwrap();
            released.recv_timeout(REAL_GIT_WAIT).unwrap();
        });
        run_parallel_dispatch_official_completion(
            &request,
            &turn_service,
            &planning,
            &ParallelDispatchTurnCompleted::from_receipt(completed_receipt(
                "auto-dispatch-success-before-pause",
                Vec::new(),
            ))
            .unwrap(),
            Some("host verification finished before the pause"),
        )
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
    guard.cancel(&fixture.workspace());
    release.send(()).unwrap();
    let outcome = fixture.join(completion);
    assert!(!outcome.official_completion_refresh_succeeded);
    assert!(!outcome.awaiting_user);
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

#[test]
fn automatic_verification_dispatch_third_failure_creates_a_preserved_question_session() {
    let fixture = Fixture::new("auto-dispatch-third-failure");
    for attempt in 1..=2 {
        fixture.record_failed_automatic_attempt(attempt);
    }
    std::fs::write(
        std::path::Path::new(&fixture.lease.worktree_path).join("feature.txt"),
        "changed attempt 3\n",
    )
    .unwrap();
    let turn_service = fixture.turn_service();
    let request = request(&fixture);
    let planning = fixture.planning.clone();
    let completion = std::thread::spawn(move || {
        run_parallel_dispatch_official_completion(
            &request,
            &turn_service,
            &planning,
            &ParallelDispatchTurnCompleted::from_receipt(completed_receipt(
                "auto-dispatch-third-failure",
                Vec::new(),
            ))
            .unwrap(),
            Some("the third host check failed"),
        )
    });
    fixture.take_job()();
    assert!(fixture.host.poll(Instant::now()));
    let outcome = fixture.join(completion);
    assert!(outcome.awaiting_user, "{:?}", outcome.notices);
    assert!(!outcome.official_completion_refresh_succeeded);
    assert_eq!(fixture.next_refresh_order(), 1);
    assert_eq!(fixture.worker.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.runner.runs.load(Ordering::SeqCst), 3);
    fixture.assert_waiting_session();
}

struct ArchiveCountingWorker {
    scripted: ScriptedParallelAgentWorkerPort,
    starts: AtomicUsize,
    archives: AtomicUsize,
}

impl ParallelAgentWorkerPort for ArchiveCountingWorker {
    fn run_isolated_new_thread_stream(
        &self,
        request: ParallelAgentWorkerStreamRequest<'_>,
        events: crate::application::port::conversation_stream::ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.scripted
            .run_isolated_new_thread_stream(request, events)
    }

    fn archive_completed_isolated_thread(&self, _cwd: &str, _thread_id: &str) -> Result<()> {
        self.archives.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn automatic_verification_dispatch_cancellation_after_third_failure_preserves_durable_question() {
    use crate::application::service::parallel_mode::set_before_automatic_completion_commit_hook;

    let fixture = Fixture::new("auto-dispatch-third-failure-cancel");
    for attempt in 1..=2 {
        fixture.record_failed_automatic_attempt(attempt);
    }
    fs::write(
        Path::new(&fixture.lease.worktree_path).join("feature.txt"),
        "changed attempt 3\n",
    )
    .unwrap();
    let guard = ParallelModeAutomationGuard::default();
    guard.activate(fixture.workspace(), 11);
    let turn = fixture.turn_service().with_automation_guard(guard.clone());
    let request = request(&fixture);
    let planning = fixture.planning.clone();
    let receipt = completed_receipt("auto-dispatch-third-failure-cancel", Vec::new());
    let backend = Arc::new(ArchiveCountingWorker {
        scripted: ScriptedParallelAgentWorkerPort::new(
            vec![
                ConversationStreamEvent::ThreadPrepared {
                    thread_id: receipt.thread_id.clone(),
                    title: "Automatic verification".into(),
                    cwd: fixture.lease.worktree_path.clone(),
                    runtime_envelope: Box::default(),
                },
                ConversationStreamEvent::TurnStarted {
                    turn_id: receipt.turn_id.clone(),
                    runtime_request: Box::default(),
                },
                ConversationStreamEvent::TurnTerminal { receipt },
            ],
            WorkerExit::Ok,
        ),
        starts: AtomicUsize::new(0),
        archives: AtomicUsize::new(0),
    });
    let worker = backend.clone();
    let (ready, at_boundary) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let completion = std::thread::spawn(move || {
        set_before_automatic_completion_commit_hook(move || {
            ready.send(()).unwrap();
            released.recv_timeout(REAL_GIT_WAIT).unwrap();
        });
        run_parallel_dispatch_worker(request, worker, turn, planning)
    });
    fixture.take_job()();
    assert!(fixture.host.poll(Instant::now()));
    at_boundary.recv_timeout(REAL_GIT_WAIT).unwrap();

    // This read happens before caller preservation can run. The policy's single
    // transaction must already provide a resumable question after a crash or Pause.
    fixture.assert_waiting_session();
    let durable = fixture.receipt().unwrap();
    assert!(!durable.passed());
    let before = fixture
        .authority
        .load_runtime_projections(&fixture.workspace())
        .unwrap();
    let before_file = fixture.session_detail_bytes();
    guard.cancel(&fixture.workspace());
    release.send(()).unwrap();
    fixture.join(completion);

    fixture.assert_waiting_session();
    assert_eq!(fixture.receipt().unwrap(), durable);
    assert_eq!(fixture.next_refresh_order(), 1);
    assert_eq!(fixture.worker.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.runner.runs.load(Ordering::SeqCst), 3);
    assert_eq!(backend.starts.load(Ordering::SeqCst), 1);
    assert_eq!(backend.archives.load(Ordering::SeqCst), 0);
    let after = fixture
        .authority
        .load_runtime_projections(&fixture.workspace())
        .unwrap();
    assert_eq!(after.session_details, before.session_details);
    assert_eq!(after.runtime_events, before.runtime_events);
    assert_eq!(fixture.session_detail_bytes(), before_file);
}

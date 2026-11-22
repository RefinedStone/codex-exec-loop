//! 병렬 제어면 완료가 이슈 원격 수집 주기를 우회하지 않는지 실제 런타임 진입점으로 확인한다.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use super::{ClientCommandDispatch, NativeClientRuntime, tests::recording_runtime_for_test};
use crate::application::service::issue_collection::IssueCollectionRunOutcome;
use crate::application::service::parallel_mode::control_plane::{
    ParallelModeControlPlaneBackgroundEvent, ParallelModePendingDispatchPollCorrelation,
};
use crate::composition::issue_observer::IssueObserver;
use crate::domain::issue_automation::collection::{
    IssueCollectionConfig, IssueCollectionFailure, IssueCollectionMode, IssueCollectionProjection,
    IssueCollectionStatus, StoredIssueCollection,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeControlPlaneWorkerEvent,
    ParallelModeControlPlaneWorkerEventKind, ParallelModeDistributorSnapshot,
    ParallelModePoolBoardSnapshot, ParallelModeSupervisorDetailSnapshot,
    ParallelModeSupervisorSnapshot, ParallelModeSupervisorState,
};

const WORKSPACE: &str = "/workspace";
const EPOCH: u64 = 7;
const WAIT: Duration = Duration::from_secs(5);
const COLLECTION_INTERVAL: Duration = Duration::from_secs(300);

struct ReadRequest {
    response: Sender<anyhow::Result<IssueCollectionRunOutcome>>,
}

struct TrackingRequest {
    response: Sender<anyhow::Result<()>>,
}

struct Harness {
    runtime: NativeClientRuntime,
    requests: Receiver<ReadRequest>,
    calls: Arc<AtomicUsize>,
    tracking_requests: Receiver<TrackingRequest>,
    tracking_calls: Arc<AtomicUsize>,
}

fn projection(mode: IssueCollectionMode) -> IssueCollectionProjection {
    IssueCollectionProjection {
        state: StoredIssueCollection {
            config: IssueCollectionConfig { mode },
            epoch: 1,
            revision: 1,
            binding_id: None,
            discovery_cursor: None,
            resync_cursor: None,
            status: IssueCollectionStatus::default(),
        },
        binding: None,
        catalog_revision: 1,
    }
}

fn outcome(projection: IssueCollectionProjection) -> IssueCollectionRunOutcome {
    IssueCollectionRunOutcome {
        projection,
        newly_tracked: Vec::new(),
        discarded: false,
    }
}

fn success() -> IssueCollectionRunOutcome {
    outcome(projection(IssueCollectionMode::Observe))
}

fn failure(consecutive_failures: u32) -> IssueCollectionRunOutcome {
    let mut projection = projection(IssueCollectionMode::Observe);
    projection.state.status.last_failure = Some(IssueCollectionFailure::OriginUnavailable);
    projection.state.status.consecutive_failures = consecutive_failures;
    outcome(projection)
}

fn supervisor_snapshot() -> ParallelModeSupervisorSnapshot {
    ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        WORKSPACE,
        ParallelModePoolBoardSnapshot::new(0, "/unused-pool", "ready", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no workers"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no worker selected"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", ""),
        None,
    )
}

impl Harness {
    fn new(mode: IssueCollectionMode) -> Self {
        Self::with_tracking(mode, false)
    }

    fn with_tracking(mode: IssueCollectionMode, tracking: bool) -> Self {
        let (mut runtime, _turns) = recording_runtime_for_test();
        runtime.force_parallel_mode_for_test(WORKSPACE, true);
        // 미완료 진입 효과를 남겨 snapshot 완료의 후속 poll과 worker의 pool 조회를 보류한다.
        runtime
            .parallel_control_plane
            .force_parallel_entry_in_flight_for_test(WORKSPACE, EPOCH);
        runtime.force_parallel_supervisor_refresh_in_flight_for_test(WORKSPACE, EPOCH);
        let (requests, reads) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let read_calls = calls.clone();
        let collect = Arc::new(move |_: &crate::application::service::issue_collection::IssueCollectionCancellation, _: u64| {
            read_calls.fetch_add(1, Ordering::SeqCst);
            let (response, answer) = mpsc::channel();
            requests
                .send(ReadRequest { response })
                .expect("the test receives every fake collection");
            answer.recv().expect("the test releases the fake collection")
        });
        let (tracking_requests, tracking_reads) = mpsc::channel();
        let tracking_calls = Arc::new(AtomicUsize::new(0));
        let track_calls = tracking_calls.clone();
        let observer = if tracking {
            IssueObserver::for_test_with_tracking(
                projection(mode),
                collect,
                Arc::new(move |_, _, _, _| {
                    track_calls.fetch_add(1, Ordering::SeqCst);
                    let (response, answer) = mpsc::channel();
                    tracking_requests
                        .send(TrackingRequest { response })
                        .expect("test receives local tracking");
                    answer.recv().expect("test releases local tracking")
                }),
                runtime.host_waker(),
            )
        } else {
            IssueObserver::for_test(projection(mode), collect, runtime.host_waker())
        };
        runtime.attach_issue_observer_for_test(observer);
        Self {
            runtime,
            requests: reads,
            calls,
            tracking_requests: tracking_reads,
            tracking_calls,
        }
    }

    fn poll(&mut self, now: Instant) -> bool {
        self.runtime
            .issue_observer
            .as_mut()
            .unwrap()
            .poll(now, 1_000)
            .changed
    }

    fn deadline(&self, now: Instant) -> Option<Instant> {
        self.runtime
            .issue_observer
            .as_ref()
            .unwrap()
            .next_deadline(now)
    }

    fn read(&self) -> ReadRequest {
        self.requests
            .recv_timeout(WAIT)
            .expect("a scheduled fake collection starts")
    }

    fn finish(
        &mut self,
        request: ReadRequest,
        result: anyhow::Result<IssueCollectionRunOutcome>,
        now: Instant,
    ) {
        // 이전 수동 깨우기가 남아 있어도 실제 수집 완료를 읽을 때까지 기다린다.
        request
            .response
            .send(result)
            .expect("the worker is reading");
        let deadline = Instant::now() + WAIT;
        while !self.poll(now) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "the fake collection completes");
            let _ = self.runtime.core.runtime.wait_pending_input(remaining);
        }
    }

    fn assert_reads(&self, expected: usize) {
        assert_eq!(self.calls.load(Ordering::SeqCst), expected);
        assert!(matches!(self.requests.try_recv(), Err(TryRecvError::Empty)));
    }

    fn track(&self) -> TrackingRequest {
        self.tracking_requests
            .recv_timeout(WAIT)
            .expect("local tracking starts")
    }

    fn finish_tracking(&mut self, request: TrackingRequest, now: Instant) {
        request
            .response
            .send(Ok(()))
            .expect("local tracking is waiting");
        let deadline = Instant::now() + WAIT;
        loop {
            self.poll(now);
            if !self
                .runtime
                .issue_observer
                .as_ref()
                .unwrap()
                .tracking_in_flight_for_test()
            {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "the expected tracking worker completes"
            );
            let _ = self.runtime.core.runtime.wait_pending_input(remaining);
        }
    }

    fn assert_tracking_calls(&self, expected: usize) {
        assert_eq!(self.tracking_calls.load(Ordering::SeqCst), expected);
        assert!(matches!(
            self.tracking_requests.try_recv(),
            Err(TryRecvError::Empty)
        ));
    }

    fn polling_completions(&mut self) {
        for operation_id in 1..=3 {
            self.runtime.dispatch_parallel_completion(
                ParallelModeControlPlaneBackgroundEvent::PendingDispatchWakePolled {
                    correlation: ParallelModePendingDispatchPollCorrelation {
                        operation_id,
                        workspace_directory: WORKSPACE.into(),
                        epoch_id: EPOCH,
                    },
                    result: Ok(None),
                },
            );
            let effect_id = self
                .runtime
                .force_parallel_supervisor_refresh_in_flight_for_test(WORKSPACE, EPOCH);
            self.runtime.dispatch_parallel_completion(
                ParallelModeControlPlaneBackgroundEvent::SupervisorSnapshotRefreshed {
                    workspace_directory: WORKSPACE.into(),
                    epoch_id: EPOCH,
                    effect_id,
                    supervisor_snapshot: Box::new(supervisor_snapshot()),
                    orchestrator_tick_signature: None,
                },
            );
        }
        self.runtime
            .force_parallel_supervisor_refresh_in_flight_for_test(WORKSPACE, EPOCH);
    }

    fn worker(
        &mut self,
        workspace: &str,
        epoch: u64,
        kind: ParallelModeControlPlaneWorkerEventKind,
    ) {
        self.runtime.dispatch_parallel_completion(
            ParallelModeControlPlaneBackgroundEvent::WorkerEvent {
                event: ParallelModeControlPlaneWorkerEvent::new(
                    workspace,
                    epoch,
                    "task-1",
                    "fake worker",
                    kind,
                    Vec::new(),
                ),
                has_actionable_queue_head: false,
            },
        );
    }
}

#[test]
fn polling_completions_preserve_the_five_minute_collection_interval() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    h.poll(now);
    let first = h.read();
    h.finish(first, Ok(success()), now);
    let due = now + COLLECTION_INTERVAL;

    h.polling_completions();
    assert_eq!(h.deadline(now), Some(due));
    h.poll(due - Duration::from_millis(1));
    h.assert_reads(1);

    h.poll(due);
    let second = h.read();
    h.finish(second, Ok(success()), due);
    h.assert_reads(2);
}

#[test]
fn polling_completions_preserve_every_failure_backoff() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let mut now = Instant::now();
    h.poll(now);

    for (failures, minutes) in [(1, 10), (2, 20), (3, 40), (4, 60), (5, 60)] {
        let request = h.read();
        h.finish(request, Ok(failure(failures)), now);
        let due = now + Duration::from_secs(minutes * 60);

        h.polling_completions();
        assert_eq!(h.deadline(now), Some(due));
        h.poll(due - Duration::from_millis(1));
        h.assert_reads(failures as usize);

        h.poll(due);
        now = due;
    }

    let final_read = h.read();
    h.finish(final_read, Ok(success()), now);
    h.assert_reads(6);
    assert_eq!(h.deadline(now), Some(now + COLLECTION_INTERVAL));
}

#[test]
fn polling_completions_during_a_read_do_not_queue_an_extra_collection() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    h.poll(now);
    let first = h.read();

    h.polling_completions();
    h.poll(now + Duration::from_secs(1));
    h.assert_reads(1);

    let completed = now + Duration::from_secs(2);
    h.finish(first, Ok(success()), completed);
    assert_eq!(h.deadline(completed), Some(completed + COLLECTION_INTERVAL));
    h.assert_reads(1);
}

#[test]
fn each_current_worker_terminal_requests_an_immediate_collection() {
    for kind in [
        ParallelModeControlPlaneWorkerEventKind::Completed,
        ParallelModeControlPlaneWorkerEventKind::LaunchFailed,
        ParallelModeControlPlaneWorkerEventKind::StreamFailed,
    ] {
        let mut h = Harness::new(IssueCollectionMode::Observe);
        let now = Instant::now();
        h.poll(now);
        let first = h.read();
        h.finish(first, Ok(failure(1)), now);

        h.worker(WORKSPACE, EPOCH, kind);
        let resumed = Instant::now();
        assert!(h.deadline(resumed).is_some_and(|due| due <= resumed));
        h.poll(resumed);
        let second = h.read();
        h.finish(second, Ok(success()), resumed);
        h.assert_reads(2);
    }
}

#[test]
fn current_worker_terminals_during_a_read_coalesce_into_one_follow_up() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    h.poll(now);
    let first = h.read();

    for kind in [
        ParallelModeControlPlaneWorkerEventKind::Completed,
        ParallelModeControlPlaneWorkerEventKind::LaunchFailed,
        ParallelModeControlPlaneWorkerEventKind::StreamFailed,
    ] {
        h.worker(WORKSPACE, EPOCH, kind);
    }
    h.assert_reads(1);

    h.finish(first, Ok(success()), now);
    let follow_up = h.read();
    h.finish(follow_up, Ok(success()), now);
    h.assert_reads(2);
    assert_eq!(h.deadline(now), Some(now + COLLECTION_INTERVAL));
}

#[test]
fn stale_or_other_workspace_worker_terminals_do_not_request_a_collection() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    h.poll(now);
    let first = h.read();
    h.finish(first, Ok(failure(1)), now);
    let due = now + Duration::from_secs(600);

    for (workspace, epoch) in [
        (WORKSPACE, EPOCH - 1),
        (WORKSPACE, EPOCH + 1),
        ("/other-workspace", EPOCH),
    ] {
        for kind in [
            ParallelModeControlPlaneWorkerEventKind::Completed,
            ParallelModeControlPlaneWorkerEventKind::LaunchFailed,
            ParallelModeControlPlaneWorkerEventKind::StreamFailed,
        ] {
            h.worker(workspace, epoch, kind);
        }
    }
    h.runtime.force_parallel_mode_for_test(WORKSPACE, false);
    h.worker(
        WORKSPACE,
        EPOCH,
        ParallelModeControlPlaneWorkerEventKind::Completed,
    );

    assert_eq!(h.deadline(now), Some(due));
    h.poll(due - Duration::from_millis(1));
    h.assert_reads(1);
}

#[test]
fn an_explicit_issue_sync_still_bypasses_the_collection_backoff() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    h.poll(now);
    let first = h.read();
    h.finish(first, Ok(failure(1)), now);

    assert!(matches!(
        h.runtime.dispatch_issue_sync(),
        ClientCommandDispatch::Ran { .. }
    ));
    let synced = Instant::now();
    h.poll(synced);
    let second = h.read();
    h.finish(second, Ok(success()), synced);
    h.assert_reads(2);
}

#[test]
fn current_worker_terminals_do_not_enable_collection_when_observe_is_off() {
    let mut h = Harness::new(IssueCollectionMode::Off);
    h.worker(
        WORKSPACE,
        EPOCH,
        ParallelModeControlPlaneWorkerEventKind::Completed,
    );
    h.poll(Instant::now());
    assert_eq!(h.deadline(Instant::now()), None);
    h.assert_reads(0);
}

#[test]
fn current_worker_terminals_wake_local_tracking_even_when_collection_is_off() {
    let mut h = Harness::with_tracking(IssueCollectionMode::Off, true);
    let now = Instant::now();
    h.poll(now);
    let initial = h.track();
    h.finish_tracking(initial, now);
    for (index, kind) in [
        ParallelModeControlPlaneWorkerEventKind::Completed,
        ParallelModeControlPlaneWorkerEventKind::LaunchFailed,
        ParallelModeControlPlaneWorkerEventKind::StreamFailed,
    ]
    .into_iter()
    .enumerate()
    {
        h.worker(WORKSPACE, EPOCH, kind);
        let woke = Instant::now();
        assert!(h.deadline(woke).is_some_and(|due| due <= woke));
        h.poll(woke);
        let tracking = h.track();
        h.finish_tracking(tracking, woke);
        h.assert_reads(0);
        h.assert_tracking_calls(index + 2);
        assert_eq!(h.deadline(woke), Some(woke + Duration::from_secs(5)));
    }
}

#[test]
fn stale_or_other_workspace_worker_terminals_do_not_wake_local_tracking() {
    let mut h = Harness::with_tracking(IssueCollectionMode::Observe, true);
    let now = Instant::now();
    h.poll(now);
    let remote = h.read();
    let local = h.track();
    h.finish(remote, Ok(failure(1)), now);
    h.finish_tracking(local, now);
    let local_due = now + Duration::from_secs(5);

    for (workspace, epoch) in [
        (WORKSPACE, EPOCH - 1),
        (WORKSPACE, EPOCH + 1),
        ("/other-workspace", EPOCH),
    ] {
        for kind in [
            ParallelModeControlPlaneWorkerEventKind::Completed,
            ParallelModeControlPlaneWorkerEventKind::LaunchFailed,
            ParallelModeControlPlaneWorkerEventKind::StreamFailed,
        ] {
            h.worker(workspace, epoch, kind);
        }
    }
    h.runtime.force_parallel_mode_for_test(WORKSPACE, false);
    h.worker(
        WORKSPACE,
        EPOCH,
        ParallelModeControlPlaneWorkerEventKind::Completed,
    );
    assert_eq!(h.deadline(now), Some(local_due));
    h.poll(local_due - Duration::from_millis(1));
    h.assert_reads(1);
    h.assert_tracking_calls(1);
}

#[test]
fn periodic_control_completions_and_local_ticks_preserve_remote_failure_backoff() {
    let mut h = Harness::with_tracking(IssueCollectionMode::Observe, true);
    let now = Instant::now();
    h.poll(now);
    let remote = h.read();
    let local = h.track();
    h.finish(remote, Ok(failure(1)), now);
    h.finish_tracking(local, now);
    h.polling_completions();
    assert_eq!(h.deadline(now), Some(now + Duration::from_secs(5)));
    h.assert_tracking_calls(1);

    for second in [5, 10, 15] {
        let tick = now + Duration::from_secs(second);
        h.poll(tick);
        let local = h.track();
        h.finish_tracking(local, tick);
        h.polling_completions();
        h.assert_reads(1);
        assert_eq!(
            h.runtime
                .issue_observer
                .as_ref()
                .unwrap()
                .view(tick)
                .next_poll_after_ms,
            Some((600 - second) * 1_000),
        );
    }
    h.assert_tracking_calls(4);
}

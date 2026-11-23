use super::*;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;

use crate::core::app::{CoreEffect, CoreInput};
use crate::core::runtime::{CoreEffectExecutor, CoreInputSender, CoreRuntime, core_input_channel};
use crate::domain::issue_automation::collection::{
    IssueCollectionConfig, IssueCollectionFailure, IssueCollectionStatus, StoredIssueCollection,
};

mod deadlines;

const WAIT: Duration = Duration::from_secs(5);

struct NoEffects;
impl CoreEffectExecutor for NoEffects {
    fn run_effect(&self, _effect: CoreEffect) -> Option<CoreInput> {
        panic!("the scheduler test must not execute a core effect")
    }
}

struct ReadRequest {
    at_unix_ms: u64,
    response: Sender<Result<IssueCollectionRunOutcome>>,
}

struct Harness {
    observer: IssueObserver,
    core: CoreRuntime<NoEffects>,
    _keepalive: CoreInputSender,
    requests: Receiver<ReadRequest>,
    calls: Arc<AtomicUsize>,
    cancelled: Arc<AtomicBool>,
}

fn projection(mode: IssueCollectionMode, epoch: u64) -> IssueCollectionProjection {
    IssueCollectionProjection {
        state: StoredIssueCollection {
            config: IssueCollectionConfig { mode },
            epoch,
            revision: epoch,
            binding_id: None,
            discovery_cursor: None,
            resync_cursor: None,
            status: IssueCollectionStatus::default(),
        },
        binding: None,
        catalog_revision: epoch,
    }
}

fn success(projection: IssueCollectionProjection) -> IssueCollectionRunOutcome {
    IssueCollectionRunOutcome {
        projection,
        newly_tracked: Vec::new(),
        discarded: false,
    }
}

impl Harness {
    fn new(mode: IssueCollectionMode) -> Self {
        let (keepalive, receiver) = core_input_channel();
        let core = CoreRuntime::new(NoEffects, receiver);
        let (requests, reads) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        let read_calls = calls.clone();
        let read_cancelled = cancelled.clone();
        let observer = IssueObserver::for_test(
            projection(mode, 1),
            Arc::new(move |cancel, at_unix_ms| {
                read_calls.fetch_add(1, Ordering::SeqCst);
                let (response, answer) = mpsc::channel();
                requests
                    .send(ReadRequest {
                        at_unix_ms,
                        response,
                    })
                    .expect("test receives the read request");
                let result = answer.recv().expect("test releases the read");
                read_cancelled.store(cancel.is_cancelled(), Ordering::SeqCst);
                result
            }),
            core.input_waker(),
        );
        Self {
            observer,
            core,
            _keepalive: keepalive,
            requests: reads,
            calls,
            cancelled,
        }
    }

    fn read(&self) -> ReadRequest {
        self.requests
            .recv_timeout(WAIT)
            .expect("scheduled read starts")
    }

    fn finish(
        &mut self,
        request: ReadRequest,
        result: Result<IssueCollectionRunOutcome>,
        now: Instant,
    ) -> IssueObserverCycle {
        let previous = self.observer.in_flight.as_ref().unwrap().cancel.clone();
        request
            .response
            .send(result)
            .expect("worker is still reading");
        let deadline = Instant::now() + WAIT;
        let mut cycle = IssueObserverCycle {
            changed: false,
            committed: Vec::new(),
        };
        loop {
            let observed = self.observer.poll(now, 1_000);
            cycle.changed |= observed.changed;
            cycle.committed.extend(observed.committed);
            if self
                .observer
                .in_flight
                .as_ref()
                .is_none_or(|flight| !Arc::ptr_eq(&flight.cancel, &previous))
            {
                return cycle;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "the expected collection worker completes"
            );
            let _ = self.core.wait_pending_input(remaining);
        }
    }

    fn assert_no_more_reads(&self) {
        assert!(matches!(self.requests.try_recv(), Err(TryRecvError::Empty)));
    }
}

#[test]
fn off_never_starts_a_provider_read_even_after_sync_or_time_advance() {
    let mut h = Harness::new(IssueCollectionMode::Off);
    let now = Instant::now();
    assert_eq!(h.observer.next_deadline(now), None);
    assert!(!h.observer.request_sync(now));
    assert!(!h.observer.poll(now, 1_000).changed);
    assert!(
        !h.observer
            .poll(now + Duration::from_secs(86_400), 2_000)
            .changed
    );
    assert_eq!(h.calls.load(Ordering::SeqCst), 0);
    h.assert_no_more_reads();
}

#[test]
fn a_success_uses_exactly_five_minutes_of_the_host_clock() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    assert_eq!(h.observer.next_deadline(now), Some(now));
    h.observer.poll(now, 42);
    let first = h.read();
    assert_eq!(first.at_unix_ms, 42);
    assert!(
        h.finish(
            first,
            Ok(success(projection(IssueCollectionMode::Observe, 1))),
            now
        )
        .changed
    );
    let due = now + Duration::from_secs(300);
    assert_eq!(h.observer.next_deadline(now), Some(due));
    assert_eq!(h.observer.view(now).next_poll_after_ms, Some(300_000));
    assert!(!h.observer.poll(due - Duration::from_millis(1), 43).changed);
    h.assert_no_more_reads();
    h.observer.poll(due, 44);
    let second = h.read();
    assert_eq!(second.at_unix_ms, 44);
    h.finish(
        second,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        due,
    );
    assert_eq!(h.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn failures_back_off_to_sixty_minutes_and_success_resets_the_interval() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let mut now = Instant::now();
    h.observer.poll(now, 1_000);
    for (failures, minutes) in [(1, 10), (2, 20), (3, 40), (4, 60), (5, 60)] {
        let request = h.read();
        let mut failed = projection(IssueCollectionMode::Observe, 1);
        failed.state.status.last_attempt_at_ms = Some(1_000);
        failed.state.status.last_failure = Some(IssueCollectionFailure::OriginUnavailable);
        failed.state.status.consecutive_failures = failures;
        assert!(h.finish(request, Ok(success(failed)), now).changed);
        let due = now + Duration::from_secs(minutes * 60);
        assert_eq!(h.observer.next_deadline(now), Some(due));
        h.observer.poll(due - Duration::from_millis(1), 1_000);
        h.assert_no_more_reads();
        h.observer.poll(due, 1_000);
        now = due;
    }
    let request = h.read();
    h.finish(
        request,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        now,
    );
    assert_eq!(
        h.observer.next_deadline(now),
        Some(now + COLLECTION_INTERVAL)
    );
}

#[test]
fn local_store_failures_also_back_off_without_publishing_a_success() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let request = h.read();
    let cycle = h.finish(
        request,
        Err(anyhow::anyhow!("synthetic local failure")),
        now,
    );
    assert!(cycle.committed.is_empty());
    assert_eq!(
        h.observer.next_deadline(now),
        Some(now + Duration::from_secs(600))
    );
}

#[test]
fn repeated_sync_is_coalesced_into_one_follow_up_without_overlapping_reads() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let request = h.read();
    for _ in 0..5 {
        assert!(h.observer.request_sync(now));
        h.observer.poll(now + Duration::from_secs(86_400), 1_000);
    }
    assert_eq!(h.observer.next_deadline(now), None);
    assert_eq!(h.calls.load(Ordering::SeqCst), 1);
    h.assert_no_more_reads();
    h.finish(
        request,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        now,
    );
    // 진행 중 들어온 승인/동기화 wake를 한 번의 후속 읽기로 보존한다.
    // 첫 worker가 끝난 뒤에만 시작하므로 중복 네트워크 읽기는 없다.
    let follow_up = h.read();
    assert_eq!(h.calls.load(Ordering::SeqCst), 2);
    assert_eq!(h.observer.next_deadline(now), None);
    h.observer.poll(now + Duration::from_secs(86_400), 1_000);
    h.assert_no_more_reads();
    h.finish(
        follow_up,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        now,
    );
    h.assert_no_more_reads();
    assert_eq!(
        h.observer.next_deadline(now),
        Some(now + COLLECTION_INTERVAL)
    );
    assert_eq!(h.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn off_discards_the_old_response_and_preserves_the_saved_epoch() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let request = h.read();
    h.observer
        .replace_projection(projection(IssueCollectionMode::Off, 2), now);
    assert!(!h.observer.view(now).collecting);
    assert!(!h.observer.request_sync(now));
    let cycle = h.finish(
        request,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        now,
    );
    assert!(cycle.committed.is_empty());
    assert!(h.cancelled.load(Ordering::SeqCst));
    assert_eq!(
        h.observer.view(now).projection,
        projection(IssueCollectionMode::Off, 2)
    );
    assert_eq!(h.observer.next_deadline(now), None);
    h.assert_no_more_reads();
}

#[test]
fn reenable_waits_for_the_cancelled_worker_instead_of_overlapping_reads() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let request = h.read();
    h.observer
        .replace_projection(projection(IssueCollectionMode::Off, 2), now);
    h.observer
        .replace_projection(projection(IssueCollectionMode::Observe, 3), now);
    h.observer.poll(now, 1_000);
    h.assert_no_more_reads();
    assert_eq!(h.observer.next_deadline(now), None);
    let cycle = h.finish(
        request,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        now,
    );
    assert!(cycle.committed.is_empty());
    let restarted = h.read();
    h.finish(
        restarted,
        Ok(success(projection(IssueCollectionMode::Observe, 3))),
        now,
    );
    assert_eq!(h.calls.load(Ordering::SeqCst), 2);
    assert_eq!(h.observer.view(now).projection.state.epoch, 3);
}

#[test]
fn dropping_an_observer_cancels_without_waiting_for_the_network_read() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let request = h.read();
    let Harness {
        observer,
        mut core,
        _keepalive,
        cancelled,
        ..
    } = h;
    let (dropped, done) = mpsc::channel();
    let drop_worker =
        crate::composition::core_effect_worker::spawn_joinable_redacted_worker(move || {
            drop(observer);
            dropped.send(()).expect("test receives the drop");
        });
    done.recv_timeout(WAIT)
        .expect("drop cannot join a blocked provider");
    drop_worker.join().expect("observer drop worker completed");
    request
        .response
        .send(Ok(success(projection(IssueCollectionMode::Observe, 1))))
        .expect("test releases the detached read");
    let _ = core.wait_pending_input(WAIT);
    assert!(cancelled.load(Ordering::SeqCst));
}

#[test]
fn a_panicked_worker_wakes_the_host_and_enters_backoff() {
    let (_keepalive, receiver) = core_input_channel();
    let mut core = CoreRuntime::new(NoEffects, receiver);
    let mut observer = IssueObserver::for_test(
        projection(IssueCollectionMode::Observe, 1),
        Arc::new(|_, _| panic!("synthetic collector panic")),
        core.input_waker(),
    );
    let now = Instant::now();
    observer.poll(now, 1_000);
    let _ = core.wait_pending_input(WAIT);
    let cycle = observer.poll(now, 1_000);
    assert!(cycle.changed);
    assert!(cycle.committed.is_empty());
    assert!(!observer.view(now).collecting);
    assert_eq!(
        observer.next_deadline(now),
        Some(now + Duration::from_secs(600))
    );
}

#[test]
fn a_policy_reset_epoch_commits_and_the_next_normal_collection_keeps_running() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let first = h.read();
    // 수집 서비스가 정책에 결속된 커서를 버리고 같은 Observe 설정의 epoch를 진전시켰다.
    let reset = projection(IssueCollectionMode::Observe, 2);
    assert!(h.finish(first, Ok(success(reset.clone())), now).changed);
    assert_eq!(h.observer.view(now).projection.state.epoch, 2);
    let due = now + COLLECTION_INTERVAL;
    assert_eq!(h.observer.next_deadline(now), Some(due));
    h.observer.poll(due, 2_000);
    let next = h.read();
    assert!(h.finish(next, Ok(success(reset)), due).changed);
    assert_eq!(
        h.observer.next_deadline(due),
        Some(due + COLLECTION_INTERVAL)
    );
    assert_eq!(h.calls.load(Ordering::SeqCst), 2);
}

struct TrackRequest {
    at_unix_ms: u64,
    execution_enabled: bool,
    response: Sender<Result<()>>,
}

struct TrackingProbe {
    requests: Receiver<TrackRequest>,
    calls: Arc<AtomicUsize>,
    cancelled: Arc<AtomicBool>,
}

impl TrackingProbe {
    fn read(&self) -> TrackRequest {
        self.requests
            .recv_timeout(WAIT)
            .expect("local tracking starts")
    }

    fn assert_calls(&self, expected: usize) {
        assert_eq!(self.calls.load(Ordering::SeqCst), expected);
        assert!(matches!(self.requests.try_recv(), Err(TryRecvError::Empty)));
    }
}

impl Harness {
    fn track(&mut self) -> TrackingProbe {
        let (requests, reads) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        let track_calls = calls.clone();
        let track_cancelled = cancelled.clone();
        self.observer.track = Some(Arc::new(move |cancel, at_unix_ms, execution_enabled, _| {
            track_calls.fetch_add(1, Ordering::SeqCst);
            let (response, answer) = mpsc::channel();
            requests
                .send(TrackRequest {
                    at_unix_ms,
                    execution_enabled,
                    response,
                })
                .expect("test receives local tracking");
            let result = answer.recv().expect("test releases local tracking");
            track_cancelled.store(cancel.is_cancelled(), Ordering::SeqCst);
            result
        }));
        TrackingProbe {
            requests: reads,
            calls,
            cancelled,
        }
    }

    fn finish_tracking(
        &mut self,
        request: TrackRequest,
        result: Result<()>,
        now: Instant,
    ) -> IssueObserverCycle {
        let previous = self
            .observer
            .tracking_in_flight
            .as_ref()
            .unwrap()
            .cancel
            .clone();
        request
            .response
            .send(result)
            .expect("local tracking is running");
        let deadline = Instant::now() + WAIT;
        let mut cycle = IssueObserverCycle {
            changed: false,
            committed: Vec::new(),
        };
        loop {
            let observed = self.observer.poll(now, 1_000);
            cycle.changed |= observed.changed;
            cycle.committed.extend(observed.committed);
            if self
                .observer
                .tracking_in_flight
                .as_ref()
                .is_none_or(|flight| !Arc::ptr_eq(&flight.cancel, &previous))
            {
                return cycle;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "the expected tracking worker completes"
            );
            let _ = self.core.wait_pending_input(remaining);
        }
    }
}

#[test]
fn collection_off_keeps_local_tracking_on_a_five_second_clock() {
    let mut h = Harness::new(IssueCollectionMode::Off);
    let track = h.track();
    let now = Instant::now();
    assert_eq!(h.observer.next_deadline(now), Some(now));
    assert_eq!(h.observer.view(now).next_poll_after_ms, None);
    assert!(!h.observer.request_sync(now));
    assert!(!h.observer.poll(now, 42).changed);
    let first = track.read();
    assert_eq!(first.at_unix_ms, 42);
    assert!(!first.execution_enabled);
    assert!(!h.finish_tracking(first, Ok(()), now).changed);
    assert_eq!(h.observer.next_deadline(now), Some(now + TRACKING_INTERVAL));

    h.observer
        .poll(now + TRACKING_INTERVAL - Duration::from_millis(1), 43);
    track.assert_calls(1);
    h.observer.poll(now + TRACKING_INTERVAL, 44);
    let second = track.read();
    assert_eq!(second.at_unix_ms, 44);
    h.finish_tracking(second, Ok(()), now + TRACKING_INTERVAL);
    assert_eq!(h.calls.load(Ordering::SeqCst), 0);
    h.assert_no_more_reads();
    track.assert_calls(2);
}

#[test]
fn local_ticks_do_not_change_remote_collection_deadlines_or_the_displayed_countdown() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let track = h.track();
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let first = h.read();
    let local = track.read();
    h.finish(
        first,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        now,
    );
    h.finish_tracking(local, Ok(()), now);
    let remote_due = now + COLLECTION_INTERVAL;
    assert_eq!(h.observer.view(now).next_poll_after_ms, Some(300_000));
    assert_eq!(h.observer.next_deadline(now), Some(now + TRACKING_INTERVAL));

    for second in [5, 10, 299] {
        let tick = now + Duration::from_secs(second);
        h.observer.poll(tick, 1_000 + second);
        let local = track.read();
        h.finish_tracking(local, Ok(()), tick);
        assert_eq!(h.observer.collection_deadline(tick), Some(remote_due));
        assert_eq!(h.calls.load(Ordering::SeqCst), 1);
        h.assert_no_more_reads();
    }
    h.observer.poll(remote_due, 2_000);
    let second = h.read();
    h.finish(
        second,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        remote_due,
    );
    assert_eq!(h.calls.load(Ordering::SeqCst), 2);
    track.assert_calls(4);
}

#[test]
fn local_tracking_success_and_failure_preserve_each_remote_backoff() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let track = h.track();
    let mut now = Instant::now();
    h.observer.poll(now, 1_000);
    for (failures, minutes) in [(1, 10), (2, 20), (3, 40), (4, 60)] {
        let remote = h.read();
        let local = track.read();
        let mut failed = projection(IssueCollectionMode::Observe, 1);
        failed.state.status.last_failure = Some(IssueCollectionFailure::OriginUnavailable);
        failed.state.status.consecutive_failures = failures;
        h.finish(remote, Ok(success(failed)), now);
        h.finish_tracking(local, Ok(()), now);
        let due = now + Duration::from_secs(minutes * 60);

        let tick = now + TRACKING_INTERVAL;
        h.observer.poll(tick, 2_000);
        let local = track.read();
        h.finish_tracking(
            local,
            Err(anyhow::anyhow!("local observation unavailable")),
            tick,
        );
        assert_eq!(h.observer.collection_deadline(tick), Some(due));
        assert_eq!(h.observer.consecutive_failures, failures);
        assert_eq!(h.calls.load(Ordering::SeqCst), failures as usize);
        h.assert_no_more_reads();

        h.observer.poll(due, 3_000);
        now = due;
    }
    let final_remote = h.read();
    let final_local = track.read();
    h.finish(
        final_remote,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        now,
    );
    h.finish_tracking(final_local, Ok(()), now);
    assert_eq!(
        h.observer.collection_deadline(now),
        Some(now + COLLECTION_INTERVAL)
    );
}

#[test]
fn a_blocked_collection_does_not_block_tracking_and_tracking_requests_coalesce() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let track = h.track();
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let remote = h.read();
    let local = track.read();
    for _ in 0..5 {
        assert!(h.observer.request_tracking(now));
        h.observer.poll(now + Duration::from_secs(86_400), 1_000);
    }
    track.assert_calls(1);
    assert_eq!(h.calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.observer.next_deadline(now), None);
    h.finish_tracking(local, Ok(()), now);
    let follow_up = track.read();
    track.assert_calls(2);
    h.finish_tracking(follow_up, Ok(()), now);
    assert_eq!(h.observer.next_deadline(now), Some(now + TRACKING_INTERVAL));
    h.finish(
        remote,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        now,
    );
    assert_eq!(
        h.observer.collection_deadline(now),
        Some(now + COLLECTION_INTERVAL)
    );
    assert_eq!(h.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_blocked_tracker_does_not_delay_the_normal_remote_collection() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let track = h.track();
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let remote = h.read();
    let local = track.read();
    h.finish(
        remote,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        now,
    );
    let due = now + COLLECTION_INTERVAL;
    assert_eq!(h.observer.next_deadline(now), Some(due));
    h.observer.poll(due, 2_000);
    let remote = h.read();
    h.finish(
        remote,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        due,
    );
    track.assert_calls(1);
    h.finish_tracking(local, Ok(()), due);
    assert_eq!(h.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn execution_toggle_cancels_only_old_tracking_and_passes_the_latest_permission() {
    let mut h = Harness::new(IssueCollectionMode::Off);
    let track = h.track();
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let old = track.read();
    assert!(!old.execution_enabled);
    h.observer.set_execution_enabled(true, now);
    assert!(
        h.observer
            .tracking_in_flight
            .as_ref()
            .unwrap()
            .cancel
            .is_cancelled()
    );
    h.finish_tracking(old, Ok(()), now);
    assert!(track.cancelled.load(Ordering::SeqCst));
    let latest = track.read();
    assert!(latest.execution_enabled);
    h.finish_tracking(latest, Ok(()), now);
    assert_eq!(h.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn turning_collection_off_does_not_cancel_existing_local_tracking() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let track = h.track();
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let remote = h.read();
    let local = track.read();
    h.observer
        .replace_projection(projection(IssueCollectionMode::Off, 2), now);
    assert!(
        !h.observer
            .tracking_in_flight
            .as_ref()
            .unwrap()
            .cancel
            .is_cancelled()
    );
    h.finish_tracking(local, Ok(()), now);
    assert!(!track.cancelled.load(Ordering::SeqCst));
    let next_local = track.read();
    h.finish(
        remote,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        now,
    );
    h.finish_tracking(next_local, Ok(()), now);
    assert_eq!(
        h.observer.view(now).projection.state.config.mode,
        IssueCollectionMode::Off
    );
    assert_eq!(h.observer.collection_deadline(now), None);
    assert_eq!(h.observer.next_deadline(now), Some(now + TRACKING_INTERVAL));
}

#[test]
fn a_panicked_tracker_wakes_the_host_without_changing_collection_backoff() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let remote = h.read();
    h.finish(remote, Err(anyhow::anyhow!("provider unavailable")), now);
    let remote_due = h.observer.collection_deadline(now);
    h.observer.track = Some(Arc::new(|_, _, _, _| panic!("synthetic tracker panic")));
    let _ = h.core.wait_pending_input(Duration::ZERO);
    h.observer.poll(now, 1_000);
    let _ = h.core.wait_pending_input(WAIT);
    let cycle = h.observer.poll(now, 1_000);
    assert!(!cycle.changed);
    assert!(cycle.committed.is_empty());
    assert!(h.observer.tracking_in_flight.is_none());
    assert_eq!(h.observer.next_deadline(now), Some(now + TRACKING_INTERVAL));
    assert_eq!(h.observer.collection_deadline(now), remote_due);
    assert_eq!(h.observer.consecutive_failures, 1);
}

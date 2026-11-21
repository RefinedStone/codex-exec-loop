//! 호스트 시계가 구동하는 이슈 관찰 작업이다 (#2538).
//! 원격 읽기는 한 작업 스레드에서만 수행하며, 주기 대기는 커널 호스트가 맡는다.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::application::service::issue_collection::{
    IssueCollectionCancellation, IssueCollectionRunOutcome, IssueCollectionService,
    IssueCollectionTrackedFact,
};
use crate::application::service::issue_triage::IssueTriagedFact;
use crate::composition::core_effect_worker::try_spawn_worker_with_panic_fallback;
use crate::core::runtime::CoreInputWaker;
use crate::domain::issue_automation::autonomy::IssueAutonomyProjection;
use crate::domain::issue_automation::collection::{IssueCollectionMode, IssueCollectionProjection};

const COLLECTION_INTERVAL: Duration = Duration::from_secs(5 * 60);
pub(crate) enum IssueObserverCommittedFact {
    Tracked(IssueCollectionTrackedFact),
    Triaged(IssueTriagedFact),
    Coordinated(crate::application::service::issue_coordinator::IssueCoordinatorFact),
}

type Reporter = dyn Fn(Vec<IssueObserverCommittedFact>) + Send + Sync;
type Collect = dyn Fn(&IssueCollectionCancellation, u64, bool, &Reporter) -> Result<IssueCollectionRunOutcome>
    + Send
    + Sync;

#[cfg(test)]
type TestCollect =
    dyn Fn(&IssueCollectionCancellation, u64) -> Result<IssueCollectionRunOutcome> + Send + Sync;

#[cfg(test)]
type TestCommittedCollect = dyn Fn(&IssueCollectionCancellation, u64, &Reporter) -> Result<IssueCollectionRunOutcome>
    + Send
    + Sync;

/// 영속 투영에 이번 커널의 작업 상태만 더한다. 원문이나 공급자 응답은 보유하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssueCollectionHostView {
    pub(crate) projection: IssueCollectionProjection,
    pub(crate) autonomy: IssueAutonomyProjection,
    pub(crate) collecting: bool,
    pub(crate) next_poll_after_ms: Option<u64>,
}

struct InFlight {
    epoch: u64,
    cancel: Arc<IssueCollectionCancellation>,
    completion: Receiver<Result<IssueCollectionRunOutcome>>,
}

pub(crate) struct IssueObserver {
    collect: Arc<Collect>,
    waker: CoreInputWaker,
    projection: IssueCollectionProjection,
    in_flight: Option<InFlight>,
    next_run: Option<Instant>,
    execution_enabled: bool,
    sync_pending: bool,
    consecutive_failures: u32,
    committed_sender: Sender<Vec<IssueObserverCommittedFact>>,
    committed_receiver: Receiver<Vec<IssueObserverCommittedFact>>,
}

/// 이번 주기에 투영 또는 작업 상태가 바뀌었는지와, 저장을 마친 결과다.
pub(crate) struct IssueObserverCycle {
    pub(crate) changed: bool,
    pub(crate) committed: Vec<IssueObserverCommittedFact>,
}

impl IssueObserver {
    pub(crate) fn new(service: Arc<IssueCollectionService>, waker: CoreInputWaker) -> Result<Self> {
        // 최초 Observe 행의 로컬 저장과 기존 Off 읽기를 마친 뒤에만 스케줄러를 만든다.
        let projection = service.projection()?;
        Ok(Self::with_collector(
            projection,
            Arc::new(move |cancel, at, execution_enabled, committed| {
                let tracked = |facts: Vec<IssueCollectionTrackedFact>| {
                    committed(
                        facts
                            .into_iter()
                            .map(IssueObserverCommittedFact::Tracked)
                            .collect(),
                    );
                };
                let mut outcome = service.collect_once_with_committed(cancel, at, &tracked)?;
                if !outcome.discarded
                    && !cancel.is_cancelled()
                    && let Some(triage) = service.triage_service()
                {
                    let triaged = |facts: Vec<IssueTriagedFact>| {
                        committed(
                            facts
                                .into_iter()
                                .map(IssueObserverCommittedFact::Triaged)
                                .collect(),
                        );
                    };
                    triage.triage_all(cancel, &triaged)?;
                }
                if !outcome.discarded
                    && !cancel.is_cancelled()
                    && let Some(coordinator) = service.coordinator_service()
                {
                    coordinator.tick(cancel, at, execution_enabled, &|facts| {
                        committed(
                            facts
                                .into_iter()
                                .map(IssueObserverCommittedFact::Coordinated)
                                .collect(),
                        );
                    })?;
                }
                if !cancel.is_cancelled() {
                    outcome.projection = service.projection()?;
                }
                Ok(outcome)
            }),
            waker,
        ))
    }

    fn with_collector(
        projection: IssueCollectionProjection,
        collect: Arc<Collect>,
        waker: CoreInputWaker,
    ) -> Self {
        let (committed_sender, committed_receiver) = mpsc::channel();
        Self {
            committed_sender,
            committed_receiver,
            consecutive_failures: projection.state.status.consecutive_failures,
            projection,
            collect,
            waker,
            in_flight: None,
            next_run: None,
            execution_enabled: false,
            sync_pending: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        projection: IssueCollectionProjection,
        collect: Arc<TestCollect>,
        waker: CoreInputWaker,
    ) -> Self {
        Self::with_collector(
            projection,
            Arc::new(move |cancel, at, _, _| collect(cancel, at)),
            waker,
        )
    }

    #[cfg(test)]
    pub(crate) fn for_test_with_committed(
        projection: IssueCollectionProjection,
        collect: Arc<TestCommittedCollect>,
        waker: CoreInputWaker,
    ) -> Self {
        Self::with_collector(
            projection,
            Arc::new(move |cancel, at, _, report| collect(cancel, at, report)),
            waker,
        )
    }

    pub(crate) fn view(&self, now: Instant) -> IssueCollectionHostView {
        IssueCollectionHostView {
            projection: self.projection.clone(),
            autonomy: IssueAutonomyProjection::Unavailable,
            collecting: self.in_flight.as_ref().is_some_and(|flight| {
                !flight.cancel.is_cancelled() && flight.epoch == self.projection.state.epoch
            }),
            next_poll_after_ms: self.next_deadline(now).map(|deadline| {
                deadline
                    .saturating_duration_since(now)
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64
            }),
        }
    }

    pub(crate) fn next_deadline(&self, now: Instant) -> Option<Instant> {
        (self.projection.state.config.mode == IssueCollectionMode::Observe
            && self.in_flight.is_none())
        .then_some(self.next_run.unwrap_or(now))
    }

    /// 진행 중인 수집에 추가 요청을 합친다. Off의 요청은 실행하지 않는다.
    pub(crate) fn set_execution_enabled(&mut self, enabled: bool, now: Instant) {
        if self.execution_enabled != enabled {
            self.execution_enabled = enabled;
            self.request_sync(now);
        }
    }

    pub(crate) fn request_sync(&mut self, now: Instant) -> bool {
        if self.projection.state.config.mode == IssueCollectionMode::Off {
            return false;
        }
        if self.in_flight.is_some() {
            self.sync_pending = true;
        }
        if self.in_flight.is_none() {
            self.next_run = Some(now);
            self.waker.wake();
        }
        true
    }

    /// 설정 저장 성공 뒤 호출한다. 취소한 스레드가 끝나기 전에는 새 스레드를 시작하지 않는다.
    pub(crate) fn replace_projection(
        &mut self,
        projection: IssueCollectionProjection,
        now: Instant,
    ) -> bool {
        if projection == self.projection {
            return false;
        }
        let configuration_changed = projection.state.epoch != self.projection.state.epoch
            || projection.state.config.mode != self.projection.state.config.mode;
        if configuration_changed {
            if let Some(flight) = &self.in_flight {
                flight.cancel.cancel();
            }
            self.next_run =
                (projection.state.config.mode == IssueCollectionMode::Observe).then_some(now);
            self.consecutive_failures = projection.state.status.consecutive_failures;
        }
        self.projection = projection;
        true
    }

    pub(crate) fn poll(&mut self, now: Instant, at_unix_ms: u64) -> IssueObserverCycle {
        let mut cycle = IssueObserverCycle {
            changed: false,
            committed: self.take_committed(),
        };
        let completion =
            self.in_flight
                .as_ref()
                .and_then(|flight| match flight.completion.try_recv() {
                    Ok(result) => Some(result),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => {
                        Some(Err(anyhow::anyhow!("issue collection worker stopped")))
                    }
                });
        if let Some(result) = completion {
            let flight = self
                .in_flight
                .take()
                .expect("completion has an active worker");
            cycle.changed = true;
            if !flight.cancel.is_cancelled() && flight.epoch == self.projection.state.epoch {
                match result {
                    Ok(outcome) => {
                        if outcome.discarded {
                            // 다른 설정 저장이 앞선 경우 그 저장된 설정만 반영한다. 신규 사건은 내지 않는다.
                            self.replace_projection(outcome.projection, now);
                        } else if outcome.projection.state.epoch >= flight.epoch {
                            self.projection = outcome.projection.clone();
                            if self.projection.state.status.last_failure.is_some() {
                                self.consecutive_failures =
                                    self.projection.state.status.consecutive_failures.max(1);
                                self.next_run =
                                    Some(now + failure_interval(self.consecutive_failures));
                            } else {
                                self.consecutive_failures = 0;
                                self.next_run = Some(now + COLLECTION_INTERVAL);
                            }
                        } else {
                            self.record_local_failure(now);
                        }
                    }
                    Err(_) => self.record_local_failure(now),
                }
            }
        }
        if self.in_flight.is_none()
            && std::mem::take(&mut self.sync_pending)
            && self.projection.state.config.mode == IssueCollectionMode::Observe
        {
            self.next_run = Some(now);
        }
        if self
            .next_deadline(now)
            .is_some_and(|deadline| deadline <= now)
        {
            self.start(now, at_unix_ms);
            cycle.changed = true;
        }
        cycle.committed.extend(self.take_committed());
        cycle.changed |= !cycle.committed.is_empty();
        cycle
    }

    /// 취소보다 먼저 끝난 commit은 완료 응답과 무관하게 감사 기록으로 옮긴다.
    pub(crate) fn cancel_and_take_committed(&mut self) -> Vec<IssueObserverCommittedFact> {
        if let Some(flight) = &self.in_flight {
            flight.cancel.cancel();
        }
        self.take_committed()
    }

    fn take_committed(&self) -> Vec<IssueObserverCommittedFact> {
        // 채널 순서를 보존한다. 종류별로 나누면 서로 다른 commit의 감사 순서가 바뀔 수 있다.
        self.committed_receiver.try_iter().flatten().collect()
    }

    fn start(&mut self, now: Instant, at_unix_ms: u64) {
        let cancel = Arc::new(IssueCollectionCancellation::new());
        let worker_cancel = cancel.clone();
        let collect = self.collect.clone();
        let execution_enabled = self.execution_enabled;
        let waker = self.waker.clone();
        let committed_sender = self.committed_sender.clone();
        let committed_waker = self.waker.clone();
        let (send, completion) = mpsc::channel();
        let epoch = self.projection.state.epoch;
        let panic_completion = send.clone();
        let panic_waker = self.waker.clone();
        let spawned = try_spawn_worker_with_panic_fallback(
            move || {
                let report = move |facts: Vec<IssueObserverCommittedFact>| {
                    if !facts.is_empty() && committed_sender.send(facts).is_ok() {
                        committed_waker.wake();
                    }
                };
                let result = collect(&worker_cancel, at_unix_ms, execution_enabled, &report);
                let _ = send.send(result);
                waker.wake();
            },
            move || {
                let _ =
                    panic_completion.send(Err(anyhow::anyhow!("issue collection worker panicked")));
                panic_waker.wake();
            },
        );
        match spawned {
            Ok(_detached_worker) => {
                self.in_flight = Some(InFlight {
                    epoch,
                    cancel,
                    completion,
                });
                self.next_run = None;
            }
            Err(_) => self.record_local_failure(now),
        }
    }

    fn record_local_failure(&mut self, now: Instant) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.next_run = Some(now + failure_interval(self.consecutive_failures));
        // DB와 작업 스레드 오류는 공급자 원문이나 로컬 경로를 표시하지 않는다.
        tracing::warn!("issue collection could not complete its local update");
    }
}

impl Drop for IssueObserver {
    fn drop(&mut self) {
        if let Some(flight) = &self.in_flight {
            flight.cancel.cancel();
        }
        // 네트워크 응답을 기다리며 join하지 않는다. 서비스가 취소와 DB epoch를 재검사한다.
    }
}

fn failure_interval(failures: u32) -> Duration {
    Duration::from_secs(match failures {
        0 => 5 * 60,
        1 => 10 * 60,
        2 => 20 * 60,
        3 => 40 * 60,
        _ => 60 * 60,
    })
}

#[cfg(test)]
mod tests;

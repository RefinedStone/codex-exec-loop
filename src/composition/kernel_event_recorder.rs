//! 커널의 판단 사건을 전용 스레드에서 차례로 기록한다 (plan 19 Q3-1). 커널 스레드는 SQLite 잠금을 기다리지 않는다.
//! 쓰기가 실패해도 커널은 멈추지 않는다. 기록기를 버리면 남은 사건을 모두 쓴 뒤에 끝난다.
//! 밀린 기록에는 바이트 한도가 있다 (#2301). 쓰지 못한 사건은 `history` 조회의 `recorder` 로 보인다 (#2300).

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::composition::core_effect_worker::spawn_worker_with_panic_fallback;
use crate::domain::kernel_event::{KernelRecorderHealth, RecordedKernelEvent};

/// 아직 쓰지 않은 묶음의 바이트 한도이다. 기록이 비어 있으면 이보다 큰 묶음 하나도 받는다.
const MAX_PENDING_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) struct KernelEventRecorder {
    batches: Option<Sender<(Vec<RecordedKernelEvent>, u64)>>,
    writer: Option<JoinHandle<()>>,
    health: Arc<Mutex<KernelRecorderHealth>>,
    max_pending_bytes: u64,
}

impl KernelEventRecorder {
    /// 그 커널의 작업 공간에 쓴다 (plan 19 Q3-2). 커널은 저장소 하나의 것이므로 모든 사건이 같은 기록에 간다. 복구도
    /// 같은 작업 공간에서 읽는다.
    pub(crate) fn start(log: Arc<dyn KernelEventLogPort>, workspace_dir: String) -> Self {
        Self::start_with_limit(log, workspace_dir, MAX_PENDING_BYTES)
    }

    fn start_with_limit(
        log: Arc<dyn KernelEventLogPort>,
        workspace_dir: String,
        max_pending_bytes: u64,
    ) -> Self {
        let health = Arc::new(Mutex::new(KernelRecorderHealth::default()));
        let (batches, received) = mpsc::channel::<(Vec<RecordedKernelEvent>, u64)>();
        let writer_health = health.clone();
        let panic_health = health.clone();
        let writer = spawn_worker_with_panic_fallback(
            move || {
                for (batch, bytes) in received {
                    let written = write_batch(log.as_ref(), &workspace_dir, &batch);
                    let mut health = locked(&writer_health);
                    health.pending_bytes = health.pending_bytes.saturating_sub(bytes);
                    if !written {
                        health.failed_events += batch.len() as u64;
                        health.degraded = true;
                    }
                    if health.pending_bytes == 0 {
                        health.saturated = false;
                    }
                }
            },
            move || {
                locked(&panic_health).degraded = true;
                tracing::warn!("kernel event recorder stopped after a panic");
            },
        );
        Self {
            batches: Some(batches),
            writer: Some(writer),
            health,
            max_pending_bytes,
        }
    }

    /// 커널 스레드는 기다리지 않는다. 밀린 기록이 한도를 넘으면 묶음을 버리고 그 사실을 상태에 남긴다.
    pub(crate) fn record(&self, records: Vec<RecordedKernelEvent>) {
        if records.is_empty() {
            return;
        }
        let Some(batches) = &self.batches else {
            return;
        };
        let bytes = serde_json::to_vec(&records).map_or(0, |json| json.len() as u64);
        let events = records.len() as u64;
        {
            let mut health = locked(&self.health);
            if health.pending_bytes > 0
                && health.pending_bytes.saturating_add(bytes) > self.max_pending_bytes
            {
                health.saturated = true;
                drop_events(&mut health, events);
                return;
            }
            health.pending_bytes += bytes;
        }
        if batches.send((records, bytes)).is_err() {
            let mut health = locked(&self.health);
            health.pending_bytes = health.pending_bytes.saturating_sub(bytes);
            drop_events(&mut health, events);
        }
    }

    pub(crate) fn health(&self) -> KernelRecorderHealth {
        locked(&self.health).clone()
    }
}

fn drop_events(health: &mut KernelRecorderHealth, events: u64) {
    health.dropped_events += events;
    health.degraded = true;
    tracing::warn!(
        events,
        pending_bytes = health.pending_bytes,
        "kernel event recorder dropped a batch"
    );
}

fn locked(health: &Mutex<KernelRecorderHealth>) -> MutexGuard<'_, KernelRecorderHealth> {
    health.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 한 dispatch 의 사건을 한 transaction 으로 쓴다. 실패하면 그 묶음을 버리고 다음 묶음을 쓴다.
fn write_batch(
    log: &dyn KernelEventLogPort,
    workspace_dir: &str,
    batch: &[RecordedKernelEvent],
) -> bool {
    let Err(error) = log.append_kernel_events(workspace_dir, batch) else {
        return true;
    };
    tracing::warn!(
        events = batch.len(),
        error_chars = error.to_string().chars().count(),
        "kernel event append failed"
    );
    false
}

/// 버릴 때 남은 사건을 다 쓰고 끝낸다. 커널은 런타임을 버린 뒤 저장소 잠금을 풀므로 다음 커널이 쓴 사건을 읽는다.
impl Drop for KernelEventRecorder {
    fn drop(&mut self) {
        drop(self.batches.take());
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::Receiver;

    use super::*;
    use crate::domain::kernel_event::KernelEvent;

    /// 처음 쓰기 한 번은 실패한다.
    #[derive(Default)]
    struct FlakyLog {
        stored: Mutex<Vec<(String, RecordedKernelEvent)>>,
        failed_once: Mutex<bool>,
    }

    impl KernelEventLogPort for FlakyLog {
        fn append_kernel_events(
            &self,
            workspace_dir: &str,
            events: &[RecordedKernelEvent],
        ) -> anyhow::Result<()> {
            let mut failed_once = self.failed_once.lock().expect("flag should lock");
            if !*failed_once {
                *failed_once = true;
                anyhow::bail!("the store is busy");
            }
            let mut stored = self.stored.lock().expect("log should lock");
            stored.extend(
                events
                    .iter()
                    .map(|event| (workspace_dir.to_string(), event.clone())),
            );
            Ok(())
        }

        fn load_kernel_events(
            &self,
            _workspace_dir: &str,
            _kinds: &[&str],
        ) -> anyhow::Result<Vec<KernelEvent>> {
            Ok(Vec::new())
        }
    }

    /// 쓰기 하나마다 시험이 허락할 때까지 기다리는 느린 기록이다.
    struct SlowLog {
        permits: Mutex<Receiver<()>>,
        stored: Mutex<Vec<RecordedKernelEvent>>,
    }

    impl KernelEventLogPort for SlowLog {
        fn append_kernel_events(
            &self,
            _workspace_dir: &str,
            events: &[RecordedKernelEvent],
        ) -> anyhow::Result<()> {
            let _ = self.permits.lock().expect("permits should lock").recv();
            self.stored
                .lock()
                .expect("log should lock")
                .extend_from_slice(events);
            Ok(())
        }

        fn load_kernel_events(
            &self,
            _workspace_dir: &str,
            _kinds: &[&str],
        ) -> anyhow::Result<Vec<KernelEvent>> {
            Ok(Vec::new())
        }
    }

    fn started(turn_id: &str) -> RecordedKernelEvent {
        RecordedKernelEvent {
            sequence: turn_id.len() as i64,
            recorded_at: "now".to_string(),
            event: KernelEvent::TurnStarted {
                thread_id: "thread-1".to_string(),
                turn_id: turn_id.to_string(),
                op_id: None,
            },
        }
    }

    #[test]
    fn dropping_the_recorder_writes_every_later_batch_in_order_past_a_failure() {
        let log = Arc::new(FlakyLog::default());
        let recorder = KernelEventRecorder::start(log.clone(), "/repo".to_string());
        let health = recorder.health.clone();
        recorder.record(vec![started("turn-1")]);
        recorder.record(vec![started("turn-2"), started("turn-3")]);
        drop(recorder);
        let stored = log.stored.lock().expect("log should lock");
        assert_eq!(
            stored.as_slice(),
            [
                ("/repo".to_string(), started("turn-2")),
                ("/repo".to_string(), started("turn-3")),
            ]
        );
        // #2300: the lost batch is visible as state, not only as a log line.
        assert_eq!(
            *locked(&health),
            KernelRecorderHealth {
                degraded: true,
                failed_events: 1,
                ..KernelRecorderHealth::default()
            }
        );
    }

    #[test]
    fn a_slow_log_saturates_the_bounded_backlog_without_blocking_the_kernel() {
        let (permit, permits) = mpsc::channel();
        let log = Arc::new(SlowLog {
            permits: Mutex::new(permits),
            stored: Mutex::new(Vec::new()),
        });
        let one_batch = serde_json::to_vec(&vec![started("turn-1")]).unwrap().len() as u64;
        // The limit holds two batches. The writer waits on the first batch.
        let recorder =
            KernelEventRecorder::start_with_limit(log.clone(), "/repo".to_string(), one_batch * 2);
        let health = recorder.health.clone();
        recorder.record(vec![started("turn-1")]);
        recorder.record(vec![started("turn-2")]);
        recorder.record(vec![started("turn-3")]);
        assert_eq!(
            recorder.health(),
            KernelRecorderHealth {
                degraded: true,
                saturated: true,
                pending_bytes: one_batch * 2,
                failed_events: 0,
                dropped_events: 1,
            }
        );
        permit.send(()).unwrap();
        permit.send(()).unwrap();
        drop(recorder);
        assert_eq!(
            log.stored.lock().expect("log should lock").as_slice(),
            [started("turn-1"), started("turn-2")]
        );
        // The drained backlog is no longer saturated. The lost batch keeps recovery degraded.
        assert_eq!(
            *locked(&health),
            KernelRecorderHealth {
                degraded: true,
                dropped_events: 1,
                ..KernelRecorderHealth::default()
            }
        );
    }
}

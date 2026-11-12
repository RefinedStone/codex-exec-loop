//! 커널의 판단 사건을 전용 스레드에서 차례로 기록한다 (plan 19 Q3-1). 커널 스레드는 SQLite 잠금을 기다리지 않는다.
//! 쓰기가 실패해도 커널은 멈추지 않는다. 기록기를 버리면 남은 사건을 모두 쓴 뒤에 끝난다.

use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::composition::core_effect_worker::spawn_worker_with_panic_fallback;
use crate::domain::kernel_event::RecordedKernelEvent;

pub(crate) struct KernelEventRecorder {
    batches: Option<Sender<Vec<RecordedKernelEvent>>>,
    writer: Option<JoinHandle<()>>,
}

impl KernelEventRecorder {
    /// 그 커널의 작업 공간에 쓴다 (plan 19 Q3-2). 커널은 저장소 하나의 것이므로 모든 사건이 같은 기록에 간다. 복구도
    /// 같은 작업 공간에서 읽는다.
    pub(crate) fn start(log: Arc<dyn KernelEventLogPort>, workspace_dir: String) -> Self {
        let (batches, received) = mpsc::channel::<Vec<RecordedKernelEvent>>();
        let writer = spawn_worker_with_panic_fallback(
            move || {
                for batch in received {
                    write_batch(log.as_ref(), &workspace_dir, &batch);
                }
            },
            || tracing::warn!("kernel event recorder stopped after a panic"),
        );
        Self {
            batches: Some(batches),
            writer: Some(writer),
        }
    }

    pub(crate) fn record(&self, records: Vec<RecordedKernelEvent>) {
        if records.is_empty() {
            return;
        }
        if let Some(batches) = &self.batches {
            let _ = batches.send(records);
        }
    }
}

/// 한 dispatch 의 사건을 한 transaction 으로 쓴다. 실패하면 그 묶음을 버리고 다음 묶음을 쓴다.
fn write_batch(log: &dyn KernelEventLogPort, workspace_dir: &str, batch: &[RecordedKernelEvent]) {
    if let Err(error) = log.append_kernel_events(workspace_dir, batch) {
        tracing::warn!(
            events = batch.len(),
            error_chars = error.to_string().chars().count(),
            "kernel event append failed"
        );
    }
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
    use std::sync::Mutex;

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
    }
}

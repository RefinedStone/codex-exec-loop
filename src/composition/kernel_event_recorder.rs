//! 커널의 판단 사건을 전용 스레드에서 차례로 기록한다 (plan 19 Q3-1). 커널 스레드는 SQLite 잠금을 기다리지 않는다.
//! 쓰기가 실패해도 커널은 멈추지 않는다. 기록기를 버리면 남은 사건을 모두 쓴 뒤에 끝난다.

use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::composition::core_effect_worker::spawn_worker_with_panic_fallback;
use crate::domain::kernel_event::KernelEvent;

pub(crate) struct KernelEventRecorder {
    batches: Option<Sender<Vec<KernelEvent>>>,
    writer: Option<JoinHandle<()>>,
}

impl KernelEventRecorder {
    pub(crate) fn start(log: Arc<dyn KernelEventLogPort>) -> Self {
        let (batches, received) = mpsc::channel::<Vec<KernelEvent>>();
        let writer = spawn_worker_with_panic_fallback(
            move || {
                for batch in received {
                    write_batch(log.as_ref(), &batch);
                }
            },
            || tracing::warn!("kernel event recorder stopped after a panic"),
        );
        Self {
            batches: Some(batches),
            writer: Some(writer),
        }
    }

    pub(crate) fn record(&self, events: Vec<KernelEvent>) {
        if events.is_empty() {
            return;
        }
        if let Some(batches) = &self.batches {
            let _ = batches.send(events);
        }
    }
}

/// 사건마다 그 작업 공간의 저장소에 쓴다. v0 의 사건은 한 묶음에 하나뿐이라 사건 단위로 쓴다.
fn write_batch(log: &dyn KernelEventLogPort, batch: &[KernelEvent]) {
    for event in batch {
        if let Err(error) =
            log.append_kernel_events(event.workspace_directory(), std::slice::from_ref(event))
        {
            tracing::warn!(
                kind = event.kind(),
                error_chars = error.to_string().chars().count(),
                "kernel event append failed"
            );
        }
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
    use crate::domain::kernel_event::{QueuedThreadPrompt, ThreadPromptSettlement};

    #[derive(Default)]
    struct MemoryLog(Mutex<Vec<(String, KernelEvent)>>);

    impl KernelEventLogPort for MemoryLog {
        fn append_kernel_events(
            &self,
            workspace_dir: &str,
            events: &[KernelEvent],
        ) -> anyhow::Result<()> {
            if workspace_dir == "/broken" {
                anyhow::bail!("the store is unavailable");
            }
            let mut stored = self.0.lock().expect("log should lock");
            stored.extend(
                events
                    .iter()
                    .map(|event| (workspace_dir.to_string(), event.clone())),
            );
            Ok(())
        }

        fn load_kernel_events(&self, _workspace_dir: &str) -> anyhow::Result<Vec<KernelEvent>> {
            Ok(Vec::new())
        }
    }

    fn event(workspace: &str, task_id: &str) -> KernelEvent {
        QueuedThreadPrompt {
            thread_id: "thread-1".to_string(),
            workspace_directory: workspace.to_string(),
            handoff: crate::domain::planning::ManualPromptMainSessionHandoff {
                prompt: "planned prompt".to_string(),
                transcript_text: "fix the test".to_string(),
                task: None,
            },
            task_id: task_id.to_string(),
        }
        .settled(ThreadPromptSettlement::Submitted)
    }

    #[test]
    fn dropping_the_recorder_writes_every_event_in_order_past_a_failure() {
        let log = Arc::new(MemoryLog::default());
        let recorder = KernelEventRecorder::start(log.clone());
        recorder.record(vec![event("/a", "task-1"), event("/broken", "task-2")]);
        recorder.record(vec![event("/b", "task-3")]);
        drop(recorder);
        let stored = log.0.lock().expect("log should lock");
        assert_eq!(
            stored.as_slice(),
            [
                ("/a".to_string(), event("/a", "task-1")),
                ("/b".to_string(), event("/b", "task-3")),
            ]
        );
    }
}

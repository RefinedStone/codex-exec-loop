//! live와 command-only 커널이 같은 sequence 공간에서 증거 outbox를 전달한다.
use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::application::service::task_evidence::TaskEvidenceService;
use crate::domain::kernel_event::{KernelEvent, RecordedKernelEvent};

pub(super) const TASK_EVIDENCE_OUTBOX_BATCH: usize = 64;

#[derive(Default)]
pub(super) struct TaskEvidenceOutboxDrain {
    /// append가 확정된 새 사건만 공개한다. ACK 실패 시에도 다음 재시도에서 다시 공개하지 않는다.
    pub records: Vec<RecordedKernelEvent>,
    pub retry: bool,
    pub more: bool,
}

pub(super) fn drain(
    service: &TaskEvidenceService,
    log: &dyn KernelEventLogPort,
    workspace: &str,
    last_sequence: &mut i64,
) -> TaskEvidenceOutboxDrain {
    let mut result = TaskEvidenceOutboxDrain::default();
    let pending = match service.pending_events(TASK_EVIDENCE_OUTBOX_BATCH) {
        Ok(pending) => pending,
        Err(_) => {
            result.retry = true;
            return result;
        }
    };
    let full_page = pending.len() >= TASK_EVIDENCE_OUTBOX_BATCH;
    for event in pending.into_iter().take(TASK_EVIDENCE_OUTBOX_BATCH) {
        let attempt = (|| -> anyhow::Result<()> {
            event.validate()?;
            let record_event = KernelEvent::task_evidence(&event);
            if !log.has_task_evidence_event(workspace, &record_event)? {
                let sequence = last_sequence
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("kernel event sequence exhausted"))?;
                // 실패한 sequence는 재사용하지 않는다. 다른 커널 기록과 같은 allocator다.
                *last_sequence = sequence;
                let record = RecordedKernelEvent {
                    sequence,
                    recorded_at: chrono::Utc::now()
                        .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
                    event: record_event,
                };
                log.append_kernel_events(workspace, std::slice::from_ref(&record))?;
                result.records.push(record);
            }
            service.acknowledge_event(event.event_id())?;
            Ok(())
        })();
        if attempt.is_err() {
            result.retry = true;
            return result;
        }
    }
    result.more = full_page;
    result
}

#[cfg(test)]
pub(crate) mod tests;

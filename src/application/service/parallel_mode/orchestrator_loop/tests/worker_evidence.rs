use super::*;
use crate::adapter::outbound::filesystem::FilesystemTaskEvidenceArtifactAdapter;
use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
use crate::application::service::parallel_mode::worker_evidence::EVIDENCE_PREPARATION_NOTICE;
use std::sync::atomic::{AtomicUsize, Ordering};

struct RecordingWorker(AtomicUsize);

impl ParallelAgentWorkerPort for RecordingWorker {
    fn run_isolated_new_thread_stream(
        &self,
        request: ParallelAgentWorkerStreamRequest<'_>,
        _events: crate::application::port::conversation_stream::ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        assert_eq!(request.evidence_incoming_root, None);
        self.0.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("fixture worker stops after observing its launch request")
    }
}

#[test]
fn unavailable_evidence_preparation_emits_a_sanitized_notice_and_still_launches_the_worker() {
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let artifacts = Arc::new(FilesystemTaskEvidenceArtifactAdapter::new(Arc::new(|_| {
        anyhow::bail!("PRIVATE-RUNTIME-PATH must remain private")
    })));
    let parallel = test_parallel_service(authority.clone())
        .with_task_evidence(Arc::new(NoopPlanningTaskRepositoryPort), artifacts);
    let worker = Arc::new(RecordingWorker(AtomicUsize::new(0)));
    let outcome = run_parallel_dispatch_worker(
        worker_request_with_secret_bodies(),
        worker.clone(),
        ParallelModeTurnService::new(parallel),
        test_planning_services(authority),
    );
    assert_eq!(worker.0.load(Ordering::SeqCst), 1);
    assert!(
        outcome
            .notices
            .iter()
            .any(|notice| notice == EVIDENCE_PREPARATION_NOTICE)
    );
    assert!(
        outcome
            .notices
            .iter()
            .all(|notice| !notice.contains("PRIVATE-RUNTIME-PATH"))
    );
}

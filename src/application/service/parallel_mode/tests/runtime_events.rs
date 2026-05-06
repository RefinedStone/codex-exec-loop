use super::*;
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogRequest;

// runtime event snapshot은 supervisor current-state projection과 별개로 authority store에 쌓인
// projection 변경 이력을 bounded feed로 읽는다. slot lease가 assigned -> running으로 바뀌면 같은
// slot key의 최신 이벤트가 먼저 보여야 UI timeline이 현재 흐름을 잃지 않는다.
#[test]
fn build_runtime_events_snapshot_reads_bounded_slot_timeline() {
    let repo = TempGitRepo::new("runtime-events-service");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");

    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should transition to running");

    let snapshot = service.build_runtime_events_snapshot(
        &repo.workspace_dir(),
        ParallelModeRuntimeEventLogRequest::for_projection("slot_lease", lease.slot_id, 2),
    );

    assert_eq!(snapshot.total_event_count, 2);
    assert_eq!(snapshot.visible_count(), 2);
    assert_eq!(snapshot.entries[0].event_kind, "slot_lease_upsert");
    assert_eq!(snapshot.entries[0].projection_kind, "slot_lease");
    assert_eq!(snapshot.entries[0].projection_key, "slot-1");
    assert!(snapshot.entries[0].summary.contains("state: running"));
    assert!(snapshot.entries[1].summary.contains("state: leased"));
}

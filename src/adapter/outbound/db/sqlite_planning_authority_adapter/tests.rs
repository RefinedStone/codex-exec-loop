/*
SQLite planning authority adapter가 application port의 snapshot 계약을 실제 DB 저장소 위에서도
지키는지 검증한다. service 계층은 `PlanningTaskRepositoryPort`만 보므로, 테스트는 concrete adapter를
포트 메서드로 호출해 task authority 문서와 queue projection의 동시 round-trip을 고정한다.
*/
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::domain::parallel_mode::ParallelModeAgentSessionDetailSnapshot;
use crate::domain::planning::{PriorityQueueProjection, TaskAuthorityDocument};

// 테스트마다 SQLite namespace를 분리하는 workspace directory를 만든다. adapter가 workspace path를
// DB 파일/row scope의 기준으로 쓰므로, 프로세스 id와 nanos를 섞어 병렬 테스트 충돌을 피한다.
fn temp_workspace(prefix: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "codex-exec-loop-db-{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be valid")
            .as_nanos()
    ));
    // SQLite adapter가 workspace 아래에 database 파일을 열 수 있도록 directory를 먼저 만든다.
    // 실패는 테스트 환경 문제이므로 expect로 즉시 드러낸다.
    std::fs::create_dir_all(&path).expect("workspace should create");
    path.display().to_string()
}

#[test]
fn task_authority_snapshot_is_committed_to_db_tables() {
    // 빈 workspace로 시작해야 commit path가 schema bootstrap, initial revision 생성, table insert를
    // 모두 지난다. 기존 DB를 재사용하면 load-only 또는 update-only 경로만 검증할 위험이 있다.
    let workspace_dir = temp_workspace("workspace");
    // concrete adapter를 만들지만 아래 호출은 PlanningTaskRepositoryPort 메서드다. application boundary에서
    // 기대하는 포트 계약이 실제 SQLite 구현에서도 유지되는지 확인한다.
    let adapter = SqlitePlanningAuthorityAdapter::new();
    // 최소 task authority 문서로 round-trip을 검증한다. 내용이 비어 있어도 version과 tasks 배열이
    // DB 직렬화/역직렬화 후 같은 domain value로 돌아와야 한다.
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    // queue_projection은 task_authority와 같은 revision으로 저장되어야 하는 실행 관점 투영이다.
    // 빈 projection도 next/active/proposed/skipped 필드가 누락 없이 DB snapshot에 남는지 확인한다.
    let queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

    // observed_planning_revision이 None인 첫 commit은 lost-update 검사를 건너뛰고 새 snapshot을 쓴다.
    // 이 호출이 성공하면 adapter는 schema 준비, transaction, JSON 저장, revision 발급까지 완료해야 한다.
    adapter
        .commit_task_authority_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &task_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("task authority should commit");

    // 같은 adapter/workspace에서 다시 읽어야 persistence boundary를 통과한다. 반환값이 None이면 commit이
    // table에 snapshot을 남기지 못한 것이고, Some이어도 아래 equality가 직렬화 손실을 잡는다.
    let snapshot = adapter
        .load_task_authority_snapshot(&workspace_dir)
        .expect("task authority should load")
        .expect("snapshot should exist");

    // task_authority와 queue_projection을 따로 비교해 "문서만 저장됨" 또는 "큐 투영만 저장됨" 같은 반쪽
    // 성공을 막는다. 두 값이 같은 snapshot으로 돌아와야 planning runtime과 repair flow가 같은 authority를 본다.
    assert_eq!(snapshot.task_authority, task_authority);
    assert_eq!(snapshot.queue_projection, queue_projection);
}

#[test]
fn runtime_reset_preserves_latest_failed_start_dispatch_block_per_task() {
    let workspace_dir = temp_workspace("failed-start-blocks");
    let adapter = SqlitePlanningAuthorityAdapter::new();

    adapter
        .upsert_runtime_session_detail(
            &workspace_dir,
            &failed_start_session_detail("session-new", "task-1", "2026-05-04T12:00:00+00:00"),
        )
        .expect("newer failed-start detail should persist");
    adapter
        .upsert_runtime_session_detail(
            &workspace_dir,
            &failed_start_session_detail("session-old", "task-1", "2026-05-04T11:00:00+00:00"),
        )
        .expect("older failed-start detail should persist");

    adapter
        .clear_parallel_runtime_projections(&workspace_dir, "test reset")
        .expect("runtime projections should clear");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(snapshot.session_details.len(), 0);
    assert_eq!(snapshot.task_dispatch_blocks.len(), 1);
    let block = &snapshot.task_dispatch_blocks[0];
    assert_eq!(block.task_id, "task-1");
    assert_eq!(block.blocked_at, "2026-05-04T12:00:00+00:00");
}

#[test]
fn runtime_projection_loads_recent_runtime_event_feed_newest_first() {
    let workspace_dir = temp_workspace("runtime-events");
    let adapter = SqlitePlanningAuthorityAdapter::new();

    for index in 1..=10 {
        adapter
            .upsert_runtime_session_detail(
                &workspace_dir,
                &failed_start_session_detail(
                    &format!("session-{index:02}"),
                    &format!("task-{index:02}"),
                    &format!("2026-05-04T12:{index:02}:00+00:00"),
                ),
            )
            .expect("session detail event should persist");
    }

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");

    assert_eq!(snapshot.runtime_events.len(), 8);
    assert_eq!(snapshot.runtime_events[0].sequence, 10);
    assert_eq!(
        snapshot.runtime_events[0].event_kind,
        "session_detail_upsert"
    );
    assert_eq!(snapshot.runtime_events[0].projection_kind, "session_detail");
    assert_eq!(snapshot.runtime_events[0].projection_key, "session-10");
    assert_eq!(snapshot.runtime_events[0].observed_planning_revision, 0);
    assert!(snapshot.runtime_events[0].summary.contains("state: failed"));
    assert_eq!(snapshot.runtime_events[7].sequence, 3);
}

fn failed_start_session_detail(
    session_key: &str,
    task_id: &str,
    updated_at: &str,
) -> ParallelModeAgentSessionDetailSnapshot {
    ParallelModeAgentSessionDetailSnapshot::new(
        session_key,
        "agent-1",
        task_id,
        "Task One",
        "slot-1",
        None,
        "/tmp/worktree",
        "akra-agent/slot-1/task-one",
        "2026-05-04T10:00:00+00:00",
        "failed",
        "aborted",
        "launch failed before the session reached the running state",
        "validation unavailable",
        "startup failed",
        None,
        Vec::new(),
        updated_at,
    )
}

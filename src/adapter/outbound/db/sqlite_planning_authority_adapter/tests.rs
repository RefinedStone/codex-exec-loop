/*
SQLite planning authority adapter가 application port의 snapshot 계약을 실제 DB 저장소 위에서도
지키는지 검증한다. service 계층은 `PlanningTaskRepositoryPort`만 보므로, 테스트는 concrete adapter를
포트 메서드로 호출해 task authority 문서와 queue projection의 동시 round-trip을 고정한다.
*/
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::{
    ParallelModeRuntimeEventLogPort, ParallelModeRuntimeEventLogRequest,
};
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
    PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningWorkspaceLoadRecord,
};
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAutomationTrigger,
    ParallelModeDispatchBlockReason, ParallelModeDispatchCommandSnapshot,
    ParallelModeDispatchCommandState, ParallelModeQueueItemState, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState, ParallelModeTaskDispatchBlockSnapshot,
};
use crate::domain::planning::{
    OriginSessionKind, PriorityQueueProjection, TaskActor, TaskAuthorityDocument, TaskDefinition,
    TaskMutationProvenance, TaskStatus,
};

use super::open_authority_connection;

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
fn task_authority_snapshot_persists_queryable_provenance_columns() {
    let workspace_dir = temp_workspace("task-provenance");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let provenance = TaskMutationProvenance::new(OriginSessionKind::Planner)
        .with_thread_turn(
            Some("worker-thread-1".to_string()),
            Some("worker-turn-1".to_string()),
        )
        .with_parent(
            Some("main-thread-1".to_string()),
            Some("main-turn-1".to_string()),
        );
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![TaskDefinition {
            id: "task-provenance-1".to_string(),
            direction_id: "direction-1".to_string(),
            direction_relation_note: "covers provenance storage".to_string(),
            title: "Persist provenance".to_string(),
            description: "Persist generic provenance columns.".to_string(),
            status: TaskStatus::Ready,
            base_priority: 80,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::Worker,
            last_updated_by: TaskActor::Worker,
            source_turn_id: Some("worker-turn-1".to_string()),
            provenance,
            updated_at: "2026-05-07T09:00:00Z".to_string(),
        }],
    };
    let queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

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

    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace_dir)
            .expect("authority location should resolve");
    let connection = open_authority_connection(&location).expect("authority db should open");
    let row = connection
        .query_row(
            "SELECT origin_session_kind, thread_id, turn_id, parent_thread_id, parent_turn_id
             FROM planning_tasks WHERE task_id = 'task-provenance-1'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .expect("provenance row should load");

    assert_eq!(
        row,
        (
            "planner".to_string(),
            "worker-thread-1".to_string(),
            "worker-turn-1".to_string(),
            "main-thread-1".to_string(),
            "main-turn-1".to_string(),
        )
    );
}

#[test]
fn active_workspace_artifact_removal_preserves_task_authority_snapshot() {
    let workspace_dir = temp_workspace("active-artifact-preserves-authority");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

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
    SqlitePlanningAuthorityAdapter::commit_active_workspace_files(
        &workspace_dir,
        &PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("operator result output".to_string()),
        },
    )
    .expect("active workspace artifact should commit");
    assert_eq!(
        SqlitePlanningAuthorityAdapter::load_active_planning_file(
            &workspace_dir,
            RESULT_OUTPUT_FILE_PATH,
        )
        .expect("active artifact should load")
        .as_deref(),
        Some("operator result output")
    );

    SqlitePlanningAuthorityAdapter::replace_active_planning_file(
        &workspace_dir,
        RESULT_OUTPUT_FILE_PATH,
        None,
    )
    .expect("active workspace artifact should be removable");

    assert!(
        !SqlitePlanningAuthorityAdapter::load_active_workspace_files(&workspace_dir)
            .expect("active workspace should load after artifact removal")
            .has_any_files()
    );
    let snapshot = adapter
        .load_task_authority_snapshot(&workspace_dir)
        .expect("task authority should still load")
        .expect("task authority snapshot should remain accepted authority");
    assert_eq!(snapshot.task_authority, task_authority);
    assert_eq!(snapshot.queue_projection, queue_projection);
}

#[test]
fn staged_draft_rows_do_not_mutate_active_workspace_or_task_authority_snapshot() {
    let workspace_dir = temp_workspace("draft-preserves-authority");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

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
    SqlitePlanningAuthorityAdapter::commit_active_workspace_files(
        &workspace_dir,
        &PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("active result output".to_string()),
        },
    )
    .expect("active workspace artifact should commit");

    SqlitePlanningAuthorityAdapter::stage_repo_scoped_draft_files(
        &workspace_dir,
        "draft-one",
        &[PlanningDraftFileRecord {
            active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
            body: "draft result output".to_string(),
        }],
    )
    .expect("draft artifact should stage");

    assert_eq!(
        SqlitePlanningAuthorityAdapter::load_active_planning_file(
            &workspace_dir,
            RESULT_OUTPUT_FILE_PATH,
        )
        .expect("active artifact should load")
        .as_deref(),
        Some("active result output")
    );
    let snapshot = adapter
        .load_task_authority_snapshot(&workspace_dir)
        .expect("task authority should still load")
        .expect("task authority snapshot should remain accepted authority");
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
fn runtime_dispatch_command_enqueue_claim_and_update_round_trips() {
    let workspace_dir = temp_workspace("dispatch-command");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("queue-head-1".to_string()),
        Some(11),
        "2026-05-08T00:00:00+00:00",
    );

    assert!(
        adapter
            .enqueue_runtime_dispatch_command(&workspace_dir, &command)
            .expect("command should enqueue")
    );
    assert!(
        !adapter
            .enqueue_runtime_dispatch_command(&workspace_dir, &command)
            .expect("duplicate command should not enqueue")
    );

    let claimed = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-1")
        .expect("command claim should succeed")
        .expect("pending command should be claimed");
    assert_eq!(claimed.command_id, command.command_id);
    assert_eq!(claimed.state, ParallelModeDispatchCommandState::Running);
    assert_eq!(claimed.owner_token.as_deref(), Some("owner-1"));
    assert!(
        adapter
            .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-2")
            .expect("second claim should inspect cleanly")
            .is_none()
    );

    let mut completed = claimed;
    completed.mark_completed("launched workers", "2026-05-08T00:00:10+00:00");
    adapter
        .update_runtime_dispatch_command(&workspace_dir, &completed)
        .expect("completed command should persist");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(snapshot.dispatch_commands.len(), 1);
    assert_eq!(
        snapshot.dispatch_commands[0].state,
        ParallelModeDispatchCommandState::Completed
    );
    assert_eq!(
        snapshot.dispatch_commands[0].status_detail.as_deref(),
        Some("launched workers")
    );
}

#[test]
fn runtime_dispatch_command_cancel_marks_only_non_terminal_commands() {
    let workspace_dir = temp_workspace("dispatch-command-cancel");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let running_command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        Some("queue-head-running".to_string()),
        Some(21),
        "2026-05-08T00:00:00+00:00",
    );
    let completed_command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("queue-head-completed".to_string()),
        Some(22),
        "2026-05-08T00:00:01+00:00",
    );
    let pending_command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("queue-head-pending".to_string()),
        Some(23),
        "2026-05-08T00:00:02+00:00",
    );

    adapter
        .enqueue_runtime_dispatch_command(&workspace_dir, &running_command)
        .expect("running seed should enqueue");
    let running = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-running")
        .expect("running command should claim")
        .expect("running command should exist");

    adapter
        .enqueue_runtime_dispatch_command(&workspace_dir, &completed_command)
        .expect("completed seed should enqueue");
    let mut completed = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-completed")
        .expect("completed command should claim")
        .expect("completed command should exist");
    completed.mark_completed("already launched workers", "2026-05-08T00:00:03+00:00");
    adapter
        .update_runtime_dispatch_command(&workspace_dir, &completed)
        .expect("completed command should persist");

    adapter
        .enqueue_runtime_dispatch_command(&workspace_dir, &pending_command)
        .expect("pending seed should enqueue");

    let canceled = adapter
        .cancel_runtime_dispatch_commands(&workspace_dir, "parallel mode disabled")
        .expect("non-terminal commands should cancel");
    assert_eq!(canceled, 2);

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(
        snapshot
            .dispatch_commands
            .iter()
            .find(|command| command.command_id == running.command_id)
            .map(|command| command.state),
        Some(ParallelModeDispatchCommandState::Canceled)
    );
    assert_eq!(
        snapshot
            .dispatch_commands
            .iter()
            .find(|command| command.command_id == completed.command_id)
            .map(|command| command.state),
        Some(ParallelModeDispatchCommandState::Completed)
    );
    assert_eq!(
        snapshot
            .dispatch_commands
            .iter()
            .find(|command| command.command_id == pending_command.command_id)
            .map(|command| command.state),
        Some(ParallelModeDispatchCommandState::Canceled)
    );
}

#[test]
fn runtime_task_cleanup_removes_deleted_task_projections_only() {
    let workspace_dir = temp_workspace("runtime-task-cleanup");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let deleted_lease = slot_lease_for_task(
        "slot-1",
        "task-deleted",
        ParallelModeSlotLeaseState::Running,
    );
    let kept_lease =
        slot_lease_for_task("slot-2", "task-kept", ParallelModeSlotLeaseState::Running);

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &deleted_lease)
        .expect("deleted task slot lease should persist");
    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &kept_lease)
        .expect("kept task slot lease should persist");
    adapter
        .upsert_runtime_session_detail(
            &workspace_dir,
            &failed_start_session_detail(
                "session-deleted",
                "task-deleted",
                "2026-05-04T12:00:00+00:00",
            ),
        )
        .expect("deleted task session should persist");
    adapter
        .upsert_runtime_session_detail(
            &workspace_dir,
            &failed_start_session_detail("session-kept", "task-kept", "2026-05-04T12:01:00+00:00"),
        )
        .expect("kept task session should persist");
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace_dir,
            &queue_record_for_task("queue-deleted", "session-deleted", "task-deleted"),
        )
        .expect("deleted task queue record should persist");
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace_dir,
            &queue_record_for_task("queue-kept", "session-kept", "task-kept"),
        )
        .expect("kept task queue record should persist");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-deleted", "owner")
            .expect("queue claim should be acquired")
    );

    adapter
        .clear_parallel_runtime_projections_for_tasks(
            &workspace_dir,
            &["task-deleted".to_string()],
            "test task delete",
        )
        .expect("deleted task runtime projections should clear");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(
        snapshot
            .slot_leases
            .values()
            .map(|lease| lease.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-kept"]
    );
    assert_eq!(snapshot.session_details.len(), 1);
    assert_eq!(snapshot.session_details[0].task_id, "task-kept");
    assert_eq!(snapshot.distributor_queue_records.len(), 1);
    assert_eq!(snapshot.distributor_queue_records[0].task_id, "task-kept");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-deleted", "owner-2")
            .expect("deleted queue claim should be cleared")
    );
}

#[test]
fn runtime_projection_snapshot_groups_current_rows_and_recent_events() {
    let workspace_dir = temp_workspace("runtime-projection-matrix");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let lease = slot_lease_for_task("slot-1", "task-1", ParallelModeSlotLeaseState::Running);
    let session = failed_start_session_detail("session-1", "task-1", "2026-05-04T12:00:00+00:00");
    let block = ParallelModeTaskDispatchBlockSnapshot::new(
        "task-1",
        "2026-05-04T11:55:00+00:00",
        "2026-05-04T12:00:00+00:00",
        ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
    );
    let queue_record = queue_record_for_task("queue-1", "session-1", "task-1");
    let command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("task-1:ready".to_string()),
        Some(31),
        "2026-05-08T00:00:00+00:00",
    );

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &lease)
        .expect("slot lease should persist");
    adapter
        .upsert_runtime_session_detail(&workspace_dir, &session)
        .expect("session detail should persist");
    adapter
        .upsert_runtime_task_dispatch_block(&workspace_dir, &block)
        .expect("task dispatch block should persist");
    adapter
        .upsert_runtime_distributor_queue_record(&workspace_dir, &queue_record)
        .expect("distributor queue record should persist");
    adapter
        .enqueue_runtime_dispatch_command(&workspace_dir, &command)
        .expect("dispatch command should persist");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(snapshot.slot_leases.get("slot-1"), Some(&lease));
    assert_eq!(snapshot.session_details, vec![session]);
    assert_eq!(snapshot.task_dispatch_blocks, vec![block]);
    assert_eq!(snapshot.distributor_queue_records, vec![queue_record]);
    assert_eq!(snapshot.dispatch_commands, vec![command]);

    let event_kinds = snapshot
        .runtime_events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect::<Vec<_>>();
    assert!(event_kinds.contains(&"slot_lease_upsert"));
    assert!(event_kinds.contains(&"session_detail_upsert"));
    assert!(event_kinds.contains(&"task_dispatch_block_upsert"));
    assert!(event_kinds.contains(&"distributor_queue_upsert"));
    assert!(event_kinds.contains(&"dispatch_command_enqueued"));
}

#[test]
fn official_refresh_claim_orders_are_enforced_by_authority_store() {
    let workspace_dir = temp_workspace("official-refresh-claims");
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let first_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("first order should reserve");
    let second_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("second order should reserve");
    assert_eq!(first_order, 1);
    assert_eq!(second_order, 2);

    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, second_order, "owner-2")
            .expect("later order should inspect"),
        PlanningAuthorityOfficialRefreshClaimStatus::Waiting
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, first_order, "owner-1")
            .expect("head order should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, first_order, "owner-other")
            .expect("competing owner should wait"),
        PlanningAuthorityOfficialRefreshClaimStatus::Waiting
    );

    adapter
        .release_official_refresh_claim(&workspace_dir, first_order, "owner-1")
        .expect("first order should release");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, first_order, "owner-1")
            .expect("completed order should inspect"),
        PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, second_order, "owner-2")
            .expect("next order should acquire after first release"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
}

#[test]
fn runtime_event_log_port_reads_recent_projection_events() {
    let workspace_dir = temp_workspace("runtime-events");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let first = slot_lease("slot-1", ParallelModeSlotLeaseState::Leased);
    let second = slot_lease("slot-1", ParallelModeSlotLeaseState::Running);

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &first)
        .expect("first slot lease event should persist");
    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &second)
        .expect("second slot lease event should persist");

    let snapshot = adapter
        .load_runtime_event_log(
            &workspace_dir,
            ParallelModeRuntimeEventLogRequest::for_projection("slot_lease", "slot-1", 1),
        )
        .expect("runtime event log should load");

    assert_eq!(snapshot.total_event_count, 2);
    assert_eq!(snapshot.visible_count(), 1);
    let latest = snapshot.latest().expect("latest event should be visible");
    assert_eq!(latest.sequence, 2);
    assert_eq!(latest.event_kind, "slot_lease_upsert");
    assert_eq!(latest.projection_kind, "slot_lease");
    assert_eq!(latest.projection_key, "slot-1");
    assert!(
        latest
            .summary
            .contains("runtime slot lease stored / slot: slot-1 / state: running")
    );
}

#[test]
fn runtime_event_log_port_filters_events_after_sequence() {
    let workspace_dir = temp_workspace("runtime-events-after-sequence");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let first = slot_lease("slot-1", ParallelModeSlotLeaseState::Leased);
    let second = slot_lease("slot-1", ParallelModeSlotLeaseState::Running);

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &first)
        .expect("first slot lease event should persist");
    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &second)
        .expect("second slot lease event should persist");

    let snapshot = adapter
        .load_runtime_event_log(
            &workspace_dir,
            ParallelModeRuntimeEventLogRequest::for_projection("slot_lease", "slot-1", 10)
                .after_sequence(1),
        )
        .expect("incremental runtime event log should load");

    assert_eq!(snapshot.total_event_count, 1);
    assert_eq!(snapshot.visible_count(), 1);
    let latest = snapshot.latest().expect("latest event should be visible");
    assert_eq!(latest.sequence, 2);
    assert!(latest.sequence > 1);
    assert!(latest.summary.contains("state: running"));
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

fn slot_lease(slot_id: &str, state: ParallelModeSlotLeaseState) -> ParallelModeSlotLeaseSnapshot {
    slot_lease_for_task(slot_id, "task-1", state)
}

fn slot_lease_for_task(
    slot_id: &str,
    task_id: &str,
    state: ParallelModeSlotLeaseState,
) -> ParallelModeSlotLeaseSnapshot {
    ParallelModeSlotLeaseSnapshot::new(
        slot_id,
        task_id,
        "Task One",
        "agent-1",
        format!("akra-agent/{slot_id}/{task_id}"),
        "/tmp/worktree",
        state,
        "2026-05-04T10:00:00+00:00",
        Some("2026-05-04T10:05:00+00:00".to_string()),
    )
}

fn queue_record_for_task(
    queue_item_id: &str,
    session_key: &str,
    task_id: &str,
) -> PlanningAuthorityDistributorQueueRecord {
    PlanningAuthorityDistributorQueueRecord {
        queue_item_id: queue_item_id.to_string(),
        queue_order_key: 1,
        session_key: session_key.to_string(),
        slot_id: "slot-1".to_string(),
        agent_id: "agent-1".to_string(),
        task_id: task_id.to_string(),
        task_title: "Task One".to_string(),
        source_branch: "prerelease".to_string(),
        source_commit_sha: "source".to_string(),
        branch_name: format!("akra-agent/slot-1/{task_id}"),
        worktree_path: "/tmp/worktree".to_string(),
        commit_sha: "commit".to_string(),
        original_commit_sha: None,
        planning_refresh_state: "complete".to_string(),
        integration_state: "queued".to_string(),
        conflict_files: Vec::new(),
        recovery_note: None,
        validation_summary: "validation unavailable".to_string(),
        authority_refresh_outcome: "not refreshed".to_string(),
        github_capabilities: None,
        pull_request_number: None,
        pull_request_url: None,
        queue_state: ParallelModeQueueItemState::Queued,
        integration_note: "queued".to_string(),
        enqueued_at: "2026-05-04T10:00:00+00:00".to_string(),
        updated_at: "2026-05-04T10:00:00+00:00".to_string(),
    }
}

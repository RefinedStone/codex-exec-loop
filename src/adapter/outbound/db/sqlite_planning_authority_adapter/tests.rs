/*
SQLite planning authority adapter가 application port의 snapshot 계약을 실제 DB 저장소 위에서도
지키는지 검증한다. service 계층은 `PlanningTaskRepositoryPort`만 보므로, 테스트는 concrete adapter를
포트 메서드로 호출해 task authority 문서와 queue projection의 동시 round-trip을 고정한다.
*/
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::app_server_prompt_log_port::{
    AppServerPromptInputRecord, AppServerPromptInteractionRecord, AppServerPromptLogPort,
    AppServerPromptOutputRecord,
};
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::{
    ParallelModeRuntimeEventLogPort, ParallelModeRuntimeEventLogRequest,
};
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
    PlanningAuthorityOfficialRefreshRecoveryStatus, PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningWorkspaceLoadRecord, RepoScopedPlanningWorkspacePort,
};
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAutomationTrigger,
    ParallelModeDispatchBlockReason, ParallelModeDispatchCommandSnapshot,
    ParallelModeDispatchCommandState, ParallelModePoolResetPolicy, ParallelModePoolResetReport,
    ParallelModePoolResetRunId, ParallelModePoolResetSlotAction, ParallelModePoolResetSlotOutcome,
    ParallelModePoolResetSlotReport, ParallelModeQueueItemState, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState, ParallelModeTaskDispatchBlockSnapshot,
};
use crate::domain::planning::{
    OriginSessionKind, PriorityQueueProjection, TaskActor, TaskAuthorityDocument, TaskDefinition,
    TaskMutationProvenance, TaskStatus,
};

use super::{DISTRIBUTOR_QUEUE_CLAIM_KIND, OFFICIAL_REFRESH_SCOPE_KEY, open_authority_connection};

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

fn authority_connection(workspace_dir: &str) -> rusqlite::Connection {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace_dir)
            .expect("authority location should resolve");
    open_authority_connection(&location).expect("authority db should open")
}

fn set_claim_timestamp(workspace_dir: &str, claim_kind: &str, scope_key: &str, claimed_at: &str) {
    let connection = authority_connection(workspace_dir);
    let changed_rows = connection
        .execute(
            "UPDATE runtime_claims
             SET claimed_at = ?1
             WHERE claim_kind = ?2 AND scope_key = ?3",
            (claimed_at, claim_kind, scope_key),
        )
        .expect("runtime claim timestamp should update");
    assert_eq!(changed_rows, 1);
}

fn set_authority_metadata(workspace_dir: &str, key: &str, value: &str) {
    authority_connection(workspace_dir)
        .execute(
            "INSERT INTO authority_metadata (key, value)
             VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            (key, value),
        )
        .expect("authority metadata should upsert");
}

fn insert_invalid_slot_marker(workspace_dir: &str, slot_id: &str) {
    let connection = authority_connection(workspace_dir);
    connection
        .execute(
            "INSERT OR REPLACE INTO runtime_invalid_slot_leases (slot_id, detected_at)
             VALUES (?1, ?2)",
            (slot_id, "2026-05-04T10:06:00+00:00"),
        )
        .expect("invalid slot marker should insert");
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
fn app_server_prompt_log_round_trips_recent_records() {
    let workspace_dir = temp_workspace("prompt-log");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .append_app_server_prompt_interaction(
            &workspace_dir,
            AppServerPromptInteractionRecord {
                sequence: 0,
                interaction_id: "interaction-1".to_string(),
                session_kind: "parallel-worker".to_string(),
                operation: "isolated_parallel_thread".to_string(),
                status: "completed".to_string(),
                workspace_dir: workspace_dir.clone(),
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                service_name: Some("akra-parallel-worker".to_string()),
                model: None,
                reasoning_effort: None,
                developer_instructions: Some("developer contract".to_string()),
                input_items: vec![AppServerPromptInputRecord::new(
                    "text",
                    "turn input",
                    "implement task",
                )],
                output_items: vec![AppServerPromptOutputRecord::new(
                    "agent-1",
                    Some("final".to_string()),
                    "done",
                )],
                error_message: None,
                started_at: "2026-05-20T00:00:00Z".to_string(),
                completed_at: "2026-05-20T00:00:01Z".to_string(),
            },
        )
        .expect("prompt log should append");

    let snapshot = adapter
        .load_recent_app_server_prompt_interactions(&workspace_dir, 10)
        .expect("prompt log should load");

    assert_eq!(snapshot.records.len(), 1);
    let record = &snapshot.records[0];
    assert!(record.sequence > 0);
    assert_eq!(record.session_kind, "parallel-worker");
    assert_eq!(record.service_name.as_deref(), Some("akra-parallel-worker"));
    assert_eq!(record.input_items[0].content, "implement task");
    assert_eq!(record.output_items[0].text, "done");
    assert_eq!(record.input_chars(), "implement task".chars().count());
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
fn repo_scoped_workspace_port_delegates_active_and_draft_operations() {
    let workspace_dir = temp_workspace("repo-scoped-port");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let port: &dyn RepoScopedPlanningWorkspacePort = &adapter;

    assert!(!port.is_git_backed_workspace(&workspace_dir));
    assert!(
        port.resolve_active_workspace_root(&workspace_dir)
            .is_absolute()
    );

    port.commit_active_workspace_files(
        &workspace_dir,
        &PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("active result output".to_string()),
        },
    )
    .expect("active workspace should commit through repo-scoped port");
    assert_eq!(
        port.load_active_workspace_files(&workspace_dir)
            .expect("active workspace should load through repo-scoped port")
            .result_output_markdown
            .as_deref(),
        Some("active result output")
    );

    port.replace_active_planning_file(
        &workspace_dir,
        RESULT_OUTPUT_FILE_PATH,
        Some("updated active result"),
    )
    .expect("active file should update through repo-scoped port");
    assert_eq!(
        port.load_active_planning_file(&workspace_dir, RESULT_OUTPUT_FILE_PATH)
            .expect("active file should load through repo-scoped port")
            .as_deref(),
        Some("updated active result")
    );

    let staged = port
        .stage_repo_scoped_draft_files(
            &workspace_dir,
            "draft-port",
            &[PlanningDraftFileRecord {
                active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
                body: "draft result output".to_string(),
            }],
        )
        .expect("draft should stage through repo-scoped port");
    assert_eq!(staged.draft_name, "draft-port");

    let staged_path = port
        .replace_repo_scoped_draft_file(
            &workspace_dir,
            "draft-port",
            RESULT_OUTPUT_FILE_PATH,
            "updated draft result",
        )
        .expect("draft file should update through repo-scoped port");
    assert!(staged_path.contains("draft-port"));

    let loaded = port
        .load_repo_scoped_draft_files(&workspace_dir, "draft-port")
        .expect("draft should load through repo-scoped port");
    assert_eq!(loaded.staged_files.len(), 1);
    assert_eq!(loaded.staged_files[0].body, "updated draft result");

    port.remove_active_planning_entry(&workspace_dir, RESULT_OUTPUT_FILE_PATH)
        .expect("active file should remove through repo-scoped port");
    assert_eq!(
        port.load_active_planning_file(&workspace_dir, RESULT_OUTPUT_FILE_PATH)
            .expect("active file lookup should still succeed")
            .as_deref(),
        None
    );
}

#[test]
fn runtime_reset_preserves_latest_failed_start_dispatch_block_per_task() {
    let workspace_dir = temp_workspace("failed-start-blocks");
    let adapter = SqlitePlanningAuthorityAdapter::new();

    adapter
        .upsert_runtime_slot_lease(
            &workspace_dir,
            &slot_lease_for_task("slot-reset", "task-1", ParallelModeSlotLeaseState::Running),
        )
        .expect("slot lease should persist before reset");
    insert_invalid_slot_marker(&workspace_dir, "slot-reset");
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
        .upsert_runtime_session_detail(
            &workspace_dir,
            &running_session_detail(
                "session-running",
                "task-running",
                "2026-05-04T12:02:00+00:00",
            ),
        )
        .expect("non-failed session should persist before reset");
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace_dir,
            &queue_record_for_task("queue-reset", "session-new", "task-1"),
        )
        .expect("queue record should persist before reset");
    adapter
        .enqueue_runtime_dispatch_command(
            &workspace_dir,
            &ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
                ParallelModeAutomationTrigger::ParallelOfficialCompletion,
                Some("task-1:ready".to_string()),
                Some(501),
                "2026-05-08T00:00:00+00:00",
            ),
        )
        .expect("dispatch command should persist before reset");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-reset", "queue-owner")
            .expect("queue claim should acquire before reset")
    );
    let refresh_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("official refresh order should reserve before reset");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "refresh-owner")
            .expect("official refresh claim should acquire before reset"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );

    adapter
        .clear_parallel_runtime_projections(&workspace_dir, "test reset")
        .expect("runtime projections should clear");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert!(snapshot.slot_leases.is_empty());
    assert!(snapshot.invalid_slot_leases.is_empty());
    assert_eq!(snapshot.session_details.len(), 0);
    assert!(snapshot.distributor_queue_records.is_empty());
    assert!(snapshot.dispatch_commands.is_empty());
    assert_eq!(snapshot.task_dispatch_blocks.len(), 1);
    let block = &snapshot.task_dispatch_blocks[0];
    assert_eq!(block.task_id, "task-1");
    assert_eq!(block.blocked_at, "2026-05-04T12:00:00+00:00");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-reset", "queue-owner-2")
            .expect("queue claim should clear during runtime reset")
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "refresh-owner-2")
            .expect("official refresh claim should clear during runtime reset"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
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
fn runtime_dispatch_command_reenqueue_revives_terminal_rows() {
    let workspace_dir = temp_workspace("dispatch-command-revive");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let mut command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("queue-head-revive".to_string()),
        Some(71),
        "2026-05-08T00:00:00+00:00",
    );

    assert!(
        adapter
            .enqueue_runtime_dispatch_command(&workspace_dir, &command)
            .expect("initial command should enqueue")
    );
    let mut claimed = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-1")
        .expect("initial command should claim")
        .expect("initial command should exist");
    claimed.mark_blocked("waiting for capacity", "2026-05-08T00:00:10+00:00");
    adapter
        .update_runtime_dispatch_command(&workspace_dir, &claimed)
        .expect("blocked command should persist");

    command.updated_at = "2026-05-08T00:01:00+00:00".to_string();
    assert!(
        adapter
            .enqueue_runtime_dispatch_command(&workspace_dir, &command)
            .expect("terminal command should revive")
    );
    let revived = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-2")
        .expect("revived command should claim")
        .expect("revived command should be pending again");

    assert_eq!(revived.command_id, command.command_id);
    assert_eq!(revived.state, ParallelModeDispatchCommandState::Running);
    assert_eq!(revived.owner_token.as_deref(), Some("owner-2"));
    assert!(
        adapter
            .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-3")
            .expect("empty dispatch queue should inspect cleanly")
            .is_none()
    );
}

#[test]
fn runtime_dispatch_command_claim_handles_payload_row_id_mismatch_as_lost_claim() {
    let workspace_dir = temp_workspace("dispatch-command-lost-claim");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let mut payload_command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("payload-head".to_string()),
        Some(72),
        "2026-05-08T00:00:00+00:00",
    );
    payload_command.command_id = "payload-command-id".to_string();
    let payload_json =
        serde_json::to_string(&payload_command).expect("dispatch command payload should serialize");
    authority_connection(&workspace_dir)
        .execute(
            "INSERT INTO runtime_dispatch_commands
                (command_id, command_kind, trigger, command_state, queue_head_signature,
                 epoch_id, created_at, updated_at, owner_token, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            (
                "row-command-id",
                payload_command.kind.label(),
                payload_command.trigger.label(),
                payload_command.state.label(),
                payload_command.queue_head_signature.as_deref(),
                payload_command.epoch_id.map(|value| value as i64),
                payload_command.created_at.as_str(),
                payload_command.updated_at.as_str(),
                payload_command.owner_token.as_deref(),
                payload_json.as_str(),
            ),
        )
        .expect("mismatched dispatch command row should insert");

    assert!(
        adapter
            .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner")
            .expect("mismatched dispatch command should be treated as lost claim")
            .is_none()
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
    insert_invalid_slot_marker(&workspace_dir, "slot-1");
    insert_invalid_slot_marker(&workspace_dir, "slot-2");
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
        .upsert_runtime_task_dispatch_block(
            &workspace_dir,
            &ParallelModeTaskDispatchBlockSnapshot::new(
                "task-deleted",
                "2026-05-04T11:55:00+00:00",
                "2026-05-04T12:00:00+00:00",
                ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
            ),
        )
        .expect("deleted task dispatch block should persist");
    adapter
        .upsert_runtime_task_dispatch_block(
            &workspace_dir,
            &ParallelModeTaskDispatchBlockSnapshot::new(
                "task-kept",
                "2026-05-04T11:56:00+00:00",
                "2026-05-04T12:01:00+00:00",
                ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
            ),
        )
        .expect("kept task dispatch block should persist");
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
    assert!(!snapshot.invalid_slot_leases.contains("slot-1"));
    assert!(snapshot.invalid_slot_leases.contains("slot-2"));
    assert_eq!(snapshot.session_details.len(), 1);
    assert_eq!(snapshot.session_details[0].task_id, "task-kept");
    assert_eq!(snapshot.task_dispatch_blocks.len(), 1);
    assert_eq!(snapshot.task_dispatch_blocks[0].task_id, "task-kept");
    assert_eq!(snapshot.distributor_queue_records.len(), 1);
    assert_eq!(snapshot.distributor_queue_records[0].task_id, "task-kept");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-deleted", "owner-2")
            .expect("deleted queue claim should be cleared")
    );
}

#[test]
fn runtime_task_cleanup_reports_malformed_json_in_lookup_rows() {
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let slot_workspace = temp_workspace("runtime-task-cleanup-bad-slot");
    authority_connection(&slot_workspace)
        .execute(
            "INSERT INTO runtime_slot_leases (slot_id, updated_at, content)
             VALUES (?1, ?2, ?3)",
            ("slot-bad", "2026-05-04T10:00:00+00:00", "{bad-json"),
        )
        .expect("malformed slot cleanup row should insert");
    let slot_error = adapter
        .clear_parallel_runtime_projections_for_tasks(
            &slot_workspace,
            &["task-bad".to_string()],
            "cleanup malformed slot",
        )
        .expect_err("malformed slot JSON should fail task cleanup");
    let slot_message = format!("{slot_error:?}");
    assert!(
        slot_message.contains("failed to iterate runtime slot ids for `task-bad`")
            || slot_message.contains("failed to decode runtime slot id for `task-bad`")
            || slot_message.contains("failed to clear runtime slot leases for `task-bad`"),
        "{slot_message}"
    );

    let queue_workspace = temp_workspace("runtime-task-cleanup-bad-queue");
    authority_connection(&queue_workspace)
        .execute(
            "INSERT INTO runtime_distributor_queue
                (queue_item_id, session_key, queue_state, enqueued_at, updated_at, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                "queue-bad",
                "session-bad",
                ParallelModeQueueItemState::Queued.label(),
                "2026-05-04T10:00:00+00:00",
                "2026-05-04T10:01:00+00:00",
                "{bad-json",
            ),
        )
        .expect("malformed queue cleanup row should insert");
    let queue_error = adapter
        .clear_parallel_runtime_projections_for_tasks(
            &queue_workspace,
            &["task-bad".to_string()],
            "cleanup malformed queue",
        )
        .expect_err("malformed queue JSON should fail task cleanup");
    let queue_message = format!("{queue_error:?}");
    assert!(
        queue_message.contains("failed to iterate runtime distributor queue ids for `task-bad`")
            || queue_message
                .contains("failed to decode runtime distributor queue id for `task-bad`")
            || queue_message
                .contains("failed to clear runtime distributor queue records for `task-bad`"),
        "{queue_message}"
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
fn malformed_runtime_projection_rows_report_row_specific_context() {
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let slot_workspace = temp_workspace("runtime-bad-slot-json");
    authority_connection(&slot_workspace)
        .execute(
            "INSERT INTO runtime_slot_leases (slot_id, updated_at, content)
             VALUES (?1, ?2, ?3)",
            ("slot-bad", "2026-05-04T10:00:00+00:00", "{bad-json"),
        )
        .expect("malformed slot row should insert");
    let slot_error = adapter
        .load_runtime_projections(&slot_workspace)
        .expect_err("malformed slot row should fail projection load");
    assert!(
        format!("{slot_error:?}").contains("failed to deserialize runtime slot lease `slot-bad`")
    );

    let session_workspace = temp_workspace("runtime-bad-session-json");
    authority_connection(&session_workspace)
        .execute(
            "INSERT INTO runtime_session_details (session_key, slot_id, updated_at, content)
             VALUES (?1, ?2, ?3, ?4)",
            (
                "session-bad",
                "slot-1",
                "2026-05-04T10:00:00+00:00",
                "{bad-json",
            ),
        )
        .expect("malformed session row should insert");
    let session_error = adapter
        .load_runtime_projections(&session_workspace)
        .expect_err("malformed session row should fail projection load");
    assert!(
        format!("{session_error:?}")
            .contains("failed to deserialize runtime session detail `session-bad`")
    );

    let block_workspace = temp_workspace("runtime-bad-block-json");
    authority_connection(&block_workspace)
        .execute(
            "INSERT INTO runtime_task_dispatch_blocks
                (task_id, reason, task_updated_at, blocked_at, content)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                "task-bad",
                ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges.label(),
                "2026-05-04T09:59:00+00:00",
                "2026-05-04T10:00:00+00:00",
                "{bad-json",
            ),
        )
        .expect("malformed dispatch block row should insert");
    let block_error = adapter
        .load_runtime_projections(&block_workspace)
        .expect_err("malformed dispatch block row should fail projection load");
    assert!(
        format!("{block_error:?}")
            .contains("failed to deserialize runtime task dispatch block `task-bad`")
    );

    let queue_workspace = temp_workspace("runtime-bad-queue-json");
    authority_connection(&queue_workspace)
        .execute(
            "INSERT INTO runtime_distributor_queue
                (queue_item_id, session_key, queue_state, enqueued_at, updated_at, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                "queue-bad",
                "session-bad",
                ParallelModeQueueItemState::Queued.label(),
                "2026-05-04T10:00:00+00:00",
                "2026-05-04T10:01:00+00:00",
                "{bad-json",
            ),
        )
        .expect("malformed queue row should insert");
    let queue_error = adapter
        .load_runtime_projections(&queue_workspace)
        .expect_err("malformed queue row should fail projection load");
    assert!(
        format!("{queue_error:?}")
            .contains("failed to deserialize runtime distributor queue record `queue-bad`")
    );

    let dispatch_workspace = temp_workspace("runtime-bad-dispatch-json");
    let command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        Some("task-bad:ready".to_string()),
        Some(51),
        "2026-05-08T00:00:00+00:00",
    );
    adapter
        .enqueue_runtime_dispatch_command(&dispatch_workspace, &command)
        .expect("dispatch command should enqueue before corruption");
    authority_connection(&dispatch_workspace)
        .execute(
            "UPDATE runtime_dispatch_commands SET content = ?1 WHERE command_id = ?2",
            ("{bad-json", command.command_id.as_str()),
        )
        .expect("dispatch command content should corrupt");
    let claim_error = adapter
        .try_claim_next_runtime_dispatch_command(&dispatch_workspace, "owner")
        .expect_err("malformed dispatch command should fail claim");
    assert!(format!("{claim_error:?}").contains(&format!(
        "failed to deserialize runtime dispatch command `{}`",
        command.command_id
    )));

    let load_error = adapter
        .load_runtime_projections(&dispatch_workspace)
        .expect_err("malformed dispatch command should fail projection load");
    assert!(format!("{load_error:?}").contains(&format!(
        "failed to deserialize runtime dispatch command `{}`",
        command.command_id
    )));
}

#[test]
fn runtime_reset_reports_malformed_failed_start_session_before_clearing_rows() {
    let workspace_dir = temp_workspace("runtime-reset-bad-session-json");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    authority_connection(&workspace_dir)
        .execute(
            "INSERT INTO runtime_session_details (session_key, slot_id, updated_at, content)
             VALUES (?1, ?2, ?3, ?4)",
            (
                "session-bad",
                "slot-1",
                "2026-05-04T10:00:00+00:00",
                "{bad-json",
            ),
        )
        .expect("malformed session row should insert before reset");

    let error = adapter
        .clear_parallel_runtime_projections(&workspace_dir, "reset malformed session")
        .expect_err("malformed failed-start preservation row should fail reset");

    assert!(
        format!("{error:?}")
            .contains("failed to deserialize session detail `session-bad` before reset")
    );
}

#[test]
fn runtime_recoverable_projection_survives_adapter_restart_boundary() {
    let workspace_dir = temp_workspace("runtime-restart-boundary");
    let writer = SqlitePlanningAuthorityAdapter::new();
    let lease = slot_lease_for_task(
        "slot-1",
        "task-recoverable",
        ParallelModeSlotLeaseState::Running,
    );
    let mut session = failed_start_session_detail(
        "session-recoverable",
        "task-recoverable",
        "2026-05-04T12:00:00+00:00",
    );
    session.thread_id = Some("thread-recoverable".to_string());
    let block = ParallelModeTaskDispatchBlockSnapshot::new(
        "task-recoverable",
        "2026-05-04T11:55:00+00:00",
        "2026-05-04T12:00:00+00:00",
        ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
    );
    let queue_record = queue_record_for_task(
        "queue-recoverable",
        "session-recoverable",
        "task-recoverable",
    );
    let command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("task-recoverable:ready".to_string()),
        Some(41),
        "2026-05-08T00:00:00+00:00",
    );

    writer
        .upsert_runtime_slot_lease(&workspace_dir, &lease)
        .expect("slot lease should persist");
    writer
        .upsert_runtime_session_detail(&workspace_dir, &session)
        .expect("session detail should persist");
    writer
        .upsert_runtime_task_dispatch_block(&workspace_dir, &block)
        .expect("task dispatch block should persist");
    writer
        .upsert_runtime_distributor_queue_record(&workspace_dir, &queue_record)
        .expect("distributor queue should persist");
    assert!(
        writer
            .enqueue_runtime_dispatch_command(&workspace_dir, &command)
            .expect("dispatch command should persist")
    );
    assert!(
        writer
            .try_acquire_distributor_queue_claim(
                &workspace_dir,
                "queue-recoverable",
                "owner-before-restart",
            )
            .expect("queue claim should acquire")
    );
    assert_eq!(
        writer
            .reserve_next_official_refresh_order(&workspace_dir)
            .expect("first refresh order should reserve"),
        1
    );

    let restarted = SqlitePlanningAuthorityAdapter::new();
    let snapshot = restarted
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should survive a new adapter handle");

    assert_eq!(snapshot.slot_leases.get("slot-1"), Some(&lease));
    assert_eq!(snapshot.session_details, vec![session]);
    assert_eq!(snapshot.task_dispatch_blocks, vec![block]);
    assert_eq!(snapshot.distributor_queue_records, vec![queue_record]);
    assert_eq!(snapshot.dispatch_commands, vec![command]);
    assert!(
        !restarted
            .try_acquire_distributor_queue_claim(
                &workspace_dir,
                "queue-recoverable",
                "owner-after-restart",
            )
            .expect("persisted queue claim should block a competing owner")
    );
    assert_eq!(
        restarted
            .reserve_next_official_refresh_order(&workspace_dir)
            .expect("official refresh order should continue after restart"),
        2
    );
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
fn runtime_claim_release_and_stale_timestamp_edges_respect_claim_ownership() {
    let workspace_dir = temp_workspace("runtime-claim-release-edges");
    let adapter = SqlitePlanningAuthorityAdapter::new();

    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-claim", "queue-owner")
            .expect("queue claim should acquire")
    );
    adapter
        .release_distributor_queue_claim(&workspace_dir, "queue-claim", "wrong-owner")
        .expect("wrong queue owner release should be harmless");
    assert!(
        !adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-claim", "queue-other")
            .expect("wrong release should not clear queue claim")
    );
    adapter
        .release_distributor_queue_claim(&workspace_dir, "queue-claim", "queue-owner")
        .expect("matching queue owner should release");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-claim", "queue-other")
            .expect("matching release should clear queue claim")
    );
    set_claim_timestamp(
        &workspace_dir,
        DISTRIBUTOR_QUEUE_CLAIM_KIND,
        "queue-claim",
        "not-a-timestamp",
    );
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-claim", "queue-reclaimer")
            .expect("invalid queue claim timestamp should be reclaimed")
    );

    let refresh_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("official refresh order should reserve");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "refresh-owner")
            .expect("official refresh claim should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    adapter
        .release_official_refresh_claim(&workspace_dir, refresh_order, "wrong-refresh-owner")
        .expect("wrong official owner release should be harmless");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "refresh-other")
            .expect("wrong release should not clear official claim"),
        PlanningAuthorityOfficialRefreshClaimStatus::Waiting
    );
    set_authority_metadata(
        &workspace_dir,
        "next_executable_refresh_order",
        &(refresh_order + 5).to_string(),
    );
    adapter
        .release_official_refresh_claim(&workspace_dir, refresh_order, "refresh-owner")
        .expect("matching official owner should release without moving advanced pointer back");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "refresh-owner")
            .expect("advanced pointer should still mark old order completed"),
        PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted
    );
}

#[test]
fn official_refresh_recovery_handles_reentry_active_and_stale_claim_edges() {
    let idle_workspace = temp_workspace("official-refresh-recovery-idle");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    assert_eq!(
        adapter
            .abandon_next_official_refresh_order(&idle_workspace, "nothing pending")
            .expect("idle recovery should inspect"),
        PlanningAuthorityOfficialRefreshRecoveryStatus::NoPendingOrder
    );

    let first_order = adapter
        .reserve_next_official_refresh_order(&idle_workspace)
        .expect("first order should reserve");
    let second_order = adapter
        .reserve_next_official_refresh_order(&idle_workspace)
        .expect("second order should reserve");
    assert_eq!(
        adapter
            .abandon_next_official_refresh_order(&idle_workspace, "head worker exited")
            .expect("head order should recover"),
        PlanningAuthorityOfficialRefreshRecoveryStatus::Recovered {
            refresh_order: first_order,
        }
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&idle_workspace, first_order, "owner-1")
            .expect("recovered order should inspect as completed"),
        PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&idle_workspace, second_order, "owner-2")
            .expect("second order should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&idle_workspace, second_order, "owner-2")
            .expect("same owner should reenter"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );

    let active_workspace = temp_workspace("official-refresh-recovery-active");
    let active_order = adapter
        .reserve_next_official_refresh_order(&active_workspace)
        .expect("active order should reserve");
    adapter
        .reserve_next_official_refresh_order(&active_workspace)
        .expect("later active order should reserve");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&active_workspace, active_order, "active-owner")
            .expect("active owner should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    assert_eq!(
        adapter
            .abandon_next_official_refresh_order(&active_workspace, "operator retry")
            .expect("active claim should block recovery"),
        PlanningAuthorityOfficialRefreshRecoveryStatus::WaitingForActiveClaim
    );

    let stale_workspace = temp_workspace("official-refresh-recovery-stale");
    let stale_order = adapter
        .reserve_next_official_refresh_order(&stale_workspace)
        .expect("stale order should reserve");
    adapter
        .reserve_next_official_refresh_order(&stale_workspace)
        .expect("later stale order should reserve");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&stale_workspace, stale_order, "stale-owner")
            .expect("stale owner should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    set_claim_timestamp(
        &stale_workspace,
        "official-refresh",
        OFFICIAL_REFRESH_SCOPE_KEY,
        "2000-01-01T00:00:00+00:00",
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&stale_workspace, stale_order, "replacement-owner")
            .expect("stale claim should be reclaimed by acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    set_claim_timestamp(
        &stale_workspace,
        "official-refresh",
        OFFICIAL_REFRESH_SCOPE_KEY,
        "2000-01-01T00:00:00+00:00",
    );
    assert_eq!(
        adapter
            .abandon_next_official_refresh_order(&stale_workspace, "stale head")
            .expect("stale claim should not block recovery"),
        PlanningAuthorityOfficialRefreshRecoveryStatus::Recovered {
            refresh_order: stale_order,
        }
    );
}

#[test]
fn stale_distributor_claims_invalid_slots_pool_reset_and_zero_limit_events_are_projected() {
    let workspace_dir = temp_workspace("runtime-projection-edges");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let lease = slot_lease_for_task(
        "slot-reset",
        "task-reset",
        ParallelModeSlotLeaseState::Running,
    );
    let session =
        failed_start_session_detail("session-reset", "task-reset", "2026-05-04T12:00:00+00:00");
    let queue_record = queue_record_for_task("queue-reset", "session-reset", "task-reset");

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &lease)
        .expect("reset slot lease should persist");
    insert_invalid_slot_marker(&workspace_dir, "slot-reset");
    adapter
        .upsert_runtime_session_detail(&workspace_dir, &session)
        .expect("reset session should persist");
    adapter
        .upsert_runtime_distributor_queue_record(&workspace_dir, &queue_record)
        .expect("reset queue record should persist");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-reset", "owner-old")
            .expect("queue claim should acquire")
    );
    set_claim_timestamp(
        &workspace_dir,
        DISTRIBUTOR_QUEUE_CLAIM_KIND,
        "queue-reset",
        "2000-01-01T00:00:00+00:00",
    );
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-reset", "owner-new")
            .expect("stale queue claim should be reclaimed")
    );

    let snapshot_with_invalid = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load invalid marker");
    assert!(
        snapshot_with_invalid
            .invalid_slot_leases
            .contains("slot-reset")
    );

    let zero_limit_events = adapter
        .load_runtime_event_log(
            &workspace_dir,
            ParallelModeRuntimeEventLogRequest::for_projection("slot_lease", "slot-reset", 0),
        )
        .expect("zero-limit event request should load");
    assert_eq!(zero_limit_events.total_event_count, 1);
    assert_eq!(zero_limit_events.visible_count(), 0);
    assert_eq!(
        zero_limit_events.empty_state,
        "runtime events hidden by request limit"
    );

    let mut report = ParallelModePoolResetReport::new(
        ParallelModePoolResetRunId::new("reset-run-1"),
        ParallelModePoolResetPolicy::ForceDisposable,
    );
    report
        .slot_reports
        .push(ParallelModePoolResetSlotReport::new(
            "slot-reset",
            ParallelModePoolResetSlotAction::Reset,
            ParallelModePoolResetSlotOutcome::Succeeded,
            "reset completed",
        ));
    report.reset_session_keys.push("session-reset".to_string());
    report.reset_queue_item_ids.push("queue-reset".to_string());
    adapter
        .apply_parallel_pool_reset_report(&workspace_dir, &report)
        .expect("pool reset report should apply");

    let reset_snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load after reset");
    assert!(reset_snapshot.slot_leases.is_empty());
    assert!(reset_snapshot.invalid_slot_leases.is_empty());
    assert!(reset_snapshot.session_details.is_empty());
    assert!(reset_snapshot.distributor_queue_records.is_empty());
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-reset", "owner-after-reset")
            .expect("reset queue claim should be cleared")
    );

    adapter
        .clear_parallel_runtime_projections_for_tasks(
            &workspace_dir,
            &[" ".to_string(), String::new()],
            "blank task cleanup",
        )
        .expect("blank task cleanup should be a no-op");
}

#[test]
fn runtime_slot_removal_clears_current_and_invalid_slot_projection() {
    let workspace_dir = temp_workspace("runtime-slot-removal");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let lease = slot_lease("slot-remove", ParallelModeSlotLeaseState::Running);

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &lease)
        .expect("slot lease should persist before removal");
    insert_invalid_slot_marker(&workspace_dir, "slot-remove");
    adapter
        .remove_runtime_slot_lease(&workspace_dir, "slot-remove")
        .expect("slot lease should remove");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load after slot removal");
    assert!(!snapshot.slot_leases.contains_key("slot-remove"));
    assert!(!snapshot.invalid_slot_leases.contains("slot-remove"));
    assert!(snapshot.runtime_events.iter().any(|event| {
        event.event_kind == "slot_lease_removed" && event.projection_key == "slot-remove"
    }));
}

#[test]
fn runtime_slot_removal_without_current_row_clears_invalid_marker_without_event() {
    let workspace_dir = temp_workspace("runtime-slot-removal-empty");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    insert_invalid_slot_marker(&workspace_dir, "slot-missing");

    adapter
        .remove_runtime_slot_lease(&workspace_dir, "slot-missing")
        .expect("missing slot removal should still clear invalid marker");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load after missing slot removal");
    assert!(snapshot.invalid_slot_leases.is_empty());
    assert!(
        !snapshot
            .runtime_events
            .iter()
            .any(|event| event.event_kind == "slot_lease_removed")
    );
}

#[test]
fn runtime_task_dispatch_block_keeps_newer_block_when_older_update_arrives() {
    let workspace_dir = temp_workspace("runtime-dispatch-block-older");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let newer = ParallelModeTaskDispatchBlockSnapshot::new(
        "task-blocked",
        "2026-05-04T12:00:00+00:00",
        "2026-05-04T12:10:00+00:00",
        ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
    );
    let older = ParallelModeTaskDispatchBlockSnapshot::new(
        "task-blocked",
        "2026-05-04T11:00:00+00:00",
        "2026-05-04T12:00:00+00:00",
        ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
    );

    adapter
        .upsert_runtime_task_dispatch_block(&workspace_dir, &newer)
        .expect("newer dispatch block should persist");
    adapter
        .upsert_runtime_task_dispatch_block(&workspace_dir, &older)
        .expect("older dispatch block should be ignored without failing");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(snapshot.task_dispatch_blocks, vec![newer]);
    let block_events = snapshot
        .runtime_events
        .iter()
        .filter(|event| event.event_kind == "task_dispatch_block_upsert")
        .count();
    assert_eq!(block_events, 1);
}

#[test]
fn runtime_event_log_empty_filter_reports_no_events() {
    let workspace_dir = temp_workspace("runtime-events-empty");
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let snapshot = adapter
        .load_runtime_event_log(
            &workspace_dir,
            ParallelModeRuntimeEventLogRequest::for_projection("slot_lease", "missing", 5),
        )
        .expect("empty runtime event log should load");

    assert_eq!(snapshot.total_event_count, 0);
    assert_eq!(snapshot.visible_count(), 0);
    assert_eq!(snapshot.empty_state, "no runtime events captured yet");
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

fn running_session_detail(
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
        Some("thread-running".to_string()),
        "/tmp/worktree",
        "akra-agent/slot-1/task-one",
        "2026-05-04T10:00:00+00:00",
        "running",
        "in_progress",
        "worker is running",
        "validation unavailable",
        "running",
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

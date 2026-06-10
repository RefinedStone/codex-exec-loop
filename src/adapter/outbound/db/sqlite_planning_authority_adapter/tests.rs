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
    OriginSessionKind, PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask,
    TaskActor, TaskAuthorityDocument, TaskDefinition, TaskMutationProvenance, TaskStatus,
};

use super::{
    DISTRIBUTOR_QUEUE_CLAIM_KIND, OFFICIAL_REFRESH_SCOPE_KEY, open_authority_connection,
    task_authority_rows::replace_task_authority_tables,
};

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

fn replace_table_schema(connection: &rusqlite::Connection, table_name: &str, columns_sql: &str) {
    let sql = format!("DROP TABLE {table_name}; CREATE TABLE {table_name} ({columns_sql});");
    connection
        .execute_batch(&sql)
        .expect("runtime table schema should be replaced");
}

fn replace_runtime_table_schema(workspace_dir: &str, table_name: &str, columns_sql: &str) {
    replace_table_schema(
        &authority_connection(workspace_dir),
        table_name,
        columns_sql,
    );
}

fn corrupt_runtime_events_schema(workspace_dir: &str) {
    replace_runtime_table_schema(
        workspace_dir,
        "runtime_events",
        "sequence INTEGER PRIMARY KEY",
    );
}

fn install_failing_delete_trigger(workspace_dir: &str, table_name: &str, trigger_name: &str) {
    let sql = format!(
        "CREATE TRIGGER {trigger_name}
         BEFORE DELETE ON {table_name}
         BEGIN
             SELECT RAISE(FAIL, 'forced delete failure');
         END;"
    );
    authority_connection(workspace_dir)
        .execute_batch(&sql)
        .expect("failing delete trigger should install");
}

fn dispatch_command_snapshot(seed: u64) -> ParallelModeDispatchCommandSnapshot {
    ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some(format!("queue-head-{seed}")),
        Some(seed),
        "2026-05-08T00:00:00+00:00",
    )
}

fn insert_pending_dispatch_command_row(
    workspace_dir: &str,
    command: &ParallelModeDispatchCommandSnapshot,
) {
    let payload_json =
        serde_json::to_string(command).expect("dispatch command payload should serialize");
    authority_connection(workspace_dir)
        .execute(
            "INSERT INTO runtime_dispatch_commands
                (command_id, command_state, created_at, content)
             VALUES (?1, ?2, ?3, ?4)",
            (
                command.command_id.as_str(),
                ParallelModeDispatchCommandState::Pending.label(),
                command.created_at.as_str(),
                payload_json.as_str(),
            ),
        )
        .expect("pending dispatch command row should insert");
}

fn assert_error_contains<T>(result: anyhow::Result<T>, expected: &str) {
    let Err(error) = result else {
        panic!("expected error containing `{expected}`");
    };
    let message = format!("{error:?}");
    assert!(message.contains(expected), "{message}");
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
fn task_authority_row_write_errors_keep_operation_context() {
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![TaskDefinition {
            id: "task-edge".to_string(),
            direction_id: "direction-1".to_string(),
            direction_relation_note: "covers row error contexts".to_string(),
            title: "Persist row context".to_string(),
            description: "Persist relation and projection rows.".to_string(),
            status: TaskStatus::Ready,
            base_priority: 80,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: vec!["task-parent".to_string()],
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            provenance: TaskMutationProvenance::default(),
            updated_at: "2026-05-07T09:00:00Z".to_string(),
        }],
    };
    let empty_task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let empty_queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

    let edge_workspace = temp_workspace("task-row-error-edge");
    let mut edge_connection = authority_connection(&edge_workspace);
    replace_table_schema(&edge_connection, "planning_task_edges", "task_id TEXT");
    let edge_transaction = edge_connection
        .transaction()
        .expect("edge transaction should open");
    assert_error_contains(
        replace_task_authority_tables(&edge_transaction, &task_authority, &empty_queue_projection),
        "failed to persist planning task edge `task-edge:depends_on`",
    );

    let active_projection_workspace = temp_workspace("task-row-error-active-projection");
    let mut active_projection_connection = authority_connection(&active_projection_workspace);
    replace_table_schema(
        &active_projection_connection,
        "planning_queue_projection",
        "bucket TEXT",
    );
    let active_queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: vec![PriorityQueueTask {
            rank: 1,
            task_id: "task-active".to_string(),
            direction_id: "direction-1".to_string(),
            direction_title: "Direction 1".to_string(),
            task_title: "Active projection".to_string(),
            status: TaskStatus::Ready,
            combined_priority: 80,
            updated_at: "2026-05-07T09:00:00Z".to_string(),
            rank_reasons: vec!["highest priority".to_string()],
        }],
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };
    let active_projection_transaction = active_projection_connection
        .transaction()
        .expect("active projection transaction should open");
    assert_error_contains(
        replace_task_authority_tables(
            &active_projection_transaction,
            &empty_task_authority,
            &active_queue_projection,
        ),
        "failed to persist planning queue projection `active:task-active`",
    );

    let skipped_projection_workspace = temp_workspace("task-row-error-skipped-projection");
    let mut skipped_projection_connection = authority_connection(&skipped_projection_workspace);
    replace_table_schema(
        &skipped_projection_connection,
        "planning_queue_projection",
        "bucket TEXT",
    );
    let skipped_queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: vec![PriorityQueueSkippedTask {
            task_id: "task-skipped".to_string(),
            task_title: "Skipped projection".to_string(),
            direction_id: "direction-1".to_string(),
            status: TaskStatus::Blocked,
            reason: "blocked by dependency".to_string(),
        }],
    };
    let skipped_projection_transaction = skipped_projection_connection
        .transaction()
        .expect("skipped projection transaction should open");
    assert_error_contains(
        replace_task_authority_tables(
            &skipped_projection_transaction,
            &empty_task_authority,
            &skipped_queue_projection,
        ),
        "failed to persist skipped planning queue projection `task-skipped`",
    );

    let metadata_workspace = temp_workspace("task-row-error-metadata");
    let mut metadata_connection = authority_connection(&metadata_workspace);
    replace_table_schema(
        &metadata_connection,
        "authority_metadata",
        "broken_key TEXT",
    );
    let metadata_transaction = metadata_connection
        .transaction()
        .expect("metadata transaction should open");
    assert_error_contains(
        replace_task_authority_tables(
            &metadata_transaction,
            &empty_task_authority,
            &empty_queue_projection,
        ),
        "failed to update authority metadata `task_authority_version`",
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

    let invalid_error = port
        .stage_repo_scoped_draft_files(
            &workspace_dir,
            "../outside",
            &[PlanningDraftFileRecord {
                active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
                body: "escaped draft result output".to_string(),
            }],
        )
        .expect_err("invalid repo-scoped draft name should not stage");
    assert!(
        invalid_error
            .to_string()
            .contains("invalid planning draft name `../outside`")
    );

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
fn pool_reset_report_clears_only_successful_slots_and_reads_unfiltered_events() {
    let workspace_dir = temp_workspace("runtime-reset-report-mixed");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let success_lease = slot_lease_for_task(
        "slot-success",
        "task-success",
        ParallelModeSlotLeaseState::Running,
    );
    let blocked_lease = slot_lease_for_task(
        "slot-blocked",
        "task-blocked",
        ParallelModeSlotLeaseState::Running,
    );
    let failed_lease = slot_lease_for_task(
        "slot-failed",
        "task-failed",
        ParallelModeSlotLeaseState::Running,
    );

    for lease in [&success_lease, &blocked_lease, &failed_lease] {
        adapter
            .upsert_runtime_slot_lease(&workspace_dir, lease)
            .expect("slot lease should persist before mixed reset");
        insert_invalid_slot_marker(&workspace_dir, &lease.slot_id);
    }
    adapter
        .upsert_runtime_session_detail(
            &workspace_dir,
            &running_session_detail(
                "session-success",
                "task-success",
                "2026-05-04T12:00:00+00:00",
            ),
        )
        .expect("reset session should persist");
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace_dir,
            &queue_record_for_task("queue-success", "session-success", "task-success"),
        )
        .expect("reset queue should persist");
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace_dir,
            &queue_record_for_task("queue-blocked", "session-blocked", "task-blocked"),
        )
        .expect("blocked queue should persist");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-success", "owner-success")
            .expect("success queue claim should acquire")
    );
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-blocked", "owner-blocked")
            .expect("blocked queue claim should acquire")
    );

    let mut report = ParallelModePoolResetReport::new(
        ParallelModePoolResetRunId::new("mixed-reset-run"),
        ParallelModePoolResetPolicy::ProtectLive,
    );
    report
        .slot_reports
        .push(ParallelModePoolResetSlotReport::new(
            "slot-success",
            ParallelModePoolResetSlotAction::Reset,
            ParallelModePoolResetSlotOutcome::Succeeded,
            "reset completed",
        ));
    report
        .slot_reports
        .push(ParallelModePoolResetSlotReport::new(
            "slot-blocked",
            ParallelModePoolResetSlotAction::PreserveLive,
            ParallelModePoolResetSlotOutcome::Blocked,
            "live turn running",
        ));
    report
        .slot_reports
        .push(ParallelModePoolResetSlotReport::new(
            "slot-failed",
            ParallelModePoolResetSlotAction::Reset,
            ParallelModePoolResetSlotOutcome::Failed,
            "worktree reset failed",
        ));
    report
        .slot_reports
        .push(ParallelModePoolResetSlotReport::new(
            "slot-missing",
            ParallelModePoolResetSlotAction::SkipMissing,
            ParallelModePoolResetSlotOutcome::Skipped,
            "slot missing",
        ));
    report
        .reset_session_keys
        .push("session-success".to_string());
    report
        .reset_queue_item_ids
        .push("queue-success".to_string());

    adapter
        .apply_parallel_pool_reset_report(&workspace_dir, &report)
        .expect("mixed pool reset report should apply");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load after mixed reset");
    assert!(!snapshot.slot_leases.contains_key("slot-success"));
    assert!(snapshot.slot_leases.contains_key("slot-blocked"));
    assert!(snapshot.slot_leases.contains_key("slot-failed"));
    assert!(!snapshot.invalid_slot_leases.contains("slot-success"));
    assert!(snapshot.invalid_slot_leases.contains("slot-blocked"));
    assert!(snapshot.invalid_slot_leases.contains("slot-failed"));
    assert!(snapshot.session_details.is_empty());
    assert_eq!(snapshot.distributor_queue_records.len(), 1);
    assert_eq!(
        snapshot.distributor_queue_records[0].queue_item_id,
        "queue-blocked"
    );
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(
                &workspace_dir,
                "queue-success",
                "owner-success-after-reset",
            )
            .expect("success queue claim should be cleared")
    );
    assert!(
        !adapter
            .try_acquire_distributor_queue_claim(
                &workspace_dir,
                "queue-blocked",
                "owner-blocked-after-reset",
            )
            .expect("blocked queue claim should remain")
    );

    let events = adapter
        .load_runtime_event_log(
            &workspace_dir,
            ParallelModeRuntimeEventLogRequest::recent(50),
        )
        .expect("unfiltered runtime event log should load");
    assert!(events.total_event_count >= snapshot.runtime_events.len());
    assert!(events.entries.iter().any(|event| {
        event.event_kind == "parallel_pool_reset_report_applied"
            && event.summary.contains("live_blockers: 1")
            && event.summary.contains("failures: 1")
    }));
}

#[test]
fn runtime_task_cleanup_trims_deduplicates_and_clears_multiple_tasks() {
    let workspace_dir = temp_workspace("runtime-task-cleanup-multi");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    for (slot_id, task_id, session_key, queue_item_id) in [
        ("slot-a", "task-a", "session-a", "queue-a"),
        ("slot-b", "task-b", "session-b", "queue-b"),
        ("slot-c", "task-c", "session-c", "queue-c"),
    ] {
        adapter
            .upsert_runtime_slot_lease(
                &workspace_dir,
                &slot_lease_for_task(slot_id, task_id, ParallelModeSlotLeaseState::Running),
            )
            .expect("task cleanup slot lease should persist");
        insert_invalid_slot_marker(&workspace_dir, slot_id);
        adapter
            .upsert_runtime_session_detail(
                &workspace_dir,
                &failed_start_session_detail(session_key, task_id, "2026-05-04T12:00:00+00:00"),
            )
            .expect("task cleanup session should persist");
        adapter
            .upsert_runtime_task_dispatch_block(
                &workspace_dir,
                &ParallelModeTaskDispatchBlockSnapshot::new(
                    task_id,
                    "2026-05-04T11:55:00+00:00",
                    "2026-05-04T12:00:00+00:00",
                    ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
                ),
            )
            .expect("task cleanup dispatch block should persist");
        adapter
            .upsert_runtime_distributor_queue_record(
                &workspace_dir,
                &queue_record_for_task(queue_item_id, session_key, task_id),
            )
            .expect("task cleanup queue should persist");
        assert!(
            adapter
                .try_acquire_distributor_queue_claim(&workspace_dir, queue_item_id, "owner")
                .expect("task cleanup queue claim should acquire")
        );
    }

    adapter
        .clear_parallel_runtime_projections_for_tasks(
            &workspace_dir,
            &[
                " task-b ".to_string(),
                "task-a".to_string(),
                "task-a".to_string(),
                String::new(),
            ],
            "multi task cleanup",
        )
        .expect("multi-task runtime cleanup should apply");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load after multi-task cleanup");
    assert_eq!(
        snapshot
            .slot_leases
            .values()
            .map(|lease| lease.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-c"]
    );
    assert_eq!(
        snapshot
            .session_details
            .iter()
            .map(|detail| detail.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-c"]
    );
    assert_eq!(
        snapshot
            .task_dispatch_blocks
            .iter()
            .map(|block| block.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-c"]
    );
    assert_eq!(
        snapshot
            .distributor_queue_records
            .iter()
            .map(|record| record.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-c"]
    );
    assert!(!snapshot.invalid_slot_leases.contains("slot-a"));
    assert!(!snapshot.invalid_slot_leases.contains("slot-b"));
    assert!(snapshot.invalid_slot_leases.contains("slot-c"));
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-a", "owner-after")
            .expect("task-a queue claim should clear")
    );
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-b", "owner-after")
            .expect("task-b queue claim should clear")
    );
    assert!(
        !adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-c", "owner-after")
            .expect("task-c queue claim should remain")
    );
    assert!(snapshot.runtime_events.iter().any(|event| {
        event.event_kind == "parallel_runtime_task_cleanup"
            && event.summary.contains("tasks: 2")
            && event.summary.contains("claims: 2")
    }));
}

#[test]
fn runtime_projection_write_error_contexts_report_broken_tables() {
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let enqueue_workspace = temp_workspace("runtime-write-error-dispatch-enqueue");
    replace_runtime_table_schema(
        &enqueue_workspace,
        "runtime_dispatch_commands",
        "command_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter
            .enqueue_runtime_dispatch_command(&enqueue_workspace, &dispatch_command_snapshot(801)),
        "failed to enqueue runtime dispatch command",
    );

    let revive_workspace = temp_workspace("runtime-write-error-dispatch-revive");
    let revive_command = dispatch_command_snapshot(802);
    adapter
        .enqueue_runtime_dispatch_command(&revive_workspace, &revive_command)
        .expect("revive seed should enqueue");
    let mut blocked = adapter
        .try_claim_next_runtime_dispatch_command(&revive_workspace, "owner-revive")
        .expect("revive seed should claim")
        .expect("revive seed should exist");
    blocked.mark_blocked("waiting for retry", "2026-05-08T00:00:10+00:00");
    adapter
        .update_runtime_dispatch_command(&revive_workspace, &blocked)
        .expect("revive seed should become terminal");
    authority_connection(&revive_workspace)
        .execute_batch(
            "CREATE TRIGGER fail_dispatch_reenqueue_update
             BEFORE UPDATE ON runtime_dispatch_commands
             BEGIN
                 SELECT RAISE(FAIL, 'forced dispatch update failure');
             END;",
        )
        .expect("dispatch update trigger should install");
    assert_error_contains(
        adapter.enqueue_runtime_dispatch_command(&revive_workspace, &revive_command),
        "failed to revive terminal runtime dispatch command",
    );

    let claim_workspace = temp_workspace("runtime-write-error-dispatch-claim");
    replace_runtime_table_schema(
        &claim_workspace,
        "runtime_dispatch_commands",
        "command_id TEXT PRIMARY KEY, command_state TEXT NOT NULL, created_at TEXT NOT NULL, content TEXT NOT NULL",
    );
    insert_pending_dispatch_command_row(&claim_workspace, &dispatch_command_snapshot(803));
    assert_error_contains(
        adapter.try_claim_next_runtime_dispatch_command(&claim_workspace, "owner-claim"),
        "failed to claim runtime dispatch command",
    );

    let update_workspace = temp_workspace("runtime-write-error-dispatch-update");
    replace_runtime_table_schema(
        &update_workspace,
        "runtime_dispatch_commands",
        "command_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.update_runtime_dispatch_command(&update_workspace, &dispatch_command_snapshot(804)),
        "failed to update runtime dispatch command",
    );

    let slot_workspace = temp_workspace("runtime-write-error-slot-invalid");
    replace_runtime_table_schema(
        &slot_workspace,
        "runtime_invalid_slot_leases",
        "broken_slot_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.upsert_runtime_slot_lease(
            &slot_workspace,
            &slot_lease_for_task(
                "slot-broken-invalid",
                "task-broken-invalid",
                ParallelModeSlotLeaseState::Running,
            ),
        ),
        "failed to clear invalid runtime slot lease `slot-broken-invalid`",
    );

    let session_workspace = temp_workspace("runtime-write-error-session");
    replace_runtime_table_schema(
        &session_workspace,
        "runtime_session_details",
        "session_key TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.upsert_runtime_session_detail(
            &session_workspace,
            &running_session_detail(
                "session-broken",
                "task-broken-session",
                "2026-05-04T12:00:00+00:00",
            ),
        ),
        "failed to persist runtime session detail `session-broken`",
    );

    let block_workspace = temp_workspace("runtime-write-error-block");
    replace_runtime_table_schema(
        &block_workspace,
        "runtime_task_dispatch_blocks",
        "task_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.upsert_runtime_task_dispatch_block(
            &block_workspace,
            &ParallelModeTaskDispatchBlockSnapshot::new(
                "task-broken-block",
                "2026-05-04T11:55:00+00:00",
                "2026-05-04T12:00:00+00:00",
                ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
            ),
        ),
        "failed to persist runtime task dispatch block `task-broken-block`",
    );

    let queue_workspace = temp_workspace("runtime-write-error-queue");
    replace_runtime_table_schema(
        &queue_workspace,
        "runtime_distributor_queue",
        "queue_item_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.upsert_runtime_distributor_queue_record(
            &queue_workspace,
            &queue_record_for_task("queue-broken", "session-broken", "task-broken-queue"),
        ),
        "failed to persist runtime distributor queue record `queue-broken`",
    );
}

#[test]
fn runtime_projection_event_append_errors_keep_projection_context() {
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let enqueue_workspace = temp_workspace("runtime-event-error-dispatch-enqueue");
    corrupt_runtime_events_schema(&enqueue_workspace);
    assert_error_contains(
        adapter
            .enqueue_runtime_dispatch_command(&enqueue_workspace, &dispatch_command_snapshot(811)),
        "failed to append runtime event `dispatch_command_enqueued`",
    );

    let claim_workspace = temp_workspace("runtime-event-error-dispatch-claim");
    adapter
        .enqueue_runtime_dispatch_command(&claim_workspace, &dispatch_command_snapshot(812))
        .expect("claim event seed should enqueue");
    corrupt_runtime_events_schema(&claim_workspace);
    assert_error_contains(
        adapter.try_claim_next_runtime_dispatch_command(&claim_workspace, "owner-event"),
        "failed to append runtime event `dispatch_command_claimed`",
    );

    let update_workspace = temp_workspace("runtime-event-error-dispatch-update");
    corrupt_runtime_events_schema(&update_workspace);
    assert_error_contains(
        adapter.update_runtime_dispatch_command(&update_workspace, &dispatch_command_snapshot(813)),
        "failed to append runtime event `dispatch_command_updated`",
    );

    let slot_workspace = temp_workspace("runtime-event-error-slot-upsert");
    corrupt_runtime_events_schema(&slot_workspace);
    assert_error_contains(
        adapter.upsert_runtime_slot_lease(
            &slot_workspace,
            &slot_lease_for_task(
                "slot-event",
                "task-event-slot",
                ParallelModeSlotLeaseState::Running,
            ),
        ),
        "failed to append runtime event `slot_lease_upsert`",
    );

    let remove_workspace = temp_workspace("runtime-event-error-slot-remove");
    adapter
        .upsert_runtime_slot_lease(
            &remove_workspace,
            &slot_lease_for_task(
                "slot-remove-event",
                "task-remove-event",
                ParallelModeSlotLeaseState::Running,
            ),
        )
        .expect("remove event seed should persist");
    corrupt_runtime_events_schema(&remove_workspace);
    assert_error_contains(
        SqlitePlanningAuthorityAdapter::remove_runtime_slot_lease(
            &remove_workspace,
            "slot-remove-event",
        ),
        "failed to append runtime event `slot_lease_removed`",
    );

    let reset_workspace = temp_workspace("runtime-event-error-reset");
    corrupt_runtime_events_schema(&reset_workspace);
    assert_error_contains(
        adapter.clear_parallel_runtime_projections(&reset_workspace, "broken event table"),
        "failed to append runtime event `parallel_runtime_reset`",
    );

    let task_cleanup_workspace = temp_workspace("runtime-event-error-task-cleanup");
    corrupt_runtime_events_schema(&task_cleanup_workspace);
    assert_error_contains(
        adapter.clear_parallel_runtime_projections_for_tasks(
            &task_cleanup_workspace,
            &["task-event-cleanup".to_string()],
            "broken event table",
        ),
        "failed to append runtime event `parallel_runtime_task_cleanup`",
    );

    let pool_reset_workspace = temp_workspace("runtime-event-error-pool-reset");
    corrupt_runtime_events_schema(&pool_reset_workspace);
    let report = ParallelModePoolResetReport::new(
        ParallelModePoolResetRunId::new("event-error-reset"),
        ParallelModePoolResetPolicy::ForceDisposable,
    );
    assert_error_contains(
        adapter.apply_parallel_pool_reset_report(&pool_reset_workspace, &report),
        "failed to append runtime event `parallel_pool_reset_report_applied`",
    );

    let session_workspace = temp_workspace("runtime-event-error-session");
    corrupt_runtime_events_schema(&session_workspace);
    assert_error_contains(
        adapter.upsert_runtime_session_detail(
            &session_workspace,
            &running_session_detail(
                "session-event",
                "task-event-session",
                "2026-05-04T12:00:00+00:00",
            ),
        ),
        "failed to append runtime event `session_detail_upsert`",
    );

    let block_workspace = temp_workspace("runtime-event-error-block");
    corrupt_runtime_events_schema(&block_workspace);
    assert_error_contains(
        adapter.upsert_runtime_task_dispatch_block(
            &block_workspace,
            &ParallelModeTaskDispatchBlockSnapshot::new(
                "task-event-block",
                "2026-05-04T11:55:00+00:00",
                "2026-05-04T12:00:00+00:00",
                ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
            ),
        ),
        "failed to append runtime event `task_dispatch_block_upsert`",
    );

    let queue_workspace = temp_workspace("runtime-event-error-queue");
    corrupt_runtime_events_schema(&queue_workspace);
    assert_error_contains(
        adapter.upsert_runtime_distributor_queue_record(
            &queue_workspace,
            &queue_record_for_task("queue-event", "session-event", "task-event-queue"),
        ),
        "failed to append runtime event `distributor_queue_upsert`",
    );

    let abandoned_workspace = temp_workspace("runtime-event-error-official-abandon");
    adapter
        .reserve_next_official_refresh_order(&abandoned_workspace)
        .expect("head order should reserve before abandoned event failure");
    adapter
        .reserve_next_official_refresh_order(&abandoned_workspace)
        .expect("tail order should reserve before abandoned event failure");
    corrupt_runtime_events_schema(&abandoned_workspace);
    assert_error_contains(
        adapter.abandon_next_official_refresh_order(&abandoned_workspace, "broken event table"),
        "failed to append runtime event `official_refresh_abandoned`",
    );
}

#[test]
fn runtime_projection_cleanup_error_contexts_report_sqlite_failures() {
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let reserve_workspace = temp_workspace("runtime-cleanup-error-reserve-metadata");
    authority_connection(&reserve_workspace)
        .execute_batch(
            "CREATE TRIGGER fail_next_official_order_metadata
             BEFORE INSERT ON authority_metadata
             WHEN NEW.key = 'next_official_refresh_order'
             BEGIN
                 SELECT RAISE(FAIL, 'forced metadata failure');
             END;",
        )
        .expect("next official metadata trigger should install");
    assert_error_contains(
        adapter.reserve_next_official_refresh_order(&reserve_workspace),
        "failed to update authority metadata `next_official_refresh_order`",
    );

    let release_workspace = temp_workspace("runtime-cleanup-error-release-metadata");
    let release_order = adapter
        .reserve_next_official_refresh_order(&release_workspace)
        .expect("release order should reserve");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&release_workspace, release_order, "release-owner")
            .expect("release order should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    authority_connection(&release_workspace)
        .execute_batch(
            "CREATE TRIGGER fail_next_executable_release_metadata
             BEFORE INSERT ON authority_metadata
             WHEN NEW.key = 'next_executable_refresh_order'
             BEGIN
                 SELECT RAISE(FAIL, 'forced metadata failure');
             END;",
        )
        .expect("release metadata trigger should install");
    assert_error_contains(
        adapter.release_official_refresh_claim(&release_workspace, release_order, "release-owner"),
        "failed to update authority metadata `next_executable_refresh_order`",
    );

    let abandon_workspace = temp_workspace("runtime-cleanup-error-abandon-metadata");
    adapter
        .reserve_next_official_refresh_order(&abandon_workspace)
        .expect("abandon head should reserve");
    adapter
        .reserve_next_official_refresh_order(&abandon_workspace)
        .expect("abandon tail should reserve");
    authority_connection(&abandon_workspace)
        .execute_batch(
            "CREATE TRIGGER fail_next_executable_abandon_metadata
             BEFORE INSERT ON authority_metadata
             WHEN NEW.key = 'next_executable_refresh_order'
             BEGIN
                 SELECT RAISE(FAIL, 'forced metadata failure');
             END;",
        )
        .expect("abandon metadata trigger should install");
    assert_error_contains(
        adapter.abandon_next_official_refresh_order(&abandon_workspace, "metadata failure"),
        "failed to update authority metadata `next_executable_refresh_order`",
    );

    let pool_queue_workspace = temp_workspace("runtime-cleanup-error-pool-queue");
    replace_runtime_table_schema(
        &pool_queue_workspace,
        "runtime_distributor_queue",
        "broken_queue_id TEXT PRIMARY KEY",
    );
    let mut pool_queue_report = ParallelModePoolResetReport::new(
        ParallelModePoolResetRunId::new("pool-queue-error"),
        ParallelModePoolResetPolicy::ForceDisposable,
    );
    pool_queue_report
        .reset_queue_item_ids
        .push("queue-pool-error".to_string());
    assert_error_contains(
        adapter.apply_parallel_pool_reset_report(&pool_queue_workspace, &pool_queue_report),
        "failed to clear reset distributor queue item `queue-pool-error`",
    );

    let pool_claim_workspace = temp_workspace("runtime-cleanup-error-pool-claim");
    replace_runtime_table_schema(
        &pool_claim_workspace,
        "runtime_claims",
        "broken_claim_id TEXT PRIMARY KEY",
    );
    let mut pool_claim_report = ParallelModePoolResetReport::new(
        ParallelModePoolResetRunId::new("pool-claim-error"),
        ParallelModePoolResetPolicy::ForceDisposable,
    );
    pool_claim_report
        .reset_queue_item_ids
        .push("queue-claim-error".to_string());
    assert_error_contains(
        adapter.apply_parallel_pool_reset_report(&pool_claim_workspace, &pool_claim_report),
        "failed to clear reset distributor claim `queue-claim-error`",
    );

    let task_invalid_workspace = temp_workspace("runtime-cleanup-error-task-invalid");
    adapter
        .upsert_runtime_slot_lease(
            &task_invalid_workspace,
            &slot_lease_for_task(
                "slot-cleanup-invalid",
                "task-cleanup-invalid",
                ParallelModeSlotLeaseState::Running,
            ),
        )
        .expect("task invalid seed slot should persist");
    replace_runtime_table_schema(
        &task_invalid_workspace,
        "runtime_invalid_slot_leases",
        "broken_slot_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.clear_parallel_runtime_projections_for_tasks(
            &task_invalid_workspace,
            &["task-cleanup-invalid".to_string()],
            "broken invalid slot table",
        ),
        "failed to clear invalid runtime slot lease `slot-cleanup-invalid`",
    );

    let task_session_workspace = temp_workspace("runtime-cleanup-error-task-session");
    replace_runtime_table_schema(
        &task_session_workspace,
        "runtime_session_details",
        "session_key TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.clear_parallel_runtime_projections_for_tasks(
            &task_session_workspace,
            &["task-cleanup-session".to_string()],
            "broken session table",
        ),
        "failed to clear runtime session details for `task-cleanup-session`",
    );

    let task_block_workspace = temp_workspace("runtime-cleanup-error-task-block");
    replace_runtime_table_schema(
        &task_block_workspace,
        "runtime_task_dispatch_blocks",
        "broken_task_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.clear_parallel_runtime_projections_for_tasks(
            &task_block_workspace,
            &["task-cleanup-block".to_string()],
            "broken block table",
        ),
        "failed to clear runtime task dispatch blocks for `task-cleanup-block`",
    );

    let task_queue_workspace = temp_workspace("runtime-cleanup-error-task-queue");
    adapter
        .upsert_runtime_distributor_queue_record(
            &task_queue_workspace,
            &queue_record_for_task(
                "queue-cleanup-delete",
                "session-cleanup-delete",
                "task-cleanup-queue",
            ),
        )
        .expect("task queue seed should persist");
    install_failing_delete_trigger(
        &task_queue_workspace,
        "runtime_distributor_queue",
        "fail_task_queue_delete",
    );
    assert_error_contains(
        adapter.clear_parallel_runtime_projections_for_tasks(
            &task_queue_workspace,
            &["task-cleanup-queue".to_string()],
            "queue delete trigger",
        ),
        "failed to clear runtime distributor queue records for `task-cleanup-queue`",
    );

    let task_claim_workspace = temp_workspace("runtime-cleanup-error-task-claim");
    adapter
        .upsert_runtime_distributor_queue_record(
            &task_claim_workspace,
            &queue_record_for_task(
                "queue-cleanup-claim",
                "session-cleanup-claim",
                "task-cleanup-claim",
            ),
        )
        .expect("task claim queue seed should persist");
    replace_runtime_table_schema(
        &task_claim_workspace,
        "runtime_claims",
        "broken_claim_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.clear_parallel_runtime_projections_for_tasks(
            &task_claim_workspace,
            &["task-cleanup-claim".to_string()],
            "broken claim table",
        ),
        "failed to clear runtime queue claim `queue-cleanup-claim`",
    );

    let preserve_workspace = temp_workspace("runtime-cleanup-error-preserve-block");
    adapter
        .upsert_runtime_session_detail(
            &preserve_workspace,
            &failed_start_session_detail(
                "session-preserve",
                "task-preserve",
                "2026-05-04T12:00:00+00:00",
            ),
        )
        .expect("preserve seed session should persist");
    replace_runtime_table_schema(
        &preserve_workspace,
        "runtime_task_dispatch_blocks",
        "task_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.clear_parallel_runtime_projections(&preserve_workspace, "broken block table"),
        "failed to preserve failed-start task dispatch block `task-preserve`",
    );

    let stale_queue_workspace = temp_workspace("runtime-cleanup-error-stale-queue-claim");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(
                &stale_queue_workspace,
                "queue-stale-delete",
                "owner-old",
            )
            .expect("stale queue seed claim should acquire")
    );
    set_claim_timestamp(
        &stale_queue_workspace,
        DISTRIBUTOR_QUEUE_CLAIM_KIND,
        "queue-stale-delete",
        "2000-01-01T00:00:00+00:00",
    );
    install_failing_delete_trigger(
        &stale_queue_workspace,
        "runtime_claims",
        "fail_stale_queue_claim_delete",
    );
    assert_error_contains(
        adapter.try_acquire_distributor_queue_claim(
            &stale_queue_workspace,
            "queue-stale-delete",
            "owner-new",
        ),
        "failed to clear stale runtime claim `distributor-queue-head:queue-stale-delete`",
    );

    let stale_official_workspace = temp_workspace("runtime-cleanup-error-stale-official-claim");
    let stale_order = adapter
        .reserve_next_official_refresh_order(&stale_official_workspace)
        .expect("stale official order should reserve");
    adapter
        .reserve_next_official_refresh_order(&stale_official_workspace)
        .expect("stale official tail should reserve");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&stale_official_workspace, stale_order, "owner-old")
            .expect("stale official claim should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    set_claim_timestamp(
        &stale_official_workspace,
        "official-refresh",
        OFFICIAL_REFRESH_SCOPE_KEY,
        "2000-01-01T00:00:00+00:00",
    );
    install_failing_delete_trigger(
        &stale_official_workspace,
        "runtime_claims",
        "fail_stale_official_claim_delete",
    );
    assert_error_contains(
        adapter.abandon_next_official_refresh_order(&stale_official_workspace, "stale failure"),
        "failed to clear stale runtime claim `official-refresh:official-refresh`",
    );
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

/*
`store.rs`는 SQLite planning authority adapter의 중심 저장소 모듈이다.

이 파일은 "DB 파일을 열었을 때 어떤 schema를 보장하고, domain/application record를 어떤 조회 형태로
다시 꺼낼 것인가"를 담당한다. 이미 별도 모듈로 분리된 active document, draft, task authority row
로직이 있지만, 이 파일은 여전히 전체 authority store의 공통 기반을 잡는다.

이번 구간의 초점은 저장소 기반 계약이다.
- `ensure_schema`는 모든 하위 projection이 의존하는 테이블과 인덱스를 만든다.
- shadow document 함수들은 파일 기반 planning workspace에서 읽은 내용을 DB에 mirror한다.
- metadata 함수들은 schema/mode/root/timestamp처럼 DB snapshot 전체를 설명하는 값을 기록한다.
- shadow load는 이전 schema나 빈 DB에서도 안전하게 빈 map으로 돌아가는 호환성 경계를 제공한다.
*/
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthoritySnapshot, PlanningTaskAuthoritySnapshot,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningAuthorityLocation,
    PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskAuthorityDocument,
};

use super::{
    AUTHORITY_STORE_MODE, AUTHORITY_STORE_SCHEMA_VERSION, TASK_LEDGER_VERSION_METADATA_KEY,
    read_metadata_i64_connection, read_metadata_string_connection, replace_task_authority_tables,
    table_exists,
};

/*
authority store가 필요로 하는 전체 schema를 idempotent하게 보장한다.

이 함수는 `CREATE TABLE IF NOT EXISTS`와 `CREATE INDEX IF NOT EXISTS`만 사용하므로, 새 DB 파일뿐 아니라
이미 존재하는 DB 파일에 여러 번 호출해도 같은 결과를 유지한다. adapter의 다른 함수들은 자기 작업에
필요한 테이블이 이미 있다고 가정하므로, DB connection을 만든 직후 이 schema 초기화가 먼저 실행되어야
한다.

테이블 묶음별 의미는 다음과 같다.
- `authority_metadata`: schema version, 저장 mode, canonical repo root, 최근 갱신 시각 같은 store 전체 정보이다.
- `shadow_documents`: 파일시스템 workspace에서 읽은 planning 파일의 mirror이다.
- `staged_drafts` / `staged_draft_files`: repo-scoped draft staging 영역이다.
- `active_documents`: commit된 planning workspace snapshot이다.
- `planning_direction_*`: direction authority 문서와 방향별 JSON 원문이다.
- `planning_tasks` / `planning_task_edges` / `planning_queue_projection`: task authority와 queue projection이다.
- `runtime_*`: app-server/parallel runtime에서 쓰는 lease, session, queue, event projection이다.

schema가 한 함수에 모여 있는 이유는 projection 모듈들이 서로 다른 테이블을 만져도 migration 기준은
하나여야 하기 때문이다. 분산된 `CREATE TABLE`은 버전 추적과 테스트 초기화를 어렵게 만든다.
*/
pub(super) fn ensure_schema(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS authority_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS shadow_documents (
                relative_path TEXT PRIMARY KEY,
                content TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS staged_drafts (
                draft_name TEXT PRIMARY KEY,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS staged_draft_files (
                draft_name TEXT NOT NULL,
                active_path TEXT NOT NULL,
                content TEXT NOT NULL,
                PRIMARY KEY (draft_name, active_path),
                FOREIGN KEY (draft_name) REFERENCES staged_drafts(draft_name) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS active_documents (
                relative_path TEXT PRIMARY KEY,
                content TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS planning_direction_config (
                config_key TEXT PRIMARY KEY,
                version INTEGER NOT NULL,
                queue_idle_policy TEXT NOT NULL,
                queue_idle_prompt_path TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS planning_directions (
                direction_id TEXT PRIMARY KEY,
                direction_order INTEGER NOT NULL,
                title TEXT NOT NULL,
                state TEXT NOT NULL,
                detail_doc_path TEXT NOT NULL,
                content_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS planning_tasks (
                task_id TEXT PRIMARY KEY,
                task_order INTEGER NOT NULL,
                direction_id TEXT NOT NULL,
                title TEXT NOT NULL,
                status TEXT NOT NULL,
                base_priority INTEGER NOT NULL,
                dynamic_priority_delta INTEGER NOT NULL,
                combined_priority INTEGER NOT NULL,
                updated_at TEXT NOT NULL,
                source_turn_id TEXT,
                origin_session_kind TEXT,
                thread_id TEXT,
                turn_id TEXT,
                parent_thread_id TEXT,
                parent_turn_id TEXT,
                content_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS planning_task_edges (
                task_id TEXT NOT NULL,
                edge_kind TEXT NOT NULL,
                target_task_id TEXT NOT NULL,
                edge_order INTEGER NOT NULL,
                PRIMARY KEY (task_id, edge_kind, edge_order),
                FOREIGN KEY (task_id) REFERENCES planning_tasks(task_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS planning_queue_projection (
                bucket TEXT NOT NULL,
                rank INTEGER NOT NULL,
                task_id TEXT NOT NULL,
                item_kind TEXT NOT NULL,
                content_json TEXT NOT NULL,
                PRIMARY KEY (bucket, rank, task_id),
                FOREIGN KEY (task_id) REFERENCES planning_tasks(task_id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_planning_tasks_status_priority_updated
                ON planning_tasks(status, combined_priority, updated_at);
            CREATE INDEX IF NOT EXISTS idx_planning_tasks_direction
                ON planning_tasks(direction_id);
            CREATE INDEX IF NOT EXISTS idx_planning_directions_order
                ON planning_directions(direction_order, direction_id);
            CREATE INDEX IF NOT EXISTS idx_planning_task_edges_lookup
                ON planning_task_edges(target_task_id, edge_kind);
            CREATE INDEX IF NOT EXISTS idx_planning_queue_projection_bucket_rank
                ON planning_queue_projection(bucket, rank);

            CREATE TABLE IF NOT EXISTS runtime_claims (
                claim_kind TEXT NOT NULL,
                scope_key TEXT NOT NULL,
                owner_token TEXT NOT NULL,
                claim_value TEXT NOT NULL,
                claimed_at TEXT NOT NULL,
                PRIMARY KEY (claim_kind, scope_key)
            );

            CREATE TABLE IF NOT EXISTS runtime_slot_leases (
                slot_id TEXT PRIMARY KEY,
                updated_at TEXT NOT NULL,
                content TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS runtime_invalid_slot_leases (
                slot_id TEXT PRIMARY KEY,
                detected_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS runtime_session_details (
                session_key TEXT PRIMARY KEY,
                slot_id TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                content TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS runtime_task_dispatch_blocks (
                task_id TEXT PRIMARY KEY,
                reason TEXT NOT NULL,
                task_updated_at TEXT NOT NULL,
                blocked_at TEXT NOT NULL,
                content TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS runtime_distributor_queue (
                queue_item_id TEXT PRIMARY KEY,
                session_key TEXT NOT NULL,
                queue_state TEXT NOT NULL,
                enqueued_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                content TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS runtime_dispatch_commands (
                command_id TEXT PRIMARY KEY,
                command_kind TEXT NOT NULL,
                trigger TEXT NOT NULL,
                command_state TEXT NOT NULL,
                queue_head_signature TEXT,
                epoch_id INTEGER,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                owner_token TEXT,
                content TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS runtime_events (
                sequence INTEGER PRIMARY KEY,
                event_kind TEXT NOT NULL,
                projection_kind TEXT NOT NULL,
                projection_key TEXT NOT NULL,
                observed_planning_revision INTEGER NOT NULL,
                summary TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                recorded_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS app_server_prompt_interactions (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                interaction_id TEXT NOT NULL,
                session_kind TEXT NOT NULL,
                operation TEXT NOT NULL,
                service_name TEXT,
                thread_id TEXT,
                turn_id TEXT,
                status TEXT NOT NULL,
                started_at TEXT NOT NULL,
                completed_at TEXT NOT NULL,
                content_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_app_server_prompt_interactions_recent
                ON app_server_prompt_interactions(sequence DESC);
            CREATE INDEX IF NOT EXISTS idx_app_server_prompt_interactions_thread
                ON app_server_prompt_interactions(thread_id, turn_id);
            "#,
        )
        .context("failed to initialize authority-store schema")?;
    ensure_planning_task_provenance_columns(connection)?;
    Ok(())
}

fn ensure_planning_task_provenance_columns(connection: &Connection) -> Result<()> {
    for (column_name, column_definition) in [
        ("origin_session_kind", "origin_session_kind TEXT"),
        ("thread_id", "thread_id TEXT"),
        ("turn_id", "turn_id TEXT"),
        ("parent_thread_id", "parent_thread_id TEXT"),
        ("parent_turn_id", "parent_turn_id TEXT"),
    ] {
        if !planning_tasks_column_exists(connection, column_name)? {
            connection
                .execute(
                    &format!("ALTER TABLE planning_tasks ADD COLUMN {column_definition}"),
                    [],
                )
                .with_context(|| {
                    format!("failed to add planning_tasks provenance column `{column_name}`")
                })?;
        }
    }
    Ok(())
}

fn planning_tasks_column_exists(connection: &Connection, column_name: &str) -> Result<bool> {
    let mut statement = connection
        .prepare("PRAGMA table_info(planning_tasks)")
        .context("failed to inspect planning_tasks schema")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}

/*
파일시스템에서 읽은 planning 문서들을 `shadow_documents` 테이블에 전체 교체 방식으로 저장한다.

shadow store는 "현재 repo의 planning 파일들이 DB 관점에서 어떻게 보이는가"를 나타내는 mirror이다.
증분 patch를 시도하지 않고 먼저 테이블을 비우는 이유는 파일 삭제를 정확하게 반영하기 위해서이다.
입력 `documents`에 없는 파일이 DB에 남아 있으면 이후 authority snapshot 로더가 삭제된 파일을 여전히
존재하는 것처럼 볼 수 있다.

이 함수는 transaction 안에서 shadow rows와 metadata timestamp를 함께 갱신한다. 따라서 mirror 내용은
항상 `last_synced_at`과 같은 commit 시점의 상태로 해석할 수 있다.
*/
pub(super) fn store_shadow_documents(
    connection: &mut Connection,
    location: &PlanningAuthorityLocation,
    documents: &BTreeMap<String, String>,
) -> Result<()> {
    let transaction = connection
        .transaction()
        .context("failed to open shadow-store transaction")?;
    transaction
        .execute("DELETE FROM shadow_documents", [])
        .context("failed to clear shadow documents")?;
    for (relative_path, content) in documents {
        transaction
            .execute(
                "INSERT INTO shadow_documents (relative_path, content) VALUES (?1, ?2)",
                params![relative_path, content],
            )
            .with_context(|| format!("failed to mirror `{relative_path}` into the shadow store"))?;
    }

    upsert_authority_metadata(&transaction, location, "last_synced_at")?;

    transaction
        .commit()
        .context("failed to commit shadow-store transaction")?;
    Ok(())
}

/*
authority DB 전체를 설명하는 공통 metadata를 upsert한다.

`timestamp_key`를 인자로 받는 이유는 호출 맥락마다 "무엇이 갱신되었는지"가 다르기 때문이다.
shadow sync는 `last_synced_at`, draft staging은 `last_draft_updated_at`처럼 다른 key를 넘긴다.
하지만 schema version, mode, canonical repo root, workspace root는 모든 갱신에서 함께 확인되어야 하는
store identity 값이다.
*/
pub(super) fn upsert_authority_metadata(
    transaction: &rusqlite::Transaction<'_>,
    location: &PlanningAuthorityLocation,
    timestamp_key: &str,
) -> Result<()> {
    upsert_metadata(
        transaction,
        "schema_version",
        &AUTHORITY_STORE_SCHEMA_VERSION.to_string(),
    )?;
    upsert_metadata(transaction, "mode", AUTHORITY_STORE_MODE)?;
    upsert_metadata(
        transaction,
        "canonical_repo_root",
        &location.canonical_repo_root,
    )?;
    upsert_metadata(transaction, "workspace_root", &location.workspace_root)?;
    upsert_metadata(transaction, timestamp_key, &Utc::now().to_rfc3339())?;
    Ok(())
}

/*
metadata key/value 하나를 insert-or-update한다.

metadata는 단순한 문자열 map이지만, 여러 저장 흐름에서 같은 key를 반복 갱신한다. `ON CONFLICT(key)`
절을 사용하면 caller가 "처음 쓰는 값인지, 기존 값을 갱신하는지"를 구분하지 않아도 된다. 이 함수는
작은 helper지만 schema version, mode, ledger version처럼 store 해석에 필요한 기준값을 쓰는 공통
입구이다.
*/
pub(super) fn upsert_metadata(
    transaction: &rusqlite::Transaction<'_>,
    key: &str,
    value: &str,
) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO authority_metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .with_context(|| format!("failed to update authority metadata `{key}`"))?;
    Ok(())
}

/*
shadow store에 mirror된 planning 문서들을 deterministic map으로 읽어온다.

`table_exists`를 먼저 확인하는 것은 이전 버전 DB나 아직 schema 초기화가 끝나지 않은 테스트 DB를
읽을 때의 방어 장치이다. shadow table이 없으면 "mirror가 없다"는 의미로 빈 map을 반환한다.

조회는 `ORDER BY relative_path`를 사용하고 결과 컨테이너도 `BTreeMap`이다. 둘 다 출력 순서를
안정화하기 위한 선택이다. planning 문서는 사람이 diff로 검토하는 일이 많기 때문에, DB가 반환하는
행 순서가 실행마다 달라지는 상황을 피해야 한다.
*/
pub(super) fn load_shadow_documents(connection: &Connection) -> Result<BTreeMap<String, String>> {
    if !table_exists(connection, "shadow_documents")? {
        return Ok(BTreeMap::new());
    }

    let mut statement = connection
        .prepare("SELECT relative_path, content FROM shadow_documents ORDER BY relative_path")
        .context("failed to read shadow-store documents")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate shadow-store documents")?;
    let mut documents = BTreeMap::new();
    for row in rows {
        let (relative_path, content) = row.context("failed to decode shadow-store row")?;
        documents.insert(relative_path, content);
    }

    Ok(documents)
}

/*
commit된 active planning 문서 snapshot을 모두 읽어온다.

`shadow_documents`가 파일시스템 mirror라면, `active_documents`는 authority DB가 현재 활성 workspace로
간주하는 확정본이다. repo-scoped flow에서는 draft를 staging한 뒤 commit하면 active snapshot이
바뀌고, 이후 TUI나 service는 이 active snapshot을 기준으로 결과 출력 파일과 authority 문서를 본다.

정렬과 `BTreeMap` 사용은 shadow load와 같은 이유이다. 같은 DB 상태는 항상 같은 iteration 순서를
만들어야 diff, 테스트, TUI rendering이 안정된다.
*/
pub(super) fn load_active_documents(connection: &Connection) -> Result<BTreeMap<String, String>> {
    let mut statement = connection
        .prepare("SELECT relative_path, content FROM active_documents ORDER BY relative_path")
        .context("failed to read active authority documents")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate active authority documents")?;
    let mut documents = BTreeMap::new();
    for row in rows {
        let (relative_path, content) = row.context("failed to decode active authority row")?;
        documents.insert(relative_path, content);
    }

    Ok(documents)
}

/*
active authority document map을 그대로 노출하는 얇은 별칭이다.

이 함수는 현재 `load_active_documents`에 바로 위임하지만 이름을 따로 둔다. 호출자 입장에서는
"active workspace의 일반 문서 map"을 읽는지, "authority 문서 저장소"를 읽는지 의도가 다를 수 있기
때문이다. 같은 구현을 공유하더라도 adapter 내부 API 이름으로 경계 의미를 남긴다.
*/
pub(super) fn load_active_authority_documents(
    connection: &Connection,
) -> Result<BTreeMap<String, String>> {
    load_active_documents(connection)
}

/*
active documents에서 `PlanningWorkspaceLoadRecord`를 구성한다.

workspace port가 필요로 하는 load record는 전체 파일 map이 아니라 현재 결과 출력 markdown이다.
그래서 active snapshot 전체를 읽은 뒤 `RESULT_OUTPUT_FILE_PATH`만 골라 optional field에 넣는다.
파일이 없으면 `None`이 되며, 이는 아직 결과 문서가 생성되지 않은 정상 상태를 뜻한다.
*/
pub(super) fn load_active_workspace_record(
    connection: &Connection,
) -> Result<PlanningWorkspaceLoadRecord> {
    let documents = load_active_documents(connection)?;
    Ok(PlanningWorkspaceLoadRecord {
        result_output_markdown: documents.get(RESULT_OUTPUT_FILE_PATH).cloned(),
    })
}

/*
direction authority snapshot을 DB에서 복원한다.

snapshot은 direction catalog 본문과 planning revision metadata의 조합이다. direction catalog가 없으면
아직 direction authority가 초기화되지 않은 상태이므로 `Ok(None)`을 반환한다. catalog는 있는데
`planning_revision` metadata가 없으면 이전 schema나 초기 상태로 보고 0을 사용한다.
*/
pub(super) fn load_direction_authority_snapshot_from_connection(
    connection: &Connection,
) -> Result<Option<PlanningDirectionAuthoritySnapshot>> {
    let Some(directions) = load_direction_catalog_from_connection(connection)? else {
        return Ok(None);
    };
    let planning_revision =
        read_metadata_i64_connection(connection, "planning_revision")?.unwrap_or(0);
    Ok(Some(PlanningDirectionAuthoritySnapshot {
        planning_revision,
        directions,
    }))
}

/*
direction authority 테이블들을 domain의 `DirectionCatalogDocument`로 다시 조립한다.

저장 구조는 config row 하나와 direction row 여러 개로 나뉜다. config row에는 catalog 전체에 적용되는
format version과 queue-idle 설정이 들어 있고, `planning_directions`에는 각 direction의 JSON 원문이
순서와 함께 들어 있다. 이 함수는 그 둘을 합쳐 application 계층이 기대하는 catalog 문서 형태로
복원한다.

`direction_authority_exists`를 먼저 확인하는 이유는 "authority가 없음"과 "authority는 있는데 direction이
0개"를 구분하기 위해서이다. 현재 schema에서는 config row 존재가 direction authority snapshot의
존재 신호이다.
*/
pub(super) fn load_direction_catalog_from_connection(
    connection: &Connection,
) -> Result<Option<DirectionCatalogDocument>> {
    if !direction_authority_exists(connection)? {
        return Ok(None);
    }
    let (version, queue_idle_policy, queue_idle_prompt_path) = connection
        .query_row(
            "SELECT version, queue_idle_policy, queue_idle_prompt_path
             FROM planning_direction_config
             WHERE config_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u32,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .context("failed to read planning direction config")?
        .unwrap_or((PLANNING_FORMAT_VERSION, "stop".to_string(), String::new()));
    let mut statement = connection
        .prepare(
            "SELECT direction_id, content_json
             FROM planning_directions
             ORDER BY direction_order ASC, direction_id ASC",
        )
        .context("failed to read planning direction rows")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate planning direction rows")?;
    let mut directions = Vec::new();
    /*
        row에는 `direction_id` column과 `content_json`이 함께 있지만, 복원 결과는 JSON 안의 domain 구조를
    신뢰한다. `direction_id`는 오류 메시지에 붙여 어느 행의 JSON decode가 실패했는지 보여주는
    진단 정보로 사용된다.
    */
    for row in rows {
        let (direction_id, content_json) =
            row.context("failed to decode planning direction row")?;
        directions.push(serde_json::from_str(&content_json).with_context(|| {
            format!("failed to deserialize planning direction row `{direction_id}`")
        })?);
    }
    /*
        queue-idle 설정은 DB column 두 개로 저장되어 있지만 domain 타입은 structured enum/record이다.
    작은 JSON value로 다시 감싼 뒤 serde가 domain 타입으로 decode하게 해서, 문자열 policy 해석 규칙을
    이 store 함수 안에 중복 구현하지 않는다.
    */
    let queue_idle = serde_json::from_value(serde_json::json!({
        "policy": queue_idle_policy,
        "prompt_path": queue_idle_prompt_path,
    }))
    .context("failed to decode planning direction queue-idle config")?;

    Ok(Some(DirectionCatalogDocument {
        version,
        queue_idle,
        directions,
    }))
}

/*
active snapshot에서 특정 문서 하나만 읽는다.

대부분의 load 함수는 전체 map을 복원하지만, 일부 호출자는 상대 경로 하나의 본문만 필요하다. 이
helper는 `OptionalExtension`을 사용해 row 없음과 SQL 오류를 분리한다. row가 없으면 정상적으로
`Ok(None)`이고, DB 조회 자체가 실패하면 context가 붙은 error가 된다.
*/
pub(super) fn load_active_document(
    connection: &Connection,
    relative_path: &str,
) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT content FROM active_documents WHERE relative_path = ?1",
            params![relative_path],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("failed to read active authority document `{relative_path}`"))
}

/*
task authority snapshot을 DB에서 복원한다.

task authority snapshot은 세 가지 조각의 조합이다.
1. `planning_tasks`에 저장된 `TaskAuthorityDocument`
2. `planning_queue_projection`에 저장된 현재 queue 계산 결과
3. `authority_metadata`에 저장된 `planning_revision`

task 문서가 없으면 아직 task authority가 초기화되지 않은 상태이므로 `Ok(None)`이다. 반대로 task
문서는 있는데 queue projection이 없으면, 이전 schema나 부분 초기화 상태를 고려해 빈 projection을
사용한다. 이렇게 하면 application 계층은 snapshot을 받았을 때 queue field가 항상 존재한다고
가정할 수 있다.
*/
pub(super) fn load_task_authority_snapshot_from_connection(
    connection: &Connection,
) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
    let Some(task_authority) = load_task_authority_from_connection(connection)? else {
        return Ok(None);
    };
    let queue_projection =
        load_queue_projection_from_connection(connection)?.unwrap_or_else(empty_queue_projection);
    let planning_revision =
        read_metadata_i64_connection(connection, "planning_revision")?.unwrap_or(0);
    Ok(Some(PlanningTaskAuthoritySnapshot {
        planning_revision,
        task_authority,
        queue_projection,
    }))
}

/*
`planning_tasks` 행들을 domain의 `TaskAuthorityDocument`로 복원한다.

저장할 때 task row는 조회용 column들과 `content_json`을 함께 갖다. 복원에서는 JSON 원문을
신뢰한다. column들은 SQL 조회/정렬/인덱싱을 위한 projection이고, domain 객체의 전체 형태는
`content_json`에 보존되어 있기 때문이다. `task_id` column은 JSON decode 오류를 설명하는 context로
사용한다.

version은 `authority_metadata`의 task ledger version에서 읽는다. 값이 없으면 format 기본 버전을
사용해 오래된 DB도 읽을 수 있게 한다.
*/
pub(super) fn load_task_authority_from_connection(
    connection: &Connection,
) -> Result<Option<TaskAuthorityDocument>> {
    if !task_authority_exists(connection)? {
        return Ok(None);
    }

    let version = read_metadata_i64_connection(connection, TASK_LEDGER_VERSION_METADATA_KEY)?
        .unwrap_or(i64::from(PLANNING_FORMAT_VERSION)) as u32;
    let mut statement = connection
        .prepare(
            "SELECT task_id, content_json
             FROM planning_tasks
             ORDER BY task_order ASC, task_id ASC",
        )
        .context("failed to read planning task rows")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate planning task rows")?;
    let mut tasks = Vec::new();
    for row in rows {
        let (task_id, content_json) = row.context("failed to decode planning task row")?;
        tasks.push(
            serde_json::from_str(&content_json)
                .with_context(|| format!("failed to deserialize planning task row `{task_id}`"))?,
        );
    }

    Ok(Some(TaskAuthorityDocument { version, tasks }))
}

/*
queue projection rows를 `PriorityQueueProjection` 구조로 복원한다.

`planning_queue_projection` 테이블은 active/proposed/skipped를 같은 schema에 담는다. bucket과
item_kind가 row 의미를 나누고, `content_json`은 각 bucket에 맞는 projection 타입으로 decode된다.
이 함수는 bucket을 기준으로 세 Vec에 분배한다.

알 수 없는 bucket은 무시한다. 이는 forward compatibility를 위한 느슨한 처리이다. 새 bucket이
추가된 DB를 오래된 binary가 읽을 때 전체 load를 실패시키기보다, 현재 binary가 이해하는 bucket만
복원한다.
*/
pub(super) fn load_queue_projection_from_connection(
    connection: &Connection,
) -> Result<Option<PriorityQueueProjection>> {
    if !task_authority_exists(connection)? {
        return Ok(None);
    }

    let mut active_tasks = Vec::new();
    let mut proposed_tasks = Vec::new();
    let mut skipped_tasks = Vec::new();
    let mut statement = connection
        .prepare(
            "SELECT bucket, task_id, content_json
             FROM planning_queue_projection
             ORDER BY bucket ASC, rank ASC, task_id ASC",
        )
        .context("failed to read planning queue projection")?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .context("failed to iterate planning queue projection")?;
    /*
        row의 `task_id`는 projection JSON에도 들어 있지만 오류 메시지를 위해 column 값을 함께 읽는다.
    queue projection은 task authority 문서의 파생 결과라 JSON decode가 실패하면 DB snapshot 자체가
    깨진 것이므로, 어느 bucket/task에서 실패했는지 최대한 좁혀 준다.
    */
    for row in rows {
        let (bucket, task_id, content_json) =
            row.context("failed to decode planning queue projection row")?;
        match bucket.as_str() {
            "active" => {
                active_tasks.push(
                    serde_json::from_str::<PriorityQueueTask>(&content_json).with_context(
                        || format!("failed to deserialize active queue projection `{task_id}`"),
                    )?,
                );
            }
            "proposed" => {
                proposed_tasks.push(
                    serde_json::from_str::<PriorityQueueTask>(&content_json).with_context(
                        || format!("failed to deserialize proposed queue projection `{task_id}`"),
                    )?,
                );
            }
            "skipped" => {
                skipped_tasks.push(
                    serde_json::from_str::<PriorityQueueSkippedTask>(&content_json).with_context(
                        || format!("failed to deserialize skipped queue projection `{task_id}`"),
                    )?,
                );
            }
            _ => {}
        }
    }

    Ok(Some(PriorityQueueProjection {
        // active queue의 첫 항목이 TUI와 worker가 우선 볼 다음 task가 된다.
        next_task: active_tasks.first().cloned(),
        active_tasks,
        proposed_tasks,
        skipped_tasks,
    }))
}

/*
DB에 task authority snapshot이 존재하는지 확인한다.

가장 명확한 신호는 `TASK_LEDGER_VERSION_METADATA_KEY` metadata이다. 다만 이전 버전 DB나 partial write
상황에서는 metadata 없이 `planning_tasks` row만 있을 수 있으므로, fallback으로 task row 존재도
확인한다. 이 함수는 load 계열에서 `None`과 실제 빈 document를 구분하는 gate 역할을 한다.
*/
pub(super) fn task_authority_exists(connection: &Connection) -> Result<bool> {
    if read_metadata_string_connection(connection, TASK_LEDGER_VERSION_METADATA_KEY)?.is_some() {
        return Ok(true);
    }
    connection
        .query_row("SELECT 1 FROM planning_tasks LIMIT 1", [], |_| Ok(()))
        .optional()
        .context("failed to inspect planning task authority")
        .map(|value| value.is_some())
}

/*
queue projection이 없을 때 사용할 구조적으로 완전한 빈 projection을 만든다.

`PriorityQueueProjection`은 option이 아니라 내부 Vec들을 가진 값이므로, caller가 매번 None 처리를
반복하지 않게 이 helper에서 빈 active/proposed/skipped 목록과 `next_task: None`을 함께 제공한다.
*/
pub(super) fn empty_queue_projection() -> PriorityQueueProjection {
    PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    }
}

/*
direction catalog 변경 후 task authority가 가리키는 direction 참조를 정리한다.

task는 반드시 존재하는 direction에 속해야 한다. direction authority가 교체되거나 지워지면 기존 task 중
사라진 direction을 참조하는 것들이 생길 수 있다. 이 함수는 현재 task authority를 읽고, 유효한
direction id 집합에 맞지 않는 task를 제거한 뒤, 남은 task의 depends_on/blocked_by에서도 제거된 task를
참조하는 edge를 지운다.

queue projection은 pruning 후 `empty_queue_projection`으로 초기화한다. task 집합이 바뀌면 기존 rank와
next task 계산은 더 이상 신뢰할 수 없으므로, 다음 계산 흐름이 새 projection을 만들게 하는 것이 안전한다.
*/
pub(super) fn reconcile_task_authority_with_directions(
    transaction: &rusqlite::Transaction<'_>,
    directions: Option<&DirectionCatalogDocument>,
) -> Result<()> {
    let Some(mut task_authority) = load_task_authority_from_connection(transaction)? else {
        return Ok(());
    };
    let direction_ids = match directions {
        Some(directions) => direction_ids(directions),
        None => BTreeSet::new(),
    };
    if !prune_task_authority_to_direction_ids(&mut task_authority, &direction_ids) {
        return Ok(());
    }
    replace_task_authority_tables(transaction, &task_authority, &empty_queue_projection())?;
    Ok(())
}

/*
direction catalog에서 유효한 direction id 집합만 추출한다.

이 helper는 task pruning과 validation에서 "현재 살아 있는 direction"의 기준을 만든다. id는 trim해서
저장한다. direction 문서 작성 과정에서 공백이 섞여도 DB의 task direction_id 비교는 정규화된
문자열끼리 이루어져야 하기 때문이다.
*/
pub(super) fn direction_ids(directions: &DirectionCatalogDocument) -> BTreeSet<String> {
    directions
        .directions
        .iter()
        .map(|direction| direction.id.trim().to_string())
        .collect()
}

/*
direction authority tables를 새 catalog 문서로 전체 교체한다.

direction authority는 config row 하나와 direction row 여러 개로 나뉘어 저장된다. config row에는 catalog
전체 version과 queue idle 정책이 들어가고, 각 direction row에는 조회용 column과 JSON 원문이 함께
들어간다. task 저장과 마찬가지로 SQL에서 자주 볼 필드는 column으로 풀고, domain 복원에는
`content_json`을 사용한다.

먼저 기존 row를 모두 지우는 이유는 삭제된 direction을 정확히 반영하기 위해서이다. direction은 개수가
많지 않고 catalog 전체가 하나의 권위 문서이므로, 부분 upsert보다 전체 재생성이 의미가 더 분명한다.
*/
pub(super) fn replace_direction_authority_tables(
    transaction: &rusqlite::Transaction<'_>,
    directions: &DirectionCatalogDocument,
) -> Result<()> {
    clear_direction_authority_rows(transaction)?;
    transaction
        .execute(
            "INSERT INTO planning_direction_config
             (config_key, version, queue_idle_policy, queue_idle_prompt_path)
             VALUES ('default', ?1, ?2, ?3)",
            params![
                directions.version,
                directions.queue_idle.policy.label(),
                directions.queue_idle.prompt_path.trim(),
            ],
        )
        .context("failed to persist planning direction config")?;
    for (index, direction) in directions.directions.iter().enumerate() {
        let direction_id = direction.id.trim();
        transaction
            .execute(
                "INSERT INTO planning_directions
                 (direction_id, direction_order, title, state, detail_doc_path, content_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    direction_id,
                    index as i64,
                    direction.title.trim(),
                    direction.state.label(),
                    direction.detail_doc_path.trim(),
                    serde_json::to_string(direction)
                        .context("failed to serialize planning direction row")?,
                ],
            )
            .with_context(|| format!("failed to persist planning direction `{direction_id}`"))?;
    }
    /*
        `direction_authority_version`은 load 쪽의 존재 판단에는 직접 쓰이지 않지만, DB를 열어 보는 도구나
    미래 migration이 현재 저장된 direction catalog version을 빠르게 확인할 수 있게 하는 metadata이다.
    */
    upsert_metadata(
        transaction,
        "direction_authority_version",
        &directions.version.to_string(),
    )?;
    Ok(())
}

/*
direction authority를 비운 뒤 metadata version을 0으로 표시한다.

row를 삭제하는 것만으로도 `direction_authority_exists`는 false를 반환한다. 여기에 version 0 metadata를
쓰는 것은 "명시적으로 비워진 상태"를 store metadata에서도 읽을 수 있게 하기 위한 표식이다.
*/
pub(super) fn clear_direction_authority_tables(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<()> {
    clear_direction_authority_rows(transaction)?;
    upsert_metadata(transaction, "direction_authority_version", "0")?;
    Ok(())
}

/*
direction authority의 실제 row들을 삭제한다.

direction rows를 먼저 지우고 config를 나중에 지우면, 중간 실패가 발생하더라도 config만 남아 있는 상태를
줄일 수 있다. 이 함수는 보통 외부 transaction 안에서 호출되므로 최종 원자성은 caller의 commit이
보장한다.
*/
fn clear_direction_authority_rows(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction
        .execute("DELETE FROM planning_directions", [])
        .context("failed to clear planning direction rows")?;
    transaction
        .execute("DELETE FROM planning_direction_config", [])
        .context("failed to clear planning direction config")?;
    Ok(())
}

/*
direction authority가 DB에 존재하는지 확인한다.

현재 존재 신호는 `planning_directions`에 최소 한 row가 있는지이다. config row만으로 판단하지 않는
이유는 config는 기본값이나 metadata 성격이 강하고, 실제 direction catalog의 핵심은 direction 목록이기
때문이다.
*/
fn direction_authority_exists(connection: &Connection) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM planning_directions LIMIT 1)",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists != 0)
        .context("failed to inspect planning direction rows")
}

/*
task authority 문서를 현재 유효한 direction id 집합에 맞게 in-place로 정리한다.

이 함수는 두 단계로 동작한다.
1. 존재하지 않는 direction을 참조하는 task를 제거하고, 제거된 task id를 모은다.
2. 남은 task들의 `depends_on` / `blocked_by`에서 제거된 task id를 참조하는 edge를 삭제한다.

반환값은 실제로 문서가 바뀌었는지 여부이다. caller는 false면 DB rewrite를 생략할 수 있고, true면
정리된 task authority와 빈 queue projection을 다시 저장한다. 빈 direction id 집합을 넘기면 모든 task가
제거되므로, direction authority가 삭제된 경우에도 같은 helper를 재사용할 수 있다.
*/
pub(super) fn prune_task_authority_to_direction_ids(
    task_authority: &mut TaskAuthorityDocument,
    direction_ids: &BTreeSet<String>,
) -> bool {
    let mut removed_task_ids = BTreeSet::new();
    task_authority.tasks.retain(|task| {
        let keep = direction_ids.contains(task.direction_id.trim());
        if !keep {
            removed_task_ids.insert(task.id.trim().to_string());
        }
        keep
    });
    if removed_task_ids.is_empty() {
        return false;
    }
    for task in &mut task_authority.tasks {
        task.depends_on
            .retain(|task_id| !removed_task_ids.contains(task_id.trim()));
        task.blocked_by
            .retain(|task_id| !removed_task_ids.contains(task_id.trim()));
    }
    true
}

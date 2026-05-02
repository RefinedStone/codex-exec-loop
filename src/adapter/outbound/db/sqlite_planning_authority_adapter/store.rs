/*
학습 주석:
`store.rs`는 SQLite planning authority adapter의 중심 저장소 모듈입니다.

이 파일은 "DB 파일을 열었을 때 어떤 schema를 보장하고, domain/application record를 어떤 조회 형태로
다시 꺼낼 것인가"를 담당합니다. 이미 별도 모듈로 분리된 active document, draft, task authority row
로직이 있지만, 이 파일은 여전히 전체 authority store의 공통 기반을 잡습니다.

이번 구간의 초점은 저장소 기반 계약입니다.
- `ensure_schema`는 모든 하위 projection이 의존하는 테이블과 인덱스를 만듭니다.
- shadow document 함수들은 파일 기반 planning workspace에서 읽은 내용을 DB에 mirror합니다.
- metadata 함수들은 schema/mode/root/timestamp처럼 DB snapshot 전체를 설명하는 값을 기록합니다.
- shadow load는 이전 schema나 빈 DB에서도 안전하게 빈 map으로 돌아가는 호환성 경계를 제공합니다.
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
학습 주석:
authority store가 필요로 하는 전체 schema를 idempotent하게 보장합니다.

이 함수는 `CREATE TABLE IF NOT EXISTS`와 `CREATE INDEX IF NOT EXISTS`만 사용하므로, 새 DB 파일뿐 아니라
이미 존재하는 DB 파일에 여러 번 호출해도 같은 결과를 유지합니다. adapter의 다른 함수들은 자기 작업에
필요한 테이블이 이미 있다고 가정하므로, DB connection을 만든 직후 이 schema 초기화가 먼저 실행되어야
합니다.

테이블 묶음별 의미는 다음과 같습니다.
- `authority_metadata`: schema version, 저장 mode, canonical repo root, 최근 갱신 시각 같은 store 전체 정보입니다.
- `shadow_documents`: 파일시스템 workspace에서 읽은 planning 파일의 mirror입니다.
- `staged_drafts` / `staged_draft_files`: repo-scoped draft staging 영역입니다.
- `active_documents`: commit된 planning workspace snapshot입니다.
- `planning_direction_*`: direction authority 문서와 방향별 JSON 원문입니다.
- `planning_tasks` / `planning_task_edges` / `planning_queue_projection`: task authority와 queue projection입니다.
- `runtime_*`: app-server/parallel runtime에서 쓰는 lease, session, queue, event projection입니다.

schema가 한 함수에 모여 있는 이유는 projection 모듈들이 서로 다른 테이블을 만져도 migration 기준은
하나여야 하기 때문입니다. 분산된 `CREATE TABLE`은 버전 추적과 테스트 초기화를 어렵게 만듭니다.
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

            CREATE TABLE IF NOT EXISTS runtime_distributor_queue (
                queue_item_id TEXT PRIMARY KEY,
                session_key TEXT NOT NULL,
                queue_state TEXT NOT NULL,
                enqueued_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
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
            "#,
        )
        .context("failed to initialize authority-store schema")?;
    Ok(())
}

/*
학습 주석:
파일시스템에서 읽은 planning 문서들을 `shadow_documents` 테이블에 전체 교체 방식으로 저장합니다.

shadow store는 "현재 repo의 planning 파일들이 DB 관점에서 어떻게 보이는가"를 나타내는 mirror입니다.
증분 patch를 시도하지 않고 먼저 테이블을 비우는 이유는 파일 삭제를 정확하게 반영하기 위해서입니다.
입력 `documents`에 없는 파일이 DB에 남아 있으면 이후 authority snapshot 로더가 삭제된 파일을 여전히
존재하는 것처럼 볼 수 있습니다.

이 함수는 transaction 안에서 shadow rows와 metadata timestamp를 함께 갱신합니다. 따라서 mirror 내용은
항상 `last_synced_at`과 같은 commit 시점의 상태로 해석할 수 있습니다.
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
학습 주석:
authority DB 전체를 설명하는 공통 metadata를 upsert합니다.

`timestamp_key`를 인자로 받는 이유는 호출 맥락마다 "무엇이 갱신되었는지"가 다르기 때문입니다.
shadow sync는 `last_synced_at`, draft staging은 `last_draft_updated_at`처럼 다른 key를 넘깁니다.
하지만 schema version, mode, canonical repo root, workspace root는 모든 갱신에서 함께 확인되어야 하는
store identity 값입니다.
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
학습 주석:
metadata key/value 하나를 insert-or-update합니다.

metadata는 단순한 문자열 map이지만, 여러 저장 흐름에서 같은 key를 반복 갱신합니다. `ON CONFLICT(key)`
절을 사용하면 caller가 "처음 쓰는 값인지, 기존 값을 갱신하는지"를 구분하지 않아도 됩니다. 이 함수는
작은 helper지만 schema version, mode, ledger version처럼 store 해석에 필요한 기준값을 쓰는 공통
입구입니다.
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
학습 주석:
shadow store에 mirror된 planning 문서들을 deterministic map으로 읽어옵니다.

`table_exists`를 먼저 확인하는 것은 이전 버전 DB나 아직 schema 초기화가 끝나지 않은 테스트 DB를
읽을 때의 방어 장치입니다. shadow table이 없으면 "mirror가 없다"는 의미로 빈 map을 반환합니다.

조회는 `ORDER BY relative_path`를 사용하고 결과 컨테이너도 `BTreeMap`입니다. 둘 다 출력 순서를
안정화하기 위한 선택입니다. planning 문서는 사람이 diff로 검토하는 일이 많기 때문에, DB가 반환하는
행 순서가 실행마다 달라지는 상황을 피해야 합니다.
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
학습 주석:
commit된 active planning 문서 snapshot을 모두 읽어옵니다.

`shadow_documents`가 파일시스템 mirror라면, `active_documents`는 authority DB가 현재 활성 workspace로
간주하는 확정본입니다. repo-scoped flow에서는 draft를 staging한 뒤 commit하면 active snapshot이
바뀌고, 이후 TUI나 service는 이 active snapshot을 기준으로 결과 출력 파일과 authority 문서를 봅니다.

정렬과 `BTreeMap` 사용은 shadow load와 같은 이유입니다. 같은 DB 상태는 항상 같은 iteration 순서를
만들어야 diff, 테스트, TUI rendering이 안정됩니다.
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
학습 주석:
active authority document map을 그대로 노출하는 얇은 별칭입니다.

이 함수는 현재 `load_active_documents`에 바로 위임하지만 이름을 따로 둡니다. 호출자 입장에서는
"active workspace의 일반 문서 map"을 읽는지, "authority 문서 저장소"를 읽는지 의도가 다를 수 있기
때문입니다. 같은 구현을 공유하더라도 adapter 내부 API 이름으로 경계 의미를 남깁니다.
*/
pub(super) fn load_active_authority_documents(
    connection: &Connection,
) -> Result<BTreeMap<String, String>> {
    load_active_documents(connection)
}

/*
학습 주석:
active documents에서 `PlanningWorkspaceLoadRecord`를 구성합니다.

workspace port가 필요로 하는 load record는 전체 파일 map이 아니라 현재 결과 출력 markdown입니다.
그래서 active snapshot 전체를 읽은 뒤 `RESULT_OUTPUT_FILE_PATH`만 골라 optional field에 넣습니다.
파일이 없으면 `None`이 되며, 이는 아직 결과 문서가 생성되지 않은 정상 상태를 뜻합니다.
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
학습 주석:
direction authority snapshot을 DB에서 복원합니다.

snapshot은 direction catalog 본문과 planning revision metadata의 조합입니다. direction catalog가 없으면
아직 direction authority가 초기화되지 않은 상태이므로 `Ok(None)`을 반환합니다. catalog는 있는데
`planning_revision` metadata가 없으면 이전 schema나 초기 상태로 보고 0을 사용합니다.
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
학습 주석:
direction authority 테이블들을 domain의 `DirectionCatalogDocument`로 다시 조립합니다.

저장 구조는 config row 하나와 direction row 여러 개로 나뉩니다. config row에는 catalog 전체에 적용되는
format version과 queue idle 설정이 들어 있고, `planning_directions`에는 각 direction의 JSON 원문이
순서와 함께 들어 있습니다. 이 함수는 그 둘을 합쳐 application 계층이 기대하는 catalog 문서 형태로
복원합니다.

`direction_authority_exists`를 먼저 확인하는 이유는 "authority가 없음"과 "authority는 있는데 direction이
0개"를 구분하기 위해서입니다. 현재 schema에서는 config row 존재가 direction authority snapshot의
존재 신호입니다.
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
    학습 주석:
    row에는 `direction_id` column과 `content_json`이 함께 있지만, 복원 결과는 JSON 안의 domain 구조를
    신뢰합니다. `direction_id`는 오류 메시지에 붙여 어느 행의 JSON decode가 실패했는지 보여주는
    진단 정보로 사용됩니다.
    */
    for row in rows {
        let (direction_id, content_json) =
            row.context("failed to decode planning direction row")?;
        directions.push(serde_json::from_str(&content_json).with_context(|| {
            format!("failed to deserialize planning direction row `{direction_id}`")
        })?);
    }
    /*
    학습 주석:
    queue idle 설정은 DB column 두 개로 저장되어 있지만 domain 타입은 structured enum/record입니다.
    작은 JSON value로 다시 감싼 뒤 serde가 domain 타입으로 decode하게 해서, 문자열 policy 해석 규칙을
    이 store 함수 안에 중복 구현하지 않습니다.
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
학습 주석:
active snapshot에서 특정 문서 하나만 읽습니다.

대부분의 load 함수는 전체 map을 복원하지만, 일부 호출자는 상대 경로 하나의 본문만 필요합니다. 이
helper는 `OptionalExtension`을 사용해 row 없음과 SQL 오류를 분리합니다. row가 없으면 정상적으로
`Ok(None)`이고, DB 조회 자체가 실패하면 context가 붙은 error가 됩니다.
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

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn load_task_authority_snapshot_from_connection(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    connection: &Connection,
) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let Some(task_authority) = load_task_authority_from_connection(connection)? else {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(None);
    };
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let queue_projection =
        load_queue_projection_from_connection(connection)?.unwrap_or_else(empty_queue_projection);
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let planning_revision =
        read_metadata_i64_connection(connection, "planning_revision")?.unwrap_or(0);
    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(Some(PlanningTaskAuthoritySnapshot {
        planning_revision,
        task_authority,
        queue_projection,
    }))
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn load_task_authority_from_connection(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    connection: &Connection,
) -> Result<Option<TaskAuthorityDocument>> {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !task_authority_exists(connection)? {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(None);
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let version = read_metadata_i64_connection(connection, TASK_LEDGER_VERSION_METADATA_KEY)?
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .unwrap_or(i64::from(PLANNING_FORMAT_VERSION)) as u32;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut statement = connection
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .prepare(
            "SELECT task_id, content_json
             FROM planning_tasks
             ORDER BY task_order ASC, task_id ASC",
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to read planning task rows")?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rows = statement
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_map([], |row| {
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to iterate planning task rows")?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut tasks = Vec::new();
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for row in rows {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let (task_id, content_json) = row.context("failed to decode planning task row")?;
        tasks.push(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            serde_json::from_str(&content_json)
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .with_context(|| format!("failed to deserialize planning task row `{task_id}`"))?,
        );
    }

    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(Some(TaskAuthorityDocument { version, tasks }))
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn load_queue_projection_from_connection(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    connection: &Connection,
) -> Result<Option<PriorityQueueProjection>> {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !task_authority_exists(connection)? {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(None);
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut active_tasks = Vec::new();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut proposed_tasks = Vec::new();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut skipped_tasks = Vec::new();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut statement = connection
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .prepare(
            "SELECT bucket, task_id, content_json
             FROM planning_queue_projection
             ORDER BY bucket ASC, rank ASC, task_id ASC",
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to read planning queue projection")?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rows = statement
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_map([], |row| {
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to iterate planning queue projection")?;
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for row in rows {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let (bucket, task_id, content_json) =
            row.context("failed to decode planning queue projection row")?;
        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match bucket.as_str() {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            "active" => {
                active_tasks.push(
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    serde_json::from_str::<PriorityQueueTask>(&content_json).with_context(
                        // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
                        || format!("failed to deserialize active queue projection `{task_id}`"),
                    )?,
                );
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            "proposed" => {
                proposed_tasks.push(
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    serde_json::from_str::<PriorityQueueTask>(&content_json).with_context(
                        // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
                        || format!("failed to deserialize proposed queue projection `{task_id}`"),
                    )?,
                );
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            "skipped" => {
                skipped_tasks.push(
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    serde_json::from_str::<PriorityQueueSkippedTask>(&content_json).with_context(
                        // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
                        || format!("failed to deserialize skipped queue projection `{task_id}`"),
                    )?,
                );
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            _ => {}
        }
    }

    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(Some(PriorityQueueProjection {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        next_task: active_tasks.first().cloned(),
        active_tasks,
        proposed_tasks,
        skipped_tasks,
    }))
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn task_authority_exists(connection: &Connection) -> Result<bool> {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if read_metadata_string_connection(connection, TASK_LEDGER_VERSION_METADATA_KEY)?.is_some() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(true);
    }
    connection
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_row("SELECT 1 FROM planning_tasks LIMIT 1", [], |_| Ok(()))
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .optional()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to inspect planning task authority")
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|value| value.is_some())
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn empty_queue_projection() -> PriorityQueueProjection {
    PriorityQueueProjection {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        next_task: None,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        active_tasks: Vec::new(),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        proposed_tasks: Vec::new(),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        skipped_tasks: Vec::new(),
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn reconcile_task_authority_with_directions(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    transaction: &rusqlite::Transaction<'_>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    directions: Option<&DirectionCatalogDocument>,
) -> Result<()> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let Some(mut task_authority) = load_task_authority_from_connection(transaction)? else {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(());
    };
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let direction_ids = match directions {
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        Some(directions) => direction_ids(directions),
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        None => BTreeSet::new(),
    };
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !prune_task_authority_to_direction_ids(&mut task_authority, &direction_ids) {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(());
    }
    replace_task_authority_tables(transaction, &task_authority, &empty_queue_projection())?;
    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(())
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn direction_ids(directions: &DirectionCatalogDocument) -> BTreeSet<String> {
    directions
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .directions
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|direction| direction.id.trim().to_string())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect()
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn replace_direction_authority_tables(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    transaction: &rusqlite::Transaction<'_>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    directions: &DirectionCatalogDocument,
) -> Result<()> {
    clear_direction_authority_rows(transaction)?;
    transaction
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
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
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to persist planning direction config")?;
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for (index, direction) in directions.directions.iter().enumerate() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let direction_id = direction.id.trim();
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
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
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    serde_json::to_string(direction)
                        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                        .context("failed to serialize planning direction row")?,
                ],
            )
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .with_context(|| format!("failed to persist planning direction `{direction_id}`"))?;
    }
    upsert_metadata(
        transaction,
        "direction_authority_version",
        &directions.version.to_string(),
    )?;
    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(())
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn clear_direction_authority_tables(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    transaction: &rusqlite::Transaction<'_>,
) -> Result<()> {
    clear_direction_authority_rows(transaction)?;
    upsert_metadata(transaction, "direction_authority_version", "0")?;
    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(())
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn clear_direction_authority_rows(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .execute("DELETE FROM planning_directions", [])
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to clear planning direction rows")?;
    transaction
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .execute("DELETE FROM planning_direction_config", [])
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to clear planning direction config")?;
    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(())
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn direction_authority_exists(connection: &Connection) -> Result<bool> {
    connection
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM planning_directions LIMIT 1)",
            [],
            // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
            |row| row.get::<_, i64>(0),
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|exists| exists != 0)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to inspect planning direction rows")
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn prune_task_authority_to_direction_ids(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    task_authority: &mut TaskAuthorityDocument,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    direction_ids: &BTreeSet<String>,
) -> bool {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut removed_task_ids = BTreeSet::new();
    task_authority.tasks.retain(|task| {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let keep = direction_ids.contains(task.direction_id.trim());
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !keep {
            removed_task_ids.insert(task.id.trim().to_string());
        }
        keep
    });
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if removed_task_ids.is_empty() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return false;
    }
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for task in &mut task_authority.tasks {
        task.depends_on
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .retain(|task_id| !removed_task_ids.contains(task_id.trim()));
        task.blocked_by
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .retain(|task_id| !removed_task_ids.contains(task_id.trim()));
    }
    true
}

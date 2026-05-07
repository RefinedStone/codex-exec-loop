/*
domain의 `TaskAuthorityDocument`와 `PriorityQueueProjection`을 SQLite 테이블 행으로 펼쳐 저장하는
adapter 내부 매핑 계층이다.

task authority는 원래 하나의 문서처럼 다룰 수 있는 데이터다. 하지만 TUI와 app-server 흐름에서는
작업 목록, 의존 관계, queue rank, skipped 이유를 따로 조회하거나 projection해야 한다. 그래서 이
파일은 문서형 domain 값을 다음 테이블들로 나눈다.

- `planning_tasks`: task의 현재 정의와 우선순위 계산 결과를 task_id 단위로 저장한다.
- `planning_task_edges`: depends_on / blocked_by 같은 task 간 관계를 edge 행으로 저장한다.
- `planning_queue_projection`: active / proposed / skipped queue에 보이는 현재 정렬 결과를 저장한다.
- `authority_metadata`: task ledger 버전처럼 전체 문서에 붙는 metadata를 저장한다.

이 파일은 domain 규칙을 새로 계산하지 않는다. 우선순위나 skipped 여부는 이미 domain/application
쪽에서 계산된 projection으로 들어오며, 이 모듈은 그 결과를 SQLite 조회에 맞게 정규화하고 transaction
안에서 원자적으로 반영한다.
*/
use std::collections::BTreeSet;

use anyhow::{Context, Result};
use rusqlite::params;

use crate::domain::planning::{
    PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskAuthorityDocument,
};

use super::{TASK_LEDGER_VERSION_METADATA_KEY, upsert_metadata};

/*
task authority 관련 테이블을 새 snapshot으로 교체한다.

이 함수는 store 계층에서 이미 열린 transaction을 받아 실행된다. 그래서 이 안의 여러 DELETE/INSERT가
하나의 commit 단위로 묶인다. 목표는 "task authority 문서와 queue projection이 서로 같은 시점의
상태를 나타내게 하는 것"이다. task 정의만 바뀌고 queue projection이 예전 상태로 남으면 TUI가 상충되는
정보를 보여줄 수 있으므로, 두 데이터를 한 함수에서 같이 갱신한다.

갱신 순서도 의도적이다.
1. queue projection은 전체 snapshot 성격이므로 먼저 비운다.
2. task 본문과 edge는 stale 행 삭제 후 upsert한다.
3. active/proposed/skipped projection을 다시 삽입한다.
4. ledger version metadata를 갱신해 DB가 어떤 task authority 버전을 반영하는지 남긴다.
*/
pub(super) fn replace_task_authority_tables(
    transaction: &rusqlite::Transaction<'_>,
    task_authority: &TaskAuthorityDocument,
    queue_projection: &PriorityQueueProjection,
) -> Result<()> {
    /*
    Projection rows are cleared before task rows are synchronized because queue rank is a
    derived snapshot, not independent state. If task upsert later fails, the outer
    transaction rolls this clear back with the rest of the authority update.
    */
    clear_queue_projection_rows(transaction)?;
    sync_task_authority_rows(transaction, task_authority)?;
    insert_queue_projection_tasks(transaction, "active", &queue_projection.active_tasks)?;
    insert_queue_projection_tasks(transaction, "proposed", &queue_projection.proposed_tasks)?;
    insert_queue_projection_skipped(transaction, &queue_projection.skipped_tasks)?;
    upsert_metadata(
        transaction,
        TASK_LEDGER_VERSION_METADATA_KEY,
        &task_authority.version.to_string(),
    )?;
    Ok(())
}

/*
`TaskAuthorityDocument.tasks` 목록을 `planning_tasks`와 `planning_task_edges`에 동기화한다.

이 함수가 단순히 모든 task를 삭제하고 다시 넣지 않는 이유는 `planning_tasks`가 task_id 기준 upsert
구조이기 때문이다. 먼저 원하는 task_id 집합을 만들고, 문서에서 사라진 stale task만 골라 삭제한다.
그 뒤 현재 문서에 남아 있는 task는 순서(`task_order`)와 JSON 원문까지 upsert한다.

edge는 task 본문과 다르게 매번 해당 task_id의 관계 행을 지우고 다시 삽입한다. depends_on이나
blocked_by 목록은 순서가 있는 배열이고, 개별 edge의 변경을 diff하는 이득이 작다. 현재 task의 관계를
"문서에 적힌 그대로" 다시 쓰는 편이 더 명확하다.
*/
fn sync_task_authority_rows(
    transaction: &rusqlite::Transaction<'_>,
    task_authority: &TaskAuthorityDocument,
) -> Result<()> {
    /*
    desired_task_ids is computed from the incoming document, not from the DB. This makes
    document absence authoritative: anything left in SQLite but omitted by the latest
    task authority must be treated as stale adapter state and removed.
    */
    let desired_task_ids = task_authority
        .tasks
        .iter()
        .map(|task| task.id.trim().to_string())
        .collect::<BTreeSet<_>>();
    delete_stale_task_rows(transaction, &desired_task_ids)?;
    for (index, task) in task_authority.tasks.iter().enumerate() {
        upsert_task_row(transaction, index, task)?;
        let task_id = task.id.trim();
        /*
        Edges are task-local projections of arrays in content_json. Clearing only the
        current task's edges preserves other tasks while guaranteeing that removed or
        reordered dependency entries do not survive from the previous snapshot.
        */
        transaction
            .execute(
                "DELETE FROM planning_task_edges WHERE task_id = ?1",
                params![task_id],
            )
            .with_context(|| format!("failed to clear planning task edges for `{task_id}`"))?;
        insert_task_edges(transaction, task_id, "depends_on", &task.depends_on)?;
        insert_task_edges(transaction, task_id, "blocked_by", &task.blocked_by)?;
    }
    Ok(())
}

/*
현재 문서에 더 이상 존재하지 않는 task 행과 그 edge를 삭제한다.

`desired_task_ids`는 domain 문서에서 온 task_id 집합이고, DB의 `planning_tasks`는 이전 snapshot의
잔여 행을 포함할 수 있다. 예를 들어 사용자가 task를 완료/삭제해서 authority 문서에서 빠졌다면 그
task는 DB 조회 결과에서도 사라져야 한다. 이 helper는 DB에만 남아 있는 stale task를 찾아 edge부터
지운 뒤 task 본문을 지운다.

edge를 먼저 지우는 순서는 관계형 저장소에서 흔한 부모-자식 정리 순서다. schema의 제약 조건이
바뀌더라도 dangling edge가 남지 않도록 명시적으로 자식 관계를 먼저 제거한다.
*/
fn delete_stale_task_rows(
    transaction: &rusqlite::Transaction<'_>,
    desired_task_ids: &BTreeSet<String>,
) -> Result<()> {
    let mut statement = transaction
        .prepare("SELECT task_id FROM planning_tasks")
        .context("failed to read existing planning task ids")?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .context("failed to iterate existing planning task ids")?;
    let mut stale_task_ids = Vec::new();
    for row in rows {
        let task_id = row.context("failed to decode existing planning task id")?;
        if !desired_task_ids.contains(task_id.trim()) {
            stale_task_ids.push(task_id);
        }
    }
    /*
    `statement`가 살아 있으면 rusqlite borrow가 transaction 사용을 계속 붙잡을 수 있다.
    아래에서 같은 transaction으로 DELETE를 실행해야 하므로, 조회 statement를 명시적으로 drop해서
    읽기 cursor의 수명을 끝낸다.
    */
    drop(statement);

    for task_id in stale_task_ids {
        /*
        Stale IDs are trimmed at deletion time to match the normalized IDs used during
        insert/upsert. This keeps a previously sloppy row from surviving just because
        the new document normalized whitespace around task ids.
        */
        transaction
            .execute(
                "DELETE FROM planning_task_edges WHERE task_id = ?1",
                params![task_id.trim()],
            )
            .with_context(|| format!("failed to clear stale planning task edges `{task_id}`"))?;
        transaction
            .execute(
                "DELETE FROM planning_tasks WHERE task_id = ?1",
                params![task_id.trim()],
            )
            .with_context(|| format!("failed to delete stale planning task `{task_id}`"))?;
    }
    Ok(())
}

/*
domain의 `TaskDefinition` 하나를 `planning_tasks` 행 하나로 저장한다.

테이블에는 조회와 정렬에 자주 쓰는 필드를 column으로 풀어 넣고, 동시에 `content_json`에는 task 전체를
직렬화해서 보존한다. 이 이중 저장은 의도된 tradeoff다. SQL에서는 title/status/priority 같은 필드를
직접 필터링하고 정렬할 수 있고, 필요하면 JSON 원문으로 domain 객체 전체를 복원할 수 있다.

`task_order`는 문서 안의 배열 순서를 보존한다. `combined_priority`는 DB에서 매번 다시 계산하지
않도록 domain method 결과를 snapshot으로 저장한다. `ON CONFLICT(task_id)`는 같은 task가 다음
snapshot에서 수정되었을 때 같은 row를 갱신하게 한다.
*/
fn upsert_task_row(
    transaction: &rusqlite::Transaction<'_>,
    index: usize,
    task: &crate::domain::planning::TaskDefinition,
) -> Result<()> {
    let task_id = task.id.trim();
    /*
    The searchable columns are intentionally normalized views of the domain task. The
    JSON payload remains the full restore source, while these columns support queue,
    admin, and diagnostic queries without forcing callers to deserialize every row.
    */
    transaction
        .execute(
            "INSERT INTO planning_tasks
             (task_id, task_order, direction_id, title, status, base_priority,
              dynamic_priority_delta, combined_priority, updated_at, source_turn_id,
              origin_session_kind, thread_id, turn_id, parent_thread_id, parent_turn_id,
              content_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(task_id) DO UPDATE SET
                 task_order = excluded.task_order,
                 direction_id = excluded.direction_id,
                 title = excluded.title,
                 status = excluded.status,
                 base_priority = excluded.base_priority,
                 dynamic_priority_delta = excluded.dynamic_priority_delta,
                 combined_priority = excluded.combined_priority,
                 updated_at = excluded.updated_at,
                 source_turn_id = excluded.source_turn_id,
                 origin_session_kind = excluded.origin_session_kind,
                 thread_id = excluded.thread_id,
                 turn_id = excluded.turn_id,
                 parent_thread_id = excluded.parent_thread_id,
                 parent_turn_id = excluded.parent_turn_id,
                 content_json = excluded.content_json",
            params![
                task_id,
                index as i64,
                task.direction_id.trim(),
                task.title.trim(),
                task.status.label(),
                task.base_priority,
                task.dynamic_priority_delta,
                task.combined_priority(),
                task.updated_at.as_str(),
                task.source_turn_id.as_deref(),
                task.provenance
                    .origin_session_kind
                    .map(|origin_session_kind| origin_session_kind.label()),
                task.provenance.thread_id.as_deref(),
                task.provenance.turn_id.as_deref(),
                task.provenance.parent_thread_id.as_deref(),
                task.provenance.parent_turn_id.as_deref(),
                serde_json::to_string(task).context("failed to serialize planning task row")?,
            ],
        )
        .with_context(|| format!("failed to persist planning task `{task_id}`"))?;
    Ok(())
}

/*
task의 관계 배열을 `planning_task_edges` 행들로 저장한다.

`edge_kind`는 현재 `"depends_on"` 또는 `"blocked_by"`로 들어온다. 같은 테이블에 두 종류를 함께
넣으면 task 관계 조회 로직이 한 테이블만 보면 되고, edge_kind column으로 의미를 구분할 수 있다.
`edge_order`는 domain 문서의 배열 순서를 보존한다. 순서 자체가 알고리즘에 필요하지 않더라도, 사람이
보는 출력과 round-trip 복원에서 원래 문서의 의도를 덜 잃게 된다.
*/
fn insert_task_edges(
    transaction: &rusqlite::Transaction<'_>,
    task_id: &str,
    edge_kind: &str,
    target_task_ids: &[String],
) -> Result<()> {
    for (index, target_task_id) in target_task_ids.iter().enumerate() {
        /*
        Edge rows use the already-normalized parent task_id from the caller and trim the
        target id here. Validation owns whether the target exists; this adapter only
        persists the document's relationship list in a query-friendly shape.
        */
        transaction
            .execute(
                "INSERT INTO planning_task_edges (task_id, edge_kind, target_task_id, edge_order)
                 VALUES (?1, ?2, ?3, ?4)",
                params![task_id, edge_kind, target_task_id.trim(), index as i64],
            )
            .with_context(|| {
                format!("failed to persist planning task edge `{task_id}:{edge_kind}`")
            })?;
    }
    Ok(())
}

/*
active/proposed queue에 보이는 task projection을 저장한다.

여기서 저장하는 값은 task 정의 자체가 아니라 "현재 queue에서 이 task가 어떤 rank로 보이는가"다.
그래서 task 본문 테이블과 별도로 `planning_queue_projection`에 들어간다. bucket은 caller가
`"active"` 또는 `"proposed"`를 넘기며, item_kind는 일반 task row임을 나타내기 위해 `'task'`로
고정한다.

`content_json`에는 `PriorityQueueTask` 전체를 저장한다. projection이 나중에 필드를 더 갖게 되더라도
DB schema를 바로 확장하지 않고 JSON 원문에서 정보를 회수할 수 있게 하기 위한 보존 장치다.
*/
fn insert_queue_projection_tasks(
    transaction: &rusqlite::Transaction<'_>,
    bucket: &str,
    tasks: &[PriorityQueueTask],
) -> Result<()> {
    for task in tasks {
        /*
        rank is trusted from PriorityQueueProjection instead of recomputed from task
        priority columns. Runtime queue policy may include more than raw priority, so
        the persisted projection must mirror the already-decided ordering.
        */
        transaction
            .execute(
                "INSERT INTO planning_queue_projection
                 (bucket, rank, task_id, item_kind, content_json)
                 VALUES (?1, ?2, ?3, 'task', ?4)",
                params![
                    bucket,
                    task.rank as i64,
                    task.task_id.trim(),
                    serde_json::to_string(task)
                        .context("failed to serialize planning queue-task projection")?,
                ],
            )
            .with_context(|| {
                format!(
                    "failed to persist planning queue projection `{bucket}:{}`",
                    task.task_id
                )
            })?;
    }
    Ok(())
}

/*
queue 계산에서 제외된 task들을 skipped bucket으로 저장한다.

skipped task는 active/proposed task와 다르게 projection 안에 이미 rank가 있는 구조가 아니라, skipped
목록의 순서 자체가 표시 순서다. 그래서 enumerate index에 1을 더해 rank로 저장한다. item_kind를
`'skipped'`로 넣어 같은 table 안에서도 일반 queue-task와 구분한다.
*/
fn insert_queue_projection_skipped(
    transaction: &rusqlite::Transaction<'_>,
    skipped_tasks: &[PriorityQueueSkippedTask],
) -> Result<()> {
    for (index, task) in skipped_tasks.iter().enumerate() {
        /*
        Skipped entries use display order as rank because skipped reasons are explanation
        rows, not executable queue candidates. That keeps them stable in admin/TUI output
        without implying they compete with active/proposed priorities.
        */
        transaction
            .execute(
                "INSERT INTO planning_queue_projection
                 (bucket, rank, task_id, item_kind, content_json)
                 VALUES ('skipped', ?1, ?2, 'skipped', ?3)",
                params![
                    index as i64 + 1,
                    task.task_id.trim(),
                    serde_json::to_string(task)
                        .context("failed to serialize skipped planning queue projection")?,
                ],
            )
            .with_context(|| {
                format!(
                    "failed to persist skipped planning queue projection `{}`",
                    task.task_id
                )
            })?;
    }
    Ok(())
}

/*
task authority 관련 데이터와 ledger version metadata를 모두 비운다.

이 함수는 "현재 DB가 task authority snapshot을 갖고 있다"는 상태 자체를 제거할 때 쓰는 공개 helper다.
row 데이터만 지우고 metadata가 남으면 상위 로직이 DB에 아직 특정 ledger version이 반영되어 있다고
오해할 수 있다. 그래서 `clear_task_authority_rows` 다음에 `authority_metadata`의 version key도
명시적으로 삭제한다.
*/
pub(super) fn clear_task_authority_tables(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    clear_task_authority_rows(transaction)?;
    /*
    Removing the ledger version is what makes store::task_authority_exists return false
    when no task rows remain. Without this metadata delete, an intentionally cleared
    authority could be misread as an initialized but empty ledger snapshot.
    */
    transaction
        .execute(
            "DELETE FROM authority_metadata WHERE key = ?1",
            params![TASK_LEDGER_VERSION_METADATA_KEY],
        )
        .context("failed to clear planning task authority metadata")?;
    Ok(())
}

/*
task authority의 행 데이터만 비운다.

삭제 순서는 projection -> edge -> task다. queue projection은 task row를 참조하는 조회용 snapshot이고,
edge는 task 사이의 관계 행이며, planning_tasks는 중심 본문이다. 중심 본문을 먼저 지우면 관계 행이나
projection이 잠깐이라도 고아 데이터처럼 남을 수 있으므로 주변 projection/edge를 먼저 비운다.
*/
fn clear_task_authority_rows(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    clear_queue_projection_rows(transaction)?;
    transaction
        .execute("DELETE FROM planning_task_edges", [])
        .context("failed to clear planning task edges")?;
    transaction
        .execute("DELETE FROM planning_tasks", [])
        .context("failed to clear planning task rows")?;
    Ok(())
}

/*
queue projection snapshot만 삭제한다.

`replace_task_authority_tables`는 task 본문을 upsert하기 전에 projection을 항상 비운다. projection은
rank가 있는 현재 계산 결과라서 부분 upsert보다 전체 재생성이 더 안전하다. stale projection row가
하나라도 남으면 TUI가 이미 사라진 task를 queue에 보여줄 수 있다.
*/
fn clear_queue_projection_rows(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction
        .execute("DELETE FROM planning_queue_projection", [])
        .context("failed to clear planning queue projection")?;
    Ok(())
}

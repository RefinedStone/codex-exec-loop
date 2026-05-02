use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;

/*
 * `PlanningWorkspaceLoadRecord`를 repo-scoped authority DB의 `active_documents` 테이블에 반영한다.
 * application 계층은 "현재 planning workspace의 canonical 파일 묶음"만 넘기고, adapter는 그 묶음을
 * SQLite row 단위의 active document로 해체한다. 지금은 result output 한 파일만 있지만, 이 함수가
 * record 전체를 받는 이유는 이후 canonical planning 파일이 늘어도 호출 계약을 유지하기 위해서다.
 */
pub(super) fn apply_active_workspace_record(
    // caller가 이미 열어 둔 transaction이다. active 문서 반영과 revision/event 기록을 한 원자 단위로 묶는다.
    transaction: &rusqlite::Transaction<'_>,
    // workspace port가 다루는 canonical planning 파일 snapshot이다.
    record: &PlanningWorkspaceLoadRecord,
) -> Result<bool> {
    // 하나라도 row가 바뀌었는지 누적한다. 상위 commit 로직은 이 값으로 불필요한 revision bump를 피할 수 있다.
    let mut changed = false;
    changed |= set_active_document(
        transaction,
        RESULT_OUTPUT_FILE_PATH,
        record.result_output_markdown.as_deref(),
    )?;
    Ok(changed)
}

/*
 * active document 한 row를 upsert 또는 delete한다.
 * `body: Option<&str>` 계약은 port 계층의 `replace_*_file`과 맞물립니다. `Some`은 전체 본문 저장,
 * `None`은 해당 planning 파일이 더 이상 active workspace에 없어야 함을 의미한다.
 */
pub(super) fn set_active_document(
    // 읽기 비교와 쓰기/삭제를 같은 transaction 안에서 처리하기 위한 DB transaction이다.
    transaction: &rusqlite::Transaction<'_>,
    // active workspace 기준 상대 경로다. 상위 adapter가 정규화를 끝낸 값이어야 한다.
    relative_path: &str,
    // 저장할 본문이 있으면 `Some`, row를 지워야 하면 `None`이다.
    body: Option<&str>,
) -> Result<bool> {
    // 먼저 기존 본문을 읽어 no-op 갱신을 걸러낸다. 이 비교가 revision churn을 줄이는 핵심이다.
    let existing = transaction
        .query_row(
            "SELECT content FROM active_documents WHERE relative_path = ?1",
            params![relative_path],
            |row| row.get::<_, String>(0),
        )
        // row가 없어도 오류가 아니라 `None`으로 변환해 "파일 부재"와 DB 오류를 구분한다.
        .optional()
        .with_context(|| format!("failed to read active document `{relative_path}`"))?;
    // DB의 기존 상태와 요청 상태가 같으면 쓰지 않고 `false`를 돌려 상위 commit이 변경 없음으로 볼 수 있게 한다.
    if existing.as_deref() == body {
        return Ok(false);
    }

    // 여기서부터는 실제 변경이다. `Some`은 upsert, `None`은 delete로 명확히 갈라진다.
    match body {
        Some(body) => {
            transaction
                // 같은 relative_path가 있으면 content만 교체해 row identity를 경로 기준으로 유지한다.
                .execute(
                    "INSERT INTO active_documents (relative_path, content) VALUES (?1, ?2)
                     ON CONFLICT(relative_path) DO UPDATE SET content = excluded.content",
                    params![relative_path, body],
                )
                .with_context(|| format!("failed to store active document `{relative_path}`"))?;
        }
        None => {
            transaction
                // 요청 snapshot에서 사라진 파일은 active table에서도 제거해 filesystem view와 DB view를 맞춘다.
                .execute(
                    "DELETE FROM active_documents WHERE relative_path = ?1",
                    params![relative_path],
                )
                .with_context(|| format!("failed to delete active document `{relative_path}`"))?;
        }
    }

    // 이 함수가 `true`를 반환하면 실제 row가 바뀌었고, 상위 호출자는 revision/event 갱신을 고려한다.
    Ok(true)
}

/*
 * active document 하나 또는 디렉터리 하위 문서 전체를 제거한다.
 * workspace port의 `remove_active_planning_entry`는 파일과 디렉터리 제거를 모두 표현하므로,
 * 이 SQL은 정확히 같은 path와 `path/` prefix를 함께 지워 디렉터리 삭제 의미를 table row들에 투영한다.
 */
pub(super) fn remove_active_documents(
    // 삭제와 상위 revision/event 기록을 함께 묶는 transaction이다.
    transaction: &rusqlite::Transaction<'_>,
    // 제거할 파일 또는 디렉터리의 active workspace 상대 경로다.
    relative_path: &str,
) -> Result<bool> {
    // SQLite가 보고한 삭제 row 수를 사용해 실제 변경 여부를 상위 계층에 전달한다.
    let deleted_rows = transaction
        .execute(
            "DELETE FROM active_documents
             WHERE relative_path = ?1 OR relative_path LIKE ?2",
            params![relative_path, format!("{relative_path}/%")],
        )
        .with_context(|| format!("failed to remove active authority entry `{relative_path}`"))?;
    // 삭제 대상이 없으면 성공이지만 변경은 없으므로 `false`를 반환한다.
    Ok(deleted_rows > 0)
}

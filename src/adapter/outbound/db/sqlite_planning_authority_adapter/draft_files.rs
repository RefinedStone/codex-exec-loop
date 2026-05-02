/*
이 모듈은 planning authority SQLite DB 안에서 "초안(draft)" 파일 묶음을 다루는
저수준 저장소 로직이다.

상위의 `RepoScopedPlanningWorkspacePort`는 파일시스템 workspace처럼 보이는 API를 제공하지만,
git으로 묶인 workspace에서는 실제 초안 파일을 디스크의 `.codex/planning/drafts/...`에
직접 쓰지 않고 authority DB의 `staged_drafts` / `staged_draft_files` 테이블에 저장한다.
그래서 이 파일의 함수들은 경계 변환을 담당한다.

- 입력 쪽 이름은 여전히 `workspace_dir`, `draft_name`, `active_path`, `body`다.
- 내부에서는 `workspace_dir`로 authority DB 위치를 찾는다.
- DB에는 초안 이름과 활성 planning 파일 경로별 본문을 저장한다.
- 반환값에는 호출자가 기존 파일 기반 흐름과 같은 형태로 볼 수 있도록 표시용 draft 경로를
  다시 만들어 담는다.

이 구조 덕분에 TUI와 application 계층은 "초안을 staging한다"는 유스케이스만 알면 되고,
초안이 실제 파일인지 SQLite 행인지는 outbound adapter 내부의 구현 세부사항으로 남는다.
*/
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadFileRecord, PlanningDraftLoadRecord,
    PlanningDraftStageRecord, PlanningStagedFileRecord,
};

use super::store::upsert_authority_metadata;
use super::workspace_paths::{draft_directory_display_path, draft_display_path};
use super::{SqlitePlanningAuthorityAdapter, open_authority_connection};

impl SqlitePlanningAuthorityAdapter {
    /*
    여러 planning 파일을 하나의 초안 이름 아래에 새로 staging한다.

    이 함수의 핵심 계약은 "같은 `draft_name`의 기존 staged 파일 목록을 전부 지운 뒤,
    전달받은 `files` 목록으로 초안을 통째로 교체한다"다. 그래서 DB 작업은 반드시
    하나의 transaction 안에서 실행된다. 중간에 한 파일만 insert되고 실패하면 초안이 반쯤
    바뀐 상태가 되므로, `DELETE`와 모든 `INSERT`와 metadata 갱신이 함께 commit되어야 한다.

    처리 흐름:
    1. workspace 경로에서 repo-scoped authority DB 위치를 찾는다.
    2. `last_draft_updated_at` metadata와 `staged_drafts`의 초안 entry를 갱신한다.
    3. 같은 초안 이름의 기존 `staged_draft_files` 행을 비운다.
    4. 전달받은 각 active planning 파일 본문을 DB에 저장한다.
    5. 호출자가 확인할 수 있도록 active path와 표시용 staged path를 반환한다.

    여기서 반환하는 `staged_path`는 실제 디스크 파일을 보장하는 경로가 아니라,
    기존 파일 기반 workspace API와 호환되도록 만들어진 "사용자에게 보여줄 위치"다.
    */
    pub(crate) fn stage_repo_scoped_draft_files(
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open authority-store draft stage transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_draft_updated_at")?;
        upsert_draft_entry(&transaction, draft_name)?;
        transaction
            .execute(
                "DELETE FROM staged_draft_files WHERE draft_name = ?1",
                params![draft_name],
            )
            .with_context(|| format!("failed to clear staged draft `{draft_name}`"))?;

        /*
        `Vec::with_capacity(files.len())`는 결과 레코드 개수가 입력 파일 수와 같다는 점을
        이용한다. 작은 최적화지만 이 함수의 의미도 드러낸다. 저장 대상 하나마다
        반환할 `PlanningStagedFileRecord`도 하나씩 생긴다.
        */
        let mut staged_files = Vec::with_capacity(files.len());
        for file in files {
            transaction
                .execute(
                    "INSERT INTO staged_draft_files (draft_name, active_path, content)
                     VALUES (?1, ?2, ?3)",
                    params![draft_name, &file.active_path, &file.body],
                )
                .with_context(|| {
                    format!(
                        "failed to persist staged draft file `{}` for `{draft_name}`",
                        file.active_path
                    )
                })?;
            staged_files.push(PlanningStagedFileRecord {
                active_path: file.active_path.clone(),
                staged_path: draft_display_path(&location, draft_name, &file.active_path),
            });
        }

        transaction
            .commit()
            .context("failed to commit authority-store draft stage transaction")?;

        Ok(PlanningDraftStageRecord {
            draft_name: draft_name.to_string(),
            draft_directory: draft_directory_display_path(&location, draft_name),
            staged_files,
        })
    }

    /*
    authority DB에 staging된 특정 초안을 다시 읽어 application 계층의 load record로 복원한다.

    먼저 `staged_drafts`에서 초안 이름 자체가 존재하는지 확인하는 이유는, "초안은 존재하지만
    파일이 0개인 상태"와 "초안 이름이 아예 없음"을 구분하기 위해서다. 아래 파일 목록 조회만
    수행하면 두 경우가 모두 빈 목록처럼 보인다. application 계층에는 존재하지 않는 초안 요청을
    명확한 오류로 돌려주어야 하므로, 존재 확인을 별도로 둔다.

    파일 행은 `ORDER BY active_path`로 정렬한다. DB는 별도 정렬 없이는 행 순서를 보장하지 않으므로,
    이 정렬은 TUI 출력과 테스트 snapshot이 매번 같은 순서로 나오게 하는 계약이다.
    */
    pub(crate) fn load_repo_scoped_draft_files(
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        let draft_exists = connection
            .query_row(
                "SELECT 1 FROM staged_drafts WHERE draft_name = ?1",
                params![draft_name],
                |_| Ok(()),
            )
            .optional()
            .with_context(|| format!("failed to inspect staged draft `{draft_name}`"))?
            .is_some();
        if !draft_exists {
            return Err(anyhow!("staged draft `{draft_name}` does not exist"));
        }

        let mut statement = connection
            .prepare(
                "SELECT active_path, content
                 FROM staged_draft_files
                 WHERE draft_name = ?1
                 ORDER BY active_path",
            )
            .with_context(|| format!("failed to read staged draft `{draft_name}`"))?;
        let rows = statement
            .query_map(params![draft_name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .with_context(|| format!("failed to iterate staged draft `{draft_name}`"))?;

        /*
        rusqlite의 `query_map`은 iterator를 만들 때뿐 아니라 각 행을 꺼내 decode할 때도 실패할 수
        있다. 그래서 loop 안에서 `row.context(...)`를 붙여, SQL 실행 실패와 행 decode 실패를
        서로 다른 message로 남긴다.
        */
        let mut staged_files = Vec::new();
        for row in rows {
            let (active_path, body) = row.context("failed to decode staged draft row")?;
            staged_files.push(PlanningDraftLoadFileRecord {
                staged_path: draft_display_path(&location, draft_name, &active_path),
                body,
                active_path,
            });
        }

        Ok(PlanningDraftLoadRecord {
            draft_name: draft_name.to_string(),
            draft_directory: draft_directory_display_path(&location, draft_name),
            staged_files,
        })
    }

    /*
    초안 안의 단일 planning 파일 본문만 추가하거나 교체한다.

    `stage_repo_scoped_draft_files`가 초안 전체를 갈아끼우는 bulk API라면, 이 함수는 사용자가
    한 파일만 수정했을 때 쓰는 좁은 API다. `ON CONFLICT(draft_name, active_path)`를 사용하므로
    같은 초안과 active path 조합이 이미 있으면 content만 바꾸고, 없으면 새 행을 만든다.

    이 함수도 `upsert_draft_entry`를 먼저 호출한다. 그 덕분에 아직 초안 이름이 없던 상태에서
    단일 파일만 저장해도 `staged_drafts` 부모 엔트리와 `staged_draft_files` 자식 행이 함께
    만들어진다. 외래키처럼 동작하는 논리적 부모-자식 관계를 application 코드가 신경 쓰지
    않아도 되게 만드는 adapter 내부 책임이다.
    */
    pub(crate) fn replace_repo_scoped_draft_file(
        workspace_dir: &str,
        draft_name: &str,
        active_path: &str,
        body: &str,
    ) -> Result<String> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open authority-store draft replace transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_draft_updated_at")?;
        upsert_draft_entry(&transaction, draft_name)?;
        transaction
            .execute(
                "INSERT INTO staged_draft_files (draft_name, active_path, content)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(draft_name, active_path) DO UPDATE
                 SET content = excluded.content",
                params![draft_name, active_path, body],
            )
            .with_context(|| {
                format!("failed to update staged draft file `{active_path}` for `{draft_name}`")
            })?;
        transaction
            .commit()
            .context("failed to commit authority-store draft replace transaction")?;
        Ok(draft_display_path(&location, draft_name, active_path))
    }
}

/*
`staged_drafts` 테이블의 초안 엔트리를 insert-or-update하는 작은 helper다.

이 helper가 별도로 있는 이유는 bulk staging과 단일 파일 교체가 모두 같은 부모 엔트리 갱신
규칙을 공유하기 때문이다. 초안 이름이 처음 등장하면 새 행을 만들고, 이미 있으면 `updated_at`만
현재 시각으로 바꾼다. 즉 `staged_drafts`는 초안의 존재와 마지막 갱신 시각을 표현하고,
`staged_draft_files`는 그 초안에 속한 실제 planning 파일 본문들을 표현한다.
*/
fn upsert_draft_entry(transaction: &rusqlite::Transaction<'_>, draft_name: &str) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO staged_drafts (draft_name, updated_at) VALUES (?1, ?2)
             ON CONFLICT(draft_name) DO UPDATE SET updated_at = excluded.updated_at",
            params![draft_name, Utc::now().to_rfc3339()],
        )
        .with_context(|| format!("failed to update staged draft `{draft_name}`"))?;
    Ok(())
}

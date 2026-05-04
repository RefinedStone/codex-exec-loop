/*
이 파일은 SQLite 기반 planning authority adapter의 최상위 조립 지점이다.

하위 모듈들은 active document, draft, task row, runtime projection처럼 저장소 내부 관심사를 나눠 맡고,
이 파일은 application port trait이 요구하는 함수들을 SQLite transaction 흐름으로 연결한다. 즉 여기의
함수들은 대부분 "port method -> workspace 위치 해석 -> DB connection 열기 -> 하위 저장소 함수 호출 ->
metadata/revision 갱신"이라는 adapter orchestration 역할을 한다.

프로젝트 구조 관점에서 이 타입은 outbound adapter이다. domain/application은 SQLite를 직접 알지 않고
`PlanningAuthorityPort`와 `PlanningTaskRepositoryPort`만 의존하며, 이 파일이 그 port 계약을 실제 DB
작업으로 번역한다.
*/
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
    PlanningAuthorityOfficialRefreshRecoveryStatus, PlanningAuthorityPort,
    PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningDirectionAuthoritySnapshot,
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskAuthoritySnapshot,
    PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
    ParallelModeTaskDispatchBlockSnapshot,
};
// active snapshot 테이블을 다루는 하위 모듈이다.
mod active_documents;
// repo-scoped draft staging을 SQLite 행으로 저장하는 하위 모듈이다.
mod draft_files;
// filesystem workspace port가 git-backed workspace를 발견했을 때 호출하는 trait adapter이다.
mod repo_scoped_workspace;
// parallel/app-server runtime projection tables를 다루는 하위 모듈이다.
mod runtime_projection;
// schema, metadata, authority document load/store의 공통 저장소 모듈이다.
mod store;
// task authority 문서와 queue projection을 정규화된 task table로 펼치는 모듈이다.
mod task_authority_rows;
// workspace path를 canonical repo root와 authority DB 위치로 해석하는 모듈이다.
mod workspace_paths;

use self::active_documents::{
    apply_active_workspace_record, remove_active_documents, set_active_document,
};
use self::store::*;
use self::task_authority_rows::{clear_task_authority_tables, replace_task_authority_tables};
use crate::domain::planning::{
    PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection,
    PlanningAuthorityShadowStoreSyncState,
};

// authority DB schema가 바뀔 때 올리는 adapter 내부 schema marker이다.
const AUTHORITY_STORE_SCHEMA_VERSION: i64 = 5;
// metadata에 저장되는 store mode 값으로, 다른 DB 파일과 planning authority store를 구분한다.
const AUTHORITY_STORE_MODE: &str = "authority-store";
// official refresh claim은 repo 전체에 하나만 있어야 하므로 고정 scope key를 사용한다.
const OFFICIAL_REFRESH_SCOPE_KEY: &str = "official-refresh";
// distributor queue head claim을 runtime_claims table에서 식별하는 claim kind이다.
const DISTRIBUTOR_QUEUE_CLAIM_KIND: &str = "distributor-queue-head";
// claim owner가 갱신하지 않은 채 이 시간을 넘기면 다른 worker가 stale claim으로 볼 수 있다.
const CLAIM_STALE_AFTER_SECS: i64 = 300;
// task authority 문서 version을 metadata table에 저장할 때 쓰는 key이다.
const TASK_LEDGER_VERSION_METADATA_KEY: &str = "task_authority_version";
#[derive(Default)]
/*
SQLite planning authority adapter의 값 타입이다.

필드를 갖지 않는 이유는 모든 상태가 repo-scoped authority DB 파일과 transaction 안에 있기 때문이다.
adapter 인스턴스는 connection pool이나 cache를 소유하지 않고, 호출마다 workspace에서 DB 위치를 해석해
connection을 연다. 그래서 `Default`와 `new()`는 단순한 생성자 역할만 한다.
*/
pub struct SqlitePlanningAuthorityAdapter;

impl SqlitePlanningAuthorityAdapter {
    /*
    상태 없는 adapter 값을 만든다.

    application wiring에서는 구체 타입을 생성해 port trait object나 service dependency로 넘긴다. 이
    생성자는 그런 조립 지점에서 `Default::default()` 대신 명시적인 의도를 보여주기 위한 API이다.
    */
    pub fn new() -> Self {
        Self
    }

    /*
    repo-scoped active workspace 파일 snapshot을 authority DB에 commit한다.

    `PlanningWorkspaceLoadRecord`는 filesystem workspace adapter가 사용하는 load record와 같은 형태이다.
    이 함수는 그 record를 SQLite의 `active_documents` table로 반영하고, 실제 내용이 바뀌었을 때만
    `planning_revision`을 올린다. revision은 runtime projection과 polling 쪽에서 "planning 상태가
    갱신되었는가"를 판단하는 기준이므로, no-op commit에서 불필요하게 증가하면 downstream worker가
    쓸데없이 다시 반응할 수 있다.

    metadata 갱신, active document 적용, revision bump는 하나의 transaction 안에서 실행된다. 따라서
    active snapshot과 revision은 항상 같은 commit 시점의 상태로 유지된다.
    */
    pub(crate) fn commit_active_workspace_files(
        workspace_dir: &str,
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open authority-store active commit transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_active_commit_at")?;
        let changed = apply_active_workspace_record(&transaction, record)?;
        if changed {
            bump_planning_revision(&transaction)?;
        }
        transaction
            .commit()
            .context("failed to commit authority-store active commit transaction")?;

        Ok(())
    }

    /*
    active workspace snapshot을 `PlanningWorkspaceLoadRecord`로 읽는다.

    이 함수는 commit 함수의 반대 방향 adapter이다. workspace path에서 같은 authority DB 위치를 찾고,
    store 모듈의 `load_active_workspace_record`로 실제 record 조립을 위임한다. 상위 caller는 SQLite
    table 구조를 모르고 기존 workspace port의 record만 받는다.
    */
    pub(crate) fn load_active_workspace_files(
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_active_workspace_record(&connection)
    }

    /*
    active snapshot에서 planning 파일 하나만 읽는다.

    전체 workspace record가 필요 없는 호출 경로를 위한 좁은 API이다. 예를 들어 특정 authority 문서나
    결과 파일 하나만 확인할 때 전체 active document map을 application 쪽으로 끌어올리지 않아도 된다.
    row가 없으면 `None`이므로, caller는 "파일 없음"과 "DB 조회 실패"를 구분할 수 있다.
    */
    pub(crate) fn load_active_planning_file(
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_active_document(&connection, relative_path)
    }

    /*
    task authority snapshot을 repo-scoped authority DB에서 읽는다.

    이 함수는 application service가 현재 task ledger와 queue projection을 확인할 때 쓰는 좁은 입구이다.
    실제 row 복원은 store/task row 모듈이 담당하고, 여기서는 workspace 경로를 DB 위치로 해석한 뒤
    connection을 열어 위임한다.
    */
    pub(crate) fn load_task_authority_snapshot(
        workspace_dir: &str,
    ) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_task_authority_snapshot_from_connection(&connection)
    }

    /*
    direction authority snapshot을 repo-scoped authority DB에서 읽는다.

    direction authority는 task가 속할 수 있는 큰 작업 방향 catalog이다. task authority와 분리되어 있지만
    task pruning에서 서로 연결되므로, 같은 DB의 planning revision 체계 안에서 읽고 쓴다.
    */
    pub(crate) fn load_direction_authority_snapshot(
        workspace_dir: &str,
    ) -> Result<Option<PlanningDirectionAuthoritySnapshot>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_direction_authority_snapshot_from_connection(&connection)
    }

    /*
    direction authority catalog를 commit하고 planning revision을 갱신한다.

    commit에는 caller가 마지막으로 관찰한 planning revision이 들어올 수 있다. 이 값이 현재 DB revision과
    다르면 optimistic concurrency conflict를 반환한다. 여러 agent나 TUI 동작이 같은 authority를 동시에
    바꾸는 상황에서 오래된 화면의 저장이 최신 상태를 덮어쓰지 않게 하는 장치이다.

    기존 snapshot과 새 directions가 같으면 no-op commit으로 보고 revision을 올리지 않는다. 실제 변경이
    있으면 direction tables를 교체하고, 사라진 direction을 참조하는 task authority도 같은 transaction에서
    정리한 뒤 revision을 올린다.
    */
    pub(crate) fn commit_direction_authority_snapshot(
        workspace_dir: &str,
        commit: PlanningDirectionAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open direction authority commit transaction")?;
        let current_revision = read_metadata_i64(&transaction, "planning_revision")?.unwrap_or(0);
        if let Some(observed_revision) = commit.observed_planning_revision
            && observed_revision != current_revision
        {
            return Ok(PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: observed_revision,
                current_planning_revision: current_revision,
            });
        }
        if let Some(existing_snapshot) =
            load_direction_authority_snapshot_from_connection(&transaction)?
            && existing_snapshot.directions == *commit.directions
        {
            return Ok(PlanningTaskAuthorityCommitResult::Committed {
                planning_revision: current_revision,
            });
        }

        upsert_authority_metadata(
            &transaction,
            &location,
            "last_direction_authority_commit_at",
        )?;
        replace_direction_authority_tables(&transaction, commit.directions)?;
        reconcile_task_authority_with_directions(&transaction, Some(commit.directions))?;
        let planning_revision = bump_planning_revision(&transaction)?;
        transaction
            .commit()
            .context("failed to commit direction authority transaction")?;

        Ok(PlanningTaskAuthorityCommitResult::Committed { planning_revision })
    }

    /*
    direction authority snapshot을 제거한다.

    direction catalog가 사라지면 task가 참조할 수 있는 direction id 집합도 비게 된다. 따라서 같은
    transaction에서 task authority reconcile을 호출해 모든 task와 edge를 정리한다. 이후 revision을
    올려 downstream runtime이 planning authority 변화로 인식하게 한다.
    */
    pub(crate) fn clear_direction_authority_snapshot(workspace_dir: &str) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open direction authority clear transaction")?;
        upsert_authority_metadata(
            &transaction,
            &location,
            "last_direction_authority_commit_at",
        )?;
        clear_direction_authority_tables(&transaction)?;
        reconcile_task_authority_with_directions(&transaction, None)?;
        bump_planning_revision(&transaction)?;
        transaction
            .commit()
            .context("failed to clear direction authority transaction")?;

        Ok(())
    }

    /*
    task authority 문서와 queue projection을 함께 commit한다.

    task authority는 task 정의 목록이고 queue projection은 그 목록에서 파생된 현재 실행 순서이다. 두 값은
    같은 planning revision의 snapshot이어야 하므로 한 transaction에서 같이 저장한다. direction commit과
    동일하게 observed revision으로 optimistic concurrency를 검사하고, 기존 task authority/queue projection과
    완전히 같으면 revision bump를 생략한다.
    */
    pub(crate) fn commit_task_authority_snapshot(
        workspace_dir: &str,
        commit: PlanningTaskAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open task authority commit transaction")?;
        let current_revision = read_metadata_i64(&transaction, "planning_revision")?.unwrap_or(0);
        if let Some(observed_revision) = commit.observed_planning_revision
            && observed_revision != current_revision
        {
            return Ok(PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: observed_revision,
                current_planning_revision: current_revision,
            });
        }
        if let Some(existing_snapshot) = load_task_authority_snapshot_from_connection(&transaction)?
            && existing_snapshot.task_authority == *commit.task_authority
            && existing_snapshot.queue_projection == *commit.queue_projection
        {
            return Ok(PlanningTaskAuthorityCommitResult::Committed {
                planning_revision: current_revision,
            });
        }

        upsert_authority_metadata(&transaction, &location, "last_task_authority_commit_at")?;
        replace_task_authority_tables(
            &transaction,
            commit.task_authority,
            commit.queue_projection,
        )?;
        let planning_revision = bump_planning_revision(&transaction)?;
        transaction
            .commit()
            .context("failed to commit task authority transaction")?;

        Ok(PlanningTaskAuthorityCommitResult::Committed { planning_revision })
    }

    /*
    task authority snapshot과 queue projection을 제거한다.

    direction clear와 달리 task clear는 direction catalog를 건드리지 않는다. 작업 목록만 초기화하고,
    metadata와 planning revision을 갱신해 이후 load가 task authority 없음 상태를 반환하도록 만든다.
    */
    pub(crate) fn clear_task_authority_snapshot(workspace_dir: &str) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open task authority clear transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_task_authority_commit_at")?;
        clear_task_authority_tables(&transaction)?;
        bump_planning_revision(&transaction)?;
        transaction
            .commit()
            .context("failed to clear task authority transaction")?;

        Ok(())
    }

    /*
    active snapshot 안의 단일 planning 파일을 교체하거나 삭제한다.

    `body: Some(...)`이면 `relative_path`에 해당하는 active document를 upsert하고, `body: None`이면 같은
    API로 삭제 의미를 표현한다. 이 `Option` 계약은 repo-scoped workspace port에서 "파일 내용 쓰기"와
    "파일 제거"를 하나의 좁은 경계로 전달하기 위해 사용된다.

    `set_active_document`는 실제 내용이 달라졌는지를 bool로 돌려준다. 이 값이 true일 때만
    `planning_revision`을 올리는 이유는 active snapshot 변경이 없는 요청을 runtime/poller에게 새
    planning 상태처럼 알리지 않기 위해서이다.
    */
    pub(crate) fn replace_active_planning_file(
        workspace_dir: &str,
        relative_path: &str,
        body: Option<&str>,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open authority-store active file transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_active_commit_at")?;
        let changed = set_active_document(&transaction, relative_path, body)?;
        if changed {
            bump_planning_revision(&transaction)?;
        }
        transaction
            .commit()
            .context("failed to commit authority-store active file transaction")?;

        Ok(())
    }

    /*
    active snapshot에서 특정 경로와 그 하위 entry들을 제거한다.

    `remove_active_documents`는 단일 파일 삭제뿐 아니라 디렉터리 성격의 prefix 삭제도 담당할 수 있는
    하위 helper이다. 그래서 함수 이름도 file이 아니라 entry이다. repo-scoped workspace에서 planning
    artifact를 제거할 때, DB의 active snapshot과 planning revision을 함께 갱신하는 경계이다.
    */
    pub(crate) fn remove_active_planning_entry(
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open authority-store active removal transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_active_commit_at")?;
        let changed = remove_active_documents(&transaction, relative_path)?;
        if changed {
            bump_planning_revision(&transaction)?;
        }
        transaction
            .commit()
            .context("failed to commit authority-store active removal transaction")?;

        Ok(())
    }

    /*
    shadow store를 검사하고, 필요하면 active authority documents를 mirror한다.

    shadow store는 DB가 active authority documents를 별도 mirror table에 보존하는 진단/복구용 영역이다.
    이 함수는 현재 active authority documents와 이전 shadow documents를 비교해 sync 상태를 판정한 뒤,
    active documents를 shadow table에 다시 저장한다. 저장 직후 다시 읽어서 parity를 검증하므로,
    inspection 결과는 "쓰기 전 상태"와 "쓰기 후 검증"을 모두 반영한다.

    반환되는 sync state 의미:
    - `Bootstrapped`: DB 파일이 없었거나 shadow가 비어 있어 새로 mirror를 만들었다.
    - `InSync`: 이전 shadow가 이미 active documents와 같았다.
    - `Resynced`: 이전 shadow에 차이가 있었고 이번 호출에서 active 상태로 맞췄다.
    */
    fn inspect_shadow_store_impl(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection> {
        let location = self.resolve_authority_location(workspace_dir)?;
        let authority_store_path = PathBuf::from(&location.authority_store_path);
        let had_store = authority_store_path.is_file();
        let mut connection = open_authority_connection(&location)?;
        let previous_documents = load_shadow_documents(&connection)?;
        let source_documents = load_active_authority_documents(&connection)?;
        let shadow_parity_issues = compare_shadow_documents(&source_documents, &previous_documents);
        store_shadow_documents(&mut connection, &location, &source_documents)?;

        let mirrored_documents = load_shadow_documents(&connection)?;
        let post_sync_issues = compare_shadow_documents(&source_documents, &mirrored_documents);
        if !post_sync_issues.is_empty() {
            let summary = post_sync_issues.join(", ");
            return Err(anyhow!(
                "shadow store parity check failed after sync: {summary}"
            ));
        }

        let sync_state = if !had_store || previous_documents.is_empty() {
            PlanningAuthorityShadowStoreSyncState::Bootstrapped
        } else if shadow_parity_issues.is_empty() {
            PlanningAuthorityShadowStoreSyncState::InSync
        } else {
            PlanningAuthorityShadowStoreSyncState::Resynced
        };
        let parity_issue_examples = shadow_parity_issues
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>();

        Ok(PlanningAuthorityShadowStoreInspection {
            location,
            sync_state,
            mirrored_document_count: source_documents.len(),
            parity_issue_count: shadow_parity_issues.len(),
            parity_issue_examples,
        })
    }
}

/*
source document map과 shadow/mirror document map의 차이를 사람이 읽을 수 있는 문자열 목록으로 만든다.

두 map의 key 전체 합집합을 기준으로 비교한다. source에는 있는데 mirror에는 없으면 shadow 누락,
mirror에만 있으면 stale content, 둘 다 있지만 본문이 다르면 mismatch로 분류한다. 이 함수는 실제
복구를 수행하지 않고 진단 문구만 만들며, `inspect_shadow_store_impl`이 이 결과를 바탕으로 sync state와
예시를 구성한다.
*/
fn compare_shadow_documents(
    source_documents: &BTreeMap<String, String>,
    mirrored_documents: &BTreeMap<String, String>,
) -> Vec<String> {
    let document_paths = source_documents
        .keys()
        .chain(mirrored_documents.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut issues = Vec::new();
    for relative_path in document_paths {
        match (
            source_documents.get(&relative_path),
            mirrored_documents.get(&relative_path),
        ) {
            (Some(_), None) => issues.push(format!("{relative_path}: missing from shadow store")),
            (None, Some(_)) => issues.push(format!(
                "{relative_path}: shadow store contains stale content"
            )),
            (Some(source), Some(mirrored)) if source != mirrored => {
                issues.push(format!("{relative_path}: content mismatch"));
            }
            _ => {}
        }
    }

    issues
}

/*
application의 `PlanningAuthorityPort`를 SQLite adapter에 연결한다.

이 trait은 app-server/parallel runtime 관점의 authority 작업을 표현한다. 구현 대부분은 같은 파일이나
`runtime_projection` 모듈에 있는 inherent method로 바로 위임한다. 이렇게 얇은 위임을 두는 이유는
application 계층이 구체 타입을 몰라도 port trait만으로 runtime claim, queue, lease, session projection을
다룰 수 있게 하기 위해서이다.
*/
impl PlanningAuthorityPort for SqlitePlanningAuthorityAdapter {
    /*
    workspace 경로를 authority DB 위치 정보로 해석한다.

    이 port method는 외부 caller가 DB 파일 경로와 canonical repo root를 확인해야 할 때 쓰는 공개 경계이다.
    실제 path 정책은 `workspace_paths` 모듈의 shared helper에 둔다.
    */
    fn resolve_authority_location(&self, workspace_dir: &str) -> Result<PlanningAuthorityLocation> {
        Self::resolve_authority_location_from_workspace(workspace_dir)
    }

    /*
    shadow store inspection port를 내부 구현으로 연결한다.

    trait 표면에서는 inspection이라는 use case만 보이고, 내부 구현은 active authority document와 shadow table을
    비교하고 필요 시 mirror를 갱신한다.
    */
    fn inspect_shadow_store(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection> {
        self.inspect_shadow_store_impl(workspace_dir)
    }

    /*
    official refresh 작업의 단조 증가 order를 예약한다.

    runtime에서 여러 actor가 refresh를 시도할 수 있으므로, SQLite claim/projection 쪽에서 다음 순번을
    발급하게 위임한다.
    */
    fn reserve_next_official_refresh_order(&self, workspace_dir: &str) -> Result<u64> {
        Self::reserve_next_official_refresh_order(workspace_dir)
    }

    /*
    특정 refresh order에 대한 official refresh claim을 획득한다.

    `owner_token`은 같은 process/worker가 자신이 잡은 claim을 식별하기 위한 값이고, stale claim 처리 규칙은
    하위 runtime projection 함수가 DB의 `runtime_claims` table에서 판단한다.
    */
    fn acquire_official_refresh_claim(
        &self,
        workspace_dir: &str,
        refresh_order: u64,
        owner_token: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshClaimStatus> {
        Self::acquire_official_refresh_claim(workspace_dir, refresh_order, owner_token)
    }

    /*
    owner token이 보유한 official refresh claim을 해제한다.

    release도 DB의 현재 owner와 token을 맞춰 보아야 하므로, trait method는 단순히 하위 SQLite claim helper로
    전달한다.
    */
    fn release_official_refresh_claim(
        &self,
        workspace_dir: &str,
        refresh_order: u64,
        owner_token: &str,
    ) -> Result<()> {
        Self::release_official_refresh_claim(workspace_dir, refresh_order, owner_token)
    }

    /*
    stale ledger refresh recovery가 다음 실행 포인터를 막는 abandoned order를 회수한다.
    */
    fn abandon_next_official_refresh_order(
        &self,
        workspace_dir: &str,
        reason: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshRecoveryStatus> {
        Self::abandon_next_official_refresh_order(workspace_dir, reason)
    }

    /*
    distributor queue item의 claim 획득을 시도한다.

    반환값은 획득 여부이다. 이미 다른 owner가 같은 queue item을 처리 중이면 false가 될 수 있고, caller는
    그 item을 건너뛰거나 나중에 다시 시도할 수 있다.
    */
    fn try_acquire_distributor_queue_claim(
        &self,
        workspace_dir: &str,
        queue_item_id: &str,
        owner_token: &str,
    ) -> Result<bool> {
        Self::try_acquire_distributor_queue_claim(workspace_dir, queue_item_id, owner_token)
    }

    /*
    distributor queue claim을 해제한다.

    queue item id와 owner token이 함께 들어가는 이유는 다른 worker가 잡은 claim을 실수로 지우지 않기
    위해서이다.
    */
    fn release_distributor_queue_claim(
        &self,
        workspace_dir: &str,
        queue_item_id: &str,
        owner_token: &str,
    ) -> Result<()> {
        Self::release_distributor_queue_claim(workspace_dir, queue_item_id, owner_token)
    }

    /*
    runtime projection snapshot 전체를 로드한다.

    slot lease, invalid lease, session detail, distributor queue, runtime event projection을 한 번에 읽는 port
    표면이다. 구체적인 table join/JSON decode는 runtime projection 모듈이 담당한다.
    */
    fn load_runtime_projections(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        Self::load_runtime_projections(workspace_dir)
    }

    fn clear_parallel_runtime_projections(&self, workspace_dir: &str, reason: &str) -> Result<()> {
        Self::clear_parallel_runtime_projections(workspace_dir, reason)
    }

    /*
    slot lease projection을 upsert한다.

    parallel-mode slot 상태는 runtime projection table에 최신 snapshot으로 저장된다. port caller는 lease
    구조만 넘기고, SQLite adapter가 직렬화와 timestamp 저장을 맡는다.
    */
    fn upsert_runtime_slot_lease(
        &self,
        workspace_dir: &str,
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()> {
        Self::upsert_runtime_slot_lease(workspace_dir, lease)
    }

    /*
    slot lease projection을 제거한다.

    worker가 slot을 더 이상 소유하지 않거나 lease가 무효화되었을 때 runtime projection에서 해당 slot id를
    제거하는 port 경계이다.
    */
    fn remove_runtime_slot_lease(&self, workspace_dir: &str, slot_id: &str) -> Result<()> {
        Self::remove_runtime_slot_lease(workspace_dir, slot_id)
    }

    /*
    agent session detail projection을 upsert한다.

    session detail은 slot보다 더 구체적인 agent 실행 상태이다. app-server/TUI가 현재 session 상태를
    조회할 수 있도록 SQLite runtime projection에 반영한다.
    */
    fn upsert_runtime_session_detail(
        &self,
        workspace_dir: &str,
        detail: &ParallelModeAgentSessionDetailSnapshot,
    ) -> Result<()> {
        Self::upsert_runtime_session_detail(workspace_dir, detail)
    }

    fn upsert_runtime_task_dispatch_block(
        &self,
        workspace_dir: &str,
        block: &ParallelModeTaskDispatchBlockSnapshot,
    ) -> Result<()> {
        Self::upsert_runtime_task_dispatch_block(workspace_dir, block)
    }

    /*
    distributor queue record projection을 upsert한다.

    distributor queue는 parallel mode에서 처리할 session/work item 흐름을 나타낸다. 이 port method는
    application이 만든 queue record를 DB projection으로 저장해 다른 process가 같은 queue 상태를 볼 수
    있게 한다.
    */
    fn upsert_runtime_distributor_queue_record(
        &self,
        workspace_dir: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
    ) -> Result<()> {
        Self::upsert_runtime_distributor_queue_record(workspace_dir, record)
    }
}

/*
application의 `PlanningTaskRepositoryPort`를 같은 SQLite authority DB 구현에 연결한다.

이 trait은 planning task/direction authority 관점의 저장소 port이다. 위의 `PlanningAuthorityPort`가
runtime/claim/projection 중심이라면, 이 impl은 planning direction catalog와 task ledger snapshot을
다룬다. 실제 저장 로직은 같은 inherent helper를 공유하므로, 두 port가 동일한 DB와 planning revision
규칙을 보게 된다.
*/
impl PlanningTaskRepositoryPort for SqlitePlanningAuthorityAdapter {
    // direction authority 읽기 port를 SQLite snapshot load helper에 연결한다.
    fn load_direction_authority_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<PlanningDirectionAuthoritySnapshot>> {
        Self::load_direction_authority_snapshot(workspace_dir)
    }

    // direction authority commit port를 optimistic revision 검사와 DB 교체 helper에 연결한다.
    fn commit_direction_authority_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningDirectionAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        Self::commit_direction_authority_snapshot(workspace_dir, commit)
    }

    // direction authority 제거 port를 DB clear와 task reconcile helper에 연결한다.
    fn clear_direction_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        Self::clear_direction_authority_snapshot(workspace_dir)
    }

    // task authority 읽기 port를 task ledger와 queue projection 복원 helper에 연결한다.
    fn load_task_authority_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
        Self::load_task_authority_snapshot(workspace_dir)
    }

    // task authority commit port를 task row와 queue projection의 원자적 저장 helper에 연결한다.
    fn commit_task_authority_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningTaskAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        Self::commit_task_authority_snapshot(workspace_dir, commit)
    }

    // task authority 제거 port를 task rows/edges/projection clear helper에 연결한다.
    fn clear_task_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        Self::clear_task_authority_snapshot(workspace_dir)
    }
}

/*
authority DB connection을 열고, 모든 caller가 의존하는 기본 DB 상태를 보장한다.

이 함수는 단순한 `Connection::open` wrapper가 아니다. repo-scoped authority DB의 진입점으로서 다음
순서를 항상 지킨다.
1. 예전 repo 내부 runtime 위치의 DB를 새 관리 디렉터리 위치로 복사할 수 있으면 migrate한다.
2. 새 authority DB의 parent directory를 만든다.
3. SQLite connection을 연다.
4. 기존 DB라면 schema version이 이 binary가 이해할 수 있는 범위인지 검사한다.
5. foreign key enforcement를 켠다.
6. 현재 schema가 필요로 하는 table/index를 보장한다.

이 순서가 중요하다. schema를 만들기 전에 version gate를 통과해야 오래된/미래 schema를 잘못 덮어쓰지
않고, foreign key pragma는 task edge/draft file 같은 자식 row 정합성을 DB 차원에서 지키게 한다.
*/
fn open_authority_connection(location: &PlanningAuthorityLocation) -> Result<Connection> {
    let authority_store_path = Path::new(&location.authority_store_path);
    migrate_legacy_authority_store_if_needed(location)?;
    if let Some(parent) = authority_store_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let connection = Connection::open(authority_store_path)
        .with_context(|| format!("failed to open {}", authority_store_path.display()))?;
    validate_authority_store_schema(&connection)?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .context("failed to enable authority-store foreign keys")?;
    ensure_schema(&connection)?;
    Ok(connection)
}

/*
legacy 위치에 있던 authority DB를 현재 관리 디렉터리 위치로 복사한다.

이 프로젝트는 authority store 위치를 repo 내부 `.codex-exec-loop/runtime/...`에서 user-level
`.akra/projects/<repo-hash>/runtime/...`로 옮긴 흐름이 있다. 새 위치에 DB가 이미 있으면 아무것도 하지
않는다. 새 위치가 비어 있고 legacy 파일이 있으면 복사해서 기존 사용자 데이터를 잃지 않게 한다.

복사만 하고 legacy 파일을 삭제하지 않는 것은 보수적인 migration 전략이다. 문제가 생겨도 원본 DB가
repo 안에 남아 있어 복구할 수 있다.
*/
fn migrate_legacy_authority_store_if_needed(location: &PlanningAuthorityLocation) -> Result<()> {
    let authority_store_path = Path::new(&location.authority_store_path);
    if authority_store_path.exists() {
        return Ok(());
    }

    let legacy_store_path = Path::new(&location.canonical_repo_root)
        .join(".codex-exec-loop/runtime/planning-authority.db");
    if !legacy_store_path.is_file() {
        return Ok(());
    }

    if let Some(parent) = authority_store_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(&legacy_store_path, authority_store_path).with_context(|| {
        format!(
            "failed to migrate legacy authority store from {} to {}",
            legacy_store_path.display(),
            authority_store_path.display()
        )
    })?;
    Ok(())
}

/*
이미 존재하는 authority DB에서 schema version metadata를 읽는다.

새 DB는 아직 `authority_metadata` table이 없을 수 있으므로 이 함수는 table 존재 확인이 끝난 뒤에만
호출된다. version은 문자열로 저장되지만, schema gate에서는 i64로 parse해 비교한다.
*/
fn load_schema_version(connection: &Connection) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("failed to read authority-store schema version")
}

/*
기존 authority DB가 현재 binary가 지원하는 schema인지 검사한다.

metadata table이 없으면 새 DB 또는 아주 초기 DB로 보고 schema 생성 단계로 넘긴다. metadata가 있으면
`schema_version`을 읽어 현재 버전 또는 명시적으로 호환 허용한 이전 버전만 통과시킨다. 여기서는 4와
현재 `AUTHORITY_STORE_SCHEMA_VERSION`을 허용한다.

이 guard가 없으면 미래 버전의 DB를 구버전 binary가 열어 schema를 덮거나 잘못 해석할 수 있다.
*/
fn validate_authority_store_schema(connection: &Connection) -> Result<()> {
    let metadata_exists = table_exists(connection, "authority_metadata")?;
    if !metadata_exists {
        return Ok(());
    }

    if let Some(schema_version) = load_schema_version(connection)?
        && !matches!(
            schema_version.parse::<i64>().ok(),
            Some(4) | Some(AUTHORITY_STORE_SCHEMA_VERSION)
        )
    {
        return Err(anyhow!(
            "unsupported authority-store schema version: {schema_version}"
        ));
    }

    Ok(())
}

/*
connection 기준으로 metadata string 값을 읽는다.

`authority_metadata`는 모든 projection이 공유하는 작은 key/value table이다. 이 helper는 connection만
있는 read path에서 사용되고, row가 없으면 `Ok(None)`을 반환한다. SQL 오류는 key 이름을 포함한 context로
올려 caller가 어떤 metadata read가 실패했는지 볼 수 있게 한다.
*/
fn read_metadata_string_connection(connection: &Connection, key: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("failed to read authority metadata `{key}`"))
}

/*
connection read path에서 metadata 값을 i64로 해석한다.

parse 실패를 error로 만들지 않고 `None`으로 접는 것은 이 metadata가 optional compatibility marker로도
쓰이기 때문이다. 값이 없거나 숫자가 아니면 caller는 기본값을 선택한다.
*/
fn read_metadata_i64_connection(connection: &Connection, key: &str) -> Result<Option<i64>> {
    read_metadata_string_connection(connection, key)
        .map(|value| value.and_then(|value| value.parse::<i64>().ok()))
}

/*
현재 transaction 안에서 planning revision을 1 증가시키고 새 값을 반환한다.

planning revision은 active documents, direction authority, task authority처럼 planning 상태를 바꾸는 commit이
발생했음을 downstream runtime에 알리는 단조 증가 값이다. 같은 transaction에서 metadata를 읽고 upsert하므로
상태 변경과 revision 변경이 함께 commit된다.
*/
fn bump_planning_revision(transaction: &rusqlite::Transaction<'_>) -> Result<i64> {
    let next_revision = read_metadata_i64(transaction, "planning_revision")?.unwrap_or(0) + 1;
    upsert_metadata(transaction, "planning_revision", &next_revision.to_string())?;
    Ok(next_revision)
}

/*
transaction 기준으로 metadata i64 값을 읽는다.

commit 함수들은 아직 commit되지 않은 metadata 변경과 같은 transaction 안에서 revision을 읽어야 하므로,
connection용 helper와 별도로 transaction용 helper를 둔다. optimistic concurrency check와 revision bump가
같은 DB snapshot을 보게 하는 작은 경계이다.
*/
fn read_metadata_i64(transaction: &rusqlite::Transaction<'_>, key: &str) -> Result<Option<i64>> {
    transaction
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("failed to read authority metadata `{key}`"))
        .map(|value| value.and_then(|value| value.parse::<i64>().ok()))
}

/*
SQLite schema catalog에서 특정 table 존재 여부를 확인한다.

schema validation과 backward-compatible load path에서 사용된다. table이 없다는 것은 오류가 아니라
`false`이며, sqlite_master 조회 자체가 실패했을 때만 error로 올린다.
*/
fn table_exists(connection: &Connection, table_name: &str) -> Result<bool> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table_name],
            |_| Ok(()),
        )
        .optional()
        .with_context(|| format!("failed to inspect sqlite table `{table_name}`"))
        .map(|value| value.is_some())
}

#[cfg(test)]
// adapter 통합 성격의 DB 저장 테스트를 별도 파일로 분리해 production code 흐름을 작게 유지한다.
mod tests;

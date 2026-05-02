/*
학습 주석:
이 파일은 SQLite 기반 planning authority adapter의 최상위 조립 지점입니다.

하위 모듈들은 active document, draft, task row, runtime projection처럼 저장소 내부 관심사를 나눠 맡고,
이 파일은 application port trait이 요구하는 함수들을 SQLite transaction 흐름으로 연결합니다. 즉 여기의
함수들은 대부분 "port method -> workspace 위치 해석 -> DB connection 열기 -> 하위 저장소 함수 호출 ->
metadata/revision 갱신"이라는 adapter orchestration 역할을 합니다.

프로젝트 구조 관점에서 이 타입은 outbound adapter입니다. domain/application은 SQLite를 직접 알지 않고
`PlanningAuthorityPort`와 `PlanningTaskRepositoryPort`만 의존하며, 이 파일이 그 port 계약을 실제 DB
작업으로 번역합니다.
*/
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningDirectionAuthoritySnapshot,
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskAuthoritySnapshot,
    PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
};
// 학습 주석: active snapshot 테이블을 다루는 하위 모듈입니다.
mod active_documents;
// 학습 주석: repo-scoped draft staging을 SQLite 행으로 저장하는 하위 모듈입니다.
mod draft_files;
// 학습 주석: filesystem workspace port가 git-backed workspace를 발견했을 때 호출하는 trait adapter입니다.
mod repo_scoped_workspace;
// 학습 주석: parallel/app-server runtime projection tables를 다루는 하위 모듈입니다.
mod runtime_projection;
// 학습 주석: schema, metadata, authority document load/store의 공통 저장소 모듈입니다.
mod store;
// 학습 주석: task authority 문서와 queue projection을 정규화된 task table로 펼치는 모듈입니다.
mod task_authority_rows;
// 학습 주석: workspace path를 canonical repo root와 authority DB 위치로 해석하는 모듈입니다.
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

// 학습 주석: authority DB schema가 바뀔 때 올리는 adapter 내부 schema marker입니다.
const AUTHORITY_STORE_SCHEMA_VERSION: i64 = 5;
// 학습 주석: metadata에 저장되는 store mode 값으로, 다른 DB 파일과 planning authority store를 구분합니다.
const AUTHORITY_STORE_MODE: &str = "authority-store";
// 학습 주석: official refresh claim은 repo 전체에 하나만 있어야 하므로 고정 scope key를 사용합니다.
const OFFICIAL_REFRESH_SCOPE_KEY: &str = "official-refresh";
// 학습 주석: distributor queue head claim을 runtime_claims table에서 식별하는 claim kind입니다.
const DISTRIBUTOR_QUEUE_CLAIM_KIND: &str = "distributor-queue-head";
// 학습 주석: claim owner가 갱신하지 않은 채 이 시간을 넘기면 다른 worker가 stale claim으로 볼 수 있습니다.
const CLAIM_STALE_AFTER_SECS: i64 = 300;
// 학습 주석: task authority 문서 version을 metadata table에 저장할 때 쓰는 key입니다.
const TASK_LEDGER_VERSION_METADATA_KEY: &str = "task_authority_version";
#[derive(Default)]
/*
학습 주석:
SQLite planning authority adapter의 값 타입입니다.

필드를 갖지 않는 이유는 모든 상태가 repo-scoped authority DB 파일과 transaction 안에 있기 때문입니다.
adapter 인스턴스는 connection pool이나 cache를 소유하지 않고, 호출마다 workspace에서 DB 위치를 해석해
connection을 엽니다. 그래서 `Default`와 `new()`는 단순한 생성자 역할만 합니다.
*/
pub struct SqlitePlanningAuthorityAdapter;

impl SqlitePlanningAuthorityAdapter {
    /*
    학습 주석:
    상태 없는 adapter 값을 만듭니다.

    application wiring에서는 구체 타입을 생성해 port trait object나 service dependency로 넘깁니다. 이
    생성자는 그런 조립 지점에서 `Default::default()` 대신 명시적인 의도를 보여주기 위한 API입니다.
    */
    pub fn new() -> Self {
        Self
    }

    /*
    학습 주석:
    repo-scoped active workspace 파일 snapshot을 authority DB에 commit합니다.

    `PlanningWorkspaceLoadRecord`는 filesystem workspace adapter가 사용하는 load record와 같은 형태입니다.
    이 함수는 그 record를 SQLite의 `active_documents` table로 반영하고, 실제 내용이 바뀌었을 때만
    `planning_revision`을 올립니다. revision은 runtime projection과 polling 쪽에서 "planning 상태가
    갱신되었는가"를 판단하는 기준이므로, no-op commit에서 불필요하게 증가하면 downstream worker가
    쓸데없이 다시 반응할 수 있습니다.

    metadata 갱신, active document 적용, revision bump는 하나의 transaction 안에서 실행됩니다. 따라서
    active snapshot과 revision은 항상 같은 commit 시점의 상태로 유지됩니다.
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
    학습 주석:
    active workspace snapshot을 `PlanningWorkspaceLoadRecord`로 읽습니다.

    이 함수는 commit 함수의 반대 방향 adapter입니다. workspace path에서 같은 authority DB 위치를 찾고,
    store 모듈의 `load_active_workspace_record`로 실제 record 조립을 위임합니다. 상위 caller는 SQLite
    table 구조를 모르고 기존 workspace port의 record만 받습니다.
    */
    pub(crate) fn load_active_workspace_files(
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_active_workspace_record(&connection)
    }

    /*
    학습 주석:
    active snapshot에서 planning 파일 하나만 읽습니다.

    전체 workspace record가 필요 없는 호출 경로를 위한 좁은 API입니다. 예를 들어 특정 authority 문서나
    결과 파일 하나만 확인할 때 전체 active document map을 application 쪽으로 끌어올리지 않아도 됩니다.
    row가 없으면 `None`이므로, caller는 "파일 없음"과 "DB 조회 실패"를 구분할 수 있습니다.
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
    학습 주석:
    task authority snapshot을 repo-scoped authority DB에서 읽습니다.

    이 함수는 application service가 현재 task ledger와 queue projection을 확인할 때 쓰는 좁은 입구입니다.
    실제 row 복원은 store/task row 모듈이 담당하고, 여기서는 workspace 경로를 DB 위치로 해석한 뒤
    connection을 열어 위임합니다.
    */
    pub(crate) fn load_task_authority_snapshot(
        workspace_dir: &str,
    ) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_task_authority_snapshot_from_connection(&connection)
    }

    /*
    학습 주석:
    direction authority snapshot을 repo-scoped authority DB에서 읽습니다.

    direction authority는 task가 속할 수 있는 큰 작업 방향 catalog입니다. task authority와 분리되어 있지만
    task pruning에서 서로 연결되므로, 같은 DB의 planning revision 체계 안에서 읽고 씁니다.
    */
    pub(crate) fn load_direction_authority_snapshot(
        workspace_dir: &str,
    ) -> Result<Option<PlanningDirectionAuthoritySnapshot>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_direction_authority_snapshot_from_connection(&connection)
    }

    /*
    학습 주석:
    direction authority catalog를 commit하고 planning revision을 갱신합니다.

    commit에는 caller가 마지막으로 관찰한 planning revision이 들어올 수 있습니다. 이 값이 현재 DB revision과
    다르면 optimistic concurrency conflict를 반환합니다. 여러 agent나 TUI 동작이 같은 authority를 동시에
    바꾸는 상황에서 오래된 화면의 저장이 최신 상태를 덮어쓰지 않게 하는 장치입니다.

    기존 snapshot과 새 directions가 같으면 no-op commit으로 보고 revision을 올리지 않습니다. 실제 변경이
    있으면 direction tables를 교체하고, 사라진 direction을 참조하는 task authority도 같은 transaction에서
    정리한 뒤 revision을 올립니다.
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
    학습 주석:
    direction authority snapshot을 제거합니다.

    direction catalog가 사라지면 task가 참조할 수 있는 direction id 집합도 비게 됩니다. 따라서 같은
    transaction에서 task authority reconcile을 호출해 모든 task와 edge를 정리합니다. 이후 revision을
    올려 downstream runtime이 planning authority 변화로 인식하게 합니다.
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
    학습 주석:
    task authority 문서와 queue projection을 함께 commit합니다.

    task authority는 task 정의 목록이고 queue projection은 그 목록에서 파생된 현재 실행 순서입니다. 두 값은
    같은 planning revision의 snapshot이어야 하므로 한 transaction에서 같이 저장합니다. direction commit과
    동일하게 observed revision으로 optimistic concurrency를 검사하고, 기존 task authority/queue projection과
    완전히 같으면 revision bump를 생략합니다.
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
    학습 주석:
    task authority snapshot과 queue projection을 제거합니다.

    direction clear와 달리 task clear는 direction catalog를 건드리지 않습니다. 작업 목록만 초기화하고,
    metadata와 planning revision을 갱신해 이후 load가 task authority 없음 상태를 반환하도록 만듭니다.
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
    학습 주석:
    active snapshot 안의 단일 planning 파일을 교체하거나 삭제합니다.

    `body: Some(...)`이면 `relative_path`에 해당하는 active document를 upsert하고, `body: None`이면 같은
    API로 삭제 의미를 표현합니다. 이 `Option` 계약은 repo-scoped workspace port에서 "파일 내용 쓰기"와
    "파일 제거"를 하나의 좁은 경계로 전달하기 위해 사용됩니다.

    `set_active_document`는 실제 내용이 달라졌는지를 bool로 돌려줍니다. 이 값이 true일 때만
    `planning_revision`을 올리는 이유는 active snapshot 변경이 없는 요청을 runtime/poller에게 새
    planning 상태처럼 알리지 않기 위해서입니다.
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
    학습 주석:
    active snapshot에서 특정 경로와 그 하위 entry들을 제거합니다.

    `remove_active_documents`는 단일 파일 삭제뿐 아니라 디렉터리 성격의 prefix 삭제도 담당할 수 있는
    하위 helper입니다. 그래서 함수 이름도 file이 아니라 entry입니다. repo-scoped workspace에서 planning
    artifact를 제거할 때, DB의 active snapshot과 planning revision을 함께 갱신하는 경계입니다.
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
    학습 주석:
    shadow store를 검사하고, 필요하면 active authority documents를 mirror합니다.

    shadow store는 DB가 active authority documents를 별도 mirror table에 보존하는 진단/복구용 영역입니다.
    이 함수는 현재 active authority documents와 이전 shadow documents를 비교해 sync 상태를 판정한 뒤,
    active documents를 shadow table에 다시 저장합니다. 저장 직후 다시 읽어서 parity를 검증하므로,
    inspection 결과는 "쓰기 전 상태"와 "쓰기 후 검증"을 모두 반영합니다.

    반환되는 sync state 의미:
    - `Bootstrapped`: DB 파일이 없었거나 shadow가 비어 있어 새로 mirror를 만들었습니다.
    - `InSync`: 이전 shadow가 이미 active documents와 같았습니다.
    - `Resynced`: 이전 shadow에 차이가 있었고 이번 호출에서 active 상태로 맞췄습니다.
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
학습 주석:
source document map과 shadow/mirror document map의 차이를 사람이 읽을 수 있는 문자열 목록으로 만듭니다.

두 map의 key 전체 합집합을 기준으로 비교합니다. source에는 있는데 mirror에는 없으면 shadow 누락,
mirror에만 있으면 stale content, 둘 다 있지만 본문이 다르면 mismatch로 분류합니다. 이 함수는 실제
복구를 수행하지 않고 진단 문구만 만들며, `inspect_shadow_store_impl`이 이 결과를 바탕으로 sync state와
예시를 구성합니다.
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

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl PlanningAuthorityPort for SqlitePlanningAuthorityAdapter {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn resolve_authority_location(&self, workspace_dir: &str) -> Result<PlanningAuthorityLocation> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::resolve_authority_location_from_workspace(workspace_dir)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn inspect_shadow_store(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection> {
        self.inspect_shadow_store_impl(workspace_dir)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn reserve_next_official_refresh_order(&self, workspace_dir: &str) -> Result<u64> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::reserve_next_official_refresh_order(workspace_dir)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn acquire_official_refresh_claim(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        refresh_order: u64,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        owner_token: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshClaimStatus> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::acquire_official_refresh_claim(workspace_dir, refresh_order, owner_token)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn release_official_refresh_claim(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        refresh_order: u64,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        owner_token: &str,
    ) -> Result<()> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::release_official_refresh_claim(workspace_dir, refresh_order, owner_token)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn try_acquire_distributor_queue_claim(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        queue_item_id: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        owner_token: &str,
    ) -> Result<bool> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::try_acquire_distributor_queue_claim(workspace_dir, queue_item_id, owner_token)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn release_distributor_queue_claim(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        queue_item_id: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        owner_token: &str,
    ) -> Result<()> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::release_distributor_queue_claim(workspace_dir, queue_item_id, owner_token)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn load_runtime_projections(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::load_runtime_projections(workspace_dir)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn upsert_runtime_slot_lease(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::upsert_runtime_slot_lease(workspace_dir, lease)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn remove_runtime_slot_lease(&self, workspace_dir: &str, slot_id: &str) -> Result<()> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::remove_runtime_slot_lease(workspace_dir, slot_id)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn upsert_runtime_session_detail(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        detail: &ParallelModeAgentSessionDetailSnapshot,
    ) -> Result<()> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::upsert_runtime_session_detail(workspace_dir, detail)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn upsert_runtime_distributor_queue_record(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        record: &PlanningAuthorityDistributorQueueRecord,
    ) -> Result<()> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::upsert_runtime_distributor_queue_record(workspace_dir, record)
    }
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl PlanningTaskRepositoryPort for SqlitePlanningAuthorityAdapter {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn load_direction_authority_snapshot(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
    ) -> Result<Option<PlanningDirectionAuthoritySnapshot>> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::load_direction_authority_snapshot(workspace_dir)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn commit_direction_authority_snapshot(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        commit: PlanningDirectionAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::commit_direction_authority_snapshot(workspace_dir, commit)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn clear_direction_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::clear_direction_authority_snapshot(workspace_dir)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn load_task_authority_snapshot(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
    ) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::load_task_authority_snapshot(workspace_dir)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn commit_task_authority_snapshot(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        commit: PlanningTaskAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::commit_task_authority_snapshot(workspace_dir, commit)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn clear_task_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::clear_task_authority_snapshot(workspace_dir)
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn open_authority_connection(location: &PlanningAuthorityLocation) -> Result<Connection> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let authority_store_path = Path::new(&location.authority_store_path);
    migrate_legacy_authority_store_if_needed(location)?;
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if let Some(parent) = authority_store_path.parent() {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        fs::create_dir_all(parent)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let connection = Connection::open(authority_store_path)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .with_context(|| format!("failed to open {}", authority_store_path.display()))?;
    validate_authority_store_schema(&connection)?;
    connection
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .execute_batch("PRAGMA foreign_keys = ON;")
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to enable authority-store foreign keys")?;
    ensure_schema(&connection)?;
    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(connection)
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn migrate_legacy_authority_store_if_needed(location: &PlanningAuthorityLocation) -> Result<()> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let authority_store_path = Path::new(&location.authority_store_path);
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if authority_store_path.exists() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(());
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let legacy_store_path = Path::new(&location.canonical_repo_root)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join(".codex-exec-loop/runtime/planning-authority.db");
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !legacy_store_path.is_file() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(());
    }

    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if let Some(parent) = authority_store_path.parent() {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        fs::create_dir_all(parent)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    fs::copy(&legacy_store_path, authority_store_path).with_context(|| {
        format!(
            "failed to migrate legacy authority store from {} to {}",
            legacy_store_path.display(),
            authority_store_path.display()
        )
    })?;
    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(())
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn load_schema_version(connection: &Connection) -> Result<Option<String>> {
    connection
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
            [],
            // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
            |row| row.get::<_, String>(0),
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .optional()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to read authority-store schema version")
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn validate_authority_store_schema(connection: &Connection) -> Result<()> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let metadata_exists = table_exists(connection, "authority_metadata")?;
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !metadata_exists {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(());
    }

    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if let Some(schema_version) = load_schema_version(connection)?
        && !matches!(
            schema_version.parse::<i64>().ok(),
            Some(4) | Some(AUTHORITY_STORE_SCHEMA_VERSION)
        )
    {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Err(anyhow!(
            "unsupported authority-store schema version: {schema_version}"
        ));
    }

    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(())
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn read_metadata_string_connection(connection: &Connection, key: &str) -> Result<Option<String>> {
    connection
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = ?1",
            params![key],
            // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
            |row| row.get::<_, String>(0),
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .optional()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .with_context(|| format!("failed to read authority metadata `{key}`"))
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn read_metadata_i64_connection(connection: &Connection, key: &str) -> Result<Option<i64>> {
    read_metadata_string_connection(connection, key)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|value| value.and_then(|value| value.parse::<i64>().ok()))
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn bump_planning_revision(transaction: &rusqlite::Transaction<'_>) -> Result<i64> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let next_revision = read_metadata_i64(transaction, "planning_revision")?.unwrap_or(0) + 1;
    upsert_metadata(transaction, "planning_revision", &next_revision.to_string())?;
    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(next_revision)
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn read_metadata_i64(transaction: &rusqlite::Transaction<'_>, key: &str) -> Result<Option<i64>> {
    transaction
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = ?1",
            params![key],
            // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
            |row| row.get::<_, String>(0),
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .optional()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .with_context(|| format!("failed to read authority metadata `{key}`"))
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|value| value.and_then(|value| value.parse::<i64>().ok()))
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn table_exists(connection: &Connection, table_name: &str) -> Result<bool> {
    connection
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table_name],
            // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
            |_| Ok(()),
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .optional()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .with_context(|| format!("failed to inspect sqlite table `{table_name}`"))
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|value| value.is_some())
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[cfg(test)]
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod tests;

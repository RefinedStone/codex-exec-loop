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
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::application::port::outbound::parallel_mode_runtime_event_log_port::{
    ParallelModeRuntimeEventLogPort, ParallelModeRuntimeEventLogRequest,
};
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityActiveDocumentMutation, PlanningAuthorityDistributorQueueRecord,
    PlanningAuthorityDocumentCommit, PlanningAuthorityDocumentSnapshot,
    PlanningAuthorityOfficialRefreshClaimStatus, PlanningAuthorityOfficialRefreshRecoveryStatus,
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot, PrValidationPollLeaseClaim,
    PrValidationPollLeaseClaimRequest, PrValidationPollLeaseRenewalRequest,
    PrValidationPollSettlement,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningAuthoritySnapshotCommit, PlanningDirectionAuthorityCommit,
    PlanningDirectionAuthoritySnapshot, PlanningTaskAuthorityCommit,
    PlanningTaskAuthorityCommitResult, PlanningTaskAuthorityMutationAudit,
    PlanningTaskAuthorityMutationRecord, PlanningTaskAuthoritySnapshot, PlanningTaskRepositoryPort,
    task_authority_mutation_records,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterRepositoryPort,
    ReviewCenterThreadProjection,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeDispatchCommandSnapshot,
    ParallelModePoolResetReport, ParallelModeRuntimeEventsSnapshot, ParallelModeSlotLeaseSnapshot,
    ParallelModeTaskDispatchBlockSnapshot, PrValidationRecord, PrValidationRecordKey,
};
// app-server prompt 입출력 trace를 authority DB runtime 영역에 저장하는 모듈이다.
mod app_server_prompt_log;
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
// Telegram control-plane polling cursor와 update idempotency inbox를 영속화한다.
mod telegram_update_ledger;
// Telegram bot id별 머신 전역 runner ownership과 canonical workspace binding을 영속화한다.
mod telegram_global_runner_lease;
// workspace path를 canonical repo root와 authority DB 위치로 해석하는 모듈이다.
mod workspace_paths;

use self::active_documents::{
    apply_active_workspace_record, remove_active_documents, set_active_document,
};
use self::draft_files::clear_staged_drafts;
use self::store::*;
use self::task_authority_rows::{clear_task_authority_tables, replace_task_authority_tables};
use crate::domain::planning::{
    PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection,
    PlanningAuthorityShadowStoreSyncState, TaskAuthorityDocument,
};

// authority DB schema가 바뀔 때 올리는 adapter 내부 schema marker이다.
const AUTHORITY_STORE_SCHEMA_VERSION: i64 = 13;
const MINIMUM_MIGRATABLE_AUTHORITY_STORE_SCHEMA_VERSION: i64 = 7;
// metadata에 저장되는 store mode 값으로, 다른 DB 파일과 planning authority store를 구분한다.
const AUTHORITY_STORE_MODE: &str = "authority-store";
// official refresh claim은 repo 전체에 하나만 있어야 하므로 고정 scope key를 사용한다.
const OFFICIAL_REFRESH_SCOPE_KEY: &str = "official-refresh";
const OFFICIAL_REFRESH_CLAIM_KIND: &str = "official-refresh";
// distributor queue head claim을 runtime_claims table에서 식별하는 claim kind이다.
const DISTRIBUTOR_QUEUE_CLAIM_KIND: &str = "distributor-queue-head";
const ADMIN_TASK_MUTATION_CLAIM_KIND: &str = "admin-task-mutation";
// Keep the historical claim value so an in-flight file-sync guard from an older
// process remains visible after upgrading. Its semantics now cover every
// workspace-wide admin authority mutation, including direction edits.
const ADMIN_AUTHORITY_MUTATION_CLAIM_KIND: &str = "admin-file-sync";
const ADMIN_AUTHORITY_MUTATION_SCOPE_KEY: &str = "workspace";
#[cfg(test)]
const ADMIN_FILE_SYNC_CLAIM_KIND: &str = ADMIN_AUTHORITY_MUTATION_CLAIM_KIND;
// claim owner가 갱신하지 않은 채 이 시간을 넘기면 다른 worker가 stale claim으로 볼 수 있다.
const CLAIM_STALE_AFTER_SECS: i64 =
    crate::domain::parallel_mode::PARALLEL_DISPATCH_COMMAND_STALE_AFTER_SECS;
// task authority 문서 version을 metadata table에 저장할 때 쓰는 key이다.
const TASK_LEDGER_VERSION_METADATA_KEY: &str = "task_authority_version";
// Concurrent TUI, admin, Telegram, and automation writers share one repo-scoped database. A short
// SQLite wait absorbs ordinary transaction overlap without hiding a persistently wedged writer.
const AUTHORITY_STORE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const AUTHORITY_STORE_SIDECAR_SUFFIXES: [&str; 3] = ["-journal", "-wal", "-shm"];
const AUTHORITY_STORE_SIDECAR_IDENTITY_RETRIES: usize = 32;
#[derive(Default)]
/*
SQLite planning authority adapter의 값 타입이다.

필드를 갖지 않는 이유는 모든 상태가 repo-scoped authority DB 파일과 transaction 안에 있기 때문이다.
adapter 인스턴스는 connection pool이나 cache를 소유하지 않고, 호출마다 workspace에서 DB 위치를 해석해
connection을 연다. 그래서 `Default`와 `new()`는 단순한 생성자 역할만 한다.
*/
pub struct SqlitePlanningAuthorityAdapter;

pub use self::telegram_global_runner_lease::SqliteTelegramGlobalRunnerLeaseAdapter;

impl SqlitePlanningAuthorityAdapter {
    /*
    상태 없는 adapter 값을 만든다.

    application wiring에서는 구체 타입을 생성해 port trait object나 service dependency로 넘긴다. 이
    생성자는 그런 조립 지점에서 `Default::default()` 대신 명시적인 의도를 보여주기 위한 API이다.
    */
    pub fn new() -> Self {
        Self
    }

    pub(crate) fn load_review_center_thread_reviews_snapshot(
        workspace_dir: &str,
        thread_id: &str,
    ) -> Result<Vec<ReviewCenterThreadProjection>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_review_center_thread_reviews_rows(&connection, &location.workspace_root, thread_id)
    }

    pub(crate) fn load_review_center_pending_inbox_snapshot(
        workspace_dir: &str,
    ) -> Result<Vec<ReviewCenterInboxItem>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_review_center_pending_inbox_rows(&connection, &location.workspace_root)
    }

    pub(crate) fn load_review_center_recent_history_snapshot(
        workspace_dir: &str,
    ) -> Result<Vec<ReviewCenterHistoryEntry>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_review_center_recent_history_rows(&connection, &location.workspace_root)
    }

    pub(crate) fn upsert_review_center_thread_review(
        workspace_dir: &str,
        review: &ReviewCenterThreadProjection,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open review center thread review transaction")?;
        upsert_authority_metadata(
            &transaction,
            &location,
            "last_review_center_thread_review_updated_at",
        )?;
        upsert_review_center_thread_review_row(&transaction, &location.workspace_root, review)?;
        transaction
            .commit()
            .context("failed to commit review center thread review transaction")?;
        Ok(())
    }

    pub(crate) fn replace_review_center_pending_inbox(
        workspace_dir: &str,
        inbox: &[ReviewCenterInboxItem],
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open review center inbox transaction")?;
        upsert_authority_metadata(
            &transaction,
            &location,
            "last_review_center_inbox_updated_at",
        )?;
        replace_review_center_pending_inbox_rows(&transaction, &location.workspace_root, inbox)?;
        transaction
            .commit()
            .context("failed to commit review center inbox transaction")?;
        Ok(())
    }

    pub(crate) fn append_review_center_history_entry(
        workspace_dir: &str,
        entry: &ReviewCenterHistoryEntry,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open review center history transaction")?;
        upsert_authority_metadata(
            &transaction,
            &location,
            "last_review_center_history_appended_at",
        )?;
        append_review_center_history_row(&transaction, &location.workspace_root, entry)?;
        transaction
            .commit()
            .context("failed to commit review center history transaction")?;
        Ok(())
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
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "active workspace commit",
            None,
        )?;
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

    pub(crate) fn compare_and_swap_active_workspace_files(
        workspace_dir: &str,
        observed: &PlanningWorkspaceLoadRecord,
        replacement: &PlanningWorkspaceLoadRecord,
    ) -> Result<bool> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open authority-store active compare-and-swap transaction")?;
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "active workspace compare-and-swap",
            None,
        )?;
        if load_active_workspace_record(&transaction)? != *observed {
            return Ok(false);
        }
        let changed = apply_active_workspace_record(&transaction, replacement)?;
        if changed {
            upsert_authority_metadata(&transaction, &location, "last_active_compare_and_swap_at")?;
            bump_planning_revision(&transaction)?;
        }
        transaction
            .commit()
            .context("failed to commit authority-store active compare-and-swap transaction")?;
        Ok(true)
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

    pub(crate) fn load_task_authority_mutations(
        workspace_dir: &str,
        after_planning_revision: i64,
        through_planning_revision: i64,
    ) -> Result<Vec<PlanningTaskAuthorityMutationRecord>> {
        if through_planning_revision <= after_planning_revision {
            return Ok(Vec::new());
        }
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        let mut statement = connection
            .prepare(
                "SELECT content_json FROM planning_task_mutation_events
                 WHERE planning_revision > ?1 AND planning_revision <= ?2
                 ORDER BY planning_revision ASC, event_order ASC",
            )
            .context("failed to prepare planning task mutation event load")?;
        let rows = statement
            .query_map(
                params![after_planning_revision, through_planning_revision],
                |row| row.get::<_, String>(0),
            )
            .context("failed to query planning task mutation events")?;
        let mut records = Vec::new();
        for row in rows {
            let content_json = row.context("failed to decode planning task mutation event")?;
            records.push(
                serde_json::from_str(&content_json)
                    .context("failed to parse planning task mutation event")?,
            );
        }
        Ok(records)
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
    direction/task authority와 accepted result output을 같은 authority read snapshot으로 읽는다.

    admin operator surface는 direction/task/result-output을 같은 시점의 accepted state로 봐야 한다. 이 helper는
    하나의 SQLite transaction 안에서 세 표면을 읽고, direction/task revision이 어긋나면 mixed snapshot으로 보지 않고
    reload 오류를 돌린다.
    */
    pub(crate) fn load_planning_authority_documents(
        workspace_dir: &str,
    ) -> Result<Option<PlanningAuthorityDocumentSnapshot>> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open planning authority document load transaction")?;
        let direction_snapshot = load_direction_authority_snapshot_from_connection(&transaction)?;
        let task_snapshot = load_task_authority_snapshot_from_connection(&transaction)?;
        let workspace_record = load_active_workspace_record(&transaction)?;
        match (direction_snapshot, task_snapshot) {
            (Some(direction_snapshot), Some(task_snapshot)) => {
                if direction_snapshot.planning_revision != task_snapshot.planning_revision {
                    return Err(anyhow!(
                        "planning authority changed while loading direction/task snapshots (direction revision {:?}, task revision {:?}); reload and retry",
                        Some(direction_snapshot.planning_revision),
                        Some(task_snapshot.planning_revision)
                    ));
                }
                let Some(result_output_markdown) = workspace_record.result_output_markdown else {
                    return Ok(None);
                };
                Ok(Some(PlanningAuthorityDocumentSnapshot {
                    planning_revision: task_snapshot.planning_revision,
                    directions: direction_snapshot.directions,
                    task_authority: task_snapshot.task_authority,
                    result_output_markdown,
                }))
            }
            (None, None) => Ok(None),
            (Some(_), None) => Err(anyhow!(
                "accepted planning authority is missing task authority"
            )),
            (None, Some(_)) => Err(anyhow!(
                "accepted planning authority is missing direction authority"
            )),
        }
    }
    /*
    direction/task authority와 operator-facing result output을 한 transaction으로 함께 commit한다.

    admin 문서 commit처럼 direction/task/result-output을 한 편집 세션에서 같이 저장할 때는,
    DB authority와 active result document가 서로 다른 revision으로 갈라지면 안 된다. 이 helper는 세 표면이
    이미 target과 같은지 먼저 비교하고, 실제 변경이 필요할 때만 같은 SQLite transaction에서 direction rows,
    task rows, active document, planning revision을 함께 갱신한다.
    */
    pub(crate) fn commit_planning_authority_documents(
        workspace_dir: &str,
        commit: PlanningAuthorityDocumentCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open planning authority document commit transaction")?;
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "planning authority document commit",
            commit.authority_mutation_owner_token,
        )?;
        let current_revision = read_metadata_i64(&transaction, "planning_revision")?.unwrap_or(0);
        if let Some(observed_revision) = commit.observed_planning_revision
            && observed_revision != current_revision
        {
            return Ok(PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: observed_revision,
                current_planning_revision: current_revision,
            });
        }
        for mutation in commit.active_document_mutations {
            match mutation {
                PlanningAuthorityActiveDocumentMutation::Replace { relative_path, .. }
                | PlanningAuthorityActiveDocumentMutation::RemoveEntry { relative_path } => {
                    validate_authority_document_mutation_path(relative_path)?;
                }
                PlanningAuthorityActiveDocumentMutation::ClearStagedDrafts => {}
            }
        }
        for task_id in commit.retired_task_ids {
            if commit
                .task_authority
                .tasks
                .iter()
                .any(|task| task.id.trim() == task_id.trim())
            {
                anyhow::bail!(
                    "retired planning task `{}` is still present in the committed authority",
                    task_id.trim()
                );
            }
        }
        runtime_projection::retire_task_runtime_projections(&transaction, commit.retired_task_ids)?;
        let direction_unchanged = load_direction_authority_snapshot_from_connection(&transaction)?
            .as_ref()
            .map(|snapshot| &snapshot.directions)
            == Some(commit.directions);
        let existing_task_snapshot = load_task_authority_snapshot_from_connection(&transaction)?;
        let task_unchanged = existing_task_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.task_authority == *commit.task_authority
                && snapshot.queue_projection == *commit.queue_projection
        });
        let mut active_changed = match commit.result_output_markdown {
            Some(result_output_markdown) => apply_active_workspace_record(
                &transaction,
                &PlanningWorkspaceLoadRecord {
                    result_output_markdown: Some(result_output_markdown.to_string()),
                },
            )?,
            None => false,
        };
        for mutation in commit.active_document_mutations {
            active_changed |= match mutation {
                PlanningAuthorityActiveDocumentMutation::Replace {
                    relative_path,
                    body,
                } => set_active_document(&transaction, relative_path, Some(body))?,
                PlanningAuthorityActiveDocumentMutation::RemoveEntry { relative_path } => {
                    remove_active_documents(&transaction, relative_path)?
                }
                PlanningAuthorityActiveDocumentMutation::ClearStagedDrafts => {
                    clear_staged_drafts(&transaction)?
                }
            };
        }
        if direction_unchanged
            && task_unchanged
            && !active_changed
            && commit.retired_task_ids.is_empty()
        {
            return Ok(PlanningTaskAuthorityCommitResult::Committed {
                planning_revision: current_revision,
                changed: false,
            });
        }

        upsert_authority_metadata(
            &transaction,
            &location,
            "last_direction_authority_commit_at",
        )?;
        upsert_authority_metadata(&transaction, &location, "last_task_authority_commit_at")?;
        upsert_authority_metadata(&transaction, &location, "last_active_commit_at")?;
        if !direction_unchanged {
            replace_direction_authority_tables(&transaction, commit.directions)?;
        }
        if !task_unchanged {
            replace_task_authority_tables(
                &transaction,
                commit.task_authority,
                commit.queue_projection,
            )?;
        }
        let planning_revision = bump_planning_revision(&transaction)?;
        if !task_unchanged {
            insert_task_authority_mutation_records(
                &transaction,
                &task_authority_mutation_records(
                    existing_task_snapshot
                        .as_ref()
                        .map(|snapshot| &snapshot.task_authority),
                    commit.task_authority,
                    planning_revision,
                    None,
                ),
            )?;
        }
        transaction
            .commit()
            .context("failed to commit planning authority document transaction")?;

        Ok(PlanningTaskAuthorityCommitResult::Committed {
            planning_revision,
            changed: true,
        })
    }
    /*
    direction/task authority를 한 transaction으로 함께 commit한다.

    admin 문서 commit처럼 direction/task/result-output을 한 편집 세션에서 같이 저장할 때는,
    direction commit과 task commit이 서로 다른 revision으로 갈라지면 안 된다. 이 helper는 두 문서가 이미
    target과 같은지 먼저 비교하고, 실제 변경이 필요할 때만 같은 SQLite transaction에서 direction rows,
    task rows, queue projection, planning revision을 함께 갱신한다.
    */
    pub(crate) fn commit_planning_authority_snapshot(
        workspace_dir: &str,
        commit: PlanningAuthoritySnapshotCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open planning authority combined commit transaction")?;
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "planning authority combined commit",
            None,
        )?;
        let current_revision = read_metadata_i64(&transaction, "planning_revision")?.unwrap_or(0);
        if let Some(observed_revision) = commit.observed_planning_revision
            && observed_revision != current_revision
        {
            return Ok(PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: observed_revision,
                current_planning_revision: current_revision,
            });
        }
        let direction_unchanged = load_direction_authority_snapshot_from_connection(&transaction)?
            .as_ref()
            .map(|snapshot| &snapshot.directions)
            == Some(commit.directions);
        let existing_task_snapshot = load_task_authority_snapshot_from_connection(&transaction)?;
        let task_unchanged = existing_task_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.task_authority == *commit.task_authority
                && snapshot.queue_projection == *commit.queue_projection
        });
        if direction_unchanged && task_unchanged {
            return Ok(PlanningTaskAuthorityCommitResult::Committed {
                planning_revision: current_revision,
                changed: false,
            });
        }

        upsert_authority_metadata(
            &transaction,
            &location,
            "last_direction_authority_commit_at",
        )?;
        upsert_authority_metadata(&transaction, &location, "last_task_authority_commit_at")?;
        if !direction_unchanged {
            replace_direction_authority_tables(&transaction, commit.directions)?;
        }
        if !task_unchanged {
            replace_task_authority_tables(
                &transaction,
                commit.task_authority,
                commit.queue_projection,
            )?;
        }
        let planning_revision = bump_planning_revision(&transaction)?;
        if !task_unchanged {
            insert_task_authority_mutation_records(
                &transaction,
                &task_authority_mutation_records(
                    existing_task_snapshot
                        .as_ref()
                        .map(|snapshot| &snapshot.task_authority),
                    commit.task_authority,
                    planning_revision,
                    None,
                ),
            )?;
        }
        transaction
            .commit()
            .context("failed to commit planning authority combined transaction")?;

        Ok(PlanningTaskAuthorityCommitResult::Committed {
            planning_revision,
            changed: true,
        })
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
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "direction authority commit",
            commit.authority_mutation_owner_token,
        )?;
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
                changed: false,
            });
        }

        upsert_authority_metadata(
            &transaction,
            &location,
            "last_direction_authority_commit_at",
        )?;
        let existing_task_snapshot = load_task_authority_snapshot_from_connection(&transaction)?;
        replace_direction_authority_tables(&transaction, commit.directions)?;
        reconcile_task_authority_with_directions(&transaction, Some(commit.directions))?;
        let planning_revision = bump_planning_revision(&transaction)?;
        let reconciled_task_snapshot = load_task_authority_snapshot_from_connection(&transaction)?;
        let reconciled_task_authority = reconciled_task_snapshot
            .as_ref()
            .map(|snapshot| snapshot.task_authority.clone())
            .unwrap_or(TaskAuthorityDocument {
                version: 1,
                tasks: Vec::new(),
            });
        insert_task_authority_mutation_records(
            &transaction,
            &task_authority_mutation_records(
                existing_task_snapshot
                    .as_ref()
                    .map(|snapshot| &snapshot.task_authority),
                &reconciled_task_authority,
                planning_revision,
                None,
            ),
        )?;
        transaction
            .commit()
            .context("failed to commit direction authority transaction")?;

        Ok(PlanningTaskAuthorityCommitResult::Committed {
            planning_revision,
            changed: true,
        })
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
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "direction authority clear",
            None,
        )?;
        upsert_authority_metadata(
            &transaction,
            &location,
            "last_direction_authority_commit_at",
        )?;
        let existing_task_snapshot = load_task_authority_snapshot_from_connection(&transaction)?;
        clear_direction_authority_tables(&transaction)?;
        reconcile_task_authority_with_directions(&transaction, None)?;
        let planning_revision = bump_planning_revision(&transaction)?;
        let reconciled_task_snapshot = load_task_authority_snapshot_from_connection(&transaction)?;
        let reconciled_task_authority = reconciled_task_snapshot
            .as_ref()
            .map(|snapshot| snapshot.task_authority.clone())
            .unwrap_or(TaskAuthorityDocument {
                version: 1,
                tasks: Vec::new(),
            });
        insert_task_authority_mutation_records(
            &transaction,
            &task_authority_mutation_records(
                existing_task_snapshot
                    .as_ref()
                    .map(|snapshot| &snapshot.task_authority),
                &reconciled_task_authority,
                planning_revision,
                None,
            ),
        )?;
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
        Self::commit_task_authority_snapshot_with_audit(workspace_dir, commit, None)
    }

    pub(crate) fn commit_task_authority_mutation_snapshot(
        workspace_dir: &str,
        commit: PlanningTaskAuthorityCommit<'_>,
        audit: PlanningTaskAuthorityMutationAudit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        Self::commit_task_authority_snapshot_with_audit(workspace_dir, commit, Some(audit))
    }

    fn commit_task_authority_snapshot_with_audit(
        workspace_dir: &str,
        commit: PlanningTaskAuthorityCommit<'_>,
        audit: Option<PlanningTaskAuthorityMutationAudit<'_>>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;

        let transaction = connection
            .transaction()
            .context("failed to open task authority commit transaction")?;
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "task authority commit",
            None,
        )?;
        let current_revision = read_metadata_i64(&transaction, "planning_revision")?.unwrap_or(0);
        if let Some(observed_revision) = commit.observed_planning_revision
            && observed_revision != current_revision
        {
            return Ok(PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: observed_revision,
                current_planning_revision: current_revision,
            });
        }
        let existing_snapshot = load_task_authority_snapshot_from_connection(&transaction)?;
        if existing_snapshot.as_ref().is_some_and(|existing_snapshot| {
            existing_snapshot.task_authority == *commit.task_authority
                && existing_snapshot.queue_projection == *commit.queue_projection
        }) {
            return Ok(PlanningTaskAuthorityCommitResult::Committed {
                planning_revision: current_revision,
                changed: false,
            });
        }

        upsert_authority_metadata(&transaction, &location, "last_task_authority_commit_at")?;
        replace_task_authority_tables(
            &transaction,
            commit.task_authority,
            commit.queue_projection,
        )?;
        let planning_revision = bump_planning_revision(&transaction)?;
        let records = task_authority_mutation_records(
            existing_snapshot
                .as_ref()
                .map(|snapshot| &snapshot.task_authority),
            commit.task_authority,
            planning_revision,
            audit,
        );
        insert_task_authority_mutation_records(&transaction, &records)?;
        transaction
            .commit()
            .context("failed to commit task authority transaction")?;

        Ok(PlanningTaskAuthorityCommitResult::Committed {
            planning_revision,
            changed: true,
        })
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
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "task authority clear",
            None,
        )?;
        upsert_authority_metadata(&transaction, &location, "last_task_authority_commit_at")?;
        clear_task_authority_tables(&transaction)?;
        transaction
            .execute("DELETE FROM planning_task_mutation_events", [])
            .context("failed to clear planning task mutation events")?;
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
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "active planning file replacement",
            None,
        )?;
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
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "active planning entry removal",
            None,
        )?;
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

fn validate_authority_document_mutation_path(relative_path: &str) -> Result<()> {
    let normalized = relative_path.trim();
    let planning_root = ".codex-exec-loop/planning/";
    let looks_like_windows_absolute = normalized.as_bytes().get(1) == Some(&b':');
    if normalized != relative_path
        || !normalized.starts_with(planning_root)
        || normalized.len() == planning_root.len()
        || normalized.starts_with('/')
        || normalized.contains('\\')
        || looks_like_windows_absolute
        || Path::new(normalized)
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        anyhow::bail!("invalid planning authority document mutation path `{relative_path}`");
    }
    Ok(())
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
impl ParallelModeRuntimeEventLogPort for SqlitePlanningAuthorityAdapter {
    fn load_runtime_event_log(
        &self,
        workspace_dir: &str,
        request: ParallelModeRuntimeEventLogRequest,
    ) -> Result<ParallelModeRuntimeEventsSnapshot> {
        Self::load_runtime_event_log(workspace_dir, request)
    }
}

impl PlanningAuthorityPort for SqlitePlanningAuthorityAdapter {
    fn current_process_claim_identity(&self) -> Result<(u32, String)> {
        let process_id = std::process::id();
        let start_identity = crate::process_liveness::required_process_start_identity(process_id)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok((process_id, start_identity))
    }

    fn supports_atomic_planning_authority_documents(&self) -> bool {
        true
    }

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

    fn load_planning_authority_documents(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<PlanningAuthorityDocumentSnapshot>> {
        Self::load_planning_authority_documents(workspace_dir)
    }

    fn commit_planning_authority_documents(
        &self,
        workspace_dir: &str,
        commit: PlanningAuthorityDocumentCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        Self::commit_planning_authority_documents(workspace_dir, commit)
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

    fn renew_official_refresh_claim(
        &self,
        workspace_dir: &str,
        refresh_order: u64,
        owner_token: &str,
    ) -> Result<bool> {
        Self::renew_official_refresh_claim(workspace_dir, refresh_order, owner_token)
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
    실패하거나 supersede된 refresh는 같은 order를 다시 실행할 수 있어야 한다. cancel은 exact owner/order
    claim만 지우고 실행 포인터는 전진시키지 않는다.
    */
    fn cancel_official_refresh_claim(
        &self,
        workspace_dir: &str,
        refresh_order: u64,
        owner_token: &str,
    ) -> Result<()> {
        Self::cancel_official_refresh_claim(workspace_dir, refresh_order, owner_token)
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
    현재 owner가 보유한 distributor queue claim의 lease 시각을 갱신한다.

    bool은 kind/scope/owner가 모두 일치하는 row를 실제로 갱신했는지 나타내므로, caller는 stale 회수로
    소유권이 교체된 뒤 늦게 도착한 heartbeat를 안전하게 중단할 수 있다.
    */
    fn renew_distributor_queue_claim(
        &self,
        workspace_dir: &str,
        queue_item_id: &str,
        owner_token: &str,
    ) -> Result<bool> {
        Self::renew_distributor_queue_claim(workspace_dir, queue_item_id, owner_token)
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
    parallel runtime projection row 묶음을 일관된 snapshot으로 로드한다.

    slot lease, invalid lease, session detail, distributor queue, runtime event projection을 한 번에 읽는 port
    표면이다. 구체적인 table join/JSON decode는 runtime projection 모듈이 담당한다.
    */
    fn load_runtime_projections(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        Self::load_runtime_projections(workspace_dir)
    }

    fn acquire_admin_task_mutation_guard(
        &self,
        workspace_dir: &str,
        task_ids: &[String],
        owner_token: &str,
    ) -> Result<()> {
        Self::acquire_admin_task_mutation_guard(workspace_dir, task_ids, owner_token)
    }

    fn release_admin_task_mutation_guard(
        &self,
        workspace_dir: &str,
        task_ids: &[String],
        owner_token: &str,
    ) -> Result<()> {
        Self::release_admin_task_mutation_guard(workspace_dir, task_ids, owner_token)
    }

    fn acquire_admin_authority_mutation_guard(
        &self,
        workspace_dir: &str,
        owner_token: &str,
        action: &str,
    ) -> Result<()> {
        Self::acquire_admin_authority_mutation_guard(workspace_dir, owner_token, action)
    }

    fn release_admin_authority_mutation_guard(
        &self,
        workspace_dir: &str,
        owner_token: &str,
    ) -> Result<()> {
        Self::release_admin_authority_mutation_guard(workspace_dir, owner_token)
    }

    fn enqueue_runtime_dispatch_command(
        &self,
        workspace_dir: &str,
        command: &ParallelModeDispatchCommandSnapshot,
    ) -> Result<bool> {
        Self::enqueue_runtime_dispatch_command(workspace_dir, command)
    }

    fn try_claim_next_runtime_dispatch_command(
        &self,
        workspace_dir: &str,
        owner_token: &str,
    ) -> Result<Option<ParallelModeDispatchCommandSnapshot>> {
        Self::try_claim_next_runtime_dispatch_command(workspace_dir, owner_token)
    }

    fn update_runtime_dispatch_command(
        &self,
        workspace_dir: &str,
        command: &ParallelModeDispatchCommandSnapshot,
    ) -> Result<()> {
        Self::update_runtime_dispatch_command(workspace_dir, command)
    }

    fn cancel_runtime_dispatch_commands(&self, workspace_dir: &str, reason: &str) -> Result<usize> {
        Self::cancel_runtime_dispatch_commands(workspace_dir, reason)
    }

    fn clear_parallel_runtime_projections(&self, workspace_dir: &str, reason: &str) -> Result<()> {
        Self::clear_parallel_runtime_projections(workspace_dir, reason)
    }

    fn clear_parallel_runtime_projections_for_tasks(
        &self,
        workspace_dir: &str,
        task_ids: &[String],
        reason: &str,
    ) -> Result<()> {
        Self::clear_parallel_runtime_projections_for_tasks(workspace_dir, task_ids, reason)
    }

    fn apply_parallel_pool_reset_report(
        &self,
        workspace_dir: &str,
        report: &ParallelModePoolResetReport,
    ) -> Result<()> {
        Self::apply_parallel_pool_reset_report(workspace_dir, report)
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

    fn remove_runtime_slot_lease_if_matches(
        &self,
        workspace_dir: &str,
        expected: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<bool> {
        Self::remove_runtime_slot_lease_if_matches(workspace_dir, expected)
    }

    fn replace_runtime_slot_lease_if_matches(
        &self,
        workspace_dir: &str,
        expected_current: &ParallelModeSlotLeaseSnapshot,
        replacement: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<bool> {
        Self::replace_runtime_slot_lease_if_matches(workspace_dir, expected_current, replacement)
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

    fn load_runtime_pr_validation_records(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<PrValidationRecord>> {
        Self::load_runtime_pr_validation_records(workspace_dir)
    }

    fn load_due_runtime_pr_validation_record_keys(
        &self,
        workspace_dir: &str,
        due_at: DateTime<Utc>,
        repository_cooldown_since: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<PrValidationRecordKey>> {
        Self::load_due_runtime_pr_validation_record_keys(
            workspace_dir,
            due_at,
            repository_cooldown_since,
            limit,
        )
    }

    fn try_claim_runtime_pr_validation_poll(
        &self,
        workspace_dir: &str,
        request: PrValidationPollLeaseClaimRequest<'_>,
    ) -> Result<Option<PrValidationPollLeaseClaim>> {
        Self::try_claim_runtime_pr_validation_poll(workspace_dir, request)
    }

    fn renew_runtime_pr_validation_poll_lease(
        &self,
        workspace_dir: &str,
        request: PrValidationPollLeaseRenewalRequest<'_>,
    ) -> Result<bool> {
        Self::renew_runtime_pr_validation_poll_lease(workspace_dir, request)
    }

    fn settle_runtime_pr_validation_poll(
        &self,
        workspace_dir: &str,
        record_key: &PrValidationRecordKey,
        owner: &str,
        token: &str,
        expected_expires_at: DateTime<Utc>,
        settlement: &PrValidationPollSettlement,
    ) -> Result<bool> {
        Self::settle_runtime_pr_validation_poll(
            workspace_dir,
            record_key,
            owner,
            token,
            expected_expires_at,
            settlement,
        )
    }

    fn load_runtime_pr_validation_record(
        &self,
        workspace_dir: &str,
        record_key: &PrValidationRecordKey,
    ) -> Result<Option<PrValidationRecord>> {
        Self::load_runtime_pr_validation_record(workspace_dir, record_key)
    }

    fn load_runtime_pr_validation_record_for_pr(
        &self,
        workspace_dir: &str,
        pull_request_number: u64,
    ) -> Result<Option<PrValidationRecord>> {
        Self::load_runtime_pr_validation_record_for_pr(workspace_dir, pull_request_number)
    }

    fn load_runtime_pr_validation_record_for_remediation(
        &self,
        workspace_dir: &str,
        task_id: &str,
    ) -> Result<Option<PrValidationRecord>> {
        Self::load_runtime_pr_validation_record_for_remediation(workspace_dir, task_id)
    }

    fn compare_and_swap_runtime_pr_validation_record(
        &self,
        workspace_dir: &str,
        record_key: &PrValidationRecordKey,
        expected: Option<&PrValidationRecord>,
        replacement: Option<&PrValidationRecord>,
    ) -> Result<bool> {
        Self::compare_and_swap_runtime_pr_validation_record(
            workspace_dir,
            record_key,
            expected,
            replacement,
        )
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

    fn commit_planning_authority_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningAuthoritySnapshotCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        Self::commit_planning_authority_snapshot(workspace_dir, commit)
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

    fn commit_task_authority_mutation_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningTaskAuthorityCommit<'_>,
        audit: PlanningTaskAuthorityMutationAudit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        Self::commit_task_authority_mutation_snapshot(workspace_dir, commit, audit)
    }

    fn load_task_authority_mutations(
        &self,
        workspace_dir: &str,
        after_planning_revision: i64,
        through_planning_revision: i64,
    ) -> Result<Vec<PlanningTaskAuthorityMutationRecord>> {
        Self::load_task_authority_mutations(
            workspace_dir,
            after_planning_revision,
            through_planning_revision,
        )
    }

    // task authority 제거 port를 task rows/edges/projection clear helper에 연결한다.
    fn clear_task_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        Self::clear_task_authority_snapshot(workspace_dir)
    }
}

/*
application의 `ReviewCenterRepositoryPort`를 같은 SQLite authority DB 구현에 연결한다.

review center는 planning authority와 별도 논리 port이지만, 같은 repo-scoped SQLite 파일에 thread review,
inbox, history projection을 저장한다. 이 impl은 review center read/write 계약을 현재 process의 canonical
repository authority DB로 연결해 TUI/admin/telegram이 같은 projection을 보게 한다.
*/
impl ReviewCenterRepositoryPort for SqlitePlanningAuthorityAdapter {
    fn load_thread_reviews(
        &self,
        workspace_dir: &str,
        thread_id: &str,
    ) -> Result<Vec<ReviewCenterThreadProjection>> {
        Self::load_review_center_thread_reviews_snapshot(workspace_dir, thread_id)
    }

    fn load_pending_inbox(&self, workspace_dir: &str) -> Result<Vec<ReviewCenterInboxItem>> {
        Self::load_review_center_pending_inbox_snapshot(workspace_dir)
    }

    fn load_recent_history(&self, workspace_dir: &str) -> Result<Vec<ReviewCenterHistoryEntry>> {
        Self::load_review_center_recent_history_snapshot(workspace_dir)
    }

    fn upsert_thread_review(
        &self,
        workspace_dir: &str,
        review: &ReviewCenterThreadProjection,
    ) -> Result<()> {
        Self::upsert_review_center_thread_review(workspace_dir, review)
    }

    fn replace_pending_inbox(
        &self,
        workspace_dir: &str,
        inbox: &[ReviewCenterInboxItem],
    ) -> Result<()> {
        Self::replace_review_center_pending_inbox(workspace_dir, inbox)
    }

    fn append_history_entry(
        &self,
        workspace_dir: &str,
        entry: &ReviewCenterHistoryEntry,
    ) -> Result<()> {
        Self::append_review_center_history_entry(workspace_dir, entry)
    }
}
/*
authority DB connection을 열고, 모든 caller가 의존하는 기본 DB 상태를 보장한다.

이 함수는 단순한 `Connection::open` wrapper가 아니다. repo-scoped authority DB의 진입점으로서 다음
순서를 항상 지킨다.
1. private parent, main DB, 기존 SQLite sidecar의 identity와 권한을 고정한다.
2. SQLite connection을 열고 store marker가 현재 repository와 일치하는지 검사한다.
3. legacy WAL을 안전하게 checkpoint하고 DELETE rollback journal mode로 고정한다.
4. foreign key, secure-delete, memory temp-store 정책을 켠다.
5. 현재 schema가 필요로 하는 table/index를 보장하고 file family를 다시 검사한다.

이 순서가 중요하다. schema를 만들기 전에 version gate를 통과해야 오래된/미래 schema를 잘못 덮어쓰지
않고, foreign key pragma는 task edge/draft file 같은 자식 row 정합성을 DB 차원에서 지키게 한다.
*/
fn open_authority_connection(location: &PlanningAuthorityLocation) -> Result<Connection> {
    let authority_store_path = Path::new(&location.authority_store_path);
    let parent = authority_store_path.parent().ok_or_else(|| {
        anyhow!(
            "authority-store path has no parent: {}",
            authority_store_path.display()
        )
    })?;
    let _directory_anchors = prepare_private_authority_directory_tree(parent)?;
    let store_anchor = prepare_private_authority_store_file(authority_store_path)?;
    let _sidecar_anchors = prepare_private_authority_sidecar_files(authority_store_path)?;

    let open_flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let mut connection = Connection::open_with_flags(authority_store_path, open_flags)
        .with_context(|| format!("failed to open {}", authority_store_path.display()))?;
    connection
        .busy_timeout(AUTHORITY_STORE_BUSY_TIMEOUT)
        .context("failed to configure authority-store busy timeout")?;
    validate_authority_store_path_identity(authority_store_path, &store_anchor)?;
    validate_authority_store_schema(&connection, location)?;
    configure_private_authority_connection(&connection)?;
    let _current_sidecar_anchors = prepare_private_authority_sidecar_files(authority_store_path)?;
    ensure_schema(&mut connection, location)?;
    validate_authority_store_path_identity(authority_store_path, &store_anchor)?;
    let _final_sidecar_anchors = prepare_private_authority_sidecar_files(authority_store_path)?;
    Ok(connection)
}

fn configure_private_authority_connection(connection: &Connection) -> Result<()> {
    let journal_mode = connection
        .query_row("PRAGMA journal_mode = DELETE", [], |row| {
            row.get::<_, String>(0)
        })
        .context("failed to configure authority-store rollback journal mode")?;
    if !journal_mode.eq_ignore_ascii_case("delete") {
        return Err(anyhow!(
            "authority-store must use DELETE rollback journaling, but SQLite retained `{journal_mode}`"
        ));
    }
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON; PRAGMA temp_store = MEMORY;",
        )
        .context("failed to enable authority-store safety pragmas")?;
    Ok(())
}

fn prepare_private_authority_sidecar_files(path: &Path) -> Result<Vec<File>> {
    let mut anchors = Vec::new();
    for suffix in AUTHORITY_STORE_SIDECAR_SUFFIXES {
        let sidecar = authority_store_sidecar_path(path, suffix);
        let mut anchored = None;
        for attempt in 0..AUTHORITY_STORE_SIDECAR_IDENTITY_RETRIES {
            match fs::symlink_metadata(&sidecar) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => {
                    #[cfg(windows)]
                    if windows_sidecar_io_error_is_transient(&error) {
                        if attempt + 1 < AUTHORITY_STORE_SIDECAR_IDENTITY_RETRIES {
                            authority_sidecar_identity_retry_delay();
                            continue;
                        }
                        return Err(anyhow!(
                            "authority-store sidecar identity kept changing while inspecting: {}",
                            sidecar.display()
                        ));
                    }
                    return Err(error).with_context(|| {
                        format!(
                            "failed to inspect authority-store sidecar {}",
                            sidecar.display()
                        )
                    });
                }
            }

            match prepare_existing_private_authority_sidecar_file(&sidecar)? {
                Some(anchor) => {
                    anchored = Some(anchor);
                    break;
                }
                None if attempt + 1 < AUTHORITY_STORE_SIDECAR_IDENTITY_RETRIES => {
                    authority_sidecar_identity_retry_delay();
                }
                None => {
                    return Err(anyhow!(
                        "authority-store sidecar identity kept changing while opening: {}",
                        sidecar.display()
                    ));
                }
            }
        }
        if let Some(anchor) = anchored {
            anchors.push(anchor);
        }
    }
    Ok(anchors)
}

fn authority_sidecar_identity_retry_delay() {
    std::thread::sleep(Duration::from_millis(1));
}

fn authority_store_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut sidecar = path.as_os_str().to_os_string();
    sidecar.push(suffix);
    PathBuf::from(sidecar)
}

fn prepare_existing_private_authority_sidecar_file(path: &Path) -> Result<Option<File>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let sidecar = match OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
        {
            Ok(sidecar) => sidecar,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to securely open {}", path.display()));
            }
        };
        secure_opened_private_authority_sidecar_file(path, sidecar)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        let sidecar = match OpenOptions::new()
            .read(true)
            .write(true)
            .access_mode(
                WINDOWS_GENERIC_READ
                    | WINDOWS_GENERIC_WRITE
                    | WINDOWS_READ_CONTROL
                    | WINDOWS_WRITE_DAC,
            )
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
        {
            Ok(sidecar) => sidecar,
            Err(error) if windows_sidecar_io_error_is_transient(&error) => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to securely open {}", path.display()));
            }
        };
        secure_opened_private_authority_sidecar_file(path, sidecar)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(anyhow!(
            "secure authority-store sidecars are unsupported on this platform: {}",
            path.display()
        ))
    }
}

#[cfg(unix)]
fn secure_opened_private_authority_sidecar_file(
    path: &Path,
    sidecar: File,
) -> Result<Option<File>> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = sidecar
        .metadata()
        .with_context(|| format!("failed to inspect opened sidecar {}", path.display()))?;
    if metadata.nlink() == 0 {
        return Ok(None);
    }
    validate_owned_private_store_metadata(path, &metadata)?;
    if !authority_sidecar_path_matches_opened_identity(path, &metadata)? {
        return Ok(None);
    }
    sidecar
        .set_permissions(fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure {}", path.display()))?;
    let secured = sidecar
        .metadata()
        .with_context(|| format!("failed to verify permissions for {}", path.display()))?;
    if secured.nlink() == 0 {
        return Ok(None);
    }
    validate_owned_private_store_metadata(path, &secured)?;
    if secured.mode() & 0o777 != 0o600 {
        return Err(anyhow!(
            "authority-store sidecar permissions are not private: {}",
            path.display()
        ));
    }
    if !authority_sidecar_path_matches_opened_identity(path, &secured)? {
        return Ok(None);
    }
    Ok(Some(sidecar))
}

#[cfg(unix)]
fn authority_sidecar_path_matches_opened_identity(
    path: &Path,
    opened: &fs::Metadata,
) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let path_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to inspect path identity for {}", path.display())
            });
        }
    };
    if path_metadata.nlink() == 0 {
        return Ok(false);
    }
    if path_metadata.file_type().is_symlink()
        || !path_metadata.is_file()
        || path_metadata.uid() != unsafe { libc::geteuid() }
        || path_metadata.nlink() != 1
    {
        return Err(anyhow!(
            "authority-store file must be a current-user-owned regular file with one link: {}",
            path.display()
        ));
    }
    Ok(path_metadata.dev() == opened.dev() && path_metadata.ino() == opened.ino())
}

#[cfg(windows)]
fn secure_opened_private_authority_sidecar_file(
    path: &Path,
    sidecar: File,
) -> Result<Option<File>> {
    if !authority_windows_sidecar_path_matches_opened_identity(path, &sidecar)? {
        return Ok(None);
    }
    if let Err(error) = set_windows_private_acl(&sidecar) {
        if windows_sidecar_anyhow_error_is_transient(&error) {
            return Ok(None);
        }
        return Err(error);
    }
    if let Err(error) = validate_windows_private_owner_and_acl(path, &sidecar) {
        if windows_sidecar_anyhow_error_is_transient(&error) {
            return Ok(None);
        }
        return Err(error);
    }
    if !authority_windows_sidecar_path_matches_opened_identity(path, &sidecar)? {
        return Ok(None);
    }
    Ok(Some(sidecar))
}

#[cfg(windows)]
fn authority_windows_sidecar_path_matches_opened_identity(
    path: &Path,
    opened: &File,
) -> Result<bool> {
    use std::os::windows::fs::OpenOptionsExt;

    let Some(opened_identity) = windows_sidecar_file_identity(opened)? else {
        return Ok(false);
    };
    if !validate_windows_sidecar_handle_identity(path, opened, &opened_identity)? {
        return Ok(false);
    }
    if opened_identity.number_of_links == 0 {
        return Ok(false);
    }
    let path_handle = match OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    {
        Ok(path_handle) => path_handle,
        Err(error) if windows_sidecar_io_error_is_transient(&error) => return Ok(false),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to reopen Windows path {}", path.display()));
        }
    };
    let Some(current_opened_identity) = windows_sidecar_file_identity(opened)? else {
        return Ok(false);
    };
    if !validate_windows_sidecar_handle_identity(path, opened, &current_opened_identity)? {
        return Ok(false);
    }
    let Some(path_identity) = windows_sidecar_file_identity(&path_handle)? else {
        return Ok(false);
    };
    if !validate_windows_sidecar_handle_identity(path, &path_handle, &path_identity)? {
        return Ok(false);
    }
    if current_opened_identity.number_of_links == 0 || path_identity.number_of_links == 0 {
        return Ok(false);
    }
    Ok(
        current_opened_identity.volume_serial_number == path_identity.volume_serial_number
            && current_opened_identity.file_index == path_identity.file_index,
    )
}

#[cfg(windows)]
fn validate_windows_sidecar_handle_identity(
    path: &Path,
    opened: &File,
    identity: &WindowsFileIdentity,
) -> Result<bool> {
    if identity.attributes & WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT != 0
        || identity.attributes & WINDOWS_FILE_ATTRIBUTE_DIRECTORY != 0
        || identity.number_of_links > 1
    {
        return Err(anyhow!(
            "authority-store Windows sidecar must be a non-reparse, single-link file: {}",
            path.display()
        ));
    }
    match validate_windows_owner(path, opened) {
        Ok(()) => Ok(true),
        Err(error) if windows_sidecar_anyhow_error_is_transient(&error) => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn windows_sidecar_file_identity(file: &File) -> Result<Option<WindowsFileIdentity>> {
    match windows_file_identity(file) {
        Ok(identity) => Ok(Some(identity)),
        Err(error) if windows_sidecar_anyhow_error_is_transient(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn windows_sidecar_io_error_is_transient(error: &std::io::Error) -> bool {
    use windows_sys::Win32::Foundation::ERROR_DELETE_PENDING;

    error.kind() == std::io::ErrorKind::NotFound
        || error.raw_os_error() == Some(ERROR_DELETE_PENDING as i32)
}

#[cfg(windows)]
fn windows_sidecar_anyhow_error_is_transient(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(windows_sidecar_io_error_is_transient)
}

/*
authority store가 속한 관리 디렉터리를 위에서 아래 순서로 준비한다.

`create_dir_all(runtime)`을 바로 호출하면 중간 component가 symlink일 때 링크 대상 안에 directory를
만드는 부작용이 먼저 발생한다. 관리 root만 만든 뒤 각 component를 `symlink_metadata`로 검사하고
하나씩 생성하면 고정된 parent symlink는 링크 대상에 손대기 전에 거부할 수 있다. Unix에서는 각
directory를 `O_NOFOLLOW|O_DIRECTORY`로 열어 owner와 inode를 확인한 뒤 file descriptor 자체를 chmod
한다. 반환한 descriptor는 SQLite open/identity 검사가 끝날 때까지 directory inode를 고정한다.
*/
fn prepare_private_authority_directory_tree(parent: &Path) -> Result<Vec<File>> {
    let managed_root = parent.ancestors().nth(2).ok_or_else(|| {
        anyhow!(
            "authority-store parent is outside the managed directory layout: {}",
            parent.display()
        )
    })?;
    let storage_root = managed_root.parent().ok_or_else(|| {
        anyhow!(
            "authority-store managed root has no storage parent: {}",
            managed_root.display()
        )
    })?;

    #[cfg(unix)]
    validate_authority_storage_ancestor_chain(storage_root)?;

    match fs::symlink_metadata(storage_root) {
        Ok(metadata) => validate_directory_path_type(storage_root, &metadata)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_private_authority_storage_root(storage_root)
                .with_context(|| format!("failed to create {}", storage_root.display()))?;
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect {}", storage_root.display()));
        }
    }
    #[cfg(unix)]
    validate_authority_storage_ancestor_chain(storage_root)?;
    let storage_root_anchor = open_and_validate_authority_storage_root(storage_root)?;

    match fs::symlink_metadata(managed_root) {
        Ok(metadata) => validate_directory_path_type(managed_root, &metadata)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Err(create_error) = fs::create_dir(managed_root)
                && create_error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(create_error)
                    .with_context(|| format!("failed to create {}", managed_root.display()));
            }
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect {}", managed_root.display()));
        }
    }

    // Secure and pin each parent before creating its child. Besides minimizing the initial
    // permission window, this prevents a concurrent first opener from propagating a newly secured
    // parent ACL over a child while another opener is validating that child.
    let mut anchors = Vec::new();
    anchors.push(storage_root_anchor);
    anchors.push(open_and_secure_private_directory(managed_root)?);
    let mut current = managed_root.to_path_buf();
    for component in parent
        .strip_prefix(managed_root)
        .with_context(|| {
            format!(
                "authority-store parent {} is outside managed root {}",
                parent.display(),
                managed_root.display()
            )
        })?
        .components()
    {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) => validate_directory_path_type(&current, &metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Err(create_error) = fs::create_dir(&current)
                    && create_error.kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(create_error)
                        .with_context(|| format!("failed to create {}", current.display()));
                }
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect {}", current.display()));
            }
        }
        anchors.push(open_and_secure_private_directory(&current)?);
    }

    Ok(anchors)
}

fn create_private_authority_storage_root(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(path)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)?;
        Ok(())
    }
}

#[cfg(unix)]
fn validate_authority_storage_ancestor_chain(path: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let current_user = unsafe { libc::geteuid() };
    let mut nearest_existing = None;
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                nearest_existing = Some(ancestor);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to inspect authority-store ancestor {}",
                        ancestor.display()
                    )
                });
            }
        }
    }
    let nearest_existing = nearest_existing
        .ok_or_else(|| anyhow!("authority-store path has no reachable existing ancestor"))?;
    let canonical_existing = fs::canonicalize(nearest_existing).with_context(|| {
        format!(
            "failed to canonicalize authority-store ancestor {}",
            nearest_existing.display()
        )
    })?;
    validate_canonical_authority_storage_ancestors(&canonical_existing, current_user)?;

    let mut ancestors = path.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        let metadata = match fs::symlink_metadata(ancestor) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to inspect authority-store ancestor {}",
                        ancestor.display()
                    )
                });
            }
        };
        let owner_is_trusted = metadata.uid() == current_user || metadata.uid() == 0;
        if metadata.file_type().is_symlink() {
            let target = fs::metadata(ancestor).with_context(|| {
                format!(
                    "failed to inspect authority-store ancestor link target {}",
                    ancestor.display()
                )
            })?;
            let target_mode = target.permissions().mode();
            let target_owner_is_trusted = target.uid() == current_user || target.uid() == 0;
            let target_is_protected =
                target_mode & 0o022 == 0 || target_mode & unix_sticky_mode_bit() != 0;
            if !owner_is_trusted
                || !target.is_dir()
                || !target_owner_is_trusted
                || !target_is_protected
            {
                return Err(anyhow!(
                    "authority-store ancestor link is not trusted: {}",
                    ancestor.display()
                ));
            }
            continue;
        }
        let mode = metadata.permissions().mode();
        let protected_from_replacement = mode & 0o022 == 0 || mode & unix_sticky_mode_bit() != 0;
        if !metadata.is_dir() || !owner_is_trusted || !protected_from_replacement {
            return Err(anyhow!(
                "authority-store ancestor is writable or owned by another user: {}",
                ancestor.display()
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn validate_canonical_authority_storage_ancestors(
    path: &Path,
    current_user: libc::uid_t,
) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let mut ancestors = path.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        let metadata = fs::symlink_metadata(ancestor).with_context(|| {
            format!(
                "failed to inspect canonical authority-store ancestor {}",
                ancestor.display()
            )
        })?;
        let mode = metadata.permissions().mode();
        let owner_is_trusted = metadata.uid() == current_user || metadata.uid() == 0;
        let protected_from_replacement = mode & 0o022 == 0 || mode & unix_sticky_mode_bit() != 0;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || !owner_is_trusted
            || !protected_from_replacement
        {
            return Err(anyhow!(
                "canonical authority-store ancestor is writable, linked, or owned by another user: {}",
                ancestor.display()
            ));
        }
    }
    Ok(())
}

fn validate_directory_path_type(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        if metadata.file_attributes() & WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(anyhow!(
                "authority-store directory must not be a Windows reparse point: {}",
                path.display()
            ));
        }
    }
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(anyhow!(
            "authority-store directory must be a real directory, not a link: {}",
            path.display()
        ));
    }
    Ok(())
}

fn open_and_validate_authority_storage_root(path: &Path) -> Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)
            .with_context(|| format!("failed to securely open {}", path.display()))?;
        let metadata = directory
            .metadata()
            .with_context(|| format!("failed to inspect storage root {}", path.display()))?;
        let mode = metadata.mode();
        let writable_by_other_users = mode & 0o022 != 0;
        let sticky = mode & unix_sticky_mode_bit() != 0;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || (writable_by_other_users && !sticky)
        {
            return Err(anyhow!(
                "authority-store storage root must be current-user-owned and protected from replacement by other users: {}",
                path.display()
            ));
        }
        validate_path_identity(path, &metadata, true)?;
        Ok(directory)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        let directory = OpenOptions::new()
            .read(true)
            .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .with_context(|| format!("failed to securely open {}", path.display()))?;
        validate_windows_path_identity(path, &directory, true)?;
        Ok(directory)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(anyhow!(
            "secure authority-store storage roots are unsupported on this platform: {}",
            path.display()
        ))
    }
}

#[cfg(unix)]
fn unix_sticky_mode_bit() -> u32 {
    // libc exposes S_ISVTX as u16 on macOS and u32 on Linux.
    #[allow(clippy::useless_conversion)]
    u32::from(libc::S_ISVTX)
}

fn open_and_secure_private_directory(path: &Path) -> Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)
            .with_context(|| format!("failed to securely open {}", path.display()))?;
        let metadata = directory
            .metadata()
            .with_context(|| format!("failed to inspect opened directory {}", path.display()))?;
        if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(anyhow!(
                "authority-store directory must be owned by the current user: {}",
                path.display()
            ));
        }
        validate_path_identity(path, &metadata, true)?;
        directory
            .set_permissions(fs::Permissions::from_mode(0o700))
            .with_context(|| format!("failed to secure {}", path.display()))?;
        let secured = directory
            .metadata()
            .with_context(|| format!("failed to verify permissions for {}", path.display()))?;
        if secured.mode() & 0o777 != 0o700 {
            return Err(anyhow!(
                "authority-store directory permissions are not private: {}",
                path.display()
            ));
        }
        validate_path_identity(path, &secured, true)?;
        Ok(directory)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        let directory = OpenOptions::new()
            .read(true)
            // SetSecurityInfo skips child DACL propagation for a MAXIMUM_ALLOWED handle. This
            // keeps hostile hardlinks and concurrent first-open children untouched while the
            // directory itself is secured.
            .access_mode(WINDOWS_MAXIMUM_ALLOWED)
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .with_context(|| format!("failed to securely open {}", path.display()))?;
        validate_windows_path_identity(path, &directory, true)?;
        set_windows_private_acl(&directory)?;
        validate_windows_private_owner_and_acl(path, &directory)?;
        validate_windows_path_identity(path, &directory, true)?;
        Ok(directory)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(anyhow!(
            "secure authority-store directories are unsupported on this platform: {}",
            path.display()
        ))
    }
}

fn prepare_private_authority_store_file(path: &Path) -> Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

        let store = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .with_context(|| format!("failed to securely open {}", path.display()))?;
        let metadata = store
            .metadata()
            .with_context(|| format!("failed to inspect opened store {}", path.display()))?;
        validate_owned_private_store_metadata(path, &metadata)?;
        validate_path_identity(path, &metadata, false)?;
        store
            .set_permissions(fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure {}", path.display()))?;
        let secured = store
            .metadata()
            .with_context(|| format!("failed to verify permissions for {}", path.display()))?;
        validate_owned_private_store_metadata(path, &secured)?;
        if secured.mode() & 0o777 != 0o600 {
            return Err(anyhow!(
                "authority-store file permissions are not private: {}",
                path.display()
            ));
        }
        validate_path_identity(path, &secured, false)?;
        Ok(store)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        let store = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .access_mode(
                WINDOWS_GENERIC_READ
                    | WINDOWS_GENERIC_WRITE
                    | WINDOWS_READ_CONTROL
                    | WINDOWS_WRITE_DAC,
            )
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .with_context(|| format!("failed to securely open {}", path.display()))?;
        validate_windows_path_identity(path, &store, false)?;
        set_windows_private_acl(&store)?;
        validate_windows_private_owner_and_acl(path, &store)?;
        validate_windows_path_identity(path, &store, false)?;
        Ok(store)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(anyhow!(
            "secure authority-store files are unsupported on this platform: {}",
            path.display()
        ))
    }
}

#[cfg(unix)]
fn validate_owned_private_store_metadata(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1
    {
        return Err(anyhow!(
            "authority-store file must be a current-user-owned regular file with one link: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_path_identity(path: &Path, opened: &fs::Metadata, directory: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let path_metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect path identity for {}", path.display()))?;
    let expected_type = if directory {
        path_metadata.is_dir()
    } else {
        path_metadata.is_file() && path_metadata.nlink() == 1
    };
    if path_metadata.file_type().is_symlink()
        || !expected_type
        || path_metadata.uid() != unsafe { libc::geteuid() }
        || path_metadata.dev() != opened.dev()
        || path_metadata.ino() != opened.ino()
    {
        return Err(anyhow!(
            "authority-store path identity changed while opening: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(windows)]
const WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
#[cfg(windows)]
const WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const WINDOWS_FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const WINDOWS_FILE_SHARE_ALL: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004;
#[cfg(windows)]
const WINDOWS_GENERIC_READ: u32 = 0x8000_0000;
#[cfg(windows)]
const WINDOWS_GENERIC_WRITE: u32 = 0x4000_0000;
#[cfg(windows)]
const WINDOWS_READ_CONTROL: u32 = 0x0002_0000;
#[cfg(windows)]
const WINDOWS_WRITE_DAC: u32 = 0x0004_0000;
#[cfg(windows)]
const WINDOWS_MAXIMUM_ALLOWED: u32 = 0x0200_0000;

#[cfg(windows)]
fn validate_windows_path_identity(path: &Path, opened: &File, directory: bool) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    let path_handle = OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(
            WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    WINDOWS_FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
        )
        .open(path)
        .with_context(|| format!("failed to reopen Windows path {}", path.display()))?;
    let opened_identity = windows_file_identity(opened)?;
    let path_identity = windows_file_identity(&path_handle)?;
    let expected_type = if directory {
        opened_identity.attributes & WINDOWS_FILE_ATTRIBUTE_DIRECTORY != 0
            && path_identity.attributes & WINDOWS_FILE_ATTRIBUTE_DIRECTORY != 0
    } else {
        opened_identity.attributes & WINDOWS_FILE_ATTRIBUTE_DIRECTORY == 0
            && path_identity.attributes & WINDOWS_FILE_ATTRIBUTE_DIRECTORY == 0
            && opened_identity.number_of_links == 1
            && path_identity.number_of_links == 1
    };
    if opened_identity.attributes & WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT != 0
        || path_identity.attributes & WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT != 0
        || !expected_type
        || opened_identity.volume_serial_number != path_identity.volume_serial_number
        || opened_identity.file_index != path_identity.file_index
    {
        return Err(anyhow!(
            "authority-store Windows path must be a stable non-reparse, single-link object: {}",
            path.display()
        ));
    }
    validate_windows_owner(path, opened)
}

#[cfg(windows)]
const WINDOWS_FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;

#[cfg(windows)]
struct WindowsFileIdentity {
    attributes: u32,
    volume_serial_number: u32,
    file_index: u64,
    number_of_links: u32,
}

#[cfg(windows)]
fn windows_file_identity(file: &File) -> Result<WindowsFileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the file handle and output structure remain valid for the duration of the call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut information) } == 0
    {
        return Err(std::io::Error::last_os_error())
            .context("failed to inspect Windows authority-store handle");
    }
    Ok(WindowsFileIdentity {
        attributes: information.dwFileAttributes,
        volume_serial_number: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
        number_of_links: information.nNumberOfLinks,
    })
}

#[cfg(windows)]
struct WindowsHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for WindowsHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper is created only for a successful owned token handle.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
struct WindowsLocalAllocation(windows_sys::Win32::Foundation::HLOCAL);

#[cfg(windows)]
impl Drop for WindowsLocalAllocation {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: Windows security APIs allocate this pointer with LocalAlloc.
            unsafe {
                windows_sys::Win32::Foundation::LocalFree(self.0);
            }
        }
    }
}

#[cfg(windows)]
struct WindowsCurrentUserSid {
    _buffer: Vec<usize>,
    sid: windows_sys::Win32::Security::PSID,
}

#[cfg(windows)]
fn windows_current_user_sid() -> Result<WindowsCurrentUserSid> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut raw_token: HANDLE = std::ptr::null_mut();
    // SAFETY: output points to valid storage and the pseudo process handle is always valid.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw_token) } == 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to open current process token");
    }
    let token = WindowsHandle(raw_token);
    let mut required = 0u32;
    // SAFETY: the null-buffer probe is the documented way to obtain the required size.
    unsafe {
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut required);
    }
    if required < std::mem::size_of::<TOKEN_USER>() as u32 {
        return Err(anyhow!(
            "Windows current-user token did not expose a user SID"
        ));
    }
    let word_count = (required as usize).div_ceil(std::mem::size_of::<usize>());
    let mut buffer = vec![0usize; word_count];
    // SAFETY: the aligned buffer is at least `required` bytes and remains alive in the result.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error()).context("failed to read current-user SID");
    }
    // SAFETY: a successful TokenUser query initializes a TOKEN_USER at the buffer start.
    let sid = unsafe { (*(buffer.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    if sid.is_null() {
        return Err(anyhow!("Windows current-user SID was null"));
    }
    Ok(WindowsCurrentUserSid {
        _buffer: buffer,
        sid,
    })
}

#[cfg(windows)]
fn validate_windows_owner(path: &Path, file: &File) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{EqualSid, OWNER_SECURITY_INFORMATION, PSID};

    let current_user = windows_current_user_sid()?;
    let mut owner: PSID = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: all out-pointers are valid and the file handle remains open for the call.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32))
            .with_context(|| format!("failed to read Windows owner for {}", path.display()));
    }
    let _descriptor = WindowsLocalAllocation(descriptor.cast());
    // SAFETY: both SIDs come from validated Windows security API responses.
    if owner.is_null() || unsafe { EqualSid(owner, current_user.sid) } == 0 {
        return Err(anyhow!(
            "authority-store Windows object must be owned by the current user: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn set_windows_private_acl(file: &File) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW,
        SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, NO_INHERITANCE, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;

    let current_user = windows_current_user_sid()?;
    let trustee = TRUSTEE_W {
        pMultipleTrustee: std::ptr::null_mut(),
        MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_USER,
        ptstrName: current_user.sid.cast(),
    };
    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        // Every managed directory, store, and sidecar is opened and secured independently.
        // Non-inheriting ACEs prevent concurrent parent ACL updates from propagating over a child
        // while another opener validates that child's protected DACL.
        grfInheritance: NO_INHERITANCE,
        Trustee: trustee,
    };
    let mut acl = std::ptr::null_mut();
    // SAFETY: the access entry and ACL out-pointer are valid for the duration of the call.
    let acl_status = unsafe { SetEntriesInAclW(1, &access, std::ptr::null(), &mut acl) };
    if acl_status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(acl_status as i32))
            .context("failed to build private Windows authority-store ACL");
    }
    let _acl = WindowsLocalAllocation(acl.cast());
    // SAFETY: the file handle is open with WRITE_DAC and `acl` remains allocated here.
    let set_status = unsafe {
        SetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl,
            std::ptr::null(),
        )
    };
    if set_status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(set_status as i32))
            .context("failed to apply private Windows authority-store ACL");
    }
    Ok(())
}

#[cfg(windows)]
fn validate_windows_private_owner_and_acl(path: &Path, file: &File) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation,
        GetSecurityDescriptorControl, OWNER_SECURITY_INFORMATION, PSID, SE_DACL_PROTECTED,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;

    let current_user = windows_current_user_sid()?;
    let mut owner: PSID = std::ptr::null_mut();
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: all out-pointers are valid and the file handle remains open for the call.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32))
            .with_context(|| format!("failed to verify Windows security for {}", path.display()));
    }
    let _descriptor = WindowsLocalAllocation(descriptor.cast());
    let mut control = 0u16;
    let mut revision = 0u32;
    let mut acl_size = ACL_SIZE_INFORMATION::default();
    // SAFETY: owner, descriptor and ACL all belong to the live security descriptor allocation.
    let header_is_valid = !owner.is_null()
        && !acl.is_null()
        && !descriptor.is_null()
        && unsafe { EqualSid(owner, current_user.sid) } != 0
        && unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } != 0
        && control & SE_DACL_PROTECTED != 0
        && unsafe {
            GetAclInformation(
                acl,
                (&mut acl_size as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } != 0
        && acl_size.AceCount == 1;
    if !header_is_valid {
        return Err(anyhow!(
            "authority-store Windows ACL is not private and owner-bound: {}",
            path.display()
        ));
    }
    let mut raw_ace = std::ptr::null_mut();
    // SAFETY: the ACL was validated to contain exactly one ACE.
    if unsafe { GetAce(acl, 0, &mut raw_ace) } == 0 || raw_ace.is_null() {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("failed to inspect Windows ACL for {}", path.display()));
    }
    // SAFETY: GetAce returned the first ACE, whose fixed header is always readable.
    let ace = unsafe { &*(raw_ace.cast::<ACCESS_ALLOWED_ACE>()) };
    let ace_sid = (&ace.SidStart as *const u32).cast_mut().cast();
    // ACCESS_ALLOWED_ACE_TYPE is zero. Reject any inherited/deny/extra principal entry.
    if ace.Header.AceType != 0
        || ace.Header.AceFlags != 0
        || ace.Mask & FILE_ALL_ACCESS != FILE_ALL_ACCESS
        // SAFETY: SidStart is the documented inline SID start for ACCESS_ALLOWED_ACE.
        || unsafe { EqualSid(ace_sid, current_user.sid) } == 0
    {
        return Err(anyhow!(
            "authority-store Windows ACL grants access beyond the current user: {}",
            path.display()
        ));
    }
    Ok(())
}

fn validate_authority_store_path_identity(path: &Path, store: &File) -> Result<()> {
    #[cfg(unix)]
    {
        let metadata = store
            .metadata()
            .with_context(|| format!("failed to inspect opened store {}", path.display()))?;
        validate_owned_private_store_metadata(path, &metadata)?;
        validate_path_identity(path, &metadata, false)?;
    }
    #[cfg(windows)]
    {
        validate_windows_path_identity(path, store, false)?;
        validate_windows_private_owner_and_acl(path, store)?;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (path, store);
        return Err(anyhow!(
            "secure authority-store identity validation is unsupported on this platform"
        ));
    }
    Ok(())
}

/*
기존 authority DB가 현재 binary가 지원하는 schema인지 검사한다.

metadata table이 없고 다른 user object도 없는 빈 DB만 새 store로 본다. metadata가 있으면
`schema_version`, store mode, canonical repository binding을 읽고 additive migration이 지원되는
v7부터 현재 버전까지만 통과시킨다.

이 guard가 없으면 미래 버전의 DB를 구버전 binary가 열어 schema를 덮거나 잘못 해석할 수 있다.
*/
fn validate_authority_store_schema(
    connection: &Connection,
    location: &PlanningAuthorityLocation,
) -> Result<()> {
    let metadata_exists = table_exists(connection, "authority_metadata")?;
    if !metadata_exists {
        let foreign_object_count = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to inspect unmarked authority-store objects")?;
        if foreign_object_count != 0 {
            return Err(anyhow!(
                "unmarked non-empty SQLite database cannot be adopted as an authority-store"
            ));
        }
        return Ok(());
    }

    let schema_version = connection
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("failed to read authority-store schema version")?
        .ok_or_else(|| anyhow!("authority-store schema version is missing"))?;
    let parsed_schema_version = schema_version
        .parse::<i64>()
        .map_err(|_| anyhow!("unsupported authority-store schema version: {schema_version}"))?;
    if !(MINIMUM_MIGRATABLE_AUTHORITY_STORE_SCHEMA_VERSION..=AUTHORITY_STORE_SCHEMA_VERSION)
        .contains(&parsed_schema_version)
    {
        return Err(anyhow!(
            "unsupported authority-store schema version: {schema_version}"
        ));
    }

    let mode = read_metadata_string_connection(connection, "mode")?
        .ok_or_else(|| anyhow!("authority-store mode marker is missing"))?;
    if mode != AUTHORITY_STORE_MODE {
        return Err(anyhow!("unsupported authority-store mode: {mode}"));
    }
    if let Some(repository_identity) =
        read_metadata_string_connection(connection, "repository_identity")?
    {
        if repository_identity != location.repository_identity {
            return Err(anyhow!(
                "authority-store is bound to a different repository identity"
            ));
        }
    } else {
        let canonical_repo_root =
            read_metadata_string_connection(connection, "canonical_repo_root")?.ok_or_else(
                || anyhow!("authority-store canonical repository binding is missing"),
            )?;
        if canonical_repo_root != location.canonical_repo_root {
            return Err(anyhow!(
                "legacy authority-store is bound to a different canonical repository"
            ));
        }
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

fn insert_task_authority_mutation_records(
    transaction: &rusqlite::Transaction<'_>,
    records: &[PlanningTaskAuthorityMutationRecord],
) -> Result<()> {
    for (event_order, record) in records.iter().enumerate() {
        let content_json = serde_json::to_string(record)
            .context("failed to serialize planning task mutation event")?;
        transaction
            .execute(
                "INSERT INTO planning_task_mutation_events
                 (planning_revision, event_order, task_id, content_json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    record.planning_revision,
                    event_order as i64,
                    record.task_id.trim(),
                    content_json
                ],
            )
            .with_context(|| {
                format!(
                    "failed to store planning task mutation event for `{}`",
                    record.task_id
                )
            })?;
    }
    Ok(())
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

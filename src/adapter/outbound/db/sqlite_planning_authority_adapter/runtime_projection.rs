// 런타임 투영은 DB에 저장된 여러 행을 메모리 스냅샷으로 다시 조립하므로,
// 중복 제거와 정렬된 출력을 동시에 제공하는 BTree 계열 컬렉션을 사용한다.
use std::collections::{BTreeMap, BTreeSet};

// 이 모듈의 public helper들은 모두 outbound adapter 경계에서 실패할 수 있으므로
// `anyhow::Result`로 오류를 올리고, `Context`로 SQLite 단계 이름을 붙인다.
use anyhow::{Context, Result};
// 클레임 만료와 큐 처리 시각은 DB 행에 문자열로 남기기 때문에,
// UTC 기준 RFC3339 타임스탬프를 만드는 `Utc`가 이 파일의 공통 시간 원천이다.
use chrono::Utc;
// `Connection`은 helper 함수들이 트랜잭션 밖 조회를 수행할 때 필요하고,
// `OptionalExtension`은 "행 없음"을 오류가 아닌 Option으로 바꿔 클레임 부재를 표현한다.
use rusqlite::{Connection, OptionalExtension, Transaction, params};

// application port의 record/status/snapshot 타입을 그대로 반환해,
// SQLite 세부 구조가 application layer로 새지 않도록 어댑터 내부에서 매핑을 끝낸다.
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogRequest;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
    PlanningAuthorityOfficialRefreshRecoveryStatus, PlanningAuthorityRuntimeProjectionSnapshot,
};
// parallel mode의 slot lease와 agent session은 domain 타입이므로,
// 여기서는 DB row를 도메인이 이해하는 스냅샷 값으로 복원하는 역할만 맡는다.
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModePoolResetReport,
    ParallelModeRuntimeEventEntry, ParallelModeRuntimeEventsSnapshot,
    ParallelModeSlotLeaseSnapshot, ParallelModeTaskDispatchBlockSnapshot,
};

// metadata upsert helper는 store 모듈의 스키마 관리와 같은 규칙을 공유한다.
// 런타임 클레임도 권위 DB가 갱신되었다는 흔적을 남겨 다른 프로세스가 변화를 감지하게 한다.
use super::store::{upsert_authority_metadata, upsert_metadata};
// adapter 본체의 위치 해석/DB 열기 함수와 클레임 상수들을 가져온다.
// 이 파일은 `runtime_claims`, distributor queue, snapshot projection만 분리한 impl 조각이다.
use super::{
    CLAIM_STALE_AFTER_SECS, DISTRIBUTOR_QUEUE_CLAIM_KIND, OFFICIAL_REFRESH_SCOPE_KEY,
    SqlitePlanningAuthorityAdapter, open_authority_connection, read_metadata_i64,
};

// 이 impl 블록은 `SqlitePlanningAuthorityAdapter`의 런타임 상태 책임을 담는다.
// 영구 planning authority 문서가 아니라, 여러 실행 주체가 동시에 움직일 때 필요한
// 순번, 임시 소유권, 큐 상태, agent session 투영을 SQLite에 기록하고 다시 읽는다.
impl SqlitePlanningAuthorityAdapter {
    // 공식 refresh는 여러 worker가 동시에 시작할 수 있으므로 먼저 단조 증가 순번을 예약한다.
    // `next_official_refresh_order`는 "발급할 번호"이고, 아래에서 발급 직후 +1로 저장해 다음 호출과 충돌하지 않게 한다.
    pub(crate) fn reserve_next_official_refresh_order(workspace_dir: &str) -> Result<u64> {
        // workspace 경로만 받은 application layer 요청을 실제 authority DB 위치로 변환한다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 순번 예약은 read-modify-write라서 한 커넥션의 트랜잭션 안에서 처리해야 한다.
        let mut connection = open_authority_connection(&location)?;
        // 트랜잭션은 두 metadata 행 갱신을 한 단위로 묶는다.
        // 중간에 실패하면 순번만 증가하거나 갱신 시각만 바뀌는 반쪽 상태가 남지 않는다.
        let transaction = connection
            .transaction()
            .context("failed to open official refresh order transaction")?;
        // `last_claim_updated_at`은 클레임/큐 계열 런타임 상태가 바뀌었다는 저렴한 변경 신호이다.
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        // 새 DB에는 metadata가 없을 수 있으므로 1을 기본 순번으로 삼는다.
        // 반환값은 현재 호출자가 처리할 refresh 번호이고, 저장값은 다음 호출자를 위한 번호이다.
        let next_refresh_order =
            read_metadata_i64(&transaction, "next_official_refresh_order")?.unwrap_or(1);
        upsert_metadata(
            &transaction,
            "next_official_refresh_order",
            &(next_refresh_order + 1).to_string(),
        )?;
        // commit이 성공해야만 예약된 순번이 다른 프로세스에 보인다.
        transaction
            .commit()
            .context("failed to commit official refresh order transaction")?;
        // application layer는 이 순번을 다시 `acquire_official_refresh_claim`에 넘겨
        // 자기 차례가 되었는지 확인하고 실제 refresh 작업을 시작한다.
        Ok(next_refresh_order as u64)
    }

    // 예약된 refresh 순번이 실행 가능한 차례인지 확인하고, 가능하면 단일 owner 클레임을 잡는다.
    // 반환 status는 caller가 "이미 끝남/아직 기다림/내가 실행함"을 구분해 busy loop 없이 조율하게 해준다.
    pub(crate) fn acquire_official_refresh_claim(
        // workspace는 authority DB를 찾기 위한 application-level 식별자이다.
        workspace_dir: &str,
        // `reserve_next_official_refresh_order`에서 받은 번호이다.
        // 이 값이 `next_executable_refresh_order`와 같을 때만 실행권을 시도할 수 있다.
        refresh_order: u64,
        // 같은 프로세스/작업이 재진입했을 때 자기 클레임을 알아보기 위한 소유자 토큰이다.
        owner_token: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshClaimStatus> {
        // 모든 판정은 동일한 authority DB에서 이루어져야 하므로 먼저 위치를 고정한다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 클레임 확인, stale 정리, 삽입은 경합 구간이므로 한 트랜잭션에 넣는다.
        let mut connection = open_authority_connection(&location)?;
        // 트랜잭션 경계가 이 함수의 분산 락 단위이다.
        // commit하기 전까지 다른 실행자는 이 함수의 삽입/삭제 결과를 관찰하지 못한다.
        let transaction = connection
            .transaction()
            .context("failed to open official refresh claim transaction")?;
        // 시도 자체도 런타임 클레임 영역의 활동이므로 heartbeat metadata를 갱신한다.
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        // `next_executable_refresh_order`는 완료된 refresh 다음 번호를 가리킨다.
        // 기본값 1은 아직 어떤 공식 refresh도 완료되지 않았다는 뜻이다.
        let next_executable =
            read_metadata_i64(&transaction, "next_executable_refresh_order")?.unwrap_or(1);
        // 요청 순번이 실행 포인터보다 작으면 이미 누군가 완료 처리한 작업이다.
        // DB를 바꿀 필요가 없으므로 rollback으로 읽기 트랜잭션을 닫고 상태만 알려준다.
        if (refresh_order as i64) < next_executable {
            transaction
                .rollback()
                .context("failed to roll back completed official refresh claim transaction")?;
            return Ok(PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted);
        }
        // 요청 순번이 실행 포인터보다 크면 앞선 refresh가 아직 끝나지 않았다.
        // 순서를 강제하기 위해 현재 caller는 클레임을 만들지 않고 대기 상태를 받는다.
        if (refresh_order as i64) > next_executable {
            transaction
                .rollback()
                .context("failed to roll back waiting official refresh claim transaction")?;
            return Ok(PlanningAuthorityOfficialRefreshClaimStatus::Waiting);
        }

        // 실행 가능한 순번이라도 이전 owner가 죽고 클레임만 남았을 수 있다.
        // stale이면 제거한 뒤 metadata를 다시 만져 polling 쪽이 "상태 변화"를 볼 수 있게 한다.
        if clear_stale_runtime_claim(&transaction, "official-refresh", OFFICIAL_REFRESH_SCOPE_KEY)?
        {
            upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        }
        // 공식 refresh는 scope key가 하나뿐인 전역 클레임이다.
        // 이미 row가 있으면 owner_token만 비교해 재진입인지 경합인지 판단한다.
        let existing_owner =
            load_runtime_claim(&transaction, "official-refresh", OFFICIAL_REFRESH_SCOPE_KEY)?
                .map(|claim| claim.owner_token);
        // 같은 owner가 이미 잡은 클레임이면 멱등 성공으로 처리한다.
        // 이 덕분에 caller가 네트워크/프로세스 경계에서 같은 시도를 반복해도 중복 실행으로 번지지 않는다.
        if let Some(existing_owner) = existing_owner {
            if existing_owner == owner_token {
                transaction
                    .commit()
                    .context("failed to commit existing official refresh claim transaction")?;
                return Ok(PlanningAuthorityOfficialRefreshClaimStatus::Acquired);
            }
            // 다른 owner가 살아있는 클레임을 잡고 있으면 현재 caller는 순서를 양보한다.
            // rollback을 사용해 방금 갱신하려던 heartbeat도 남기지 않으므로 실제 소유권 상태를 흐리지 않는다.
            transaction
                .rollback()
                .context("failed to roll back contended official refresh claim transaction")?;
            return Ok(PlanningAuthorityOfficialRefreshClaimStatus::Waiting);
        }

        // 클레임이 없을 때만 새 row를 삽입한다.
        // `claim_value`에 refresh_order를 문자열로 저장해 release 단계에서 "내가 잡은 그 순번"만 지우게 한다.
        transaction
            .execute(
                "INSERT INTO runtime_claims (claim_kind, scope_key, owner_token, claim_value, claimed_at)
                 VALUES ('official-refresh', ?1, ?2, ?3, ?4)",
                params![
                    OFFICIAL_REFRESH_SCOPE_KEY,
                    owner_token,
                    refresh_order.to_string(),
                    // stale 판정은 `claimed_at`과 현재 UTC 시각의 차이로 계산된다.
                    Utc::now().to_rfc3339()
                ],
            )
            .context("failed to acquire official refresh claim")?;
        // 삽입 commit이 성공하면 caller는 official refresh 작업을 수행할 권한을 얻은 것이다.
        transaction
            .commit()
            .context("failed to commit official refresh claim transaction")?;
        Ok(PlanningAuthorityOfficialRefreshClaimStatus::Acquired)
    }

    // refresh worker가 작업을 끝낸 뒤 자기 클레임을 지우고 실행 포인터를 다음 순번으로 넘긴다.
    // 삭제 조건에 owner와 순번을 모두 넣어, 늦게 도착한 release가 남의 새 클레임을 지우지 못하게 막는다.
    pub(crate) fn release_official_refresh_claim(
        // 어떤 authority DB에서 release할지 결정하는 workspace 경로이다.
        workspace_dir: &str,
        // 완료 처리할 refresh 순번이다. 실행 포인터는 최소 이 값 다음으로만 이동한다.
        refresh_order: u64,
        // acquire 때 저장한 owner token이다. 다른 owner의 클레임은 이 함수가 건드리지 않는다.
        owner_token: &str,
    ) -> Result<()> {
        // release도 acquire와 같은 DB 위치 규칙을 사용해야 순번 포인터가 한곳에서 움직인다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 클레임 삭제와 metadata 포인터 갱신을 원자적으로 묶기 위해 mutable connection을 연다.
        let mut connection = open_authority_connection(&location)?;
        // 이 트랜잭션이 성공해야 "클레임 해제"와 "다음 refresh 실행 허용"이 함께 보인다.
        let transaction = connection
            .transaction()
            .context("failed to open official refresh release transaction")?;
        // release는 대기 중인 worker에게 중요한 변화이므로 authority metadata를 갱신한다.
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        // row가 삭제되었다면 현재 caller가 실제 소유자였다는 뜻이다.
        // 삭제 수가 0이면 이미 stale 정리되었거나 다른 owner/순번이므로 포인터를 움직이면 안 된다.
        let deleted_rows = transaction
            .execute(
                "DELETE FROM runtime_claims
                 WHERE claim_kind = 'official-refresh' AND scope_key = ?1 AND owner_token = ?2 AND claim_value = ?3",
                params![OFFICIAL_REFRESH_SCOPE_KEY, owner_token, refresh_order.to_string()],
            )
            .context("failed to release official refresh claim")?;
        // 실제로 삭제한 owner만 실행 포인터를 전진시킨다.
        // 이 조건이 없으면 실패한 release 재시도가 앞선 refresh를 건너뛰는 버그가 된다.
        if deleted_rows > 0 {
            // 이미 더 큰 순번까지 완료 처리된 경우에는 과거 release가 포인터를 되돌리지 않도록 현재값을 먼저 읽는다.
            let next_executable =
                read_metadata_i64(&transaction, "next_executable_refresh_order")?.unwrap_or(1);
            // 현재 포인터가 이 refresh 이하일 때만 다음 순번으로 올린다.
            // 큰 값이면 다른 경로가 이미 더 앞까지 진행한 상태라 그대로 둔다.
            if next_executable <= refresh_order as i64 {
                upsert_metadata(
                    &transaction,
                    "next_executable_refresh_order",
                    &(refresh_order + 1).to_string(),
                )?;
            }
        }
        // commit 후에야 다음 순번 worker가 acquire에서 `Acquired`를 받을 수 있다.
        transaction
            .commit()
            .context("failed to commit official refresh release transaction")?;
        Ok(())
    }

    // abandoned official refresh order를 회수해 뒤 refresh가 영구 대기하지 않게 한다.
    pub(crate) fn abandon_next_official_refresh_order(
        workspace_dir: &str,
        reason: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshRecoveryStatus> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open official refresh recovery transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;

        let next_executable =
            read_metadata_i64(&transaction, "next_executable_refresh_order")?.unwrap_or(1);
        let next_official =
            read_metadata_i64(&transaction, "next_official_refresh_order")?.unwrap_or(1);
        if next_executable >= next_official {
            transaction
                .rollback()
                .context("failed to roll back idle official refresh recovery")?;
            return Ok(PlanningAuthorityOfficialRefreshRecoveryStatus::NoPendingOrder);
        }

        let stale_claim_cleared = clear_stale_runtime_claim(
            &transaction,
            "official-refresh",
            OFFICIAL_REFRESH_SCOPE_KEY,
        )?;
        if !stale_claim_cleared
            && load_runtime_claim(&transaction, "official-refresh", OFFICIAL_REFRESH_SCOPE_KEY)?
                .is_some()
        {
            transaction
                .rollback()
                .context("failed to roll back active official refresh recovery")?;
            return Ok(PlanningAuthorityOfficialRefreshRecoveryStatus::WaitingForActiveClaim);
        }

        let refresh_order = next_executable as u64;
        upsert_metadata(
            &transaction,
            "next_executable_refresh_order",
            &(refresh_order + 1).to_string(),
        )?;
        append_runtime_event(
            &transaction,
            "official_refresh_abandoned",
            "official_refresh",
            &refresh_order.to_string(),
            &format!("official refresh order {refresh_order} abandoned during recovery: {reason}"),
            &serde_json::json!({
                "refresh_order": refresh_order,
                "reason": reason,
            })
            .to_string(),
        )?;
        transaction
            .commit()
            .context("failed to commit official refresh recovery transaction")?;
        Ok(PlanningAuthorityOfficialRefreshRecoveryStatus::Recovered { refresh_order })
    }

    // distributor queue의 특정 item을 처리할 권한을 시도한다.
    // 공식 refresh처럼 전역 순번을 따르지 않고, queue_item_id 하나가 하나의 scope key가 되어 병렬 처리를 허용한다.
    pub(crate) fn try_acquire_distributor_queue_claim(
        // queue 상태를 담은 authority DB를 workspace 기준으로 찾는다.
        workspace_dir: &str,
        // 처리할 distributor queue row의 식별자이다.
        // 같은 id에 대해서만 서로 경합하고, 다른 id들은 독립적으로 클레임될 수 있다.
        queue_item_id: &str,
        // 현재 worker를 나타내는 토큰이다. release 때 같은 토큰이어야 row를 지울 수 있다.
        owner_token: &str,
    ) -> Result<bool> {
        // 모든 queue claim은 해당 workspace의 authority DB 안에서만 의미가 있다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // stale 정리와 새 클레임 삽입을 한 경합 구간으로 묶기 위해 트랜잭션을 연다.
        let mut connection = open_authority_connection(&location)?;
        // 이 트랜잭션 안에서만 "기존 클레임이 사라졌으니 내가 삽입한다"는 판단이 안전한다.
        let transaction = connection
            .transaction()
            .context("failed to open distributor queue claim transaction")?;
        // queue claim 시도도 runtime projection의 관찰 가능한 변화이므로 metadata heartbeat를 갱신한다.
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        // 이전 worker가 죽어 같은 queue item의 클레임이 오래 남았으면 먼저 삭제한다.
        // 삭제가 있었다면 곧바로 재시도할 수 있는 상태 변화이므로 metadata를 한 번 더 갱신한다.
        if clear_stale_runtime_claim(&transaction, DISTRIBUTOR_QUEUE_CLAIM_KIND, queue_item_id)? {
            upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        }
        // `INSERT OR IGNORE`는 runtime_claims의 고유키를 경합 제어로 사용한다.
        // 이미 같은 queue item 클레임이 있으면 0행 삽입이 되어 false를 반환하고, 새로 잡으면 true가 된다.
        let inserted_rows = transaction
            .execute(
                "INSERT OR IGNORE INTO runtime_claims
                 (claim_kind, scope_key, owner_token, claim_value, claimed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    DISTRIBUTOR_QUEUE_CLAIM_KIND,
                    queue_item_id,
                    owner_token,
                    // queue claim에서는 scope와 값이 모두 queue item id이다.
                    // scope는 중복 소유권 방지에 쓰이고, value는 snapshot/debug 출력에서 어떤 item인지 보여준다.
                    queue_item_id,
                    // stale claim 청소는 이 시각과 현재 UTC 시각의 차이를 기준으로 한다.
                    Utc::now().to_rfc3339()
                ],
            )
            .context("failed to acquire distributor queue claim")?;
        // commit 전까지는 다른 worker가 방금 잡은 queue item을 볼 수 없다.
        transaction
            .commit()
            .context("failed to commit distributor queue claim transaction")?;
        // bool 반환은 caller가 "내가 처리해야 함"과 "다른 worker가 이미 처리 중"을 즉시 구분하게 한다.
        Ok(inserted_rows > 0)
    }

    // queue item 처리가 끝났거나 포기할 때 현재 owner의 클레임만 해제한다.
    // owner_token 조건을 둬서 늦은 release가 다른 worker가 새로 잡은 같은 item을 지우지 못하게 한다.
    pub(crate) fn release_distributor_queue_claim(
        // claim row가 저장된 authority DB를 찾기 위한 workspace 경로이다.
        workspace_dir: &str,
        // 해제할 queue item scope이다.
        queue_item_id: &str,
        // acquire 때 저장한 소유자 토큰이다. 일치하지 않으면 삭제 조건에 걸리지 않는다.
        owner_token: &str,
    ) -> Result<()> {
        // release는 acquire와 같은 위치 해석을 사용해야 같은 runtime_claims 테이블을 수정한다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 삭제와 heartbeat 갱신을 하나의 DB 작업 단위로 묶는다.
        let mut connection = open_authority_connection(&location)?;
        // 트랜잭션이 성공해야 대기 worker가 "클레임이 풀렸다"는 상태를 일관되게 관찰한다.
        let transaction = connection
            .transaction()
            .context("failed to open distributor queue release transaction")?;
        // 삭제 결과가 0행이어도 release 시도 자체는 상태 확인자가 다시 볼 수 있는 시점이다.
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        // kind, scope, owner를 모두 맞춰 지운다.
        // 공식 refresh와 달리 별도 순번 포인터가 없으므로 삭제만으로 queue item 소유권이 풀린다.
        transaction
            .execute(
                "DELETE FROM runtime_claims
                 WHERE claim_kind = ?1 AND scope_key = ?2 AND owner_token = ?3",
                params![DISTRIBUTOR_QUEUE_CLAIM_KIND, queue_item_id, owner_token],
            )
            .context("failed to release distributor queue claim")?;
        // commit 후 다음 worker의 `INSERT OR IGNORE`가 같은 item을 다시 잡을 수 있다.
        transaction
            .commit()
            .context("failed to commit distributor queue release transaction")?;
        Ok(())
    }

    // runtime projection 전체를 application port snapshot으로 읽는 얇은 진입점이다.
    // DB row를 도메인/application 타입으로 조립하는 실제 작업은 아래 free function에 위임한다.
    pub(crate) fn load_runtime_projections(
        // 읽을 authority DB를 고르는 workspace 경로이다.
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        // adapter 경계에서는 workspace만 알고 있으므로 먼저 DB 위치로 변환한다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // snapshot 로드는 읽기 전용이므로 트랜잭션 없이 열린 connection을 helper에 전달한다.
        let connection = open_authority_connection(&location)?;
        load_runtime_projection_snapshot(&connection)
    }

    // runtime_events audit feed를 bounded read model로 읽는다.
    // current projection snapshot과 달리 이 경로는 시간순 변경 이력을 보여 주기 때문에 request limit/filter를 받는다.
    pub(crate) fn load_runtime_event_log(
        // 읽을 authority DB를 고르는 workspace 경로이다.
        workspace_dir: &str,
        // UI나 진단 경로가 요청한 limit/projection filter이다.
        request: ParallelModeRuntimeEventLogRequest,
    ) -> Result<ParallelModeRuntimeEventsSnapshot> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        load_runtime_event_log_snapshot(&connection, &request)
    }

    // parallel mode의 slot lease snapshot을 authority DB의 현재 런타임 투영으로 저장한다.
    // slot_id가 같은 row는 덮어써서 "지금 이 슬롯의 최신 상태"만 남기고, 변경 이력은 runtime_events에 따로 기록한다.
    pub(crate) fn upsert_runtime_slot_lease(
        // slot lease projection을 저장할 authority DB를 찾는 workspace 경로이다.
        workspace_dir: &str,
        // domain layer가 만든 slot lease 상태이다. 이 adapter는 내용을 해석하지 않고 JSON으로 보존한다.
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()> {
        // 같은 workspace의 parallel runtime 상태는 같은 authority DB에 모이다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // upsert, invalid marker 삭제, event append를 원자적으로 묶기 위해 mutable connection을 연다.
        let mut connection = open_authority_connection(&location)?;

        // `runtime_slot_leases.content`는 원본 snapshot 전체를 JSON으로 저장한다.
        // 나중에 load 단계에서 같은 domain 타입으로 역직렬화해 adapter 밖에는 DB column 세부사항을 숨긴다.
        let payload_json = serde_json::to_string(lease)
            .context("failed to serialize runtime slot lease projection")?;
        // 이 트랜잭션이 성공해야 current row, invalid marker 정리, event log가 같은 상태를 가리킨다.
        let transaction = connection
            .transaction()
            .context("failed to open runtime slot lease transaction")?;
        // projection 쪽 변경 신호는 claim heartbeat와 별도로 `last_runtime_projection_at`에 남긴다.
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        // slot_id를 primary identity로 보고, 같은 slot의 lease가 오면 updated_at/content만 최신화한다.
        // 그래서 snapshot 조회자는 중복 row를 합칠 필요 없이 테이블 그대로 현재 슬롯 상태로 읽을 수 있다.
        transaction
            .execute(
                "INSERT INTO runtime_slot_leases (slot_id, updated_at, content)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(slot_id) DO UPDATE
                 SET updated_at = excluded.updated_at,
                     content = excluded.content",
                params![lease.slot_id, Utc::now().to_rfc3339(), payload_json],
            )
            .with_context(|| format!("failed to persist runtime slot lease `{}`", lease.slot_id))?;
        // 정상 lease가 들어왔다는 것은 같은 slot의 invalid marker가 더 이상 유효하지 않다는 뜻이다.
        // invalid 테이블을 같이 비워야 UI나 reducer가 오래된 "무효" 상태를 현재 lease 위에 덧씌우지 않는다.
        transaction
            .execute(
                "DELETE FROM runtime_invalid_slot_leases WHERE slot_id = ?1",
                params![lease.slot_id],
            )
            .with_context(|| {
                format!(
                    "failed to clear invalid runtime slot lease `{}`",
                    lease.slot_id
                )
            })?;
        // current row는 최신 상태만 보존하므로, 무엇이 저장되었는지 추적할 감사/디버그 이벤트를 별도로 남긴다.
        append_runtime_event(
            &transaction,
            "slot_lease_upsert",
            "slot_lease",
            &lease.slot_id,
            &format!(
                "runtime slot lease stored / slot: {} / state: {}",
                lease.slot_id,
                lease.state.label()
            ),
            &serde_json::to_string(lease)
                .context("failed to serialize runtime slot lease event payload")?,
        )?;
        // commit이 끝나야 snapshot loader가 lease row와 event log를 같은 시점의 결과로 볼 수 있다.
        transaction
            .commit()
            .context("failed to commit runtime slot lease transaction")?;

        Ok(())
    }

    // 더 이상 현재 상태로 보여주면 안 되는 slot lease를 runtime projection에서 제거한다.
    // 삭제 이벤트는 실제 row를 지웠을 때만 남겨, 없는 lease를 반복 삭제하는 호출이 이벤트 로그를 부풀리지 않게 한다.
    pub(crate) fn remove_runtime_slot_lease(workspace_dir: &str, slot_id: &str) -> Result<()> {
        // 제거할 slot lease가 저장된 authority DB 위치를 workspace에서 계산한다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // current row 삭제, invalid marker 정리, event append를 한 트랜잭션으로 처리한다.
        let mut connection = open_authority_connection(&location)?;

        // 트랜잭션을 사용해 snapshot 조회자가 삭제 전/후 상태가 섞인 중간 결과를 보지 않게 한다.
        let transaction = connection
            .transaction()
            .context("failed to open runtime slot lease removal transaction")?;
        // projection 변경 시각을 갱신해 외부 관찰자가 snapshot을 다시 읽을 근거를 남긴다.
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        // 삭제된 행 수로 "실제로 현재 lease가 있었는지"를 판단한다.
        // 이 값은 아래에서 이벤트를 남길지 결정하는 신호이다.
        let deleted_rows = transaction
            .execute(
                "DELETE FROM runtime_slot_leases WHERE slot_id = ?1",
                params![slot_id],
            )
            .with_context(|| format!("failed to delete runtime slot lease `{slot_id}`"))?;
        // lease를 제거하면 같은 slot에 남아 있던 invalid marker도 더 이상 보여줄 현재 대상이 없다.
        // 두 테이블을 같이 정리해 snapshot 조립 단계의 상태 해석을 단순하게 유지한다.
        transaction
            .execute(
                "DELETE FROM runtime_invalid_slot_leases WHERE slot_id = ?1",
                params![slot_id],
            )
            .with_context(|| format!("failed to clear invalid runtime slot lease `{slot_id}`"))?;
        // 실제 row가 있었을 때만 제거 이벤트를 남긴다.
        // 호출자가 cleanup을 여러 번 반복해도 runtime_events는 의미 있는 전이만 담게 된다.
        if deleted_rows > 0 {
            append_runtime_event(
                &transaction,
                "slot_lease_removed",
                "slot_lease",
                slot_id,
                &format!("runtime slot lease removed / slot: {slot_id}"),
                "{}",
            )?;
        }
        // commit 후 snapshot loader는 해당 slot이 current lease 목록에서 빠진 상태를 읽는다.
        transaction
            .commit()
            .context("failed to commit runtime slot lease removal transaction")?;

        Ok(())
    }

    pub(crate) fn clear_parallel_runtime_projections(
        workspace_dir: &str,
        reason: &str,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open parallel runtime reset transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        let dispatch_block_rows = preserve_failed_start_dispatch_blocks(&transaction)
            .context("failed to preserve failed-start dispatch blocks before runtime reset")?;
        let slot_rows = transaction
            .execute("DELETE FROM runtime_slot_leases", [])
            .context("failed to clear runtime slot leases")?;
        let invalid_rows = transaction
            .execute("DELETE FROM runtime_invalid_slot_leases", [])
            .context("failed to clear invalid runtime slot leases")?;
        let session_rows = transaction
            .execute("DELETE FROM runtime_session_details", [])
            .context("failed to clear runtime session details")?;
        let queue_rows = transaction
            .execute("DELETE FROM runtime_distributor_queue", [])
            .context("failed to clear runtime distributor queue")?;
        let claim_rows = transaction
            .execute(
                "DELETE FROM runtime_claims
                 WHERE claim_kind IN (?1, ?2)
                    OR scope_key = ?3",
                params![
                    DISTRIBUTOR_QUEUE_CLAIM_KIND,
                    "official-refresh",
                    OFFICIAL_REFRESH_SCOPE_KEY
                ],
            )
            .context("failed to clear parallel runtime claims")?;
        append_runtime_event(
            &transaction,
            "parallel_runtime_reset",
            "parallel_runtime",
            "pool",
            &format!(
                "parallel runtime reset / leases: {slot_rows} / invalid: {invalid_rows} / sessions: {session_rows} / queue: {queue_rows} / claims: {claim_rows} / dispatch_blocks_preserved: {dispatch_block_rows} / reason: {reason}"
            ),
            "{}",
        )?;
        transaction
            .commit()
            .context("failed to commit parallel runtime reset transaction")?;

        Ok(())
    }

    pub(crate) fn apply_parallel_pool_reset_report(
        workspace_dir: &str,
        report: &ParallelModePoolResetReport,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction()
            .context("failed to open parallel pool reset report transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        let dispatch_block_rows = preserve_failed_start_dispatch_blocks(&transaction)
            .context("failed to preserve failed-start dispatch blocks before pool reset report")?;

        let reset_slot_ids = report.succeeded_reset_slot_ids();
        let mut lease_rows = 0;
        let mut invalid_rows = 0;
        let mut session_rows = 0;
        let mut queue_rows = 0;
        let mut claim_rows = 0;

        for slot_id in &reset_slot_ids {
            lease_rows += transaction
                .execute(
                    "DELETE FROM runtime_slot_leases WHERE slot_id = ?1",
                    params![slot_id],
                )
                .with_context(|| format!("failed to clear reset slot lease `{slot_id}`"))?;
            invalid_rows += transaction
                .execute(
                    "DELETE FROM runtime_invalid_slot_leases WHERE slot_id = ?1",
                    params![slot_id],
                )
                .with_context(|| format!("failed to clear invalid reset slot lease `{slot_id}`"))?;
        }

        for session_key in &report.reset_session_keys {
            session_rows += transaction
                .execute(
                    "DELETE FROM runtime_session_details WHERE session_key = ?1",
                    params![session_key],
                )
                .with_context(|| format!("failed to clear reset session detail `{session_key}`"))?;
        }

        for queue_item_id in &report.reset_queue_item_ids {
            queue_rows += transaction
                .execute(
                    "DELETE FROM runtime_distributor_queue WHERE queue_item_id = ?1",
                    params![queue_item_id],
                )
                .with_context(|| {
                    format!("failed to clear reset distributor queue item `{queue_item_id}`")
                })?;
            claim_rows += transaction
                .execute(
                    "DELETE FROM runtime_claims
                     WHERE claim_kind = ?1
                       AND scope_key = ?2",
                    params![DISTRIBUTOR_QUEUE_CLAIM_KIND, queue_item_id],
                )
                .with_context(|| {
                    format!("failed to clear reset distributor claim `{queue_item_id}`")
                })?;
        }

        append_runtime_event(
            &transaction,
            "parallel_pool_reset_report_applied",
            "parallel_runtime",
            report.run_id.as_str(),
            &format!(
                "parallel pool reset report applied / reset_slots: {} / live_blockers: {} / failures: {} / leases: {lease_rows} / invalid: {invalid_rows} / sessions: {session_rows} / queue: {queue_rows} / claims: {claim_rows} / dispatch_blocks_preserved: {dispatch_block_rows}",
                reset_slot_ids.len(),
                report.live_blocker_count(),
                report.failed_reset_count()
            ),
            &serde_json::to_string(report)
                .context("failed to serialize parallel pool reset report event payload")?,
        )?;
        transaction
            .commit()
            .context("failed to commit parallel pool reset report transaction")?;

        Ok(())
    }

    // parallel agent session의 상세 상태를 runtime projection에 저장한다.
    // session_key 단위로 최신 row를 유지하고, slot_id는 어떤 slot에서 실행 중인 session인지 빠르게 필터링하는 색인 역할을 한다.
    pub(crate) fn upsert_runtime_session_detail(
        // session detail row를 저장할 authority DB를 찾는 workspace 경로이다.
        workspace_dir: &str,
        // domain이 만든 session 상태 snapshot이다. adapter는 이 값을 JSON으로 저장하고 다시 복원한다.
        detail: &ParallelModeAgentSessionDetailSnapshot,
    ) -> Result<()> {
        // workspace 경계를 DB 위치로 바꿔 같은 실행군의 runtime projection을 한 파일에 모은다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // session row upsert와 event append를 원자적으로 처리하기 위해 connection을 mutable로 연다.
        let mut connection = open_authority_connection(&location)?;

        // content column에는 전체 snapshot을 JSON으로 담는다.
        // 개별 column은 조회/정렬에 필요한 최소 필드이고, 나머지 상세 내용은 JSON이 원본 구조를 보존한다.
        let payload_json = serde_json::to_string(detail)
            .context("failed to serialize runtime session detail projection")?;
        // current row와 event log가 서로 다른 상태를 가리키지 않도록 같은 트랜잭션에 넣는다.
        let transaction = connection
            .transaction()
            .context("failed to open runtime session detail transaction")?;
        // runtime projection 변경이므로 claim용 metadata가 아니라 projection용 metadata를 갱신한다.
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        // session_key가 같은 row는 업데이트해 한 session의 최신 상태만 남긴다.
        // slot 이동이나 상태 변화가 생기면 slot_id, updated_at, content가 함께 새 snapshot으로 교체된다.
        transaction
            .execute(
                "INSERT INTO runtime_session_details (session_key, slot_id, updated_at, content)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(session_key) DO UPDATE
                 SET slot_id = excluded.slot_id,
                     updated_at = excluded.updated_at,
                     content = excluded.content",
                params![
                    detail.session_key,
                    detail.slot_id,
                    detail.updated_at,
                    payload_json
                ],
            )
            .with_context(|| {
                format!(
                    "failed to persist runtime session detail `{}`",
                    detail.session_key
                )
            })?;
        // current row만 보면 이전 상태 전이를 알 수 없으므로 upsert 이벤트를 남긴다.
        // event payload도 같은 snapshot JSON을 담아 나중에 진단할 때 당시 값을 재현할 수 있다.
        append_runtime_event(
            &transaction,
            "session_detail_upsert",
            "session_detail",
            &detail.session_key,
            &format!(
                "runtime session detail stored / session: {} / state: {}",
                detail.session_key, detail.state_label
            ),
            &serde_json::to_string(detail)
                .context("failed to serialize runtime session detail event payload")?,
        )?;
        // commit이 성공해야 snapshot loader가 session row와 event를 함께 관찰한다.
        transaction
            .commit()
            .context("failed to commit runtime session detail transaction")?;

        Ok(())
    }

    pub(crate) fn upsert_runtime_task_dispatch_block(
        workspace_dir: &str,
        block: &ParallelModeTaskDispatchBlockSnapshot,
    ) -> Result<()> {
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let payload_json = serde_json::to_string(block)
            .context("failed to serialize runtime task dispatch block projection")?;
        let transaction = connection
            .transaction()
            .context("failed to open runtime task dispatch block transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        let changed_rows = transaction
            .execute(
                "INSERT INTO runtime_task_dispatch_blocks
                    (task_id, reason, task_updated_at, blocked_at, content)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(task_id) DO UPDATE
                 SET reason = excluded.reason,
                     task_updated_at = excluded.task_updated_at,
                     blocked_at = excluded.blocked_at,
                     content = excluded.content
                 WHERE excluded.blocked_at >= runtime_task_dispatch_blocks.blocked_at",
                params![
                    block.task_id,
                    block.reason.label(),
                    block.task_updated_at,
                    block.blocked_at,
                    payload_json
                ],
            )
            .with_context(|| {
                format!(
                    "failed to persist runtime task dispatch block `{}`",
                    block.task_id
                )
            })?;
        if changed_rows > 0 {
            append_runtime_event(
                &transaction,
                "task_dispatch_block_upsert",
                "task_dispatch_block",
                &block.task_id,
                &format!(
                    "runtime task dispatch block stored / task: {} / reason: {}",
                    block.task_id,
                    block.reason.label()
                ),
                &payload_json,
            )?;
        }
        transaction
            .commit()
            .context("failed to commit runtime task dispatch block transaction")?;
        Ok(())
    }

    // distributor queue의 현재 item 상태를 runtime projection에 저장한다.
    // queue_item_id 단위로 최신 row를 유지하고, queue_state column은 UI/worker가 JSON을 열지 않고도 상태별로 볼 수 있게 한다.
    pub(crate) fn upsert_runtime_distributor_queue_record(
        // distributor queue projection을 저장할 authority DB를 찾는 workspace 경로이다.
        workspace_dir: &str,
        // application port에서 정의한 queue item snapshot이다.
        // 이 adapter는 port 타입을 DB row와 JSON content로 매핑한다.
        record: &PlanningAuthorityDistributorQueueRecord,
    ) -> Result<()> {
        // workspace를 실제 authority DB 위치로 변환해 queue 상태를 같은 저장소에 모은다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // row upsert와 event append를 하나의 트랜잭션으로 묶기 위해 mutable connection을 연다.
        let mut connection = open_authority_connection(&location)?;

        // content에는 queue record 전체를 저장하고, 주요 조회 필드만 별도 column으로 중복 저장한다.
        // 이 중복은 schema가 projection 조회를 빠르게 하면서도 원본 port 타입을 잃지 않게 하는 절충이다.
        let payload_json = serde_json::to_string(record)
            .context("failed to serialize runtime distributor queue projection")?;
        // 이 트랜잭션 안에서 current queue row와 event log가 같은 record를 기준으로 갱신된다.
        let transaction = connection
            .transaction()
            .context("failed to open runtime distributor queue transaction")?;
        // projection 변경 시각을 갱신해 외부 polling이 queue snapshot을 다시 읽을 수 있게 한다.
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        // queue_item_id가 같은 row는 최신 상태로 덮어쓴다.
        // session_key와 queue_state를 column으로 둬 session별/상태별 queue 목록을 만들 때 JSON 파싱을 피한다.
        transaction
            .execute(
                "INSERT INTO runtime_distributor_queue
                 (queue_item_id, session_key, queue_state, enqueued_at, updated_at, content)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(queue_item_id) DO UPDATE
                 SET session_key = excluded.session_key,
                     queue_state = excluded.queue_state,
                     enqueued_at = excluded.enqueued_at,
                     updated_at = excluded.updated_at,
                     content = excluded.content",
                params![
                    record.queue_item_id,
                    record.session_key,
                    record.queue_state.label(),
                    record.enqueued_at,
                    record.updated_at,
                    payload_json
                ],
            )
            .with_context(|| {
                format!(
                    "failed to persist runtime distributor queue record `{}`",
                    record.queue_item_id
                )
            })?;
        // queue row는 최신 상태만 남으므로, 상태 전이를 추적할 이벤트를 같이 기록한다.
        append_runtime_event(
            &transaction,
            "distributor_queue_upsert",
            "distributor_queue",
            &record.queue_item_id,
            &format!(
                "runtime distributor queue stored / item: {} / state: {}",
                record.queue_item_id,
                record.queue_state.label()
            ),
            &serde_json::to_string(record)
                .context("failed to serialize runtime distributor queue event payload")?,
        )?;
        // commit 후 queue snapshot과 event stream이 같은 upsert 결과를 보여준다.
        transaction
            .commit()
            .context("failed to commit runtime distributor queue transaction")?;

        Ok(())
    }
}

// runtime projection 테이블들을 한 번씩 읽어 application port가 요구하는 snapshot 구조로 조립한다.
// 저장 함수들은 table별 current row를 유지하고, 이 함수는 그 row들을 domain/application 타입으로 되돌리는 반대편이다.
fn load_runtime_projection_snapshot(
    // 호출자가 이미 authority DB를 열어 전달한다. 이 helper는 위치 해석을 모르고 row 조립에만 집중한다.
    connection: &Connection,
) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
    // slot lease는 slot_id로 바로 찾는 current map이 필요하므로 BTreeMap에 담는다.
    // BTreeMap은 key 정렬을 보장해 snapshot 출력과 테스트 결과가 안정적이다.
    let mut slot_leases = BTreeMap::new();
    // invalid slot은 중복 없는 id 집합이면 충분하고, BTreeSet은 정렬된 집합으로 직렬화/표시 순서를 고정한다.
    let mut invalid_slot_leases = BTreeSet::new();
    // session detail은 SQL의 updated_at DESC 순서를 유지해야 하므로 Vec에 push한다.
    let mut session_details = Vec::new();
    // task dispatch block은 pool reset과 별도로 유지되는 orchestrator read model이다.
    let mut task_dispatch_blocks = Vec::new();
    // distributor queue도 enqueued_at 순서가 의미 있으므로 Vec 순서를 그대로 snapshot 순서로 사용한다.
    let mut distributor_queue_records = Vec::new();

    // slot lease 테이블에서 id와 JSON content만 읽는다.
    // updated_at은 저장 시각으로는 유용하지만 domain snapshot은 JSON content 안의 값을 기준으로 복원된다.
    let mut slot_statement = connection
        .prepare("SELECT slot_id, content FROM runtime_slot_leases ORDER BY slot_id")
        .context("failed to read runtime slot leases")?;
    // rusqlite의 `query_map`은 각 row를 Rust tuple로 바꾸는 iterator를 만든다.
    // 여기서는 아직 JSON 파싱을 하지 않고 DB column decode 실패와 content parse 실패를 분리한다.
    let slot_rows = slot_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate runtime slot leases")?;
    // 각 row의 content를 원래 저장했던 `ParallelModeSlotLeaseSnapshot`으로 복원해 slot_id map에 넣는다.
    // row의 slot_id를 key로 쓰면 content 내부 id가 잘못됐을 때도 어느 DB row가 문제인지 오류 context에 남길 수 있다.
    for row in slot_rows {
        let (slot_id, content) = row.context("failed to decode runtime slot lease row")?;
        let lease = serde_json::from_str::<ParallelModeSlotLeaseSnapshot>(&content)
            .with_context(|| format!("failed to deserialize runtime slot lease `{slot_id}`"))?;
        slot_leases.insert(slot_id, lease);
    }

    // invalid slot lease 테이블은 slot_id 자체가 payload이다.
    // 별도 JSON을 열 필요 없이 id 집합만 snapshot에 실어 현재 lease와 함께 해석하게 한다.
    let mut invalid_slot_statement = connection
        .prepare("SELECT slot_id FROM runtime_invalid_slot_leases ORDER BY slot_id")
        .context("failed to read invalid runtime slot leases")?;
    // 단일 column row를 문자열 iterator로 바꾸고, 아래 loop에서 BTreeSet에 누적한다.
    let invalid_slot_rows = invalid_slot_statement
        .query_map([], |row| row.get::<_, String>(0))
        .context("failed to iterate invalid runtime slot leases")?;
    // BTreeSet 삽입은 중복을 자연스럽게 제거한다.
    // DB 스키마가 중복을 막더라도 snapshot 타입의 의미를 여기서 한 번 더 분명히 한다.
    for row in invalid_slot_rows {
        invalid_slot_leases.insert(row.context("failed to decode invalid runtime slot row")?);
    }

    // session detail은 최근 업데이트된 session을 먼저 보여주기 위해 updated_at 내림차순으로 읽는다.
    // 같은 시각이면 session_key 오름차순으로 tie-break해 화면/테스트 순서를 안정화한다.
    let mut session_statement = connection
        .prepare(
            "SELECT session_key, content
             FROM runtime_session_details
             ORDER BY updated_at DESC, session_key ASC",
        )
        .context("failed to read runtime session details")?;
    // session_key와 content를 함께 읽어 JSON 파싱 실패 시 어떤 session row가 깨졌는지 알려준다.
    let session_rows = session_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate runtime session details")?;
    // SQL이 정한 순서대로 Vec에 push하므로 snapshot consumer는 별도 정렬 없이 그대로 표시할 수 있다.
    for row in session_rows {
        let (session_key, content) = row.context("failed to decode runtime session detail row")?;
        session_details.push(
            // 저장 시 JSON으로 보존한 전체 session snapshot을 domain 타입으로 되돌린다.
            serde_json::from_str::<ParallelModeAgentSessionDetailSnapshot>(&content).with_context(
                // session_key를 캡처해 오류 메시지에 넣으면 깨진 row를 바로 찾을 수 있다.
                || format!("failed to deserialize runtime session detail `{session_key}`"),
            )?,
        );
    }

    let mut block_statement = connection
        .prepare(
            "SELECT task_id, content
             FROM runtime_task_dispatch_blocks
             ORDER BY blocked_at DESC, task_id ASC",
        )
        .context("failed to read runtime task dispatch blocks")?;
    let block_rows = block_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate runtime task dispatch blocks")?;
    for row in block_rows {
        let (task_id, content) = row.context("failed to decode runtime task dispatch block row")?;
        task_dispatch_blocks.push(
            serde_json::from_str::<ParallelModeTaskDispatchBlockSnapshot>(&content).with_context(
                || format!("failed to deserialize runtime task dispatch block `{task_id}`"),
            )?,
        );
    }

    // distributor queue는 오래 들어온 item부터 처리/표시해야 하므로 enqueued_at 오름차순으로 읽는다.
    // 같은 enqueue 시각이면 queue_item_id로 tie-break한다.
    let mut queue_statement = connection
        .prepare(
            "SELECT queue_item_id, content
             FROM runtime_distributor_queue
             ORDER BY enqueued_at ASC, queue_item_id ASC",
        )
        .context("failed to read runtime distributor queue records")?;
    // queue_item_id와 content를 함께 읽어 decode와 deserialize 오류 위치를 분리한다.
    let queue_rows = queue_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate runtime distributor queue records")?;
    // queue Vec의 순서는 SQL 처리 순서와 같다.
    // worker나 UI가 이 snapshot을 읽으면 먼저 enqueue된 item을 앞에서 보게 된다.
    for row in queue_rows {
        let (queue_item_id, content) =
            row.context("failed to decode runtime distributor queue row")?;
        distributor_queue_records.push(
            // 저장된 JSON을 application port record로 복원해 DB schema 바깥에는 port 타입만 보이게 한다.
            serde_json::from_str::<PlanningAuthorityDistributorQueueRecord>(&content)
                .with_context(|| {
                    format!(
                        "failed to deserialize runtime distributor queue record `{queue_item_id}`"
                    )
                })?,
        );
    }

    // 네 projection 영역을 하나의 snapshot으로 묶어 application service가 DB를 몰라도 런타임 상태를 읽게 한다.
    Ok(PlanningAuthorityRuntimeProjectionSnapshot {
        slot_leases,
        invalid_slot_leases,
        session_details,
        task_dispatch_blocks,
        distributor_queue_records,
    })
}

fn load_runtime_event_log_snapshot(
    connection: &Connection,
    request: &ParallelModeRuntimeEventLogRequest,
) -> Result<ParallelModeRuntimeEventsSnapshot> {
    let projection_kind = request.projection_kind.as_deref();
    let projection_key = request.projection_key.as_deref();
    let total_event_count = connection
        .query_row(
            "SELECT COUNT(*)
             FROM runtime_events
             WHERE (?1 IS NULL OR projection_kind = ?1)
               AND (?2 IS NULL OR projection_key = ?2)",
            params![projection_kind, projection_key],
            |row| row.get::<_, i64>(0),
        )
        .context("failed to count runtime events")?;
    let limit = request.bounded_limit() as i64;
    let mut statement = connection
        .prepare(
            "SELECT sequence,
                    event_kind,
                    projection_kind,
                    projection_key,
                    observed_planning_revision,
                    summary,
                    recorded_at
             FROM runtime_events
             WHERE (?1 IS NULL OR projection_kind = ?1)
               AND (?2 IS NULL OR projection_key = ?2)
             ORDER BY sequence DESC
             LIMIT ?3",
        )
        .context("failed to read runtime events")?;
    let rows = statement
        .query_map(params![projection_kind, projection_key, limit], |row| {
            Ok(ParallelModeRuntimeEventEntry::new(
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .context("failed to iterate runtime events")?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.context("failed to decode runtime event row")?);
    }

    let total_event_count = total_event_count.max(0) as usize;
    let empty_state = if total_event_count > 0 && limit == 0 {
        "runtime events hidden by request limit"
    } else {
        "no runtime events captured yet"
    };
    Ok(ParallelModeRuntimeEventsSnapshot::new(
        entries,
        total_event_count,
        empty_state,
    ))
}

fn preserve_failed_start_dispatch_blocks(transaction: &Transaction<'_>) -> Result<usize> {
    let mut preserved_rows = 0;
    let mut statement = transaction
        .prepare("SELECT session_key, content FROM runtime_session_details")
        .context("failed to read session details before runtime reset")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate session details before runtime reset")?;

    for row in rows {
        let (session_key, content) =
            row.context("failed to decode session detail before runtime reset")?;
        let detail = serde_json::from_str::<ParallelModeAgentSessionDetailSnapshot>(&content)
            .with_context(|| {
                format!("failed to deserialize session detail `{session_key}` before reset")
            })?;
        if !is_failed_start_session_detail(&detail) {
            continue;
        }
        let block = ParallelModeTaskDispatchBlockSnapshot::new(
            detail.task_id.clone(),
            String::new(),
            detail.updated_at.clone(),
            crate::domain::parallel_mode::ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
        );
        let payload_json = serde_json::to_string(&block)
            .context("failed to serialize preserved task dispatch block")?;
        let changed_rows = transaction
            .execute(
                "INSERT INTO runtime_task_dispatch_blocks
                    (task_id, reason, task_updated_at, blocked_at, content)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(task_id) DO UPDATE
                 SET reason = excluded.reason,
                     task_updated_at = excluded.task_updated_at,
                     blocked_at = excluded.blocked_at,
                     content = excluded.content
                 WHERE excluded.blocked_at >= runtime_task_dispatch_blocks.blocked_at",
                params![
                    block.task_id,
                    block.reason.label(),
                    block.task_updated_at,
                    block.blocked_at,
                    payload_json
                ],
            )
            .with_context(|| {
                format!(
                    "failed to preserve failed-start task dispatch block `{}`",
                    detail.task_id
                )
            })?;
        preserved_rows += changed_rows;
    }

    Ok(preserved_rows)
}

fn is_failed_start_session_detail(detail: &ParallelModeAgentSessionDetailSnapshot) -> bool {
    detail.state_label == "failed"
        && detail.completion_state_label == "aborted"
        && detail
            .latest_summary
            .contains("launch failed before the session reached the running state")
}

// runtime projection의 current row 변경을 시간순 이벤트로 남긴다.
// 호출자는 이미 트랜잭션을 열어 두었고, 이 helper는 같은 트랜잭션 안에서 sequence 증가와 event insert를 함께 수행한다.
fn append_runtime_event(
    // current projection row를 저장/삭제하는 트랜잭션이다. event만 따로 commit되지 않게 빌려 쓴다.
    transaction: &rusqlite::Transaction<'_>,
    // upsert/remove 같은 event 동작 종류이다.
    event_kind: &str,
    // slot_lease, session_detail, distributor_queue처럼 어떤 projection 영역의 이벤트인지 나타낸다.
    projection_kind: &str,
    // projection 영역 안에서 대상 row를 찾는 key이다. 예를 들면 slot_id나 session_key이다.
    projection_key: &str,
    // 사람이 로그를 훑을 때 바로 이해할 수 있는 짧은 설명이다.
    summary: &str,
    // 당시 projection payload를 JSON으로 보존해 current row가 나중에 덮여도 과거 값을 추적할 수 있게 한다.
    payload_json: &str,
) -> Result<()> {
    // runtime_event_sequence metadata는 이벤트 로그의 단조 증가 번호이다.
    // runtime_events 테이블의 recorded_at만으로는 동시 저장 순서를 완전히 설명하기 어려워 sequence를 따로 둔다.
    let sequence = read_metadata_i64(transaction, "runtime_event_sequence")?.unwrap_or(0) + 1;
    // 이벤트가 기록될 당시의 planning_revision을 같이 저장한다.
    // 나중에 런타임 변화가 어떤 planning authority 버전을 보고 일어났는지 연결하는 단서이다.
    let observed_planning_revision =
        read_metadata_i64(transaction, "planning_revision")?.unwrap_or(0);
    // 다음 이벤트가 같은 sequence를 쓰지 않도록 insert 전에 metadata를 갱신한다.
    // 같은 트랜잭션이므로 insert 실패 시 sequence 증가도 rollback된다.
    upsert_metadata(transaction, "runtime_event_sequence", &sequence.to_string())?;
    // 이벤트 row에는 정렬용 sequence, 대상 식별자, 사람이 읽는 summary, 원본 payload가 함께 들어간다.
    transaction
        .execute(
            "INSERT INTO runtime_events
             (sequence, event_kind, projection_kind, projection_key, observed_planning_revision, summary, payload_json, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                sequence,
                event_kind,
                projection_kind,
                projection_key,
                observed_planning_revision,
                summary,
                payload_json,
                // recorded_at은 사람이 보는 시간 축이고, sequence는 DB 안에서의 엄격한 순서 축이다.
                Utc::now().to_rfc3339()
            ],
        )
        .with_context(|| {
            format!(
                "failed to append runtime event `{event_kind}` for `{projection_kind}:{projection_key}`"
            )
        })?;
    Ok(())
}

// claim helper가 DB에서 읽는 최소 필드 묶음이다.
// Debug 파생은 오류 추적이나 테스트 진단에서 구조체 내용을 쉽게 볼 수 있게 한다.
#[derive(Debug)]
struct RuntimeClaimRecord {
    // 현재 클레임을 잡은 worker/token이다. 재진입과 경합 판단에 쓰인다.
    owner_token: String,
    // 클레임을 잡은 시각이다. stale 판정은 이 값과 현재 UTC 시각의 차이로 결정된다.
    claimed_at: String,
}

// runtime_claims에서 특정 kind/scope의 현재 소유권 row를 읽는다.
// row가 없으면 오류가 아니라 None으로 반환해 caller가 "비어 있음"과 "DB 실패"를 구분하게 한다.
fn load_runtime_claim(
    // acquire/release 함수가 열어 둔 트랜잭션이다. 같은 경합 구간 안에서 claim을 읽는다.
    transaction: &rusqlite::Transaction<'_>,
    // official-refresh인지 distributor-queue인지 같은 클레임 분류이다.
    claim_kind: &str,
    // 같은 kind 안에서 충돌 범위를 나누는 key이다. official refresh는 전역 key, queue는 item id를 쓴다.
    scope_key: &str,
) -> Result<Option<RuntimeClaimRecord>> {
    // owner와 claimed_at만 읽으면 acquire 쪽의 재진입/경합 판단과 stale 판단에 충분하다.
    transaction
        .query_row(
            "SELECT owner_token, claimed_at
             FROM runtime_claims
             WHERE claim_kind = ?1 AND scope_key = ?2",
            params![claim_kind, scope_key],
            // row callback은 SQLite column을 작은 Rust record로 매핑하는 adapter 경계이다.
            |row| {
                Ok(RuntimeClaimRecord {
                    owner_token: row.get::<_, String>(0)?,
                    claimed_at: row.get::<_, String>(1)?,
                })
            },
        )
        // OptionalExtension이 QueryReturnedNoRows를 Ok(None)으로 바꿔 "클레임 없음"을 정상 상태로 표현한다.
        .optional()
        .with_context(|| format!("failed to read runtime claim `{claim_kind}:{scope_key}`"))
}

// 오래된 runtime claim을 발견하면 지워서 다른 worker가 다시 소유권을 잡을 수 있게 한다.
// 반환 bool은 실제 삭제가 있었는지 알려줘 caller가 metadata heartbeat를 추가로 갱신할지 결정하게 한다.
fn clear_stale_runtime_claim(
    // stale 확인과 삭제는 acquire 트랜잭션 안에서 실행되어 경합 창을 줄인다.
    transaction: &rusqlite::Transaction<'_>,
    // 지울 후보의 클레임 분류이다.
    claim_kind: &str,
    // 지울 후보의 충돌 범위 key이다.
    scope_key: &str,
) -> Result<bool> {
    // row가 없으면 정리할 것도 없으므로 false를 반환한다.
    let Some(existing_claim) = load_runtime_claim(transaction, claim_kind, scope_key)? else {
        return Ok(false);
    };
    // 아직 stale 임계값을 넘지 않은 클레임은 살아 있는 owner로 간주하고 건드리지 않는다.
    if !claim_is_stale(&existing_claim.claimed_at) {
        return Ok(false);
    }

    // stale이라고 판단된 row만 kind/scope 기준으로 삭제한다.
    // owner_token을 조건에 넣지 않는 이유는 "이 scope의 오래된 소유권을 회수한다"가 목적이기 때문이다.
    transaction
        .execute(
            "DELETE FROM runtime_claims WHERE claim_kind = ?1 AND scope_key = ?2",
            params![claim_kind, scope_key],
        )
        .with_context(|| {
            format!("failed to clear stale runtime claim `{claim_kind}:{scope_key}`")
        })?;
    Ok(true)
}

// claimed_at 문자열이 stale 임계값을 넘었는지 판단한다.
// 파싱할 수 없는 timestamp는 안전하게 stale로 취급해 영구히 회수되지 않는 클레임을 만들지 않는다.
fn claim_is_stale(claimed_at: &str) -> bool {
    // DB에는 RFC3339 문자열로 저장했으므로 먼저 timezone이 포함된 DateTime으로 파싱한다.
    chrono::DateTime::parse_from_rfc3339(claimed_at)
        // 파싱에 성공하면 UTC 기준 현재 시각과 claimed_at의 차이를 초 단위로 계산한다.
        .map(|timestamp| {
            Utc::now()
                .signed_duration_since(timestamp.with_timezone(&Utc))
                .num_seconds()
                >= CLAIM_STALE_AFTER_SECS
        })
        // timestamp가 깨졌다면 owner 생존을 신뢰할 수 없으므로 stale=true로 회수 가능하게 둔다.
        .unwrap_or(true)
}

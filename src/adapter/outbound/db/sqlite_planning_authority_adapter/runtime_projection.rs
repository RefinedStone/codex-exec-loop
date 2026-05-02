// 학습 주석: 런타임 투영은 DB에 저장된 여러 행을 메모리 스냅샷으로 다시 조립하므로,
// 중복 제거와 정렬된 출력을 동시에 제공하는 BTree 계열 컬렉션을 사용합니다.
use std::collections::{BTreeMap, BTreeSet};

// 학습 주석: 이 모듈의 public helper들은 모두 outbound adapter 경계에서 실패할 수 있으므로
// `anyhow::Result`로 오류를 올리고, `Context`로 SQLite 단계 이름을 붙입니다.
use anyhow::{Context, Result};
// 학습 주석: 클레임 만료와 큐 처리 시각은 DB 행에 문자열로 남기기 때문에,
// UTC 기준 RFC3339 타임스탬프를 만드는 `Utc`가 이 파일의 공통 시간 원천입니다.
use chrono::Utc;
// 학습 주석: `Connection`은 helper 함수들이 트랜잭션 밖 조회를 수행할 때 필요하고,
// `OptionalExtension`은 "행 없음"을 오류가 아닌 Option으로 바꿔 클레임 부재를 표현합니다.
use rusqlite::{Connection, OptionalExtension, params};

// 학습 주석: application port의 record/status/snapshot 타입을 그대로 반환해,
// SQLite 세부 구조가 application layer로 새지 않도록 어댑터 내부에서 매핑을 끝냅니다.
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
    PlanningAuthorityRuntimeProjectionSnapshot,
};
// 학습 주석: parallel mode의 slot lease와 agent session은 domain 타입이므로,
// 여기서는 DB row를 도메인이 이해하는 스냅샷 값으로 복원하는 역할만 맡습니다.
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
};

// 학습 주석: metadata upsert helper는 store 모듈의 스키마 관리와 같은 규칙을 공유합니다.
// 런타임 클레임도 권위 DB가 갱신되었다는 흔적을 남겨 다른 프로세스가 변화를 감지하게 합니다.
use super::store::{upsert_authority_metadata, upsert_metadata};
// 학습 주석: adapter 본체의 위치 해석/DB 열기 함수와 클레임 상수들을 가져옵니다.
// 이 파일은 `runtime_claims`, distributor queue, snapshot projection만 분리한 impl 조각입니다.
use super::{
    CLAIM_STALE_AFTER_SECS, DISTRIBUTOR_QUEUE_CLAIM_KIND, OFFICIAL_REFRESH_SCOPE_KEY,
    SqlitePlanningAuthorityAdapter, open_authority_connection, read_metadata_i64,
};

// 학습 주석: 이 impl 블록은 `SqlitePlanningAuthorityAdapter`의 런타임 상태 책임을 담습니다.
// 영구 planning authority 문서가 아니라, 여러 실행 주체가 동시에 움직일 때 필요한
// 순번, 임시 소유권, 큐 상태, agent session 투영을 SQLite에 기록하고 다시 읽습니다.
impl SqlitePlanningAuthorityAdapter {
    // 학습 주석: 공식 refresh는 여러 worker가 동시에 시작할 수 있으므로 먼저 단조 증가 순번을 예약합니다.
    // `next_official_refresh_order`는 "발급할 번호"이고, 아래에서 발급 직후 +1로 저장해 다음 호출과 충돌하지 않게 합니다.
    pub(crate) fn reserve_next_official_refresh_order(workspace_dir: &str) -> Result<u64> {
        // 학습 주석: workspace 경로만 받은 application layer 요청을 실제 authority DB 위치로 변환합니다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 학습 주석: 순번 예약은 read-modify-write라서 한 커넥션의 트랜잭션 안에서 처리해야 합니다.
        let mut connection = open_authority_connection(&location)?;
        // 학습 주석: 트랜잭션은 두 metadata 행 갱신을 한 단위로 묶습니다.
        // 중간에 실패하면 순번만 증가하거나 갱신 시각만 바뀌는 반쪽 상태가 남지 않습니다.
        let transaction = connection
            .transaction()
            .context("failed to open official refresh order transaction")?;
        // 학습 주석: `last_claim_updated_at`은 클레임/큐 계열 런타임 상태가 바뀌었다는 저렴한 변경 신호입니다.
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        // 학습 주석: 새 DB에는 metadata가 없을 수 있으므로 1을 기본 순번으로 삼습니다.
        // 반환값은 현재 호출자가 처리할 refresh 번호이고, 저장값은 다음 호출자를 위한 번호입니다.
        let next_refresh_order =
            read_metadata_i64(&transaction, "next_official_refresh_order")?.unwrap_or(1);
        upsert_metadata(
            &transaction,
            "next_official_refresh_order",
            &(next_refresh_order + 1).to_string(),
        )?;
        // 학습 주석: commit이 성공해야만 예약된 순번이 다른 프로세스에 보입니다.
        transaction
            .commit()
            .context("failed to commit official refresh order transaction")?;
        // 학습 주석: application layer는 이 순번을 다시 `acquire_official_refresh_claim`에 넘겨
        // 자기 차례가 되었는지 확인하고 실제 refresh 작업을 시작합니다.
        Ok(next_refresh_order as u64)
    }

    // 학습 주석: 예약된 refresh 순번이 실행 가능한 차례인지 확인하고, 가능하면 단일 owner 클레임을 잡습니다.
    // 반환 status는 caller가 "이미 끝남/아직 기다림/내가 실행함"을 구분해 busy loop 없이 조율하게 해줍니다.
    pub(crate) fn acquire_official_refresh_claim(
        // 학습 주석: workspace는 authority DB를 찾기 위한 application-level 식별자입니다.
        workspace_dir: &str,
        // 학습 주석: `reserve_next_official_refresh_order`에서 받은 번호입니다.
        // 이 값이 `next_executable_refresh_order`와 같을 때만 실행권을 시도할 수 있습니다.
        refresh_order: u64,
        // 학습 주석: 같은 프로세스/작업이 재진입했을 때 자기 클레임을 알아보기 위한 소유자 토큰입니다.
        owner_token: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshClaimStatus> {
        // 학습 주석: 모든 판정은 동일한 authority DB에서 이루어져야 하므로 먼저 위치를 고정합니다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 학습 주석: 클레임 확인, stale 정리, 삽입은 경합 구간이므로 한 트랜잭션에 넣습니다.
        let mut connection = open_authority_connection(&location)?;
        // 학습 주석: 트랜잭션 경계가 이 함수의 분산 락 단위입니다.
        // commit하기 전까지 다른 실행자는 이 함수의 삽입/삭제 결과를 관찰하지 못합니다.
        let transaction = connection
            .transaction()
            .context("failed to open official refresh claim transaction")?;
        // 학습 주석: 시도 자체도 런타임 클레임 영역의 활동이므로 heartbeat metadata를 갱신합니다.
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        // 학습 주석: `next_executable_refresh_order`는 완료된 refresh 다음 번호를 가리킵니다.
        // 기본값 1은 아직 어떤 공식 refresh도 완료되지 않았다는 뜻입니다.
        let next_executable =
            read_metadata_i64(&transaction, "next_executable_refresh_order")?.unwrap_or(1);
        // 학습 주석: 요청 순번이 실행 포인터보다 작으면 이미 누군가 완료 처리한 작업입니다.
        // DB를 바꿀 필요가 없으므로 rollback으로 읽기 트랜잭션을 닫고 상태만 알려줍니다.
        if (refresh_order as i64) < next_executable {
            transaction
                .rollback()
                .context("failed to roll back completed official refresh claim transaction")?;
            return Ok(PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted);
        }
        // 학습 주석: 요청 순번이 실행 포인터보다 크면 앞선 refresh가 아직 끝나지 않았습니다.
        // 순서를 강제하기 위해 현재 caller는 클레임을 만들지 않고 대기 상태를 받습니다.
        if (refresh_order as i64) > next_executable {
            transaction
                .rollback()
                .context("failed to roll back waiting official refresh claim transaction")?;
            return Ok(PlanningAuthorityOfficialRefreshClaimStatus::Waiting);
        }

        // 학습 주석: 실행 가능한 순번이라도 이전 owner가 죽고 클레임만 남았을 수 있습니다.
        // stale이면 제거한 뒤 metadata를 다시 만져 polling 쪽이 "상태 변화"를 볼 수 있게 합니다.
        if clear_stale_runtime_claim(&transaction, "official-refresh", OFFICIAL_REFRESH_SCOPE_KEY)?
        {
            upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        }
        // 학습 주석: 공식 refresh는 scope key가 하나뿐인 전역 클레임입니다.
        // 이미 row가 있으면 owner_token만 비교해 재진입인지 경합인지 판단합니다.
        let existing_owner =
            load_runtime_claim(&transaction, "official-refresh", OFFICIAL_REFRESH_SCOPE_KEY)?
                .map(|claim| claim.owner_token);
        // 학습 주석: 같은 owner가 이미 잡은 클레임이면 멱등 성공으로 처리합니다.
        // 이 덕분에 caller가 네트워크/프로세스 경계에서 같은 시도를 반복해도 중복 실행으로 번지지 않습니다.
        if let Some(existing_owner) = existing_owner {
            if existing_owner == owner_token {
                transaction
                    .commit()
                    .context("failed to commit existing official refresh claim transaction")?;
                return Ok(PlanningAuthorityOfficialRefreshClaimStatus::Acquired);
            }
            // 학습 주석: 다른 owner가 살아있는 클레임을 잡고 있으면 현재 caller는 순서를 양보합니다.
            // rollback을 사용해 방금 갱신하려던 heartbeat도 남기지 않으므로 실제 소유권 상태를 흐리지 않습니다.
            transaction
                .rollback()
                .context("failed to roll back contended official refresh claim transaction")?;
            return Ok(PlanningAuthorityOfficialRefreshClaimStatus::Waiting);
        }

        // 학습 주석: 클레임이 없을 때만 새 row를 삽입합니다.
        // `claim_value`에 refresh_order를 문자열로 저장해 release 단계에서 "내가 잡은 그 순번"만 지우게 합니다.
        transaction
            .execute(
                "INSERT INTO runtime_claims (claim_kind, scope_key, owner_token, claim_value, claimed_at)
                 VALUES ('official-refresh', ?1, ?2, ?3, ?4)",
                params![
                    OFFICIAL_REFRESH_SCOPE_KEY,
                    owner_token,
                    refresh_order.to_string(),
                    // 학습 주석: stale 판정은 `claimed_at`과 현재 UTC 시각의 차이로 계산됩니다.
                    Utc::now().to_rfc3339()
                ],
            )
            .context("failed to acquire official refresh claim")?;
        // 학습 주석: 삽입 commit이 성공하면 caller는 official refresh 작업을 수행할 권한을 얻은 것입니다.
        transaction
            .commit()
            .context("failed to commit official refresh claim transaction")?;
        Ok(PlanningAuthorityOfficialRefreshClaimStatus::Acquired)
    }

    // 학습 주석: refresh worker가 작업을 끝낸 뒤 자기 클레임을 지우고 실행 포인터를 다음 순번으로 넘깁니다.
    // 삭제 조건에 owner와 순번을 모두 넣어, 늦게 도착한 release가 남의 새 클레임을 지우지 못하게 막습니다.
    pub(crate) fn release_official_refresh_claim(
        // 학습 주석: 어떤 authority DB에서 release할지 결정하는 workspace 경로입니다.
        workspace_dir: &str,
        // 학습 주석: 완료 처리할 refresh 순번입니다. 실행 포인터는 최소 이 값 다음으로만 이동합니다.
        refresh_order: u64,
        // 학습 주석: acquire 때 저장한 owner token입니다. 다른 owner의 클레임은 이 함수가 건드리지 않습니다.
        owner_token: &str,
    ) -> Result<()> {
        // 학습 주석: release도 acquire와 같은 DB 위치 규칙을 사용해야 순번 포인터가 한곳에서 움직입니다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 학습 주석: 클레임 삭제와 metadata 포인터 갱신을 원자적으로 묶기 위해 mutable connection을 엽니다.
        let mut connection = open_authority_connection(&location)?;
        // 학습 주석: 이 트랜잭션이 성공해야 "클레임 해제"와 "다음 refresh 실행 허용"이 함께 보입니다.
        let transaction = connection
            .transaction()
            .context("failed to open official refresh release transaction")?;
        // 학습 주석: release는 대기 중인 worker에게 중요한 변화이므로 authority metadata를 갱신합니다.
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        // 학습 주석: row가 삭제되었다면 현재 caller가 실제 소유자였다는 뜻입니다.
        // 삭제 수가 0이면 이미 stale 정리되었거나 다른 owner/순번이므로 포인터를 움직이면 안 됩니다.
        let deleted_rows = transaction
            .execute(
                "DELETE FROM runtime_claims
                 WHERE claim_kind = 'official-refresh' AND scope_key = ?1 AND owner_token = ?2 AND claim_value = ?3",
                params![OFFICIAL_REFRESH_SCOPE_KEY, owner_token, refresh_order.to_string()],
            )
            .context("failed to release official refresh claim")?;
        // 학습 주석: 실제로 삭제한 owner만 실행 포인터를 전진시킵니다.
        // 이 조건이 없으면 실패한 release 재시도가 앞선 refresh를 건너뛰는 버그가 됩니다.
        if deleted_rows > 0 {
            // 학습 주석: 이미 더 큰 순번까지 완료 처리된 경우에는 과거 release가 포인터를 되돌리지 않도록 현재값을 먼저 읽습니다.
            let next_executable =
                read_metadata_i64(&transaction, "next_executable_refresh_order")?.unwrap_or(1);
            // 학습 주석: 현재 포인터가 이 refresh 이하일 때만 다음 순번으로 올립니다.
            // 큰 값이면 다른 경로가 이미 더 앞까지 진행한 상태라 그대로 둡니다.
            if next_executable <= refresh_order as i64 {
                upsert_metadata(
                    &transaction,
                    "next_executable_refresh_order",
                    &(refresh_order + 1).to_string(),
                )?;
            }
        }
        // 학습 주석: commit 후에야 다음 순번 worker가 acquire에서 `Acquired`를 받을 수 있습니다.
        transaction
            .commit()
            .context("failed to commit official refresh release transaction")?;
        Ok(())
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(crate) fn try_acquire_distributor_queue_claim(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        queue_item_id: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        owner_token: &str,
    ) -> Result<bool> {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut connection = open_authority_connection(&location)?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let transaction = connection
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .transaction()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to open distributor queue claim transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if clear_stale_runtime_claim(&transaction, DISTRIBUTOR_QUEUE_CLAIM_KIND, queue_item_id)? {
            upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        }
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let inserted_rows = transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .execute(
                "INSERT OR IGNORE INTO runtime_claims
                 (claim_kind, scope_key, owner_token, claim_value, claimed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    DISTRIBUTOR_QUEUE_CLAIM_KIND,
                    queue_item_id,
                    owner_token,
                    queue_item_id,
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    Utc::now().to_rfc3339()
                ],
            )
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to acquire distributor queue claim")?;
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .commit()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to commit distributor queue claim transaction")?;
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(inserted_rows > 0)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(crate) fn release_distributor_queue_claim(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        queue_item_id: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        owner_token: &str,
    ) -> Result<()> {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut connection = open_authority_connection(&location)?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let transaction = connection
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .transaction()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to open distributor queue release transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_claim_updated_at")?;
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .execute(
                "DELETE FROM runtime_claims
                 WHERE claim_kind = ?1 AND scope_key = ?2 AND owner_token = ?3",
                params![DISTRIBUTOR_QUEUE_CLAIM_KIND, queue_item_id, owner_token],
            )
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to release distributor queue claim")?;
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .commit()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to commit distributor queue release transaction")?;
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(())
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(crate) fn load_runtime_projections(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let connection = open_authority_connection(&location)?;
        load_runtime_projection_snapshot(&connection)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(crate) fn upsert_runtime_slot_lease(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()> {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut connection = open_authority_connection(&location)?;

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let payload_json = serde_json::to_string(lease)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to serialize runtime slot lease projection")?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let transaction = connection
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .transaction()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to open runtime slot lease transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .execute(
                "INSERT INTO runtime_slot_leases (slot_id, updated_at, content)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(slot_id) DO UPDATE
                 SET updated_at = excluded.updated_at,
                     content = excluded.content",
                params![lease.slot_id, Utc::now().to_rfc3339(), payload_json],
            )
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .with_context(|| format!("failed to persist runtime slot lease `{}`", lease.slot_id))?;
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .execute(
                "DELETE FROM runtime_invalid_slot_leases WHERE slot_id = ?1",
                params![lease.slot_id],
            )
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .with_context(|| {
                format!(
                    "failed to clear invalid runtime slot lease `{}`",
                    lease.slot_id
                )
            })?;
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
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .context("failed to serialize runtime slot lease event payload")?,
        )?;
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .commit()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to commit runtime slot lease transaction")?;

        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(())
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(crate) fn remove_runtime_slot_lease(workspace_dir: &str, slot_id: &str) -> Result<()> {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut connection = open_authority_connection(&location)?;

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let transaction = connection
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .transaction()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to open runtime slot lease removal transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let deleted_rows = transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .execute(
                "DELETE FROM runtime_slot_leases WHERE slot_id = ?1",
                params![slot_id],
            )
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .with_context(|| format!("failed to delete runtime slot lease `{slot_id}`"))?;
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .execute(
                "DELETE FROM runtime_invalid_slot_leases WHERE slot_id = ?1",
                params![slot_id],
            )
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .with_context(|| format!("failed to clear invalid runtime slot lease `{slot_id}`"))?;
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
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
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .commit()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to commit runtime slot lease removal transaction")?;

        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(())
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(crate) fn upsert_runtime_session_detail(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        detail: &ParallelModeAgentSessionDetailSnapshot,
    ) -> Result<()> {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut connection = open_authority_connection(&location)?;

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let payload_json = serde_json::to_string(detail)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to serialize runtime session detail projection")?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let transaction = connection
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .transaction()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to open runtime session detail transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
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
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .with_context(|| {
                format!(
                    "failed to persist runtime session detail `{}`",
                    detail.session_key
                )
            })?;
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
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .context("failed to serialize runtime session detail event payload")?,
        )?;
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .commit()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to commit runtime session detail transaction")?;

        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(())
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(crate) fn upsert_runtime_distributor_queue_record(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        record: &PlanningAuthorityDistributorQueueRecord,
    ) -> Result<()> {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut connection = open_authority_connection(&location)?;

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let payload_json = serde_json::to_string(record)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to serialize runtime distributor queue projection")?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let transaction = connection
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .transaction()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to open runtime distributor queue transaction")?;
        upsert_authority_metadata(&transaction, &location, "last_runtime_projection_at")?;
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
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
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .with_context(|| {
                format!(
                    "failed to persist runtime distributor queue record `{}`",
                    record.queue_item_id
                )
            })?;
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
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .context("failed to serialize runtime distributor queue event payload")?,
        )?;
        transaction
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .commit()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .context("failed to commit runtime distributor queue transaction")?;

        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(())
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn load_runtime_projection_snapshot(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    connection: &Connection,
) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut slot_leases = BTreeMap::new();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut invalid_slot_leases = BTreeSet::new();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut session_details = Vec::new();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut distributor_queue_records = Vec::new();

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut slot_statement = connection
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .prepare("SELECT slot_id, content FROM runtime_slot_leases ORDER BY slot_id")
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to read runtime slot leases")?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let slot_rows = slot_statement
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_map([], |row| {
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to iterate runtime slot leases")?;
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for row in slot_rows {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let (slot_id, content) = row.context("failed to decode runtime slot lease row")?;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let lease = serde_json::from_str::<ParallelModeSlotLeaseSnapshot>(&content)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .with_context(|| format!("failed to deserialize runtime slot lease `{slot_id}`"))?;
        slot_leases.insert(slot_id, lease);
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut invalid_slot_statement = connection
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .prepare("SELECT slot_id FROM runtime_invalid_slot_leases ORDER BY slot_id")
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to read invalid runtime slot leases")?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let invalid_slot_rows = invalid_slot_statement
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_map([], |row| row.get::<_, String>(0))
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to iterate invalid runtime slot leases")?;
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for row in invalid_slot_rows {
        invalid_slot_leases.insert(row.context("failed to decode invalid runtime slot row")?);
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut session_statement = connection
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .prepare(
            "SELECT session_key, content
             FROM runtime_session_details
             ORDER BY updated_at DESC, session_key ASC",
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to read runtime session details")?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let session_rows = session_statement
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_map([], |row| {
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to iterate runtime session details")?;
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for row in session_rows {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let (session_key, content) = row.context("failed to decode runtime session detail row")?;
        session_details.push(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            serde_json::from_str::<ParallelModeAgentSessionDetailSnapshot>(&content).with_context(
                // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
                || format!("failed to deserialize runtime session detail `{session_key}`"),
            )?,
        );
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut queue_statement = connection
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .prepare(
            "SELECT queue_item_id, content
             FROM runtime_distributor_queue
             ORDER BY enqueued_at ASC, queue_item_id ASC",
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to read runtime distributor queue records")?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let queue_rows = queue_statement
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_map([], |row| {
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .context("failed to iterate runtime distributor queue records")?;
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for row in queue_rows {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let (queue_item_id, content) =
            row.context("failed to decode runtime distributor queue row")?;
        distributor_queue_records.push(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            serde_json::from_str::<PlanningAuthorityDistributorQueueRecord>(&content)
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .with_context(|| {
                    format!(
                        "failed to deserialize runtime distributor queue record `{queue_item_id}`"
                    )
                })?,
        );
    }

    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(PlanningAuthorityRuntimeProjectionSnapshot {
        slot_leases,
        invalid_slot_leases,
        session_details,
        distributor_queue_records,
    })
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn append_runtime_event(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    transaction: &rusqlite::Transaction<'_>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    event_kind: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    projection_kind: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    projection_key: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    summary: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    payload_json: &str,
) -> Result<()> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let sequence = read_metadata_i64(transaction, "runtime_event_sequence")?.unwrap_or(0) + 1;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let observed_planning_revision =
        read_metadata_i64(transaction, "planning_revision")?.unwrap_or(0);
    upsert_metadata(transaction, "runtime_event_sequence", &sequence.to_string())?;
    transaction
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
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
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                Utc::now().to_rfc3339()
            ],
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .with_context(|| {
            format!(
                "failed to append runtime event `{event_kind}` for `{projection_kind}:{projection_key}`"
            )
        })?;
    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(())
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
struct RuntimeClaimRecord {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    owner_token: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    claimed_at: String,
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn load_runtime_claim(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    transaction: &rusqlite::Transaction<'_>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    claim_kind: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    scope_key: &str,
) -> Result<Option<RuntimeClaimRecord>> {
    transaction
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .query_row(
            "SELECT owner_token, claimed_at
             FROM runtime_claims
             WHERE claim_kind = ?1 AND scope_key = ?2",
            params![claim_kind, scope_key],
            // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
            |row| {
                // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
                Ok(RuntimeClaimRecord {
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    owner_token: row.get::<_, String>(0)?,
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    claimed_at: row.get::<_, String>(1)?,
                })
            },
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .optional()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .with_context(|| format!("failed to read runtime claim `{claim_kind}:{scope_key}`"))
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn clear_stale_runtime_claim(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    transaction: &rusqlite::Transaction<'_>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    claim_kind: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    scope_key: &str,
) -> Result<bool> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let Some(existing_claim) = load_runtime_claim(transaction, claim_kind, scope_key)? else {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(false);
    };
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !claim_is_stale(&existing_claim.claimed_at) {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(false);
    }

    transaction
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .execute(
            "DELETE FROM runtime_claims WHERE claim_kind = ?1 AND scope_key = ?2",
            params![claim_kind, scope_key],
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .with_context(|| {
            format!("failed to clear stale runtime claim `{claim_kind}:{scope_key}`")
        })?;
    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
    Ok(true)
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn claim_is_stale(claimed_at: &str) -> bool {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    chrono::DateTime::parse_from_rfc3339(claimed_at)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|timestamp| {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Utc::now()
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .signed_duration_since(timestamp.with_timezone(&Utc))
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .num_seconds()
                >= CLAIM_STALE_AFTER_SECS
        })
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .unwrap_or(true)
}

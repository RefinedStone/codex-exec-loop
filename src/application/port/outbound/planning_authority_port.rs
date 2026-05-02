use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::application::port::outbound::github_automation_port::GithubAutomationCapabilities;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeDistributorQueueItem,
    ParallelModeQueueItemState, ParallelModeSlotLeaseSnapshot,
};
use crate::domain::planning::{
    PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection,
    PlanningAuthorityShadowStoreSyncState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * 학습 주석: official refresh claim은 여러 worker가 같은 planning authority를 동시에 갱신하지 않도록
 * 순서를 잡는 작은 분산 락입니다. refresh order가 낮은 작업부터 authority를 공식 상태로 동기화하고,
 * 늦게 온 작업은 DB adapter가 이 상태 enum으로 "기다릴지/이미 끝났는지/내 차례인지"를 알려 줍니다.
 */
pub enum PlanningAuthorityOfficialRefreshClaimStatus {
    // 학습 주석: 현재 owner_token이 refresh claim을 얻었고, official authority 갱신을 진행해도 됩니다.
    Acquired,
    // 학습 주석: 앞선 refresh order나 다른 owner가 아직 처리 중이므로 호출자는 재시도/대기해야 합니다.
    Waiting,
    // 학습 주석: 요청한 refresh order의 효과가 이미 반영되어 추가 갱신을 할 필요가 없습니다.
    AlreadyCompleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/*
 * 학습 주석: distributor queue record는 parallel mode에서 한 agent 결과물을 통합 큐에 올릴 때의 영속 모델입니다.
 * SQLite authority adapter는 이 구조체를 JSON payload로 보관하고, distributor/pool 서비스는 같은 구조체를 읽어
 * PR 생성, 충돌 복구, integration 상태 표시를 이어 갑니다. 그래서 UI 표시 필드와 복구용 원본 메타데이터가
 * 함께 들어 있으며, 오래된 저장 데이터를 깨지 않기 위해 새 필드는 주로 `serde(default)`로 확장됩니다.
 */
pub struct PlanningAuthorityDistributorQueueRecord {
    // 학습 주석: 큐 항목의 불변 식별자입니다. claim/release와 upsert가 이 값을 기준으로 같은 작업을 찾습니다.
    pub queue_item_id: String,
    // 학습 주석: DB가 부여하는 안정 정렬 키입니다. 이전 payload에는 없을 수 있어 기본값을 허용합니다.
    #[serde(default)]
    pub queue_order_key: u64,
    // 학습 주석: 이 큐 항목을 만든 parallel mode session입니다. session detail projection과 연결됩니다.
    pub session_key: String,
    // 학습 주석: 원 대화 turn과 연결되는 선택 필드입니다. session 단위 작업만 있는 오래된 기록은 비어 있을 수 있습니다.
    #[serde(default)]
    pub root_turn_id: Option<String>,
    // 학습 주석: 작업을 수행한 slot입니다. slot lease projection과 조인해 어떤 lane에서 나온 결과인지 보여 줍니다.
    #[serde(default)]
    pub slot_id: String,
    // 학습 주석: parallel agent 식별자입니다. 화면의 큐 행과 delivery 로그에서 작업 주체를 드러냅니다.
    pub agent_id: String,
    // 학습 주석: planning task authority 안의 task id입니다. 큐 항목이 어느 계획 항목을 해결하려 했는지 연결합니다.
    pub task_id: String,
    // 학습 주석: 큐/PR 표시용 task 제목입니다. authority 문서를 다시 열지 않아도 목록을 렌더링할 수 있게 합니다.
    pub task_title: String,
    // 학습 주석: agent가 시작한 기준 브랜치입니다. 없는 옛 기록은 branch_name을 fallback으로 사용합니다.
    #[serde(default)]
    pub source_branch: String,
    // 학습 주석: agent 작업 시작 시점의 기준 commit입니다. delivery가 diff 기준을 재구성할 때 필요합니다.
    #[serde(default)]
    pub source_commit_sha: String,
    // 학습 주석: agent 결과물이 들어 있는 작업 브랜치입니다. 오래된 record에서는 source_branch 대체값이기도 합니다.
    pub branch_name: String,
    // 학습 주석: agent가 사용한 worktree 경로입니다. cleanup, 충돌 조사, 수동 복구의 실마리가 됩니다.
    pub worktree_path: String,
    // 학습 주석: 통합 대상으로 올릴 현재 결과 commit입니다. UI에서는 짧은 SHA로 표시합니다.
    pub commit_sha: String,
    // 학습 주석: 재작성/retry 전 원래 결과 commit입니다. delivery가 복구 이력을 설명할 때 사용합니다.
    #[serde(default)]
    pub original_commit_sha: Option<String>,
    // 학습 주석: 이 항목이 planning authority refresh와 어떤 관계에 있는지 나타내는 문자열 상태입니다.
    #[serde(default)]
    pub planning_refresh_state: String,
    // 학습 주석: branch 결과를 prerelease 쪽으로 통합하는 단계의 상태입니다.
    #[serde(default)]
    pub integration_state: String,
    // 학습 주석: rebase/merge 충돌이 난 파일 목록입니다. 빈 배열 기본값으로 정상 항목과 충돌 항목을 같은 타입에 담습니다.
    #[serde(default)]
    pub conflict_files: Vec<String>,
    // 학습 주석: 자동 복구나 수동 조치가 남긴 설명입니다. 큐 소비자가 실패 원인을 다시 계산하지 않도록 저장합니다.
    #[serde(default)]
    pub recovery_note: Option<String>,
    // 학습 주석: agent 결과 검증 요약입니다. delivery와 TUI가 결과 신뢰도를 짧게 보여 줄 때 씁니다.
    pub validation_summary: String,
    // 학습 주석: authority refresh 시도 결과입니다. queue state와 별도로 planning 문서 동기화 결과를 보존합니다.
    pub authority_refresh_outcome: String,
    // 학습 주석: GitHub PR/리뷰 자동화 가능 여부입니다. delivery 단계가 connector 능력을 재판단하지 않게 합니다.
    #[serde(default)]
    pub github_capabilities: Option<GithubAutomationCapabilities>,
    // 학습 주석: 이미 열린 PR 번호입니다. 재실행 시 중복 PR을 만들지 않고 기존 PR을 이어 가게 합니다.
    #[serde(default)]
    pub pull_request_number: Option<u64>,
    // 학습 주석: 사람이 클릭할 수 있는 PR URL입니다. TUI/로그 표시가 번호만으로 부족할 때 사용합니다.
    #[serde(default)]
    pub pull_request_url: Option<String>,
    // 학습 주석: distributor queue의 현재 처리 상태입니다. pool snapshot과 delivery loop가 이 값으로 작업을 고릅니다.
    pub queue_state: ParallelModeQueueItemState,
    // 학습 주석: 상태를 사람이 이해할 수 있게 보조하는 한 줄 설명입니다.
    pub integration_note: String,
    // 학습 주석: 큐에 들어온 시각입니다. 정렬/감사 로그에서 queue_order_key와 함께 사용됩니다.
    pub enqueued_at: String,
    // 학습 주석: 마지막 상태 변경 시각입니다. stale queue 감지와 운영자 진단에 쓰입니다.
    pub updated_at: String,
}

impl PlanningAuthorityDistributorQueueRecord {
    /*
     * 학습 주석: 영속 queue record를 화면/분배 로직용 domain item으로 축약합니다.
     * 모든 복구 메타데이터를 노출하지 않고 agent, 제목, 상태, 기준 브랜치, 짧은 SHA, note만 남겨
     * `parallel_mode::distributor::snapshot`이 목록을 빠르게 렌더링하게 합니다.
     */
    pub fn display_item(&self) -> ParallelModeDistributorQueueItem {
        ParallelModeDistributorQueueItem::new(
            self.agent_id.clone(),
            self.task_title.clone(),
            self.queue_state,
            self.effective_source_branch(),
            self.commit_sha.chars().take(7).collect::<String>(),
            self.integration_note.clone(),
        )
    }

    // 학습 주석: source_branch가 비어 있는 legacy record는 결과 branch를 기준 브랜치로 간주해 기존 동작을 보존합니다.
    pub fn effective_source_branch(&self) -> String {
        if self.source_branch.trim().is_empty() {
            self.branch_name.clone()
        } else {
            self.source_branch.clone()
        }
    }

    // 학습 주석: source commit이 없는 legacy record는 결과 commit을 기준 commit으로 삼아 delivery 경로를 유지합니다.
    pub fn effective_source_commit_sha(&self) -> String {
        if self.source_commit_sha.trim().is_empty() {
            self.commit_sha.clone()
        } else {
            self.source_commit_sha.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
/*
 * 학습 주석: runtime projection snapshot은 parallel mode 운영 상태를 한 번에 읽는 read model입니다.
 * authority adapter는 slot lease, session detail, distributor queue를 각각 저장하지만,
 * pool reconcile과 admin file sync는 이 네 묶음을 같이 봐야 "현재 실행 중인 슬롯", "깨진 lease",
 * "agent session 상태", "통합 대기 큐"를 일관된 한 화면으로 판단할 수 있습니다.
 */
pub struct PlanningAuthorityRuntimeProjectionSnapshot {
    // 학습 주석: slot_id별 lease 상태입니다. active worktree/lane 점유를 추적합니다.
    pub slot_leases: BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
    // 학습 주석: 저장소에는 남아 있지만 더 이상 유효하지 않은 slot id 목록입니다. reconcile이 cleanup 후보로 씁니다.
    pub invalid_slot_leases: BTreeSet<String>,
    // 학습 주석: agent session별 상세 상태입니다. pool UI와 recovery 판단이 lease만으로 부족한 정보를 얻습니다.
    pub session_details: Vec<ParallelModeAgentSessionDetailSnapshot>,
    // 학습 주석: distributor가 아직 처리 중이거나 처리할 queue record들입니다.
    pub distributor_queue_records: Vec<PlanningAuthorityDistributorQueueRecord>,
}

/*
 * 학습 주석: `PlanningAuthorityPort`는 planning authority 저장소의 운영 제어면입니다.
 * task/direction 문서 자체는 `PlanningTaskRepositoryPort`가 다루고, 이 포트는 그 문서들이 놓인
 * authority store의 위치, shadow store 진단, parallel mode runtime projection, 분산 claim을 관리합니다.
 * application service는 이 trait만 보고 공식 SQLite authority인지 테스트용 Noop인지 구분하지 않습니다.
 */
pub trait PlanningAuthorityPort: Send + Sync {
    /*
     * 학습 주석: workspace 문자열에서 authority store의 실제 위치를 해석합니다.
     * repo-scoped workspace에서는 canonical repo root와 runtime dir이 중요하고, admin/readiness 흐름은
     * 이 위치 정보를 기준으로 shadow store 경로와 SQLite store 경로를 사용자에게 보고합니다.
     */
    fn resolve_authority_location(&self, workspace_dir: &str) -> Result<PlanningAuthorityLocation>;

    // 학습 주석: filesystem mirror와 authority store가 동기화되어 있는지 검사해 admin file sync의 판단 근거를 만듭니다.
    fn inspect_shadow_store(
        &self,
        // 학습 주석: 검사 대상 workspace입니다. adapter는 이 값에서 repo root와 authority DB 위치를 다시 계산합니다.
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection>;

    /*
     * 학습 주석: official completion/refresh 작업에 순번을 부여합니다.
     * 여러 worker가 동시에 종료되어도 낮은 refresh order부터 authority를 갱신해야 task/direction 문서와
     * parallel runtime projection이 예측 가능한 순서로 공식화됩니다.
     */
    fn reserve_next_official_refresh_order(&self, workspace_dir: &str) -> Result<u64>;

    /*
     * 학습 주석: 특정 refresh order가 지금 실행 가능한지 확인하고, 가능하면 owner_token으로 claim을 잡습니다.
     * 반환값은 worker orchestration이 "진행", "대기", "이미 완료"를 나눠 처리하는 분기점입니다.
     */
    fn acquire_official_refresh_claim(
        &self,
        // 학습 주석: claim을 저장할 authority namespace입니다.
        workspace_dir: &str,
        // 학습 주석: `reserve_next_official_refresh_order`가 발급한 순번입니다.
        refresh_order: u64,
        // 학습 주석: 같은 worker가 재진입했는지, 다른 worker가 선점했는지 구분하는 소유자 토큰입니다.
        owner_token: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshClaimStatus>;

    /*
     * 학습 주석: official refresh claim을 해제하고 다음 refresh order가 실행될 수 있게 진행 포인터를 옮깁니다.
     * release는 acquire와 같은 owner_token을 받으므로, 다른 worker가 실수로 claim을 닫는 상황을 adapter가 막을 수 있습니다.
     */
    fn release_official_refresh_claim(
        &self,
        // 학습 주석: claim이 저장된 authority namespace입니다.
        workspace_dir: &str,
        // 학습 주석: 완료 처리할 refresh 순번입니다.
        refresh_order: u64,
        // 학습 주석: claim을 잡은 worker의 토큰입니다.
        owner_token: &str,
    ) -> Result<()>;

    /*
     * 학습 주석: distributor queue 항목 하나를 처리할 권리를 잡습니다.
     * queue head를 여러 dispatcher가 동시에 PR 생성/merge 처리하지 않게 하는 잠금이며,
     * bool 반환은 "내가 처리해도 되는가"만 알려 주고 대기 사유는 상위 정책이 결정합니다.
     */
    fn try_acquire_distributor_queue_claim(
        &self,
        // 학습 주석: distributor queue가 속한 authority namespace입니다.
        workspace_dir: &str,
        // 학습 주석: claim할 queue record의 안정 식별자입니다.
        queue_item_id: &str,
        // 학습 주석: dispatcher 실행 단위를 구분하는 소유자 토큰입니다.
        owner_token: &str,
    ) -> Result<bool>;

    // 학습 주석: distributor queue claim을 해제해 retry나 다음 dispatcher 처리가 가능하게 합니다.
    fn release_distributor_queue_claim(
        &self,
        // 학습 주석: claim이 저장된 authority namespace입니다.
        workspace_dir: &str,
        // 학습 주석: 해제할 queue record 식별자입니다.
        queue_item_id: &str,
        // 학습 주석: claim 소유자 토큰입니다. adapter는 소유자가 맞는 경우에만 제거할 수 있습니다.
        owner_token: &str,
    ) -> Result<()>;

    /*
     * 학습 주석: parallel mode runtime 상태를 한 번에 읽습니다.
     * pool board, supervisor snapshot, admin busy-state 판단은 slot lease/session detail/queue record를 따로 읽으면
     * 서로 다른 시점이 섞일 수 있으므로 이 projection snapshot을 통해 같은 authority 읽기 모델을 공유합니다.
     */
    fn load_runtime_projections(
        &self,
        // 학습 주석: projection을 읽을 authority namespace입니다.
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityRuntimeProjectionSnapshot>;

    // 학습 주석: slot lease projection을 삽입하거나 갱신해 pool reconcile과 supervisor roster가 같은 lease를 보게 합니다.
    fn upsert_runtime_slot_lease(
        &self,
        // 학습 주석: lease를 저장할 authority namespace입니다.
        workspace_dir: &str,
        // 학습 주석: slot id, branch, worktree, 상태가 들어 있는 runtime lease snapshot입니다.
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()>;

    // 학습 주석: slot lease projection을 제거해 cleanup 이후 해당 slot이 더 이상 점유 상태로 보이지 않게 합니다.
    fn remove_runtime_slot_lease(&self, workspace_dir: &str, slot_id: &str) -> Result<()>;

    // 학습 주석: agent session detail projection을 저장해 lease보다 긴 수명의 실행/결과 정보를 유지합니다.
    fn upsert_runtime_session_detail(
        &self,
        // 학습 주석: session detail을 저장할 authority namespace입니다.
        workspace_dir: &str,
        // 학습 주석: agent session key, 상태, timestamps, outcome을 담은 projection입니다.
        detail: &ParallelModeAgentSessionDetailSnapshot,
    ) -> Result<()>;

    // 학습 주석: distributor queue record를 저장해 agent 결과물이 통합될 때까지 durable queue에 남깁니다.
    fn upsert_runtime_distributor_queue_record(
        &self,
        // 학습 주석: queue record를 저장할 authority namespace입니다.
        workspace_dir: &str,
        // 학습 주석: branch/commit/PR/상태/복구 정보를 담은 distributor queue 영속 record입니다.
        record: &PlanningAuthorityDistributorQueueRecord,
    ) -> Result<()>;
}

#[derive(Default)]
/*
 * 학습 주석: `NoopPlanningAuthorityPort`는 authority DB가 연결되지 않은 경량 조립 경로의 fallback입니다.
 * TUI 테스트, 단일 workspace 실행, `PlanningServices::from_workspace_port`처럼 planning workspace만 주입하는 경로도
 * 같은 application service를 사용할 수 있어야 하므로, 이 구현은 runtime projection을 저장하지 않고
 * claim도 항상 성공한 것처럼 돌려줍니다. 즉 실제 동기화 보장이 아니라 "비영속 단일 실행용 무해한 대체물"입니다.
 */
pub struct NoopPlanningAuthorityPort {
    // 학습 주석: official refresh 순번만은 단조 증가시켜 orchestration 코드가 정상 adapter와 같은 흐름을 타게 합니다.
    next_refresh_order: AtomicU64,
}

impl PlanningAuthorityPort for NoopPlanningAuthorityPort {
    // 학습 주석: 별도 authority store가 없으므로 workspace_dir 자체를 workspace root와 canonical repo root로 간주합니다.
    fn resolve_authority_location(&self, workspace_dir: &str) -> Result<PlanningAuthorityLocation> {
        Ok(PlanningAuthorityLocation {
            // 학습 주석: 호출자가 넘긴 경로를 그대로 운영 기준 루트로 보고합니다.
            workspace_root: workspace_dir.to_string(),
            // 학습 주석: repo-scoped 정규화가 없는 fallback이므로 canonical root도 같은 값입니다.
            canonical_repo_root: workspace_dir.to_string(),
            // 학습 주석: runtime projection을 저장하지 않으므로 runtime_dir은 비워 둡니다.
            runtime_dir: String::new(),
            // 학습 주석: SQLite authority store가 없다는 것을 빈 경로로 표현합니다.
            authority_store_path: String::new(),
        })
    }

    // 학습 주석: mirror 자체가 없으므로 shadow store는 항상 동기화 상태이며 문서/문제 수는 0으로 보고합니다.
    fn inspect_shadow_store(
        &self,
        // 학습 주석: location fallback을 만들기 위해 그대로 전달되는 workspace 기준입니다.
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection> {
        Ok(PlanningAuthorityShadowStoreInspection {
            // 학습 주석: Noop location도 inspection 결과에 넣어 admin/readiness 출력 형식을 유지합니다.
            location: self.resolve_authority_location(workspace_dir)?,
            // 학습 주석: 비교할 shadow store가 없으므로 불일치가 없다는 의미로 InSync를 사용합니다.
            sync_state: PlanningAuthorityShadowStoreSyncState::InSync,
            // 학습 주석: mirror 문서를 만들지 않는 adapter라 count는 항상 0입니다.
            mirrored_document_count: 0,
            // 학습 주석: parity 검사도 수행하지 않으므로 issue 수는 0입니다.
            parity_issue_count: 0,
            // 학습 주석: 보고할 mismatch 예시가 없습니다.
            parity_issue_examples: Vec::new(),
        })
    }

    // 학습 주석: process-local counter로 refresh order를 발급해 worker orchestration의 순번 의존 로직을 살립니다.
    fn reserve_next_official_refresh_order(&self, _workspace_dir: &str) -> Result<u64> {
        // 학습 주석: relaxed ordering이면 충분합니다. Noop은 persistence/교차 process 동기화를 제공하지 않기 때문입니다.
        Ok(self.next_refresh_order.fetch_add(1, Ordering::Relaxed) + 1)
    }

    // 학습 주석: 단일 실행 fallback에서는 경합을 추적하지 않고 모든 official refresh claim을 즉시 허용합니다.
    fn acquire_official_refresh_claim(
        &self,
        // 학습 주석: Noop은 namespace별 claim table이 없으므로 사용하지 않습니다.
        _workspace_dir: &str,
        // 학습 주석: 순번 검사는 실제 adapter 책임이며 Noop에서는 항상 실행 가능하다고 봅니다.
        _refresh_order: u64,
        // 학습 주석: owner_token을 저장하지 않으므로 재진입/경합 구분도 하지 않습니다.
        _owner_token: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshClaimStatus> {
        Ok(PlanningAuthorityOfficialRefreshClaimStatus::Acquired)
    }

    // 학습 주석: 저장된 claim이 없으므로 release는 no-op입니다.
    fn release_official_refresh_claim(
        &self,
        // 학습 주석: Noop release에서는 namespace를 확인하지 않습니다.
        _workspace_dir: &str,
        // 학습 주석: 진행 포인터를 저장하지 않으므로 refresh order도 사용하지 않습니다.
        _refresh_order: u64,
        // 학습 주석: owner 검증을 하지 않는 것도 Noop의 비영속 fallback 성격 때문입니다.
        _owner_token: &str,
    ) -> Result<()> {
        Ok(())
    }

    // 학습 주석: distributor queue도 실제 저장소가 없으므로 모든 claim을 허용해 호출 흐름을 막지 않습니다.
    fn try_acquire_distributor_queue_claim(
        &self,
        // 학습 주석: queue namespace를 저장하지 않습니다.
        _workspace_dir: &str,
        // 학습 주석: queue item별 lock table이 없으므로 식별자는 무시됩니다.
        _queue_item_id: &str,
        // 학습 주석: owner token도 저장하지 않습니다.
        _owner_token: &str,
    ) -> Result<bool> {
        Ok(true)
    }

    // 학습 주석: 저장된 distributor claim이 없기 때문에 해제도 no-op입니다.
    fn release_distributor_queue_claim(
        &self,
        // 학습 주석: namespace를 쓰지 않습니다.
        _workspace_dir: &str,
        // 학습 주석: item id를 쓰지 않습니다.
        _queue_item_id: &str,
        // 학습 주석: owner token을 쓰지 않습니다.
        _owner_token: &str,
    ) -> Result<()> {
        Ok(())
    }

    // 학습 주석: runtime projection을 저장하지 않으므로 항상 빈 snapshot을 반환합니다.
    fn load_runtime_projections(
        &self,
        // 학습 주석: workspace별 projection 분리도 제공하지 않습니다.
        _workspace_dir: &str,
    ) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        Ok(PlanningAuthorityRuntimeProjectionSnapshot::default())
    }

    // 학습 주석: slot lease upsert는 받아들이지만 저장하지 않습니다. 경량 경로에서 pool 상태가 누적되지 않게 합니다.
    fn upsert_runtime_slot_lease(
        &self,
        // 학습 주석: 저장소가 없으므로 workspace를 쓰지 않습니다.
        _workspace_dir: &str,
        // 학습 주석: lease 내용도 검증하지 않고 버립니다.
        _lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()> {
        Ok(())
    }

    // 학습 주석: 저장된 slot lease가 없으므로 제거 요청도 성공한 no-op으로 처리합니다.
    fn remove_runtime_slot_lease(&self, _workspace_dir: &str, _slot_id: &str) -> Result<()> {
        Ok(())
    }

    // 학습 주석: session detail upsert도 저장하지 않습니다. 실제 session history는 SQLite authority adapter의 책임입니다.
    fn upsert_runtime_session_detail(
        &self,
        // 학습 주석: workspace namespace를 사용하지 않습니다.
        _workspace_dir: &str,
        // 학습 주석: detail payload도 저장하지 않습니다.
        _detail: &ParallelModeAgentSessionDetailSnapshot,
    ) -> Result<()> {
        Ok(())
    }

    // 학습 주석: distributor queue record도 저장하지 않아 Noop projection은 항상 빈 queue를 유지합니다.
    fn upsert_runtime_distributor_queue_record(
        &self,
        // 학습 주석: workspace namespace를 사용하지 않습니다.
        _workspace_dir: &str,
        // 학습 주석: record payload도 저장하지 않습니다.
        _record: &PlanningAuthorityDistributorQueueRecord,
    ) -> Result<()> {
        Ok(())
    }
}

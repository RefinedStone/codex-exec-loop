use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::application::port::outbound::github_automation_port::GithubAutomationCapabilities;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeDistributorQueueItem,
    ParallelModePoolResetReport, ParallelModeQueueItemState, ParallelModeSlotLeaseSnapshot,
    ParallelModeTaskDispatchBlockSnapshot,
};
use crate::domain::planning::{
    PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection,
    PlanningAuthorityShadowStoreSyncState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * official refresh claim은 여러 worker가 같은 planning authority를 동시에 갱신하지 않도록
 * 순서를 잡는 작은 분산 락입니다. refresh order가 낮은 작업부터 authority를 공식 상태로 동기화하고,
 * 늦게 온 작업은 DB adapter가 이 상태 enum으로 "기다릴지/이미 끝났는지/내 차례인지"를 알려 줍니다.
 */
pub enum PlanningAuthorityOfficialRefreshClaimStatus {
    // The caller owns the refresh slot and may update official authority state.
    Acquired,
    // An earlier order or another owner still blocks this refresh.
    Waiting,
    // The requested order is already reflected in the authority store.
    AlreadyCompleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * official refresh worker가 시작 표시만 남긴 뒤 사라진 경우 recovery path가 실행 포인터를
 * 한 칸 전진시킬 수 있어야 합니다. 상태 enum은 회수 성공, 회수할 예약 없음,
 * 아직 살아 있는 claim 존재를 구분합니다.
 */
pub enum PlanningAuthorityOfficialRefreshRecoveryStatus {
    Recovered { refresh_order: u64 },
    NoPendingOrder,
    WaitingForActiveClaim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/*
 * distributor queue record는 parallel mode에서 한 agent 결과물을 통합 큐에 올릴 때의 영속 모델입니다.
 * SQLite authority adapter는 이 구조체를 JSON payload로 보관하고, distributor/pool 서비스는 같은 구조체를 읽어
 * PR 생성, 충돌 복구, integration 상태 표시를 이어 갑니다. 그래서 UI 표시 필드와 복구용 원본 메타데이터가
 * 함께 들어 있으며, 오래된 저장 데이터를 깨지 않기 위해 새 필드는 주로 `serde(default)`로 확장됩니다.
 */
pub struct PlanningAuthorityDistributorQueueRecord {
    // Stable queue identity used by claim/release and idempotent upserts.
    pub queue_item_id: String,
    // Store-assigned ordering key; legacy JSON payloads may not contain it.
    #[serde(default)]
    pub queue_order_key: u64,
    // Parallel-mode session that produced the queue item.
    pub session_key: String,
    // Optional root conversation turn; older session-only records leave it empty.
    #[serde(default)]
    pub root_turn_id: Option<String>,
    // Slot that ran the work, used to join queue rows with lease projections.
    #[serde(default)]
    pub slot_id: String,
    // Agent identity shown in queue rows and delivery diagnostics.
    pub agent_id: String,
    // Planning task id that the queued branch attempted to resolve.
    pub task_id: String,
    // Cached title for queue and PR copy without reopening the task authority.
    pub task_title: String,
    // Branch the agent started from; legacy records fall back to branch_name.
    #[serde(default)]
    pub source_branch: String,
    // Start commit for reconstructing delivery diffs and recovery provenance.
    #[serde(default)]
    pub source_commit_sha: String,
    // Working branch containing the agent result, also a legacy source fallback.
    pub branch_name: String,
    // Worktree path for cleanup, conflict inspection, and manual recovery.
    pub worktree_path: String,
    // Current result commit targeted for integration.
    pub commit_sha: String,
    // Original result commit before rewrite/retry, retained for recovery history.
    #[serde(default)]
    pub original_commit_sha: Option<String>,
    // String state describing how this item relates to authority refresh.
    #[serde(default)]
    pub planning_refresh_state: String,
    // Integration phase for carrying the branch result into prerelease.
    #[serde(default)]
    pub integration_state: String,
    // Rebase/merge conflict files; empty by default for normal records.
    #[serde(default)]
    pub conflict_files: Vec<String>,
    // Recovery note persisted so queue consumers do not recalculate failure cause.
    #[serde(default)]
    pub recovery_note: Option<String>,
    // Validation summary surfaced by delivery and TUI projections.
    pub validation_summary: String,
    // Authority-refresh outcome preserved separately from queue state.
    pub authority_refresh_outcome: String,
    // GitHub automation capabilities captured at delivery time.
    #[serde(default)]
    pub github_capabilities: Option<GithubAutomationCapabilities>,
    // Existing PR number, preventing duplicate PR creation on retry.
    #[serde(default)]
    pub pull_request_number: Option<u64>,
    // Clickable PR URL for TUI/log surfaces that need more than a number.
    #[serde(default)]
    pub pull_request_url: Option<String>,
    // Current distributor queue state used by snapshots and delivery loops.
    pub queue_state: ParallelModeQueueItemState,
    // Human-facing one-line state explanation.
    pub integration_note: String,
    // Enqueue time for ordering and audit displays.
    pub enqueued_at: String,
    // Last state change time for stale-queue detection and operator diagnostics.
    pub updated_at: String,
}

impl PlanningAuthorityDistributorQueueRecord {
    /*
     * 영속 queue record를 화면/분배 로직용 domain item으로 축약합니다.
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

    // Legacy records without source_branch treat the result branch as the baseline.
    pub fn effective_source_branch(&self) -> String {
        if self.source_branch.trim().is_empty() {
            self.branch_name.clone()
        } else {
            self.source_branch.clone()
        }
    }

    // Legacy records without source_commit_sha use the result commit as baseline.
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
 * runtime projection snapshot은 parallel mode 운영 상태를 한 번에 읽는 read model입니다.
 * authority adapter는 slot lease, session detail, distributor queue를 각각 저장하지만,
 * pool reconcile과 admin file sync는 이 네 묶음을 같이 봐야 "현재 실행 중인 슬롯", "깨진 lease",
 * "agent session 상태", "통합 대기 큐"를 일관된 한 화면으로 판단할 수 있습니다.
 */
pub struct PlanningAuthorityRuntimeProjectionSnapshot {
    // Lease state by slot id, representing active worktree/lane ownership.
    pub slot_leases: BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
    // Persisted but invalid slot ids that reconciliation can clean up.
    pub invalid_slot_leases: BTreeSet<String>,
    // Session detail projections that outlive individual lease snapshots.
    pub session_details: Vec<ParallelModeAgentSessionDetailSnapshot>,
    // Task-level dispatch blocks survive disposable pool reset.
    pub task_dispatch_blocks: Vec<ParallelModeTaskDispatchBlockSnapshot>,
    // Queue records still pending, blocked, or otherwise visible to distributor.
    pub distributor_queue_records: Vec<PlanningAuthorityDistributorQueueRecord>,
}

/*
 * `PlanningAuthorityPort`는 planning authority 저장소의 운영 제어면입니다.
 * task/direction 문서 자체는 `PlanningTaskRepositoryPort`가 다루고, 이 포트는 그 문서들이 놓인
 * authority store의 위치, shadow store 진단, parallel mode runtime projection, 분산 claim을 관리합니다.
 * application service는 이 trait만 보고 공식 SQLite authority인지 테스트용 Noop인지 구분하지 않습니다.
 */
pub trait PlanningAuthorityPort: Send + Sync {
    /*
     * workspace 문자열에서 authority store의 실제 위치를 해석합니다.
     * repo-scoped workspace에서는 canonical repo root와 runtime dir이 중요하고, admin/readiness 흐름은
     * 이 위치 정보를 기준으로 shadow store 경로와 SQLite store 경로를 사용자에게 보고합니다.
     */
    fn resolve_authority_location(&self, workspace_dir: &str) -> Result<PlanningAuthorityLocation>;

    // filesystem mirror와 authority store가 동기화되어 있는지 검사해 admin file sync의 판단 근거를 만듭니다.
    fn inspect_shadow_store(
        &self,
        // Workspace whose repo root and authority DB location should be inspected.
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection>;

    /*
     * official completion/refresh 작업에 순번을 부여합니다.
     * 여러 worker가 동시에 종료되어도 낮은 refresh order부터 authority를 갱신해야 task/direction 문서와
     * parallel runtime projection이 예측 가능한 순서로 공식화됩니다.
     */
    fn reserve_next_official_refresh_order(&self, workspace_dir: &str) -> Result<u64>;

    /*
     * 특정 refresh order가 지금 실행 가능한지 확인하고, 가능하면 owner_token으로 claim을 잡습니다.
     * 반환값은 worker orchestration이 "진행", "대기", "이미 완료"를 나눠 처리하는 분기점입니다.
     */
    fn acquire_official_refresh_claim(
        &self,
        // Authority namespace that owns the refresh claim table.
        workspace_dir: &str,
        // Order previously issued by reserve_next_official_refresh_order.
        refresh_order: u64,
        // Owner token distinguishing re-entry from a competing worker.
        owner_token: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshClaimStatus>;

    /*
     * official refresh claim을 해제하고 다음 refresh order가 실행될 수 있게 진행 포인터를 옮깁니다.
     * release는 acquire와 같은 owner_token을 받으므로, 다른 worker가 실수로 claim을 닫는 상황을 adapter가 막을 수 있습니다.
     */
    fn release_official_refresh_claim(
        &self,
        // Authority namespace containing the claim.
        workspace_dir: &str,
        // Refresh order being marked complete.
        refresh_order: u64,
        // Token that originally acquired the claim.
        owner_token: &str,
    ) -> Result<()>;

    /*
     * 다음 실행 포인터가 이미 예약된 order를 가리키지만 살아 있는 claim이 없을 때,
     * 그 order를 abandoned로 표시하고 다음 order가 실행될 수 있게 합니다.
     */
    fn abandon_next_official_refresh_order(
        &self,
        // Authority namespace containing the official refresh metadata.
        workspace_dir: &str,
        // Operator-facing reason recorded in runtime events.
        reason: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshRecoveryStatus>;

    /*
     * distributor queue 항목 하나를 처리할 권리를 잡습니다.
     * queue head를 여러 dispatcher가 동시에 PR 생성/merge 처리하지 않게 하는 잠금이며,
     * bool 반환은 "내가 처리해도 되는가"만 알려 주고 대기 사유는 상위 정책이 결정합니다.
     */
    fn try_acquire_distributor_queue_claim(
        &self,
        // Authority namespace containing the distributor queue.
        workspace_dir: &str,
        // Stable queue record id to claim.
        queue_item_id: &str,
        // Owner token for this dispatcher attempt.
        owner_token: &str,
    ) -> Result<bool>;

    // Release a queue claim so retry or another dispatcher can proceed.
    fn release_distributor_queue_claim(
        &self,
        // Authority namespace containing the claim.
        workspace_dir: &str,
        // Queue record id to release.
        queue_item_id: &str,
        // Owner token; adapters should only release matching owners.
        owner_token: &str,
    ) -> Result<()>;

    /*
     * parallel mode runtime 상태를 한 번에 읽습니다.
     * pool board, supervisor snapshot, admin busy-state 판단은 slot lease/session detail/queue record를 따로 읽으면
     * 서로 다른 시점이 섞일 수 있으므로 이 projection snapshot을 통해 같은 authority 읽기 모델을 공유합니다.
     */
    fn load_runtime_projections(
        &self,
        // Authority namespace to read.
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityRuntimeProjectionSnapshot>;

    // Clear current parallel runtime rows when the disposable pool is reset on enable.
    fn clear_parallel_runtime_projections(&self, workspace_dir: &str, reason: &str) -> Result<()>;

    // Apply a pool reset report after git reset has succeeded for selected slots.
    fn apply_parallel_pool_reset_report(
        &self,
        workspace_dir: &str,
        report: &ParallelModePoolResetReport,
    ) -> Result<()>;

    // Upsert a slot lease projection shared by pool reconciliation and supervisor roster.
    fn upsert_runtime_slot_lease(
        &self,
        // Authority namespace to write.
        workspace_dir: &str,
        // Runtime lease snapshot with slot id, branch, worktree, and state.
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()>;

    // Remove a lease projection after cleanup returns a slot to the idle pool.
    fn remove_runtime_slot_lease(&self, workspace_dir: &str, slot_id: &str) -> Result<()>;

    // Store session detail projection that can outlive an individual slot lease.
    fn upsert_runtime_session_detail(
        &self,
        // Authority namespace to write.
        workspace_dir: &str,
        // Session-keyed projection containing state, timestamps, and outcome.
        detail: &ParallelModeAgentSessionDetailSnapshot,
    ) -> Result<()>;

    // Store a task-level dispatch block that should survive disposable pool reset.
    fn upsert_runtime_task_dispatch_block(
        &self,
        workspace_dir: &str,
        block: &ParallelModeTaskDispatchBlockSnapshot,
    ) -> Result<()>;

    // Store a durable distributor queue record until the agent result is integrated.
    fn upsert_runtime_distributor_queue_record(
        &self,
        // Authority namespace to write.
        workspace_dir: &str,
        // Queue record containing branch, commit, PR, state, and recovery metadata.
        record: &PlanningAuthorityDistributorQueueRecord,
    ) -> Result<()>;
}

#[derive(Default)]
/*
 * `NoopPlanningAuthorityPort`는 authority DB가 연결되지 않은 경량 조립 경로의 fallback입니다.
 * TUI 테스트, 단일 workspace 실행, `PlanningServices::from_workspace_port`처럼 planning workspace만 주입하는 경로도
 * 같은 application service를 사용할 수 있어야 하므로, 이 구현은 runtime projection을 저장하지 않고
 * claim도 항상 성공한 것처럼 돌려줍니다. 즉 실제 동기화 보장이 아니라 "비영속 단일 실행용 무해한 대체물"입니다.
 */
pub struct NoopPlanningAuthorityPort {
    // Monotonic refresh counter keeps orchestration on the same path as real adapters.
    next_refresh_order: AtomicU64,
}

impl PlanningAuthorityPort for NoopPlanningAuthorityPort {
    // Without a store, the supplied workspace is both workspace root and canonical root.
    fn resolve_authority_location(&self, workspace_dir: &str) -> Result<PlanningAuthorityLocation> {
        Ok(PlanningAuthorityLocation {
            // Caller-supplied path as the operational root.
            workspace_root: workspace_dir.to_string(),
            // No repo-scoped normalization exists in the fallback.
            canonical_repo_root: workspace_dir.to_string(),
            // Runtime projections are not persisted.
            runtime_dir: String::new(),
            // Empty path represents absence of a SQLite authority store.
            authority_store_path: String::new(),
        })
    }

    // No mirror exists, so shadow-store inspection is always an empty in-sync report.
    fn inspect_shadow_store(
        &self,
        // Workspace basis used only to build the fallback location.
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection> {
        Ok(PlanningAuthorityShadowStoreInspection {
            // Include a location so admin/readiness output keeps the same shape.
            location: self.resolve_authority_location(workspace_dir)?,
            // With no mirror to compare, there are no parity mismatches.
            sync_state: PlanningAuthorityShadowStoreSyncState::InSync,
            // No mirrored documents are produced by this adapter.
            mirrored_document_count: 0,
            // No parity check runs in the fallback.
            parity_issue_count: 0,
            // No mismatch examples exist.
            parity_issue_examples: Vec::new(),
        })
    }

    // Process-local ordering is enough to exercise worker orchestration paths.
    fn reserve_next_official_refresh_order(&self, _workspace_dir: &str) -> Result<u64> {
        // No persistence or cross-process synchronization is promised here.
        Ok(self.next_refresh_order.fetch_add(1, Ordering::Relaxed) + 1)
    }

    // Single-process fallback grants every official refresh claim immediately.
    fn acquire_official_refresh_claim(
        &self,
        // No namespace-specific claim table exists.
        _workspace_dir: &str,
        // Real adapters enforce order; the fallback always allows execution.
        _refresh_order: u64,
        // Owner tokens are not stored, so re-entry and contention are indistinguishable.
        _owner_token: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshClaimStatus> {
        Ok(PlanningAuthorityOfficialRefreshClaimStatus::Acquired)
    }

    // No persisted claim exists, so release is a no-op.
    fn release_official_refresh_claim(
        &self,
        // Namespace is ignored by the fallback.
        _workspace_dir: &str,
        // No progress pointer is stored.
        _refresh_order: u64,
        // Owner validation is intentionally absent from the non-persistent fallback.
        _owner_token: &str,
    ) -> Result<()> {
        Ok(())
    }

    fn abandon_next_official_refresh_order(
        &self,
        _workspace_dir: &str,
        _reason: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshRecoveryStatus> {
        Ok(PlanningAuthorityOfficialRefreshRecoveryStatus::NoPendingOrder)
    }

    // With no durable queue, every distributor claim succeeds to keep callers moving.
    fn try_acquire_distributor_queue_claim(
        &self,
        // Queue namespace is not stored.
        _workspace_dir: &str,
        // No per-item lock table exists.
        _queue_item_id: &str,
        // Owner token is ignored.
        _owner_token: &str,
    ) -> Result<bool> {
        Ok(true)
    }

    // No stored distributor claim exists, so release is a no-op.
    fn release_distributor_queue_claim(
        &self,
        // Namespace is ignored.
        _workspace_dir: &str,
        // Item id is ignored.
        _queue_item_id: &str,
        // Owner token is ignored.
        _owner_token: &str,
    ) -> Result<()> {
        Ok(())
    }

    // Runtime projections are not persisted, so the snapshot is always empty.
    fn load_runtime_projections(
        &self,
        // Workspace partitioning is not provided by the fallback.
        _workspace_dir: &str,
    ) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        Ok(PlanningAuthorityRuntimeProjectionSnapshot::default())
    }

    // No runtime store exists in the fallback, so clearing is a no-op.
    fn clear_parallel_runtime_projections(
        &self,
        _workspace_dir: &str,
        _reason: &str,
    ) -> Result<()> {
        Ok(())
    }

    fn apply_parallel_pool_reset_report(
        &self,
        _workspace_dir: &str,
        _report: &ParallelModePoolResetReport,
    ) -> Result<()> {
        Ok(())
    }

    // Accept but discard slot leases so lightweight paths do not accumulate pool state.
    fn upsert_runtime_slot_lease(
        &self,
        // No store means no workspace partition.
        _workspace_dir: &str,
        // Lease payload is ignored.
        _lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()> {
        Ok(())
    }

    // No stored slot lease exists, so removal succeeds as a no-op.
    fn remove_runtime_slot_lease(&self, _workspace_dir: &str, _slot_id: &str) -> Result<()> {
        Ok(())
    }

    // Session details are discarded; durable session history belongs to SQLite authority.
    fn upsert_runtime_session_detail(
        &self,
        // Workspace namespace is ignored.
        _workspace_dir: &str,
        // Detail payload is ignored.
        _detail: &ParallelModeAgentSessionDetailSnapshot,
    ) -> Result<()> {
        Ok(())
    }

    // Task dispatch blocks are discarded with the empty fallback projection.
    fn upsert_runtime_task_dispatch_block(
        &self,
        _workspace_dir: &str,
        _block: &ParallelModeTaskDispatchBlockSnapshot,
    ) -> Result<()> {
        Ok(())
    }

    // Queue records are discarded, keeping the fallback projection empty.
    fn upsert_runtime_distributor_queue_record(
        &self,
        // Workspace namespace is ignored.
        _workspace_dir: &str,
        // Record payload is ignored.
        _record: &PlanningAuthorityDistributorQueueRecord,
    ) -> Result<()> {
        Ok(())
    }
}

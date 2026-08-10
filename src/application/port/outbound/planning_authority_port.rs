use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
#[cfg(test)]
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubRepositoryVisibility,
};
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogPort;
#[cfg(test)]
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogRequest;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskAuthorityCommitResult;
#[cfg(test)]
use crate::domain::parallel_mode::ParallelModeRuntimeEventsSnapshot;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeDispatchCommandSnapshot,
    ParallelModeDistributorQueueItem, ParallelModePoolResetReport, ParallelModeQueueItemState,
    ParallelModeSlotLeaseSnapshot, ParallelModeTaskDispatchBlockSnapshot,
    PrValidationPollErrorClass, PrValidationRecord, PrValidationRecordKey,
};
#[cfg(test)]
use crate::domain::planning::PlanningAuthorityShadowStoreSyncState;
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection,
    PriorityQueueProjection, TaskAuthorityDocument,
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
pub struct PlanningAuthorityDistributorDeliveryTarget {
    pub push_remote: String,
    #[serde(default)]
    pub credential_redacted_push_url: Option<String>,
    pub github_repository: String,
    pub repository_visibility: GithubRepositoryVisibility,
    pub integration_branch: String,
}

impl PlanningAuthorityDistributorDeliveryTarget {
    pub fn new(
        push_remote: impl Into<String>,
        github_repository: impl Into<String>,
        repository_visibility: GithubRepositoryVisibility,
        integration_branch: impl Into<String>,
    ) -> Self {
        Self {
            push_remote: push_remote.into(),
            credential_redacted_push_url: None,
            github_repository: github_repository.into(),
            repository_visibility,
            integration_branch: integration_branch.into(),
        }
    }

    pub fn with_credential_redacted_push_url(
        mut self,
        credential_redacted_push_url: impl Into<String>,
    ) -> Self {
        self.credential_redacted_push_url = Some(credential_redacted_push_url.into());
        self
    }
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
    // Slot that ran the work, used to join queue rows with lease projections.
    #[serde(default)]
    pub slot_id: String,
    // Agent identity shown in queue rows and delivery diagnostics.
    pub agent_id: String,
    // Planning task id that the queued branch attempted to resolve.
    pub task_id: String,
    // Cached title for queue and PR copy without reopening the task authority.
    pub task_title: String,
    // Immutable remote/repository/base target frozen before queue persistence.
    #[serde(default)]
    pub delivery_target: Option<PlanningAuthorityDistributorDeliveryTarget>,
    // Branch the agent started from; legacy records fall back to branch_name.
    #[serde(default)]
    pub source_branch: String,
    // Frozen merge-base between the source result and remote integration branch.
    #[serde(default)]
    pub source_base_commit_sha: String,
    // Frozen source branch tip reviewed by GitHub and targeted for delivery.
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
    // Remote integration head fetched before the dedicated worktree is mutated.
    // This is the compare-and-swap base for crash-safe push resumption.
    #[serde(default)]
    pub integration_base_commit_sha: Option<String>,
    // Detached worktree HEAD produced by the reviewed cherry-pick sequence.
    // Recovery may resume its non-force push only when the worktree still points
    // at this exact commit and the remote still points at the frozen base.
    #[serde(default)]
    pub integration_commit_sha: Option<String>,
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
    // Number of automatic retry attempts already admitted for this delivery.
    #[serde(default)]
    pub retry_attempts: u32,
    // Durable RFC3339 lower bound for the next automatic retry.
    #[serde(default)]
    pub retry_not_before: Option<String>,
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
        .with_identity(
            self.queue_item_id.clone(),
            self.session_key.clone(),
            self.slot_id.clone(),
            self.task_id.clone(),
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

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * runtime event record는 runtime_events append-only log의 operator-facing read model입니다.
 * current projection row는 최신 상태만 담기 때문에, Supersession UI와 복구 진단은 이 feed로
 * 어떤 projection 전이가 어떤 planning revision을 보고 저장됐는지 확인합니다.
 */
pub struct PlanningAuthorityRuntimeEventRecord {
    // Monotonic event sequence assigned inside the authority store.
    pub sequence: i64,
    // Stored transition type such as slot_lease_upsert or session_detail_upsert.
    pub event_kind: String,
    // Projection table family affected by the event.
    pub projection_kind: String,
    // Projection-local row identity, for example slot id or session key.
    pub projection_key: String,
    // Planning revision visible when the runtime event was appended.
    pub observed_planning_revision: i64,
    // Short human-facing event summary stored with the row.
    pub summary: String,
    // Store timestamp used as the operator timeline label.
    pub recorded_at: String,
}

impl PlanningAuthorityRuntimeEventRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sequence: i64,
        event_kind: impl Into<String>,
        projection_kind: impl Into<String>,
        projection_key: impl Into<String>,
        observed_planning_revision: i64,
        summary: impl Into<String>,
        recorded_at: impl Into<String>,
    ) -> Self {
        Self {
            sequence,
            event_kind: event_kind.into(),
            projection_kind: projection_kind.into(),
            projection_key: projection_key.into(),
            observed_planning_revision,
            summary: summary.into(),
            recorded_at: recorded_at.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
/*
 * 이 Snapshot은 PlanningRuntimeProjection과 다른 개념입니다. parallel mode가 저장한 여러 runtime projection
 * row를 한 번의 일관된 authority capture로 묶어 읽기 위한 DB-facing snapshot입니다.
 * authority adapter는 slot lease, session detail, distributor queue를 각각 저장하지만,
 * pool reconcile과 admin file sync는 current row와 최근 runtime event를 같이 봐야 "현재 실행 중인 슬롯",
 * "깨진 lease", "agent session 상태", "통합 대기 큐", "최근 전이"를 일관된 한 화면으로 판단할 수 있습니다.
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
    // Durable orchestrator dispatch commands waiting to assign planning queue work to slots.
    pub dispatch_commands: Vec<ParallelModeDispatchCommandSnapshot>,
    // Recent append-only runtime events, newest first and bounded by the adapter.
    pub runtime_events: Vec<PlanningAuthorityRuntimeEventRecord>,
}

/// Exact durable ownership returned after a scheduler wins one due validation record. The token is
/// single-claim entropy; owner, token, and expiry must all match for renew or settle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationPollLeaseClaim {
    pub record: PrValidationRecord,
    pub repository: String,
    pub owner: String,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub poll_attempt: u64,
    pub consecutive_error_count: u32,
}

/// Atomic inputs for claiming one due validation record. Grouping the lease identity and timing
/// constraints keeps every adapter implementation aligned with the same compare-and-swap contract.
#[derive(Debug, Clone, Copy)]
pub struct PrValidationPollLeaseClaimRequest<'a> {
    pub record_key: &'a PrValidationRecordKey,
    pub owner: &'a str,
    pub token: &'a str,
    pub claimed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub repository_cooldown_since: DateTime<Utc>,
}

/// Atomic inputs for extending one exact live validation lease.
#[derive(Debug, Clone, Copy)]
pub struct PrValidationPollLeaseRenewalRequest<'a> {
    pub record_key: &'a PrValidationRecordKey,
    pub owner: &'a str,
    pub token: &'a str,
    pub expected_expires_at: DateTime<Utc>,
    pub renewed_at: DateTime<Utc>,
    pub renewed_expires_at: DateTime<Utc>,
}

/// Scheduler settlement metadata stored beside the domain record so cadence, provider health, and
/// rate-limit state stay queryable without parsing JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationPollSettlement {
    pub polled_at: DateTime<Utc>,
    pub next_poll_at: DateTime<Utc>,
    pub consecutive_error_count: u32,
    pub error_class: Option<PrValidationPollErrorClass>,
    pub rate_limit_remaining: Option<u64>,
    pub rate_limit_reset_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationAuthorityRecordSnapshot {
    pub record: PrValidationRecord,
    pub updated_at: String,
    pub last_polled_at: Option<String>,
    pub next_poll_at: Option<String>,
    pub poll_attempt: u64,
    pub consecutive_error_count: u32,
    pub last_error_class: Option<PrValidationPollErrorClass>,
    pub rate_limit_remaining: Option<u64>,
    pub rate_limit_reset_at: Option<String>,
    pub operator_paused: bool,
    pub operator_acknowledged_at: Option<String>,
    pub last_operator_command_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationAuthorityAdminAction {
    RetryNow,
    Pause,
    Resume,
    QueueRemediation,
    Acknowledge,
}

impl PrValidationAuthorityAdminAction {
    pub const fn label(self) -> &'static str {
        match self {
            Self::RetryNow => "retry_now",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::QueueRemediation => "queue_remediation",
            Self::Acknowledge => "acknowledge",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PrValidationAuthorityAdminCommandRequest<'a> {
    pub command_id: &'a str,
    pub record_key: &'a PrValidationRecordKey,
    pub action: PrValidationAuthorityAdminAction,
    pub expected_observation_revision: u64,
    pub requested_at: DateTime<Utc>,
    pub remediation_admission_allowed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationAuthorityAdminCommandState {
    Applied,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationAuthorityAdminCommandRejection {
    NotFound,
    StaleRevision,
    IdempotencyConflict,
    ObserveModeAdmission,
    InvalidState,
    RateLimitActive,
    NoActionableFinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrValidationAuthorityAdminCommandOutcome {
    pub command_id: String,
    pub record_key: String,
    pub action: PrValidationAuthorityAdminAction,
    pub state: PrValidationAuthorityAdminCommandState,
    pub rejection: Option<PrValidationAuthorityAdminCommandRejection>,
    pub duplicate: bool,
    pub expected_observation_revision: u64,
    pub observed_revision: Option<u64>,
    pub board_revision: i64,
    pub message: String,
    pub applied_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationAuthorityPagePosition {
    pub terminal_rank: u8,
    pub updated_at: String,
    pub record_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationAuthorityPageRequest {
    pub limit: usize,
    pub terminal_since: String,
    pub stale_before: String,
    pub after: Option<PrValidationAuthorityPagePosition>,
    pub expected_revision: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrValidationAuthorityBoardSummary {
    pub active: usize,
    pub integrated: usize,
    pub verifying: usize,
    pub remediation: usize,
    pub remediation_queued: usize,
    pub verified: usize,
    pub blocked: usize,
    pub failed: usize,
    pub stale: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationAuthorityPage {
    pub revision: i64,
    pub summary: PrValidationAuthorityBoardSummary,
    pub records: Vec<PrValidationAuthorityRecordSnapshot>,
    pub next_position: Option<PrValidationAuthorityPagePosition>,
    pub cursor_reset_required: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum PlanningAuthorityActiveDocumentMutation<'a> {
    Replace {
        relative_path: &'a str,
        body: &'a str,
    },
    RemoveEntry {
        relative_path: &'a str,
    },
    // Repo-scoped drafts live outside active_documents. Full reset carries this
    // explicit operation so draft rows disappear in the same authority transaction.
    ClearStagedDrafts,
}

#[derive(Debug, Clone, Copy)]
/*
 * planning authority 저장소 전체를 한 번에 갱신하기 위한 admin 전용 commit 명령이다.
 * direction/task authority와 operator-facing result output이 같은 logical edit session에 속할 때,
 * concrete authority adapter는 이 값을 하나의 durable transaction으로 반영해 split-brain을 막는다.
 */
pub struct PlanningAuthorityDocumentCommit<'a> {
    pub observed_planning_revision: Option<i64>,
    pub directions: &'a DirectionCatalogDocument,
    pub task_authority: &'a TaskAuthorityDocument,
    pub queue_projection: &'a PriorityQueueProjection,
    // None preserves the existing active result-output row. Narrow maintenance
    // editors use this to commit direction/task authority without rewriting
    // hidden editor context.
    pub result_output_markdown: Option<&'a str>,
    // Supporting document and staged-draft lifecycle mutations that must become
    // visible in the same repo-scoped transaction as the authority rewrite.
    pub active_document_mutations: &'a [PlanningAuthorityActiveDocumentMutation<'a>],
    // Tasks intentionally removed by this edit. The concrete authority store
    // validates runtime ownership and retires terminal residue atomically with
    // the document commit.
    pub retired_task_ids: &'a [String],
    // Workspace-wide admin authority mutations hold an exclusive DB claim. The
    // owner token lets that same mutation commit while every unrelated writer
    // still fails closed against the claim.
    pub authority_mutation_owner_token: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * accepted planning authority를 한 번의 일관된 읽기로 돌려주는 admin read model이다.
 * direction/task authority와 active result output이 같은 planning revision으로 관찰되었음을 나타내며,
 * admin/operator surface가 mixed snapshot을 만들지 않도록 하는 read-side 계약이다.
 */
pub struct PlanningAuthorityDocumentSnapshot {
    pub planning_revision: i64,
    pub directions: DirectionCatalogDocument,
    pub task_authority: TaskAuthorityDocument,
    pub result_output_markdown: String,
}

/*
 * `PlanningAuthorityPort`는 planning authority 저장소의 운영 제어면입니다.
 * task/direction 문서 자체는 `PlanningTaskRepositoryPort`가 다루고, 이 포트는 그 문서들이 놓인
 * authority store의 위치, shadow store 진단, parallel mode runtime projection, 분산 claim을 관리합니다.
 * application service는 이 trait만 보고 공식 SQLite authority인지 테스트용 Noop인지 구분하지 않습니다.
 */
pub trait PlanningAuthorityPort: ParallelModeRuntimeEventLogPort + Send + Sync {
    fn current_process_claim_identity(&self) -> Result<(u32, String)> {
        Err(anyhow::anyhow!(
            "planning authority process identity is unavailable"
        ))
    }

    // Atomic document rewrites are required for production operator mutations.
    // Lightweight test adapters may return false and exercise the explicit
    // sequential fallback owned by the application service.
    fn supports_atomic_planning_authority_documents(&self) -> bool {
        false
    }

    #[cfg(test)]
    fn allows_non_atomic_planning_authority_rewrite_for_tests(&self) -> bool {
        false
    }

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
     * direction/task authority와 operator-facing result output을 하나의 durable authority update로 저장합니다.
     * 지원하지 않는 adapter는 명시적으로 unsupported를 반환해 caller가 half-commit-safe 경로를 강제하게 합니다.
     */
    fn commit_planning_authority_documents(
        &self,
        _workspace_dir: &str,
        _commit: PlanningAuthorityDocumentCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        Err(anyhow!(
            "planning authority document commits are unsupported by this authority adapter"
        ))
    }

    /*
     * direction/task authority와 accepted result output을 같은 authority read snapshot으로 읽습니다.
     * 지원하지 않는 adapter는 None을 반환해 caller가 기존 fallback 조립 경로를 사용할 수 있게 합니다.
     */
    fn load_planning_authority_documents(
        &self,
        _workspace_dir: &str,
    ) -> Result<Option<PlanningAuthorityDocumentSnapshot>> {
        Ok(None)
    }

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

    // Long-running app-server turns renew the claim while they are in flight. A false result
    // means the exact owner/order fence was lost and no result may be applied.
    fn renew_official_refresh_claim(
        &self,
        _workspace_dir: &str,
        _refresh_order: u64,
        _owner_token: &str,
    ) -> Result<bool> {
        Ok(true)
    }

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
     * A refresh that did not commit host-side authority must relinquish only its claim without
     * advancing the execution pointer. The exact owner/order fence keeps a late cancellation from
     * deleting a replacement worker's claim for the same refresh order.
     */
    fn cancel_official_refresh_claim(
        &self,
        workspace_dir: &str,
        refresh_order: u64,
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

    /*
     * 현재 dispatcher가 보유한 queue claim의 lease 시각을 갱신합니다.
     * adapter는 claim kind, queue item scope, owner token이 모두 같은 row만 갱신해야 하며,
     * bool 반환으로 caller가 갱신 직전에 소유권을 잃었는지 원자적으로 판단할 수 있게 합니다.
     */
    fn renew_distributor_queue_claim(
        &self,
        // Authority namespace containing the claim.
        workspace_dir: &str,
        // Stable queue record id whose claim should be renewed.
        queue_item_id: &str,
        // Owner token returned by the successful acquire attempt.
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

    // Serialize an operator task edit against runtime ownership for the same tasks.
    fn acquire_admin_task_mutation_guard(
        &self,
        _workspace_dir: &str,
        _task_ids: &[String],
        _owner_token: &str,
    ) -> Result<()> {
        Err(anyhow!(
            "admin task mutation guards are unsupported by this authority adapter"
        ))
    }

    // Release only the task-edit guard owned by this caller.
    fn release_admin_task_mutation_guard(
        &self,
        _workspace_dir: &str,
        _task_ids: &[String],
        _owner_token: &str,
    ) -> Result<()> {
        Err(anyhow!(
            "admin task mutation guard release is unsupported by this authority adapter"
        ))
    }

    // Serialize workspace-wide operator authority edits against task/runtime ownership.
    fn acquire_admin_authority_mutation_guard(
        &self,
        _workspace_dir: &str,
        _owner_token: &str,
        _action: &str,
    ) -> Result<()> {
        Err(anyhow!(
            "admin authority mutation guards are unsupported by this authority adapter"
        ))
    }

    // Release only the workspace-wide authority guard owned by this caller.
    fn release_admin_authority_mutation_guard(
        &self,
        _workspace_dir: &str,
        _owner_token: &str,
    ) -> Result<()> {
        Err(anyhow!(
            "admin authority mutation guard release is unsupported by this authority adapter"
        ))
    }

    // Backward-compatible file-sync surface shares the authority mutation fence.
    fn acquire_admin_file_sync_guard(
        &self,
        workspace_dir: &str,
        owner_token: &str,
        action: &str,
    ) -> Result<()> {
        self.acquire_admin_authority_mutation_guard(workspace_dir, owner_token, action)
    }

    fn release_admin_file_sync_guard(&self, workspace_dir: &str, owner_token: &str) -> Result<()> {
        self.release_admin_authority_mutation_guard(workspace_dir, owner_token)
    }

    // Insert a pending dispatch command if the same command id is not already stored.
    fn enqueue_runtime_dispatch_command(
        &self,
        workspace_dir: &str,
        command: &ParallelModeDispatchCommandSnapshot,
    ) -> Result<bool>;

    // Claim the oldest pending dispatch command so only one scheduler executes it.
    fn try_claim_next_runtime_dispatch_command(
        &self,
        workspace_dir: &str,
        owner_token: &str,
    ) -> Result<Option<ParallelModeDispatchCommandSnapshot>>;

    // Store the latest state for a dispatch command after execution, block, or cancel.
    fn update_runtime_dispatch_command(
        &self,
        workspace_dir: &str,
        command: &ParallelModeDispatchCommandSnapshot,
    ) -> Result<()>;

    // Cancel all non-terminal dispatch commands for mode-off or recovery boundaries.
    fn cancel_runtime_dispatch_commands(&self, workspace_dir: &str, reason: &str) -> Result<usize>;

    // Clear current parallel runtime rows when the disposable pool is reset on enable.
    fn clear_parallel_runtime_projections(&self, workspace_dir: &str, reason: &str) -> Result<()>;

    // Clear runtime rows that belong to deleted planning tasks.
    fn clear_parallel_runtime_projections_for_tasks(
        &self,
        workspace_dir: &str,
        task_ids: &[String],
        reason: &str,
    ) -> Result<()>;

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

    // Cleanup must delete only the lease generation it inspected. Returning false means the row
    // disappeared or was replaced; callers must preserve the replacement slot generation.
    fn remove_runtime_slot_lease_if_matches(
        &self,
        workspace_dir: &str,
        expected: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<bool>;

    // A failed two-store lifecycle transition may restore only the exact next snapshot it wrote.
    // Returning false preserves a concurrently installed generation or state.
    fn replace_runtime_slot_lease_if_matches(
        &self,
        workspace_dir: &str,
        expected_current: &ParallelModeSlotLeaseSnapshot,
        replacement: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<bool>;

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

    // Load all authoritative PR validation records for one runtime polling delivery.
    fn load_runtime_pr_validation_records(
        &self,
        _workspace_dir: &str,
    ) -> Result<Vec<PrValidationRecord>> {
        Ok(Vec::new())
    }

    /// Bounded keyset page used by operator read models. Implementations must return all
    /// non-terminal records before recent terminal records and must never expose lease tokens.
    fn load_runtime_pr_validation_page(
        &self,
        _workspace_dir: &str,
        _request: &PrValidationAuthorityPageRequest,
    ) -> Result<PrValidationAuthorityPage> {
        Ok(PrValidationAuthorityPage {
            revision: 0,
            summary: PrValidationAuthorityBoardSummary::default(),
            records: Vec::new(),
            next_position: None,
            cursor_reset_required: false,
        })
    }

    /// Read the newest authority event that can change the validation board. This deliberately
    /// excludes unrelated slot, session, and distributor activity.
    fn load_runtime_pr_validation_revision(&self, _workspace_dir: &str) -> Result<i64> {
        Ok(0)
    }

    fn load_runtime_pr_validation_record_snapshot(
        &self,
        _workspace_dir: &str,
        _record_key: &PrValidationRecordKey,
    ) -> Result<Option<PrValidationAuthorityRecordSnapshot>> {
        Ok(None)
    }

    /// Apply one operator command under the validation record's observation revision. Concrete
    /// stores must keep command-id replay and record mutation in one transaction.
    fn execute_runtime_pr_validation_admin_command(
        &self,
        _workspace_dir: &str,
        _request: PrValidationAuthorityAdminCommandRequest<'_>,
    ) -> Result<PrValidationAuthorityAdminCommandOutcome> {
        Err(anyhow!(
            "PR validation Admin commands are unsupported by this adapter"
        ))
    }

    /// Select a bounded set of due, pollable keys without loading every validation record.
    fn load_due_runtime_pr_validation_record_keys(
        &self,
        _workspace_dir: &str,
        _due_at: DateTime<Utc>,
        _repository_cooldown_since: DateTime<Utc>,
        _limit: usize,
    ) -> Result<Vec<PrValidationRecordKey>> {
        Ok(Vec::new())
    }

    /// Claim one due record when no unexpired lease already owns the same repository.
    fn try_claim_runtime_pr_validation_poll(
        &self,
        _workspace_dir: &str,
        _request: PrValidationPollLeaseClaimRequest<'_>,
    ) -> Result<Option<PrValidationPollLeaseClaim>> {
        Err(anyhow!(
            "PR validation poll lease claims are unsupported by this adapter"
        ))
    }

    /// Extend only the exact live claim. A stale owner cannot revive an expired or replaced lease.
    fn renew_runtime_pr_validation_poll_lease(
        &self,
        _workspace_dir: &str,
        _request: PrValidationPollLeaseRenewalRequest<'_>,
    ) -> Result<bool> {
        Err(anyhow!(
            "PR validation poll lease renewal is unsupported by this adapter"
        ))
    }

    /// Release the exact claim and atomically publish its next schedule/provider metadata.
    fn settle_runtime_pr_validation_poll(
        &self,
        _workspace_dir: &str,
        _record_key: &PrValidationRecordKey,
        _owner: &str,
        _token: &str,
        _expected_expires_at: DateTime<Utc>,
        _settlement: &PrValidationPollSettlement,
    ) -> Result<bool> {
        Err(anyhow!(
            "PR validation poll settlement is unsupported by this adapter"
        ))
    }

    // Load one authoritative PR validation record across process/restart boundaries.
    fn load_runtime_pr_validation_record(
        &self,
        _workspace_dir: &str,
        _record_key: &PrValidationRecordKey,
    ) -> Result<Option<PrValidationRecord>> {
        Ok(None)
    }

    // Resolve one durable validation record by the operator-visible pull request number.
    fn load_runtime_pr_validation_record_for_pr(
        &self,
        _workspace_dir: &str,
        _pull_request_number: u64,
    ) -> Result<Option<PrValidationRecord>> {
        Ok(None)
    }

    // Resolve the validation record correlated to one ordinary remediation task.
    fn load_runtime_pr_validation_record_for_remediation(
        &self,
        _workspace_dir: &str,
        _task_id: &str,
    ) -> Result<Option<PrValidationRecord>> {
        Ok(None)
    }

    // Advance or remove a validation record only when the exact prior snapshot still owns the key.
    fn compare_and_swap_runtime_pr_validation_record(
        &self,
        _workspace_dir: &str,
        _record_key: &PrValidationRecordKey,
        _expected: Option<&PrValidationRecord>,
        _replacement: Option<&PrValidationRecord>,
    ) -> Result<bool> {
        Err(anyhow!(
            "PR validation authority compare-and-swap is unsupported by this adapter"
        ))
    }
}

#[derive(Default)]
/*
 * `NoopPlanningAuthorityPort`는 tests가 authority DB 없이 service graph를 조립하기 위한 fake입니다.
 * Production composition은 실제 authority boundary를 명시적으로 주입합니다.
 */
#[cfg(test)]
pub struct NoopPlanningAuthorityPort {
    // Monotonic refresh counter keeps orchestration on the same path as real adapters.
    next_refresh_order: AtomicU64,
    resolve_authority_location_error: Option<&'static str>,
    runtime_projection: Option<PlanningAuthorityRuntimeProjectionSnapshot>,
    shared_runtime_projection: Option<Arc<Mutex<PlanningAuthorityRuntimeProjectionSnapshot>>>,
    shared_runtime_dispatch_mutation_gate: Option<Arc<Mutex<()>>>,
    shared_runtime_dispatch_mutation_count: Option<Arc<AtomicUsize>>,
    cancel_runtime_dispatch_commands_error: Option<&'static str>,
    shared_cancel_runtime_dispatch_commands_error: Option<Arc<Mutex<Option<String>>>>,
    clear_parallel_runtime_projections_error: Option<&'static str>,
    clear_parallel_runtime_projections_for_tasks_error: Option<&'static str>,
    apply_parallel_pool_reset_report_error: Option<&'static str>,
    admin_authority_guard_events: Option<Arc<Mutex<Vec<String>>>>,
    admin_authority_guard_release_error: Option<&'static str>,
}

#[cfg(test)]
impl NoopPlanningAuthorityPort {
    pub fn with_authority_location_error(mut self, message: &'static str) -> Self {
        self.resolve_authority_location_error = Some(message);
        self
    }

    pub fn with_runtime_projection(
        mut self,
        snapshot: PlanningAuthorityRuntimeProjectionSnapshot,
    ) -> Self {
        self.runtime_projection = Some(snapshot);
        self
    }

    pub fn with_shared_runtime_projection(
        mut self,
        snapshot: Arc<Mutex<PlanningAuthorityRuntimeProjectionSnapshot>>,
    ) -> Self {
        self.shared_runtime_projection = Some(snapshot);
        self
    }

    pub fn with_shared_runtime_dispatch_mutation_gate(mut self, gate: Arc<Mutex<()>>) -> Self {
        self.shared_runtime_dispatch_mutation_gate = Some(gate);
        self
    }

    pub fn with_shared_runtime_dispatch_mutation_count(mut self, count: Arc<AtomicUsize>) -> Self {
        self.shared_runtime_dispatch_mutation_count = Some(count);
        self
    }

    pub fn with_cancel_runtime_dispatch_commands_error(mut self, message: &'static str) -> Self {
        self.cancel_runtime_dispatch_commands_error = Some(message);
        self
    }

    pub fn with_shared_cancel_runtime_dispatch_commands_error(
        mut self,
        error: Arc<Mutex<Option<String>>>,
    ) -> Self {
        self.shared_cancel_runtime_dispatch_commands_error = Some(error);
        self
    }

    pub fn with_clear_parallel_runtime_projections_error(mut self, message: &'static str) -> Self {
        self.clear_parallel_runtime_projections_error = Some(message);
        self
    }

    pub fn with_clear_parallel_runtime_projections_for_tasks_error(
        mut self,
        message: &'static str,
    ) -> Self {
        self.clear_parallel_runtime_projections_for_tasks_error = Some(message);
        self
    }

    pub fn with_apply_parallel_pool_reset_report_error(mut self, message: &'static str) -> Self {
        self.apply_parallel_pool_reset_report_error = Some(message);
        self
    }

    pub fn with_admin_authority_guard_events(mut self, events: Arc<Mutex<Vec<String>>>) -> Self {
        self.admin_authority_guard_events = Some(events);
        self
    }

    pub fn with_admin_authority_guard_release_error(mut self, message: &'static str) -> Self {
        self.admin_authority_guard_release_error = Some(message);
        self
    }
}

#[cfg(test)]
impl ParallelModeRuntimeEventLogPort for NoopPlanningAuthorityPort {
    fn load_runtime_event_log(
        &self,
        _workspace_dir: &str,
        _request: ParallelModeRuntimeEventLogRequest,
    ) -> Result<ParallelModeRuntimeEventsSnapshot> {
        Ok(ParallelModeRuntimeEventsSnapshot::empty(
            "runtime event log is unavailable without an authority store",
        ))
    }
}

#[cfg(test)]
impl PlanningAuthorityPort for NoopPlanningAuthorityPort {
    fn current_process_claim_identity(&self) -> Result<(u32, String)> {
        Ok((1, "noop-process-start".to_string()))
    }

    fn allows_non_atomic_planning_authority_rewrite_for_tests(&self) -> bool {
        true
    }

    // Without a store, the supplied workspace is both workspace root and canonical root.
    fn resolve_authority_location(&self, workspace_dir: &str) -> Result<PlanningAuthorityLocation> {
        if let Some(message) = self.resolve_authority_location_error {
            anyhow::bail!(message);
        }
        Ok(PlanningAuthorityLocation {
            // Caller-supplied path as the operational root.
            workspace_root: workspace_dir.to_string(),
            // No repo-scoped normalization exists in the fallback.
            canonical_repo_root: workspace_dir.to_string(),
            // The fallback has no distinct Git common-dir identity.
            repository_identity: workspace_dir.to_string(),
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

    fn renew_official_refresh_claim(
        &self,
        _workspace_dir: &str,
        _refresh_order: u64,
        _owner_token: &str,
    ) -> Result<bool> {
        Ok(true)
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

    // No persisted claim or execution pointer exists in the process-local fallback.
    fn cancel_official_refresh_claim(
        &self,
        _workspace_dir: &str,
        _refresh_order: u64,
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

    // The non-persistent fallback cannot lose a durable claim, so renewal always retains ownership.
    fn renew_distributor_queue_claim(
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
        if let Some(snapshot) = &self.shared_runtime_projection {
            return snapshot
                .lock()
                .map(|snapshot| snapshot.clone())
                .map_err(|_| anyhow!("shared runtime projection lock is poisoned"));
        }
        Ok(self.runtime_projection.clone().unwrap_or_default())
    }

    // This test-only adapter has no concurrent store, so explicit guard calls are no-ops.
    fn acquire_admin_task_mutation_guard(
        &self,
        _workspace_dir: &str,
        _task_ids: &[String],
        _owner_token: &str,
    ) -> Result<()> {
        Ok(())
    }

    fn release_admin_task_mutation_guard(
        &self,
        _workspace_dir: &str,
        _task_ids: &[String],
        _owner_token: &str,
    ) -> Result<()> {
        Ok(())
    }

    fn acquire_admin_authority_mutation_guard(
        &self,
        workspace_dir: &str,
        owner_token: &str,
        action: &str,
    ) -> Result<()> {
        if let Some(events) = &self.admin_authority_guard_events {
            events
                .lock()
                .map_err(|_| anyhow!("admin authority guard event lock is poisoned"))?
                .push(format!("acquire|{workspace_dir}|{action}|{owner_token}"));
        }
        Ok(())
    }

    fn release_admin_authority_mutation_guard(
        &self,
        workspace_dir: &str,
        owner_token: &str,
    ) -> Result<()> {
        if let Some(events) = &self.admin_authority_guard_events {
            events
                .lock()
                .map_err(|_| anyhow!("admin authority guard event lock is poisoned"))?
                .push(format!("release|{workspace_dir}|{owner_token}"));
        }
        if let Some(message) = self.admin_authority_guard_release_error {
            anyhow::bail!(message);
        }
        Ok(())
    }

    fn enqueue_runtime_dispatch_command(
        &self,
        _workspace_dir: &str,
        _command: &ParallelModeDispatchCommandSnapshot,
    ) -> Result<bool> {
        if let Some(count) = &self.shared_runtime_dispatch_mutation_count {
            count.fetch_add(1, Ordering::SeqCst);
        }
        let _guard = self
            .shared_runtime_dispatch_mutation_gate
            .as_ref()
            .map(|gate| {
                gate.lock()
                    .map_err(|_| anyhow!("shared runtime dispatch mutation gate is poisoned"))
            })
            .transpose()?;
        Ok(true)
    }

    fn try_claim_next_runtime_dispatch_command(
        &self,
        _workspace_dir: &str,
        _owner_token: &str,
    ) -> Result<Option<ParallelModeDispatchCommandSnapshot>> {
        Ok(None)
    }

    fn update_runtime_dispatch_command(
        &self,
        _workspace_dir: &str,
        _command: &ParallelModeDispatchCommandSnapshot,
    ) -> Result<()> {
        Ok(())
    }

    fn cancel_runtime_dispatch_commands(
        &self,
        _workspace_dir: &str,
        _reason: &str,
    ) -> Result<usize> {
        if let Some(count) = &self.shared_runtime_dispatch_mutation_count {
            count.fetch_add(1, Ordering::SeqCst);
        }
        let _guard = self
            .shared_runtime_dispatch_mutation_gate
            .as_ref()
            .map(|gate| {
                gate.lock()
                    .map_err(|_| anyhow!("shared runtime dispatch mutation gate is poisoned"))
            })
            .transpose()?;
        if let Some(message) = self.cancel_runtime_dispatch_commands_error {
            anyhow::bail!(message);
        }
        if let Some(error) = self
            .shared_cancel_runtime_dispatch_commands_error
            .as_ref()
            .map(|error| {
                error
                    .lock()
                    .map_err(|_| anyhow!("shared cancel dispatch error is poisoned"))
                    .map(|mut error| error.take())
            })
            .transpose()?
            .flatten()
        {
            anyhow::bail!(error);
        }
        Ok(0)
    }

    // No runtime store exists in the fallback, so clearing is a no-op.
    fn clear_parallel_runtime_projections(
        &self,
        _workspace_dir: &str,
        _reason: &str,
    ) -> Result<()> {
        if let Some(message) = self.clear_parallel_runtime_projections_error {
            anyhow::bail!(message);
        }
        Ok(())
    }

    fn clear_parallel_runtime_projections_for_tasks(
        &self,
        _workspace_dir: &str,
        _task_ids: &[String],
        _reason: &str,
    ) -> Result<()> {
        if let Some(message) = self.clear_parallel_runtime_projections_for_tasks_error {
            anyhow::bail!(message);
        }
        Ok(())
    }

    fn apply_parallel_pool_reset_report(
        &self,
        _workspace_dir: &str,
        _report: &ParallelModePoolResetReport,
    ) -> Result<()> {
        if let Some(message) = self.apply_parallel_pool_reset_report_error {
            anyhow::bail!(message);
        }
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

    fn remove_runtime_slot_lease_if_matches(
        &self,
        _workspace_dir: &str,
        _expected: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<bool> {
        Ok(false)
    }

    fn replace_runtime_slot_lease_if_matches(
        &self,
        _workspace_dir: &str,
        _expected_current: &ParallelModeSlotLeaseSnapshot,
        _replacement: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<bool> {
        Ok(false)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogRequest;
    use crate::domain::parallel_mode::{
        ParallelModeAutomationTrigger, ParallelModeDispatchCommandSnapshot,
    };

    fn queue_record_with_source(
        source_branch: &str,
        source_commit_sha: &str,
    ) -> PlanningAuthorityDistributorQueueRecord {
        PlanningAuthorityDistributorQueueRecord {
            queue_item_id: "queue-1".to_string(),
            queue_order_key: 7,
            session_key: "session-1".to_string(),
            slot_id: "slot-1".to_string(),
            agent_id: "agent-a".to_string(),
            task_id: "task-1".to_string(),
            task_title: "Implement coverage guards".to_string(),
            delivery_target: Some(PlanningAuthorityDistributorDeliveryTarget::new(
                "origin",
                "acme/widgets",
                GithubRepositoryVisibility::Private,
                "prerelease",
            )),
            source_branch: source_branch.to_string(),
            source_base_commit_sha: "0000000000000000".to_string(),
            source_commit_sha: source_commit_sha.to_string(),
            branch_name: "akra-agent/slot-1/task-1".to_string(),
            worktree_path: "/tmp/worktree".to_string(),
            commit_sha: "abcdef1234567890".to_string(),
            original_commit_sha: Some("abcdef1234567890".to_string()),
            planning_refresh_state: "completed".to_string(),
            integration_state: "pr_pending".to_string(),
            integration_base_commit_sha: None,
            integration_commit_sha: None,
            conflict_files: Vec::new(),
            recovery_note: None,
            validation_summary: "cargo test passed".to_string(),
            authority_refresh_outcome: "promoted".to_string(),
            github_capabilities: None,
            pull_request_number: Some(12),
            pull_request_url: Some("https://github.example/pr/12".to_string()),
            queue_state: ParallelModeQueueItemState::PrPending,
            integration_note: "waiting for review".to_string(),
            enqueued_at: "2026-05-23T00:00:00Z".to_string(),
            updated_at: "2026-05-23T00:01:00Z".to_string(),
            retry_attempts: 0,
            retry_not_before: None,
        }
    }

    #[test]
    fn distributor_queue_record_display_uses_effective_source_and_short_commit() {
        let record = queue_record_with_source("prerelease", "1234567890abcdef");

        assert_eq!(record.effective_source_branch(), "prerelease");
        assert_eq!(record.effective_source_commit_sha(), "1234567890abcdef");

        let display = record.display_item();

        assert_eq!(display.source_agent, "agent-a");
        assert_eq!(display.task_title, "Implement coverage guards");
        assert_eq!(display.queue_state, ParallelModeQueueItemState::PrPending);
        assert_eq!(display.branch_name, "prerelease");
        assert_eq!(display.commit_short_sha, "abcdef1");
        assert_eq!(display.integration_note, "waiting for review");
        let identity = display
            .identity
            .expect("display item should preserve typed delivery identity");
        assert_eq!(identity.queue_item_id, "queue-1");
        assert_eq!(identity.session_key, "session-1");
        assert_eq!(identity.slot_id, "slot-1");
        assert_eq!(identity.task_id, "task-1");
    }

    #[test]
    fn distributor_queue_record_preserves_legacy_branch_and_commit_fallbacks() {
        let record = queue_record_with_source("   ", "");

        assert_eq!(record.effective_source_branch(), "akra-agent/slot-1/task-1");
        assert_eq!(record.effective_source_commit_sha(), "abcdef1234567890");
    }

    #[test]
    fn distributor_delivery_target_round_trip_never_contains_remote_credentials() {
        let record = queue_record_with_source("prerelease", "1234567890abcdef");
        let serialized = serde_json::to_string(&record).expect("queue record should serialize");

        assert!(serialized.contains("acme/widgets"));
        assert!(!serialized.contains("secret-token"));
        assert_eq!(
            serde_json::from_str::<PlanningAuthorityDistributorQueueRecord>(&serialized)
                .expect("queue record should deserialize"),
            record
        );
    }

    #[test]
    fn noop_planning_authority_reports_empty_in_sync_fallbacks() {
        let port = NoopPlanningAuthorityPort::default();

        let location = port
            .resolve_authority_location("/tmp/root")
            .expect("fallback location should resolve");
        assert_eq!(location.workspace_root, "/tmp/root");
        assert_eq!(location.canonical_repo_root, "/tmp/root");
        assert_eq!(location.repository_identity, "/tmp/root");
        assert_eq!(location.runtime_dir, "");
        assert_eq!(location.authority_store_path, "");

        let inspection = port
            .inspect_shadow_store("/tmp/root")
            .expect("fallback shadow inspection should resolve");
        assert_eq!(
            inspection.sync_state,
            PlanningAuthorityShadowStoreSyncState::InSync
        );
        assert_eq!(inspection.mirrored_document_count, 0);
        assert_eq!(inspection.parity_issue_count, 0);
        assert!(inspection.parity_issue_examples.is_empty());

        assert_eq!(
            port.load_runtime_event_log("/tmp/root", ParallelModeRuntimeEventLogRequest::default())
                .expect("runtime event log should load")
                .compact_summary(),
            "runtime event log is unavailable without an authority store"
        );
        assert!(
            port.load_runtime_projections("/tmp/root")
                .expect("runtime projections should load")
                .distributor_queue_records
                .is_empty()
        );
    }

    #[test]
    fn noop_planning_authority_keeps_callers_on_claim_and_command_paths() {
        let port = NoopPlanningAuthorityPort::default();

        assert_eq!(
            port.reserve_next_official_refresh_order("/tmp/root")
                .expect("first order should reserve"),
            1
        );
        assert_eq!(
            port.reserve_next_official_refresh_order("/tmp/root")
                .expect("second order should reserve"),
            2
        );
        assert_eq!(
            port.acquire_official_refresh_claim("/tmp/root", 1, "owner")
                .expect("fallback claim should acquire"),
            PlanningAuthorityOfficialRefreshClaimStatus::Acquired
        );
        port.release_official_refresh_claim("/tmp/root", 1, "owner")
            .expect("fallback release should succeed");
        assert_eq!(
            port.abandon_next_official_refresh_order("/tmp/root", "test")
                .expect("fallback abandon should resolve"),
            PlanningAuthorityOfficialRefreshRecoveryStatus::NoPendingOrder
        );
        assert!(
            port.try_acquire_distributor_queue_claim("/tmp/root", "queue-1", "owner")
                .expect("fallback queue claim should acquire")
        );
        assert!(
            port.renew_distributor_queue_claim("/tmp/root", "queue-1", "owner")
                .expect("fallback queue claim should renew")
        );
        port.release_distributor_queue_claim("/tmp/root", "queue-1", "owner")
            .expect("fallback queue release should succeed");

        let command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
            ParallelModeAutomationTrigger::MainTurnPostEvaluation,
            Some("task-1".to_string()),
            Some(3),
            "2026-05-23T00:00:00Z",
        );
        assert!(
            port.enqueue_runtime_dispatch_command("/tmp/root", &command)
                .expect("fallback enqueue should report inserted")
        );
        assert!(
            port.try_claim_next_runtime_dispatch_command("/tmp/root", "owner")
                .expect("fallback claim should load")
                .is_none()
        );
        port.update_runtime_dispatch_command("/tmp/root", &command)
            .expect("fallback update should succeed");
        assert_eq!(
            port.cancel_runtime_dispatch_commands("/tmp/root", "reset")
                .expect("fallback cancel should succeed"),
            0
        );
    }
}

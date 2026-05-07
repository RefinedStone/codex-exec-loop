use serde::{Deserialize, Serialize};

// 병렬 모드 도메인의 공개 관문이다. 세부 투영은 하위 모듈에 두고, 이 파일은
// 서비스와 TUI가 공유하는 supervisor/slot/lease/pool board 어휘를 한곳에 모은다.
mod agent_session;
mod distributor;
mod orchestrator;
mod pool_reset;
mod readiness;
mod runtime_events;

// 호출자는 `domain::parallel_mode::*`만 알면 된다. 하위 모듈 경로는 이 퍼사드가
// 흡수해 병렬 모드 화면과 서비스가 같은 도메인 언어를 쓰게 둔다.
pub use self::agent_session::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeLiveSessionDetailDefaults, ParallelModeSupervisorDetailSnapshot,
};
use self::agent_session::{roster_recency_key, roster_state_priority};
pub use self::distributor::{
    ParallelModeCompletionFeedEntry, ParallelModeDistributorQueueItem,
    ParallelModeDistributorSnapshot, ParallelModeOrchestratorStatus, ParallelModeQueueItemState,
    ParallelModeRuntimeEventFeedEntry, ParallelModeSupervisorSnapshot,
};
pub use self::orchestrator::{
    ParallelModeAutomationTrigger, ParallelModeDispatchBlockReason,
    ParallelModeDispatchCommandSnapshot, ParallelModeDispatchCommandState,
    ParallelModeDispatchOutcome, ParallelModeOrchestratorState,
    ParallelModeOrchestratorStateMachine, ParallelModePoolResetScope,
    ParallelModePostTurnQueueSignal, ParallelModeRuntimeEvent,
    ParallelModeTaskDispatchBlockSnapshot,
};
pub use self::pool_reset::{
    ParallelModePoolResetPolicy, ParallelModePoolResetReport, ParallelModePoolResetRunId,
    ParallelModePoolResetSlotAction, ParallelModePoolResetSlotOutcome,
    ParallelModePoolResetSlotReport,
};
pub use self::readiness::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModeReadinessSnapshot, ParallelModeReadinessState,
};
pub use self::runtime_events::{ParallelModeRuntimeEventEntry, ParallelModeRuntimeEventsSnapshot};

// supervisor 상태는 병렬 모드 전체 제어면의 큰 흐름이다. 준비, 정상 감독,
// 복구를 readiness gate와 사용자의 mode toggle 조합으로만 판정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeSupervisorState {
    Prepare,
    Supervise,
    Recover,
}

impl ParallelModeSupervisorState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Supervise => "supervise",
            Self::Recover => "recover",
        }
    }

    // readiness가 병렬 실행을 막는 동안에는 toggle이 켜져도 Recover가 우선이다.
    // 이 규칙 덕분에 TUI와 서비스가 서로 다른 fallback 상태를 만들지 않는다.
    pub fn derive(
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    ) -> Self {
        if mode_enabled
            && readiness_snapshot.is_some_and(|snapshot| !snapshot.allows_parallel_mode())
        {
            return Self::Recover;
        }

        if mode_enabled {
            return Self::Supervise;
        }

        Self::Prepare
    }
}

// pool slot 상태는 실제 lease 상태보다 넓다. 구성된 슬롯, 작업 lease, worktree
// 존재 여부, 차단 사유를 합쳐 TUI board가 바로 렌더링할 수 있는 행 상태로 만든다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModePoolSlotState {
    Idle,
    Leased,
    Running,
    AwaitingCleanup,
    Blocked,
    Missing,
    Unavailable,
}

impl ParallelModePoolSlotState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Leased => "leased",
            Self::Running => "running",
            Self::AwaitingCleanup => "awaiting_cleanup",
            Self::Blocked => "blocked",
            Self::Missing => "missing",
            Self::Unavailable => "unavailable",
        }
    }
}

// lease 상태는 저장 가능한 작업 점유 생명주기다. pool slot state로 변환될 수 있지만
// Missing/Blocked 같은 파일 시스템 관찰 상태는 lease 자체에 섞지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModeSlotLeaseState {
    Leased,
    Running,
    CleanupPending,
}

impl ParallelModeSlotLeaseState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Leased => "leased",
            Self::Running => "running",
            Self::CleanupPending => "cleanup_pending",
        }
    }

    pub fn pool_slot_state(self) -> ParallelModePoolSlotState {
        match self {
            Self::Leased => ParallelModePoolSlotState::Leased,
            Self::Running => ParallelModePoolSlotState::Running,
            Self::CleanupPending => ParallelModePoolSlotState::AwaitingCleanup,
        }
    }
}

// slot을 잡기 전 dispatcher가 넘기는 최소 요청이다. branch/worktree는 할당 과정의
// 산물이므로 요청에는 task, agent, slug만 담아 lease 생성 책임을 분리한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeSlotLeaseRequest {
    pub task_id: String,
    pub task_title: String,
    pub agent_id: String,
    pub task_slug: String,
}

impl ParallelModeSlotLeaseRequest {
    pub fn new(
        task_id: impl Into<String>,
        task_title: impl Into<String>,
        agent_id: impl Into<String>,
        task_slug: impl Into<String>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            task_title: task_title.into(),
            agent_id: agent_id.into(),
            task_slug: task_slug.into(),
        }
    }
}

// lease snapshot은 slot 소유권의 기준 데이터다. branch, worktree, agent, task,
// 시간 정보를 함께 보존해 재시작 뒤에도 같은 병렬 작업을 다시 식별할 수 있다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeSlotLeaseSnapshot {
    pub slot_id: String,
    pub task_id: String,
    pub task_title: String,
    pub agent_id: String,
    pub branch_name: String,
    pub worktree_path: String,
    pub state: ParallelModeSlotLeaseState,
    pub leased_at: String,
    pub running_started_at: Option<String>,
}

impl ParallelModeSlotLeaseSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        slot_id: impl Into<String>,
        task_id: impl Into<String>,
        task_title: impl Into<String>,
        agent_id: impl Into<String>,
        branch_name: impl Into<String>,
        worktree_path: impl Into<String>,
        state: ParallelModeSlotLeaseState,
        leased_at: impl Into<String>,
        running_started_at: Option<String>,
    ) -> Self {
        Self {
            slot_id: slot_id.into(),
            task_id: task_id.into(),
            task_title: task_title.into(),
            agent_id: agent_id.into(),
            branch_name: branch_name.into(),
            worktree_path: worktree_path.into(),
            state,
            leased_at: leased_at.into(),
            running_started_at,
        }
    }

    pub fn owner_label(&self) -> String {
        format!("{} / {}", self.agent_id, self.task_id)
    }

    pub fn session_key(&self) -> String {
        format!("{}@{}", self.slot_id, self.leased_at)
    }

    // lease가 Running 이후라면 agent session의 더 세밀한 후속 파이프라인 상태를
    // 화면에 올릴 수 있다. Leased 단계는 아직 런타임 세션이 주인이 아니므로 덮지 않는다.
    pub fn runtime_state_override<'a>(
        &self,
        detail: &'a ParallelModeAgentSessionDetailSnapshot,
    ) -> Option<&'a str> {
        match self.state {
            ParallelModeSlotLeaseState::Running => match detail.state_label.as_str() {
                "reported_complete"
                | "ledger_refreshing"
                | "commit_ready"
                | "merge_queued"
                | "pushing"
                | "pr_pending"
                | "merge_pending"
                | "integrating"
                | "official_refresh_recovery_needed"
                | "failed" => Some(detail.state_label.as_str()),
                _ => None,
            },
            ParallelModeSlotLeaseState::CleanupPending => match detail.state_label.as_str() {
                "official_refresh_recovery_needed" | "failed" => Some(detail.state_label.as_str()),
                _ => None,
            },
            ParallelModeSlotLeaseState::Leased => None,
        }
    }

    // roster 정렬과 slot 선택은 같은 우선순위 함수를 공유한다. 새 상태가 추가될 때
    // agent_session 쪽 정렬 규칙과 이 core snapshot의 선택 규칙이 함께 움직여야 한다.
    pub fn selection_priority(&self) -> (u8, &str) {
        (roster_state_priority(self.state), roster_recency_key(self))
    }
}

// pool board 한 행에 필요한 표시 모델이다. adapter가 경로와 branch를 사람이 읽는
// label로 정리한 뒤 이 타입으로 넘기면 TUI는 추가 매핑 없이 그릴 수 있다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModePoolSlotSnapshot {
    pub slot_id: String,
    pub state: ParallelModePoolSlotState,
    pub branch_name: String,
    pub worktree_label: String,
    pub owner_label: String,
}

impl ParallelModePoolSlotSnapshot {
    pub fn new(
        slot_id: impl Into<String>,
        state: ParallelModePoolSlotState,
        branch_name: impl Into<String>,
        worktree_label: impl Into<String>,
        owner_label: impl Into<String>,
    ) -> Self {
        Self {
            slot_id: slot_id.into(),
            state,
            branch_name: branch_name.into(),
            worktree_label: worktree_label.into(),
            owner_label: owner_label.into(),
        }
    }

    // lease snapshot에서 온 행은 lease 생명주기를 board 상태로 변환한다. 이 변환은
    // TUI가 cleanup pending을 별도 열로 세기 위한 단일 기준이다.
    pub fn from_lease(
        slot_id: impl Into<String>,
        branch_name: impl Into<String>,
        worktree_label: impl Into<String>,
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Self {
        Self::new(
            slot_id,
            lease.state.pool_slot_state(),
            branch_name,
            worktree_label,
            lease.owner_label(),
        )
    }
}

// cleanup 판단은 lease 상태와 git/worktree 관찰값의 교차 규칙이다. 활성 lease는
// 절대 청소 대상이 아니며, lease가 사라진 잔여 worktree는 clean+integrated일 때만 허용한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelModePoolSlotCleanupDecision {
    pub lease_state: Option<ParallelModeSlotLeaseState>,
    pub worktree_clean: bool,
    pub branch_integrated: bool,
}

impl ParallelModePoolSlotCleanupDecision {
    pub fn new(
        lease_state: Option<ParallelModeSlotLeaseState>,
        worktree_clean: bool,
        branch_integrated: bool,
    ) -> Self {
        Self {
            lease_state,
            worktree_clean,
            branch_integrated,
        }
    }

    pub fn is_cleanup_ready(self) -> bool {
        match self.lease_state {
            Some(ParallelModeSlotLeaseState::CleanupPending) => self.branch_integrated,
            Some(ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running) => false,
            None => self.worktree_clean && self.branch_integrated,
        }
    }
}

// pool board snapshot은 slot 행과 집계 카운트를 함께 가진다. 서비스가 한 번만 집계해
// TUI와 CLI report가 동일한 exhausted/reconcile 상태를 보게 만드는 읽기 모델이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModePoolBoardSnapshot {
    pub configured_size: usize,
    pub pool_root_label: String,
    pub idle_slots: usize,
    pub leased_slots: usize,
    pub running_slots: usize,
    pub awaiting_cleanup_slots: usize,
    pub blocked_slots: usize,
    pub missing_slots: usize,
    pub unavailable_slots: usize,
    pub exhausted: bool,
    pub reconcile_status: String,
    pub slots: Vec<ParallelModePoolSlotSnapshot>,
}

impl ParallelModePoolBoardSnapshot {
    pub fn new(
        configured_size: usize,
        pool_root_label: impl Into<String>,
        reconcile_status: impl Into<String>,
        slots: Vec<ParallelModePoolSlotSnapshot>,
    ) -> Self {
        let idle_slots = slots
            .iter()
            .filter(|slot| slot.state == ParallelModePoolSlotState::Idle)
            .count();
        let leased_slots = slots
            .iter()
            .filter(|slot| slot.state == ParallelModePoolSlotState::Leased)
            .count();
        let running_slots = slots
            .iter()
            .filter(|slot| slot.state == ParallelModePoolSlotState::Running)
            .count();
        let awaiting_cleanup_slots = slots
            .iter()
            .filter(|slot| slot.state == ParallelModePoolSlotState::AwaitingCleanup)
            .count();
        let blocked_slots = slots
            .iter()
            .filter(|slot| slot.state == ParallelModePoolSlotState::Blocked)
            .count();
        let missing_slots = slots
            .iter()
            .filter(|slot| slot.state == ParallelModePoolSlotState::Missing)
            .count();
        let unavailable_slots = slots
            .iter()
            .filter(|slot| slot.state == ParallelModePoolSlotState::Unavailable)
            .count();
        // exhausted는 단순히 idle이 없는 상태가 아니다. 구성된 pool에 실제 진행 또는
        // cleanup 대기 작업이 있어 새 lease를 줄 수 없을 때만 압박 신호로 올린다.
        let exhausted = configured_size > 0
            && idle_slots == 0
            && leased_slots + running_slots + awaiting_cleanup_slots > 0;

        Self {
            configured_size,
            pool_root_label: pool_root_label.into(),
            idle_slots,
            leased_slots,
            running_slots,
            awaiting_cleanup_slots,
            blocked_slots,
            missing_slots,
            unavailable_slots,
            exhausted,
            reconcile_status: reconcile_status.into(),
            slots,
        }
    }

    // 상단 status line에 쓰는 압축 문자열이다. 0인 보조 상태를 숨겨 폭이 좁은 TUI에서도
    // 핵심 병목만 남기고, idle은 항상 보여 pool 크기 기준을 잃지 않게 한다.
    pub fn compact_summary(&self) -> String {
        let mut parts = vec![format!("idle {}/{}", self.idle_slots, self.configured_size)];

        if self.leased_slots > 0 {
            parts.push(format!("leased {}", self.leased_slots));
        }
        if self.running_slots > 0 {
            parts.push(format!("running {}", self.running_slots));
        }
        if self.awaiting_cleanup_slots > 0 {
            parts.push(format!("cleanup {}", self.awaiting_cleanup_slots));
        }
        if self.blocked_slots > 0 {
            parts.push(format!("blocked {}", self.blocked_slots));
        }
        if self.missing_slots > 0 {
            parts.push(format!("missing {}", self.missing_slots));
        }
        if self.unavailable_slots > 0 {
            parts.push(format!("unavailable {}", self.unavailable_slots));
        }

        parts.join(" / ")
    }
}

#[cfg(test)]
mod tests;

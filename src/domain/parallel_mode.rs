use serde::{Deserialize, Serialize};

mod agent_session;
mod distributor;
mod readiness;

pub use self::agent_session::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeLiveSessionDetailDefaults, ParallelModeSupervisorDetailSnapshot,
    roster_latest_summary, roster_state_label,
};
use self::agent_session::{roster_recency_key, roster_state_priority};
pub use self::distributor::{
    ParallelModeCompletionFeedEntry, ParallelModeDistributorQueueItem,
    ParallelModeDistributorSnapshot, ParallelModeOrchestratorStatus, ParallelModeQueueItemState,
    ParallelModeSupervisorSnapshot,
};
pub use self::readiness::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModeReadinessSnapshot, ParallelModeReadinessState,
};

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

    pub fn runtime_state_override<'a>(
        &self,
        detail: &'a ParallelModeAgentSessionDetailSnapshot,
    ) -> Option<&'a str> {
        match self.state {
            ParallelModeSlotLeaseState::Running => match detail.state_label.as_str() {
                "reported_complete" | "ledger_refreshing" | "commit_ready" | "merge_queued"
                | "pushing" | "pr_pending" | "merge_pending" | "integrating" | "failed" => {
                    Some(detail.state_label.as_str())
                }
                _ => None,
            },
            ParallelModeSlotLeaseState::CleanupPending => match detail.state_label.as_str() {
                "failed" => Some(detail.state_label.as_str()),
                _ => None,
            },
            ParallelModeSlotLeaseState::Leased => None,
        }
    }

    pub fn selection_priority(&self) -> (u8, &str) {
        (roster_state_priority(self.state), roster_recency_key(self))
    }
}

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

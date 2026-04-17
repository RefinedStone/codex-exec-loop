use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeReadinessState {
    Ready,
    Degraded,
    Blocked,
    Repairing,
}

impl ParallelModeReadinessState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Blocked => "blocked",
            Self::Repairing => "repairing",
        }
    }

    pub fn allows_parallel_mode(self) -> bool {
        matches!(self, Self::Ready | Self::Degraded)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeCapabilityKey {
    GitRepository,
    GitWorktree,
    AkraBranch,
    PushRemote,
    GhBinary,
    GhAuth,
    Planning,
}

impl ParallelModeCapabilityKey {
    pub fn label(self) -> &'static str {
        match self {
            Self::GitRepository => "git repo",
            Self::GitWorktree => "git worktree",
            Self::AkraBranch => "akra branch",
            Self::PushRemote => "push",
            Self::GhBinary => "gh binary",
            Self::GhAuth => "gh auth",
            Self::Planning => "planning",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeCapabilityState {
    Ready,
    Degraded,
    Blocked,
    Repairing,
}

impl ParallelModeCapabilityState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Blocked => "blocked",
            Self::Repairing => "repairing",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeCapabilitySnapshot {
    pub key: ParallelModeCapabilityKey,
    pub state: ParallelModeCapabilityState,
    pub detail: String,
    pub next_action: Option<String>,
}

impl ParallelModeCapabilitySnapshot {
    pub fn new(
        key: ParallelModeCapabilityKey,
        state: ParallelModeCapabilityState,
        detail: impl Into<String>,
        next_action: Option<String>,
    ) -> Self {
        Self {
            key,
            state,
            detail: detail.into(),
            next_action,
        }
    }

    pub fn summary(&self) -> String {
        match &self.next_action {
            Some(next_action) => format!(
                "{}: {} / cause: {} / next action: {}",
                self.key.label(),
                self.state.label(),
                self.detail,
                next_action
            ),
            None => format!(
                "{}: {} / detail: {}",
                self.key.label(),
                self.state.label(),
                self.detail
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeReadinessSnapshot {
    pub workspace_path: String,
    pub readiness: ParallelModeReadinessState,
    pub capabilities: Vec<ParallelModeCapabilitySnapshot>,
    pub top_alert: Option<String>,
}

impl ParallelModeReadinessSnapshot {
    pub fn new(
        workspace_path: impl Into<String>,
        readiness: ParallelModeReadinessState,
        capabilities: Vec<ParallelModeCapabilitySnapshot>,
        top_alert: Option<String>,
    ) -> Self {
        Self {
            workspace_path: workspace_path.into(),
            readiness,
            capabilities,
            top_alert,
        }
    }

    pub fn readiness_label(&self) -> &'static str {
        self.readiness.label()
    }

    pub fn allows_parallel_mode(&self) -> bool {
        self.readiness.allows_parallel_mode()
    }

    pub fn capability(
        &self,
        key: ParallelModeCapabilityKey,
    ) -> Option<&ParallelModeCapabilitySnapshot> {
        self.capabilities
            .iter()
            .find(|capability| capability.key == key)
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeAgentRosterEntry {
    pub agent_id: String,
    pub task_title: String,
    pub slot_id: String,
    pub branch_name: String,
    pub state_label: String,
    pub duration_label: String,
    pub latest_summary: String,
}

impl ParallelModeAgentRosterEntry {
    pub fn new(
        agent_id: impl Into<String>,
        task_title: impl Into<String>,
        slot_id: impl Into<String>,
        branch_name: impl Into<String>,
        state_label: impl Into<String>,
        duration_label: impl Into<String>,
        latest_summary: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            task_title: task_title.into(),
            slot_id: slot_id.into(),
            branch_name: branch_name.into(),
            state_label: state_label.into(),
            duration_label: duration_label.into(),
            latest_summary: latest_summary.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeAgentSessionHistoryEntry {
    pub state_label: String,
    pub timestamp: String,
    pub summary: String,
}

impl ParallelModeAgentSessionHistoryEntry {
    pub fn new(
        state_label: impl Into<String>,
        timestamp: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            state_label: state_label.into(),
            timestamp: timestamp.into(),
            summary: summary.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeAgentSessionDetailSnapshot {
    pub session_key: String,
    pub agent_id: String,
    pub task_id: String,
    pub task_title: String,
    pub slot_id: String,
    pub thread_id: Option<String>,
    pub worktree_path: String,
    pub branch_name: String,
    pub lease_started_at: String,
    pub state_label: String,
    pub completion_state_label: String,
    pub latest_summary: String,
    pub validation_summary: String,
    pub ledger_refresh_outcome: String,
    pub distributor_outcome: Option<String>,
    pub history: Vec<ParallelModeAgentSessionHistoryEntry>,
    pub updated_at: String,
}

impl ParallelModeAgentSessionDetailSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_key: impl Into<String>,
        agent_id: impl Into<String>,
        task_id: impl Into<String>,
        task_title: impl Into<String>,
        slot_id: impl Into<String>,
        thread_id: Option<String>,
        worktree_path: impl Into<String>,
        branch_name: impl Into<String>,
        lease_started_at: impl Into<String>,
        state_label: impl Into<String>,
        completion_state_label: impl Into<String>,
        latest_summary: impl Into<String>,
        validation_summary: impl Into<String>,
        ledger_refresh_outcome: impl Into<String>,
        distributor_outcome: Option<String>,
        history: Vec<ParallelModeAgentSessionHistoryEntry>,
        updated_at: impl Into<String>,
    ) -> Self {
        Self {
            session_key: session_key.into(),
            agent_id: agent_id.into(),
            task_id: task_id.into(),
            task_title: task_title.into(),
            slot_id: slot_id.into(),
            thread_id,
            worktree_path: worktree_path.into(),
            branch_name: branch_name.into(),
            lease_started_at: lease_started_at.into(),
            state_label: state_label.into(),
            completion_state_label: completion_state_label.into(),
            latest_summary: latest_summary.into(),
            validation_summary: validation_summary.into(),
            ledger_refresh_outcome: ledger_refresh_outcome.into(),
            distributor_outcome,
            history,
            updated_at: updated_at.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeSupervisorDetailSnapshot {
    pub session: Option<ParallelModeAgentSessionDetailSnapshot>,
    pub empty_state: String,
}

impl ParallelModeSupervisorDetailSnapshot {
    pub fn new(
        session: Option<ParallelModeAgentSessionDetailSnapshot>,
        empty_state: impl Into<String>,
    ) -> Self {
        Self {
            session,
            empty_state: empty_state.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeAgentRosterSnapshot {
    pub entries: Vec<ParallelModeAgentRosterEntry>,
    pub empty_state: String,
}

impl ParallelModeAgentRosterSnapshot {
    pub fn new(entries: Vec<ParallelModeAgentRosterEntry>, empty_state: impl Into<String>) -> Self {
        Self {
            entries,
            empty_state: empty_state.into(),
        }
    }

    pub fn active_count(&self) -> usize {
        self.entries.len()
    }

    pub fn compact_summary(&self) -> String {
        format!("{} active", self.active_count())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModeQueueItemState {
    Idle,
    Queued,
    Pushing,
    PrPending,
    MergePending,
    Integrating,
    Cleaning,
    Done,
    Blocked,
    Failed,
}

impl ParallelModeQueueItemState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Queued => "queued",
            Self::Pushing => "pushing",
            Self::PrPending => "pr_pending",
            Self::MergePending => "merge_pending",
            Self::Integrating => "integrating",
            Self::Cleaning => "cleaning",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
        }
    }

    pub fn is_active(self) -> bool {
        !matches!(self, Self::Idle | Self::Done | Self::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeCompletionFeedEntry {
    pub stage_label: String,
    pub summary: String,
}

impl ParallelModeCompletionFeedEntry {
    pub fn new(stage_label: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            stage_label: stage_label.into(),
            summary: summary.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeDistributorQueueItem {
    pub source_agent: String,
    pub task_title: String,
    pub queue_state: ParallelModeQueueItemState,
    pub branch_name: String,
    pub commit_short_sha: String,
    pub integration_note: String,
}

impl ParallelModeDistributorQueueItem {
    pub fn new(
        source_agent: impl Into<String>,
        task_title: impl Into<String>,
        queue_state: ParallelModeQueueItemState,
        branch_name: impl Into<String>,
        commit_short_sha: impl Into<String>,
        integration_note: impl Into<String>,
    ) -> Self {
        Self {
            source_agent: source_agent.into(),
            task_title: task_title.into(),
            queue_state,
            branch_name: branch_name.into(),
            commit_short_sha: commit_short_sha.into(),
            integration_note: integration_note.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeDistributorSnapshot {
    pub queue_items: Vec<ParallelModeDistributorQueueItem>,
    pub completion_feed: Vec<ParallelModeCompletionFeedEntry>,
    pub head_summary: String,
    pub note: String,
}

impl ParallelModeDistributorSnapshot {
    pub fn new(
        queue_items: Vec<ParallelModeDistributorQueueItem>,
        completion_feed: Vec<ParallelModeCompletionFeedEntry>,
        head_summary: impl Into<String>,
        note: impl Into<String>,
    ) -> Self {
        Self {
            queue_items,
            completion_feed,
            head_summary: head_summary.into(),
            note: note.into(),
        }
    }

    pub fn queue_depth(&self) -> usize {
        self.queue_items.len()
    }

    pub fn compact_summary(&self) -> String {
        if self.queue_items.is_empty() {
            return self.head_summary.clone();
        }

        format!("{} / depth {}", self.head_summary, self.queue_depth())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeSupervisorSnapshot {
    pub state: ParallelModeSupervisorState,
    pub workspace_path: String,
    pub pool: ParallelModePoolBoardSnapshot,
    pub roster: ParallelModeAgentRosterSnapshot,
    pub detail: ParallelModeSupervisorDetailSnapshot,
    pub distributor: ParallelModeDistributorSnapshot,
    pub top_notice: Option<String>,
}

impl ParallelModeSupervisorSnapshot {
    pub fn new(
        state: ParallelModeSupervisorState,
        workspace_path: impl Into<String>,
        pool: ParallelModePoolBoardSnapshot,
        roster: ParallelModeAgentRosterSnapshot,
        detail: ParallelModeSupervisorDetailSnapshot,
        distributor: ParallelModeDistributorSnapshot,
        top_notice: Option<String>,
    ) -> Self {
        Self {
            state,
            workspace_path: workspace_path.into(),
            pool,
            roster,
            detail,
            distributor,
            top_notice,
        }
    }

    pub fn state_label(&self) -> &'static str {
        self.state.label()
    }
}

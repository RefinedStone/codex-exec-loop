use serde::{Deserialize, Serialize};

mod agent_session;
mod distributor;

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

    pub fn derive_from_capabilities(capabilities: &[ParallelModeCapabilitySnapshot]) -> Self {
        let mut degraded = false;
        for capability in capabilities {
            match capability.state {
                ParallelModeCapabilityState::Blocked => return Self::Blocked,
                ParallelModeCapabilityState::Degraded | ParallelModeCapabilityState::Repairing => {
                    degraded = true;
                }
                ParallelModeCapabilityState::Ready => {}
            }
        }

        if degraded {
            Self::Degraded
        } else {
            Self::Ready
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModeCapabilityKey {
    GitRepository,
    GitWorktree,
    AkraBranch,
    PushRemote,
    GhBinary,
    GhAuth,
    Planning,
    AuthorityStore,
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
            Self::AuthorityStore => "authority store",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
mod tests {
    use std::collections::BTreeMap;

    use super::{
        ParallelModeAgentSessionDetailSnapshot, ParallelModeCapabilityKey,
        ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
        ParallelModeLiveSessionDetailDefaults, ParallelModePoolSlotCleanupDecision,
        ParallelModePoolSlotState, ParallelModeReadinessSnapshot, ParallelModeReadinessState,
        ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState, ParallelModeSupervisorState,
    };

    #[test]
    fn readiness_derivation_marks_blocked_when_any_blocker_exists() {
        let readiness = ParallelModeReadinessState::derive_from_capabilities(&[
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GitRepository,
                ParallelModeCapabilityState::Ready,
                "ready",
                None,
            ),
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::Planning,
                ParallelModeCapabilityState::Blocked,
                "planning invalid",
                Some("repair planning".to_string()),
            ),
        ]);

        assert_eq!(readiness, ParallelModeReadinessState::Blocked);
    }

    #[test]
    fn readiness_derivation_marks_degraded_when_only_optional_capabilities_fail() {
        let readiness = ParallelModeReadinessState::derive_from_capabilities(&[
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GitRepository,
                ParallelModeCapabilityState::Ready,
                "ready",
                None,
            ),
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Degraded,
                "push unavailable",
                Some("restore auth".to_string()),
            ),
        ]);

        assert_eq!(readiness, ParallelModeReadinessState::Degraded);
    }

    #[test]
    fn readiness_derivation_marks_degraded_when_capability_is_repairing() {
        let readiness = ParallelModeReadinessState::derive_from_capabilities(&[
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GitRepository,
                ParallelModeCapabilityState::Ready,
                "ready",
                None,
            ),
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::Planning,
                ParallelModeCapabilityState::Repairing,
                "repair in progress",
                Some("wait for repair".to_string()),
            ),
        ]);

        assert_eq!(readiness, ParallelModeReadinessState::Degraded);
    }

    #[test]
    fn readiness_derivation_marks_ready_when_all_capabilities_are_ready() {
        let readiness = ParallelModeReadinessState::derive_from_capabilities(&[
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GitRepository,
                ParallelModeCapabilityState::Ready,
                "ready",
                None,
            ),
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::Planning,
                ParallelModeCapabilityState::Ready,
                "ready",
                None,
            ),
        ]);

        assert_eq!(readiness, ParallelModeReadinessState::Ready);
    }

    #[test]
    fn supervisor_state_recovers_when_enabled_readiness_blocks_parallel_mode() {
        let readiness = ParallelModeReadinessSnapshot::new(
            "/repo",
            ParallelModeReadinessState::Blocked,
            Vec::new(),
            None,
        );

        assert_eq!(
            ParallelModeSupervisorState::derive(true, Some(&readiness)),
            ParallelModeSupervisorState::Recover
        );
        assert_eq!(
            ParallelModeSupervisorState::derive(false, Some(&readiness)),
            ParallelModeSupervisorState::Prepare
        );
    }

    #[test]
    fn roster_projection_sorts_active_leases_and_applies_runtime_detail_overrides() {
        let running = lease(
            "slot-1",
            "task-1",
            "Task One",
            "agent-1",
            ParallelModeSlotLeaseState::Running,
            "2026-01-01T00:00:00Z",
            Some("2026-01-01T00:05:00Z"),
        );
        let leased = lease(
            "slot-2",
            "task-2",
            "Task Two",
            "agent-2",
            ParallelModeSlotLeaseState::Leased,
            "2026-01-01T00:10:00Z",
            None,
        );
        let cleanup = lease(
            "slot-3",
            "task-3",
            "Task Three",
            "agent-3",
            ParallelModeSlotLeaseState::CleanupPending,
            "2026-01-01T00:20:00Z",
            Some("2026-01-01T00:25:00Z"),
        );
        let detail = session_detail(
            &running,
            "commit_ready",
            "official ledger refresh accepted the completion report",
        );
        let duration_labels = BTreeMap::from([(running.session_key(), "7m".to_string())]);

        let roster = super::ParallelModeAgentRosterSnapshot::project_from_leases(
            vec![cleanup, leased, running],
            &[detail],
            true,
            &duration_labels,
        );

        assert_eq!(roster.active_count(), 3);
        assert_eq!(
            roster.empty_state,
            "no agent sessions launched in this slice"
        );
        assert_eq!(roster.entries[0].slot_id, "slot-1");
        assert_eq!(roster.entries[0].state_label, "commit_ready");
        assert_eq!(roster.entries[0].duration_label, "official");
        assert_eq!(
            roster.entries[0].latest_summary,
            "official ledger refresh accepted the completion report"
        );
        assert_eq!(roster.entries[1].slot_id, "slot-2");
        assert_eq!(roster.entries[1].state_label, "starting");
        assert_eq!(roster.entries[1].duration_label, "launch pending");
        assert_eq!(roster.entries[2].slot_id, "slot-3");
        assert_eq!(roster.entries[2].state_label, "cleanup_pending");
        assert_eq!(roster.entries[2].duration_label, "complete");
    }

    #[test]
    fn live_detail_enrichment_fills_missing_runtime_fields_from_lease() {
        let cleanup = lease(
            "slot-3",
            "task-3",
            "Task Three",
            "agent-3",
            ParallelModeSlotLeaseState::CleanupPending,
            "2026-01-01T00:20:00Z",
            Some("2026-01-01T00:25:00Z"),
        );
        let mut detail = session_detail(&cleanup, "running", "");
        detail.validation_summary.clear();
        detail.authority_refresh_outcome.clear();
        detail.updated_at.clear();

        let enriched = ParallelModeAgentSessionDetailSnapshot::live_for_lease(
            &cleanup,
            Some(detail),
            live_defaults(),
        );

        assert_eq!(enriched.session_key, cleanup.session_key());
        assert_eq!(enriched.state_label, "cleanup_pending");
        assert_eq!(enriched.completion_state_label, "merged");
        assert_eq!(
            enriched.latest_summary,
            "agent session reported completion and slot cleanup is pending"
        );
        assert_eq!(enriched.validation_summary, "validation unavailable");
        assert_eq!(enriched.authority_refresh_outcome, "authority unavailable");
        assert_eq!(
            enriched.distributor_outcome.as_deref(),
            Some("branch is merged into prerelease and the slot is awaiting cleanup")
        );
        assert_eq!(enriched.updated_at, "2026-01-01T00:25:00Z");
    }

    #[test]
    fn runtime_detail_selection_prefers_active_queue_head_then_active_lease_then_history() {
        let running = lease(
            "slot-1",
            "task-1",
            "Task One",
            "agent-1",
            ParallelModeSlotLeaseState::Running,
            "2026-01-01T00:00:00Z",
            Some("2026-01-01T00:05:00Z"),
        );
        let leased = lease(
            "slot-2",
            "task-2",
            "Task Two",
            "agent-2",
            ParallelModeSlotLeaseState::Leased,
            "2026-01-01T00:10:00Z",
            None,
        );
        let history = vec![session_detail(
            &running,
            "running",
            "agent session entered the running state",
        )];

        let queue_selected = ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
            &[running.clone(), leased.clone()],
            &history,
            Some(leased.session_key().as_str()),
            live_defaults(),
        )
        .expect("active queue lease should produce live detail");
        assert_eq!(queue_selected.slot_id, "slot-2");
        assert_eq!(queue_selected.state_label, "assigned");

        let lease_selected = ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
            &[leased, running],
            &history,
            None,
            live_defaults(),
        )
        .expect("active lease should produce live detail");
        assert_eq!(lease_selected.slot_id, "slot-1");
        assert_eq!(lease_selected.state_label, "running");

        let history_selected = ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
            &[],
            &history,
            None,
            live_defaults(),
        )
        .expect("history fallback should be selected");
        assert_eq!(history_selected.slot_id, "slot-1");
    }

    #[test]
    fn pool_slot_cleanup_decision_respects_lease_state_and_branch_integration() {
        assert!(
            ParallelModePoolSlotCleanupDecision::new(
                Some(ParallelModeSlotLeaseState::CleanupPending),
                false,
                true
            )
            .is_cleanup_ready()
        );
        assert!(
            !ParallelModePoolSlotCleanupDecision::new(
                Some(ParallelModeSlotLeaseState::Running),
                true,
                true
            )
            .is_cleanup_ready()
        );
        assert!(ParallelModePoolSlotCleanupDecision::new(None, true, true).is_cleanup_ready());
        assert!(!ParallelModePoolSlotCleanupDecision::new(None, false, true).is_cleanup_ready());
    }

    #[test]
    fn pool_slot_snapshot_projects_lease_state_to_pool_slot_state() {
        let lease = lease(
            "slot-1",
            "task-1",
            "Task One",
            "agent-1",
            ParallelModeSlotLeaseState::CleanupPending,
            "2026-01-01T00:00:00Z",
            Some("2026-01-01T00:05:00Z"),
        );

        let slot = super::ParallelModePoolSlotSnapshot::from_lease(
            "slot-1",
            lease.branch_name.as_str(),
            "slot-1 / clean",
            &lease,
        );

        assert_eq!(slot.state, ParallelModePoolSlotState::AwaitingCleanup);
        assert_eq!(slot.owner_label, "agent-1 / task-1");
    }

    fn lease(
        slot_id: &str,
        task_id: &str,
        task_title: &str,
        agent_id: &str,
        state: ParallelModeSlotLeaseState,
        leased_at: &str,
        running_started_at: Option<&str>,
    ) -> ParallelModeSlotLeaseSnapshot {
        ParallelModeSlotLeaseSnapshot::new(
            slot_id,
            task_id,
            task_title,
            agent_id,
            format!("akra-agent/{slot_id}/{task_id}"),
            format!("/repo/.akra-worktrees/{slot_id}"),
            state,
            leased_at,
            running_started_at.map(str::to_string),
        )
    }

    fn session_detail(
        lease: &ParallelModeSlotLeaseSnapshot,
        state_label: &str,
        latest_summary: &str,
    ) -> ParallelModeAgentSessionDetailSnapshot {
        ParallelModeAgentSessionDetailSnapshot::new(
            lease.session_key(),
            lease.agent_id.clone(),
            lease.task_id.clone(),
            lease.task_title.clone(),
            lease.slot_id.clone(),
            Some("thread-1".to_string()),
            lease.worktree_path.clone(),
            lease.branch_name.clone(),
            lease.leased_at.clone(),
            state_label,
            state_label,
            latest_summary,
            "cargo test passed",
            "authority refreshed",
            None,
            Vec::new(),
            "2026-01-01T00:30:00Z",
        )
    }

    fn live_defaults() -> ParallelModeLiveSessionDetailDefaults<'static> {
        ParallelModeLiveSessionDetailDefaults {
            validation_summary: "validation unavailable",
            authority_refresh_outcome: "authority unavailable",
        }
    }
}

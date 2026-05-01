use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};

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
    pub authority_refresh_outcome: String,
    pub distributor_outcome: Option<String>,
    pub history: Vec<ParallelModeAgentSessionHistoryEntry>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelModeLiveSessionDetailDefaults<'a> {
    pub validation_summary: &'a str,
    pub authority_refresh_outcome: &'a str,
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
        authority_refresh_outcome: impl Into<String>,
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
            authority_refresh_outcome: authority_refresh_outcome.into(),
            distributor_outcome,
            history,
            updated_at: updated_at.into(),
        }
    }

    pub fn assigned_for_lease(
        lease: &ParallelModeSlotLeaseSnapshot,
        defaults: ParallelModeLiveSessionDetailDefaults<'_>,
    ) -> Self {
        Self::new(
            lease.session_key(),
            lease.agent_id.clone(),
            lease.task_id.clone(),
            lease.task_title.clone(),
            lease.slot_id.clone(),
            None,
            lease.worktree_path.clone(),
            lease.branch_name.clone(),
            lease.leased_at.clone(),
            "assigned",
            "in_progress",
            "slot lease acquired and branch reserved for launch",
            defaults.validation_summary,
            defaults.authority_refresh_outcome,
            None,
            vec![ParallelModeAgentSessionHistoryEntry::new(
                "assigned",
                lease.leased_at.clone(),
                "slot lease acquired and branch reserved for launch",
            )],
            lease.leased_at.clone(),
        )
    }

    pub fn live_for_lease(
        lease: &ParallelModeSlotLeaseSnapshot,
        detail: Option<Self>,
        defaults: ParallelModeLiveSessionDetailDefaults<'_>,
    ) -> Self {
        let mut detail = detail.unwrap_or_else(|| Self::assigned_for_lease(lease, defaults));
        detail.session_key = lease.session_key();
        detail.agent_id = lease.agent_id.clone();
        detail.task_id = lease.task_id.clone();
        detail.task_title = lease.task_title.clone();
        detail.slot_id = lease.slot_id.clone();
        detail.worktree_path = lease.worktree_path.clone();
        detail.branch_name = lease.branch_name.clone();
        detail.lease_started_at = lease.leased_at.clone();
        detail.state_label = live_detail_state_label(lease, &detail);
        detail.completion_state_label = live_completion_state_label(lease, &detail);
        if detail.latest_summary.trim().is_empty() {
            detail.latest_summary = roster_latest_summary(lease, Some(&detail));
        }
        if detail.validation_summary.trim().is_empty() {
            detail.validation_summary = defaults.validation_summary.to_string();
        }
        if detail.authority_refresh_outcome.trim().is_empty() {
            detail.authority_refresh_outcome = defaults.authority_refresh_outcome.to_string();
        }
        if detail.distributor_outcome.is_none() {
            detail.distributor_outcome = live_distributor_outcome(lease);
        }
        if detail.updated_at.trim().is_empty() {
            detail.updated_at = live_detail_updated_at(lease).to_string();
        }
        detail
    }

    pub fn select_runtime_detail(
        leases: &[ParallelModeSlotLeaseSnapshot],
        history: &[ParallelModeAgentSessionDetailSnapshot],
        active_queue_session_key: Option<&str>,
        defaults: ParallelModeLiveSessionDetailDefaults<'_>,
    ) -> Option<Self> {
        if let Some(session_key) = active_queue_session_key
            && let Some(detail) =
                Self::detail_for_runtime_session(leases, history, session_key, defaults)
        {
            return Some(detail);
        }

        if let Some(lease) = leases
            .iter()
            .max_by(|left, right| compare_lease_selection(left, right))
        {
            return Some(Self::live_for_lease(
                lease,
                detail_for_lease(history, lease),
                defaults,
            ));
        }

        history.first().cloned()
    }

    fn detail_for_runtime_session(
        leases: &[ParallelModeSlotLeaseSnapshot],
        history: &[ParallelModeAgentSessionDetailSnapshot],
        session_key: &str,
        defaults: ParallelModeLiveSessionDetailDefaults<'_>,
    ) -> Option<Self> {
        let detail = history
            .iter()
            .find(|detail| detail.session_key == session_key)
            .cloned();
        if let Some(lease) = leases
            .iter()
            .find(|lease| lease.session_key() == session_key)
        {
            return Some(Self::live_for_lease(lease, detail, defaults));
        }

        detail
    }
}

fn live_detail_state_label(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: &ParallelModeAgentSessionDetailSnapshot,
) -> String {
    if let Some(label) = lease.runtime_state_override(detail) {
        return label.to_string();
    }

    match lease.state {
        ParallelModeSlotLeaseState::Leased => {
            if detail.thread_id.is_some() || detail.state_label == "starting" {
                "starting".to_string()
            } else {
                "assigned".to_string()
            }
        }
        ParallelModeSlotLeaseState::Running => "running".to_string(),
        ParallelModeSlotLeaseState::CleanupPending => "cleanup_pending".to_string(),
    }
}

fn live_completion_state_label(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: &ParallelModeAgentSessionDetailSnapshot,
) -> String {
    if lease.runtime_state_override(detail).is_some() {
        return detail.completion_state_label.clone();
    }

    match lease.state {
        ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running => {
            "in_progress".to_string()
        }
        ParallelModeSlotLeaseState::CleanupPending => "merged".to_string(),
    }
}

fn live_distributor_outcome(lease: &ParallelModeSlotLeaseSnapshot) -> Option<String> {
    match lease.state {
        ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running => None,
        ParallelModeSlotLeaseState::CleanupPending => {
            Some("branch is merged into prerelease and the slot is awaiting cleanup".to_string())
        }
    }
}

fn live_detail_updated_at(lease: &ParallelModeSlotLeaseSnapshot) -> &str {
    lease
        .running_started_at
        .as_deref()
        .unwrap_or(lease.leased_at.as_str())
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

    pub fn project_from_leases(
        leases: Vec<ParallelModeSlotLeaseSnapshot>,
        details: &[ParallelModeAgentSessionDetailSnapshot],
        mode_enabled: bool,
        running_duration_labels: &BTreeMap<String, String>,
    ) -> Self {
        let active_leases = sorted_active_leases(leases);

        let entries = active_leases
            .iter()
            .map(|lease| {
                let detail = details
                    .iter()
                    .find(|detail| detail.session_key == lease.session_key());
                project_agent_roster_entry(lease, detail, running_duration_labels)
            })
            .collect::<Vec<_>>();
        let empty_state = if mode_enabled {
            "no agent sessions launched in this slice"
        } else {
            "parallel mode is off / agent roster is read-only"
        };

        Self::new(entries, empty_state)
    }
}

fn sorted_active_leases(
    mut active_leases: Vec<ParallelModeSlotLeaseSnapshot>,
) -> Vec<ParallelModeSlotLeaseSnapshot> {
    active_leases.sort_by(|left, right| compare_lease_selection(right, left));
    active_leases
}

fn compare_lease_selection(
    left: &ParallelModeSlotLeaseSnapshot,
    right: &ParallelModeSlotLeaseSnapshot,
) -> std::cmp::Ordering {
    left.selection_priority()
        .cmp(&right.selection_priority())
        .then_with(|| right.slot_id.cmp(&left.slot_id))
}

fn detail_for_lease(
    history: &[ParallelModeAgentSessionDetailSnapshot],
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Option<ParallelModeAgentSessionDetailSnapshot> {
    history
        .iter()
        .find(|detail| detail.session_key == lease.session_key())
        .cloned()
}

fn project_agent_roster_entry(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
    running_duration_labels: &BTreeMap<String, String>,
) -> ParallelModeAgentRosterEntry {
    ParallelModeAgentRosterEntry::new(
        lease.agent_id.clone(),
        lease.task_title.clone(),
        lease.slot_id.clone(),
        lease.branch_name.clone(),
        roster_state_label(lease, detail),
        roster_duration_label(lease, detail, running_duration_labels),
        roster_latest_summary(lease, detail),
    )
}

pub(super) fn roster_state_priority(state: ParallelModeSlotLeaseState) -> u8 {
    match state {
        ParallelModeSlotLeaseState::Running => 3,
        ParallelModeSlotLeaseState::Leased => 2,
        ParallelModeSlotLeaseState::CleanupPending => 1,
    }
}

pub(super) fn roster_recency_key(lease: &ParallelModeSlotLeaseSnapshot) -> &str {
    lease
        .running_started_at
        .as_deref()
        .unwrap_or(lease.leased_at.as_str())
}

pub fn roster_state_label(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
) -> String {
    if let Some(detail) = detail
        && let Some(label) = lease.runtime_state_override(detail)
    {
        return label.to_string();
    }

    match lease.state {
        ParallelModeSlotLeaseState::Leased => "starting".to_string(),
        ParallelModeSlotLeaseState::Running => "running".to_string(),
        ParallelModeSlotLeaseState::CleanupPending => "cleanup_pending".to_string(),
    }
}

fn roster_duration_label(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
    running_duration_labels: &BTreeMap<String, String>,
) -> String {
    if let Some(detail) = detail {
        match detail.state_label.as_str() {
            "reported_complete" => return "reported".to_string(),
            "ledger_refreshing" => return "refreshing".to_string(),
            "commit_ready" => return "official".to_string(),
            "merge_queued" => return "queued".to_string(),
            "pushing" => return "pushing".to_string(),
            "pr_pending" => return "pr pending".to_string(),
            "merge_pending" => return "merge pending".to_string(),
            "integrating" => return "integrating".to_string(),
            "failed" => return "blocked".to_string(),
            _ => {}
        }
    }

    match lease.state {
        ParallelModeSlotLeaseState::Leased => "launch pending".to_string(),
        ParallelModeSlotLeaseState::Running => running_duration_labels
            .get(&lease.session_key())
            .cloned()
            .unwrap_or_else(|| "active".to_string()),
        ParallelModeSlotLeaseState::CleanupPending => "complete".to_string(),
    }
}

pub fn roster_latest_summary(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
) -> String {
    detail
        .map(|detail| detail.latest_summary.trim())
        .filter(|summary| !summary.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match lease.state {
            ParallelModeSlotLeaseState::Leased => {
                "branch reserved and agent bootstrap in progress".to_string()
            }
            ParallelModeSlotLeaseState::Running => {
                "agent session is active in the leased slot".to_string()
            }
            ParallelModeSlotLeaseState::CleanupPending => {
                "agent session reported completion and slot cleanup is pending".to_string()
            }
        })
}

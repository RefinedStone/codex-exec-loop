use serde::{Deserialize, Serialize};

use super::{
    ParallelModeAgentRosterSnapshot, ParallelModePoolBoardSnapshot,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorState,
};

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
            Self::PrPending => "pr pending",
            Self::MergePending => "merge pending",
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
pub struct ParallelModeOrchestratorStatus {
    pub queue_head: String,
    pub barrier_state: String,
    pub blocked_reason: Option<String>,
    pub conflict_files: Vec<String>,
    pub held_queue_count: usize,
    pub integration_worktree_readiness: String,
    pub slot_return_wait_reason: Option<String>,
}

impl ParallelModeOrchestratorStatus {
    pub fn idle() -> Self {
        Self {
            queue_head: "none".to_string(),
            barrier_state: "idle".to_string(),
            blocked_reason: None,
            conflict_files: Vec::new(),
            held_queue_count: 0,
            integration_worktree_readiness: "not inspected".to_string(),
            slot_return_wait_reason: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeDistributorSnapshot {
    pub queue_items: Vec<ParallelModeDistributorQueueItem>,
    pub completion_feed: Vec<ParallelModeCompletionFeedEntry>,
    pub head_summary: String,
    pub note: String,
    pub head_blocked_detail: Option<String>,
    pub head_rebase_provenance: Option<String>,
    pub orchestrator_status: ParallelModeOrchestratorStatus,
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
            head_blocked_detail: None,
            head_rebase_provenance: None,
            orchestrator_status: ParallelModeOrchestratorStatus::idle(),
        }
    }

    pub fn with_head_blocked_detail(mut self, detail: Option<String>) -> Self {
        self.head_blocked_detail = detail.filter(|detail| !detail.trim().is_empty());
        self
    }

    pub fn with_head_rebase_provenance(mut self, provenance: Option<String>) -> Self {
        self.head_rebase_provenance = provenance.filter(|provenance| !provenance.trim().is_empty());
        self
    }

    pub fn with_orchestrator_status(mut self, status: ParallelModeOrchestratorStatus) -> Self {
        self.orchestrator_status = status;
        self
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

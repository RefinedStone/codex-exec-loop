use crate::domain::planning::PostTurnExecution;
use crate::domain::recent_sessions::SessionRenameRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupCheckCorrelation {
    pub generation: u64,
    pub workspace_directory: String,
}

impl StartupCheckCorrelation {
    pub fn new(generation: u64, workspace_directory: impl Into<String>) -> Self {
        Self {
            generation,
            workspace_directory: workspace_directory.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCatalogLoadMode {
    EnsureLoaded,
    Refresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCatalogLoadIntent {
    pub mode: SessionCatalogLoadMode,
    pub limit: usize,
    pub workspace_directory: String,
}

impl SessionCatalogLoadIntent {
    pub fn ensure_loaded(limit: usize, workspace_directory: impl Into<String>) -> Self {
        Self {
            mode: SessionCatalogLoadMode::EnsureLoaded,
            limit,
            workspace_directory: workspace_directory.into(),
        }
    }

    pub fn refresh(limit: usize, workspace_directory: impl Into<String>) -> Self {
        Self {
            mode: SessionCatalogLoadMode::Refresh,
            limit,
            workspace_directory: workspace_directory.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCatalogLoadCorrelation {
    pub generation: u64,
    pub limit: usize,
    pub workspace_directory: String,
}

impl SessionCatalogLoadCorrelation {
    pub fn new(generation: u64, limit: usize, workspace_directory: impl Into<String>) -> Self {
        Self {
            generation,
            limit,
            workspace_directory: workspace_directory.into(),
        }
    }

    pub fn matches_target(&self, intent: &SessionCatalogLoadIntent) -> bool {
        self.limit == intent.limit && self.workspace_directory == intent.workspace_directory
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRenameCorrelation {
    pub generation: u64,
    pub request: SessionRenameRequest,
}

impl SessionRenameCorrelation {
    pub fn new(generation: u64, request: SessionRenameRequest) -> Self {
        Self {
            generation,
            request,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRenameAdmission {
    Accepted {
        correlation: SessionRenameCorrelation,
    },
    RejectedActive {
        active_correlation: SessionRenameCorrelation,
    },
    RejectedCatalogLoading {
        active_correlation: SessionCatalogLoadCorrelation,
    },
    RejectedConversationLoading {
        active_correlation: ConversationLoadCorrelation,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationLoadCorrelation {
    pub generation: u64,
    pub requested_thread_id: String,
}

impl ConversationLoadCorrelation {
    pub fn new(generation: u64, requested_thread_id: impl Into<String>) -> Self {
        Self {
            generation,
            requested_thread_id: requested_thread_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelPeekLoadCorrelation {
    pub generation: u64,
    pub requested_thread_id: String,
}

impl ParallelPeekLoadCorrelation {
    pub fn new(generation: u64, requested_thread_id: impl Into<String>) -> Self {
        Self {
            generation,
            requested_thread_id: requested_thread_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCenterLoadCorrelation {
    pub generation: u64,
    pub workspace_directory: String,
    pub active_thread_id: Option<String>,
}

impl ReviewCenterLoadCorrelation {
    pub fn new(
        generation: u64,
        workspace_directory: impl Into<String>,
        active_thread_id: Option<String>,
    ) -> Self {
        Self {
            generation,
            workspace_directory: workspace_directory.into(),
            active_thread_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAuthorityLoadCorrelation {
    pub generation: u64,
    pub workspace_directory: String,
    pub active_thread_id: Option<String>,
}

impl QueueAuthorityLoadCorrelation {
    pub fn new(
        generation: u64,
        workspace_directory: impl Into<String>,
        active_thread_id: Option<String>,
    ) -> Self {
        Self {
            generation,
            workspace_directory: workspace_directory.into(),
            active_thread_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionsMaintenanceLoadCorrelation {
    pub generation: u64,
    pub workspace_directory: String,
}

impl DirectionsMaintenanceLoadCorrelation {
    pub fn new(generation: u64, workspace_directory: impl Into<String>) -> Self {
        Self {
            generation,
            workspace_directory: workspace_directory.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeRefreshCorrelation {
    pub generation: u64,
    pub workspace_directory: String,
}

impl PlanningRuntimeRefreshCorrelation {
    pub fn new(generation: u64, workspace_directory: impl Into<String>) -> Self {
        Self {
            generation,
            workspace_directory: workspace_directory.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnEvaluationCorrelation {
    pub generation: u64,
    pub thread_id: String,
    pub completed_turn_id: String,
    pub turn_workspace_directory: String,
    pub planning_workspace_directory: String,
}

impl PostTurnEvaluationCorrelation {
    pub fn new(
        generation: u64,
        thread_id: impl Into<String>,
        completed_turn_id: impl Into<String>,
        turn_workspace_directory: impl Into<String>,
        planning_workspace_directory: impl Into<String>,
    ) -> Self {
        Self {
            generation,
            thread_id: thread_id.into(),
            completed_turn_id: completed_turn_id.into(),
            turn_workspace_directory: turn_workspace_directory.into(),
            planning_workspace_directory: planning_workspace_directory.into(),
        }
    }

    pub fn matches_execution(&self, execution: &PostTurnExecution) -> bool {
        execution.thread_id == self.thread_id
            && execution.completed_turn_id == self.completed_turn_id
            && execution.evaluation.provenance.completed_turn_id == self.completed_turn_id
            && execution
                .evaluation
                .provenance
                .queue_mutation_receipt
                .as_ref()
                .is_none_or(|receipt| receipt.completed_turn_id == self.completed_turn_id)
            && (execution.runtime_projection_workspace_directory == self.turn_workspace_directory
                || execution.runtime_projection_workspace_directory
                    == self.planning_workspace_directory)
    }
}

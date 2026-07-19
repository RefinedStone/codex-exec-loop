use crate::domain::recent_sessions::SessionRenameRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupCheckCorrelation {
    pub generation: u64,
}

impl StartupCheckCorrelation {
    pub fn new(generation: u64) -> Self {
        Self { generation }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCatalogLoadCorrelation {
    pub generation: u64,
}

impl SessionCatalogLoadCorrelation {
    pub fn new(generation: u64) -> Self {
        Self { generation }
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

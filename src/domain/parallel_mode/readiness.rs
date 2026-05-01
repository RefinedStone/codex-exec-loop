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

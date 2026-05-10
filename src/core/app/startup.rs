#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupReadySnapshot {
    pub workspace_path: String,
    pub can_continue: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupSnapshot {
    Idle,
    Loading,
    Ready(StartupReadySnapshot),
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupState {
    Idle,
    Loading,
    Ready(StartupReadySnapshot),
    Failed(String),
}

impl StartupState {
    pub fn snapshot(&self) -> StartupSnapshot {
        match self {
            Self::Idle => StartupSnapshot::Idle,
            Self::Loading => StartupSnapshot::Loading,
            Self::Ready(ready) => StartupSnapshot::Ready(ready.clone()),
            Self::Failed(message) => StartupSnapshot::Failed {
                message: message.clone(),
            },
        }
    }
}

impl Default for StartupState {
    fn default() -> Self {
        Self::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_state_projects_message_snapshot() {
        assert_eq!(
            StartupState::Failed("missing codex".to_string()).snapshot(),
            StartupSnapshot::Failed {
                message: "missing codex".to_string(),
            }
        );
    }
}

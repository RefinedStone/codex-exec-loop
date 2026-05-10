use crate::domain::startup_diagnostics::StartupDiagnostics;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupReadySnapshot {
    pub workspace_path: String,
    pub can_continue: bool,
    pub warnings: Vec<String>,
}

impl StartupReadySnapshot {
    pub(crate) fn from_diagnostics(diagnostics: StartupDiagnostics) -> Self {
        let can_continue = diagnostics.can_continue();
        Self {
            workspace_path: diagnostics.workspace_path,
            can_continue,
            warnings: diagnostics.warnings,
        }
    }
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
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    #[test]
    fn failed_state_projects_message_snapshot() {
        assert_eq!(
            StartupState::Failed("missing codex".to_string()).snapshot(),
            StartupSnapshot::Failed {
                message: "missing codex".to_string(),
            }
        );
    }

    #[test]
    fn ready_snapshot_keeps_core_startup_projection_only() {
        let diagnostics = StartupDiagnostics {
            cwd: "/tmp/workspace".to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "/usr/bin/codex".to_string(),
            workspace_ok: true,
            workspace_path: "/tmp/workspace".to_string(),
            workspace_detail: "git repo: /tmp/workspace".to_string(),
            attachment_profile: TerminalBridgeAttachmentProfile::default(),
            initialize_ok: true,
            initialize_detail: "initialized".to_string(),
            account_ok: true,
            account_detail: "authenticated".to_string(),
            warnings: vec!["schema warning".to_string()],
            schema_snapshot: "embedded schema".to_string(),
        };

        assert_eq!(
            StartupReadySnapshot::from_diagnostics(diagnostics),
            StartupReadySnapshot {
                workspace_path: "/tmp/workspace".to_string(),
                can_continue: true,
                warnings: vec!["schema warning".to_string()],
            }
        );
    }
}

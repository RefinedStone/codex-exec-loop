use crate::domain::startup_diagnostics::StartupDiagnostics;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupDiagnosticSnapshot {
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupAttachmentSnapshot {
    pub mode_label: String,
    pub recovery_anchor_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupReadySnapshot {
    pub cwd: String,
    pub workspace_path: String,
    pub can_continue: bool,
    pub codex_binary: StartupDiagnosticSnapshot,
    pub workspace: StartupDiagnosticSnapshot,
    pub app_server_initialize: StartupDiagnosticSnapshot,
    pub account: StartupDiagnosticSnapshot,
    pub attachment: StartupAttachmentSnapshot,
    pub warnings: Vec<String>,
    pub schema_snapshot: String,
}

impl StartupReadySnapshot {
    pub(crate) fn from_diagnostics(diagnostics: StartupDiagnostics) -> Self {
        let can_continue = diagnostics.can_continue();
        let attachment_profile = diagnostics.attachment_profile;
        Self {
            cwd: diagnostics.cwd,
            workspace_path: diagnostics.workspace_path,
            can_continue,
            codex_binary: StartupDiagnosticSnapshot {
                ok: diagnostics.codex_binary_ok,
                detail: diagnostics.codex_binary_detail,
            },
            workspace: StartupDiagnosticSnapshot {
                ok: diagnostics.workspace_ok,
                detail: diagnostics.workspace_detail,
            },
            app_server_initialize: StartupDiagnosticSnapshot {
                ok: diagnostics.initialize_ok,
                detail: diagnostics.initialize_detail,
            },
            account: StartupDiagnosticSnapshot {
                ok: diagnostics.account_ok,
                detail: diagnostics.account_detail,
            },
            attachment: StartupAttachmentSnapshot {
                mode_label: attachment_profile.mode.label().to_string(),
                recovery_anchor_label: attachment_profile.recovery_anchor.label().to_string(),
            },
            warnings: diagnostics.warnings,
            schema_snapshot: diagnostics.schema_snapshot,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupSnapshot {
    Idle,
    Loading,
    Ready(Box<StartupReadySnapshot>),
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StartupState {
    #[default]
    Idle,
    Loading,
    Ready(Box<StartupReadySnapshot>),
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
                cwd: "/tmp/workspace".to_string(),
                workspace_path: "/tmp/workspace".to_string(),
                can_continue: true,
                codex_binary: StartupDiagnosticSnapshot {
                    ok: true,
                    detail: "/usr/bin/codex".to_string(),
                },
                workspace: StartupDiagnosticSnapshot {
                    ok: true,
                    detail: "git repo: /tmp/workspace".to_string(),
                },
                app_server_initialize: StartupDiagnosticSnapshot {
                    ok: true,
                    detail: "initialized".to_string(),
                },
                account: StartupDiagnosticSnapshot {
                    ok: true,
                    detail: "authenticated".to_string(),
                },
                attachment: StartupAttachmentSnapshot {
                    mode_label: "provider-launched".to_string(),
                    recovery_anchor_label: "provider-thread-id".to_string(),
                },
                warnings: vec!["schema warning".to_string()],
                schema_snapshot: "embedded schema".to_string(),
            }
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalBridgeAttachmentMode {
    ProviderLaunch,
    ProviderReattach,
    LocalAttach,
    ManagedWrapper,
    RemoteAttach,
    ProxyMediated,
}

impl TerminalBridgeAttachmentMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProviderLaunch => "provider-launched",
            Self::ProviderReattach => "provider-reattach",
            Self::LocalAttach => "local-attach",
            Self::ManagedWrapper => "managed-wrapper",
            Self::RemoteAttach => "remote-attach",
            Self::ProxyMediated => "proxy-mediated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalBridgeRecoveryAnchor {
    None,
    ProviderThreadId,
    SessionHandle,
    TerminalSession,
}

impl TerminalBridgeRecoveryAnchor {
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ProviderThreadId => "provider-thread-id",
            Self::SessionHandle => "session-handle",
            Self::TerminalSession => "terminal-session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalBridgeAttachmentProfile {
    pub mode: TerminalBridgeAttachmentMode,
    pub recovery_anchor: TerminalBridgeRecoveryAnchor,
}

impl TerminalBridgeAttachmentProfile {
    pub const fn new(
        mode: TerminalBridgeAttachmentMode,
        recovery_anchor: TerminalBridgeRecoveryAnchor,
    ) -> Self {
        Self {
            mode,
            recovery_anchor,
        }
    }

    pub const fn codex_app_server_launch() -> Self {
        Self::new(
            TerminalBridgeAttachmentMode::ProviderLaunch,
            TerminalBridgeRecoveryAnchor::ProviderThreadId,
        )
    }

    pub const fn codex_app_server_reattach() -> Self {
        Self::new(
            TerminalBridgeAttachmentMode::ProviderReattach,
            TerminalBridgeRecoveryAnchor::ProviderThreadId,
        )
    }

    pub const fn codex_app_server() -> Self {
        Self::codex_app_server_launch()
    }
}

impl Default for TerminalBridgeAttachmentProfile {
    fn default() -> Self {
        Self::codex_app_server()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TerminalBridgeAttachmentMode, TerminalBridgeAttachmentProfile, TerminalBridgeRecoveryAnchor,
    };

    #[test]
    fn codex_launch_profile_reports_provider_launch_and_thread_id_recovery() {
        assert_eq!(
            TerminalBridgeAttachmentProfile::codex_app_server_launch(),
            TerminalBridgeAttachmentProfile::new(
                TerminalBridgeAttachmentMode::ProviderLaunch,
                TerminalBridgeRecoveryAnchor::ProviderThreadId,
            )
        );
    }

    #[test]
    fn codex_reattach_profile_reports_provider_reattach_and_thread_id_recovery() {
        assert_eq!(
            TerminalBridgeAttachmentProfile::codex_app_server_reattach(),
            TerminalBridgeAttachmentProfile::new(
                TerminalBridgeAttachmentMode::ProviderReattach,
                TerminalBridgeRecoveryAnchor::ProviderThreadId,
            )
        );
    }

    #[test]
    fn attachment_labels_stay_kebab_case() {
        assert_eq!(
            TerminalBridgeAttachmentMode::ProviderLaunch.label(),
            "provider-launched"
        );
        assert_eq!(
            TerminalBridgeAttachmentMode::ProviderReattach.label(),
            "provider-reattach"
        );
        assert_eq!(
            TerminalBridgeAttachmentMode::LocalAttach.label(),
            "local-attach"
        );
        assert_eq!(
            TerminalBridgeAttachmentMode::ManagedWrapper.label(),
            "managed-wrapper"
        );
        assert_eq!(
            TerminalBridgeAttachmentMode::RemoteAttach.label(),
            "remote-attach"
        );
        assert_eq!(
            TerminalBridgeAttachmentMode::ProxyMediated.label(),
            "proxy-mediated"
        );
        assert_eq!(
            TerminalBridgeRecoveryAnchor::ProviderThreadId.label(),
            "provider-thread-id"
        );
        assert_eq!(
            TerminalBridgeRecoveryAnchor::SessionHandle.label(),
            "session-handle"
        );
        assert_eq!(
            TerminalBridgeRecoveryAnchor::TerminalSession.label(),
            "terminal-session"
        );
    }
}

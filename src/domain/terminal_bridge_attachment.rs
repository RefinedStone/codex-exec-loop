#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalBridgeAttachmentMode {
    ProviderLaunch,
    LocalAttach,
    ManagedWrapper,
    RemoteAttach,
    ProxyMediated,
}

impl TerminalBridgeAttachmentMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProviderLaunch => "provider-launched",
            Self::LocalAttach => "local attach",
            Self::ManagedWrapper => "managed wrapper",
            Self::RemoteAttach => "remote attach",
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
            Self::ProviderThreadId => "provider thread id",
            Self::SessionHandle => "session handle",
            Self::TerminalSession => "terminal session",
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

    pub const fn codex_app_server() -> Self {
        Self::new(
            TerminalBridgeAttachmentMode::ProviderLaunch,
            TerminalBridgeRecoveryAnchor::ProviderThreadId,
        )
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
    fn codex_profile_reports_provider_launch_and_thread_id_recovery() {
        assert_eq!(
            TerminalBridgeAttachmentProfile::codex_app_server(),
            TerminalBridgeAttachmentProfile::new(
                TerminalBridgeAttachmentMode::ProviderLaunch,
                TerminalBridgeRecoveryAnchor::ProviderThreadId,
            )
        );
    }
}

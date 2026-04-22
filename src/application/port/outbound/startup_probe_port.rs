use anyhow::Result;

use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

#[derive(Debug, Clone)]
pub struct AppServerStartupContext {
    pub attachment_profile: TerminalBridgeAttachmentProfile,
    pub initialize_detail: String,
    pub account_detail: String,
    pub account_ok: bool,
    pub warnings: Vec<String>,
}

pub trait StartupProbePort: Send + Sync {
    fn load_startup_context(&self) -> Result<AppServerStartupContext>;
}

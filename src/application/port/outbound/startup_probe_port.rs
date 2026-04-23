use anyhow::Result;

use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

#[derive(Debug, Clone)]
pub struct StartupProbeContext {
    pub launch_target_ok: bool,
    pub launch_target_detail: String,
    pub readiness_ok: bool,
    pub attachment_profile: TerminalBridgeAttachmentProfile,
    pub readiness_detail: String,
    pub access_detail: String,
    pub access_ok: bool,
    pub schema_snapshot: String,
    pub warnings: Vec<String>,
}

pub type AppServerStartupContext = StartupProbeContext;

pub trait StartupProbePort: Send + Sync {
    fn load_startup_context(&self) -> Result<StartupProbeContext>;
}

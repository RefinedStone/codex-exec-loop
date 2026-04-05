#[derive(Debug, Clone)]
pub struct StartupDiagnostics {
    pub cwd: String,
    pub codex_binary_ok: bool,
    pub codex_binary_detail: String,
    pub workspace_ok: bool,
    pub workspace_detail: String,
    pub initialize_ok: bool,
    pub initialize_detail: String,
    pub account_ok: bool,
    pub account_detail: String,
    pub warnings: Vec<String>,
    pub schema_snapshot: String,
}

impl StartupDiagnostics {
    pub fn can_continue(&self) -> bool {
        self.codex_binary_ok && self.workspace_ok && self.initialize_ok && self.account_ok
    }
}

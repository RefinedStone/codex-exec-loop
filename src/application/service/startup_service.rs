use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::application::port::outbound::codex_app_server_port::CodexAppServerPort;
use crate::domain::startup_diagnostics::StartupDiagnostics;

#[derive(Clone)]
pub struct StartupService {
    codex_app_server_port: Arc<dyn CodexAppServerPort>,
}

impl StartupService {
    pub fn new(codex_app_server_port: Arc<dyn CodexAppServerPort>) -> Self {
        Self {
            codex_app_server_port,
        }
    }

    pub fn run_checks(&self) -> Result<StartupDiagnostics> {
        let current_directory = std::env::current_dir()
            .context("failed to resolve current directory")?
            .display()
            .to_string();

        let codex_path = which::which("codex").context("`codex` was not found on PATH")?;
        let workspace_status = self.detect_workspace_status()?;

        let startup_context = self.codex_app_server_port.load_startup_context()?;

        Ok(StartupDiagnostics {
            cwd: current_directory,
            codex_binary_ok: true,
            codex_binary_detail: codex_path.display().to_string(),
            workspace_ok: workspace_status.ok,
            workspace_path: workspace_status.path,
            workspace_detail: workspace_status.detail,
            initialize_ok: true,
            initialize_detail: startup_context.initialize_detail,
            account_ok: startup_context.account_ok,
            account_detail: startup_context.account_detail,
            warnings: startup_context.warnings,
            schema_snapshot: StartupDiagnostics::bundled_schema_snapshot_label(),
        })
    }

    fn detect_workspace_status(&self) -> Result<WorkspaceStatus> {
        let current_directory = std::env::current_dir()
            .context("failed to resolve current directory for workspace status")?
            .display()
            .to_string();

        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();

        match output {
            Ok(result) if result.status.success() => {
                let root = String::from_utf8_lossy(&result.stdout).trim().to_string();
                Ok(WorkspaceStatus {
                    ok: true,
                    path: root.clone(),
                    detail: format!("git repo: {root}"),
                })
            }
            _ => Ok(WorkspaceStatus {
                ok: true,
                path: current_directory,
                detail: "directory only (not inside a git repo)".to_string(),
            }),
        }
    }
}

struct WorkspaceStatus {
    ok: bool,
    path: String,
    detail: String,
}

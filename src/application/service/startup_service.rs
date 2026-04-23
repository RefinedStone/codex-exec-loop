use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::application::port::outbound::startup_probe_port::StartupProbePort;
use crate::domain::startup_diagnostics::StartupDiagnostics;

#[derive(Clone)]
pub struct StartupService {
    startup_probe_port: Arc<dyn StartupProbePort>,
}

impl StartupService {
    pub fn new(startup_probe_port: Arc<dyn StartupProbePort>) -> Self {
        Self { startup_probe_port }
    }

    pub fn run_checks(&self) -> Result<StartupDiagnostics> {
        let current_directory = std::env::current_dir()
            .context("failed to resolve current directory")?
            .display()
            .to_string();
        let workspace_status = self.detect_workspace_status()?;

        let startup_context = self.startup_probe_port.load_startup_context()?;

        Ok(StartupDiagnostics {
            cwd: current_directory,
            codex_binary_ok: startup_context.launch_target_ok,
            codex_binary_detail: startup_context.launch_target_detail,
            workspace_ok: workspace_status.ok,
            workspace_path: workspace_status.path,
            workspace_detail: workspace_status.detail,
            attachment_profile: startup_context.attachment_profile,
            initialize_ok: startup_context.readiness_ok,
            initialize_detail: startup_context.readiness_detail,
            account_ok: startup_context.access_ok,
            account_detail: startup_context.access_detail,
            warnings: startup_context.warnings,
            schema_snapshot: startup_context.schema_snapshot,
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

use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::domain::startup_diagnostics::StartupDiagnostics;
use crate::infrastructure::app_server_client::AppServerClient;

#[derive(Clone)]
pub struct StartupService {
    app_server_client: AppServerClient,
}

impl StartupService {
    pub fn new(app_server_client: AppServerClient) -> Self {
        Self { app_server_client }
    }

    pub fn run_checks(&self) -> Result<StartupDiagnostics> {
        let current_directory = std::env::current_dir()
            .context("failed to resolve current directory")?
            .display()
            .to_string();

        let codex_path = which::which("codex").context("`codex` was not found on PATH")?;
        let workspace_detail = self.detect_workspace_status();
        let workspace_ok =
            workspace_detail.starts_with("git repo") || workspace_detail.starts_with("directory");

        let mut connection = self.app_server_client.open_connection()?;
        let initialize_result = connection.initialize()?;
        let account_result = connection.read_account()?;
        let warnings = connection.finish();

        let initialize_detail = format!(
            "{} / {} / {}",
            initialize_result.platform_os,
            initialize_result.platform_family,
            initialize_result.user_agent,
        );

        let account_detail = account_result.to_summary_text();

        Ok(StartupDiagnostics {
            cwd: current_directory,
            codex_binary_ok: true,
            codex_binary_detail: codex_path.display().to_string(),
            workspace_ok,
            workspace_detail,
            initialize_ok: true,
            initialize_detail,
            account_ok: account_result.is_authenticated(),
            account_detail,
            warnings,
            schema_snapshot: "native/schema/codex_app_server_protocol.v2.schemas.json".to_string(),
        })
    }

    fn detect_workspace_status(&self) -> String {
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();

        match output {
            Ok(result) if result.status.success() => {
                let root = String::from_utf8_lossy(&result.stdout).trim().to_string();
                format!("git repo: {root}")
            }
            _ => "directory only (not inside a git repo)".to_string(),
        }
    }
}

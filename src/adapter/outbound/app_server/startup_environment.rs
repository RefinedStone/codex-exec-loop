use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};

use crate::application::port::outbound::startup_probe_port::{
    LocalStartupPrerequisites, StartupWorkspaceStatus,
};
use crate::{git_subprocess, subprocess};

pub(super) fn load_local_startup_prerequisites(
    workspace_directory: &str,
) -> Result<LocalStartupPrerequisites> {
    let current_directory = std::env::current_dir()
        .context("failed to resolve current directory")?
        .display()
        .to_string();
    let codex_command = crate::trusted_executable::pinned_codex_command()
        .context("failed to pin a trusted `codex` executable at startup")?;

    Ok(LocalStartupPrerequisites {
        current_directory,
        codex_binary_detail: codex_command.source_executable.display().to_string(),
        workspace_status: detect_workspace_status_for(Path::new(workspace_directory))?,
    })
}

fn detect_workspace_status_for(current_directory: &Path) -> Result<StartupWorkspaceStatus> {
    let fallback_directory = current_directory.display().to_string();
    let mut command = git_subprocess::command(["rev-parse", "--show-toplevel"]);
    command
        .current_dir(current_directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = subprocess::command_output(&mut command, "git rev-parse --show-toplevel");

    match output {
        Ok(result) if result.status.success() => {
            let root = String::from_utf8_lossy(&result.stdout).trim().to_string();
            Ok(StartupWorkspaceStatus {
                ok: true,
                path: root.clone(),
                detail: format!("git repo: {root}"),
            })
        }
        _ => Ok(StartupWorkspaceStatus {
            ok: true,
            path: fallback_directory,
            detail: "directory only (not inside a git repo)".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::detect_workspace_status_for;

    #[test]
    fn workspace_status_falls_back_to_directory_only_outside_git_repo() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "akra-startup-environment-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&temp_dir).expect("temp workspace should be created");

        let status = detect_workspace_status_for(&temp_dir)
            .expect("workspace status should fall back outside git repositories");

        assert!(status.ok);
        assert_eq!(status.path, temp_dir.display().to_string());
        assert_eq!(status.detail, "directory only (not inside a git repo)");

        fs::remove_dir_all(temp_dir).expect("temp workspace should be removed");
    }
}

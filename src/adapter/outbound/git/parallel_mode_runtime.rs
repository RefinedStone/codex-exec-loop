use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::Utc;

use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;

#[derive(Debug, Clone, Default)]
pub struct GitParallelModeRuntimeAdapter;

impl GitParallelModeRuntimeAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl ParallelModeRuntimePort for GitParallelModeRuntimeAdapter {
    fn detect_git_repo_root(&self, workspace_dir: &str) -> Option<String> {
        self.run_command(
            "git",
            &["-C", workspace_dir, "rev-parse", "--show-toplevel"],
            None,
        )
        .filter(|value| !value.is_empty())
    }

    fn command_succeeds(&self, program: &str, args: &[&str]) -> bool {
        let mut command = Command::new(program);
        command.args(args);
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        command.env("GIT_TERMINAL_PROMPT", "0");
        command.status().is_ok_and(|status| status.success())
    }

    fn run_command(
        &self,
        program: &str,
        args: &[&str],
        current_dir: Option<&str>,
    ) -> Option<String> {
        let mut command = Command::new(program);
        command.args(args);
        if let Some(current_dir) = current_dir {
            command.current_dir(current_dir);
        }
        command.stderr(Stdio::null());
        command.env("GIT_TERMINAL_PROMPT", "0");

        let output = command.output().ok()?;
        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8(output.stdout).ok()?;
        let trimmed = stdout.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    fn run_command_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        stdin_body: &str,
    ) -> Option<String> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0")
            .spawn()
            .ok()?;

        let mut stdin = child.stdin.take()?;
        stdin.write_all(stdin_body.as_bytes()).ok()?;
        drop(stdin);

        let output = child.wait_with_output().ok()?;
        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8(output.stdout).ok()?;
        let trimmed = stdout.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    fn find_executable(&self, program: &str) -> Option<PathBuf> {
        which::which(program).ok()
    }

    fn gh_auth_status(&self, repo_root: Option<&str>) -> bool {
        let mut command = Command::new("gh");
        command.args(["auth", "status"]);
        if let Some(repo_root) = repo_root {
            command.current_dir(repo_root);
        }
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        command.env("GIT_TERMINAL_PROMPT", "0");
        command.status().is_ok_and(|status| status.success())
    }

    fn current_timestamp(&self) -> String {
        Utc::now().to_rfc3339()
    }

    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn ensure_directory_exists(&self, path: &Path) -> std::io::Result<()> {
        if path.exists() {
            return Ok(());
        }

        std::fs::create_dir_all(path)
    }

    fn read_dir_paths(&self, path: &Path) -> std::io::Result<Vec<PathBuf>> {
        std::fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect()
    }

    fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn write_string(&self, path: &Path, body: &str) -> std::io::Result<()> {
        std::fs::write(path, body)
    }

    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)
    }
}

use std::path::{Path, PathBuf};

pub trait ParallelModeRuntimePort: Send + Sync {
    fn detect_git_repo_root(&self, workspace_dir: &str) -> Option<String>;
    fn command_succeeds(&self, program: &str, args: &[&str]) -> bool;
    fn run_command(
        &self,
        program: &str,
        args: &[&str],
        current_dir: Option<&str>,
    ) -> Option<String>;
    fn run_command_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        stdin_body: &str,
    ) -> Option<String>;
    fn find_executable(&self, program: &str) -> Option<PathBuf>;
    fn gh_auth_status(&self, repo_root: Option<&str>) -> bool;
    fn current_timestamp(&self) -> String;

    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf;
    fn path_exists(&self, path: &Path) -> bool;
    fn ensure_directory_exists(&self, path: &Path) -> std::io::Result<()>;
    fn read_dir_paths(&self, path: &Path) -> std::io::Result<Vec<PathBuf>>;
    fn read_to_string(&self, path: &Path) -> std::io::Result<String>;
    fn write_string(&self, path: &Path, body: &str) -> std::io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn remove_file(&self, path: &Path) -> std::io::Result<()>;
}

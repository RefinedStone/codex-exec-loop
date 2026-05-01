use std::fs;
use std::path::Path;

use chrono::Utc;

use super::git_sequence::{GitCommandStep, run_git_sequence};
use super::pool::reset_slot_worktree_to_akra;
use super::readiness::run_command;

pub(crate) fn ensure_directory_exists(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }

    fs::create_dir_all(path)
}

pub(crate) fn current_timestamp() -> String {
    Utc::now().to_rfc3339()
}

pub(crate) fn current_branch_name(worktree_path: &Path) -> Option<String> {
    let worktree_path_string = worktree_path.display().to_string();
    run_command(
        "git",
        [
            "-C",
            worktree_path_string.as_str(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ],
        None,
    )
}

pub(crate) fn discard_unstarted_slot_branch(
    repo_root: &str,
    slot_path: &Path,
    branch_name: &str,
) -> bool {
    reset_slot_worktree_to_akra(slot_path).succeeded()
        && run_git_sequence(
            "delete unstarted slot branch",
            vec![GitCommandStep::new(
                "delete agent branch",
                ["-C", repo_root, "branch", "-D", branch_name],
            )],
        )
        .succeeded()
}

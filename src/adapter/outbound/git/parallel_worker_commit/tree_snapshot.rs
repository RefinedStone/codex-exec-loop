//! Host tree observation through a disposable copy of the current real index.
//! Git may add blob/tree objects, but this path never changes the real index, refs,
//! HEAD, or worktree. Split indexes are explicitly unsupported rather than copied
//! incompletely. No model, hooks, filters, or remote Git operations are invoked.

use super::*;
use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelWorkerTreeRequest, ParallelWorkerTreeSnapshot,
};

mod index;
#[cfg(test)]
mod tests;

pub(crate) fn read_commit_tree(
    workspace_directory: &str,
    commit_sha: &str,
) -> Result<String, String> {
    ensure_valid_object_id(commit_sha, "frozen evidence source")?;
    if workspace_directory.trim().is_empty() {
        return Err("immutable evidence source workspace is missing".into());
    }
    let workspace = Path::new(workspace_directory);
    crate::git_execution_guard::ensure_host_git_execution_config_safe(workspace)
        .map_err(|error| error.to_string())?;
    ensure_exact_worktree_root(workspace)?;
    ensure_exact_commit(workspace, commit_sha)?;
    let reference = format!("{commit_sha}^{{tree}}");
    let tree = checked_git_text(
        workspace,
        ["rev-parse", "--verify", reference.as_str()],
        None,
    )?;
    ensure_valid_object_id(&tree, "frozen evidence tree")?;
    Ok(tree)
}

pub(crate) fn snapshot_parallel_worker_tree(
    request: ParallelWorkerTreeRequest<'_>,
) -> Result<ParallelWorkerTreeSnapshot, String> {
    snapshot_with_hook(request, || {})
}

fn snapshot_with_hook(
    request: ParallelWorkerTreeRequest<'_>,
    after_first_stage: impl FnOnce(),
) -> Result<ParallelWorkerTreeSnapshot, String> {
    if request.workspace_directory.trim().is_empty()
        || request.expected_branch_name.trim().is_empty()
    {
        return Err("parallel worker tree request is incomplete".into());
    }
    ensure_valid_object_id(request.expected_base_commit_sha, "frozen integration base")?;
    ensure_valid_object_id(request.expected_head_commit_sha, "observed HEAD")?;
    let workspace = Path::new(request.workspace_directory);
    validate_context(workspace, request)?;
    reject_untracked_special_files(workspace)?;
    let split_index = checked_git_output(workspace, ["rev-parse", "--shared-index-path"], None)?;
    if !split_index.stdout.iter().all(u8::is_ascii_whitespace) {
        return Err("parallel tree observation does not accept a shared/split index".into());
    }
    let mut original = index::OriginalIndex::open(workspace)?;
    let original_status = status(workspace)?;
    let paths = collect_changed_paths(workspace, None)?;
    let original_files = inspect_changed_path_metadata(workspace, &paths)?;
    let strong_identity = snapshot_changed_identities(workspace, &paths)?;
    let temporary = index::TemporaryIndex::new(original.bytes())?;

    stage_worker_index(workspace, Some(temporary.path()))?;
    let first_tree = read_tree(workspace, temporary.path())?;
    after_first_stage();
    // A second staging pass catches a tree produced from transient content even
    // when the final changed-path inventory is identical to the initial inventory.
    stage_worker_index(workspace, Some(temporary.path()))?;
    let second_tree = read_tree(workspace, temporary.path())?;
    if first_tree != second_tree {
        return Err("parallel worker tree changed while being observed".into());
    }
    ensure_changed_path_set_unchanged(&paths, &collect_changed_paths(workspace, None)?)?;
    if original_files != inspect_changed_path_metadata(workspace, &paths)?
        || strong_identity != snapshot_changed_identities(workspace, &paths)?
        || original_status != status(workspace)?
    {
        return Err("parallel worker files changed while observing the evidence tree".into());
    }
    original.verify()?;
    temporary.verify()?;
    validate_context(workspace, request)?;
    original.verify()?;
    Ok(ParallelWorkerTreeSnapshot {
        head_commit_sha: request.expected_head_commit_sha.into(),
        tree_sha: first_tree,
        worktree_clean: original_status.is_empty(),
    })
}

fn validate_context(
    workspace: &Path,
    request: ParallelWorkerTreeRequest<'_>,
) -> Result<(), String> {
    crate::git_execution_guard::ensure_host_git_execution_config_safe(workspace)
        .map_err(|error| error.to_string())?;
    ensure_exact_worktree_root(workspace)?;
    reject_pending_git_operation(workspace)?;
    reject_unsafe_index_and_submodule_state(workspace)?;
    ensure_exact_branch_and_head(
        workspace,
        request.expected_branch_name,
        request.expected_head_commit_sha,
    )?;
    ensure_exact_commit(workspace, request.expected_base_commit_sha)?;
    ensure_ancestor(
        workspace,
        request.expected_base_commit_sha,
        request.expected_head_commit_sha,
    )?;
    ensure_linear_history(
        workspace,
        request.expected_base_commit_sha,
        request.expected_head_commit_sha,
    )
}

fn status(workspace: &Path) -> Result<Vec<u8>, String> {
    Ok(checked_git_output(
        workspace,
        [
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=no",
            "--ignore-submodules=none",
        ],
        None,
    )?
    .stdout)
}

fn read_tree(workspace: &Path, index: &Path) -> Result<String, String> {
    let output = checked_git_output_with_index(workspace, ["write-tree"], None, Some(index))?;
    let tree = std::str::from_utf8(&output.stdout)
        .map_err(|_| "Git returned a non-UTF-8 evidence tree".to_string())?
        .trim();
    ensure_valid_object_id(tree, "observed evidence tree")?;
    Ok(tree.into())
}

fn snapshot_changed_identities(
    workspace: &Path,
    paths: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|relative| {
            let path = workspace.join(relative);
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok("missing".into());
                }
                Err(error) => {
                    return Err(format!("could not inspect evidence path identity: {error}"));
                }
            };
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                Ok(format!(
                    "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
                    metadata.dev(),
                    metadata.ino(),
                    metadata.mode(),
                    metadata.uid(),
                    metadata.nlink(),
                    metadata.len(),
                    metadata.mtime(),
                    metadata.mtime_nsec(),
                    metadata.ctime(),
                    metadata.ctime_nsec()
                ))
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(crate::private_fs::WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(&path)
                    .map_err(|error| error.to_string())?;
                crate::private_fs::validate_windows_path_identity(&path, &file, false)
                    .map_err(|error| error.to_string())?;
                let identity = crate::private_fs::windows_file_identity_snapshot(&file)
                    .map_err(|error| error.to_string())?;
                let _ = metadata;
                Ok(format!("{identity:?}"))
            }
            #[cfg(not(any(unix, windows)))]
            {
                let _ = metadata;
                Err("stable evidence file identity is unavailable".into())
            }
        })
        .collect()
}

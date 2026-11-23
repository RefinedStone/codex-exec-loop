//! Immutable commit-to-commit diff inspection for issue delivery decisions.
//! The bounded result contains every changed path or the whole inspection fails.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::application::port::outbound::parallel_mode_runtime_port::ParallelCommitDiffRequest;
use crate::domain::issue_automation::delivery::{
    IssueDeliveryDiff, IssueDeliveryDiffFile, IssueDeliveryDiffLines,
    MAX_ISSUE_DELIVERY_DIFF_FILES, validate_commit_range,
};
use crate::{git_subprocess, subprocess};

const MAX_DIFF_STDOUT_BYTES: usize = 1024 * 1024;
const MAX_DIFF_STDERR_BYTES: usize = 64 * 1024;

pub(super) fn inspect_parallel_commit_diff(
    request: ParallelCommitDiffRequest<'_>,
) -> Result<IssueDeliveryDiff, String> {
    validate_commit_range(request.base_commit_sha, request.source_commit_sha)?;
    let workspace = std::fs::canonicalize(request.workspace_directory)
        .map_err(|_| "the delivery diff worktree is unavailable".to_string())?;
    crate::git_execution_guard::ensure_host_git_execution_config_safe(&workspace)
        .map_err(|error| error.to_string())?;
    ensure_exact_worktree_root(&workspace)?;
    ensure_exact_commit(&workspace, request.base_commit_sha)?;
    ensure_exact_commit(&workspace, request.source_commit_sha)?;
    checked_git_output(
        &workspace,
        &[
            "merge-base",
            "--is-ancestor",
            request.base_commit_sha,
            request.source_commit_sha,
        ],
    )?;
    let output = checked_git_output(
        &workspace,
        &[
            "diff",
            "--numstat",
            "-z",
            // Each rename is a deletion plus an addition: both paths need protection checks.
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--no-relative",
            "--ignore-submodules=none",
            "--submodule=short",
            "--diff-algorithm=myers",
            "--no-indent-heuristic",
            request.base_commit_sha,
            request.source_commit_sha,
            "--",
        ],
    )?;
    parse_numstat(request, &output.stdout)
}

fn ensure_exact_worktree_root(workspace: &Path) -> Result<(), String> {
    // --work-tree pins Git's routing, so also require the repository marker at this root.
    let marker = std::fs::symlink_metadata(workspace.join(".git"))
        .map_err(|_| "delivery diff inspection requires the exact worktree root".to_string())?;
    if marker.file_type().is_symlink() || !(marker.is_file() || marker.is_dir()) {
        return Err("the delivery diff worktree marker is unsupported".into());
    }
    let output = checked_git_output(workspace, &["rev-parse", "--show-toplevel"])?;
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| "the delivery diff repository root is not UTF-8".to_string())?;
    let observed = PathBuf::from(text.trim_end_matches(['\r', '\n']));
    let observed = std::fs::canonicalize(observed)
        .map_err(|_| "the delivery diff repository root is unavailable".to_string())?;
    if observed.as_path() != workspace {
        return Err("delivery diff inspection requires the exact worktree root".into());
    }
    Ok(())
}

fn ensure_exact_commit(workspace: &Path, sha: &str) -> Result<(), String> {
    let commit = format!("{sha}^{{commit}}");
    let output = checked_git_output(workspace, &["rev-parse", "--verify", &commit])?;
    let observed = std::str::from_utf8(&output.stdout)
        .map_err(|_| "the delivery commit object ID is not UTF-8".to_string())?;
    if observed.trim_end_matches(['\r', '\n']) != sha {
        return Err("delivery diff inspection requires a complete commit object ID".into());
    }
    Ok(())
}

fn parse_numstat(
    request: ParallelCommitDiffRequest<'_>,
    bytes: &[u8],
) -> Result<IssueDeliveryDiff, String> {
    if bytes.is_empty() || bytes.len() > MAX_DIFF_STDOUT_BYTES || bytes.last() != Some(&0) {
        return Err("Git returned an empty, incomplete, or oversized delivery diff".into());
    }
    let mut files = BTreeMap::new();
    for row in bytes[..bytes.len() - 1].split(|byte| *byte == 0) {
        if files.len() >= MAX_ISSUE_DELIVERY_DIFF_FILES {
            return Err("the complete delivery diff exceeds its file bound".into());
        }
        let mut fields = row.splitn(3, |byte| *byte == b'\t');
        let added = fields
            .next()
            .ok_or_else(|| "Git returned an invalid delivery diff row".to_string())?;
        let deleted = fields
            .next()
            .ok_or_else(|| "Git returned an invalid delivery diff row".to_string())?;
        let path = fields
            .next()
            .ok_or_else(|| "Git returned an invalid delivery diff row".to_string())?;
        let path = std::str::from_utf8(path)
            .map_err(|_| "the complete delivery diff contains a non-UTF-8 path".to_string())?;
        let lines = match (added, deleted) {
            (b"-", b"-") => IssueDeliveryDiffLines::Binary,
            (b"-", _) | (_, b"-") => {
                return Err("Git returned an incomplete binary delivery diff row".into());
            }
            _ => IssueDeliveryDiffLines::Text {
                added: parse_line_count(added)?,
                deleted: parse_line_count(deleted)?,
            },
        };
        if files.insert(path.to_string(), lines).is_some() {
            return Err("Git returned a repeated delivery diff path".into());
        }
    }
    let diff = IssueDeliveryDiff {
        base_commit_sha: request.base_commit_sha.to_owned(),
        source_commit_sha: request.source_commit_sha.to_owned(),
        files: files
            .into_iter()
            .map(|(path, lines)| IssueDeliveryDiffFile { path, lines })
            .collect(),
    };
    diff.validate()?;
    Ok(diff)
}

fn parse_line_count(value: &[u8]) -> Result<u64, String> {
    if value.is_empty()
        || !value.iter().all(u8::is_ascii_digit)
        || (value.len() > 1 && value[0] == b'0')
    {
        return Err("Git returned an invalid delivery diff line count".into());
    }
    value.iter().try_fold(0u64, |total, byte| {
        total
            .checked_mul(10)
            .and_then(|total| total.checked_add(u64::from(*byte - b'0')))
            .ok_or_else(|| "the delivery diff line count overflowed".to_string())
    })
}

fn checked_git_output(workspace: &Path, args: &[&str]) -> Result<Output, String> {
    let mut command = safe_git_command(workspace);
    command.args(args);
    let child = subprocess::spawn(&mut command)
        .map_err(|error| format!("the delivery diff Git command could not start: {error}"))?;
    let output = subprocess::wait_with_output_timeout_and_limits(
        child,
        "git frozen delivery diff inspection",
        subprocess::configured_subprocess_timeout(),
        MAX_DIFF_STDOUT_BYTES,
        MAX_DIFF_STDERR_BYTES,
    )
    .map_err(|error| format!("the delivery diff could not be inspected completely: {error}"))?;
    if !output.status.success() {
        return Err("Git could not verify the complete frozen delivery commit range".into());
    }
    Ok(output)
}

fn safe_git_command(workspace: &Path) -> Command {
    let mut command = git_subprocess::command(std::iter::empty::<&str>());
    command
        .arg("--work-tree")
        .arg(workspace)
        .arg("-C")
        .arg(workspace)
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

#[cfg(test)]
mod tests;

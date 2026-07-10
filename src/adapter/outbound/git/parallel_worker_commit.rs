use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelWorkerCommitOutcome, ParallelWorkerCommitRequest,
};
use crate::{git_subprocess, subprocess};

const MAX_GIT_STDOUT_BYTES: usize = 1024 * 1024;
const MAX_GIT_STDERR_BYTES: usize = 256 * 1024;
const MAX_CHANGED_PATHS: usize = 4096;
const MAX_CHANGED_PATH_BYTES: usize = 128 * 1024;
const MAX_CHANGED_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CHANGED_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const CHANGED_PATH_DIFF_FILTER: &str = "--diff-filter=ACDMRTUXB";
const MAX_WORKTREE_SCAN_ENTRIES: usize = 1_000_000;
const MAX_ATTRIBUTE_BATCH_BYTES: usize = 8 * 1024;
const MAX_TRACKED_SUBMODULES: usize = 256;
const MAX_SUBMODULE_DEPTH: usize = 16;
const MAX_SUBMODULE_PATH_BYTES: usize = 128 * 1024;

pub(super) fn prepare_parallel_worker_commit(
    request: ParallelWorkerCommitRequest<'_>,
) -> Result<ParallelWorkerCommitOutcome, String> {
    validate_request(&request)?;
    let workspace = Path::new(request.workspace_directory);
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
    )?;

    let status = checked_git_output(
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
    )?;
    reject_untracked_special_files(workspace)?;
    if status.stdout.is_empty() {
        ensure_branch_tree_differs_from_base(
            workspace,
            request.expected_base_commit_sha,
            request.expected_head_commit_sha,
        )?;
        ensure_bounded_result_history(
            workspace,
            request.expected_base_commit_sha,
            request.expected_head_commit_sha,
            0,
        )?;
        return Ok(ParallelWorkerCommitOutcome::existing(
            request.expected_head_commit_sha,
        ));
    }

    ensure_bounded_result_history(
        workspace,
        request.expected_base_commit_sha,
        request.expected_head_commit_sha,
        1,
    )?;
    let head_gitlinks = gitlink_entries_at_head(workspace)?;
    let changed_paths = collect_changed_paths(workspace)?;
    let changed_path_snapshot = inspect_changed_path_metadata(workspace, &changed_paths)?;
    reject_external_filters_for_changed_paths(workspace)?;
    checked_git_output(workspace, ["add", "-A", "--", "."], None)?;
    reject_untracked_special_files(workspace)?;
    let staged_paths = collect_changed_paths(workspace)?;
    ensure_changed_path_set_unchanged(&changed_paths, &staged_paths)?;
    ensure_index_gitlinks_match_head(workspace, &head_gitlinks)?;
    let staged_path_snapshot = inspect_changed_path_metadata(workspace, &staged_paths)?;
    if staged_path_snapshot != changed_path_snapshot {
        return Err(
            "parallel worker changed file identity while host staging was in progress".to_string(),
        );
    }
    reject_unsafe_index_and_submodule_state(workspace)?;
    ensure_only_staged_changes_remain(workspace)?;

    let staged_diff = git_output(
        workspace,
        [
            "diff",
            "--cached",
            "--quiet",
            "--no-ext-diff",
            "--ignore-submodules=none",
            "--",
        ],
        None,
    )?;
    match staged_diff.status.code() {
        Some(1) => {}
        Some(0) => {
            return Err(
                "parallel worker produced no committable changes after secure staging".to_string(),
            );
        }
        _ => return Err(git_failure("inspect staged worker changes", &staged_diff)),
    }

    let tree_sha = checked_git_text(workspace, ["write-tree"], None)?;
    ensure_valid_object_id(&tree_sha, "staged tree")?;
    let base_tree = checked_git_text(
        workspace,
        [
            "rev-parse",
            "--verify",
            &format!("{}^{{tree}}", request.expected_base_commit_sha),
        ],
        None,
    )?;
    if tree_sha == base_tree {
        return Err(
            "parallel worker result has no tree changes relative to its frozen integration base"
                .to_string(),
        );
    }

    let commit_sha = checked_git_text(
        workspace,
        [
            "commit-tree",
            tree_sha.as_str(),
            "-p",
            request.expected_head_commit_sha,
            "-m",
            request.commit_message,
        ],
        Some(request.commit_timestamp),
    )?;
    ensure_valid_object_id(&commit_sha, "host-owned commit")?;

    let branch_ref = format!("refs/heads/{}", request.expected_branch_name);
    checked_git_output(
        workspace,
        [
            "update-ref",
            branch_ref.as_str(),
            commit_sha.as_str(),
            request.expected_head_commit_sha,
        ],
        None,
    )?;
    ensure_exact_branch_and_head(workspace, request.expected_branch_name, &commit_sha)?;
    reject_unsafe_index_and_submodule_state(workspace)?;
    ensure_clean_worktree(workspace)?;
    ensure_branch_tree_differs_from_base(workspace, request.expected_base_commit_sha, &commit_sha)?;

    Ok(ParallelWorkerCommitOutcome::created(commit_sha))
}

fn reject_untracked_special_files(workspace: &Path) -> Result<(), String> {
    let ignored = ignored_worktree_paths(workspace)?;
    let mut pending = vec![workspace.to_path_buf()];
    let mut inspected = 0usize;
    while let Some(directory) = pending.pop() {
        let entries = std::fs::read_dir(&directory)
            .map_err(|error| format!("worker worktree could not be scanned safely: {error}"))?;
        for entry in entries {
            inspected = inspected.saturating_add(1);
            if inspected > MAX_WORKTREE_SCAN_ENTRIES {
                return Err(
                    "worker worktree contains too many filesystem entries for safe host staging"
                        .to_string(),
                );
            }
            let entry = entry
                .map_err(|error| format!("worker worktree entry could not be read: {error}"))?;
            let path = entry.path();
            let relative = path
                .strip_prefix(workspace)
                .map_err(|_| "worker worktree scan escaped the leased workspace".to_string())?;
            if relative == Path::new(".git")
                || ignored.iter().any(|ignored_path| {
                    relative == ignored_path || relative.starts_with(ignored_path)
                })
            {
                continue;
            }
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                format!("worker worktree entry metadata could not be read: {error}")
            })?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() && !metadata_is_link_or_reparse(&metadata) {
                pending.push(path);
                continue;
            }
            if metadata.is_file() && !metadata_is_link_or_reparse(&metadata) {
                continue;
            }
            return Err(format!(
                "host staging allows only regular files or repository symlinks; `{}` has an unsupported type",
                relative.display()
            ));
        }
    }
    Ok(())
}

fn ignored_worktree_paths(workspace: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let output = checked_git_output(
        workspace,
        [
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
            "--ignore-submodules=none",
        ],
        None,
    )?;
    let mut ignored = BTreeSet::new();
    for record in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let Some(path) = record.strip_prefix(b"!! ") else {
            continue;
        };
        let path = std::str::from_utf8(path)
            .map_err(|_| "Git returned a non-UTF-8 ignored path".to_string())?
            .trim_end_matches('/');
        let path = PathBuf::from(path);
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || !path
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err("Git returned an unsafe ignored path during host staging".to_string());
        }
        ignored.insert(path);
    }
    Ok(ignored)
}

fn validate_request(request: &ParallelWorkerCommitRequest<'_>) -> Result<(), String> {
    if request.workspace_directory.trim().is_empty()
        || request.expected_branch_name.trim().is_empty()
        || request.commit_message.trim().is_empty()
    {
        return Err("host-owned parallel worker commit request is incomplete".to_string());
    }
    ensure_valid_object_id(request.expected_base_commit_sha, "frozen integration base")?;
    ensure_valid_object_id(request.expected_head_commit_sha, "pre-commit HEAD")?;
    chrono::DateTime::parse_from_rfc3339(request.commit_timestamp)
        .map_err(|_| "parallel worker lease timestamp is not valid RFC3339".to_string())?;
    Ok(())
}

fn ensure_valid_object_id(value: &str, label: &str) -> Result<(), String> {
    if matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!("{label} is not a full Git object id"))
    }
}

fn reject_pending_git_operation(workspace: &Path) -> Result<(), String> {
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
    ] {
        let marker_path = checked_git_text(workspace, ["rev-parse", "--git-path", marker], None)?;
        let marker_path = absolutize(workspace, Path::new(&marker_path));
        if std::fs::symlink_metadata(&marker_path).is_ok() {
            return Err(format!(
                "host-owned parallel worker commit is blocked by pending Git operation `{marker}`"
            ));
        }
    }
    Ok(())
}

fn ensure_exact_worktree_root(workspace: &Path) -> Result<(), String> {
    let reported = checked_git_text(workspace, ["rev-parse", "--show-toplevel"], None)?;
    let expected = std::fs::canonicalize(workspace)
        .map_err(|error| format!("parallel worker worktree could not be canonicalized: {error}"))?;
    let reported = std::fs::canonicalize(&reported).map_err(|error| {
        format!("Git-reported parallel worker worktree could not be canonicalized: {error}")
    })?;
    if expected == reported {
        Ok(())
    } else {
        Err(format!(
            "Git worktree root `{}` differs from leased workspace `{}`",
            reported.display(),
            expected.display()
        ))
    }
}

fn ensure_exact_branch_and_head(
    workspace: &Path,
    expected_branch: &str,
    expected_head: &str,
) -> Result<(), String> {
    let branch = checked_git_text(
        workspace,
        ["symbolic-ref", "--quiet", "--short", "HEAD"],
        None,
    )?;
    if branch != expected_branch {
        return Err(format!(
            "parallel worker branch drifted from `{expected_branch}` to `{branch}`"
        ));
    }
    let branch_ref = format!("refs/heads/{expected_branch}^{{commit}}");
    let branch_head = checked_git_text(
        workspace,
        ["rev-parse", "--verify", branch_ref.as_str()],
        None,
    )?;
    let head = checked_git_text(workspace, ["rev-parse", "--verify", "HEAD^{commit}"], None)?;
    if branch_head != expected_head || head != expected_head {
        return Err(format!(
            "parallel worker HEAD changed before host commit; expected `{expected_head}`"
        ));
    }
    Ok(())
}

fn ensure_exact_commit(workspace: &Path, expected_commit: &str) -> Result<(), String> {
    let resolved = checked_git_text(
        workspace,
        [
            "rev-parse",
            "--verify",
            &format!("{expected_commit}^{{commit}}"),
        ],
        None,
    )?;
    if resolved == expected_commit {
        Ok(())
    } else {
        Err("frozen integration base no longer resolves exactly".to_string())
    }
}

fn ensure_ancestor(workspace: &Path, base: &str, head: &str) -> Result<(), String> {
    let output = git_output(workspace, ["merge-base", "--is-ancestor", base, head], None)?;
    match output.status.code() {
        Some(0) => Ok(()),
        Some(1) => Err(
            "parallel worker HEAD does not descend from its frozen integration base".to_string(),
        ),
        _ => Err(git_failure("validate worker commit ancestry", &output)),
    }
}

fn ensure_linear_history(workspace: &Path, base: &str, head: &str) -> Result<(), String> {
    let range = format!("{base}..{head}");
    let output = checked_git_output(workspace, ["rev-list", "--merges", range.as_str()], None)?;
    if output.stdout.is_empty() {
        Ok(())
    } else {
        Err("parallel worker existing history contains a merge commit".to_string())
    }
}

fn ensure_bounded_result_history(
    workspace: &Path,
    base: &str,
    head: &str,
    host_commit_count: usize,
) -> Result<(), String> {
    let range = format!("{base}..{head}");
    let existing = checked_git_text(workspace, ["rev-list", "--count", range.as_str()], None)?
        .parse::<usize>()
        .map_err(|_| "Git returned an invalid parallel worker commit count".to_string())?;
    let total = existing.saturating_add(host_commit_count);
    if (1..=128).contains(&total) {
        Ok(())
    } else {
        Err(format!(
            "parallel worker result contains {total} commits; delivery requires 1 through 128"
        ))
    }
}

fn reject_unsafe_index_and_submodule_state(workspace: &Path) -> Result<(), String> {
    reject_unsafe_index_state(workspace)?;
    let canonical_root = std::fs::canonicalize(workspace)
        .map_err(|error| format!("parallel worker worktree could not be canonicalized: {error}"))?;
    let mut visited = BTreeSet::from([canonical_root.clone()]);
    let mut inspected_count = 0usize;
    let mut inspected_path_bytes = 0usize;
    inspect_tracked_submodules(
        workspace,
        &canonical_root,
        0,
        &mut visited,
        &mut inspected_count,
        &mut inspected_path_bytes,
    )
}

fn reject_unsafe_index_state(workspace: &Path) -> Result<(), String> {
    let unmerged = checked_git_output(workspace, ["ls-files", "--unmerged", "-z", "--"], None)?;
    if !unmerged.stdout.is_empty() {
        return Err(
            "host-owned parallel worker commit is blocked by unmerged index entries".to_string(),
        );
    }

    let tagged = checked_git_output(workspace, ["ls-files", "-v", "-z", "--"], None)?;
    let hidden_entries = count_hidden_index_entries(&tagged.stdout)?;
    if hidden_entries == 0 {
        Ok(())
    } else {
        Err(format!(
            "host-owned parallel worker commit is blocked by {hidden_entries} assume-unchanged or skip-worktree index entry(s)"
        ))
    }
}

fn count_hidden_index_entries(output: &[u8]) -> Result<usize, String> {
    let mut hidden_entries = 0usize;
    for entry in output
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        if entry.len() < 3 || entry[1] != b' ' {
            return Err("Git index flag inspection returned an invalid response".to_string());
        }
        let tag = entry[0];
        if tag == b'S' || tag.is_ascii_lowercase() {
            hidden_entries = hidden_entries.saturating_add(1);
        }
    }
    Ok(hidden_entries)
}

fn inspect_tracked_submodules(
    workspace: &Path,
    canonical_root: &Path,
    depth: usize,
    visited: &mut BTreeSet<PathBuf>,
    inspected_count: &mut usize,
    inspected_path_bytes: &mut usize,
) -> Result<(), String> {
    let submodule_paths = tracked_gitlink_paths(workspace)?;
    if depth >= MAX_SUBMODULE_DEPTH && !submodule_paths.is_empty() {
        return Err("tracked submodule nesting exceeds the host commit safety limit".to_string());
    }

    for relative_path in submodule_paths {
        *inspected_count = inspected_count.saturating_add(1);
        *inspected_path_bytes = inspected_path_bytes.saturating_add(relative_path.len());
        if *inspected_count > MAX_TRACKED_SUBMODULES
            || *inspected_path_bytes > MAX_SUBMODULE_PATH_BYTES
        {
            return Err(
                "tracked submodule inventory exceeds the host commit safety limit".to_string(),
            );
        }
        let relative_path = Path::new(&relative_path);
        if relative_path.is_absolute()
            || !relative_path
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err("Git returned an unsafe tracked submodule path".to_string());
        }
        let submodule_path = workspace.join(relative_path);
        let metadata = match std::fs::symlink_metadata(&submodule_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "tracked submodule worktree could not be inspected: {error}"
                ));
            }
        };
        if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
            return Err(
                "tracked submodule path must be a regular directory inside the leased worktree"
                    .to_string(),
            );
        }
        let mut entries = std::fs::read_dir(&submodule_path)
            .map_err(|error| format!("tracked submodule directory could not be read: {error}"))?;
        if entries
            .next()
            .transpose()
            .map_err(|error| format!("tracked submodule directory could not be read: {error}"))?
            .is_none()
        {
            continue;
        }

        ensure_exact_worktree_root(&submodule_path).map_err(|_| {
            "non-empty tracked submodule path is not an exact Git worktree root".to_string()
        })?;
        let canonical_submodule = std::fs::canonicalize(&submodule_path)
            .map_err(|error| format!("tracked submodule could not be canonicalized: {error}"))?;
        if canonical_submodule == canonical_root || !canonical_submodule.starts_with(canonical_root)
        {
            return Err(
                "tracked submodule worktree resolves outside the leased worktree".to_string(),
            );
        }
        if !visited.insert(canonical_submodule.clone()) {
            return Err("tracked submodule worktree inventory contains a cycle".to_string());
        }

        crate::git_execution_guard::ensure_verified_submodule_git_execution_config_safe(
            &submodule_path,
            &canonical_submodule,
            canonical_root,
        )
        .map_err(|error| error.to_string())?;
        reject_pending_git_operation(&submodule_path)?;
        reject_unsafe_index_state(&submodule_path)?;
        let status = checked_git_output(
            &submodule_path,
            [
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignored=matching",
                "--ignore-submodules=none",
            ],
            None,
        )?;
        if !status.stdout.is_empty() {
            return Err(
                "tracked submodule contains staged, unstaged, untracked, ignored, or nested-submodule changes"
                    .to_string(),
            );
        }
        inspect_tracked_submodules(
            &submodule_path,
            canonical_root,
            depth.saturating_add(1),
            visited,
            inspected_count,
            inspected_path_bytes,
        )?;
    }
    Ok(())
}

fn tracked_gitlink_paths(workspace: &Path) -> Result<Vec<String>, String> {
    let output = checked_git_output(workspace, ["ls-files", "--stage", "-z", "--"], None)?;
    parse_tracked_gitlink_paths(&output.stdout)
}

fn gitlink_entries_at_head(workspace: &Path) -> Result<BTreeMap<String, String>, String> {
    let output = checked_git_output(workspace, ["ls-tree", "-r", "-z", "HEAD"], None)?;
    parse_head_gitlink_entries(&output.stdout)
}

fn ensure_index_gitlinks_match_head(
    workspace: &Path,
    head_gitlinks: &BTreeMap<String, String>,
) -> Result<(), String> {
    let output = checked_git_output(workspace, ["ls-files", "--stage", "-z", "--"], None)?;
    let index_gitlinks = parse_index_gitlink_entries(&output.stdout)?;
    if &index_gitlinks == head_gitlinks {
        Ok(())
    } else {
        Err(
            "host staging blocked a new, removed, or changed Git gitlink; submodule pointer changes require explicit operator review"
                .to_string(),
        )
    }
}

fn parse_head_gitlink_entries(output: &[u8]) -> Result<BTreeMap<String, String>, String> {
    parse_gitlink_entries(output, true)
}

fn parse_index_gitlink_entries(output: &[u8]) -> Result<BTreeMap<String, String>, String> {
    parse_gitlink_entries(output, false)
}

fn parse_gitlink_entries(
    output: &[u8],
    head_inventory: bool,
) -> Result<BTreeMap<String, String>, String> {
    let mut entries = BTreeMap::new();
    for record in output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| "Git gitlink inventory returned an invalid record".to_string())?;
        let metadata = std::str::from_utf8(&record[..tab])
            .map_err(|_| "Git gitlink inventory returned non-UTF-8 metadata".to_string())?;
        let fields = metadata.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err("Git gitlink inventory returned invalid metadata".to_string());
        }
        if fields[0] != "160000" {
            continue;
        }
        if (!head_inventory && fields[2] != "0") || (head_inventory && fields[1] != "commit") {
            return Err("Git gitlink inventory contains an invalid stage or type".to_string());
        }
        let object_id = if head_inventory { fields[2] } else { fields[1] };
        ensure_valid_object_id(object_id, "Git gitlink")?;
        let path = std::str::from_utf8(&record[tab.saturating_add(1)..])
            .map_err(|_| "Git gitlink path is not valid UTF-8".to_string())?;
        if path.is_empty()
            || entries
                .insert(path.to_string(), object_id.to_string())
                .is_some()
        {
            return Err("Git gitlink inventory contains an invalid duplicate path".to_string());
        }
    }
    Ok(entries)
}

fn parse_tracked_gitlink_paths(output: &[u8]) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    for record in output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| "Git index inventory returned an invalid record".to_string())?;
        let metadata = std::str::from_utf8(&record[..tab])
            .map_err(|_| "Git index inventory returned non-UTF-8 metadata".to_string())?;
        let fields = metadata.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err("Git index inventory returned invalid metadata".to_string());
        }
        if fields[0] != "160000" {
            continue;
        }
        if fields[2] != "0" {
            return Err("tracked submodule has an unmerged index stage".to_string());
        }
        let path = std::str::from_utf8(&record[tab.saturating_add(1)..])
            .map_err(|_| "tracked submodule path is not valid UTF-8".to_string())?;
        if path.is_empty() {
            return Err("Git index inventory returned an empty submodule path".to_string());
        }
        paths.push(path.to_string());
    }
    Ok(paths)
}

fn ensure_branch_tree_differs_from_base(
    workspace: &Path,
    base: &str,
    head: &str,
) -> Result<(), String> {
    let output = git_output(
        workspace,
        [
            "diff",
            "--quiet",
            "--no-ext-diff",
            "--ignore-submodules=none",
            base,
            head,
            "--",
        ],
        None,
    )?;
    match output.status.code() {
        Some(1) => Ok(()),
        Some(0) => Err(
            "parallel worker produced no tree changes relative to its frozen integration base"
                .to_string(),
        ),
        _ => Err(git_failure(
            "compare worker result with frozen base",
            &output,
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChangedPathKind {
    Missing,
    Regular,
    #[cfg(not(windows))]
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChangedPathMetadataSnapshot {
    kind: ChangedPathKind,
    size: u64,
    identity: String,
}

fn inspect_changed_path_metadata(
    workspace: &Path,
    paths: &BTreeSet<String>,
) -> Result<BTreeMap<String, ChangedPathMetadataSnapshot>, String> {
    let mut snapshots = BTreeMap::new();
    let mut total_bytes = 0u64;
    for relative_path in paths {
        let relative = Path::new(relative_path);
        if relative.is_absolute()
            || !relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err("Git returned an unsafe changed path for host staging".to_string());
        }
        ensure_changed_path_ancestors_are_directories(workspace, relative)?;
        let path = workspace.join(relative);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                snapshots.insert(
                    relative_path.clone(),
                    ChangedPathMetadataSnapshot {
                        kind: ChangedPathKind::Missing,
                        size: 0,
                        identity: String::new(),
                    },
                );
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "changed path metadata could not be inspected safely: {error}"
                ));
            }
        };
        let (kind, size) = if metadata.file_type().is_symlink() {
            #[cfg(windows)]
            {
                return Err(
                    "host staging rejects Windows symlink or reparse-point paths".to_string(),
                );
            }
            #[cfg(not(windows))]
            {
                (ChangedPathKind::Symlink, metadata.len())
            }
        } else if metadata.is_file() && !metadata_is_link_or_reparse(&metadata) {
            reject_changed_path_hardlinks(&path, &metadata)?;
            (ChangedPathKind::Regular, metadata.len())
        } else {
            return Err(format!(
                "host staging allows only regular files or repository symlinks; `{relative_path}` has an unsupported type"
            ));
        };
        if size > MAX_CHANGED_FILE_BYTES {
            return Err(format!(
                "host staging rejects `{relative_path}` because it exceeds the 64 MiB per-file limit"
            ));
        }
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or_else(|| "host staging changed-file byte count overflowed".to_string())?;
        if total_bytes > MAX_CHANGED_TOTAL_BYTES {
            return Err(
                "host staging rejects changes that exceed the 256 MiB aggregate limit".to_string(),
            );
        }
        snapshots.insert(
            relative_path.clone(),
            ChangedPathMetadataSnapshot {
                kind,
                size,
                identity: changed_path_identity(&path, &metadata)?,
            },
        );
    }
    Ok(snapshots)
}

fn ensure_changed_path_ancestors_are_directories(
    workspace: &Path,
    relative_path: &Path,
) -> Result<(), String> {
    let mut current = workspace.to_path_buf();
    let component_count = relative_path.components().count();
    for component in relative_path
        .components()
        .take(component_count.saturating_sub(1))
    {
        current.push(component.as_os_str());
        let metadata = match std::fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "changed path ancestor could not be inspected safely: {error}"
                ));
            }
        };
        if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
            return Err(
                "host staging rejects changed paths below symlink or reparse-point ancestors"
                    .to_string(),
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn reject_changed_path_hardlinks(path: &Path, metadata: &std::fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    if metadata.nlink() == 1 {
        Ok(())
    } else {
        Err(format!(
            "host staging rejects hard-linked file `{}`",
            path.display()
        ))
    }
}

#[cfg(windows)]
fn reject_changed_path_hardlinks(path: &Path, _metadata: &std::fs::Metadata) -> Result<(), String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("changed Windows file could not be opened safely: {error}"))?;
    let links = crate::private_fs::windows_file_link_count(&file)
        .map_err(|error| format!("changed Windows file link count could not be read: {error}"))?;
    if links == 1 {
        Ok(())
    } else {
        Err(format!(
            "host staging rejects hard-linked file `{}`",
            path.display()
        ))
    }
}

#[cfg(not(any(unix, windows)))]
fn reject_changed_path_hardlinks(
    _path: &Path,
    _metadata: &std::fs::Metadata,
) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn changed_path_identity(path: &Path, metadata: &std::fs::Metadata) -> Result<String, String> {
    use std::os::unix::fs::MetadataExt;

    Ok(format!(
        "{}:{}:{}:{}:{}:{}:{}",
        metadata.dev(),
        metadata.ino(),
        metadata.nlink(),
        metadata.size(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        path.display()
    ))
}

#[cfg(windows)]
fn changed_path_identity(path: &Path, _metadata: &std::fs::Metadata) -> Result<String, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("changed Windows file could not be opened safely: {error}"))?;
    crate::private_fs::windows_file_identity_key(&file)
        .map_err(|error| format!("changed Windows file identity could not be read: {error}"))
}

#[cfg(not(any(unix, windows)))]
fn changed_path_identity(path: &Path, metadata: &std::fs::Metadata) -> Result<String, String> {
    Ok(format!("{}:{}", metadata.len(), path.display()))
}

fn reject_external_filters_for_changed_paths(workspace: &Path) -> Result<(), String> {
    let paths = collect_changed_paths(workspace)?;
    if paths.is_empty() {
        return Ok(());
    }

    let mut batch = Vec::new();
    let mut batch_bytes: usize = 0;
    for path in &paths {
        if !batch.is_empty() && batch_bytes.saturating_add(path.len()) > MAX_ATTRIBUTE_BATCH_BYTES {
            reject_external_filters_for_batch(workspace, &batch)?;
            batch.clear();
            batch_bytes = 0;
        }
        batch_bytes = batch_bytes.saturating_add(path.len());
        batch.push(path.as_str());
    }
    reject_external_filters_for_batch(workspace, &batch)
}

fn collect_changed_paths(workspace: &Path) -> Result<BTreeSet<String>, String> {
    let mut paths = BTreeSet::new();
    for args in [
        vec![
            "diff",
            "--name-only",
            "-z",
            CHANGED_PATH_DIFF_FILTER,
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=none",
            "--",
        ],
        vec![
            "diff",
            "--cached",
            "--name-only",
            "-z",
            CHANGED_PATH_DIFF_FILTER,
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=none",
            "--",
        ],
        vec!["ls-files", "--others", "--exclude-standard", "-z", "--"],
    ] {
        let output = checked_git_output(workspace, args, None)?;
        for path in parse_nul_paths(&output.stdout)? {
            paths.insert(path);
        }
    }
    enforce_changed_path_limits(&paths)?;
    Ok(paths)
}

fn enforce_changed_path_limits(paths: &BTreeSet<String>) -> Result<(), String> {
    if paths.len() > MAX_CHANGED_PATHS
        || paths.iter().map(String::len).sum::<usize>() > MAX_CHANGED_PATH_BYTES
    {
        return Err("parallel worker changed too many paths for safe host staging".to_string());
    }
    Ok(())
}

fn ensure_changed_path_set_unchanged(
    initial_paths: &BTreeSet<String>,
    staged_paths: &BTreeSet<String>,
) -> Result<(), String> {
    if initial_paths == staged_paths {
        Ok(())
    } else {
        Err("parallel worker changed path inventory while host staging was in progress".to_string())
    }
}

fn reject_external_filters_for_batch(workspace: &Path, paths: &[&str]) -> Result<(), String> {
    let mut args = vec!["check-attr", "-z", "filter", "--"];
    args.extend(paths.iter().copied());
    let output = checked_git_output(workspace, args, None)?;
    let fields = parse_nul_paths(&output.stdout)?;
    let mut chunks = fields.chunks_exact(3);
    for chunk in &mut chunks {
        if chunk[1] != "filter" {
            return Err("Git filter attribute inspection returned an invalid response".to_string());
        }
        if !matches!(chunk[2].as_str(), "unspecified" | "unset") {
            return Err(format!(
                "host staging blocked because `{}` activates Git filter `{}`; commit this path manually after reviewing the filter",
                chunk[0], chunk[2]
            ));
        }
    }
    if !chunks.remainder().is_empty() {
        return Err("Git filter attribute inspection returned a truncated response".to_string());
    }
    Ok(())
}

fn parse_nul_paths(output: &[u8]) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    for field in output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
    {
        let value = std::str::from_utf8(field)
            .map_err(|_| "parallel worker changed a path that is not valid UTF-8".to_string())?;
        values.push(value.to_string());
    }
    Ok(values)
}

fn ensure_only_staged_changes_remain(workspace: &Path) -> Result<(), String> {
    let unstaged = git_output(
        workspace,
        [
            "diff",
            "--quiet",
            "--no-ext-diff",
            "--ignore-submodules=none",
            "--",
        ],
        None,
    )?;
    match unstaged.status.code() {
        Some(0) => {}
        Some(1) => {
            return Err(
                "parallel worker worktree changed while host staging was in progress".to_string(),
            );
        }
        _ => return Err(git_failure("verify staged worker files", &unstaged)),
    }
    let untracked = checked_git_output(
        workspace,
        ["ls-files", "--others", "--exclude-standard", "-z", "--"],
        None,
    )?;
    if !untracked.stdout.is_empty() {
        return Err("parallel worker left untracked files after host staging".to_string());
    }
    Ok(())
}

fn ensure_clean_worktree(workspace: &Path) -> Result<(), String> {
    let output = checked_git_output(
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
    )?;
    if output.stdout.is_empty() {
        Ok(())
    } else {
        Err("host-owned parallel worker commit did not leave a clean worktree".to_string())
    }
}

fn checked_git_text<I, S>(
    workspace: &Path,
    args: I,
    commit_timestamp: Option<&str>,
) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = checked_git_output(workspace, args, commit_timestamp)?;
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "Git returned non-UTF-8 output during host commit".to_string())?;
    let value = stdout.trim();
    if value.is_empty() {
        Err("Git returned an empty value during host commit".to_string())
    } else {
        Ok(value.to_string())
    }
}

fn checked_git_output<I, S>(
    workspace: &Path,
    args: I,
    commit_timestamp: Option<&str>,
) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_output(workspace, args, commit_timestamp)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(git_failure(
            "prepare host-owned parallel worker commit",
            &output,
        ))
    }
}

fn git_output<I, S>(
    workspace: &Path,
    args: I,
    commit_timestamp: Option<&str>,
) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = safe_git_command(workspace)?;
    command.args(args);
    if let Some(timestamp) = commit_timestamp {
        command.env("GIT_AUTHOR_NAME", "Akra Parallel Worker");
        command.env("GIT_AUTHOR_EMAIL", "akra@localhost.invalid");
        command.env("GIT_COMMITTER_NAME", "Akra Parallel Worker");
        command.env("GIT_COMMITTER_EMAIL", "akra@localhost.invalid");
        command.env("GIT_AUTHOR_DATE", timestamp);
        command.env("GIT_COMMITTER_DATE", timestamp);
    }
    subprocess::wait_with_output_timeout_and_limits(
        subprocess::spawn(&mut command)
            .map_err(|error| format!("failed to spawn Git for host commit: {error}"))?,
        "git host-owned parallel worker commit",
        subprocess::configured_subprocess_timeout(),
        MAX_GIT_STDOUT_BYTES,
        MAX_GIT_STDERR_BYTES,
    )
    .map_err(|error| format!("host-owned parallel worker Git command failed: {error}"))
}

fn safe_git_command(workspace: &Path) -> Result<Command, String> {
    let canonical_workspace = std::fs::canonicalize(workspace)
        .map_err(|error| format!("host Git workspace could not be canonicalized: {error}"))?;
    let mut command = git_subprocess::command(std::iter::empty::<&str>());
    command
        // `-c core.worktree=...` is applied too late for Git's initial repository setup.
        // Pin the worktree as a global option so a submodule-local core.worktree cannot redirect
        // any status, index, or commit operation after its raw value has been audited.
        .arg("--work-tree")
        .arg(&canonical_workspace);
    #[cfg(windows)]
    command.args([
        "-c",
        &format!("core.autocrlf={}", windows_host_autocrlf_policy(workspace)?),
    ]);
    command
        .args(["-c", "i18n.commitEncoding=UTF-8"])
        .args(["-c", "user.name=Akra Parallel Worker"])
        .args(["-c", "user.email=akra@localhost.invalid"])
        .arg("-C")
        .arg(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_CONFIG_GLOBAL", null_device());
    Ok(command)
}

#[cfg(windows)]
fn windows_host_autocrlf_policy(workspace: &Path) -> Result<&'static str, String> {
    let mut command = git_subprocess::command(std::iter::empty::<&str>());
    command
        .arg("-C")
        .arg(workspace)
        .args(["config", "--includes", "--get", "core.autocrlf"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = subprocess::wait_with_output_timeout_and_limits(
        subprocess::spawn(&mut command)
            .map_err(|error| format!("failed to inspect repository core.autocrlf: {error}"))?,
        "host Git core.autocrlf inspection",
        subprocess::configured_subprocess_timeout(),
        MAX_GIT_STDOUT_BYTES,
        MAX_GIT_STDERR_BYTES,
    )
    .map_err(|error| format!("host Git core.autocrlf inspection failed: {error}"))?;
    if output.status.code() == Some(1) && output.stdout.is_empty() && output.stderr.is_empty() {
        // Git for Windows commonly checks out through a user-level `true` policy.
        // `input` keeps that index/worktree clean while avoiding checkout conversion.
        return Ok("input");
    }
    if !output.status.success() {
        return Err(git_failure("inspect repository core.autocrlf", &output));
    }
    let value = std::str::from_utf8(&output.stdout)
        .map_err(|_| "repository core.autocrlf is not valid UTF-8".to_string())?
        .trim();
    normalize_host_autocrlf_value(value)
}

#[cfg(any(windows, test))]
fn normalize_host_autocrlf_value(value: &str) -> Result<&'static str, String> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok("true"),
        "false" | "no" | "off" | "0" => Ok("false"),
        "input" => Ok("input"),
        _ => Err("repository core.autocrlf has an unsupported value".to_string()),
    }
}

#[cfg(windows)]
fn null_device() -> &'static str {
    "NUL"
}

#[cfg(not(windows))]
fn null_device() -> &'static str {
    "/dev/null"
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn absolutize(workspace: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    }
}

fn git_failure(operation: &str, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    if detail.is_empty() {
        format!("{operation} failed with status {}", output.status)
    } else {
        format!("{operation} failed: {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CHANGED_PATH_DIFF_FILTER, MAX_CHANGED_PATHS, count_hidden_index_entries,
        enforce_changed_path_limits, ensure_changed_path_set_unchanged,
        normalize_host_autocrlf_value, parse_head_gitlink_entries, parse_index_gitlink_entries,
        parse_tracked_gitlink_paths,
    };
    use std::collections::BTreeSet;

    #[test]
    fn nul_delimited_index_parsers_do_not_split_control_char_paths() {
        let sha = "a".repeat(40);
        let inventory = format!(
            "160000 {sha} 0\tmodule\nwith\ttabs\0\
             100644 {sha} 0\tnormal\r\nfile\0"
        );
        assert_eq!(
            parse_tracked_gitlink_paths(inventory.as_bytes()).expect("inventory should parse"),
            vec!["module\nwith\ttabs".to_string()]
        );

        assert_eq!(
            count_hidden_index_entries(b"H normal\nfile\0S skip\tfile\0h assumed\r\nfile\0")
                .expect("tagged index should parse"),
            2
        );
    }

    #[test]
    fn head_and_index_gitlink_inventory_detects_pointer_or_path_changes() {
        let original = "a".repeat(40);
        let changed = "b".repeat(40);
        let head = format!("160000 commit {original}\tmodule\0");
        let matching_index = format!("160000 {original} 0\tmodule\0");
        let changed_index = format!("160000 {changed} 0\tmodule\0");
        let added_index =
            format!("160000 {original} 0\tmodule\0160000 {changed} 0\tnested-repository\0");
        let head = parse_head_gitlink_entries(head.as_bytes()).expect("HEAD should parse");
        assert_eq!(
            parse_index_gitlink_entries(matching_index.as_bytes()).expect("index should parse"),
            head
        );
        assert_ne!(
            parse_index_gitlink_entries(changed_index.as_bytes()).expect("index should parse"),
            head
        );
        assert_ne!(
            parse_index_gitlink_entries(added_index.as_bytes()).expect("index should parse"),
            head
        );
    }

    #[test]
    fn changed_path_limit_counts_mass_tracked_deletions() {
        assert!(
            CHANGED_PATH_DIFF_FILTER.contains('D'),
            "tracked deletions must be present in the diff inventory"
        );
        let deleted_paths = (0..=MAX_CHANGED_PATHS)
            .map(|index| format!("deleted/{index:05}.txt"))
            .collect::<BTreeSet<_>>();

        assert_eq!(deleted_paths.len(), MAX_CHANGED_PATHS + 1);
        assert_eq!(
            enforce_changed_path_limits(&deleted_paths),
            Err("parallel worker changed too many paths for safe host staging".to_string())
        );
    }

    #[test]
    fn staged_path_inventory_rejects_files_created_during_git_add() {
        let initial = BTreeSet::from(["known.txt".to_string()]);
        let staged = BTreeSet::from(["known.txt".to_string(), "raced-into-index.txt".to_string()]);

        assert_eq!(
            ensure_changed_path_set_unchanged(&initial, &staged),
            Err(
                "parallel worker changed path inventory while host staging was in progress"
                    .to_string()
            )
        );
    }

    #[test]
    fn host_autocrlf_policy_accepts_only_non_executable_git_values() {
        for (value, expected) in [
            ("true", "true"),
            ("YES", "true"),
            ("0", "false"),
            ("off", "false"),
            ("input", "input"),
        ] {
            assert_eq!(
                normalize_host_autocrlf_value(value).expect("known Git value should normalize"),
                expected
            );
        }
        assert!(normalize_host_autocrlf_value("checkout-command").is_err());
    }
}

//! 기존 checkout guard와 private directory anchor로 사용자 대화 공간만 새로 만든다.
use std::path::Path;

use anyhow::{Context, Result, ensure};

use super::{GitDirs, absolute, read, run, write_files};
use crate::application::port::outbound::git_workspace_port::GitConversationWorktree;
use crate::private_sqlite::PinnedPrivateDirectory;

pub(super) fn create(repository: &str, destination: &str, branch: &str, base: &str) -> Result<()> {
    ensure!(
        crate::domain::git_graph::is_git_commit_id(base),
        "worktree base must be a full SHA"
    );
    let suffix = branch
        .strip_prefix("akra-conversation/")
        .context("workspace branch prefix is invalid")?;
    ensure!(
        suffix.len() == 64 && suffix.bytes().all(|b| b.is_ascii_hexdigit()),
        "workspace branch identity is invalid"
    );
    let destination = absolute(destination)?;
    ensure!(
        destination.file_name().and_then(|s| s.to_str()) == Some(suffix),
        "workspace destination differs from its identity"
    );
    let parent = PinnedPrivateDirectory::open(
        destination.parent().context("workspace parent missing")?,
        false,
    )?;
    ensure!(
        matches!(std::fs::symlink_metadata(destination), Err(e) if e.kind() == std::io::ErrorKind::NotFound),
        "workspace destination already exists or cannot be inspected"
    );
    let source = PinnedPrivateDirectory::pin_ancestors(absolute(repository)?)?;
    let target = git_path(destination)?;
    parent.verify()?;
    source.verify()?;
    write_files(
        repository,
        base,
        &[
            "worktree",
            "add",
            "--no-track",
            "-b",
            branch,
            "--",
            &target,
            base,
        ],
    )?;
    parent.verify()?;
    source.verify()
}

pub(super) fn inspect(
    repository: &str,
    destination: &str,
) -> Result<Option<GitConversationWorktree>> {
    let destination = absolute(destination)?;
    match std::fs::symlink_metadata(destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("cannot inspect conversation workspace"),
        Ok(_) => {}
    }
    let pinned = PinnedPrivateDirectory::pin_ancestors(destination)?;
    let source = absolute(repository)?;
    let source_anchor = PinnedPrivateDirectory::pin_ancestors(source)?;
    let source_common = std::fs::canonicalize(GitDirs::read(source)?.common)?;
    let target_common = std::fs::canonicalize(GitDirs::read(destination)?.common)?;
    ensure!(
        source_common == target_common,
        "workspace belongs to a different Git repository"
    );
    let canonical = std::fs::canonicalize(destination)?;
    let listing = read(source, &["worktree", "list", "--porcelain", "-z"])?;
    ensure!(listing.split('\0').filter_map(|field| field.strip_prefix("worktree "))
        .any(|path| std::fs::canonicalize(path).is_ok_and(|registered| registered == canonical)),
        "workspace is not registered at this path");
    let toplevel = read(destination, &["rev-parse", "--show-toplevel"])?;
    ensure!(
        std::fs::canonicalize(toplevel.trim())? == canonical,
        "workspace path is not its Git root"
    );
    let head = read(destination, &["rev-parse", "--verify", "HEAD^{commit}"])?
        .trim()
        .to_string();
    let branch = run(destination, &["symbolic-ref", "--quiet", "--short", "HEAD"])?
        .ok()
        .map(|s| s.trim().to_string());
    pinned.verify()?;
    source_anchor.verify()?;
    Ok(Some(GitConversationWorktree { head, branch }))
}

fn git_path(path: &Path) -> Result<String> {
    let value = path.to_str().context("workspace path is not Unicode")?;
    #[cfg(windows)]
    {
        if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            return Ok(format!(r"\\{unc}"));
        }
        Ok(value.strip_prefix(r"\\?\").unwrap_or(value).to_string())
    }
    #[cfg(not(windows))]
    Ok(value.to_string())
}

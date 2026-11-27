//! 저장소 식별자를 planning 기준 데이터의 위치 계산에서 얻는다 (A03). 커널 잠금과 같은 값이라 같은 저장소의
//! worktree 들이 같은 식별자를 갖는다.

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::repository_identity_port::RepositoryIdentityPort;

pub(crate) struct PlanningRepositoryIdentityAdapter;

impl RepositoryIdentityPort for PlanningRepositoryIdentityAdapter {
    fn repository_identity(&self, cwd: &str) -> Result<String> {
        // 없는 폴더는 위치 계산이 그 글자를 그대로 식별자로 쓴다. 장부에 엉뚱한 범위를 만들지 않게 먼저 막는다.
        let path = Path::new(cwd);
        if !path.is_absolute() || !path.is_dir() {
            bail!("`cwd` must be an existing absolute folder")
        }
        Ok(
            SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(cwd)?
                .repository_identity,
        )
    }

    fn task_exists(&self, cwd: &str, task_id: &str) -> Result<bool> {
        Ok(
            SqlitePlanningAuthorityAdapter::load_task_authority_snapshot(cwd)?.is_some_and(
                |snapshot| {
                    snapshot
                        .task_authority
                        .tasks
                        .iter()
                        .any(|task| task.id == task_id)
                },
            ),
        )
    }

    fn validate_path(&self, cwd: &str, path: &str) -> Result<()> {
        // Use this checkout's root, not the primary checkout of a linked worktree.
        let mut git = crate::git_subprocess::command(["rev-parse", "--show-toplevel"]);
        git.current_dir(cwd);
        let output = crate::subprocess::command_output(&mut git, "context worktree root")?;
        if !output.status.success() {
            bail!("context paths require a Git worktree");
        }
        let root = std::fs::canonicalize(String::from_utf8(output.stdout)?.trim())
            .context("context worktree root is unavailable")?;
        validate_relative_path(&root, path)
    }
}

fn validate_relative_path(root: &Path, path: &str) -> Result<()> {
    let mut cursor = root.to_path_buf();
    for component in Path::new(path).components() {
        let std::path::Component::Normal(name) = component else {
            bail!("context path is not repository-relative");
        };
        // Reject Windows aliases, alternate data streams and device paths even if absent.
        let name_text = name.to_str().context("context path is not UTF-8")?;
        let stem = name_text
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let reserved_device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ["COM", "LPT"].iter().any(|prefix| {
                stem.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
            });
        if reserved_device
            || name_text.contains([':', '<', '>', '"', '|', '?', '*'])
            || name_text.ends_with(['.', ' '])
            || name_text.eq_ignore_ascii_case(".git")
        {
            bail!("context path contains an ambiguous or reserved component");
        }
        cursor.push(name);
        match std::fs::symlink_metadata(&cursor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    bail!("context path traverses a symlink");
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        bail!("context path traverses a reparse point");
                    }
                }
                if !std::fs::canonicalize(&cursor)?.starts_with(root) {
                    bail!("context path is outside the repository");
                }
            }
            // A deleted/renamed path may still be a meaningful ledger scope. Never follow links.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_paths_reject_reserved_components_and_link_escape() {
        let base = crate::test_utils::platform_safe_temp_dir()
            .join(format!("akra-context-path-{:032x}", rand::random::<u128>()));
        let root = base.join("repo");
        let outside = base.join("outside");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        for path in ["src/file.rs", "src/renamed/deleted.rs"] {
            validate_relative_path(&root, path).unwrap();
        }
        for path in [
            "../outside",
            "src/../../outside",
            "src/NUL.txt",
            ".git/config",
            "src/file:secret",
            "src/.. ",
        ] {
            assert!(validate_relative_path(&root, path).is_err(), "{path}");
        }
        let link = root.join("linked");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        #[cfg(windows)]
        {
            // Directory junction creation does not require developer mode or elevation.
            let status = std::process::Command::new("cmd.exe")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(&link)
                .arg(&outside)
                .output()
                .unwrap();
            assert!(status.status.success(), "failed to create fixture junction");
        }
        assert!(validate_relative_path(&root, "linked/file.rs").is_err());
        #[cfg(windows)]
        std::fs::remove_dir(&link).unwrap();
        #[cfg(unix)]
        std::fs::remove_file(&link).unwrap();
        std::fs::remove_dir_all(base).unwrap();
    }
}

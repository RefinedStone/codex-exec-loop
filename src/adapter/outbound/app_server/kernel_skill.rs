//! Installs the generated kernel CLI skill independently of MCP server registration.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const SKILL_TEXT: &str =
    include_str!("../../../../assets/app-server/hub-skills/akra-kernel/SKILL.md");

pub(super) fn agent_skill_root() -> Result<PathBuf> {
    install(&crate::configuration::akra_home_dir()?.join("kernel-skills"))
}

fn install(root: &Path) -> Result<PathBuf> {
    let directory = root.join("akra-kernel");
    let path = directory.join("SKILL.md");
    if !std::fs::read_to_string(&path).is_ok_and(|current| current == SKILL_TEXT) {
        crate::private_directory::ensure_private_directory(&directory)?;
        std::fs::write(&path, SKILL_TEXT)
            .with_context(|| format!("failed to write `{}`", path.display()))?;
    }
    Ok(root.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_kernel_skill_without_mcp_servers() {
        let root = std::env::temp_dir().join(format!("akra-kernel-skill-{}", std::process::id()));
        let registered = install(&root).unwrap();
        assert_eq!(registered, root);
        assert_eq!(
            std::fs::read_to_string(registered.join("akra-kernel/SKILL.md")).unwrap(),
            SKILL_TEXT
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

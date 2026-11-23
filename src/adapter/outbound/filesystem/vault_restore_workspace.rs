//! 대표 checkout 조회는 runtime/lock/저널 디렉터리를 만들지 않는다.
use crate::adapter::outbound::db::sqlite_planning_authority_adapter::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::vault_restore_workspace_port::{
    VaultRestoreWorkspaceError, VaultRestoreWorkspacePort,
};
use anyhow::{Result, bail};
use std::path::Path;

pub(crate) struct VaultRestoreWorkspaceAdapter;
impl VaultRestoreWorkspacePort for VaultRestoreWorkspaceAdapter {
    fn canonical_workspace(&self, workspace: &str) -> Result<String, VaultRestoreWorkspaceError> {
        canonical_workspace(workspace).map_err(|_| VaultRestoreWorkspaceError::Unavailable)
    }
}
fn canonical_workspace(workspace: &str) -> Result<String> {
    let path = Path::new(workspace);
    if !path.is_absolute() {
        bail!("invalid restore workspace");
    }
    let canonical = std::fs::canonicalize(path)?;
    if !canonical.is_dir() {
        bail!("invalid restore workspace");
    }
    let Some(primary) = SqlitePlanningAuthorityAdapter::resolve_primary_checkout_root_for_read(
        canonical
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("invalid restore workspace"))?,
    )?
    else {
        bail!("restore workspace has no primary checkout");
    };
    let primary = std::fs::canonicalize(primary)?;
    if primary != canonical {
        bail!("restore workspace is not the primary checkout");
    }
    primary
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("invalid restore workspace"))
}

#[cfg(test)]
mod tests;
